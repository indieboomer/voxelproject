"""Shared macOS build helpers. Nothing imports or edits the Windows packager."""
from __future__ import annotations

import hashlib
import os
from pathlib import Path
import platform
import shutil
import subprocess
import sys

MACOS = Path(__file__).resolve().parents[1]
ROOT = MACOS.parent
MODEL_NAME = "qwen2.5-coder-7b-instruct-q4_k_m.gguf"
DEPLOYMENT_TARGET = "13.0"
TARGETS = {"arm64": "aarch64-apple-darwin", "x86_64": "x86_64-apple-darwin"}


def run(args, **kwargs):
    print("+", " ".join(str(arg) for arg in args), flush=True)
    return subprocess.run([str(arg) for arg in args], check=True, **kwargs)


def require_mac():
    if sys.platform != "darwin":
        raise RuntimeError("Build this package on macOS with Xcode Command Line Tools. Windows packaging is unchanged.")


def require_tools(*names):
    for name in names:
        if not shutil.which(name):
            raise RuntimeError(f"Missing {name}. See macos/README.md for one-time setup.")
    run(["xcrun", "--find", "clang"], stdout=subprocess.DEVNULL)


def native_arch():
    arch = platform.machine()
    if arch not in TARGETS:
        raise RuntimeError(f"Unsupported Mac architecture: {arch}")
    return arch


def build_env():
    env = os.environ.copy()
    env["MACOSX_DEPLOYMENT_TARGET"] = DEPLOYMENT_TARGET
    # All Cargo output stays separate from target/debug and target/release on Windows.
    env["CARGO_TARGET_DIR"] = str(MACOS / "build" / "cargo")
    return env


def sha256(path):
    digest = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for block in iter(lambda: stream.read(8 * 1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def copy_model(source, destination):
    source, destination = Path(source), Path(destination)
    with source.open("rb") as stream:
        if stream.read(4) != b"GGUF":
            raise RuntimeError(f"Not a GGUF model: {source}")
    destination.parent.mkdir(parents=True, exist_ok=True)
    if source.resolve() != destination.resolve():
        shutil.copyfile(source, destination)


def linked_libraries(binary):
    output = subprocess.check_output(["otool", "-L", str(binary)], text=True)
    return [line.strip().split(" (compatibility version", 1)[0]
            for line in output.splitlines()[1:] if " (compatibility version" in line]


def is_system_library(name):
    return name.startswith(("/System/Library/", "/usr/lib/"))


def check_arch(binary, arch):
    run(["lipo", "-verify_arch", arch, binary])


def check_static_server(binary, arch):
    check_arch(binary, arch)
    external = [lib for lib in linked_libraries(binary) if not is_system_library(lib)]
    if external:
        raise RuntimeError(f"AI server has unbundled libraries: {external}. Rebuild using setup.command.")


def main_guard(main):
    try:
        main()
    except (OSError, RuntimeError, subprocess.CalledProcessError) as error:
        print(f"ERROR: {error}", file=sys.stderr)
        sys.exit(1)
