"""Mirror official NVIDIA release DLLs; private patch dependencies stay pinned.

No DLL execution. No patch, NR, XeSS, EXE or client-update publication here.
Only stable SDK releases, x64 release DLLs, a valid NVIDIA signature and the SDK
license qualify. Version transitions outside the reviewed major require review.
"""
import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import re
import struct
import subprocess
import tempfile
import time
import urllib.parse
import urllib.request
import zipfile

import gitcode_publish as pub

CATALOG = 'cloud/runtime-catalog.json'
KINDS = {'sr': ('dlss', 'nvngx_dlss.dll'), 'rr': ('dlss_d', 'nvngx_dlssd.dll'),
         'fg': ('dlss_g', 'nvngx_dlssg.dll')}
LIMIT = 512 * 1024 * 1024
RAW = 'https://raw.githubusercontent.com/NVIDIA/DLSS/'


def version(value):
    if not isinstance(value, str) or not re.fullmatch(r'\d{1,4}\.\d{1,4}\.\d{1,4}(?:\.0)?', value):
        raise RuntimeError('Unsupported version identity')
    return tuple(map(int, value.split('.')[:3]))


def digest(data, algorithm='sha256'):
    return hashlib.new(algorithm, data).hexdigest()


def allowed(url):
    p = urllib.parse.urlsplit(url)
    return (p.scheme == 'https' and p.port in (None, 443) and not p.username
            and not p.password and not p.fragment and not p.query and (
                re.fullmatch(re.escape(RAW) + r'[a-f0-9]{40}/(?:LICENSE\.txt|lib/Windows_x86_64/rel/nvngx_dlss[dg]?\.dll)', url) is not None))


class UpstreamRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        if not allowed(newurl):
            raise RuntimeError('Unapproved upstream redirect')
        return super().redirect_request(req, fp, code, msg, headers, newurl)


def download(url, limit):
    if not allowed(url):
        raise RuntimeError('Unapproved upstream URL')
    req = urllib.request.Request(url, headers={'User-Agent': 'RTXFG-Runtime-Sync', 'Accept-Encoding': 'identity'})
    with urllib.request.build_opener(UpstreamRedirect).open(req, timeout=60) as response:
        if not allowed(response.url):
            raise RuntimeError('Unapproved upstream destination')
        data = response.read(limit + 1)
    if not data or len(data) > limit:
        raise RuntimeError('Upstream size bound exceeded')
    return data


def gh_api(path, data=None, method=None):
    args = ['gh', 'api', path]
    if method:
        args += ['--method', method]
    if data is not None:
        args += ['--input', '-']
    out = subprocess.run(args, input=None if data is None else json.dumps(data),
                         text=True, capture_output=True, timeout=90)
    if out.returncode:
        raise RuntimeError('GitHub API failed; credential details withheld')
    return json.loads(out.stdout)


def validate_catalog(c):
    if (c.get('schema') != 1 or type(c.get('revision')) is not int or c['revision'] < 1
            or c.get('min_manager_version') != '4.3.10' or len(c.get('packages', [])) != 3):
        raise RuntimeError('Invalid runtime catalog')
    seen = set()
    for p in c['packages']:
        k = p['kind']
        if k not in KINDS or k in seen or version(p['version'])[0] != 310:
            raise RuntimeError('Invalid automatic runtime scope')
        seen.add(k)
        if (type(p['bytes']) is not int or not 0 < p['bytes'] <= LIMIT
                or not re.fullmatch('[a-f0-9]{64}', p['sha256'])):
            raise RuntimeError('Invalid runtime archive identity')
        name = f'runtime-{k}-{p["version"]}-{p["sha256"][:12]}.zip'
        if p['urls'] != urls(name) or len(p['files']) != 2:
            raise RuntimeError('Runtime sources must be immutable publisher Resources')
        for f, expected in zip(p['files'], [KINDS[k][1], 'NVIDIA-LICENSE.txt']):
            if (f['name'] != expected or type(f['bytes']) is not int or not 0 < f['bytes'] <= LIMIT
                    or not re.fullmatch('[a-f0-9]{64}', f['sha256'])):
                raise RuntimeError('Invalid runtime file identity')


def validate_advance(old, new):
    validate_catalog(new)
    validate_catalog(old)
    if new['revision'] <= old['revision']:
        raise RuntimeError('Runtime catalog must advance')
    for a in old['packages']:
        b = next(p for p in new['packages'] if p['kind'] == a['kind'])
        if version(b['version']) < version(a['version']) or (version(a['version']) == version(b['version']) and a != b):
            raise RuntimeError('Cannot replace or downgrade an existing runtime version')


