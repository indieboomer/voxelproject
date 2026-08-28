use std::collections::{HashMap, HashSet};
use std::net::{SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::voxel::block::BlockType;

pub type PlayerId = u32;
pub const HOST_PLAYER_ID: PlayerId = 0;
pub const DEFAULT_PORT: u16 = 7878;
pub const RELIABLE_RESEND_INTERVAL: Duration = Duration::from_millis(200);
pub const SNAPSHOT_INTERVAL: f32 = 1.0 / 20.0;
pub const CONNECTION_TIMEOUT: Duration = Duration::from_secs(10);
/// Longest nickname kept, in characters -- applied both by the menu's text
/// field and (since a nickname arrives over the network) defensively again
/// wherever a host stores one it received from a client.
pub const MAX_NICKNAME_LEN: usize = 24;

/// How a `Notify` should be styled, so a joined client renders the same
/// toast color / chat-log color the host did instead of a single generic
/// look for every kind of message.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum NotifyKind {
    /// A routine FYI -- rule enabled/disabled/deleted, a player joined, ...
    Info,
    /// Something the player actually needs to notice and act on, e.g. a
    /// rule crashing or a new rule waiting to be enabled.
    Important,
    /// A player-authored chat line, already formatted with its sender.
    Chat,
}

/// Messages that must arrive, delivered by resending until the receiver
/// acknowledges them. Safe to apply more than once (idempotent).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ReliableMsg {
    Hello { nickname: String },
    Welcome {
        player_id: PlayerId,
        seed: u32,
        time_of_day: f32,
        spawn: [f32; 3],
        edits: Vec<((i32, i32, i32), BlockType)>,
    },
    BlockEdit {
        x: i32,
        y: i32,
        z: i32,
        block: BlockType,
    },
    PlayerLeft {
        player_id: PlayerId,
    },
    /// A short host-originated status message shown as a toast (and logged
    /// to the persistent chat scrollback) on every client -- rule
    /// activated/deactivated/deleted/generated, players joining, chat
    /// messages, etc.
    Notify { kind: NotifyKind, text: String },
    /// A client's typed chat message, not yet attributed to a sender -- the
    /// host fills that in from the connection it arrived on (so a client
    /// can't spoof another player's identity) and relays the formatted
    /// result to everyone as `Notify`.
    ChatMessage(String),
}

/// Best-effort messages sent every tick; a dropped one is superseded by the
/// next, so no retry logic is needed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum UnreliableMsg {
    PlayerState {
        pos: [f32; 3],
        yaw: f32,
        carrying_crystal: bool,
    },
    Snapshot {
        time_of_day: f32,
        weather: u8,
        players: Vec<(PlayerId, [f32; 3], f32, bool)>,
        creatures: Vec<([f32; 3], u8, f32)>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Packet {
    Reliable { id: u64, msg: ReliableMsg },
    Ack { id: u64 },
    Unreliable(UnreliableMsg),
}

pub fn encode(packet: &Packet) -> Vec<u8> {
    bincode::serialize(packet).expect("packet serialization cannot fail")
}

pub fn decode(bytes: &[u8]) -> Option<Packet> {
    bincode::deserialize(bytes).ok()
}

/// Tracks outgoing reliable messages until acknowledged, and de-duplicates
/// incoming ones so they're only applied once per logical send.
pub struct ReliableChannel {
    next_id: u64,
    pending: HashMap<u64, (Instant, SocketAddr, Vec<u8>)>,
    seen: HashMap<SocketAddr, HashSet<u64>>,
}

impl ReliableChannel {
    pub fn new() -> Self {
        Self {
            next_id: 1,
            pending: HashMap::new(),
            seen: HashMap::new(),
        }
    }

    /// Sends a reliable message and remembers it for resending until acked.
    pub fn send(&mut self, socket: &UdpSocket, addr: SocketAddr, msg: ReliableMsg) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        let bytes = encode(&Packet::Reliable { id, msg });
        let _ = socket.send_to(&bytes, addr);
        self.pending.insert(id, (Instant::now(), addr, bytes));
        id
    }

    /// Resends any reliable message that hasn't been acked within the resend
    /// interval.
    pub fn resend_due(&mut self, socket: &UdpSocket) {
        let now = Instant::now();
        for (timestamp, addr, bytes) in self.pending.values_mut() {
            if now.duration_since(*timestamp) >= RELIABLE_RESEND_INTERVAL {
                let _ = socket.send_to(bytes, *addr);
                *timestamp = now;
            }
        }
    }

    pub fn ack(&mut self, id: u64) {
        self.pending.remove(&id);
    }

    /// Stops resending a message we no longer care about (e.g. Hello once
    /// Welcome has already arrived).
    pub fn forget(&mut self, id: u64) {
        self.pending.remove(&id);
    }

    /// Returns true the first time `id` is seen from `addr`; false for
    /// repeats, so callers can skip re-applying a message they already
    /// processed.
    pub fn mark_seen(&mut self, addr: SocketAddr, id: u64) -> bool {
        self.seen.entry(addr).or_default().insert(id)
    }

    pub fn ack_reply(socket: &UdpSocket, addr: SocketAddr, id: u64) {
        let bytes = encode(&Packet::Ack { id });
        let _ = socket.send_to(&bytes, addr);
    }
}

