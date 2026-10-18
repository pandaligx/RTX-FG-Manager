import importlib.util
import hashlib
import json
import os
from pathlib import Path
import tempfile
import unittest
from datetime import datetime, timezone, timedelta
from unittest.mock import patch, Mock

spec=importlib.util.spec_from_file_location('gitcode_publish',Path(__file__).resolve().parents[1]/'tools/gitcode_publish.py')
pub=importlib.util.module_from_spec(spec)
spec.loader.exec_module(pub)


class PublicationTests(unittest.TestCase):
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
        m=dict(schema=1,version='4.3.0',file='RTXManager-v4.3.0-x64.exe',bytes=1000000,sha256='a'*64)
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
