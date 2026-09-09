//! Transport-neutral peers keep Steam identities separate from UDP addresses.
use crate::settings::MultiplayerMode;
use std::{
    fmt, io,
    net::{SocketAddr, UdpSocket},
    str::FromStr,
};

pub const MAX_PACKET_BYTES: usize = 60_000;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Peer {
    Direct(SocketAddr),
    #[cfg_attr(not(feature = "steam"), allow(dead_code))]
    Steam(u64),
}
impl fmt::Display for Peer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Direct(addr) => write!(f, "{addr}"),
            Self::Steam(id) => write!(f, "steam:{id}"),
        }
    }
}
impl Peer {
    pub fn account_key(self, nickname: &str) -> String {
        match self {
            Self::Direct(_) => format!("direct:{nickname}"),
            Self::Steam(id) => format!("steam:{id}"),
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinTarget {
    Direct(SocketAddr),
    SteamLobby(u64),
}
impl fmt::Display for JoinTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Direct(addr) => write!(f, "{addr}"),
            Self::SteamLobby(id) => write!(f, "steam:{id}"),
        }
    }
}
impl FromStr for JoinTarget {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        if let Some(id) = s.trim().strip_prefix("steam:") {
            let id: u64 = id.parse().map_err(|_| "Invalid Steam lobby ID")?;
            if id == 0 {
                return Err("Invalid Steam lobby ID".into());
            }
            Ok(Self::SteamLobby(id))
        } else {
            s.trim()
                .parse()
                .map(Self::Direct)
                .map_err(|_| "Enter ip:port or steam:lobby_id".into())
        }
    }
}

pub enum Transport {
    Direct(UdpSocket),
    #[cfg(feature = "steam")]
    Steam(crate::steam_transport::SteamTransport),
}
impl Transport {
    pub fn direct(addr: &str) -> io::Result<Self> {
        let socket = UdpSocket::bind(addr)?;
        socket.set_nonblocking(true)?;
        Ok(Self::Direct(socket))
    }
    pub fn host(mode: MultiplayerMode, port: u16, app_id: u32) -> Result<Self, String> {
        match mode {
            MultiplayerMode::Direct => {
                Self::direct(&format!("0.0.0.0:{port}")).map_err(|e| e.to_string())
            }
            MultiplayerMode::Steam => Self::steam(None, app_id),
        }
    }
    pub fn join(target: JoinTarget, app_id: u32) -> Result<(Self, Peer), String> {
        match target {
            JoinTarget::Direct(addr) => Ok((
                Self::direct(if addr.is_ipv6() {
                    "[::]:0"
                } else {
                    "0.0.0.0:0"
                })
                .map_err(|e| e.to_string())?,
                Peer::Direct(addr),
            )),
            JoinTarget::SteamLobby(id) => {
                let socket = Self::steam(Some(id), app_id)?;
                let peer = socket.steam_host().ok_or("Steam lobby has no host")?;
                Ok((socket, peer))
            }
        }
    }
    fn steam(lobby: Option<u64>, app_id: u32) -> Result<Self, String> {
        #[cfg(feature = "steam")]
        {
            crate::steam_transport::SteamTransport::open(lobby, app_id).map(Self::Steam)
        }
        #[cfg(not(feature = "steam"))]
        {
            let _ = (lobby, app_id);
            Err("Steam support is not included in this build. Choose Direct in Settings or use the Steam build.".into())
        }
    }
    fn steam_host(&self) -> Option<Peer> {
        match self {
            #[cfg(feature = "steam")]
            Self::Steam(s) => Some(Peer::Steam(s.host_id())),
            _ => None,
        }
    }
    pub fn lobby_code(&self) -> Option<String> {
        match self {
            #[cfg(feature = "steam")]
            Self::Steam(s) => Some(format!("steam:{}", s.lobby_id())),
            _ => None,
        }
    }
    pub fn invite_friends(&self) {
        #[cfg(feature = "steam")]
        if let Self::Steam(s) = self {
            s.invite_friends();
        }
    }
    pub fn peer_name(&self, peer: Peer) -> Option<String> {
        match (self, peer) {
            #[cfg(feature = "steam")]
            (Self::Steam(s), Peer::Steam(id)) => Some(s.peer_name(id)),
            _ => None,
        }
    }
    pub fn send_to(&self, bytes: &[u8], peer: Peer) -> io::Result<usize> {
        if bytes.len() > MAX_PACKET_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Packet exceeds transport budget",
            ));
        }
        match (self, peer) {
            (Self::Direct(s), Peer::Direct(addr)) => s.send_to(bytes, addr),
            #[cfg(feature = "steam")]
            (Self::Steam(s), Peer::Steam(id)) => s.send(bytes, id),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Peer belongs to another transport",
            )),
        }
    }
    pub fn recv_from(&self, bytes: &mut [u8]) -> io::Result<(usize, Peer)> {
        match self {
            Self::Direct(s) => s.recv_from(bytes).map(|(n, addr)| (n, Peer::Direct(addr))),
            #[cfg(feature = "steam")]
            Self::Steam(s) => s.receive(bytes).map(|(n, id)| (n, Peer::Steam(id))),
        }
    }
    #[cfg(test)]
    pub fn local_peer(&self) -> Peer {
        match self {
            Self::Direct(s) => Peer::Direct(s.local_addr().unwrap()),
            #[cfg(feature = "steam")]
            Self::Steam(_) => panic!("Use a Direct transport in local tests"),
        }
    }
}
