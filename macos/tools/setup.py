"""Build a native, self-contained llama-server and prepare the existing GGUF."""
from __future__ import annotations

import argparse
from pathlib import Path
import shutil
import subprocess

from common import (MACOS, ROOT, MODEL_NAME, TARGETS, DEPLOYMENT_TARGET, run,
                    require_mac, require_tools, native_arch, copy_model,
                    check_static_server, main_guard)

# Same revision reported by the existing bundled Windows llama server.
LLAMA_REVISION = "4d19b2876"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--arch", choices=TARGETS)
    parser.add_argument("--model", type=Path, default=ROOT / "llm-runtime/models" / MODEL_NAME)
    parser.add_argument("--llama-source", type=Path, help="Use an existing llama.cpp checkout; no clone")
    parser.add_argument("--llama-ref", default=LLAMA_REVISION, help="Git revision for the managed checkout")
    args = parser.parse_args()
    require_mac()
    require_tools("git", "cmake", "rustup", "cargo", "xcrun", "otool", "lipo")
    arch = args.arch or native_arch()
    if not args.model.is_file():
        raise RuntimeError(f"Copy the existing GGUF from Windows, then run setup.command --model /path/to/{MODEL_NAME}")
    source = args.llama_source.resolve() if args.llama_source else MACOS / "vendor/llama.cpp"
    if not args.llama_source:
        if not source.exists():
            source.parent.mkdir(parents=True, exist_ok=True)
            run(["git", "clone", "--filter=blob:none", "https://github.com/ggml-org/llama.cpp.git", source])
        if subprocess.check_output(["git", "-C", str(source), "status", "--porcelain"], text=True).strip():
            raise RuntimeError("Managed llama checkout has local changes. Use --llama-source or preserve those changes first.")
        run(["git", "-C", source, "checkout", "--detach", args.llama_ref])
    if not (source / "CMakeLists.txt").is_file():
        raise RuntimeError(f"Not a llama.cpp source tree: {source}")
    run(["rustup", "target", "add", TARGETS[arch]])
    build = MACOS / "build" / ("llama-" + arch)
    run(["cmake", "-S", source, "-B", build,
         "-DCMAKE_BUILD_TYPE=Release", f"-DCMAKE_OSX_ARCHITECTURES={arch}",
         f"-DCMAKE_OSX_DEPLOYMENT_TARGET={DEPLOYMENT_TARGET}",
         "-DCMAKE_EXE_LINKER_FLAGS=-Wl,-headerpad_max_install_names",
         "-DBUILD_SHARED_LIBS=OFF", "-DGGML_BACKEND_DL=OFF", "-DGGML_NATIVE=OFF",
         "-DGGML_METAL=ON", "-DGGML_METAL_EMBED_LIBRARY=ON", "-DGGML_OPENMP=OFF",
         "-DLLAMA_CURL=OFF", "-DLLAMA_OPENSSL=OFF", "-DLLAMA_BUILD_TESTS=OFF",
         "-DLLAMA_BUILD_EXAMPLES=OFF", "-DLLAMA_BUILD_SERVER=ON",
         "-DLLAMA_BUILD_UI=OFF", "-DLLAMA_USE_PREBUILT_UI=OFF"])
    run(["cmake", "--build", build, "--config", "Release", "--target", "llama-server", "--parallel"])
    binary = build / "bin/llama-server"
    check_static_server(binary, arch)
    server = MACOS / "runtime" / arch / "server"
    server.mkdir(parents=True, exist_ok=True)
    shutil.copy2(binary, server / "llama-server")
    (server / "llama-server").chmod(0o755)
    for pattern in ("LICENSE*", "NOTICE*", "COPYING*"):
        for notice in source.glob(pattern):
            if notice.is_file():
                shutil.copy2(notice, server / notice.name)
    # Retain any separate Metal payload emitted by the selected upstream version.
    for pattern in ("*.metallib", "*.metal"):
        for payload in (build / "bin").glob(pattern):
            shutil.copy2(payload, server / payload.name)
    copy_model(args.model, MACOS / "runtime/models" / MODEL_NAME)
    print(f"Runtime prepared for {arch}. Next: bash macos/make_package.command --arch {arch}")


if __name__ == "__main__":
    main_guard(main)
