import importlib.util
import hashlib
import json
import os
import subprocess
from pathlib import Path
import tempfile
import unittest
from datetime import datetime, timezone, timedelta
from unittest.mock import patch, Mock

spec=importlib.util.spec_from_file_location('gitcode_publish',Path(__file__).resolve().parents[1]/'tools/gitcode_publish.py')
pub=importlib.util.module_from_spec(spec)
spec.loader.exec_module(pub)


class ClientAssetRoutingTests(unittest.TestCase):
    def run_release(self, names):
        ci_spec = importlib.util.spec_from_file_location('gitcode_ci_test',
            Path(__file__).resolve().parents[1] / 'tools/gitcode_ci.py')
        ci = importlib.util.module_from_spec(ci_spec)
        with patch.dict('sys.modules', {'gitcode_publish': pub}):
            ci_spec.loader.exec_module(ci)
        release = dict(name='4.3.31', body='Reviewed release', targetCommitish='main',
                       assets=[dict(name=name, size=size) for name, size in names])
        env = dict(RELEASE_TAG='v4.3.31', DESTINATION_TAG='', PAYLOADS='false', ASSET_NAME='')
        with patch.dict(os.environ, env), \
                patch.object(pub, 'sync_source'), patch.object(pub, 'sync_release_tag'), \
                patch.object(pub, 'create_release'), patch.object(pub, 'mirror_assets'), \
                patch.object(ci.subprocess, 'check_output', return_value=json.dumps(release)), \
                patch.object(ci.subprocess, 'run') as download:
            ci.main()
        return [call.args[0][call.args[0].index('--pattern') + 1]
                for call in download.call_args_list]

    def test_linux_executable_stays_local_and_windows_channels_are_preserved(self):
        names = [('RTXManager.exe', 48_000_000),
                 ('RTXManager-4.3.31-x86_64.AppImage', 24_000_000),
                 ('SHA256SUMS-linux.txt', 120), ('update.json', 2400), ('update-v2.json', 5100)]
        self.assertEqual(self.run_release(names),
                         ['SHA256SUMS-linux.txt', 'update.json', 'update-v2.json'])

    def test_unrelated_or_wrong_version_linux_executable_is_rejected(self):
        for name in ('unreviewed.AppImage', 'RTXManager-4.3.30-x86_64.AppImage'):
            with self.subTest(name=name), self.assertRaisesRegex(RuntimeError, 'Unexpected stable'):
                self.run_release([(name, 100)])

    def test_linux_checksum_keeps_small_asset_size_limit(self):
        with self.assertRaisesRegex(RuntimeError, 'Unexpected asset size'):
            self.run_release([('SHA256SUMS-linux.txt', 2 * 1024 * 1024 + 1)])

    def test_reviewed_license_notice_is_mirrored_with_small_asset_limit(self):
        self.assertEqual(self.run_release([('THIRD_PARTY_NOTICES.txt', 92_440)]),
                         ['THIRD_PARTY_NOTICES.txt'])
        with self.assertRaisesRegex(RuntimeError, 'Unexpected asset size'):
            self.run_release([('THIRD_PARTY_NOTICES.txt', 2 * 1024 * 1024 + 1)])
        with self.assertRaisesRegex(RuntimeError, 'Unexpected stable'):
            self.run_release([('private-notes.txt', 100)])


class MergeHistoryTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        spec = importlib.util.spec_from_file_location('source_review',
            Path(__file__).resolve().parents[1] / '.github/scripts/mirror_gitee.py')
        self.review = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.review)
        self.git('init', '-b', 'main')
        self.git('config', 'user.name', 'Publication test')
        self.git('config', 'user.email', 'test@example.invalid')
        self.write('README.md', 'base')
        self.write('app/tools/aria2.conf', 'legacy published configuration')
        self.write('cloud/catalog.json', '{"version":"old"}')
        self.commit('legacy base')
        self.git('branch', 'feature')
        self.git('rm', 'app/tools/aria2.conf')
        self.write('cloud/catalog.json', '{"version":"current"}')
        self.previous = self.commit('published main')
        self.git('switch', 'feature')

    def git(self, *args):
        return subprocess.check_output(['git', '-C', str(self.root), *args],
            stderr=subprocess.PIPE, text=True, encoding='utf-8').strip()

    def write(self, path, content):
        target = self.root / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(content, encoding='utf-8')

    def commit(self, message):
        self.git('add', '.')
        self.git('commit', '-m', message)
        return self.git('rev-parse', 'HEAD')

    def merge(self):
        self.git('switch', 'main')
        self.git('merge', '--no-ff', 'feature', '-m', 'merge feature')
        return self.git('rev-parse', 'HEAD')

    def history(self, source):
        return pub.source_publication_history(self.root, self.review, self.previous, source)

    def test_old_feature_inheritance_does_not_roll_back_publication_metadata(self):
        self.write('rust/src/new_feature.rs', 'pub fn feature() {}')
        self.commit('feature')
        source = self.merge()
        history = self.history(source)
        self.assertEqual(history, [self.review.tree_entries(self.root, source)])
        self.assertNotIn('app/tools/aria2.conf', history[0])
        self.assertEqual(history[0]['cloud/catalog.json'],
                         self.review.tree_entries(self.root, self.previous)['cloud/catalog.json'])

    def test_forbidden_side_branch_file_is_rejected_even_after_deletion(self):
        self.write('private.key', 'private test fixture')
        self.commit('unsafe addition')
        self.git('rm', 'private.key')
        self.commit('remove unsafe addition')
        with self.assertRaisesRegex(RuntimeError, 'allowlist rejected: private.key'):
            self.history(self.merge())

    def test_changed_obsolete_file_is_not_grandfathered(self):
        self.write('app/tools/aria2.conf', 'new unreviewed content')
        self.commit('modify legacy file')
        self.git('rm', 'app/tools/aria2.conf')
        self.commit('remove modified file')
        with self.assertRaisesRegex(RuntimeError, 'allowlist rejected: app/tools/aria2.conf'):
            self.history(self.merge())

    def test_safe_blob_under_forbidden_path_is_still_rejected(self):
        self.write('private.key', 'base')
        self.commit('reuse existing blob at unsafe path')
        self.git('rm', 'private.key')
        self.commit('remove renamed blob')
        with self.assertRaisesRegex(RuntimeError, 'allowlist rejected: private.key'):
            self.history(self.merge())

    def test_main_checkpoints_are_in_order(self):
        self.write('rust/src/new_feature.rs', 'pub fn feature() {}')
        self.commit('feature')
        merged = self.merge()
        self.write('README.md', 'after merge')
        after = self.commit('main update')
        self.assertEqual(self.history(after), [self.review.tree_entries(self.root, merged),
                                              self.review.tree_entries(self.root, after)])


