use crate::transport::{JoinTarget, Peer, Transport};
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::voxel::block::BlockType;

pub type PlayerId = u32;
pub type WorldEdit = ((i32, i32, i32), BlockType);
pub const MAX_PLAYERS: usize = 4;
pub const PROTOCOL_VERSION: u32 = 8;
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
    Hotbar(crate::equipment::Hotbar),
    ItemAction(crate::equipment::Intent),
    /// Client intention; the host resolves the recipe and checks its own account revision.
    CraftRequest {
        revision: u64,
        action: crate::crafting::Action,
    },
    /// Absolute host-owned balances; older revisions must not overwrite newer ones.
    CraftState {
        account: crate::crafting::Account,
        feedback: Option<String>,
    },
    CraftRegistry(crate::crafting::Registry),
    Hello {
        nickname: String,
        protocol: u32,
    },
    JoinRejected(String),
    Welcome {
        player_id: PlayerId,
        seed: u32,
        time_of_day: f32,
        spawn: [f32; 3],
        edits: Vec<((i32, i32, i32), BlockType)>,
        edit_chunks: u32,
    },
    WorldEditsChunk {
        index: u32,
        count: u32,
        edits: Vec<((i32, i32, i32), BlockType)>,
    },
    Goodbye,
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
    Notify {
        kind: NotifyKind,
        text: String,
    },
    /// A client's typed chat message, not yet attributed to a sender -- the
    /// host fills that in from the connection it arrived on (so a client
    /// can't spoof another player's identity) and relays the formatted
    /// result to everyone as `Notify`.
    ChatMessage(String),
    /// Credits `amount` of `block` into the receiving client's own
    /// Resources inventory -- the network side of `api.give_item`
    /// targeting a remote player. Sent only to that one player (unlike
    /// `Notify`, never broadcast to everyone); the host's own grant is
    /// applied locally instead of round-tripping through the network.
    GrantItem {
        block: BlockType,
        amount: u32,
    },
    /// A joined client reporting that it pressed the interact key aimed at
    /// this block position -- the network side of `on_interact`'s trigger.
    /// Client -> host only, mirroring how a client's own block break is
    /// reported via `BlockEdit` rather than run through Lua locally (Lua
    /// only ever runs on the host). Carries no block kind: the host reads
    /// the block at `(x, y, z)` itself, since it's the authoritative copy
    /// and a client's view could be stale.
    Interact {
        x: i32,
        y: i32,
        z: i32,
    },
    /// Snaps the receiving client's own player to `pos` -- the network side
    /// of `api.teleport_player` targeting a remote player. Sent only to
    /// that one player, same as `GrantItem`; the host applies its own
    /// teleport locally instead of round-tripping through the network.
    Teleport {
        pos: [f32; 3],
    },
}

/// One player's position/status as carried in a `Snapshot` -- see
/// `UnreliableMsg::Snapshot::players`. Broadcast to every connected player,
/// not just the one it belongs to (see world_api/schema.yaml's
/// `replication.player_attributes`); each client applies its own entry's
/// `health`/`poisoned`/`speed_multiplier`/`jump_multiplier` to its local
/// `Player`, and everyone else's purely for future display (nothing reads
/// another player's copy of these yet).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SnapshotPlayer {
    pub held: Option<crate::equipment::Entry>,
    pub id: PlayerId,
    pub pos: [f32; 3],
    pub yaw: f32,
    pub carrying_crystal: bool,
    pub health: f32,
    pub poisoned: bool,
    pub speed_multiplier: f32,
    pub jump_multiplier: f32,
    pub oxygen: f32,
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
        players: Vec<SnapshotPlayer>,
        /// `(pos, kind, facing, anim_clip, anim_time)` -- see
        /// `creature::Creatures::snapshot`/`AnimClip::to_u8` for what the
        /// two anim fields mean.
        creatures: Vec<([f32; 3], u8, f32, u8, f32)>,
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
    use bincode::Options;
    if bytes.len() > crate::transport::MAX_PACKET_BYTES {
        return None;
    }
    bincode::DefaultOptions::new()
        .with_fixint_encoding()
        .with_limit(crate::transport::MAX_PACKET_BYTES as u64)
        .reject_trailing_bytes()
        .deserialize(bytes)
        .ok()
}

