import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch, Mock

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('client_release', ROOT / 'tools/client_release.py')
client = importlib.util.module_from_spec(spec)
spec.loader.exec_module(client)
spec = importlib.util.spec_from_file_location('gitcode_publish', ROOT / 'tools/gitcode_publish.py')
publisher = importlib.util.module_from_spec(spec)
spec.loader.exec_module(publisher)


def manifest(v='4.3.21', schema=2, digest='a'):
    return dict(schema=schema, version=v, file='RTXManager.exe' if schema == 2 else f'RTXManager-v{v}-x64.exe',
                sha256=digest * 64, bytes=1048576, notes={'zh-CN': '更新', 'en': 'Update'})


class ClientReleaseTests(unittest.TestCase):
    def test_bridge_and_two_future_releases_keep_old_clients_on_the_same_bridge(self):
        bridge = client.release_manifests(manifest())
        old = client.checked(bridge['update.json'], 'update.json')
        self.assertEqual(old['file'], 'RTXManager-v4.3.21-x64.exe')
        previous = bridge['update-v2.json']
        for v, digest in [('4.3.22', 'b'), ('4.3.23', 'c')]:
            assets = client.release_manifests(manifest(v, digest=digest), bridge['update.json'])
            self.assertEqual(assets['update.json'], bridge['update.json'])
            self.assertEqual(client.transition(previous, assets['update-v2.json'], 'update-v2.json'), 'advance')
            self.assertEqual(client.checked(assets['update-v2.json'])['file'], 'RTXManager.exe')
            previous = assets['update-v2.json']

    def test_wrong_channel_rollback_and_bridge_replacement_are_rejected(self):
        for channel, data in [('update.json', manifest()), ('update-v2.json', manifest(schema=1))]:
            with self.assertRaises(RuntimeError):
                client.checked(json.dumps(data), channel)
        bridge = client.release_manifests(manifest())
        for current in [manifest('4.3.20', 1), manifest('4.3.22', 1), manifest(schema=1, digest='b')]:
            with self.assertRaises(RuntimeError):
                client.transition(bridge['update.json'], json.dumps(current), 'update.json')
        with self.assertRaises(RuntimeError):
            client.transition(json.dumps(manifest('4.3.23')), json.dumps(manifest('4.3.22')), 'update-v2.json')
        with self.assertRaises(RuntimeError):
            client.release_manifests(manifest('4.3.22'), json.dumps(manifest('4.3.20', 1)).encode())

    def test_manifest_validation_before_any_download(self):
        for key, value in [('schema', True), ('schema', 3), ('file', '../RTXManager.exe'),
                           ('bytes', True), ('bytes', 0), ('sha256', 'bad'), ('version', '1.2.3/evil'),
                           ('notes', {'en': 'x' * 16385})]:
            with self.subTest(key=key), patch.object(publisher, 'verify') as verify:
                with self.assertRaises(RuntimeError):
                    publisher.verify_update(json.dumps({**manifest(), key: value}))
                verify.assert_not_called()

    def test_modern_release_checks_both_sites_and_both_channels_before_promotion(self):
        bridge = client.release_manifests(manifest())
        modern = client.release_manifests(manifest('4.3.22', digest='b'), bridge['update.json'])
        for name, raw in modern.items():
            with patch.object(publisher, 'verify') as verify:
                publisher.verify_update(raw)
                self.assertEqual(verify.call_count, 2)
                expected = 'v4.3.22/RTXManager.exe' if name == 'update-v2.json' else 'v4.3.21/RTXManager-v4.3.21-x64.exe'
                self.assertTrue(all(call.args[0].endswith(expected) for call in verify.call_args_list))

    def test_ci_mirrors_small_manifests_last_and_never_an_exe(self):
        bridge = client.release_manifests(manifest())
        assets = client.release_manifests(manifest('4.3.22', digest='b'), bridge['update.json'])
        with tempfile.TemporaryDirectory() as d:
            folder = Path(d)
            for name, raw in {**assets, 'RTXManager.exe': b'fixture', 'SHA256SUMS.txt': b'checksums'}.items():
                (folder / name).write_bytes(raw)
            with patch.object(publisher, 'verify_update'), patch.object(publisher, 'upload') as upload:
                publisher.mirror_assets(folder, publisher.GC_REPO, 'v4.3.22')
                self.assertEqual([call.args[2].name for call in upload.call_args_list],
                                 ['SHA256SUMS.txt', 'update.json', 'update-v2.json'])
            (folder / 'RTXManager-v4.3.22-x64.exe').write_bytes(b'unexpected')
            with patch.object(publisher, 'upload') as upload, self.assertRaises(RuntimeError):
                publisher.mirror_assets(folder, publisher.GC_REPO, 'v4.3.22')
            upload.assert_not_called()

    def test_source_promotion_retains_bridge_and_rejects_removal_or_replacement(self):
        bridge = client.release_manifests(manifest())
        future = client.release_manifests(manifest('4.3.22', digest='b'), bridge['update.json'])
        before = {'update.json': ('100644', 'blob', 'legacy'), 'update-v2.json': ('100644', 'blob', 'old')}
        after = {**before, 'update-v2.json': ('100644', 'blob', 'new')}
        blobs = {'legacy': bridge['update.json'], 'old': bridge['update-v2.json'], 'new': future['update-v2.json']}
        review = Mock()
        review.git.side_effect = lambda root, *args, **kwargs: blobs[args[-1]]
        with patch.object(publisher, 'verify_update') as verify:
            publisher.verify_client_metadata(ROOT, review, before, after)
            verify.assert_called_once_with(future['update-v2.json'])
        for missing in ('update.json', 'update-v2.json'):
            with self.assertRaises(RuntimeError):
                publisher.verify_client_metadata(ROOT, review, before, {k: v for k, v in before.items() if k != missing})

    def test_local_packaging_uses_one_signed_input_and_never_resigns_or_overwrites(self):
        with tempfile.TemporaryDirectory() as d:
            folder = Path(d)
            exe = folder / 'signed.exe'
            exe.write_bytes(b'x' * 1048576)
            # Only the signature primitive is mocked for this packaging test;
            # actual release preparation has no signature-bypass option.
            with patch.object(client, 'verify_signed') as verify:
                files = client.prepare(exe, '4.3.21', {}, folder / 'bridge')
                verify.assert_called_once_with(exe, '4.3.21')
                self.assertEqual(len([f for f in files if f.endswith('.exe')]), 2)
                self.assertEqual((folder/'bridge/RTXManager.exe').read_bytes(), (folder/'bridge/RTXManager-v4.3.21-x64.exe').read_bytes())
                files = client.prepare(exe, '4.3.22', {}, folder / 'future', folder/'bridge/update.json')
                self.assertEqual([f for f in files if f.endswith('.exe')], ['RTXManager.exe'])
                with self.assertRaises(RuntimeError):
                    client.prepare(exe, '4.3.22', {}, folder / 'future', folder/'bridge/update.json')
            with patch.object(client, 'verify_signed', side_effect=RuntimeError('unsigned')):
                with self.assertRaisesRegex(RuntimeError, 'unsigned'):
                    client.prepare(exe, '4.3.21', {}, folder / 'rejected')
            self.assertFalse((folder / 'rejected').exists())


if __name__ == '__main__':
    unittest.main()
