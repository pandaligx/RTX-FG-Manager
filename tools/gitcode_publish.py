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
_protocol_spec = importlib.util.spec_from_file_location('client_release', Path(__file__).with_name('client_release.py'))
client_release = importlib.util.module_from_spec(_protocol_spec)
_protocol_spec.loader.exec_module(client_release)
_linux_spec = importlib.util.spec_from_file_location('linux_release', Path(__file__).with_name('linux_release.py'))
linux_release = importlib.util.module_from_spec(_linux_spec)
_linux_spec.loader.exec_module(linux_release)


def artifact_size_limit(url):
    prefixes = ('https://gitcode.com/' + PAYLOAD_REPO + '/releases/download/',
                'https://github.com/' + GH_REPO + '/releases/download/')
    if url.startswith(prefixes) and url.endswith('.zip'):
        return 512 * 1024 * 1024
    return 256 * 1024 * 1024


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
                re.fullmatch(r'(?:r|bundles)-[0-9a-f]{20}\.json', name)
                or (re.fullmatch(r'[A-Za-z0-9._-]+-[0-9a-f]{12}\.zip', name)
                    and name.endswith('-' + digest[:12] + '.zip'))))
            if valid and immutable and type(size) is int and 0 < size <= artifact_size_limit(url):
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
    if type(size) is not int or not 0 < size <= artifact_size_limit(url) or not re.fullmatch('[0-9a-f]{64}', digest):
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
    limit_mb = 256 if is_exe else 512 if repo == PAYLOAD_REPO and path.suffix.lower() == '.zip' else 128
    if not data or len(data) > limit_mb * 1024 * 1024:
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
    # Cross-region runners may need several minutes for a verified runtime ZIP.
    # Keep an explicit upper bound and stop stalled transfers rather than retry
    # indefinitely or expose the delegated upload URL in command-line arguments.
    seconds = min(1800, max(180, int(os.environ.get('RTXFG_UPLOAD_TIMEOUT', '180'))))
    try:
        result = subprocess.run(['curl','--config','-','--ipv4','--silent','--show-error',
            '--proto','=https','--connect-timeout','15','--max-time',str(seconds),
            '--speed-limit','1024','--speed-time','90',
            '--upload-file',str(path),'--output',os.devnull,'--write-out',stats],
            input=config,text=True,encoding='utf-8',errors='replace',capture_output=True,timeout=seconds+10)
    except subprocess.TimeoutExpired:
        raise RuntimeError('GitCode upload exceeded its bounded timeout; delegation withheld') from None
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
    m = client_release.checked(data)
    v = m['version']
    verify(asset_url(GC_REPO, 'v' + v, m['file']), m['bytes'], m['sha256'])
    verify(f'https://github.com/{GH_REPO}/releases/download/v{v}/{m["file"]}', m['bytes'], m['sha256'])


def verify_linux_update(data):
    m = linux_release.checked(data)
    for url in (asset_url(GC_REPO, 'v' + m['version'], m['file']),
                f'https://github.com/{GH_REPO}/releases/download/v{m["version"]}/{m["file"]}'):
        verify(url, m['bytes'], m['sha256'])


def mirror_assets(folder, repo, tag, allow_exe=False):
    files = list(Path(folder).iterdir())
    if not files or any(not p.is_file() or p.is_symlink() for p in files):
        raise RuntimeError('Asset directory must contain only reviewed regular files')
    channels = ('update.json', 'update-v2.json', linux_release.CHANNEL)
    if repo == GC_REPO and re.fullmatch(r'v\d{1,4}\.\d{1,4}\.\d{1,4}', tag) and tuple(map(int, tag[1:].split('.'))) >= (4, 3, 21):
        modern = client_release.checked((Path(folder) / 'update-v2.json').read_bytes(), 'update-v2.json')
        legacy = (Path(folder) / 'update.json').read_bytes()
        client_release.release_manifests(modern, legacy)
        if modern['version'] != tag[1:]:
            raise RuntimeError('Modern manifest must match the release tag')
        expected = {'RTXManager.exe'}
        if tag == 'v4.3.21':
            expected.add('RTXManager-v4.3.21-x64.exe')
        if any(p.suffix.lower() == '.exe' and p.name not in expected for p in files):
            raise RuntimeError('Unexpected client executable attachment')
    # Both channels follow the locally uploaded and verified executables.
    for path in sorted(files, key=lambda p: (p.name in channels, p.name == 'update-v2.json')):
        if path.suffix.lower() == '.exe' and not allow_exe:
            continue
        if path.suffix == '.AppImage':
            if repo != GC_REPO or path.name != f'RTXManager-{tag.removeprefix("v")}-x86_64.AppImage':
                raise RuntimeError('Unexpected Linux client attachment')
            if not allow_exe:
                continue  # Like the EXE, the publisher uploads this locally.
        if path.name in channels:
            if path.name == linux_release.CHANNEL:
                m = linux_release.checked(path.read_bytes())
                if tag != 'v' + m['version'] or repo != GC_REPO:
                    raise RuntimeError('Linux manifest must match its client release tag')
                verify_linux_update(path.read_bytes())
            else:
                client_release.checked(path.read_bytes(), path.name)
                verify_update(path.read_bytes())
        upload(repo, tag, path, allow_exe)