pub fn bind_nonblocking(addr: &str) -> std::io::Result<UdpSocket> {
    let socket = UdpSocket::bind(addr)?;
    socket.set_nonblocking(true)?;
    Ok(socket)
}

pub const DEFAULT_LLM_URL: &str = "http://127.0.0.1:8090";

pub struct LaunchConfig {
    /// Join this host instead of hosting our own game.
    pub connect: Option<SocketAddr>,
    /// Port to listen on when hosting.
    pub port: u16,
    /// Base URL of a locally running `llama-server` (llama.cpp), used to
    /// generate new rule modules from natural-language prompts.
    pub llm_url: String,
    /// When hosting, generate a brand new world even if a save file exists
    /// (the main menu's "New World" vs. "Load World" distinction; CLI-only
    /// launches keep the old auto-load-else-fresh behavior).
    pub fresh: bool,
    /// Shown to other players in chat and join notifications instead of a
    /// bare "P{id}". The menu always asks for this before launching; a
    /// direct `--connect` CLI launch (no menu) falls back to `--nickname`
    /// or else a generic default.
    pub nickname: String,
}

/// Minimal `--connect <ip:port>` / `--port <n>` / `--llm-url <url>` /
/// `--nickname <name>` parsing. Any other args are ignored so this stays
/// forgiving for an MVP.
pub fn parse_args() -> LaunchConfig {
    let args: Vec<String> = std::env::args().collect();
    let mut connect = None;
    let mut port = DEFAULT_PORT;
    let mut llm_url = DEFAULT_LLM_URL.to_string();
    let mut nickname = "Player".to_string();

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--connect" => {
                if let Some(value) = args.get(i + 1) {
                    match value.parse::<SocketAddr>() {
                        Ok(addr) => connect = Some(addr),
                        Err(_) => {
                            eprintln!("Invalid --connect address '{value}', expected ip:port");
                            std::process::exit(1);
                        }
                    }
                    i += 1;
                }
            }
            "--port" => {
                if let Some(value) = args.get(i + 1) {
                    if let Ok(p) = value.parse::<u16>() {
                        port = p;
                    }
                    i += 1;
                }
            }
            "--llm-url" => {
                if let Some(value) = args.get(i + 1) {
                    llm_url = value.clone();
                    i += 1;
                }
            }
            "--nickname" => {
                if let Some(value) = args.get(i + 1) {
                    nickname = value.clone();
                    i += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }

    LaunchConfig {
        connect,
        port,
        llm_url,
        fresh: false,
        nickname,
    }
}
