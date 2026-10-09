"""Offline Encore cloud protocol checks; no signing, GPU or network claims."""
import copy
import io
import json
from pathlib import Path
import runpy
import unittest
import zipfile

ROOT = Path(__file__).resolve().parents[1]
cloud = runpy.run_path(str(ROOT / "tools/cloud_release.py"))
PROFILE = cloud["ENCORE_PROFILE"]
CONFIG = cloud["ENCORE_CONFIG"]
NOTICES = cloud["ENCORE_NOTICES"]


def config():
    return cloud["jsonc_loads"]((ROOT / "rust/assets/encore-defaults.jsonc").read_text(encoding="utf-8-sig"))


def spec():
    return {"schema": 1, "default_scheme": "dlssg-transfusion-1.4.5.3", "schemes": [{
        "id": "dlssg-transfusion-1.4.5.3", "name": "RTX Encore · 1.0.0-beta.2",
        "profile": PROFILE, "version": "1.0.0", "upstream_version": "1.0.0-beta.2",
        "min_manager_version": "4.2.9", "defaults": {},
        "archives": ["rtx-encore-1.0.0-beta.2-v429-universal.zip"],
    }]}


def archive(configuration=None, proxy="version.dll", extra=None, omit=None):
    pe = bytearray(512)
    pe[:2] = b"MZ"
    pe[60:64] = (128).to_bytes(4, "little")
    pe[128:134] = b"PE\0\0\x64\x86"
    pe[150:152] = (0x2000).to_bytes(2, "little")
    entries = {proxy: bytes(pe), CONFIG: json.dumps(config() if configuration is None else configuration).encode(),
               NOTICES: b"Synthetic test notice, not a distributable package.\n"}
    entries.update(extra or {})
    if omit:
        entries.pop(omit)
    stream = io.BytesIO()
    with zipfile.ZipFile(stream, "w", zipfile.ZIP_DEFLATED) as zipped:
        for name, content in entries.items():
            zipped.writestr(name, content)
    return stream.getvalue()


def package(configuration=None, **kwargs):
    scheme = spec()["schemes"][0]
    return cloud["package_from_zip"](scheme, scheme["archives"][0], archive(configuration, **kwargs))


