"""GitCode one-way publishing. Credentials stay in environment, never in URLs/logs.

Source input must be the reviewed public Git checkout. The desktop publisher
uploads signed EXEs; CI can mirror resource ZIPs and small release metadata.
GitCode API: docs.gitcode.com/docs/apis/get-api-v-5-repos-owner-repo-releases-tag-upload-url
"""
import argparse
from datetime import datetime, timezone
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request

GH_REPO = 'pandaligx/RTX-FG-Manager'
GC_REPO = 'gcw_qWIjmve7/RTX-FG-Manager'
PAYLOAD_REPO = GC_REPO + '-payloads'
API = 'https://api.gitcode.com/api/v5/repos/'
VERIFIED = set()


def verify_metadata_artifact(url, size, digest, metadata_digest):
    """Reuse a publisher-approved readback for identical immutable resources only.

    RTXFG_RESOURCE_RECEIPTS is a maintainer-controlled Actions repository variable,
    populated from completed local anonymous downloads, never from PR inputs.
    This cache cannot approve executables or advance a client update manifest.
    Missing, expired or nonmatching receipts use the normal download verifier.
    """
    encoded = os.environ.get('RTXFG_RESOURCE_RECEIPTS', '')
    if encoded:
        try:
            if len(encoded) > 48000:
                raise ValueError('oversized')
            receipt = json.loads(encoded)
            checked = datetime.fromisoformat(receipt['verified_at'])
            age = (datetime.now(timezone.utc) - checked).total_seconds()
            valid = (receipt['schema'] == 1 and receipt['scope'] == 'immutable-resource-readback'
                     and 0 <= age <= 7 * 86400 and metadata_digest in receipt['metadata_sha256'])
            prefixes = ('https://gitcode.com/' + PAYLOAD_REPO + '/releases/download/',
                        'https://github.com/' + GH_REPO + '/releases/download/')
            name = url.rsplit('/', 1)[-1]
            immutable = (url.startswith(prefixes) and (
                re.fullmatch(r'r-[0-9a-f]{20}\.json', name)
                or (re.fullmatch(r'[A-Za-z0-9._-]+-[0-9a-f]{12}\.zip', name)
                    and name.endswith('-' + digest[:12] + '.zip'))))
            if valid and immutable and type(size) is int and 0 < size <= 256 * 1024 * 1024:
                row = receipt['assets'].get(url)
                if row == {'bytes': size, 'sha256': digest} and re.fullmatch('[0-9a-f]{64}', digest):
                    print('Reused completed anonymous readback: ' + name)
                    return
        except (ValueError, KeyError, TypeError, AttributeError):
            raise RuntimeError('Invalid publisher resource receipt') from None
    verify(url, size, digest)


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


def safe_https(url):
    p = urllib.parse.urlsplit(url)
    if p.scheme != 'https' or not p.hostname or p.username or p.password or p.fragment or p.port not in (None, 443):
        raise RuntimeError('Unsafe publishing URL')
    return url


def api(repo, path, data=None, query=None, missing=False):
    if repo not in (GC_REPO, PAYLOAD_REPO):
        raise RuntimeError('Unexpected publication repository')
    token = os.environ.get('GITCODE_TOKEN', '')
    if not token:
        raise RuntimeError('GITCODE_TOKEN is required in the publisher environment')
    # Authorization header avoids a token in proxy logs, request URLs or errors.
    url = API + repo + path
    if query:
        url += '?' + urllib.parse.urlencode(query)
    req = urllib.request.Request(url, data=None if data is None else json.dumps(data).encode(),
        headers={'Authorization': 'Bearer ' + token, 'Content-Type': 'application/json'})
    try:
        with urllib.request.build_opener(NoRedirect).open(req, timeout=60) as response:
            return json.load(response)
    except urllib.error.HTTPError as e:
        if missing and e.code == 404:
            return None
        raise RuntimeError(f'GitCode API returned HTTP {e.code}; no credentials printed') from None
    except urllib.error.URLError:
        raise RuntimeError('GitCode API connection failed') from None


