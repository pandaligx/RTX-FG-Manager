"""Fast-forward reviewed manager sources and verify stable Gitee release assets.

The source mirror cannot promote cloud catalogs/indexes. Only cloud_release.py
may do that, after both resource sites have been verified. EXEs are uploaded by
the publisher from their own machine, never by this Actions script.
"""
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import time
import http.client
import urllib.error
import urllib.parse
import urllib.request
import uuid

REPO='pandaligx/RTX-FG-Manager'
API='https://gitee.com/api/v5/repos/'+REPO
# Keep in step with tools/export-source.ps1. This is deliberately not a recursive
# copy of the private checkout, nor an unrestricted mirror of arbitrary commits.
ALLOWED = {
    'Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml', '.cargo/config.toml', '.gitignore',
    'BUILDING.md', 'LICENSE', 'THIRD_PARTY_NOTICES.txt', 'rust/build.rs',
    'rust/cloud-catalog.json', 'rust/cloud-identities.json', 'rust/delta-runtime.json',
    'rust/ui-translations.json', 'app/assets/cleanup-catalog.json',
    'app/assets/FONTAWESOME_LICENSE.txt', 'app/assets/licenses/aria2/COPYING',
    'app/locales/en.json', 'app/locales/ru.json', 'app/locales/ja.json', 'app/locales/ko.json',
    'app/tools/aria2.conf', 'tools/build-resources.json', 'tools/Get-VerifiedResources.ps1',
    'tools/prepare-build.ps1', 'tools/prepare-fixtures.ps1', 'tools/export-source.ps1',
    'tests/fixture-manifest.json', 'tests/source_export.ps1', 'tests/test_publication.py',
    'vendor/gpui_windows/Cargo.toml', 'vendor/gpui_windows/build.rs',
    'vendor/gpui_windows/LICENSE-APACHE', 'vendor/sum_tree/Cargo.toml',
    'vendor/sum_tree/LICENSE-APACHE', 'vendor/sum_tree/LOCAL_CHANGES.md',
    'README.md', 'README.zh-CN.md', 'CHANGELOG.md', 'CHANGELOG.zh-CN.md',
    'CONTRIBUTING.md', 'SECURITY.md', 'cloud/schemes.json', 'cloud/catalog.json',
    'tools/cloud_release.py', 'docs/cloud-publishing.md', '.github/scripts/mirror_gitee.py',
    '.github/workflows/build.yml', '.github/workflows/mirror-gitee.yml',
    '.github/workflows/payloads.yml', '.github/workflows/cloud-probe.yml',
    'docs/screenshot-home.png', 'docs/screenshot-home-themes.png',
    # Existing public history has this screenshot; retaining its history is safe.
    'docs/screenshot-home-zh.png',
}
TREES = (
    (r'rust/src/.+', {'.rs'}),
    (r'rust/assets/.+', {'.json', '.svg', '.png', '.ico', '.txt', '.md'}),
    (r'vendor/gpui_windows/src/.+', {'.rs', '.hlsl'}),
    (r'vendor/sum_tree/src/.+', {'.rs'}),
    (r'tests/[^/]+', {'.rs'}),
    (r'tests/fixtures/catalog(?:420|421|422)/.+', {'.json'}),
    (r'cloud/indexes/.+', {'.json'}),
)
FORBIDDEN = re.compile(
    r'(^|/)(development|runtime|payloads|log|\.git|\.workspace|target|\.codex|\.agents)(/|$)'
    r'|^rust/native(/|$)|\.(dll|exe|pfx|p12|key|pml|dmp)$'
    r'|(^|/)(agent\.md|AGENTS\.md|RELEASE_WORKFLOW\.md|\.env(?:\..*)?)$', re.I)
CONTENT_RULES = {
    'private-key material': r'-----BEGIN (?:RSA |EC |OPENSSH )?PRIVATE KEY-----',
    'GitHub credential': r'(?:gh[pousr]_[A-Za-z0-9]{30,}|github_pat_[A-Za-z0-9_]{30,})',
    'credential-bearing URL': r'''https?://[^\s/"'<>]+:[^\s/"'<>]+@[^\s"'<>]+''',
    'assigned long credential': r'''(?i)(?:access_token|gitee_token|github_token|token|api_key|auth_token|password|client_secret|secret)["']?\s*[:=]\s*["'][A-Za-z0-9_+/.=-]{20,}["']''',
    'developer home path': r'''(?i)[A-Z]:[\\/]+Users[\\/]+(?:Administrator|[^\\/\s"']+)[\\/]+(?:Desktop|AppData|Documents)''',
}
SYNTHETIC_INPUTS = {
    'rust/src/cloud.rs': '4f85bc59f748edddacc5e3639f8e3c99362c24cdeaf8adaed0dee75faded566c',
    'tests/v423_transfer.rs': 'e4b4359e2f12069916814ed61dcde4be96912ac5b4c35b87b2822cf2d4b83972',
}