def verify_client_metadata(root, review, before, entries):
    channel = linux_release.CHANNEL
    update, old = entries.get(channel), before.get(channel)
    if update != old:
        if not update:
            raise RuntimeError('Cannot remove the Linux update channel')
        raw = review.git(root, 'cat-file', 'blob', update[2], binary=True)
        previous = review.git(root, 'cat-file', 'blob', old[2], binary=True) if old else None
        linux_release.transition(previous, raw)
        verify_linux_update(raw)
    for channel in ('update.json', 'update-v2.json'):
        update = entries.get(channel)
        old = before.get(channel)
        if update == old:
            continue
        if not update:
            raise RuntimeError('Cannot remove an update channel')
        raw = review.git(root, 'cat-file', 'blob', update[2], binary=True)
        previous = review.git(root, 'cat-file', 'blob', old[2], binary=True) if old else None
        client_release.transition(previous, raw, channel)
        verify_update(raw)
    if entries.get('update-v2.json'):
        legacy = entries.get('update.json')
        if not legacy:
            raise RuntimeError('Modern updates require the retained legacy bridge')
        modern = review.git(root, 'cat-file', 'blob', entries['update-v2.json'][2], binary=True)
        old = review.git(root, 'cat-file', 'blob', legacy[2], binary=True)
        client_release.release_manifests(client_release.checked(modern, 'update-v2.json'), old)


def complete_pending_bundle_index(root, review, entries, final):
    """Validate a split metadata commit against its unchanged, completed tip.

    Only supply a missing immutable index, never replace an existing blob.
    The normal metadata verifier still checks bytes, digest and remote receipts.
    """
    catalog = 'cloud/bundles/catalog.json'
    if catalog not in entries or entries[catalog] != final.get(catalog):
        return entries
    raw = review.git(root, 'cat-file', 'blob', entries[catalog][2], binary=True)
    revision = json.loads(raw).get('revision', '')
    if not re.fullmatch(r'bundles-[0-9a-f]{20}', revision):
        return entries
    index = f'cloud/bundles/indexes/{revision}.json'
    if index in entries or index not in final:
        return entries
    return {**entries, index: final[index]}