/// Tracks outgoing reliable messages until acknowledged, and de-duplicates
/// incoming ones so they're only applied once per logical send.
pub struct ReliableChannel {
    next_id: u64,
    pending: HashMap<u64, (Instant, Peer, Vec<u8>)>,
    seen: HashMap<Peer, HashSet<u64>>,
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
    pub fn send(&mut self, socket: &Transport, addr: Peer, msg: ReliableMsg) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        let bytes = encode(&Packet::Reliable { id, msg });
        if let Err(e) = socket.send_to(&bytes, addr) {
            log::debug!("Network send to {addr}: {e}");
        }
        self.pending.insert(id, (Instant::now(), addr, bytes));
        id
    }

    /// Resends any reliable message that hasn't been acked within the resend
    /// interval.
    pub fn resend_due(&mut self, socket: &Transport) {
        let now = Instant::now();
        for (timestamp, addr, bytes) in self.pending.values_mut() {
            if now.duration_since(*timestamp) >= RELIABLE_RESEND_INTERVAL {
                let _ = socket.send_to(bytes, *addr);
                *timestamp = now;
            }
        }
    }

    pub fn ack(&mut self, id: u64, peer: Peer) {
        if self
            .pending
            .get(&id)
            .is_some_and(|(_, recipient, _)| *recipient == peer)
        {
            self.pending.remove(&id);
        }
    }
    pub fn forget_peer(&mut self, peer: Peer) {
        self.pending
            .retain(|_, (_, recipient, _)| *recipient != peer);
        self.seen.remove(&peer);
    }

    /// Stops resending a message we no longer care about (e.g. Hello once
    /// Welcome has already arrived).
    pub fn forget(&mut self, id: u64) {
        self.pending.remove(&id);
    }

    /// Returns true the first time `id` is seen from `addr`; false for
    /// repeats, so callers can skip re-applying a message they already
    /// processed.
    pub fn mark_seen(&mut self, addr: Peer, id: u64) -> bool {
        self.seen.entry(addr).or_default().insert(id)
    }

    pub fn ack_reply(socket: &Transport, addr: Peer, id: u64) {
        let bytes = encode(&Packet::Ack { id });
        if let Err(e) = socket.send_to(&bytes, addr) {
            log::debug!("Network send to {addr}: {e}");
        }
    }
}

