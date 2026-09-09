//! Steam lobbies provide membership; NetworkingMessages carries the existing game protocol.
use std::{
    collections::HashSet,
    io,
    sync::{mpsc, Arc, Mutex},
    time::{Duration, Instant},
};
use steamworks::networking_types::{NetworkingIdentity, SendFlags};
use steamworks::{Client, LobbyId, LobbyType, SteamId};

const GAME_KEY: &str = "voxel_project";
const CHANNEL: u32 = 0;
static INVITE: Mutex<Option<u64>> = Mutex::new(None);
struct Runtime {
    client: Client,
    app_id: u32,
    _invite: steamworks::CallbackHandle,
}
thread_local! { static RUNTIME: std::cell::RefCell<Option<Runtime>> = const { std::cell::RefCell::new(None) }; }
pub fn initialize(app_id: u32) -> Result<Client, String> {
    RUNTIME.with(|slot| {
        let mut slot = slot.borrow_mut();
        if let Some(runtime) = slot.as_ref() {
            if runtime.app_id != app_id {
                return Err("Restart the game after changing the Steam App ID".into());
            }
            return Ok(runtime.client.clone());
        }
        let client = Client::init_app(app_id)
            .map_err(|e| format!("Steam initialization failed: {e}. Start Steam and sign in."))?;
        let invite = client.register_callback(|event: steamworks::GameLobbyJoinRequested| {
            *INVITE.lock().unwrap() = Some(event.lobby_steam_id.raw());
        });
        *slot = Some(Runtime {
            client: client.clone(),
            app_id,
            _invite: invite,
        });
        Ok(client)
    })
}
pub fn poll_runtime() {
    RUNTIME.with(|slot| {
        if let Some(runtime) = slot.borrow().as_ref() {
            runtime.client.run_callbacks();
        }
    });
}
pub fn pending_invite() -> Option<u64> {
    *INVITE.lock().unwrap()
}
pub fn take_invite() -> Option<u64> {
    INVITE.lock().unwrap().take()
}

