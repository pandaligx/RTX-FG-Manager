import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch, Mock

spec = importlib.util.spec_from_file_location('linux_release', Path(__file__).resolve().parents[1] / 'tools/linux_release.py')
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)
spec = importlib.util.spec_from_file_location('gitcode_publish', Path(__file__).resolve().parents[1] / 'tools/gitcode_publish.py')
publisher = importlib.util.module_from_spec(spec)
spec.loader.exec_module(publisher)


class LinuxReleaseTests(unittest.TestCase):
    def test_promotion_checks_both_sites_without_touching_windows_channels(self):
        m = dict(schema=3, version='4.3.32', file='RTXManager-4.3.32-x86_64.AppImage',
                 bytes=1048576, sha256='a'*64, notes={})
        raw = json.dumps(m).encode()
        with patch.object(publisher, 'verify') as verify:
            publisher.verify_linux_update(raw)
            self.assertEqual(verify.call_count, 2)
            self.assertTrue(all(c.args[0].endswith('/v4.3.32/'+m['file']) for c in verify.call_args_list))
        review = Mock()
        windows = publisher.client_release.release_manifests(dict(schema=2, version='4.3.21',
            file='RTXManager.exe', bytes=1048576, sha256='b'*64, notes={}))
        blobs = {'legacy': windows['update.json'], 'win': windows['update-v2.json'], 'linux': raw}
        review.git.side_effect = lambda root, *args, **kwargs: blobs[args[-1]]
        before = {'update.json': ('100644','blob','legacy'), 'update-v2.json': ('100644','blob','win')}
        after = {**before, release.CHANNEL: ('100644','blob','linux')}
        with patch.object(publisher, 'verify_linux_update') as linux, patch.object(publisher, 'verify_update') as windows:
            publisher.verify_client_metadata(Path('.'), review, before, after)
            linux.assert_called_once_with(raw)
            windows.assert_not_called()
        with self.assertRaises(RuntimeError):
            publisher.verify_client_metadata(Path('.'), review, after, before)

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