pub const EDITS_PER_CHUNK: usize = 256;
pub const MAX_WORLD_CHUNKS: u32 = 4096;
pub fn welcome_messages(
    player_id: PlayerId,
    seed: u32,
    time_of_day: f32,
    spawn: [f32; 3],
    edits: Vec<((i32, i32, i32), BlockType)>,
) -> Result<Vec<ReliableMsg>, &'static str> {
    let count = edits.len().div_ceil(EDITS_PER_CHUNK);
    if count > MAX_WORLD_CHUNKS as usize {
        return Err("World exceeds the supported multiplayer save size");
    }
    let mut messages = vec![ReliableMsg::Welcome {
        player_id,
        seed,
        time_of_day,
        spawn,
        edits: Vec::new(),
        edit_chunks: count as u32,
    }];
    messages.extend(
        edits
            .chunks(EDITS_PER_CHUNK)
            .enumerate()
            .map(|(index, chunk)| ReliableMsg::WorldEditsChunk {
                index: index as u32,
                count: count as u32,
                edits: chunk.to_vec(),
            }),
    );
    Ok(messages)
}
pub fn join_rejection(protocol: u32, guests: usize) -> Option<&'static str> {
    if protocol != PROTOCOL_VERSION {
        Some("Game versions do not match")
    } else if guests >= MAX_PLAYERS - 1 {
        Some("Session is full (4 players maximum)")
    } else {
        None
    }
}
pub fn chat_text(text: &str) -> Option<String> {
    let text: String = text.chars().filter(|c| !c.is_control()).take(512).collect();
    let text = text.trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_owned())
    }
}
pub struct InitialWorld {
    pub player_id: PlayerId,
    pub seed: u32,
    pub time_of_day: f32,
    pub spawn: [f32; 3],
    pub edits: Vec<((i32, i32, i32), BlockType)>,
}
#[derive(Default)]
pub struct WorldTransfer {
    header: Option<InitialWorld>,
    count: Option<u32>,
    chunks: HashMap<u32, Vec<WorldEdit>>,
}
impl WorldTransfer {
    pub fn accept(&mut self, msg: ReliableMsg) -> Result<Option<InitialWorld>, String> {
        let count = match &msg {
            ReliableMsg::Welcome { edit_chunks, .. } => *edit_chunks,
            ReliableMsg::WorldEditsChunk { count, .. } => *count,
            _ => return Ok(None),
        };
        if count > MAX_WORLD_CHUNKS || self.count.is_some_and(|c| c != count) {
            return Err("Invalid world transfer size".into());
        }
        self.count = Some(count);
        match msg {
            ReliableMsg::Welcome {
                player_id,
                seed,
                time_of_day,
                spawn,
                edits,
                ..
            } => {
                if !edits.is_empty() {
                    return Err("World edits must use bounded chunks".into());
                }
                self.header = Some(InitialWorld {
                    player_id,
                    seed,
                    time_of_day,
                    spawn,
                    edits,
                });
            }
            ReliableMsg::WorldEditsChunk { index, edits, .. } => {
                if index >= count || edits.len() > EDITS_PER_CHUNK {
                    return Err("Invalid world chunk".into());
                }
                self.chunks.entry(index).or_insert(edits);
            }
            _ => unreachable!(),
        }
        if self.chunks.len() == count as usize {
            if let Some(mut header) = self.header.take() {
                for i in 0..count {
                    header.edits.extend(self.chunks.remove(&i).unwrap());
                }
                return Ok(Some(header));
            }
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::creature::{AnimClip, CreatureKind};
    use crate::weather::Weather;

    /// A World API rule's `replace_block` reaches clients exclusively as a
    /// `ReliableMsg::BlockEdit` (see world_api/schema.yaml's
    /// `replication.block_edits`) -- this pins down that the wire format
    /// actually round-trips every field, including the block kind itself.
    #[test]
    fn block_edit_round_trips_through_encode_decode() {
        let packet = Packet::Reliable {
            id: 42,
            msg: ReliableMsg::BlockEdit {
                x: -3,
                y: 12,
                z: 100,
                block: BlockType::RedStone,
            },
        };
        let bytes = encode(&packet);
        let decoded = decode(&bytes).expect("a just-encoded packet must decode");
        match decoded {
            Packet::Reliable {
                id,
                msg: ReliableMsg::BlockEdit { x, y, z, block },
            } => {
                assert_eq!(id, 42);
                assert_eq!((x, y, z), (-3, 12, 100));
                assert_eq!(block, BlockType::RedStone);
            }
            other => panic!("expected a Reliable BlockEdit packet, got {other:?}"),
        }
    }

    /// Weather, time of day, and creature state all reach clients through
    /// this one best-effort snapshot (see world_api/schema.yaml's
    /// `replication.creature_state`/`weather_and_time_of_day`) -- a rule
    /// calling api.set_weather/api.set_time_of_day/api.spawn_creature is
    /// only actually visible to clients if this shape round-trips.
    #[test]
    fn snapshot_round_trips_through_encode_decode() {
        let player = SnapshotPlayer {
            held: Some(crate::equipment::Entry::Gear(crate::equipment::Gear::Pickaxe)),
            id: 7,
            pos: [1.0, 2.0, 3.0],
            yaw: 1.57,
            carrying_crystal: true,
            health: 63.0,
            poisoned: true,
            speed_multiplier: 1.5,
            jump_multiplier: 0.8,
            oxygen: 42.0,
        };
        let packet = Packet::Unreliable(UnreliableMsg::Snapshot {
            time_of_day: 0.42,
            weather: Weather::Rain.to_u8(),
            players: vec![player],
            creatures: vec![(
                [4.0, 5.0, 6.0],
                CreatureKind::Chicken.to_u8(),
                0.5,
                AnimClip::Walk.to_u8(),
                1.2,
            )],
        });
        let bytes = encode(&packet);
        let decoded = decode(&bytes).expect("a just-encoded packet must decode");
        match decoded {
            Packet::Unreliable(UnreliableMsg::Snapshot {
                time_of_day,
                weather,
                players,
                creatures,
            }) => {
                assert_eq!(time_of_day, 0.42);
                assert_eq!(weather, Weather::Rain.to_u8());
                assert_eq!(players, vec![player]);
                assert_eq!(
                    creatures,
                    vec![(
                        [4.0, 5.0, 6.0],
                        CreatureKind::Chicken.to_u8(),
                        0.5,
                        AnimClip::Walk.to_u8(),
                        1.2,
                    )]
                );
            }
            other => panic!("expected an Unreliable Snapshot packet, got {other:?}"),
        }
    }

    /// api.broadcast reaches clients as a `Notify` (see world_api/schema.yaml's
    /// `replication.broadcast_messages`) -- confirms the message text and
    /// its styling both survive the wire.
    #[test]
    fn notify_round_trips_through_encode_decode() {
        let packet = Packet::Reliable {
            id: 1,
            msg: ReliableMsg::Notify {
                kind: NotifyKind::Important,
                text: "a rule broadcast this".to_string(),
            },
        };
        let bytes = encode(&packet);
        let decoded = decode(&bytes).expect("a just-encoded packet must decode");
        match decoded {
            Packet::Reliable {
                msg: ReliableMsg::Notify { kind, text },
                ..
            } => {
                assert!(matches!(kind, NotifyKind::Important));
                assert_eq!(text, "a rule broadcast this");
            }
            other => panic!("expected a Reliable Notify packet, got {other:?}"),
        }
    }

    /// api.give_item targeting a remote player reaches that client as a
    /// `GrantItem` (see world_api/schema.yaml's `replication.item_grants`)
    /// -- confirms the block kind and amount both survive the wire.
    #[test]
    fn grant_item_round_trips_through_encode_decode() {
        let packet = Packet::Reliable {
            id: 5,
            msg: ReliableMsg::GrantItem {
                block: BlockType::Stone,
                amount: 100,
            },
        };
        let bytes = encode(&packet);
        let decoded = decode(&bytes).expect("a just-encoded packet must decode");
        match decoded {
            Packet::Reliable {
                msg: ReliableMsg::GrantItem { block, amount },
                ..
            } => {
                assert_eq!(block, BlockType::Stone);
                assert_eq!(amount, 100);
            }
            other => panic!("expected a Reliable GrantItem packet, got {other:?}"),
        }
    }

    /// A joined client's interact-key press reaches the host as an
    /// `Interact` (see world_api/schema.yaml's `on_interact`) -- confirms
    /// the target position survives the wire.
    #[test]
    fn interact_round_trips_through_encode_decode() {
        let packet = Packet::Reliable {
            id: 9,
            msg: ReliableMsg::Interact {
                x: 10,
                y: -2,
                z: 30,
            },
        };
        let bytes = encode(&packet);
        let decoded = decode(&bytes).expect("a just-encoded packet must decode");
        match decoded {
            Packet::Reliable {
                msg: ReliableMsg::Interact { x, y, z },
                ..
            } => {
                assert_eq!((x, y, z), (10, -2, 30));
            }
            other => panic!("expected a Reliable Interact packet, got {other:?}"),
        }
    }

    /// api.teleport_player targeting a remote player reaches that client as
    /// a `Teleport` (see world_api/schema.yaml's `replication`) -- confirms
    /// the destination survives the wire.
    #[test]
    fn teleport_round_trips_through_encode_decode() {
        let packet = Packet::Reliable {
            id: 6,
            msg: ReliableMsg::Teleport {
                pos: [1.5, 64.0, -8.25],
            },
        };
        let bytes = encode(&packet);
        let decoded = decode(&bytes).expect("a just-encoded packet must decode");
        match decoded {
            Packet::Reliable {
                msg: ReliableMsg::Teleport { pos },
                ..
            } => {
                assert_eq!(pos, [1.5, 64.0, -8.25]);
            }
            other => panic!("expected a Reliable Teleport packet, got {other:?}"),
        }
    }

    /// Garbage bytes (e.g. a truncated or corrupted UDP datagram) must fail
    /// to decode rather than panicking or fabricating a packet.
    #[test]
    fn decode_rejects_garbage_bytes() {
        assert!(decode(&[1, 2, 3, 4, 5]).is_none());
    }
}

pub const DEFAULT_LLM_URL: &str = "http://127.0.0.1:8090";

pub struct LaunchConfig {
    /// Join this host instead of hosting our own game.
    pub connect: Option<JoinTarget>,
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
                    match value.parse::<JoinTarget>() {
                        Ok(addr) => connect = Some(addr),
                        Err(_) => {
                            eprintln!("Invalid --connect address '{value}', expected ip:port or steam:lobby_id");
                            std::process::exit(1);
                        }
                    }
                    i += 1;
                }
            }
            "+connect_lobby" => {
                if let Some(value) = args.get(i + 1) {
                    connect = format!("steam:{value}").parse().ok();
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

#[cfg(test)]
mod multiplayer_tests {
    use super::*;
    fn receive(socket: &Transport) -> (Packet, Peer) {
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut bytes = [0; crate::transport::MAX_PACKET_BYTES];
        loop {
            match socket.recv_from(&mut bytes) {
                Ok((n, peer)) => return (decode(&bytes[..n]).unwrap(), peer),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(Instant::now() < deadline, "local packet timed out");
                    std::thread::sleep(Duration::from_millis(1));
                }
                Err(e) => panic!("{e}"),
            }
        }
    }
    #[test]
    fn four_local_peers_exchange_chat_and_block_edits() {
        let host = Transport::direct("127.0.0.1:0").unwrap();
        let guests: Vec<_> = (0..3)
            .map(|_| Transport::direct("127.0.0.1:0").unwrap())
            .collect();
        let mut reliable = ReliableChannel::new();
        let mut members = Vec::new();
        for guest in &guests {
            guest
                .send_to(
                    &encode(&Packet::Reliable {
                        id: 1,
                        msg: ReliableMsg::Hello {
                            nickname: "Guest".into(),
                            protocol: PROTOCOL_VERSION,
                        },
                    }),
                    host.local_peer(),
                )
                .unwrap();
            let (
                Packet::Reliable {
                    msg: ReliableMsg::Hello { protocol, .. },
                    ..
                },
                peer,
            ) = receive(&host)
            else {
                panic!("hello")
            };
            assert!(join_rejection(protocol, members.len()).is_none());
            members.push(peer);
        }
        assert!(join_rejection(PROTOCOL_VERSION, members.len())
            .unwrap()
            .contains("full"));
        assert!(join_rejection(PROTOCOL_VERSION + 1, 0).is_some());
        for sender in &guests {
            sender
                .send_to(
                    &encode(&Packet::Reliable {
                        id: 2,
                        msg: ReliableMsg::ChatMessage(" hello\n friends ".into()),
                    }),
                    host.local_peer(),
                )
                .unwrap();
            let (
                Packet::Reliable {
                    msg: ReliableMsg::ChatMessage(text),
                    ..
                },
                peer,
            ) = receive(&host)
            else {
                panic!("chat")
            };
            assert!(members.contains(&peer));
            for guest in &guests {
                reliable.send(
                    &host,
                    guest.local_peer(),
                    ReliableMsg::Notify {
                        kind: NotifyKind::Chat,
                        text: chat_text(&text).unwrap(),
                    },
                );
            }
            for guest in &guests {
                let (
                    Packet::Reliable {
                        id,
                        msg:
                            ReliableMsg::Notify {
                                kind: NotifyKind::Chat,
                                text,
                            },
                    },
                    peer,
                ) = receive(guest)
                else {
                    panic!("relay")
                };
                assert_eq!(text, "hello friends");
                assert_eq!(peer, host.local_peer());
                ReliableChannel::ack_reply(guest, peer, id);
                let (Packet::Ack { id }, peer) = receive(&host) else {
                    panic!("ack")
                };
                reliable.ack(id, peer);
            }
        }
        assert!(reliable.pending.is_empty());
        for guest in &guests {
            reliable.send(
                &host,
                guest.local_peer(),
                ReliableMsg::BlockEdit {
                    x: 1,
                    y: 2,
                    z: 3,
                    block: BlockType::Stone,
                },
            );
            assert!(matches!(
                receive(guest).0,
                Packet::Reliable {
                    msg: ReliableMsg::BlockEdit {
                        x: 1,
                        y: 2,
                        z: 3,
                        block: BlockType::Stone
                    },
                    ..
                }
            ));
        }
    }
    #[test]
    fn acknowledgements_are_bound_to_recipient_and_reconnect_resets_duplicates() {
        let socket = Transport::direct("127.0.0.1:0").unwrap();
        let guest = Transport::direct("127.0.0.1:0").unwrap();
        let peer = guest.local_peer();
        let mut reliable = ReliableChannel::new();
        let id = reliable.send(&socket, peer, ReliableMsg::Goodbye);
        reliable.ack(id, socket.local_peer());
        assert!(reliable.pending.contains_key(&id));
        assert!(reliable.mark_seen(peer, 1));
        assert!(!reliable.mark_seen(peer, 1));
        reliable.forget_peer(peer);
        assert!(reliable.pending.is_empty());
        assert!(reliable.mark_seen(peer, 1));
    }
    #[test]
    fn large_world_chunks_survive_reordering_and_duplicates() {
        let edits: Vec<_> = (0..20_000)
            .map(|x| ((x, 50, 0), BlockType::Stone))
            .collect();
        let mut messages = welcome_messages(3, 42, 0.5, [1., 2., 3.], edits.clone()).unwrap();
        let header = messages.remove(0);
        let mut transfer = WorldTransfer::default();
        for msg in messages.into_iter().rev() {
            let bytes = encode(&Packet::Reliable {
                id: 1,
                msg: msg.clone(),
            });
            assert!(bytes.len() < crate::transport::MAX_PACKET_BYTES);
            assert!(decode(&bytes).is_some());
            assert!(transfer.accept(msg.clone()).unwrap().is_none());
            assert!(transfer.accept(msg).unwrap().is_none());
        }
        let world = transfer.accept(header).unwrap().unwrap();
        assert_eq!(world.edits, edits);
        assert_eq!(world.player_id, 3);
        assert_eq!(world.seed, 42);
    }
    #[test]
    fn malformed_transfers_and_packets_are_rejected() {
        assert!(WorldTransfer::default()
            .accept(ReliableMsg::WorldEditsChunk {
                index: 2,
                count: 1,
                edits: vec![]
            })
            .is_err());
        assert!(WorldTransfer::default()
            .accept(ReliableMsg::WorldEditsChunk {
                index: 0,
                count: MAX_WORLD_CHUNKS + 1,
                edits: vec![]
            })
            .is_err());
        let mut bytes = encode(&Packet::Ack { id: 1 });
        bytes.push(0);
        assert!(decode(&bytes).is_none());
        assert!(decode(&vec![0; crate::transport::MAX_PACKET_BYTES + 1]).is_none());
        assert_eq!(chat_text("\n\t "), None);
        assert_eq!(chat_text(&"x".repeat(900)).unwrap().len(), 512);
    }
    #[test]
    fn steam_accounts_cannot_be_claimed_by_direct_nicknames() {
        let direct = Peer::Direct("127.0.0.1:1".parse().unwrap());
        assert_ne!(
            direct.account_key("steam:123"),
            Peer::Steam(123).account_key("Player")
        );
        assert_eq!(
            Peer::Steam(123).account_key("Before"),
            Peer::Steam(123).account_key("After")
        );
        assert_ne!(
            Peer::Steam(123).account_key("Same"),
            Peer::Steam(124).account_key("Same")
        );
        assert_eq!(
            "steam:480".parse::<JoinTarget>().unwrap(),
            JoinTarget::SteamLobby(480)
        );
        assert!("steam:0".parse::<JoinTarget>().is_err());
    }
}
