# macOS setup and packaging

All Mac build tools, runtime binaries, intermediate output and packages live in
this directory. Windows `build.bat`, `make_package.bat` and `tools/*.ps1` are unchanged.
Run these tools on a Mac with macOS 13 or later. The current environment used to
write these tools is Windows; native compilation, signing and gameplay remain to
be validated on macOS.

## One-time builder setup

Install Xcode Command Line Tools (`xcode-select --install`), Rust via
[rustup](https://rustup.rs/), and Python 3.9+ and CMake (for example, using an existing
Homebrew installation: `brew install python cmake`). Steam must be installed and
signed in for Steam gameplay, but isn't required to compile the game.

Copy/clone the complete game repository onto the Mac, including the added player
models, sounds and other compile-time assets. Keep its Cargo.lock. Copy the existing
`qwen2.5-coder-7b-instruct-q4_k_m.gguf` from Windows; the model itself is portable.
From the repository root:

```sh
bash macos/setup.command --model "/path/to/qwen2.5-coder-7b-instruct-q4_k_m.gguf"
bash macos/make_package.command
```

Setup checks prerequisites, clones llama.cpp at revision `4d19b2876` (the revision
reported by the existing Windows runtime), builds a static native server with
Metal and embedded Metal shaders, and copies the model under `macos/runtime/`.
No Windows binaries are reused. The clone and initial Rust build need internet
access; no model download is performed. To use an existing source checkout:

```sh
bash macos/setup.command --llama-source /path/to/llama.cpp --model /path/to/model.gguf
```

The builder rejects a server that still depends on non-system dynamic libraries,
so a hidden dependency on the builder's Homebrew installation cannot be shipped.
For maintained upstream updates, `--llama-ref REVISION` selects a different revision
in the managed checkout. Local modifications in that checkout are preserved and
cause setup to stop before switching revisions.

After a fresh checkout, use the `bash` commands above. To enable Finder double-click
launching, run `chmod +x macos/*.command` once.

## Package options

```sh
bash macos/make_package.command                       # Steam + full AI, native architecture
bash macos/make_package.command --no-ai               # small client; ordinary hosting still works
bash macos/make_package.command --direct-only          # no Steam dependency
bash macos/make_package.command --offline --no-dmg     # cached Cargo dependencies, .app only
bash macos/setup.command --arch x86_64 --model /path/to/model.gguf
bash macos/make_package.command --arch x86_64
```

Options can be combined. `--arch arm64` selects Apple Silicon; `--arch x86_64` selects
Intel. Build each architecture's native AI runtime with setup before packaging it.
There are separate packages per architecture, not a universal app. Cross-building
still requires a compatible Mac SDK and the Rust target installed by setup; test
each result on its intended architecture. No minimum-spec performance claim is made.

`--model PATH` on the packager overrides the prepared model source. The default
full package uses the same model and 32768-token context as Windows. Client builds
can host gameplay but need a separately running inference server to generate rules.

Output: `macos/dist/VoxelProject-macos-ARCH-.../Voxel Project.app`, plus a timestamped
DMG and SHA-256 file. Cargo output is isolated under `macos/build/cargo/`, preserving
existing Windows executables. New builds never overwrite old package directories.
The model is copied into the app; allow disk space for runtime, app and DMG copies.

## Signing and normal distribution

Default packages are ad-hoc signed for testing, with a verified nested-code layout.
Downloaded ad-hoc builds can trigger Gatekeeper; this is not a notarized release.
For normal distribution, use your Developer ID Application identity and an existing
notarytool keychain profile:

```sh
bash macos/make_package.command \
  --identity "Developer ID Application: Your Name (TEAMID)" \
  --notary-profile "voxel-notary"
```

That explicit option submits the DMG to Apple's notary service, waits for the result,
and staples/validates the ticket. Credentials stay in your keychain. Without the
option, no package is submitted anywhere. Code is signed inside out after Steam
library paths are made relative. Unexpected external library dependencies fail
the build. The Steam overlay under hardened runtime needs a real-Mac test; lobby
code joining is available without relying on the overlay.

## Runtime layout

```text
Voxel Project.app/Contents/
  Info.plist
  MacOS/voxelproject
  Frameworks/libsteam_api.dylib              (Steam edition)
  Helpers/llm-runtime/server/llama-server    (full edition)
  Resources/
    settings.json                          (fresh-install defaults)
    data/, modules/, licenses/
    llama/                                 (shader resources and runtime notices)
    llm-runtime/models/*.gguf               (full edition)
```

Finder launches resolve assets relative to the executable. Before starting threads,
the Mac bundle sets its working directory to
`~/Library/Application Support/Voxel Project/` for existing save/settings code.
Only missing settings are initialized from bundle defaults; updates preserve user
preferences and saves. Resources and models are read from the bundle, and server
logs go to the user directory. Running `cargo run` outside an app keeps the existing
development working-directory behavior, as does every Windows launch.

## Checks

Platform-independent checks (also runnable on Windows with Python):

```sh
python3 -m unittest discover -s macos/tools -p 'test_*.py' -v
cargo test --offline runtime_paths::tests
```

On the Mac, build the package, then validate:

- `codesign --verify --deep --strict --verbose=2 "path/to/Voxel Project.app"`
- For a notarized package: `xcrun stapler validate path/to/package.dmg`
- Copy out of the DMG and launch from Finder; test input capture, rendering and audio.
- Create/save/reload a world; replace the app and verify preferences/world preservation.
- Host on Mac and join from Windows, then reverse roles; test Steam and Direct.
- Generate a rule, check the AI log and confirm effects on both players.
- Confirm menu-only and joining without prompting don't start a local AI server.
- Enable guest prompting on the host; submit from a guest and review/activate on the host.

References: [llama.cpp build options](https://github.com/ggml-org/llama.cpp/blob/master/docs/build.md),
[Rust macOS targets](https://doc.rust-lang.org/rustc/platform-support/apple-darwin.html),
[Apple notarization](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution).