def public_bytes(url, limit):
    req = urllib.request.Request(safe_https(url), headers={'Accept-Encoding': 'identity', 'User-Agent': 'RTXFG-Publisher'})
    with urllib.request.urlopen(req, timeout=60) as response:
        safe_https(response.url)
        data = response.read(limit + 1)
    if len(data) > limit:
        raise RuntimeError('Public metadata exceeds size bound')
    return data


def verify(url, size, digest):
    if type(size) is not int or not 0 < size <= 256 * 1024 * 1024 or not re.fullmatch('[0-9a-f]{64}', digest):
        raise RuntimeError('Invalid artifact size/digest')
    key = (url, size, digest)
    if key in VERIFIED:
        return
    h, count = hashlib.sha256(), 0
    with urllib.request.urlopen(urllib.request.Request(safe_https(url), headers={'Accept-Encoding': 'identity'}), timeout=60) as response:
        safe_https(response.url)
        while chunk := response.read(1024 * 1024):
            count += len(chunk)
            if count > size:
                raise RuntimeError('Artifact exceeds expected size')
            h.update(chunk)
    if (count, h.hexdigest()) != (size, digest):
        raise RuntimeError('Published artifact differs from reviewed bytes')
    VERIFIED.add(key)


def asset_url(repo, tag, name):
    if repo not in (GC_REPO, PAYLOAD_REPO) or not re.fullmatch(r'[A-Za-z0-9._-]{1,100}', tag) or not re.fullmatch(r'[A-Za-z0-9._-]{1,160}', name):
        raise RuntimeError('Unsafe release path')
    return f'https://gitcode.com/{repo}/releases/download/{tag}/{name}'


def upload(repo, tag, path, allow_exe=False):
    path = Path(path)
    if path.is_symlink() or not path.is_file():
        raise RuntimeError('Asset must be a regular file')
    is_exe = path.suffix.lower() == '.exe'
    if is_exe and (not allow_exe or os.environ.get('CI') or os.environ.get('GITHUB_ACTIONS')):
        raise RuntimeError('Signed EXEs must be uploaded explicitly from the publisher machine')
    if path.suffix.lower() not in ('.zip', '.json', '.txt', '.exe'):
        raise RuntimeError('Unexpected asset type')
    data = path.read_bytes()
    if not data or len(data) > (256 if is_exe else 128) * 1024 * 1024:
        raise RuntimeError('Asset exceeds publication bound')
    digest = hashlib.sha256(data).hexdigest()
    url = asset_url(repo, tag, path.name)
    release = api(repo, '/releases/tags/' + tag)
    if any(a.get('name') == path.name for a in release['assets']):
        verify(url, len(data), digest)  # Never replace same-name immutable bytes.
        return
    ticket = api(repo, f'/releases/{tag}/upload_url', query={'file_name': path.name})
    endpoint = safe_https(ticket['url'])
    host = urllib.parse.urlsplit(endpoint).hostname
    if not (host.endswith('.myhuaweicloud.com') or host.endswith('.gitcode.com')):
        raise RuntimeError('Unexpected GitCode upload storage host')
    headers = ticket['headers']
    if any(k.lower() not in ('content-type', 'x-obs-meta-project-id', 'x-obs-acl', 'x-obs-callback') for k in headers):
        raise RuntimeError('Unexpected upload delegation headers')
    upload_ticket(endpoint, headers, path)
    verify(url, len(data), digest)
    print('Uploaded and verified: ' + path.name, flush=True)