def allowed_source(path):
    if not path or '\\' in path or any(p in ('', '.', '..') for p in path.split('/')):
        return False
    if FORBIDDEN.search(path):
        return False
    return path in ALLOWED or any(re.fullmatch(pattern, path) and Path(path).suffix.lower() in extensions
                                 for pattern, extensions in TREES)


def validate_content(path, data):
    if len(data) > 20 * 1024 * 1024:
        raise RuntimeError('Oversized source input: ' + path)
    if Path(path).suffix.lower() in ('.png', '.ico'):
        return
    try:
        text = data.decode('utf-8-sig')
    except UnicodeDecodeError as exc:
        raise RuntimeError('Non-text source input: ' + path) from exc
    for label, pattern in CONTENT_RULES.items():
        for match in re.finditer(pattern, text):
            synthetic = (label == 'credential-bearing URL'
                         and path in SYNTHETIC_INPUTS
                         and (path.startswith('tests/') or '#[cfg(test)]' in text[:match.start()])
                         and hashlib.sha256(match.group().encode()).hexdigest() == SYNTHETIC_INPUTS[path])
            if not synthetic:
                # Never print a matched token or private path in workflow logs.
                raise RuntimeError('Source content requires review: ' + path + ' [' + label + ']')


def git(root, *args, env=None, binary=False):
    output = subprocess.check_output(['git', '-C', str(root), *args], env=env,
                                     stderr=subprocess.PIPE, timeout=300, text=not binary)
    return output if binary else output.strip()


def tree_entries(root, ref):
    result = {}
    for entry in git(root, 'ls-tree', '-r', '-z', ref, binary=True).split(b'\0'):
        if not entry:
            continue
        metadata, path = entry.split(b'\t', 1)
        mode, kind, oid = metadata.decode('ascii').split()
        result[path.decode('utf-8')] = (mode, kind, oid)
    return result


def protected_metadata(entries):
    return {path: entry for path, entry in entries.items()
            if path == 'cloud/catalog.json' or path.startswith('cloud/indexes/')}


def validate_tree(root, entries, seen=None):
    seen = set() if seen is None else seen
    for path, (mode, kind, oid) in entries.items():
        if mode not in ('100644', '100755') or kind != 'blob' or not allowed_source(path):
            raise RuntimeError('Source publication allowlist rejected: ' + path)
        if (path, oid) not in seen:
            validate_content(path, git(root, 'cat-file', 'blob', oid, binary=True))
            seen.add((path, oid))


def ancestor(root, older, newer):
    result = subprocess.run(['git', '-C', str(root), 'merge-base', '--is-ancestor', older, newer],
                            stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, timeout=60)
    if result.returncode not in (0, 1):
        raise RuntimeError('Could not verify publication history')
    return result.returncode == 0


def assert_source_range(root, source, remote=None):
    """Gate every newly public commit, including private files later deleted.

    Source-only pushes cannot introduce, change, remove or roll back catalog or
    index blobs. A partial payload publication must be resumed by cloud_release.
    """
    seen = set()
    entries = tree_entries(root, source)
    validate_tree(root, entries, seen)
    baseline = protected_metadata(tree_entries(root, remote)) if remote else {}
    if protected_metadata(entries) != baseline:
        raise RuntimeError('Cloud metadata differs from Gitee; run tools/cloud_release.py publish to verify both sites before promotion')
    revision = remote + '..' + source if remote else source
    for commit in git(root, 'rev-list', '--reverse', revision).splitlines():
        entries = tree_entries(root, commit)
        validate_tree(root, entries, seen)
        parents = git(root, 'rev-list', '--parents', '-n', '1', commit).split()[1:]
        diff_args = [parents[0], commit] if parents else ['--root', commit]
        changed = git(root, 'diff-tree', '--no-commit-id', '--name-only', '--no-renames',
                      '-r', '-z', *diff_args, binary=True).decode('utf-8').split('\0')
        # Compare changes to the first parent, not every historical tree to the
        # current catalog: a normal feature branch can start before a promotion.
        if any(path == 'cloud/catalog.json' or path.startswith('cloud/indexes/') for path in changed):
            raise RuntimeError('Source history contains cloud metadata changes; resume tools/cloud_release.py publish first')


