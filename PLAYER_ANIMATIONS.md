# Player gestures, actions, and held items

While controlling your character, press:

| Key | Gesture |
| --- | --- |
| `,` | Dance |
| `.` | Angry |
| `/` | Jump gesture |

Gestures play once. Press again to restart or choose another gesture. Walking
cancels dance/angry; using an item or interacting replaces the current gesture.
Holding a gesture key does not repeatedly restart it. Chat, the rule console,
inventory, crafting, and settings consume their own input without triggering gestures.
The slash gesture is cosmetic; **Space** still performs the physical jump.

Other players see idle, walk, and run animations based on movement, jump while
airborne, attack when swinging a sword, and work when mining, placing blocks,
interacting with F, or submitting crafting actions. These are visual feedback for
an attempted action; the host still validates whether it changes the world.
The first-person camera remains in first person; gestures animate the character
seen by other players.

The selected tool or resource follows the character's animated `grip_R` socket,
including its rotation during work, attack, walking, and gestures. Resources are
centered at the grip and tools are aligned by their handle. Empty/depleted slots
show no extra held mesh. Hats continue following their own animated socket.

Multiplayer protocol **21** carries the animation clip, playback time, and restart
sequence in player state and host snapshots. The host relays each guest's cosmetic
state and broadcasts its own state. Repeated actions restart even when the clip
name stays the same; stale or invalid animation samples are ignored. Every peer
must use the same build. Animation playback is transient and is not saved.

No World API additions are needed for these player controls. Existing interaction,
block, and combat APIs/events retain their gameplay semantics; cosmetic gestures
do not execute Lua or grant world-editing capabilities.

Validation covers gesture key edges, animation transitions, repeated/reordered
network state, serialized snapshots, and all four models' animated hand attachments
across all supported clips and several facing directions.

## Overhead chat

Sending chat with T also displays the message above your character for other players. Each player has one bubble; a new message replaces it and restarts the four-second timer. The bubble fades during the last 0.75 seconds, follows the character above the nameplate, and is hidden beyond 48 blocks or behind terrain. Normal chat history and notifications remain available. The host attaches the sender ID to chat, so duplicate or changed display names do not move a bubble to the wrong player. Bubbles are transient and are not saved.

## Chat emoticons

Open chat with **T**, then click an icon beneath the input to append it to your draft.
Press **Enter** to send. You can also type the following shortcodes:

| Emoticon | Shortcode | Text shortcut |
| --- | --- | --- |
| Smile | `:smile:` | `:)`, `:-)` |
| Laugh | `:laugh:` | `:D` |
| Wink | `:wink:` | `;)`, `;-)` |
| Sad | `:sad:` | `:(`, `:-(` |
| Angry | `:angry:` | `>:(` |
| Surprised | `:surprised:` | `:O` |
| Heart | `:heart:` | `<3` |
| Thumbs-up | `:thumbsup:` | — |

Text shortcuts must stand separately from surrounding words; unknown shortcodes
stay as text. Icons appear inline in chat history, notifications, and overhead
bubbles, including wrapped lines and fading bubbles. Drafts retain editable text
shortcodes. The picker respects the existing 512-character message limit.

The eight icons are original vector artwork implemented in `src/emoticons.rs`.
No third-party artwork, external downloads, or platform emoji fonts are used.
Messages still travel as ordinary text through protocol 21; peers with this build
render the icons, while older rendering code shows the shortcodes as text.