def transient_unreferenced_bundle_indexes(root, review, baseline, final, history):
    """Ignore abandoned candidate indexes in metadata transitions, not source review.

    They must be absent from both published endpoints and never referenced by
    any catalog in the pending main history. New side-branch content and every
    main checkpoint still pass source review before this projection is used.
    """
    referenced = set()
    candidates = set()
    for entries in [baseline, *history, final]:
        catalog = entries.get('cloud/bundles/catalog.json')
        if catalog:
            raw = review.git(root, 'cat-file', 'blob', catalog[2], binary=True)
            revision = json.loads(raw).get('revision', '')
            referenced.add(f'cloud/bundles/indexes/{revision}.json')
        candidates.update(path for path in entries
                          if re.fullmatch(r'cloud/bundles/indexes/bundles-[0-9a-f]{20}\.json', path))
    return candidates - set(baseline) - set(final) - referenced


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
        before = review.tree_entries(root, previous)
        final = review.tree_entries(root, source)
        history = source_publication_history(root, review, previous, source)
        abandoned = transient_unreferenced_bundle_indexes(root, review, before, final, history)
        for entries in history:
            entries = {path: value for path, value in entries.items() if path not in abandoned}
            verify_client_metadata(root, review, before, entries)
            # Legacy catalogs are a transition contract. Schema 3 references
            # are verified before their source commit becomes visible here.
            legacy = lambda e: {p:v for p,v in e.items() if p == 'cloud/catalog.json' or p.startswith('cloud/indexes/')}
            if legacy(entries) != legacy(before):
                raise RuntimeError('Legacy catalogs must remain unchanged during migration')
            completed = complete_pending_bundle_index(root, review, entries, final)
            verify_metadata_changes(root, review, before, completed)
            before = completed
        review.git(root, '-c', 'credential.helper=', 'push', remote, source + ':refs/heads/main', env=env)
        refs = review.git(root, '-c', 'credential.helper=', 'ls-remote', remote, 'refs/heads/main', env=env)
        if not refs or refs.split()[0] != source:
            raise RuntimeError('GitCode source readback mismatch')
        print('GitCode source fast-forward verified: ' + source)


def source_publication_history(root, review, previous, source):
    """Review all newly introduced content; promote metadata along main only.

    An old feature branch inherits obsolete files and catalogs from its base.
    Those blobs are already in the mirror's history, not new publications.
    Check each new entry against every parent so even a forbidden file added
    and deleted on a side branch is rejected. Validate the entire final tree.
    """
    lineage = review.git(root, 'rev-list', '--first-parent', source).splitlines()
    if previous not in lineage:
        raise RuntimeError('GitCode baseline must be on the main first-parent history')
    trees = {}

    def tree(commit):
        if commit not in trees:
            trees[commit] = review.tree_entries(root, commit)
        return trees[commit]

    seen = set()
    review.validate_tree(root, tree(source), seen)
    commits = review.git(root, 'rev-list', '--reverse', '--topo-order',
                         previous + '..' + source).splitlines()
    for commit in commits:
        parents = review.git(root, 'rev-list', '--parents', '-n', '1', commit).split()[1:]
        parent_trees = [tree(parent) for parent in parents]
        introduced = {path: entry for path, entry in tree(commit).items()
                      if not any(parent.get(path) == entry for parent in parent_trees)}
        review.validate_tree(root, introduced, seen)
    history = [tree(commit) for commit in reversed(lineage[:lineage.index(previous)])]
    # Every published main checkpoint must pass the full current allowlist.
    for entries in history:
        review.validate_tree(root, entries, seen)
    return history


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


def validate_bundle_registration(prior, current, index, embedded):
    """Allow restoring an omitted declaration, never changing published bytes.

    Both the immutable index and the already-reviewed embedded catalog must be
    unchanged. Every previous declaration stays identical; additions must agree
    with the embedded policy and the scheme's complete archive set.
    """
    unchanged = lambda c: {k: v for k, v in c.items() if k != 'schemes'}
    if unchanged(prior) != unchanged(current):
        raise RuntimeError('Bundle registration cannot change index or sources')
    old = {s['id']: s for s in prior['schemes']}
    new = {s['id']: s for s in current['schemes']}
    if len(old) != len(prior['schemes']) or len(new) != len(current['schemes']):
        raise RuntimeError('Duplicate bundle registration')
    if not old.keys() < new.keys() or any(new[k] != v for k, v in old.items()):
        raise RuntimeError('Bundle registration must only add omitted schemes')
    for key in new.keys() - old.keys():
        scheme = new[key]
        policy = embedded.get('scheme_policies', {}).get(key)
        archives = {p['archive'] for p in index['packages'] if p.get('scheme_id') == key}
        if (not policy or not archives or set(scheme.get('archives', [])) != archives
                or scheme.get('profile') != policy.get('parameter_profile')
                or scheme.get('defaults', {}) != policy.get('defaults', {})
                or scheme.get('capabilities', []) != policy.get('capabilities', [])):
            raise RuntimeError('Bundle registration differs from published scheme')