def sync_main(root, env, remote_url=None):
    """Push the exact reviewed Git history, never rewrite or synthesize commits."""
    remote_url = remote_url or 'https://gitee.com/' + REPO + '.git'
    source = git(root, 'rev-parse', 'refs/heads/main^{commit}')
    refs = git(root, '-c', 'credential.helper=', 'ls-remote', remote_url, 'refs/heads/main', env=env)
    remote = None
    if refs:
        # Fetch rather than trust a stale ls-remote result during concurrent pushes.
        git(root, '-c', 'credential.helper=', 'fetch', '--no-tags', remote_url,
            'refs/heads/main:refs/remotes/rtxfg-mirror/main', env=env)
        remote = git(root, 'rev-parse', 'refs/remotes/rtxfg-mirror/main^{commit}')
        if remote == source:
            validate_tree(root, tree_entries(root, source))
            return 'already-current'
        if ancestor(root, source, remote):
            # An earlier checkout must not roll back a newer catalog/source commit.
            return 'gitee-ahead'
        if not ancestor(root, remote, source):
            raise RuntimeError('GitHub/Gitee main histories diverged; refusing a non-fast-forward mirror')
    assert_source_range(root, source, remote)
    git(root, '-c', 'credential.helper=', 'push', remote_url,
        source + ':refs/heads/main', env=env)
    # A normal push also rejects any remote advance after the fetch above.
    refs = git(root, '-c', 'credential.helper=', 'ls-remote', remote_url, 'refs/heads/main', env=env)
    if not refs or refs.split()[0] != source:
        raise RuntimeError('Gitee branch readback differs after source push; inspect concurrent publication')
    return 'fast-forwarded' if remote else 'initialized'


def api(path, fields=None, file=None):
    if file is not None and file.suffix.lower() == '.exe':
        raise RuntimeError('EXE upload from Actions is prohibited; upload the signed EXE locally')
    headers={'User-Agent':'RTXFG-Publication-Mirror'}
    data=None
    if fields is not None:
        fields=dict(fields,access_token=os.environ['GITEE_TOKEN'])
        if file:
            boundary='RTXFG'+uuid.uuid4().hex
            chunks=[]
            for name,value in fields.items():
                chunks.append(f'--{boundary}\r\nContent-Disposition: form-data; name="{name}"\r\n\r\n{value}\r\n'.encode())
            chunks.append(f'--{boundary}\r\nContent-Disposition: form-data; name="file"; filename="{file.name}"\r\nContent-Type: application/octet-stream\r\n\r\n'.encode())
            chunks.extend([file.read_bytes(),f'\r\n--{boundary}--\r\n'.encode()])
            data=b''.join(chunks)
            headers['Content-Type']='multipart/form-data; boundary='+boundary
        else:
            data=urllib.parse.urlencode(fields).encode()
            headers['Content-Type']='application/x-www-form-urlencoded'
    with urllib.request.urlopen(urllib.request.Request(API+path,data=data,headers=headers),timeout=900 if file else 30) as r:
        return json.load(r)


def digest(path):
    with path.open('rb') as stream:return hashlib.file_digest(stream,'sha256').hexdigest()


def validate_assets(folder,tag):
    if any(not p.is_file() or p.is_symlink() for p in folder.iterdir()):
        raise RuntimeError('Release assets must be regular files')
    manifest=json.loads((folder/'update.json').read_text('utf-8'))
    name=f'RTXManager-{tag}-x64.exe'
    if manifest.get('schema')!=1 or manifest.get('version')!=tag[1:] or manifest.get('file')!=name:
        raise RuntimeError('Invalid release manifest')
    if type(manifest.get('bytes')) is not int or not 1024*1024<=manifest['bytes']<=256*1024*1024:
        raise RuntimeError('Invalid executable size')
    exe=folder/name
    if exe.stat().st_size!=manifest['bytes'] or digest(exe)!=manifest['sha256']:
        raise RuntimeError('GitHub EXE checksum mismatch')
    allowed={name,'update.json','SHA256SUMS.txt','third-party-info.json'}
    files={p.name for p in folder.iterdir() if p.is_file()}
    if files!=allowed:raise RuntimeError('Release assets are missing or unexpected')
    if any((folder/n).stat().st_size > 2*1024*1024 for n in allowed-{name}):
        raise RuntimeError('Unexpectedly large release metadata')
    entries={}
    for line in (folder/'SHA256SUMS.txt').read_text('utf-8').splitlines():
        sha,filename=line.split('  ',1)
        if not re.fullmatch('[0-9a-f]{64}',sha) or filename not in allowed-{'SHA256SUMS.txt'} or filename in entries:
            raise RuntimeError('Unsafe checksum manifest')
        entries[filename]=sha
    if set(entries)!=allowed-{'SHA256SUMS.txt'}:raise RuntimeError('Incomplete checksums')
    for filename,sha in entries.items():
        if digest(folder/filename)!=sha:raise RuntimeError('GitHub asset checksum mismatch: '+filename)
    return sorted(folder.iterdir(),key=lambda p:p.name=='update.json')