class PublicationTests(unittest.TestCase):
    def test_only_never_published_never_referenced_transient_indexes_are_ignored(self):
        catalog = 'cloud/bundles/catalog.json'
        active = 'cloud/bundles/indexes/bundles-' + 'a' * 20 + '.json'
        candidate = 'cloud/bundles/indexes/bundles-' + 'b' * 20 + '.json'
        baseline = {catalog: ('100644', 'blob', 'c'), active: ('100644', 'blob', 'a')}
        pending = {**baseline, candidate: ('100644', 'blob', 'b')}
        review = Mock()
        review.git.return_value = json.dumps({'revision': 'bundles-' + 'a' * 20}).encode()
        check = lambda before, final, history: pub.transient_unreferenced_bundle_indexes(Path('.'), review, before, final, history)
        self.assertEqual(check(baseline, baseline, [pending, baseline]), {candidate})
        self.assertEqual(check(pending, baseline, [baseline]), set())
        self.assertEqual(check(baseline, pending, [pending]), set())
        review.git.return_value = json.dumps({'revision': 'bundles-' + 'b' * 20}).encode()
        self.assertEqual(check(baseline, baseline, [pending]), set())

    def test_large_resource_receipt_and_download_bound_keep_exe_limit(self):
        digest='a'*64;size=274667717
        url=pub.asset_url(pub.PAYLOAD_REPO,'payloads','encore-complete-'+digest[:12]+'.zip')
        receipt={'schema':1,'scope':'immutable-resource-readback',
                 'verified_at':datetime.now(timezone.utc).isoformat(),
                 'metadata_sha256':['b'*64],'assets':{url:{'bytes':size,'sha256':digest}}}
        with patch.dict(os.environ,{'RTXFG_RESOURCE_RECEIPTS':json.dumps(receipt)}), patch.object(pub,'verify') as verify:
            pub.verify_metadata_artifact(url,size,digest,'b'*64)
            verify.assert_not_called()
        with patch.object(pub.urllib.request,'urlopen',side_effect=RuntimeError('download reached')):
            with self.assertRaisesRegex(RuntimeError,'download reached'):pub.verify(url,size,digest)
            with self.assertRaisesRegex(RuntimeError,'Invalid artifact'):pub.verify(url,512*1024*1024+1,digest)
            for other in [url.replace('.zip','.exe'),url.replace('.zip','.json'),url.replace('gitcode.com','example.com')]:
                with self.assertRaisesRegex(RuntimeError,'Invalid artifact'):pub.verify(other,size,digest)

    def test_split_bundle_index_can_only_complete_an_unchanged_final_catalog(self):
        catalog='cloud/bundles/catalog.json';index='cloud/bundles/indexes/bundles-'+'a'*20+'.json'
        entry=('100644','blob','catalog');blob=('100644','blob','index')
        review=Mock();review.git.return_value=json.dumps({'revision':'bundles-'+'a'*20}).encode()
        current={catalog:entry};final={catalog:entry,index:blob}
        self.assertEqual(pub.complete_pending_bundle_index(Path('.'),review,current,final),final)
        self.assertNotIn(index,current)
        different={catalog:('100644','blob','other'),index:blob}
        self.assertEqual(pub.complete_pending_bundle_index(Path('.'),review,current,different),current)
        existing={**current,index:('100644','blob','original')}
        self.assertEqual(pub.complete_pending_bundle_index(Path('.'),review,existing,final),existing)
        self.assertEqual(pub.complete_pending_bundle_index(Path('.'),review,current,current),current)

    def test_exact_approved_immutable_readback_is_reused_only_for_bound_metadata(self):
        digest = 'a'*64
        url = pub.asset_url(pub.PAYLOAD_REPO, 'v430', 'patch-' + digest[:12] + '.zip')
        receipt = {'schema':1, 'scope':'immutable-resource-readback',
                   'verified_at':datetime.now(timezone.utc).isoformat(),
                   'metadata_sha256':['b'*64], 'assets':{url:{'bytes':10,'sha256':digest}}}
        with patch.dict(os.environ, {'RTXFG_RESOURCE_RECEIPTS':json.dumps(receipt)}), patch.object(pub,'verify') as verify:
            pub.verify_metadata_artifact(url,10,digest,'b'*64)
            verify.assert_not_called()
            pub.verify_metadata_artifact(url,10,digest,'c'*64)
            pub.verify_metadata_artifact(url,11,digest,'b'*64)
            pub.verify_metadata_artifact(url,10,'d'*64,'b'*64)
            self.assertEqual(verify.call_count,3)
        receipt['verified_at']=(datetime.now(timezone.utc)-timedelta(days=8)).isoformat()
        with patch.dict(os.environ, {'RTXFG_RESOURCE_RECEIPTS':json.dumps(receipt)}), patch.object(pub,'verify') as verify:
            pub.verify_metadata_artifact(url,10,digest,'b'*64)
            verify.assert_called_once()

    def test_receipts_never_approve_exe_or_nonimmutable_resource(self):
        for name in ['RTXManager-v4.3.0-x64.exe', 'update.json', 'mutable.zip']:
            url=pub.asset_url(pub.PAYLOAD_REPO,'v430',name)
            receipt={'schema':1,'scope':'immutable-resource-readback',
                     'verified_at':datetime.now(timezone.utc).isoformat(),
                     'metadata_sha256':['b'*64],'assets':{url:{'bytes':10,'sha256':'a'*64}}}
            with patch.dict(os.environ,{'RTXFG_RESOURCE_RECEIPTS':json.dumps(receipt)}), patch.object(pub,'verify') as verify:
                pub.verify_metadata_artifact(url,10,'a'*64,'b'*64)
                verify.assert_called_once()
        with patch.dict(os.environ,{'RTXFG_RESOURCE_RECEIPTS':'{}'}), self.assertRaisesRegex(RuntimeError,'Invalid publisher'):
            pub.verify_metadata_artifact(url,10,'a'*64,'b'*64)

    def test_ci_cannot_upload_exe_even_with_override(self):
        with tempfile.TemporaryDirectory() as d:
            p=Path(d)/'RTXManager-v4.3.0-x64.exe';p.write_bytes(b'unsigned fixture')
            with patch.dict(os.environ,{'GITHUB_ACTIONS':'true'}), self.assertRaisesRegex(RuntimeError,'publisher machine'):
                pub.upload(pub.GC_REPO,'v4.3.0',p,True)

    def test_update_waits_for_both_exact_executables(self):
        m=dict(schema=1,version='4.3.0',file='RTXManager-v4.3.0-x64.exe',bytes=1048576,sha256='a'*64)
        with patch.object(pub,'verify') as verify:
            pub.verify_update(json.dumps(m))
            self.assertEqual(verify.call_count,2)
            self.assertIn(pub.GC_REPO,verify.call_args_list[0].args[0])
            self.assertIn(pub.GH_REPO,verify.call_args_list[1].args[0])
        m['file']='../different.exe'
        with self.assertRaises(RuntimeError):pub.verify_update(json.dumps(m))

    def test_update_last_and_no_ci_exe_upload(self):
        with tempfile.TemporaryDirectory() as d:
            for name in ['update.json','RTXManager-v4.3.0-x64.exe','SHA256SUMS.txt','third-party-info.json']:
                (Path(d)/name).write_text('{}')
            (Path(d)/'update.json').write_text(json.dumps(dict(schema=1, version='4.3.0',
                file='RTXManager-v4.3.0-x64.exe', bytes=1048576, sha256='a'*64)))
            events=[]
            with (patch.object(pub,'verify_update',side_effect=lambda _:events.append('verify')),
                 patch.object(pub,'upload',side_effect=lambda repo,tag,path,allow:events.append(path.name))):
                pub.mirror_assets(Path(d),pub.GC_REPO,'v4.3.0')
            self.assertEqual(events[-2:],['verify','update.json'])
            self.assertFalse(any(e.endswith('.exe') for e in events))

    def test_existing_asset_must_match_and_is_never_replaced(self):
        with tempfile.TemporaryDirectory() as d:
            p=Path(d)/'data.zip';p.write_bytes(b'zip fixture')
            with (patch.object(pub,'api',return_value={'assets':[{'name':p.name}]}) as api,
                 patch.object(pub,'verify',side_effect=RuntimeError('different'))):
                with self.assertRaisesRegex(RuntimeError,'different'):pub.upload(pub.PAYLOAD_REPO,'v430',p)
                self.assertEqual(api.call_count,1)

    def test_paths_and_transport_reject_credentials_and_traversal(self):
        for name in ['../data.zip','x/y.zip','data.zip?token=x']:
            with self.assertRaises(RuntimeError):pub.asset_url(pub.PAYLOAD_REPO,'v430',name)
        for url in ['http://gitcode.com/x','https://gitcode.com:444/x','https://gitcode.com/x#fragment']:
            with self.assertRaises(RuntimeError):pub.safe_https(url)

    def test_metadata_cannot_be_deleted_or_immutable_index_replaced(self):
        review = Mock()
        with self.assertRaisesRegex(RuntimeError,'deleted'):
            pub.verify_metadata_changes(Path('.'), review, {'cloud/help.json':('100644','blob','old')}, {})
        revision = 'r-' + 'a'*20
        index = 'cloud/v3/indexes/' + revision + '.json'
        catalog = ('100644','blob','catalog')
        before = {'cloud/v3/catalog.json':catalog,index:('100644','blob','old')}
        after = {**before,index:('100644','blob','new')}
        with self.assertRaisesRegex(RuntimeError,'immutable'):
            pub.verify_metadata_changes(Path('.'),review,before,after)

    def test_new_catalog_checks_new_packages_and_reuses_legacy_sources(self):
        revision = 'r-'+'b'*20
        raw = json.dumps({'packages':[
            {'archive':'new.zip','bytes':10,'sha256':'c'*64},
            {'archive':'legacy.zip','bytes':20,'sha256':'d'*64,'urls':{
                'domestic':'https://gitee.com/pandaligx/RTX-FG-Manager-payloads/releases/download/old/legacy.zip'}}
        ]}).encode()
        catalog = json.dumps({'schema':3,'revision':revision,'index':{
            'url':pub.asset_url(pub.PAYLOAD_REPO,'v430',revision+'.json'),
            'bytes':len(raw),'sha256':hashlib.sha256(raw).hexdigest()},
            'sources':{'domestic':{'base_url':pub.asset_url(pub.PAYLOAD_REPO,'v430','new.zip')[:-7]}}}).encode()
        review=Mock()
        review.git.side_effect=lambda root,*args,**kwargs: {'catalog':catalog,'index':raw}[args[-1]]
        after={'cloud/v3/catalog.json':('100644','blob','catalog'),
               'cloud/v3/indexes/'+revision+'.json':('100644','blob','index')}
        with patch.object(pub,'verify') as verify:
            pub.verify_metadata_changes(Path('.'),review,{},after)
            self.assertEqual(verify.call_count,2)
            self.assertTrue(all('gitcode.com' in call.args[0] for call in verify.call_args_list))

    def test_help_requires_increasing_content_version(self):
        review=Mock()
        review.git.side_effect=lambda root,*args,**kwargs: json.dumps({
            'schema':1,'version':'4.3.0.1','urls':[]}).encode()
        with self.assertRaisesRegex(RuntimeError,'newer content version'):
            pub.verify_metadata_changes(Path('.'),review,
                {'cloud/help.json':('100644','blob','old')},
                {'cloud/help.json':('100644','blob','new')})

    def test_registration_only_recovery_keeps_all_immutable_inputs(self):
        import copy
        old = {'schema':3, 'revision':'bundles-'+'a'*20, 'schemes':[{'id':'old'}]}
        added = {'id':'new', 'profile':'upstream041', 'archives':['new.zip'],
                 'defaults':{}, 'capabilities':['delta_force_mfg_v1']}
        current = {**old, 'schemes':old['schemes']+[added]}
        index = {'packages':[{'scheme_id':'new', 'archive':'new.zip'}]}
        embedded = {'scheme_policies':{'new':{'parameter_profile':'upstream041',
                                            'capabilities':['delta_force_mfg_v1']}}}
        pub.validate_bundle_registration(old, current, index, embedded)
        invalid = []
        for key, value in [('profile','wrong'), ('archives',['other.zip']),
                           ('defaults',{'Optimized':'2'}), ('capabilities',[])]:
            changed = copy.deepcopy(current)
            changed['schemes'][1][key] = value
            invalid.append(changed)
        invalid.extend([{**current,'revision':'bundles-'+'b'*20},
                        {**current,'schemes':[added]},
                        {**current,'schemes':[{'id':'old','name':'changed'},added]},
                        {**current,'schemes':current['schemes']+[added]}])
        for changed in invalid:
            with self.subTest(changed=changed), self.assertRaises(RuntimeError):
                pub.validate_bundle_registration(old, changed, index, embedded)

    def test_live_registration_reuses_only_prior_metadata_readback(self):
        root = Path(__file__).resolve().parents[1]
        # Git blobs keep LF even when the Windows checkout uses CRLF.
        current = (root/'cloud/bundles/catalog.json').read_text('utf-8').encode()
        catalog = json.loads(current)
        prior = {**catalog, 'schemes':[s for s in catalog['schemes']
                                      if s['id'] != 'rtxfg-0.4.1-dx12-vulkan']}
        prior_raw = json.dumps(prior).encode()
        index = 'cloud/bundles/indexes/'+catalog['revision']+'.json'
        contents = {'old':prior_raw, 'new':current, 'index':(root/index).read_text('utf-8').encode(),
                    'embedded':(root/'rust/cloud-catalog.json').read_bytes()}
        before = {'cloud/bundles/catalog.json':('100644','blob','old'),
                  index:('100644','blob','index'),
                  'rust/cloud-catalog.json':('100644','blob','embedded')}
        after = {**before, 'cloud/bundles/catalog.json':('100644','blob','new')}
        review = Mock()
        review.git.side_effect = lambda root,*args,**kwargs:contents[args[-1]]
        with patch.object(pub,'verify_metadata_artifact') as verify:
            pub.verify_metadata_changes(root,review,before,after)
            self.assertGreater(verify.call_count,1)
            self.assertTrue(all(c.args[-1] == hashlib.sha256(prior_raw).hexdigest()
                                for c in verify.call_args_list))
        for name in [index,'rust/cloud-catalog.json']:
            changed = {**after, name:('100644','blob','new')}
            with self.assertRaisesRegex(RuntimeError,'immutable'):
                pub.verify_metadata_changes(root,review,before,changed)

    def test_complete_bundle_verifies_once_and_rejects_conflicting_identity(self):
        revision='bundles-'+'e'*20
        url=pub.asset_url(pub.PAYLOAD_REPO,'payloads','scheme-complete-'+'a'*12+'.zip')
        package={'archive':url.rsplit('/',1)[1],'archive_prefix':'loader-one',
                 'bytes':30,'sha256':'a'*64,'urls':{'domestic':url}}
        def metadata(packages):
            raw=json.dumps({'packages':packages}).encode()
            catalog=json.dumps({'schema':3,'revision':revision,'index':{
                'url':pub.asset_url(pub.PAYLOAD_REPO,'payloads',revision+'.json'),
                'bytes':len(raw),'sha256':hashlib.sha256(raw).hexdigest()}}).encode()
            review=Mock()
            review.git.side_effect=lambda root,*args,**kwargs:{'catalog':catalog,'index':raw}[args[-1]]
            return review,{'cloud/bundles/catalog.json':('100644','blob','catalog'),
                          'cloud/bundles/indexes/'+revision+'.json':('100644','blob','index')}
        review,after=metadata([package,{**package,'archive_prefix':'loader-two'}])
        with patch.object(pub,'verify') as verify:
            pub.verify_metadata_changes(Path('.'),review,{},after)
            self.assertEqual(verify.call_count,2) # one index, one complete archive
        review,after=metadata([package,{**package,'bytes':31}])
        with patch.object(pub,'verify'),self.assertRaisesRegex(RuntimeError,'Conflicting'):
            pub.verify_metadata_changes(Path('.'),review,{},after)


if __name__=='__main__':unittest.main()