def validate_pe(data):
    if len(data) < 256 or data[:2] != b'MZ':
        raise RuntimeError('Not a PE DLL')
    pe = struct.unpack_from('<I', data, 60)[0]
    if (pe + 26 > len(data) or data[pe:pe+4] != b'PE\0\0'
            or struct.unpack_from('<H', data, pe + 4)[0] != 0x8664
            or not struct.unpack_from('<H', data, pe + 22)[0] & 0x2000
            or struct.unpack_from('<H', data, pe + 24)[0] != 0x20b):
        raise RuntimeError('Not an x64 PE32+ DLL')


def official_release(releases):
    candidates = []
    for r in releases:
        if r.get('draft') or r.get('prerelease'):
            continue
        try:
            v = version(r['tag_name'].removeprefix('v'))
        except (RuntimeError, KeyError):
            continue
        candidates.append((v, r['tag_name']))
    if not candidates:
        raise RuntimeError('No stable NVIDIA SDK release')
    return max(candidates)


def verify_vendor(path, expected_version):
    env = dict(os.environ, RTXFG_VERIFY_PATH=str(path.resolve()))
    script = "$ErrorActionPreference='Stop'; $p=$env:RTXFG_VERIFY_PATH; $s=Get-AuthenticodeSignature -LiteralPath $p; $v=(Get-Item -LiteralPath $p).VersionInfo; @{status=[string]$s.Status;subject=$s.SignerCertificate.Subject;version=$v.FileVersion;company=$v.CompanyName} | ConvertTo-Json -Compress"
    result = subprocess.run(['pwsh', '-NoProfile', '-NonInteractive', '-Command', script],
                            env=env, text=True, capture_output=True, timeout=90)
    if result.returncode:
        raise RuntimeError('Vendor signature verification failed')
    info = json.loads(result.stdout)
    if (info['status'] != 'Valid' or not re.search(r'(?:^|,\s*)O=NVIDIA Corporation(?:,|$)', info.get('subject') or '')
            or info.get('company') not in ('NVIDIA', 'NVIDIA Corporation')
            or version(info['version'].strip().replace(',', '.')) != version(expected_version)):
        raise RuntimeError('Not the expected signed NVIDIA runtime')
    return info


def urls(name):
    return [pub.asset_url(pub.PAYLOAD_REPO, 'payloads', name),
            f'https://github.com/{pub.GH_REPO}/releases/download/payloads/{name}']


def package(kind, expected_version, sdk_commit, tree_entry, folder):
    name = KINDS[kind][1]
    dll = download(f'{RAW}{sdk_commit}/lib/Windows_x86_64/rel/{name}', tree_entry['size'])
    git_hash = digest(b'blob ' + str(len(dll)).encode() + b'\0' + dll, 'sha1')
    if len(dll) != tree_entry['size'] or git_hash != tree_entry['sha']:
        raise RuntimeError('Official Git blob identity mismatch')
    validate_pe(dll)
    license_data = download(f'{RAW}{sdk_commit}/LICENSE.txt', 256 * 1024)
    if b'NVIDIA' not in license_data:
        raise RuntimeError('Missing vendor SDK license')
    path = folder / name
    path.write_bytes(dll)
    signature = verify_vendor(path, expected_version)
    version_text = '.'.join(map(str, version(expected_version)))
    archive = folder / f'{kind}.zip'
    with zipfile.ZipFile(archive, 'w', zipfile.ZIP_DEFLATED, compresslevel=9) as z:
        for n, data in [(name, dll), ('NVIDIA-LICENSE.txt', license_data)]:
            info = zipfile.ZipInfo(n, (2020, 1, 1, 0, 0, 0))
            info.compress_type = zipfile.ZIP_DEFLATED
            info.external_attr = 0o100644 << 16
            z.writestr(info, data)
    sha = digest(archive.read_bytes())
    final = folder / f'runtime-{kind}-{version_text}-{sha[:12]}.zip'
    archive.rename(final)
    result = {'kind': kind, 'version': version_text, 'bytes': final.stat().st_size, 'sha256': sha,
              'files': [{'name': n, 'bytes': len(b), 'sha256': digest(b)}
                        for n, b in [(name, dll), ('NVIDIA-LICENSE.txt', license_data)]], 'urls': urls(final.name)}
    evidence = {'source': f'{RAW}{sdk_commit}/lib/Windows_x86_64/rel/{name}',
                'nvidia_commit': sdk_commit, 'git_blob': tree_entry['sha'],
                'dll_sha256': digest(dll), 'signature': signature}
    return result, final, evidence


