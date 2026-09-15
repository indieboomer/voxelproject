# Steam multiplayer development

For a distributable release with Steam, the AI runtime and a per-user installer,
run `make_package.bat`; see [PACKAGING.md](../PACKAGING.md).

Steam support uses **test App ID 480 (Spacewar)** by default. Up to four players share one host-owned world. The default transport remains Direct / LAN; select Steam friends in Settings before hosting a new session. The setting persists and applies to the next session.

## Implementation plan and status

1. Preserve gameplay behind a transport interface: implemented. Direct UDP and Steam share the existing authoritative host, snapshots, block changes, crafting, Lua effects and chat protocol.
2. Add Steam identity and session discovery: implemented with optional `steamworks` 0.13.1, friends-only four-member lobbies, lobby codes, native invite overlay and Steam invite callbacks.
3. Harden joining and lifecycle: implemented protocol checks, four-player admission, world transfer chunks, sender-bound acknowledgements, departure messages, timeouts and host-loss handling.
4. Validate locally: automated protocol/localhost tests implemented. Live Steam smoke test requires a signed-in client and is explicitly opt-in.
5. Validate with friends: the separate-account matrix below remains required before declaring Steam multiplayer verified end to end.

## Windows build and use

From the repository directory:

Use `build.bat steam` (or `build.bat steam release`) from Command Prompt. Plain `build.bat` and `cargo build` produce Direct-only builds, where the Steam option is disabled. Restart the game after rebuilding.

```powershell
.\tools\build_steam.ps1
.\target\debug\voxelproject.exe
```

Use `-Release` for a release build. The script copies the bundled Steam API redistributable beside the executable. It uses the standard Cargo output directory, so a later default build replaces that executable with the Direct-only build. Run from the repository directory to use its data, modules, settings and saves. A distributable folder also needs those runtime data directories; this script builds a runnable development executable, not an installer.

Every player needs the same Steam-enabled game build, Steam running and a signed-in, distinct Steam account. Host: choose Settings > Multiplayer > Steam friends, then New World or Load World. Press Escape to copy the `steam:LOBBY_ID` code or open Invite Steam friends. Friends choose Join Steam Lobby and enter the code (plain numeric IDs also work when Steam is selected). Friends-only membership still applies when using a code.

Steam overlay invites depend on the overlay being available for the game. Code joining is available independently. An accepted invite in the menu opens the join/nickname flow. During play, an invitation shows a notification; return to the main menu to join. `--connect steam:LOBBY_ID` and Steam's `+connect_lobby LOBBY_ID` launch argument are supported. Steam launching an unregistered development executable through App ID 480 is not configured by this repository; start the build manually and use a code if necessary.

For Direct: select Direct / LAN and host, then join `ip:7878` (or the selected port). Direct uses the existing UDP implementation and may require router/firewall configuration outside a LAN. A build without Steam needs neither a Steam account nor its DLL:

```powershell
cargo build
cargo test --offline
```

## Design choices

Steam lobbies supply membership and invitations; SteamNetworkingMessages carries game packets and supplies authenticated Steam identities plus Steam's networking/relay facilities. It avoids requiring friends to enter a public IP. Reliable game messages and chat use reliable Steam sends; snapshots use unreliable sends. The existing acknowledgement/deduplication layer is retained for transport parity. Only lobby members are accepted, clients accept world state from the original host, and each message carries its lobby ID to reject traffic from old sessions. No host migration: the session ends when its original host leaves.

A shared Steam test App ID contains unrelated games. Lobby metadata identifies this game, protocol version and original host, and joining rejects mismatches. App ID 480 is for development; replace `multiplayer.steam_app_id` in `settings.json` with the assigned product ID and restart before configuring a real Steam release.

Steam guest crafting inventories persist by authenticated Steam ID, independent of display name. Direct guest inventories use a separate nickname namespace; ordinary legacy nickname accounts migrate on join. Legacy nicknames beginning `steam:` or `direct:` are not automatically migrated because they overlap the reserved namespaces. Direct nickname identity remains unauthenticated, as before.

Saved edits transfer in chunks of 256 rather than one oversized UDP packet. The current supported join limit is 1,048,576 saved edits. All peers must use the same protocol/build. Host-owned Lua, crafting and chat handlers remain shared between transports. World loading is synchronous with bounded lobby/join timeouts; there is no host migration, public browser or matchmaking service.

The Rust wrapper was chosen because it exposes both Steam lobbies and SteamNetworkingMessages. Standalone GameNetworkingSockets would still require a separate Steam identity/lobby integration. References: [Valve lobby documentation](https://partner.steamgames.com/doc/features/multiplayer/matchmaking), [Steamworks Rust NetworkingMessages API](https://docs.rs/steamworks/latest/steamworks/networking_messages/struct.NetworkingMessages.html), [Valve GameNetworkingSockets](https://github.com/ValveSoftware/GameNetworkingSockets).

## Verification

```powershell
cargo test --offline
cargo test --offline --features steam
# Explicitly creates then leaves one friends-only App ID 480 lobby; sends no invitations:
cargo test --offline --features steam live_test_app_lobby -- --ignored --nocapture
```

Automated checks cover three local guests plus a host, chat fanout, block-edit delivery, admission/version checks, acknowledgement ownership, duplicate/reconnect bookkeeping, malformed packets, identity namespaces and reordered/duplicate large-world chunks. These exercise real local UDP sockets and shared protocol components; they do not simulate Steam's network or replace a graphical four-player session.

Before accepting the feature, run the following with distinct Steam accounts, including two different internet connections:

- Host plus one guest, then three guests: join by code and native invite; reject a fifth player and wrong-build lobby.
- Move, run, jump, break/place blocks and observe the same creatures, weather and time on all peers.
- Every player sends chat, including Unicode; verify one attributed message reaches every participant.
- Craft and extract resources; activate and disable host Lua rules; verify inventories and world consequences match.
- Join a substantially edited save; reconnect with the same Steam account after changing its display name; verify inventory persistence.
- Disconnect a guest, reconnect, and fill its slot with another account. Quit/crash the host; verify guests return to the menu and the old lobby cannot become a migrated game.
- Repeat host/join using Direct / LAN; verify Generic/Fantasy settings and chat UI in both modes.

Validation on 2026-09-09: Direct and Steam automated suites pass; the live App ID 480 smoke test passed outside the filesystem sandbox using the signed-in Steam client. It created, validated and left one lobby. No separate-account gameplay test has been performed. Strict Clippy is blocked by existing project warnings (renderer/model, scripting, voxel helpers and UI signatures).
