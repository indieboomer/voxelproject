"""Build a macOS app and DMG, with optional Steam and bundled inference."""
from __future__ import annotations

import argparse
from datetime import datetime, timezone
import json
from pathlib import Path
import plistlib
import shutil
import subprocess

from common import (MACOS, ROOT, MODEL_NAME, TARGETS, DEPLOYMENT_TARGET,
                    run, require_mac, require_tools, native_arch, build_env,
                    copy_model, sha256, linked_libraries, is_system_library,
                    check_arch, check_static_server, main_guard)


def assemble_bundle(app, executable, steam_library, runtime, model, version, project=ROOT):
    """Only copy an explicit list of release inputs; never settings, saves or logs."""
    contents = app / "Contents"
    resources = contents / "Resources"
    for directory in (contents / "MacOS", contents / "Frameworks", resources):
        directory.mkdir(parents=True, exist_ok=True)
    shutil.copy2(executable, contents / "MacOS/voxelproject")
    (contents / "MacOS/voxelproject").chmod(0o755)
    if steam_library:
        shutil.copy2(steam_library, contents / "Frameworks/libsteam_api.dylib")
    for filename in ("data/crafting.json", "data/resources.json"):
        destination = resources / filename
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(project / filename, destination)
    (resources / "modules").mkdir()
    for module in (project / "modules").glob("*.lua"):
        shutil.copy2(module, resources / "modules" / module.name)
    if (project / "licenses").is_dir():
        shutil.copytree(project / "licenses", resources / "licenses")
    (resources / "settings.json").write_text(json.dumps({"multiplayer": {
        "mode": "steam" if steam_library else "direct", "steam_app_id": 480}}), encoding="utf-8")
    if runtime:
        server = contents / "Helpers/llm-runtime/server"
        server.mkdir(parents=True)
        shutil.copy2(runtime / "llama-server", server / "llama-server")
        (server / "llama-server").chmod(0o755)
        # Setup builds a static server. Retain shader payloads and notices, not Windows DLLs/logs.
        for item in runtime.iterdir():
            if item.is_file() and (item.suffix in (".metal", ".metallib") or
                                   item.name.startswith(("LICENSE", "NOTICE", "COPYING"))):
                # Shader resources belong in Resources, away from nested signed code.
                destination = resources / "llama" / item.name
                destination.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(item, destination)
        copy_model(model, resources / "llm-runtime/models" / MODEL_NAME)
    with (contents / "Info.plist").open("wb") as stream:
        plistlib.dump({
            "CFBundleDevelopmentRegion": "en",
            "CFBundleExecutable": "voxelproject",
            "CFBundleIdentifier": "com.voxelproject.game",
            "CFBundleName": "Voxel Project",
            "CFBundleDisplayName": "Voxel Project",
            "CFBundlePackageType": "APPL",
            "CFBundleShortVersionString": version,
            "CFBundleVersion": version,
            "LSMinimumSystemVersion": DEPLOYMENT_TARGET,
            "NSHighResolutionCapable": True,
            "NSLocalNetworkUsageDescription": "Find and join Voxel Project multiplayer sessions on your local network.",
        }, stream)


def fix_game_libraries(app, arch, steam):
    game = app / "Contents/MacOS/voxelproject"
    check_arch(game, arch)
    for dependency in linked_libraries(game):
        if Path(dependency).name == "libsteam_api.dylib" and steam:
            run(["install_name_tool", "-change", dependency,
                 "@executable_path/../Frameworks/libsteam_api.dylib", game])
        elif not is_system_library(dependency):
            raise RuntimeError(f"Game has an unbundled dependency: {dependency}")
    if steam:
        library = app / "Contents/Frameworks/libsteam_api.dylib"
        check_arch(library, arch)
        run(["install_name_tool", "-id", "@rpath/libsteam_api.dylib", library])
        for dependency in linked_libraries(library):
            if Path(dependency).name != library.name and not is_system_library(dependency):
                raise RuntimeError(f"Steam API has an unbundled dependency: {dependency}")


def dependency_notices(metadata, resources):
    root = resources / "licenses/rust-dependencies"
    root.mkdir(parents=True, exist_ok=True)
    index = []
    for package in metadata["packages"]:
        if package["name"] == "voxelproject":
            continue
        index.append(f"{package['name']} {package['version']}: {package.get('license')} {package.get('repository')}")
        source = Path(package["manifest_path"]).parent
        notices = [p for p in source.iterdir() if p.is_file() and
                   p.name.startswith(("LICENSE", "LICENCE", "COPYING", "NOTICE"))]
        if package.get("license_file"):
            notices.append(source / package["license_file"])
        for notice in notices:
            dest = root / f"{package['name']}-{package['version']}" / notice.name
            dest.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(notice, dest)
    (root / "INDEX.txt").write_text("\n".join(index) + "\n", encoding="utf-8")