def upload_ticket(endpoint, headers, path):
    # curl supplies bounded connect/transfer timings and IPv4 explicitly.
    # Delegated credentials stay on stdin, never in argv, a file or error logs.
    def quote(value):
        if any(ord(c) < 32 or ord(c) == 127 for c in value):
            raise RuntimeError('Invalid upload delegation value')
        return '"' + value.replace('\\', '\\\\').replace('"', '\\"') + '"'
    config = 'url = ' + quote(endpoint) + '\n'
    config += ''.join('header = ' + quote(k + ': ' + v) + '\n' for k,v in headers.items())
    stats = '{"http_code":%{http_code},"uploaded_bytes":%{size_upload},"bytes_per_second":%{speed_upload},"seconds":%{time_total},"connect_seconds":%{time_connect}}'
    started = time.monotonic()
    try:
        result = subprocess.run(['curl','--config','-','--ipv4','--silent','--show-error',
            '--proto','=https','--connect-timeout','15','--max-time','180',
            '--upload-file',str(path),'--output',os.devnull,'--write-out',stats],
            input=config,text=True,capture_output=True,timeout=190)
    except subprocess.TimeoutExpired:
        raise RuntimeError('GitCode upload exceeded 190 seconds; delegation withheld') from None
    try:
        report = json.loads(result.stdout)
        report = {k:report[k] for k in ('http_code','uploaded_bytes','bytes_per_second','seconds','connect_seconds')}
    except (ValueError,KeyError):
        report = {'seconds':round(time.monotonic()-started,3)}
    report['curl_exit'] = result.returncode
    print('GitCode upload transfer: '+json.dumps(report),flush=True)
    if result.returncode or not 200 <= report.get('http_code',0) < 300:
        raise RuntimeError('GitCode attachment upload failed; signed upload URL withheld')


def create_release(repo, tag, commit, title, body):
    asset_url(repo, tag, 'check.json')
    if api(repo, '/releases/tags/' + tag, missing=True) is None:
        api(repo, '/releases', {'tag_name': tag, 'target_commitish': commit,
            'name': title, 'body': body, 'release_status': 'pre'})


def verify_update(data):
    m = json.loads(data)
    v = m.get('version', '')
    if m.get('schema') != 1 or not re.fullmatch(r'\d{1,4}\.\d{1,4}\.\d{1,4}', v) or m.get('file') != f'RTXManager-v{v}-x64.exe':
        raise RuntimeError('Invalid update manifest')
    verify(asset_url(GC_REPO, 'v' + v, m['file']), m['bytes'], m['sha256'])
    verify(f'https://github.com/{GH_REPO}/releases/download/v{v}/{m["file"]}', m['bytes'], m['sha256'])


def mirror_assets(folder, repo, tag, allow_exe=False):
    files = list(Path(folder).iterdir())
    if not files or any(not p.is_file() or p.is_symlink() for p in files):
        raise RuntimeError('Asset directory must contain only reviewed regular files')
    manifest = next((p for p in files if p.name == 'update.json'), None)
    for path in sorted(files, key=lambda p: p.name == 'update.json'):
        if path.suffix.lower() == '.exe' and not allow_exe:
            continue
        if path == manifest:
            verify_update(path.read_bytes())
        upload(repo, tag, path, allow_exe)


