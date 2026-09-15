# Windows distribution

Double-click `make_package.bat`, or run it from Command Prompt. It builds a locked
release with Steam and Direct/LAN support, then creates a timestamped folder and
ZIP64 archive in `dist/`. The full package includes the currently configured
Qwen 2.5 Coder 7B Q4_K_M GGUF, llama-server, all its DLL backends, the MSVC runtime,
runtime data, starter Lua modules, fresh settings, and an installer/launcher.
Art, textures, sounds, prompts and World API documentation are compiled into the
game executable; their development directories are unnecessary on the recipient PC.
Personal saves, settings, server logs, Rust tools and source code are excluded.

```bat
make_package.bat
make_package.bat -Offline
make_package.bat -DirectOnly
make_package.bat -NoAI
make_package.bat -NoZip
make_package.bat -VerifyOnly
```

Options can be combined. `-NoAI` makes a smaller client package; it can host
ordinary sessions but needs a separate local inference server for rule generation.
`-RuntimeDirectory "D:\AI\llm-runtime"` overrides the source runtime folder.
`-CrtDirectory "...\x64\Microsoft.VC145.CRT"` overrides automatic Visual Studio
redistributable discovery. Full packaging expects exactly
`models/qwen2.5-coder-7b-instruct-q4_k_m.gguf` within the runtime folder, matching
`src/llm_server.rs`. It does not download dependencies or model files on recipient PCs.
The builder needs Rust, the Windows C++ build tools, and the existing AI bundle.
`-Offline` also prevents Cargo from contacting the registry during the build.

Share the ZIP and optionally its `.sha256` file. Your friend extracts it and runs
`Install.bat`, which verifies SHA-256 hashes and installs per user, without admin
rights. `Play.bat` also works directly from the extracted folder. Installation
preserves existing saves and settings. For a custom destination:

```bat
Install.bat -Destination "D:\Games\Voxel Project"
```

For automated installer checks, invoke `Install.ps1 -Destination ... -NoShortcuts`.
This is a ZIP with an offline installation script, not an MSI or signed setup EXE.
ZIP64 streaming is intentional: the 7B GGUF exceeds 4 GB, and PowerShell's
`Compress-Archive` is unsuitable for this payload. Files are stored without
compression so large model packaging remains quick. Plan for roughly three copies
of the payload while keeping the ZIP, extracted folder and installed game.

Steam editions start with Steam selected (test App ID 480); Direct remains available
in Settings. See [Steam multiplayer](docs/STEAM_MULTIPLAYER.md) for current multiplayer limitations.
Hosting prewarms inference. Guests start their local server on their first prompt
when the host enables guest prompting in Settings. Menu-only and non-prompting guests
do not start a server. The default full package includes AI for both hosts and guests;
`-NoAI` omits local generation capability unless a server is configured separately.
The existing server process remains running after the game exits.

Supplied server/model notices are copied, along with `licenses/` and available
license files from Cargo's dependency sources. An index records dependency versions.

Installer regression checks (all writes stay under `target/package-tests/`):

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File tools/test_package.ps1
```
