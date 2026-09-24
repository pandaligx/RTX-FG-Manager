"""Release mirror rejects incomplete/tampered assets before any upload."""
import hashlib,json,runpy,tempfile,unittest,io,os,subprocess
from unittest.mock import patch,Mock
from urllib.error import HTTPError
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
mirror=runpy.run_path(str(ROOT/'.github/scripts/mirror_gitee.py'))

class PublicationTests(unittest.TestCase):
    def test_remote_download_resumes_and_restarts_if_range_is_ignored(self):
        function=mirror['verify_remote']
        class Response(io.BytesIO):
            url='https://gitee.com/file'
            def __init__(self,data,status=200,content_range='',interrupt=False):
                super().__init__(data);self.status=status;self.headers={'Content-Range':content_range};self.interrupt=interrupt
            def read(self,n=-1):
                if self.interrupt and self.tell():raise TimeoutError('interrupted')
                return super().read(n)
        with tempfile.TemporaryDirectory() as td:
            file=Path(td)/'test.exe';file.write_bytes(b'a'*262144+b'b'*262144)
            for status in (206,200):
                response=Response(b'b'*262144,206,'bytes 262144-524287/524288') if status==206 else Response(file.read_bytes())
                api=Mock(side_effect=[Response(b'a'*262144,interrupt=True),response])
                with patch('urllib.request.urlopen',api),patch('time.sleep'):
                    function('https://gitee.com/file',file)
                self.assertEqual(api.call_args.args[0].get_header('Range'),'bytes=262144-')
            with patch('urllib.request.urlopen',return_value=Response(b'z'*524288)):
                with self.assertRaises(RuntimeError):function('https://gitee.com/file',file)
            with patch('urllib.request.urlopen',return_value=Response(b'b'*262144,206,'bytes 1-262144/524288')):
                with self.assertRaises(RuntimeError):function('https://gitee.com/file',file)

    def test_missing_release_null_or_404_creates_once_and_invalid_response_fails(self):
        function=mirror['ensure_release'];release={'name':'3.7.3','body':'Changes since 3.7.0'}
        for missing in (None,HTTPError('https://gitee.com',404,'missing',{},None)):
            api=Mock(side_effect=[missing,{'id':123}])
            with patch.dict(function.__globals__,api=api):
                self.assertEqual(function('v3.7.3',release,'a'*40),{'id':123})
            self.assertEqual(api.call_count,2)
            self.assertEqual(api.call_args.args[1]['body'],release['body'])
        api=Mock(return_value={'id':123})
        with patch.dict(function.__globals__,api=api):function('v3.7.3',release,'a'*40)
        self.assertEqual(api.call_count,1)
        for invalid in ([],{'id':'123'}):
            api=Mock(return_value=invalid)
            with patch.dict(function.__globals__,api=api):
                with self.assertRaises(RuntimeError):function('v3.7.3',release,'a'*40)
            self.assertEqual(api.call_count,1)

    def test_missing_local_upload_waits_and_duplicate_is_rejected(self):
        exe=Path('RTXManager-v3.7.0-x64.exe')
        self.assertFalse(mirror['verify_manual_exe']([],exe))
        asset={'name':exe.name,'browser_download_url':'https://gitee.com/example'}
        with self.assertRaises(RuntimeError):mirror['verify_manual_exe']([asset,asset],exe)
        function=mirror['verify_manual_exe']
        with patch.dict(function.__globals__,verify_remote=lambda url,path: None):
            self.assertTrue(function([asset],exe))
        with patch.dict(function.__globals__,verify_remote=lambda url,path: (_ for _ in ()).throw(RuntimeError('mismatch'))):
            with self.assertRaises(RuntimeError):function([asset],exe)

    def test_tag_sync_preserves_equivalent_annotated_and_lightweight_tags(self):
        commit='a'*40;annotation='b'*40;ref='refs/tags/v3.7.0'
        check=mirror['tag_push_required']
        self.assertTrue(check('', 'v3.7.0',commit))
        self.assertFalse(check(commit+'\t'+ref,'v3.7.0',commit))
        self.assertFalse(check(annotation+'\t'+ref+'\n'+commit+'\t'+ref+'^{}','v3.7.0',commit))
        with self.assertRaises(RuntimeError):check(annotation+'\t'+ref,'v3.7.0',commit)

    def assets(self,folder):
        exe=folder/'RTXManager-v3.7.0-x64.exe';exe.write_bytes(b'x'*1048576)
        (folder/'third-party-info.json').write_bytes(b'source fixture')
        manifest={'schema':1,'version':'3.7.0','file':exe.name,'bytes':exe.stat().st_size,'sha256':hashlib.sha256(exe.read_bytes()).hexdigest()}
        (folder/'update.json').write_text(json.dumps(manifest),'utf-8')
        files=[exe,folder/'update.json',folder/'third-party-info.json']
        (folder/'SHA256SUMS.txt').write_text(''.join(mirror['digest'](f)+'  '+f.name+'\n' for f in files),'utf-8')

    def test_manifest_uploaded_last(self):
        with tempfile.TemporaryDirectory() as tmp:
            folder=Path(tmp);self.assets(folder)
            self.assertEqual(mirror['validate_assets'](folder,'v3.7.0')[-1].name,'update.json')

    def test_extra_source_or_changed_exe_is_rejected(self):
        for change in ('extra','exe','missing'):
            with self.subTest(change=change),tempfile.TemporaryDirectory() as tmp:
                folder=Path(tmp);self.assets(folder)
                if change=='extra':(folder/'private-main.py').write_text('not for publication')
                elif change=='exe':(folder/'RTXManager-v3.7.0-x64.exe').write_bytes(b'changed')
                else:(folder/'third-party-info.json').unlink()
                with self.assertRaises(RuntimeError):mirror['validate_assets'](folder,'v3.7.0')

    def test_checksum_path_injection_is_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            folder=Path(tmp);self.assets(folder)
            (folder/'SHA256SUMS.txt').write_text('a'*64+'  ../private-main.py\n')
            with self.assertRaises(RuntimeError):mirror['validate_assets'](folder,'v3.7.0')

    def test_actions_cannot_upload_an_exe_even_through_api_helper(self):
        with patch('urllib.request.urlopen') as network:
            with self.assertRaisesRegex(RuntimeError, 'EXE upload'):
                mirror['api']('/releases/1/attach_files', {}, Path('RTXManager.exe'))
            network.assert_not_called()

    def test_small_assets_wait_for_exe_and_publish_update_last(self):
        function = mirror['mirror_small_assets']
        with tempfile.TemporaryDirectory() as tmp:
            folder = Path(tmp)
            self.assets(folder)
            files = mirror['validate_assets'](folder, 'v3.7.0')
            exe = folder / 'RTXManager-v3.7.0-x64.exe'
            attachments = [{'name': exe.name, 'url': 'https://gitee.com/example'}]
            events = []

            def api(path, fields=None, file=None):
                if file:
                    events.append(('upload', file.name))
                    attachments.append({'name': file.name, 'url': 'https://gitee.com/example'})
                return list(attachments)

            def verify(url, file):
                events.append(('verify', file.name))

            with patch.dict(function.__globals__, api=api, verify_remote=verify):
                self.assertFalse(function([], files, '/releases/1/attach_files', exe))
                self.assertEqual(events, [])
                self.assertTrue(function(attachments, list(reversed(files)), '/releases/1/attach_files', exe))
            self.assertEqual(events[0], ('verify', exe.name))
            self.assertEqual(events[-2:], [('upload', 'update.json'), ('verify', 'update.json')])
            self.assertNotIn(('upload', exe.name), events)

    def test_metadata_failure_withholds_update_and_never_overwrites(self):
        function = mirror['mirror_small_assets']
        with tempfile.TemporaryDirectory() as tmp:
            folder = Path(tmp)
            self.assets(folder)
            files = mirror['validate_assets'](folder, 'v3.7.0')
            exe = folder / 'RTXManager-v3.7.0-x64.exe'
            attachments = [{'name': p.name, 'url': 'https://gitee.com/example'} for p in files]
            api = Mock(return_value=attachments)

            def verify(url, file):
                if file.suffix != '.exe':
                    raise RuntimeError('mismatch')

            with patch.dict(function.__globals__, api=api, verify_remote=verify):
                with self.assertRaisesRegex(RuntimeError, 'mismatch'):
                    function(attachments, files, '/releases/1/attach_files', exe)
            self.assertTrue(all(len(call.args) == 1 for call in api.call_args_list))


class SourceMirrorTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix='rtxfg-mirror-test-')
        self.addCleanup(self.tmp.cleanup)
        self.base = Path(self.tmp.name)
        self.repo = self.base / 'source'
        self.remote = self.base / 'gitee.git'
        self.env = dict(os.environ, GIT_CONFIG_NOSYSTEM='1', GIT_CONFIG_GLOBAL=os.devnull,
                        GIT_AUTHOR_NAME='Offline test', GIT_AUTHOR_EMAIL='offline@example.invalid',
                        GIT_COMMITTER_NAME='Offline test', GIT_COMMITTER_EMAIL='offline@example.invalid')
        self.git('init', '--initial-branch=main', str(self.repo))
        self.git('init', '--bare', '--initial-branch=main', str(self.remote))
        self.write('README.md', 'Public manager documentation\n')
        self.commit('Initial public documents')
        self.git('push', str(self.remote), 'main:main', cwd=self.repo)

    def git(self, *args, cwd=None):
        return subprocess.check_output(['git', '-c', 'commit.gpgsign=false', *args],
                                       cwd=cwd or self.base, env=self.env,
                                       stderr=subprocess.PIPE, text=True).strip()

    def write(self, relative, contents):
        path = self.repo / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(contents, encoding='utf-8')

    def commit(self, message):
        self.git('add', '--all', cwd=self.repo)
        self.git('commit', '-m', message, cwd=self.repo)
        return self.git('rev-parse', 'HEAD', cwd=self.repo)

    def sync(self):
        # Git uses an isolated local bare repository. No HTTP request is permitted.
        with patch('urllib.request.urlopen', side_effect=AssertionError('offline test made an HTTP request')):
            return mirror['sync_main'](self.repo, self.env, str(self.remote))

    def remote_head(self):
        return self.git('--git-dir=' + str(self.remote), 'rev-parse', 'main')

    def test_source_sync_keeps_identical_history_and_is_idempotent(self):
        self.write('Cargo.toml', '[package]\nname="fixture"\n')
        self.write('rust/src/core.rs', '// public manager source\n')
        self.write('cloud/schemes.json', '{"maintainer":"public presets only"}\n')
        head = self.commit('Reviewed Rust source')
        self.assertEqual(self.sync(), 'fast-forwarded')
        self.assertEqual(self.remote_head(), head)
        self.assertEqual(self.sync(), 'already-current')

    def test_private_file_in_intermediate_commit_is_rejected(self):
        original = self.remote_head()
        self.write('development/private.py', 'private research\n')
        self.commit('Accidental private input')
        (self.repo / 'development/private.py').unlink()
        self.write('README.md', 'Public documentation again\n')
        self.commit('Remove private input')
        with self.assertRaisesRegex(RuntimeError, 'allowlist'):
            self.sync()
        self.assertEqual(self.remote_head(), original)

    def test_source_mirror_cannot_add_replace_remove_or_revert_cloud_metadata(self):
        self.write('cloud/catalog.json', '{"revision":"verified"}\n')
        self.write('cloud/indexes/verified.json', '{"packages":[]}\n')
        self.commit('Previously verified cloud metadata')
        self.git('push', str(self.remote), 'main:main', cwd=self.repo)
        original = self.remote_head()
        mutations = [('cloud/catalog.json', '{"revision":"unverified"}\n'),
                     ('cloud/indexes/new.json', '{}\n'), ('cloud/indexes/verified.json', None)]
        for path, value in mutations:
            with self.subTest(path=path):
                self.git('reset', '--hard', original, cwd=self.repo)
                if value is None:
                    (self.repo / path).unlink()
                else:
                    self.write(path, value)
                self.commit('Unverified cloud edit')
                with self.assertRaisesRegex(RuntimeError, 'Cloud metadata differs'):
                    self.sync()
                self.assertEqual(self.remote_head(), original)
        self.git('reset', '--hard', original, cwd=self.repo)
        self.write('cloud/catalog.json', '{"revision":"unverified"}\n')
        self.commit('Unverified intermediate metadata')
        self.write('cloud/catalog.json', '{"revision":"verified"}\n')
        self.commit('Restore old metadata')
        with self.assertRaisesRegex(RuntimeError, 'history contains cloud'):
            self.sync()
        self.assertEqual(self.remote_head(), original)

    def test_stale_checkout_never_rolls_back_newer_remote(self):
        original = self.remote_head()
        self.write('README.md', 'Newer reviewed docs\n')
        head = self.commit('Newer docs')
        self.git('push', str(self.remote), 'main:main', cwd=self.repo)
        self.git('reset', '--hard', original, cwd=self.repo)
        self.assertEqual(self.sync(), 'gitee-ahead')
        self.assertEqual(self.remote_head(), head)
        self.write('README.md', 'Diverged docs\n')
        self.commit('Independent history')
        with self.assertRaisesRegex(RuntimeError, 'histories diverged'):
            self.sync()
        self.assertEqual(self.remote_head(), head)

    def test_first_sync_cannot_bootstrap_an_unverified_catalog(self):
        empty = self.base / 'empty.git'
        self.git('init', '--bare', '--initial-branch=main', str(empty))
        self.write('cloud/catalog.json', '{"revision":"unverified"}\n')
        self.commit('Unverified initial catalog')
        with self.assertRaisesRegex(RuntimeError, 'Cloud metadata differs'):
            mirror['sync_main'](self.repo, self.env, str(empty))
        self.assertEqual(self.git('--git-dir=' + str(empty), 'for-each-ref', '--format=%(refname)'), '')

    def test_feature_branch_from_before_a_verified_catalog_can_merge_without_rollback(self):
        self.git('checkout', '-b', 'feature', cwd=self.repo)
        self.write('rust/src/core.rs', '// independently reviewed feature\n')
        self.commit('Feature started before promotion')
        self.git('checkout', 'main', cwd=self.repo)
        self.write('cloud/catalog.json', '{"revision":"already-verified"}\n')
        self.commit('Cloud publication completed on both sites')
        self.git('push', str(self.remote), 'main:main', cwd=self.repo)
        self.git('merge', '--no-ff', 'feature', '-m', 'Merge reviewed source', cwd=self.repo)
        head = self.git('rev-parse', 'HEAD', cwd=self.repo)
        self.assertEqual(self.sync(), 'fast-forwarded')
        self.assertEqual(self.remote_head(), head)
        self.assertEqual(self.git('--git-dir=' + str(self.remote), 'show', 'main:cloud/catalog.json'),
                         '{"revision":"already-verified"}')

    def test_private_content_and_links_are_rejected_without_printing_secrets(self):
        check = mirror['validate_content']
        token = 'ghp_' + 'A' * 40
        with self.assertRaisesRegex(RuntimeError, 'GitHub credential') as error:
            check('rust/src/main.rs', token.encode())
        self.assertNotIn(token, str(error.exception))
        with self.assertRaisesRegex(RuntimeError, 'allowlist'):
            mirror['validate_tree'](self.repo, {'rust/src/main.rs': ('120000', 'blob', 'a' * 40)})
        for path in ['payloads/version.dll', 'log/game.txt', 'rust/native/probe.cpp',
                     'agent.md', '.env', 'tests/../development/private.rs', 'rust/src/../../private.rs']:
            self.assertFalse(mirror['allowed_source'](path), path)

    def test_workflows_share_cloud_publication_lock(self):
        for name in ('mirror-gitee.yml', 'payloads.yml'):
            text = (ROOT / '.github/workflows' / name).read_text(encoding='utf-8')
            self.assertIn('group: rtxfg-cloud-publish', text)
            self.assertIn('cancel-in-progress: false', text)
