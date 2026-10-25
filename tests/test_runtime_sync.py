import copy
import importlib
import json
from pathlib import Path
import struct
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'tools'))
sync = importlib.import_module('sync_runtimes')


class RuntimeSyncTests(unittest.TestCase):
    def test_history_retention_and_immutable_versions(self):
        old = self.baseline(); old.pop('history', None)
        fallback = copy.deepcopy(old['packages'][0]); fallback['version'] = '310.9.1'
        fallback['urls'] = sync.urls(f'runtime-sr-310.9.1-{fallback["sha256"][:12]}.zip')
        new = copy.deepcopy(old); new['revision'] += 1
        sync.add_history(new, [fallback]); sync.validate_advance(old, new)
        changed = copy.deepcopy(new); changed['revision'] += 1; changed['history'][0]['bytes'] += 1
        with self.assertRaises(RuntimeError): sync.validate_advance(new, changed)
        duplicate = copy.deepcopy(new); duplicate['history'].append(fallback)
        with self.assertRaises(RuntimeError): sync.validate_catalog(duplicate)

    def test_history_keeps_only_eight_latest_per_kind(self):
        c = self.baseline(); c['history'] = []
        rows = []
        for n in range(1, 13):
            p = copy.deepcopy(c['packages'][0]); p['version'] = f'310.8.{n}'
            p['urls'] = sync.urls(f'runtime-sr-{p["version"]}-{p["sha256"][:12]}.zip'); rows.append(p)
        sync.add_history(c, rows); sync.validate_catalog(c)
        self.assertEqual([p['version'] for p in c['history']], [f'310.8.{n}' for n in range(12, 4, -1)])

    def baseline(self):
        return json.loads((ROOT / 'cloud/runtime-catalog.json').read_text(encoding='utf-8'))

    def test_scope_and_sources(self):
        c = self.baseline()
        sync.validate_catalog(c)
        for kind in ['nr', 'xemfg', 'sr']:
            v = copy.deepcopy(c)
            v['packages'][1]['kind'] = kind
            with self.assertRaises(RuntimeError): sync.validate_catalog(v)
        v = copy.deepcopy(c)
        v['packages'][0]['urls'][0] = 'https://gitcode.com/other/runtime.zip'
        with self.assertRaises(RuntimeError): sync.validate_catalog(v)

    def test_private_names_oversized_hashes_and_paths_rejected(self):
        for key, value in [('bytes', 0), ('bytes', sync.LIMIT+1), ('sha256', 'a'*63), ('version', '311.0.0')]:
            c = self.baseline(); c['packages'][0][key] = value
            with self.assertRaises(RuntimeError): sync.validate_catalog(c)
        c = self.baseline(); c['packages'][0]['files'][0]['name'] = '../nvngx_dlss.dll'
        with self.assertRaises(RuntimeError): sync.validate_catalog(c)

    def test_no_downgrade_or_same_version_replacement(self):
        c = self.baseline(); n = copy.deepcopy(c)
        with self.assertRaises(RuntimeError): sync.validate_advance(c, n)
        n['revision'] += 1; sync.validate_advance(c, n)
        n['packages'][0]['bytes'] += 1
        with self.assertRaises(RuntimeError): sync.validate_advance(c, n)

    def test_only_commit_pinned_official_release_paths(self):
        good = sync.RAW + 'a'*40 + '/lib/Windows_x86_64/rel/nvngx_dlss.dll'
        self.assertTrue(sync.allowed(good))
        for bad in [good.replace('/rel/', '/dev/'), good.replace('x86_64', 'aarch64'),
                    good.replace('https:', 'http:'), good.replace('NVIDIA/', 'Other/'),
                    good.replace('a'*40, 'main'), good+'?query=1', good+'/../x.dll',
                    'https://dlss-swapper-downloads.beeradmoore.com/dlss/a.zip']:
            self.assertFalse(sync.allowed(bad), bad)

    def test_select_stable_release_by_version_not_response_order(self):
        rows = [{'tag_name':'v310.9.16'}, {'tag_name':'v310.10.0', 'prerelease':True},
                {'tag_name':'v310.11.0', 'draft':True}, {'tag_name':'v310.8.0'}, {'tag_name':'bad'}]
        self.assertEqual(sync.official_release(rows), ((310,9,16), 'v310.9.16'))

    def test_x64_dll_required(self):
        dll = bytearray(512); dll[:2] = b'MZ'; struct.pack_into('<I',dll,60,128)
        dll[128:132] = b'PE\0\0'; struct.pack_into('<H',dll,132,0x8664)
        struct.pack_into('<H',dll,150,0x2000); struct.pack_into('<H',dll,152,0x20b)
        sync.validate_pe(dll)
        for offset, value in [(132,0x14c), (150,2), (152,0x10b)]:
            bad = dll.copy(); struct.pack_into('<H',bad,offset,value)
            with self.assertRaises(RuntimeError): sync.validate_pe(bad)

    def test_vendor_verification_rejects_other_publishers_and_bad_signatures(self):
        valid = {'status':'Valid','subject':'CN=NVIDIA Corporation, O=NVIDIA Corporation, C=US',
                 'version':'310.9.16.0','company':'NVIDIA Corporation'}
        from types import SimpleNamespace
        for changes in [{}, {'status':'NotSigned'}, {'subject':'CN=Other, O=Other'}, {'version':'310.9.1.0'}]:
            info = valid | changes
            result = SimpleNamespace(returncode=0,stdout=json.dumps(info))
            with patch.object(sync.subprocess, 'run', return_value=result):
                if changes:
                    with self.assertRaises(RuntimeError): sync.verify_vendor(Path('runtime.dll'),'310.9.16')
                else: sync.verify_vendor(Path('runtime.dll'),'310.9.16')
        vendor_format = valid | {'company':'NVIDIA','version':'310,9,16,0'}
        with patch.object(sync.subprocess, 'run', return_value=SimpleNamespace(returncode=0,stdout=json.dumps(vendor_format))):
            sync.verify_vendor(Path('runtime.dll'),'310.9.16')

    def test_failed_upload_never_reaches_github(self):
        with patch.object(sync.pub,'upload',side_effect=RuntimeError('offline')), patch.object(sync,'gh_api') as api:
            with self.assertRaises(RuntimeError): sync.publish_packages([({},Path('runtime.zip'),{})])
            api.assert_not_called()

    def test_both_public_copies_verified_before_return(self):
        p = self.baseline()['packages'][0]; path = Path(p['urls'][0].rsplit('/',1)[1]); order=[]
        with (patch.object(sync.pub,'upload',side_effect=lambda *a:order.append('gitcode')),
             patch.object(sync,'gh_api',return_value={'assets':[{'name':path.name}]}),
             patch.object(sync.pub,'verify',side_effect=lambda url,*a:order.append(url))):
            sync.publish_packages([(p,path,{})])
        self.assertEqual(order,['gitcode',*p['urls']])

    def test_metadata_guard_verifies_only_changed_packages_on_both_sites(self):
        old = self.baseline(); new = copy.deepcopy(old); new['revision'] += 1
        p = new['packages'][0]; p['version'] = '310.10.0'
        p['urls'] = sync.urls(f'runtime-sr-{p["version"]}-{p["sha256"][:12]}.zip')
        from unittest.mock import Mock
        review = Mock(); review.git.side_effect = [json.dumps(new).encode(), json.dumps(old).encode()]
        path = sync.CATALOG
        with patch.object(sync.pub, 'verify_metadata_artifact') as verify:
            sync.pub.verify_metadata_changes(ROOT, review, {path:('100644','blob','old')}, {path:('100644','blob','new')})
        self.assertEqual([call.args[0] for call in verify.call_args_list], p['urls'])


if __name__ == '__main__': unittest.main()