def sync_source(root):
    spec = importlib.util.spec_from_file_location('review', root / '.github/scripts/mirror_gitee.py')
    review = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(review)
    remote = 'https://gitcode.com/' + GC_REPO + '.git'
    source = review.git(root, 'rev-parse', 'refs/heads/main^{commit}')
    token = os.environ.get('GITCODE_TOKEN')
    if not token:
        raise RuntimeError('GITCODE_TOKEN is required')
    with tempfile.TemporaryDirectory() as temp:
        ask = Path(temp) / 'askpass.py'
        ask.write_text('#!/usr/bin/env python3\nimport os,sys\nprint(os.environ["GITCODE_USERNAME"] if "username" in sys.argv[1].lower() else os.environ["GITCODE_TOKEN"])\n', encoding='utf-8')
        ask.chmod(0o700)
        env = {**os.environ, 'GIT_ASKPASS': str(ask), 'GIT_TERMINAL_PROMPT': '0', 'GITCODE_USERNAME': 'gcw_qWIjmve7'}
        review.git(root, '-c', 'credential.helper=', 'fetch', remote, 'main', env=env)
        previous = review.git(root, 'rev-parse', 'FETCH_HEAD')
        if not review.ancestor(root, previous, source):
            raise RuntimeError('GitCode has diverged; fast-forward required')
        seen = set()
        before = review.tree_entries(root, previous)
        for commit in review.git(root, 'rev-list', '--reverse', previous + '..' + source).splitlines():
            entries = review.tree_entries(root, commit)
            review.validate_tree(root, entries, seen)
            update = entries.get('update.json')
            if update != before.get('update.json'):
                if not update:
                    raise RuntimeError('Cannot remove transition update manifest')
                old = before.get('update.json')
                new_bytes = review.git(root, 'cat-file', 'blob', update[2], binary=True)
                old_bytes = review.git(root, 'cat-file', 'blob', old[2], binary=True) if old else None
                if review.static_update_action(old_bytes, new_bytes) == 'older-release':
                    raise RuntimeError('Update manifest cannot roll back')
                verify_update(new_bytes)
            # Legacy catalogs are a transition contract. Schema 3 references
            # are verified before their source commit becomes visible here.
            legacy = lambda e: {p:v for p,v in e.items() if p == 'cloud/catalog.json' or p.startswith('cloud/indexes/')}
            if legacy(entries) != legacy(before):
                raise RuntimeError('Legacy catalogs must remain unchanged during migration')
            verify_metadata_changes(root, review, before, entries)
            before = entries
        review.git(root, '-c', 'credential.helper=', 'push', remote, source + ':refs/heads/main', env=env)
        refs = review.git(root, '-c', 'credential.helper=', 'ls-remote', remote, 'refs/heads/main', env=env)
        if not refs or refs.split()[0] != source:
            raise RuntimeError('GitCode source readback mismatch')
        print('GitCode source fast-forward verified: ' + source)


def sync_release_tag(root, tag):
    if not re.fullmatch(r'v\d{1,4}\.\d{1,4}\.\d{1,4}', tag):
        raise RuntimeError('Only stable client tags can be mirrored to the source repository')
    def git(*args):
        return subprocess.check_output(['git', '-C', str(root), *args], text=True,
                                       stderr=subprocess.PIPE, timeout=120).strip()
    commit = git('rev-parse', 'refs/tags/' + tag + '^{commit}')
    subprocess.run(['git', '-C', str(root), 'merge-base', '--is-ancestor', commit,
                    'refs/heads/main'], check=True, capture_output=True, timeout=30)
    # Source has already been validated and fast-forwarded by sync_source.
    # API tag creation avoids passing a credential through a command argument.
    remote = 'https://gitcode.com/' + GC_REPO + '.git'
    def remote_commit():
        refs = dict(line.split()[::-1] for line in git('ls-remote', remote,
                    'refs/tags/' + tag, 'refs/tags/' + tag + '^{}').splitlines())
        return refs.get('refs/tags/' + tag + '^{}', refs.get('refs/tags/' + tag))
    existing = remote_commit()
    if existing is not None:
        if existing != commit:
            raise RuntimeError('Existing GitCode tag differs from reviewed source')
        return
    api(GC_REPO, '/tags', {'tag_name': tag, 'refs': commit})
    if remote_commit() != commit:
        raise RuntimeError('GitCode source tag readback mismatch')