def sign_bundle(app, identity):
    options = ["--force", "--sign", identity]
    if identity != "-":
        options += ["--timestamp", "--options", "runtime"]
    # Sign inside out; never use --deep as a substitute for signing nested code.
    for library in sorted((app / "Contents/Frameworks").glob("*.dylib")):
        run(["codesign", *options, library])
    helper = app / "Contents/Helpers/llm-runtime/server/llama-server"
    if helper.is_file():
        run(["codesign", *options, helper])
    run(["codesign", *options, app])
    run(["codesign", "--verify", "--deep", "--strict", "--verbose=2", app])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--arch", choices=TARGETS)
    parser.add_argument("--direct-only", action="store_true")
    parser.add_argument("--no-ai", action="store_true")
    parser.add_argument("--no-dmg", action="store_true")
    parser.add_argument("--offline", action="store_true")
    parser.add_argument("--model", type=Path)
    parser.add_argument("--identity", default="-", help="Developer ID Application identity; default ad-hoc for local tests")
    parser.add_argument("--notary-profile", help="Existing xcrun notarytool keychain profile; submits the DMG to Apple")
    args = parser.parse_args()
    require_mac()
    require_tools("cargo", "xcrun", "codesign", "otool", "lipo", "install_name_tool", "hdiutil")
    if args.notary_profile and (args.identity == "-" or args.no_dmg):
        raise RuntimeError("Notarization needs --identity and a DMG (omit --no-dmg).")
    arch = args.arch or native_arch()
    env = build_env()
    runtime = None if args.no_ai else MACOS / "runtime" / arch / "server"
    model = args.model or MACOS / "runtime/models" / MODEL_NAME
    if runtime:
        if not (runtime / "llama-server").is_file() or not model.is_file():
            raise RuntimeError("Native AI runtime/model missing. Run setup.command first, or use --no-ai.")
        check_static_server(runtime / "llama-server", arch)
    flags = ["--locked", "--target", TARGETS[arch]]
    if not args.direct_only:
        flags += ["--features", "steam"]
    if args.offline:
        flags.append("--offline")
    # rustc-link-arg reserves space for relocatable Steam install names.
    build = run(["cargo", "rustc", "--release", *flags, "--message-format=json-render-diagnostics",
                 "--", "-C", "link-arg=-Wl,-headerpad_max_install_names"],
                cwd=ROOT, env=env, stdout=subprocess.PIPE, text=True)
    executable = steam_library = None
    for line in build.stdout.splitlines():
        message = json.loads(line)
        if message.get("reason") == "compiler-artifact" and message.get("executable") and message["target"]["name"] == "voxelproject":
            executable = Path(message["executable"])
        if message.get("reason") == "build-script-executed" and "steamworks-sys" in message["package_id"]:
            steam_library = Path(message["out_dir"]) / "libsteam_api.dylib"
    if not executable or (not args.direct_only and (not steam_library or not steam_library.is_file())):
        raise RuntimeError("Cargo did not report the executable or required Steam dylib.")
    if args.direct_only:
        steam_library = None
    metadata_flags = ["--locked", "--format-version", "1", "--filter-platform", TARGETS[arch]]
    if not args.direct_only:
        metadata_flags += ["--features", "steam"]
    if args.offline:
        metadata_flags.append("--offline")
    metadata = json.loads(subprocess.check_output(["cargo", "metadata", *metadata_flags], cwd=ROOT, env=env, text=True))
    version = next(p["version"] for p in metadata["packages"] if p["name"] == "voxelproject")
    stamp = datetime.now(timezone.utc).strftime("%Y%m%d-%H%M%S-%f")
    edition = "direct" if args.direct_only else "steam"
    name = f"VoxelProject-macos-{arch}-{edition}-{'client' if args.no_ai else 'full'}-{stamp}"
    output = MACOS / "dist" / name
    output.mkdir(parents=True)
    app = output / "Voxel Project.app"
    assemble_bundle(app, executable, steam_library, runtime, model, version)
    fix_game_libraries(app, arch, not args.direct_only)
    resources = app / "Contents/Resources"
    dependency_notices(metadata, resources)
    (resources / "build-info.json").write_text(json.dumps({
        "version": version, "target": TARGETS[arch], "steam": not args.direct_only,
        "bundled_ai": not args.no_ai, "model_sha256": None if args.no_ai else sha256(model),
    }, indent=2), encoding="utf-8")
    sign_bundle(app, args.identity)
    shutil.copy2(MACOS / "START-HERE.txt", output / "START-HERE.txt")
    if not args.no_dmg:
        (output / "Applications").symlink_to("/Applications", target_is_directory=True)
        dmg = output.with_suffix(".dmg")
        run(["hdiutil", "create", "-volname", "Voxel Project", "-srcfolder", output,
             "-format", "UDZO", "-fs", "HFS+", dmg])
        if args.identity != "-":
            run(["codesign", "--sign", args.identity, "--timestamp", dmg])
        if args.notary_profile:
            run(["xcrun", "notarytool", "submit", dmg, "--keychain-profile", args.notary_profile, "--wait"])
            run(["xcrun", "stapler", "staple", dmg])
            run(["xcrun", "stapler", "validate", dmg])
        dmg.with_suffix(".dmg.sha256").write_text(f"{sha256(dmg)}  {dmg.name}\n", encoding="utf-8")
        print(f"Package: {dmg}")
    print(f"App: {app}")
    if args.identity == "-":
        print("Ad-hoc test build. For normal downloaded-app installation, use Developer ID signing and notarization; see README.md.")


if __name__ == "__main__":
    main_guard(main)