pub struct SteamTransport {
    client: Client,
    lobby: LobbyId,
    host: u64,
    allowed: Arc<Mutex<HashSet<u64>>>,
}
impl SteamTransport {
    pub fn open(lobby: Option<u64>, app_id: u32) -> Result<Self, String> {
        if app_id == 0 {
            return Err("Steam App ID must be nonzero".into());
        }
        let client = initialize(app_id)?;
        client.networking_utils().init_relay_network_access();
        let (tx, rx) = mpsc::channel();
        let cleanup = client.clone();
        if let Some(id) = lobby {
            client
                .matchmaking()
                .join_lobby(LobbyId::from_raw(id), move |result| {
                    if let Err(mpsc::SendError(Ok(late))) = tx.send(result.map_err(|_| {
                        "Cannot join lobby: it may be full, private, or closed".to_string()
                    })) {
                        cleanup.matchmaking().leave_lobby(late);
                    }
                });
        } else {
            client.matchmaking().create_lobby(
                LobbyType::FriendsOnly,
                crate::net::MAX_PLAYERS as u32,
                move |result| {
                    if let Err(mpsc::SendError(Ok(late))) =
                        tx.send(result.map_err(|e| format!("Cannot create Steam lobby: {e}")))
                    {
                        cleanup.matchmaking().leave_lobby(late);
                    }
                },
            );
        }
        let started = Instant::now();
        let joined = loop {
            client.run_callbacks();
            if let Ok(result) = rx.try_recv() {
                break result?;
            }
            if started.elapsed() > Duration::from_secs(15) {
                return Err("Steam lobby request timed out".into());
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        let mm = client.matchmaking();
        if lobby.is_none()
            && (!mm.set_lobby_data(joined, "game", GAME_KEY)
                || !mm.set_lobby_data(
                    joined,
                    "protocol",
                    &crate::net::PROTOCOL_VERSION.to_string(),
                )
                || !mm.set_lobby_data(joined, "host", &client.user().steam_id().raw().to_string()))
        {
            mm.leave_lobby(joined);
            return Err("Failed to publish Steam lobby metadata".into());
        }
        let host = mm.lobby_owner(joined).raw();
        if mm.lobby_data(joined, "game").as_deref() != Some(GAME_KEY)
            || mm.lobby_data(joined, "protocol") != Some(crate::net::PROTOCOL_VERSION.to_string())
            || mm.lobby_data(joined, "host") != Some(host.to_string())
        {
            mm.leave_lobby(joined);
            return Err("Lobby belongs to another game/build, or its host has left".into());
        }
        let allowed = Arc::new(Mutex::new(HashSet::new()));
        let gate = allowed.clone();
        client
            .networking_messages()
            .session_request_callback(move |request| {
                if request
                    .remote()
                    .steam_id()
                    .is_some_and(|id| gate.lock().unwrap().contains(&id.raw()))
                {
                    request.accept();
                }
            });
        client
            .networking_messages()
            .session_failed_callback(|info| {
                log::warn!("Steam networking session failed: {:?}", info);
            });
        let result = Self {
            client,
            lobby: joined,
            host,
            allowed,
        };
        result.refresh_members();
        Ok(result)
    }
    fn refresh_members(&self) {
        let me = self.client.user().steam_id().raw();
        let members = self.client.matchmaking().lobby_members(self.lobby);
        let mut allowed = self.allowed.lock().unwrap();
        allowed.clear();
        if self.client.matchmaking().lobby_owner(self.lobby).raw() != self.host {
            return;
        }
        allowed.extend(
            members
                .into_iter()
                .map(|m| m.raw())
                .filter(|id| *id != me && (me == self.host || *id == self.host)),
        );
    }
    pub fn host_id(&self) -> u64 {
        self.host
    }
    pub fn lobby_id(&self) -> u64 {
        self.lobby.raw()
    }
    pub fn peer_name(&self, id: u64) -> String {
        self.client
            .friends()
            .get_friend(SteamId::from_raw(id))
            .name()
    }
    pub fn invite_friends(&self) {
        self.client.friends().activate_invite_dialog(self.lobby);
    }
    pub fn send(&self, bytes: &[u8], id: u64) -> io::Result<usize> {
        self.refresh_members();
        if !self.allowed.lock().unwrap().contains(&id) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Peer is not a member of this session",
            ));
        }
        let flags = match crate::net::decode(bytes) {
            Some(crate::net::Packet::Unreliable(_)) => SendFlags::UNRELIABLE_NO_NAGLE,
            _ => SendFlags::RELIABLE,
        };
        let mut envelope = self.lobby.raw().to_le_bytes().to_vec();
        envelope.extend_from_slice(bytes);
        self.client
            .networking_messages()
            .send_message_to_user(
                NetworkingIdentity::new_steam_id(SteamId::from_raw(id)),
                flags | SendFlags::AUTO_RESTART_BROKEN_SESSION,
                &envelope,
                CHANNEL,
            )
            .map_err(|e| io::Error::other(format!("Steam send: {e}")))?;
        Ok(bytes.len())
    }
    pub fn receive(&self, bytes: &mut [u8]) -> io::Result<(usize, u64)> {
        self.refresh_members();
        self.client.run_callbacks();
        if self.client.matchmaking().lobby_owner(self.lobby).raw() != self.host {
            return Err(io::Error::new(
                io::ErrorKind::ConnectionAborted,
                "Steam host left; session ended",
            ));
        }
        for _ in 0..32 {
            let Some(msg) = self
                .client
                .networking_messages()
                .receive_messages_on_channel(CHANNEL, 1)
                .pop()
            else {
                break;
            };
            let Some(id) = msg.identity_peer().steam_id().map(|s| s.raw()) else {
                continue;
            };
            if !self.allowed.lock().unwrap().contains(&id) {
                continue;
            }
            let envelope = msg.data();
            if envelope.len() < 8 || envelope[..8] != self.lobby.raw().to_le_bytes() {
                continue;
            }
            let data = &envelope[8..];
            if data.len() > bytes.len() || data.len() > crate::transport::MAX_PACKET_BYTES {
                continue;
            }
            bytes[..data.len()].copy_from_slice(data);
            return Ok((data.len(), id));
        }
        Err(io::ErrorKind::WouldBlock.into())
    }
}
impl Drop for SteamTransport {
    fn drop(&mut self) {
        self.allowed.lock().unwrap().clear();
        if self.client.user().steam_id().raw() == self.host {
            self.client
                .matchmaking()
                .set_lobby_joinable(self.lobby, false);
        }
        self.client.matchmaking().leave_lobby(self.lobby);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "Requires a signed-in Steam client; creates then leaves a test lobby"]
    fn live_test_app_lobby() {
        let transport = super::SteamTransport::open(None, 480).expect("Steam test lobby");
        assert_ne!(transport.lobby_id(), 0);
        assert_eq!(
            transport.host_id(),
            transport.client.user().steam_id().raw()
        );
        assert_eq!(
            transport
                .client
                .matchmaking()
                .lobby_data(transport.lobby, "game")
                .as_deref(),
            Some(super::GAME_KEY)
        );
    }
}