def verify_metadata_changes(root, review, before, after):
    if any(p.startswith('cloud/') and p.endswith('.json') and p not in after for p in before):
        raise RuntimeError('Published metadata cannot be deleted')
    def blob(path):
        if path not in after:
            raise RuntimeError('Missing referenced metadata: ' + path)
        return review.git(root, 'cat-file', 'blob', after[path][2], binary=True)
    runtime_path = 'cloud/runtime-catalog.json'
    if before.get(runtime_path) != after.get(runtime_path):
        import sync_runtimes
        raw = blob(runtime_path)
        next_catalog = json.loads(raw)
        sync_runtimes.validate_catalog(next_catalog)
        prior_packages = []
        if runtime_path in before:
            prior = json.loads(review.git(root, 'cat-file', 'blob', before[runtime_path][2], binary=True))
            sync_runtimes.validate_advance(prior, next_catalog)
            prior_packages = prior['packages'] + prior.get('history', [])
        metadata_digest = hashlib.sha256(raw).hexdigest()
        for p in next_catalog['packages'] + next_catalog.get('history', []):
            if p not in prior_packages:
                for url in p['urls']:
                    verify_metadata_artifact(url, p['bytes'], p['sha256'], metadata_digest)
    bundle_path = 'cloud/bundles/catalog.json'
    if before.get(bundle_path) != after.get(bundle_path):
        c = json.loads(blob(bundle_path))
        if c.get('schema') != 3 or not re.fullmatch(r'bundles-[0-9a-f]{20}', c.get('revision', '')):
            raise RuntimeError('Invalid complete-bundle catalog')
        index_path = f'cloud/bundles/indexes/{c["revision"]}.json'
        raw = blob(index_path)
        metadata_digest = hashlib.sha256(blob(bundle_path)).hexdigest()
        if index_path in before:
            embedded_path = 'rust/cloud-catalog.json'
            if (before.get(index_path) != after.get(index_path)
                    or embedded_path not in before
                    or before[embedded_path] != after.get(embedded_path)
                    or bundle_path not in before):
                raise RuntimeError('A bundle catalog change requires a new immutable revision')
            previous = review.git(root, 'cat-file', 'blob', before[bundle_path][2], binary=True)
            validate_bundle_registration(json.loads(previous), c, json.loads(raw),
                                         json.loads(blob(embedded_path)))
            # Only identical index/archive bytes may reuse the previous readback.
            metadata_digest = hashlib.sha256(previous).hexdigest()
        index = c['index']
        if len(raw) != index['bytes'] or hashlib.sha256(raw).hexdigest() != index['sha256']:
            raise RuntimeError('Bundle index does not match catalog')
        expected = asset_url(PAYLOAD_REPO, 'payloads', c['revision'] + '.json')
        if index['url'] != expected:
            raise RuntimeError('Bundle index must use Resources')
        verify_metadata_artifact(expected, len(raw), index['sha256'], metadata_digest)
        seen = {}
        for p in json.loads(raw)['packages']:
            if not p.get('archive_prefix'):
                raise RuntimeError('Complete bundle requires an explicit package prefix')
            url = asset_url(PAYLOAD_REPO, 'payloads', p['archive'])
            if p.get('urls', {}).get('domestic') != url:
                raise RuntimeError('Bundle must use the GitCode Resources location')
            identity = (p['bytes'], p['sha256'])
            if url in seen and seen[url] != identity:
                raise RuntimeError('Conflicting bundle identity')
            if url not in seen:
                verify_metadata_artifact(url, *identity, metadata_digest)
                seen[url] = identity
    for path, entry in after.items():
        if path.startswith('cloud/bundles/indexes/') and before.get(path) != entry:
            if path in before or path != f'cloud/bundles/indexes/{json.loads(blob(bundle_path))["revision"]}.json':
                raise RuntimeError('Bundle indexes are immutable and must be referenced')
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
        create_release(repo, a.tag, a.commit,
            'Resources (not a manager update)' if a.payloads else a.tag,
            'GitHub 同源发行资源；客户端更新清单在附件校验后发布。')
        mirror_assets(a.folder, repo, a.tag, a.allow_local_exe)


if __name__ == '__main__':
    try:
        main()
    except Exception as e:
        # API/signed-upload URLs and subprocess output can contain credentials.
        print(str(e) if isinstance(e, RuntimeError) else type(e).__name__ + ': publishing failed; details withheld')
        raise SystemExit(1)
