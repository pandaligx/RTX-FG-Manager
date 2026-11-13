import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('linux_release', Path(__file__).resolve().parents[1] / 'tools/linux_release.py')
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)


class LinuxReleaseTests(unittest.TestCase):
    def test_appimage_preparation_and_immutable_channel(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            image = root / 'input.AppImage'
            data = bytearray(1048576)
            data[:6], data[8:11], data[18:20] = b'\x7fELF\x02\x01', b'AI\x02', b'\x3e\0'
            image.write_bytes(data)
            m = release.prepare(image, '4.3.32', {'en': 'Test'}, root / 'output')
            raw = json.dumps(m)
            self.assertEqual(release.transition(raw, raw), m)
            self.assertEqual((root/'output'/m['file']).read_bytes(), bytes(data))
            self.assertFalse((root/'output'/'update-v2.json').exists())
            self.assertFalse((root/'output'/'update.json').exists())
            for change in [{'schema': 2}, {'file': 'RTXManager.exe'}, {'file': '../x.AppImage'},
                           {'sha256': 'bad'}, {'bytes': True}, {'version': '4.3.31'}]:
                with self.subTest(change=change), self.assertRaises(RuntimeError):
                    release.transition(raw, json.dumps({**m, **change}))
            with self.assertRaises(RuntimeError):
                release.transition(raw, json.dumps({**m, 'sha256': 'b'*64}))

    def test_windows_elf_and_wrong_architecture_are_rejected(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            image = root/'input'
            for header in [b'MZ', b'\x7fELF\x02\x01', b'\x7fELF\x02\x01\0\0AI\x02']:
                image.write_bytes(header + b'\0' * 1048576)
                with self.assertRaises(RuntimeError):
                    release.prepare(image, '4.3.32', {}, root/'output')
                self.assertFalse((root/'output').exists())


if __name__ == '__main__':
    unittest.main()