class EncoreCloudTests(unittest.TestCase):
    def test_metadata_covers_native_schema4_template_without_flattened_paths(self):
        fields = cloud["encore_fields"]()
        def leaves(value, path=()):
            if isinstance(value, dict):
                for key, item in value.items():
                    yield from leaves(item, path + (key,))
            else:
                yield path
        paths = {tuple(field["path"]) for field in fields}
        native_paths = set(leaves(config()))
        self.assertEqual(len(fields), 88)
        self.assertEqual(len({field["key"] for field in fields}), len(fields))
        self.assertEqual(len(paths), len(fields))
        # Multiplier is exposed through the composite tf_mode selector. The
        # schema marker is not editable; upstream persists menuKey on demand.
        self.assertEqual(native_paths - paths, {("configVersion",), ("frameGeneration", "multiplier")})
        self.assertEqual(paths - native_paths, {("menuState", "menuKey")})
        self.assertTrue(next(field for field in fields if field["key"] == "menuKey")["optional"])

    def test_schema4_three_file_package_retains_full_upstream_version(self):
        payload = package()
        self.assertEqual(payload["backends"], ["encore"])
        self.assertEqual(payload["proxy"], "version.dll")
        self.assertEqual(payload["version"], "1.0.0")
        self.assertEqual(payload["upstream_version"], "1.0.0-beta.2")
        self.assertEqual({f["name"] for f in payload["files"]}, {"version.dll", CONFIG, NOTICES})
        candidate = spec()
        name = candidate["schemes"][0]["archives"][0]
        _, index, catalog = cloud["build_documents"](candidate, {name: archive()})
        self.assertEqual(json.loads(index)["packages"][0]["upstream_version"], "1.0.0-beta.2")
        catalog = json.loads(catalog)
        self.assertEqual(catalog["schemes"][0]["min_manager_version"], "4.2.9")
        self.assertEqual(catalog["index"]["sha256"], cloud["digest"](index))

    def test_schema4_uses_real_nested_paths_and_types(self):
        malformed = []
        for version in (3, 5, True, "4"):
            value = config()
            value["configVersion"] = version
            malformed.append(value)
        value = config()
        value["neuralRendering"] = value["neuralRendering"]["core"]
        malformed.append(value)
        for path, bad in ((["neuralRendering", "core", "nrEnabled"], "false"),
                          (["neuralRendering", "appearance", "nrIntensity"], 2.1),
                          (["neuralRendering", "openExperimental", "nrOpenUltraFastGhostTolerance"], 0.31),
                          (["frameGeneration", "multiplier"], True),
                          (["overlay", "metrics"], []),
                          (["diagnostics", "logFilesKept"], 0)):
            value = config()
            owner = value
            for part in path[:-1]:
                owner = owner[part]
            owner[path[-1]] = bad
            malformed.append(value)
        for value in malformed:
            with self.subTest(value=value), self.assertRaises(RuntimeError):
                package(value)

    def test_cloud_preserves_conservative_activation_defaults(self):
        for section, key, changed in ((["frameGeneration"], "mode", "fixed"),
                                      (["general"], "gpuSeries", "rtx30"),
                                      (["smoothMotion"], "smoothMotionEnabled", True),
                                      (["neuralRendering", "core"], "nrEnabled", True)):
            value = config()
            owner = value
            for part in section:
                owner = owner[part]
            owner[key] = changed
            with self.subTest(key=key), self.assertRaises(RuntimeError):
                package(value)

    def test_only_encore_accepts_notices_and_only_canonical_proxy_is_packaged(self):
        for arguments in ({"omit": NOTICES}, {"extra": {"game.log": b"private"}},
                          {"extra": {NOTICES: b"  \n"}}, {"proxy": "dxgi.dll"},
                          {"extra": {"../escaped.txt": b"no"}}):
            with self.subTest(arguments=arguments), self.assertRaises(RuntimeError):
                package(**arguments)
        legacy = spec()["schemes"][0]
        legacy["profile"] = "transfusion_json_v3"
        with self.assertRaisesRegex(RuntimeError, "exactly DLL"):
            cloud["package_from_zip"](legacy, legacy["archives"][0], archive())

    def test_old_clients_have_minimum_gate_and_scheme_id_is_preserved(self):
        candidate = spec()
        cloud["validate_spec"](candidate)
        for minimum in (None, "4.2.8"):
            old = copy.deepcopy(candidate)
            if minimum is None:
                del old["schemes"][0]["min_manager_version"]
            else:
                old["schemes"][0]["min_manager_version"] = minimum
            with self.assertRaisesRegex(RuntimeError, "4.2.9"):
                cloud["validate_spec"](old)
        duplicate = copy.deepcopy(candidate)
        duplicate["schemes"][0]["archives"].append("another.zip")
        with self.assertRaisesRegex(RuntimeError, "one canonical"):
            cloud["validate_spec"](duplicate)
        current = json.loads((ROOT / "cloud/schemes.json").read_text(encoding="utf-8-sig"))
        scheme = next(s for s in current["schemes"] if s["id"] == candidate["default_scheme"])
        self.assertEqual(scheme["profile"], PROFILE)
        self.assertEqual(scheme["upstream_version"], "1.0.0-beta.2")
        self.assertEqual(scheme["archives"], candidate["schemes"][0]["archives"])

    def test_full_version_and_metadata_defaults_do_not_accept_foreign_values(self):
        for version in ("1.0.1-beta.2", "1.0.0-", "1.0.0/evil", "1.0.0-beta.2\n"):
            value = spec()
            value["schemes"][0]["upstream_version"] = version
            with self.subTest(version=version), self.assertRaises(RuntimeError):
                cloud["validate_spec"](value)
        cloud["validate_defaults"](PROFILE, {"tf_mode": "dynamic", "tf_target": "237", "nrResolutionScale": "33"})
        for defaults in ({"rtx_mode": "follow"}, {"tf_target": "1001"}, {"nrIntensity": "nan"},
                         {"nrEnabled": "true"}, {"nrResolutionScale": "32"}):
            with self.subTest(defaults=defaults), self.assertRaises(RuntimeError):
                cloud["validate_defaults"](PROFILE, defaults)

    def test_comments_and_unknown_sections_preserved_but_duplicates_refused(self):
        value = config()
        value["futureOption"] = {"documentation": "https://example.org/a//b", "enabled": True}
        cloud["validate_encore_config"](value)
        text = "// source comment\n" + json.dumps(value)
        cloud["validate_encore_config"](cloud["jsonc_loads"](text))
        with self.assertRaisesRegex(RuntimeError, "Duplicate JSON"):
            cloud["jsonc_loads"]('{"configVersion":4,"configVersion":4}')

    def test_hotkeys_use_the_same_bounded_combinations_as_the_manager(self):
        for value in ("", "Insert", "Ctrl+Alt+2, Ctrl+Alt+Num2", "Control+Shift+F24", "Win+PageDown"):
            cloud["validate_defaults"](PROFILE, {"hotkeyFixed2": value})
        for value in ("Ctrl+Control+2", "F25", "Ctrl+", "Ctrl+Alt+Unknown", "2+Ctrl", "Insert," * 17):
            with self.subTest(value=value), self.assertRaises(RuntimeError):
                cloud["validate_defaults"](PROFILE, {"hotkeyFixed2": value})


if __name__ == "__main__":
    unittest.main()
