"""Platform-independent tests for the bundle layout and release input selection."""
import json
from pathlib import Path
import plistlib
import tempfile
import unittest
from unittest.mock import patch

from common import MODEL_NAME, check_static_server
from package import assemble_bundle, fix_game_libraries


class BundleTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="voxel-mac-package-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.project = self.root / "source with spaces"
        for name, payload in {
            "data/crafting.json": b"{}", "data/resources.json": b"{}",
            "modules/example.lua": b"return {}", "modules/private.log": b"secret",
            "settings.json": b"personal", "saves/world.bin": b"private world",
            "licenses/NOTICE": b"notice", "game": b"game binary", "steam.dylib": b"steam",
            "server/llama-server": b"native binary", "server/server.log": b"private prompts",
            "server/llama-server.exe": b"windows binary", "server/llama.dll": b"windows dll",
            "server/LICENSE": b"license", "server/default.metallib": b"metal shaders",
            "model.gguf": b"GGUFtest model",
        }.items():
            path = self.project / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(payload)
        self.app = self.root / "output with spaces/Voxel Project.app"

    def assemble(self, ai=True, steam=True):
        assemble_bundle(self.app, self.project / "game",
                        self.project / "steam.dylib" if steam else None,
                        self.project / "server" if ai else None,
                        self.project / "model.gguf", "0.1.0", self.project)

    def test_full_bundle_has_correct_resources_and_no_personal_or_windows_files(self):
        self.assemble()
        contents = self.app / "Contents"
        resources = contents / "Resources"
        plist = plistlib.loads((contents / "Info.plist").read_bytes())
        self.assertEqual(plist["CFBundleExecutable"], "voxelproject")
        self.assertEqual((resources / "llm-runtime/models" / MODEL_NAME).read_bytes(), b"GGUFtest model")
        self.assertTrue((contents / "Helpers/llm-runtime/server/llama-server").is_file())
        self.assertTrue((contents / "Frameworks/libsteam_api.dylib").is_file())
        self.assertTrue((resources / "llama/default.metallib").is_file())
        self.assertEqual(json.loads((resources / "settings.json").read_text())["multiplayer"]["mode"], "steam")
        files = [p.name for p in self.app.rglob("*") if p.is_file()]
        for forbidden in ("world.bin", "server.log", "private.log", "llama-server.exe", "llama.dll"):
            self.assertNotIn(forbidden, files)

    def test_client_direct_bundle_does_not_need_ai_or_steam(self):
        self.assemble(ai=False, steam=False)
        self.assertFalse((self.app / "Contents/Helpers").exists())
        self.assertFalse((self.app / "Contents/Resources/llm-runtime").exists())
        self.assertFalse(list((self.app / "Contents/Frameworks").iterdir()))
        settings = json.loads((self.app / "Contents/Resources/settings.json").read_text())
        self.assertEqual(settings["multiplayer"]["mode"], "direct")

    def test_non_gguf_is_rejected(self):
        (self.project / "model.gguf").write_bytes(b"download failed")
        with self.assertRaisesRegex(RuntimeError, "Not a GGUF"):
            self.assemble()

    def test_game_dependency_is_relocated_and_external_dependencies_rejected(self):
        self.assemble()
        with patch("package.check_arch"), patch("package.run") as command, patch(
            "package.linked_libraries", side_effect=[
                ["/build/sdk/libsteam_api.dylib", "/usr/lib/libSystem.B.dylib"],
                ["@rpath/libsteam_api.dylib", "/usr/lib/libSystem.B.dylib"]]):
            fix_game_libraries(self.app, "arm64", True)
            self.assertIn("@executable_path/../Frameworks/libsteam_api.dylib", command.call_args_list[0].args[0])
        with patch("package.check_arch"), patch("package.linked_libraries", return_value=["/opt/homebrew/lib/libunexpected.dylib"]):
            with self.assertRaisesRegex(RuntimeError, "unbundled"):
                fix_game_libraries(self.app, "arm64", True)

    def test_server_cannot_silently_depend_on_builder_homebrew_install(self):
        with patch("common.check_arch"), patch("common.linked_libraries", return_value=["/opt/homebrew/lib/libssl.dylib"]):
            with self.assertRaisesRegex(RuntimeError, "unbundled"):
                check_static_server(self.project / "server/llama-server", "arm64")


if __name__ == "__main__":
    unittest.main()