def verify_remote(url,expected):
    parsed=urllib.parse.urlsplit(url)
    if parsed.scheme!='https' or parsed.username or parsed.password or parsed.hostname not in ('gitee.com','gitee.cn') and not (parsed.hostname or '').endswith('.gitee.com'):
        raise RuntimeError('Unsafe mirror asset URL')
    h=hashlib.sha256();size=0;expected_size=expected.stat().st_size
    for attempt in range(4):
        headers={'User-Agent':'RTXFG-Publication-Mirror','Accept-Encoding':'identity'}
        if size:headers['Range']=f'bytes={size}-'
        try:
            with urllib.request.urlopen(urllib.request.Request(url,headers=headers),timeout=60) as response:
                if not response.url.startswith('https://'):raise RuntimeError('Insecure mirror redirect')
                if response.status==206:
                    match=re.fullmatch(r'bytes (\d+)-(\d+)/(\d+)',response.headers.get('Content-Range',''))
                    if not match or tuple(map(int,match.groups()))!=(size,expected_size-1,expected_size):
                        raise RuntimeError('Invalid mirror resume range')
                elif response.status==200:
                    h=hashlib.sha256();size=0
                else:raise RuntimeError('Unexpected mirror download status')
                while chunk:=response.read(256*1024):
                    h.update(chunk);size+=len(chunk)
                    if size>expected_size:raise RuntimeError('Mirror asset too large')
                if size!=expected_size:raise ConnectionError('Incomplete mirror download')
            break
        except (TimeoutError,ConnectionError,urllib.error.URLError,http.client.IncompleteRead):
            if attempt==3:raise
            print(f'Mirror download interrupted at {size} bytes; retry {attempt+1}/3',flush=True)
            time.sleep(2)
    if size!=expected.stat().st_size or h.hexdigest()!=digest(expected):
        raise RuntimeError('Mirror asset differs: '+expected.name)


def tag_push_required(remote_refs,tag,local_commit):
    """Accept annotated or lightweight tags only when their commits agree."""
    refs={}
    for line in remote_refs.splitlines():
        sha,ref=line.split()
        if not re.fullmatch('[0-9a-f]{40}',sha):raise RuntimeError('Invalid remote tag')
        refs[ref]=sha
    ref='refs/tags/'+tag
    if ref not in refs:return True
    if refs.get(ref+'^{}',refs[ref])!=local_commit:
        raise RuntimeError('Existing Gitee tag points to a different commit; refusing overwrite')
    return False


def verify_manual_exe(existing,exe):
    matches=[a for a in existing if a.get('name')==exe.name]
    if len(matches)>1:raise RuntimeError('Duplicate mirror EXE')
    if not matches:
        print('Waiting for publisher local EXE upload to Gitee; update manifest withheld. Rerun with this tag after upload.')
        return False
    asset=matches[0]
    verify_remote(asset.get('browser_download_url') or asset.get('download_url') or asset.get('url',''),exe)
    print('Verified locally uploaded mirror EXE: '+exe.name)
    return True


def ensure_release(tag,release,commit):
    try:remote=api('/releases/tags/'+tag)
    except urllib.error.HTTPError as exc:
        if exc.code!=404:raise
        remote=None
    # Gitee can return HTTP 200 with JSON null for a tag without a release.
    if remote is None:
        remote=api('/releases',{'tag_name':tag,'name':release['name'],'body':release['body'],
                               'prerelease':'false','target_commitish':commit})
    if not isinstance(remote,dict) or type(remote.get('id')) is not int:
        raise RuntimeError('Invalid Gitee release response')
    return remote


def read_remote_tag(args,env):
    for attempt in range(3):
        try:return subprocess.check_output(args,env=env,text=True,timeout=60)
        except subprocess.TimeoutExpired:
            if attempt==2:raise
            print('Gitee tag lookup timed out; retrying read-only query',flush=True)
            time.sleep(2)


