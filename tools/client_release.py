"""Stable client channels and local signed-EXE packaging. No upload or promotion.

4.3.21 is the last versioned attachment. Keep its legacy manifest available on
main AND every later release for old clients' latest-release fallback.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess

BRIDGE = '4.3.21'
MODERN = 'update-v2.json'
LEGACY = 'update.json'
PUBLISHER = 'BF4DED3827EB26116EAE9E9ACACE0B0791D502BF'


def checked(data, channel=None):
    m = json.loads(data)
    if not isinstance(m, dict):
        raise RuntimeError('Invalid update manifest')
    v = m.get('version', '')
    schema = m.get('schema')
    notes = m.get('notes', {})
    expected = f'RTXManager-v{v}-x64.exe' if schema == 1 else 'RTXManager.exe'
    if (type(schema) is not int or schema not in (1, 2)
            or not isinstance(v, str) or not re.fullmatch(r'\d{1,4}\.\d{1,4}\.\d{1,4}', v, re.ASCII)
            or m.get('file') != expected
            or not isinstance(m.get('sha256'), str) or not re.fullmatch('[0-9a-fA-F]{64}', m['sha256'])
            or type(m.get('bytes')) is not int or not 1048576 <= m['bytes'] <= 268435456
            or not isinstance(notes, dict) or len(notes) > 8
            or any(not isinstance(n, str) or len(n.encode('utf-8')) > 16384 for n in notes.values())):
        raise RuntimeError('Invalid update manifest')
    if channel is not None and channel != (LEGACY if schema == 1 else MODERN):
        raise RuntimeError('Manifest belongs to another update channel')
    return m


def transition(previous, current, channel):
    new = checked(current, channel)
    number = lambda m: tuple(map(int, m['version'].split('.')))
    bridge = tuple(map(int, BRIDGE.split('.')))
    if channel == LEGACY and number(new) > bridge:
        raise RuntimeError('Legacy channel must stop at the 4.3.21 bridge')
    if channel == MODERN and number(new) < bridge:
        raise RuntimeError('Modern channel starts at 4.3.21')
    if previous is None:
        return 'advance'
    old = checked(previous, channel)
    if number(new) < number(old):
        raise RuntimeError('Update manifest cannot roll back')
    if number(new) == number(old):
        if new != old:
            raise RuntimeError('Same-version metadata cannot be replaced')
        return 'current'
    return 'advance'


def release_manifests(modern, legacy_bytes=None):
    """Return both channels; future releases reuse the exact legacy JSON bytes."""
    raw = (json.dumps(modern, ensure_ascii=False, indent=2) + '\n').encode('utf-8')
    transition(None, raw, MODERN)
    if modern['version'] == BRIDGE:
        legacy = {**modern, 'schema': 1, 'file': f'RTXManager-v{BRIDGE}-x64.exe'}
        bridge_bytes = (json.dumps(legacy, ensure_ascii=False, indent=2) + '\n').encode('utf-8')
        if legacy_bytes is not None and checked(legacy_bytes, LEGACY) != legacy:
            raise RuntimeError('Published bridge identity differs')
        legacy_bytes = legacy_bytes or bridge_bytes
    if legacy_bytes is None or checked(legacy_bytes, LEGACY)['version'] != BRIDGE:
        raise RuntimeError('Retain the verified 4.3.21 legacy manifest')
    return {MODERN: raw, LEGACY: legacy_bytes}


def verify_signed(path, version):
    if os.name != 'nt':
        raise RuntimeError('Signed EXEs must be prepared on the Windows publisher machine')
    # Argument is an environment value, not interpolated PowerShell source.
    script = '''$ErrorActionPreference='Stop'
[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)
$env:PSModulePath=Join-Path $PSHOME 'Modules'
$p=$env:RTXFG_RELEASE_EXE
$s=Get-AuthenticodeSignature -LiteralPath $p
$v=(Get-Item -LiteralPath $p).VersionInfo
@{status=[string]$s.Status;publisher=$s.SignerCertificate.Thumbprint;version=$v.FileVersion;product=$v.ProductName}|ConvertTo-Json -Compress
'''
    result = subprocess.run(['powershell.exe', '-NoProfile', '-NonInteractive', '-Command', script],
                            env={**os.environ, 'RTXFG_RELEASE_EXE': str(path.resolve())},
                            capture_output=True, text=True, encoding='utf-8', check=True, timeout=120,
                            creationflags=subprocess.CREATE_NO_WINDOW)
    info = json.loads(result.stdout.lstrip('\ufeff'))
    if (info['status'] != 'Valid' or info['publisher'] != PUBLISHER
            or info['version'] not in (version, version + '.0')
            or info['product'] != 'RTX Frame Generation Manager'):
        raise RuntimeError('EXE signature, product or version verification failed')


def prepare(exe, version, notes, output, legacy=None):
    if exe.is_symlink() or not exe.is_file() or output.exists():
        raise RuntimeError('Use a regular signed EXE and a new output directory')
    data = exe.read_bytes()
    modern = dict(schema=2, version=version, file='RTXManager.exe',
                  sha256=hashlib.sha256(data).hexdigest(), bytes=len(data), notes=notes)
    manifests = release_manifests(modern, legacy.read_bytes() if legacy else None)
    verify_signed(exe, version)
    if exe.read_bytes() != data:
        raise RuntimeError('EXE changed while checking its signature')
    files = {'RTXManager.exe': data, **manifests}
    if version == BRIDGE:
        files[f'RTXManager-v{BRIDGE}-x64.exe'] = data
    output.mkdir(parents=True, exist_ok=False)
    for name, raw in files.items():
        (output / name).write_bytes(raw)
    (output / 'SHA256SUMS.txt').write_text(''.join(
        f'{hashlib.sha256(raw).hexdigest()}  {name}\n' for name, raw in sorted(files.items())), encoding='utf-8')
    return sorted(files)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--exe', required=True, type=Path)
    parser.add_argument('--version', required=True)
    parser.add_argument('--notes', required=True, type=Path, help='JSON language-to-text object')
    parser.add_argument('--output', required=True, type=Path)
    parser.add_argument('--legacy-manifest', type=Path, help='Verified, frozen 4.3.21 update.json')
    args = parser.parse_args()
    print(json.dumps(prepare(args.exe, args.version, json.loads(args.notes.read_text('utf-8-sig')),
                             args.output, args.legacy_manifest), ensure_ascii=False))