def verify_metadata_changes(root, review, before, after):
    if any(p.startswith('cloud/') and p.endswith('.json') and p not in after for p in before):
        raise RuntimeError('Published metadata cannot be deleted')
    def blob(path):
        if path not in after:
            raise RuntimeError('Missing referenced metadata: ' + path)
        return review.git(root, 'cat-file', 'blob', after[path][2], binary=True)
    catalog_path = 'cloud/v3/catalog.json'
    if before.get(catalog_path) != after.get(catalog_path):
        c = json.loads(blob(catalog_path))
        if c.get('schema') != 3 or not re.fullmatch(r'r-[0-9a-f]{20}', c.get('revision', '')):
            raise RuntimeError('Invalid schema 3 catalog')
        index_path = f'cloud/v3/indexes/{c["revision"]}.json'
        if index_path in before:
            raise RuntimeError('A catalog change requires a new immutable revision')
        raw = blob(index_path)
        index = c['index']
        if len(raw) != index['bytes'] or hashlib.sha256(raw).hexdigest() != index['sha256']:
            raise RuntimeError('Index does not match catalog')
        expected_index = asset_url(PAYLOAD_REPO, 'v430', c['revision'] + '.json')
        if index['url'] != expected_index:
            raise RuntimeError('Unexpected domestic index location')
        metadata_digest = hashlib.sha256(blob(catalog_path)).hexdigest()
        verify_metadata_artifact(expected_index, len(raw), hashlib.sha256(raw).hexdigest(), metadata_digest)
        for package in json.loads(raw)['packages']:
            url = package.get('urls', {}).get('domestic') or c['sources']['domestic']['base_url'] + package['archive']
            if url.startswith('https://gitcode.com/' + PAYLOAD_REPO + '/releases/download/'):
                verify_metadata_artifact(url, package['bytes'], package['sha256'], metadata_digest)
            elif not package.get('urls', {}).get('domestic', '').startswith('https://gitee.com/pandaligx/RTX-FG-Manager-payloads/'):
                raise RuntimeError('Unexpected legacy package source')
    for path, entry in after.items():
        if path.startswith('cloud/v3/indexes/') and before.get(path) != entry:
            if path in before or path != (f'cloud/v3/indexes/{json.loads(blob(catalog_path))["revision"]}.json'):
                raise RuntimeError('Indexes are immutable and must be referenced by the catalog')
    if before.get('cloud/help.json') != after.get('cloud/help.json'):
        h = json.loads(blob('cloud/help.json'))
        if h.get('schema') != 1:
            raise RuntimeError('Invalid help manifest')
        if 'cloud/help.json' in before:
            old = json.loads(review.git(root, 'cat-file', 'blob', before['cloud/help.json'][2], binary=True))
            version = lambda v: tuple(int(x) for x in v.split('.'))
            if version(h['version']) <= version(old['version']):
                raise RuntimeError('Changed help requires a newer content version')
        for url in h['urls']:
            verify_metadata_artifact(url, h['bytes'], h['sha256'], hashlib.sha256(blob('cloud/help.json')).hexdigest())


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('mode', choices=['source', 'assets'])
    p.add_argument('--root', type=Path, default=Path.cwd())
    p.add_argument('--folder', type=Path)
    p.add_argument('--tag')
    p.add_argument('--commit', default='main')
    p.add_argument('--payloads', action='store_true')
    p.add_argument('--allow-local-exe', action='store_true')
    a = p.parse_args()
    if a.mode == 'source':
        sync_source(a.root)
    else:
        if not a.folder or not a.tag:
            p.error('assets requires --folder and --tag')
        repo = PAYLOAD_REPO if a.payloads else GC_REPO
        create_release(repo, a.tag, a.commit, a.tag, 'GitHub 同源发行资源；客户端更新清单在附件校验后发布。')
        mirror_assets(a.folder, repo, a.tag, a.allow_local_exe)


if __name__ == '__main__':
    try:
        main()
    except Exception as e:
        # API/signed-upload URLs and subprocess output can contain credentials.
        print(str(e) if isinstance(e, RuntimeError) else type(e).__name__ + ': publishing failed; details withheld')
        raise SystemExit(1)