def mirror_small_assets(existing, files, endpoint, exe):
    if not isinstance(existing, list):
        raise RuntimeError('Invalid Gitee attachments')
    if not verify_manual_exe(existing, exe):
        return False
    for file in sorted(files, key=lambda p: p.name == 'update.json'):
        if file.suffix.lower() == '.exe':
            continue
        matches = [a for a in existing if a.get('name') == file.name]
        if len(matches) > 1:
            raise RuntimeError('Duplicate mirror asset')
        if not matches:
            api(endpoint, {}, file)
        latest = api(endpoint + '?per_page=100')
        if not isinstance(latest, list):
            raise RuntimeError('Invalid Gitee attachments')
        found = [a for a in latest if a.get('name') == file.name]
        if len(found) != 1:
            raise RuntimeError('Attachment not visible after upload')
        verify_remote(found[0].get('browser_download_url') or found[0].get('download_url')
                      or found[0].get('url', ''), file)
        print('Verified mirror asset: ' + file.name)
    return True


def main():
    if not os.environ.get('GITEE_TOKEN'):
        raise RuntimeError('Configure repository secret GITEE_TOKEN; mirror has NOT completed')
    os.environ.setdefault('GITEE_USERNAME',REPO.split('/')[0])
    root = Path.cwd()
    origin = git(root, 'remote', 'get-url', 'origin').removesuffix('.git')
    if origin not in {'https://github.com/' + REPO, 'git@github.com:' + REPO}:
        raise RuntimeError('Run only from the reviewed GitHub publication checkout')
    if git(root, 'status', '--porcelain'):
        raise RuntimeError('Publication checkout must be clean')
    tag=os.environ.get('TAG_NAME','')
    if tag and not re.fullmatch(r'v\d{1,4}\.\d{1,4}\.\d{1,4}',tag):raise RuntimeError('Invalid stable tag')
    # Gitee repository must be created by its owner before enabling synchronization.
    api('')
    with tempfile.TemporaryDirectory(prefix='rtxfg-mirror-') as tmp:
        askpass=Path(tmp)/'askpass.py'
        askpass.write_text('#!/usr/bin/env python3\nimport os,sys\nprint(os.environ["GITEE_USERNAME"] if "username" in sys.argv[1].lower() else os.environ["GITEE_TOKEN"])\n')
        askpass.chmod(0o700)
        env=dict(os.environ,GIT_ASKPASS=str(askpass),GIT_TERMINAL_PROMPT='0')
        result = sync_main(root, env)
        print('Source/documentation mirror: ' + result)
        if tag:
            remote_refs=read_remote_tag(['git','-c','credential.helper=','ls-remote',
                'https://gitee.com/'+REPO+'.git',f'refs/tags/{tag}',f'refs/tags/{tag}^{{}}'],env)
            commit=subprocess.check_output(['git','rev-parse',f'{tag}^{{commit}}'],text=True).strip()
            if not ancestor(root, commit, 'refs/heads/main'):
                raise RuntimeError('Release tag is outside reviewed main history')
            if tag_push_required(remote_refs,tag,commit):
                git(root, '-c', 'credential.helper=', 'push', 'https://gitee.com/' + REPO + '.git',
                    f'refs/tags/{tag}:refs/tags/{tag}', env=env)
        if not tag:
            print('Source/documentation mirror completed; cloud metadata was not promoted.');return
        assets=Path(tmp)/'assets';assets.mkdir()
        subprocess.run(['gh','release','download',tag,'--repo',REPO,'--dir',str(assets)],check=True,timeout=900)
        files=validate_assets(assets,tag)
        release=json.loads(subprocess.check_output(['gh','release','view',tag,'--repo',REPO,'--json','name,body,isDraft,isPrerelease']))
        if release['isDraft'] or release['isPrerelease']:raise RuntimeError('Only stable published releases are mirrored')
        commit=subprocess.check_output(['git','rev-list','-n','1',tag],text=True).strip()
        remote=ensure_release(tag,release,commit)
        rid=remote.get('id')
        if type(rid) is not int:raise RuntimeError('Invalid Gitee release response')
        endpoint=f'/releases/{rid}/attach_files'
        existing=api(endpoint+'?per_page=100')
        # Large EXEs are uploaded from the publisher machine, never from Actions.
        exe=assets/f'RTXManager-{tag}-x64.exe'
        if not mirror_small_assets(existing, files, endpoint, exe):return
        print('Release mirror completed; GitHub and Gitee downloads match.')


if __name__=='__main__':main()
