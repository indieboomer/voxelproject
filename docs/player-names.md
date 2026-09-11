# Player names

Settings [F10] has a Player name field and an AI suggestion button. Empty names
start an asynchronous request to the configured local model. If unavailable or
its response is invalid, a clearly labeled generated fantasy fallback is used.
Requests time out after 12 seconds; players can type their own name while waiting.
AI names are cosmetic and do not spend in-game mana.

The main menu and New World / Load World dialogs do not request AI names or
display suggestion messages. These dialogs prefill the saved player name once
when opened, leaving subsequent edits alone. Menu suggestions run only in
Settings or while joining multiplayer.

The multiplayer join nickname dialog uses the settings name, or shows the new
suggestion when it arrives. Names are limited to 24 characters. Settings edits
update the current session's visible name. The first menu launch stores a stable
connection name separately, so changing the display name does not change the
direct-session inventory key. Existing users should initially use their old
nickname to retain their previous direct-session inventory. Steam account identity
continues to use Steam's stable identifier.

Other players' names appear above their models within 48 blocks, hidden when
behind terrain, behind the camera or outside the screen. Display names are sent
in authoritative multiplayer snapshots. Local players do not see their own tag.