def publish_packages(items):
    for p, path, _ in items:
        pub.upload(pub.PAYLOAD_REPO, 'payloads', path)
        release = gh_api(f'repos/{pub.GH_REPO}/releases/tags/payloads')
        assets = release.get('assets', [])
        if not any(a['name'] == path.name for a in assets):
            out = subprocess.run(['gh', 'release', 'upload', 'payloads', str(path), '--repo', pub.GH_REPO],
                                 text=True, capture_output=True, timeout=600)
            if out.returncode:
                raise RuntimeError('GitHub resource upload failed; catalog unchanged')
        for url in p['urls']:
            pub.verify(url, p['bytes'], p['sha256'])


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--publish', action='store_true')
    parser.add_argument('--report', default='.runtime-sync/report.json')
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    old_bytes = (root / CATALOG).read_bytes()
    current = json.loads(old_bytes)
    validate_catalog(current)
    report = []
    release_version, tag = official_release(gh_api('repos/NVIDIA/DLSS/releases?per_page=100'))
    sdk_commit = gh_api('repos/NVIDIA/DLSS/commits/' + tag)['sha']
    if not re.fullmatch('[a-f0-9]{40}', sdk_commit):
        raise RuntimeError('Invalid SDK commit')
    tree = gh_api('repos/NVIDIA/DLSS/git/trees/' + sdk_commit + '?recursive=1')
    if tree.get('truncated'):
        raise RuntimeError('Truncated NVIDIA tree')
    entries = {r['path']: r for r in tree['tree']}
    nr = [name for name in entries if name.endswith('/nvngx_dlssnr.dll')]
    summary = {'source': 'NVIDIA/DLSS', 'tag': tag, 'commit': sdk_commit,
               'nr': 'review-new-official-file' if nr else 'not-in-official-sdk',
               'intel': 'pinned-3.0.2', 'candidates': report, 'changed': False,
               'catalog_revision': current['revision']}
    with tempfile.TemporaryDirectory(prefix='rtxfg-runtime-') as temp:
        items = []
        for prior in current['packages']:
            kind = prior['kind']
            name = 'lib/Windows_x86_64/rel/' + KINDS[kind][1]
            entry = entries.get(name)
            if (not entry or entry.get('type') != 'blob' or entry.get('mode') != '100644'
                    or type(entry.get('size')) is not int or not 0 < entry['size'] <= LIMIT
                    or not re.fullmatch('[a-f0-9]{40}', entry.get('sha', ''))):
                raise RuntimeError('Missing official release DLL')
            origin = current.get('origins', {}).get(kind, {})
            if entry['sha'] == origin.get('git_blob'):
                report.append({'kind': kind, 'state': 'unchanged'})
                continue
            if release_version <= version(prior['version']) or release_version[0] != 310:
                report.append({'kind': kind, 'version': tag, 'state': 'review-version-or-build-change'})
                continue
            item = package(kind, tag.removeprefix('v'), sdk_commit, entry, Path(temp))
            items.append(item)
            report.append({'kind': kind, 'version': item[0]['version'], 'state': 'vendor-verified'})
        if items and args.publish:
            publish_packages(items)  # Complete anonymous readback precedes metadata.
            next_catalog = json.loads(old_bytes)
            next_catalog['revision'] = max(int(time.time()), current['revision'] + 1)
            for p, _, evidence in items:
                i = next(i for i, prior in enumerate(next_catalog['packages']) if prior['kind'] == p['kind'])
                next_catalog['packages'][i] = p
                next_catalog.setdefault('origins', {})[p['kind']] = evidence
            validate_advance(current, next_catalog)
            remote = gh_api(f'repos/{pub.GH_REPO}/contents/{CATALOG}?ref=main')
            if base64.b64decode(remote['content']) != old_bytes:
                raise RuntimeError('Catalog changed concurrently; staged assets retained, no metadata overwritten')
            content = (json.dumps(next_catalog, indent=2, ensure_ascii=False) + '\n').encode()
            gh_api(f'repos/{pub.GH_REPO}/contents/{CATALOG}', {
                'message': 'Update verified optional DLSS runtime catalog', 'branch': 'main',
                'sha': remote['sha'], 'content': base64.b64encode(content).decode()}, method='PUT')
            summary.update(changed=True, catalog_revision=next_catalog['revision'])
    path = root / args.report
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(summary, indent=2), encoding='utf-8')
    print(json.dumps(summary))


if __name__ == '__main__':
    try:
        main()
    except Exception as e:
        print(str(e) if isinstance(e, RuntimeError) else type(e).__name__ + ': runtime sync failed; catalog retained')
        raise SystemExit(1)
