# Bundled component notices

- `llama.cpp-LICENSE.txt`: [llama.cpp upstream license](https://github.com/ggml-org/llama.cpp/blob/master/LICENSE).
- `Qwen2.5-Coder-LICENSE.txt`: [Qwen2.5-Coder-7B-Instruct-GGUF license](https://huggingface.co/Qwen/Qwen2.5-Coder-7B-Instruct-GGUF/blob/main/LICENSE).

Retrieved on 2026-09-10. The packager also collects available license and notice
files from Cargo dependency sources and copies the notices supplied with the
local llama runtime (including LLVM OpenMP).

Windows CRT DLLs are taken from Visual Studio's x64 redistributable folder;
see [Microsoft's redistribution documentation](https://learn.microsoft.com/en-us/cpp/windows/redistributing-visual-cpp-files).
Steam API files come from the selected Cargo Steamworks build. Test builds use
App ID 480; the Steam client is installed and signed into separately by each player.
