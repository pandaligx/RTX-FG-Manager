"""Build the localized patch ZIP, reusing the existing dual-mirror Streamline archive."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import struct
import subprocess
import zipfile
from client_release import PUBLISHER

ROOT = Path(__file__).resolve().parents[1]
ARCHIVE_SHA = '25c09f79741f7ffc2f1e4c8dcfd20394e67572cfc178b7b93b19de315a7daf4f'
SCHEME = 'optiscaler-nr-presr-mfg'
SIGNED = {'OptiScaler.dll', 'OptiScaler/nvsmooth30.dll', 'OptiScaler/dlssg_sm86/dlssg_sm86.dll',
          'OptiScaler/dlssg_sm86/dlssg_sm86/backend/sm86_backend.dll',
          'OptiScaler/dlssg_sm86/dlssg_sm86/kernels/fg_kernels.dll'}
NOTICE = 'OptiScaler/licenses/THIRD-PARTY-NOTICES.txt'

def sha(data):
    return hashlib.sha256(data).hexdigest()

def image(data):
    """Ignore only PE Authenticode fields and its trailing certificate table."""
    body = bytearray(data)
    pe = struct.unpack_from('<I', body, 60)[0]
    opt = pe + 24
    if body[pe:pe+4] != b'PE\0\0' or struct.unpack_from('<H', body, opt)[0] != 0x20b:
        raise ValueError('Expected PE32+')
    addr = opt + 112 + 4 * 8
    start, size = struct.unpack_from('<II', body, addr)
    if start or size:
        if not start or start + size != len(body):
            raise ValueError('Invalid certificate table')
        del body[start:]
    body[opt+64:opt+68] = bytes(4)
    body[addr:addr+8] = bytes(8)
    return body

def main():
    parser = argparse.ArgumentParser()
    for name in ['upstream', 'signed', 'output']:
        parser.add_argument(name, type=Path)
    parser.add_argument('--localized-build', type=Path, required=True,
                        help='Original, unsigned localized build kept by the publisher')
    parser.add_argument('--signed-entry', type=Path, help='Signed copy of the localized build')
    parser.add_argument('--preview-catalog', type=Path, help='Unsigned local QA only; write a separate catalog')
    args = parser.parse_args()
    catalog_path = ROOT/'rust/cloud-catalog.json'
    if args.preview_catalog and args.preview_catalog.resolve() == catalog_path.resolve():
        parser.error('Use a separate preview catalog')
    if bool(args.signed_entry) == bool(args.preview_catalog):
        parser.error('Choose a signed entry or an isolated preview catalog')
    if sha(args.upstream.read_bytes()) != ARCHIVE_SHA:
        raise ValueError('Unexpected upstream ZIP')
    components = (ROOT/'rust/assets/optiscaler-nr-files.txt').read_text().splitlines()
    args.output.mkdir(parents=True, exist_ok=True)
    staged = args.output/'optiscaler-nr-0.9.97.tmp'
    files = []
    with zipfile.ZipFile(args.upstream) as source, zipfile.ZipFile(staged,'w') as archive:
        sl_manifest = json.loads(source.read('redist/streamline/manifest.json'))
        sl_hashes = {f['name']: f['sha256'].lower() for f in sl_manifest['files'] if f['name'].endswith('.dll')}
        if sl_manifest['version'] != '2.14.16' or set(sl_hashes) != {
            Path(n).name for n in components if n.startswith('OptiScaler/streamline/')
        }:
            raise ValueError('Unexpected private Streamline manifest')
        for dest in ['dxgi.dll', 'OptiScaler.ini', *components]:
            name = 'OptiScaler.dll' if dest == 'dxgi.dll' else dest
            if name == NOTICE:
                licenses = ['LICENSE'] + sorted(i for i in source.namelist() if not i.endswith('/') and
                    (i.startswith('Licenses/') or i.startswith('OptiScaler/') and any(k in i.lower() for k in ['license', 'notices', 'source.nvsmooth30'])))
                sections = []
                for filename in licenses:
                    encoding = 'cp1252' if filename == 'OptiScaler/streamline/reflex.license.txt' else 'utf-8-sig'
                    sections.append(filename + '\n' + source.read(filename).decode(encoding))
                ui = ROOT/'packaging/optiscaler-menu-zh-CN'
                sections.append('Simplified Chinese menu translations adapted from A-XIANGQAQ, GPL-3.0.\n'
                                'https://github.com/A-XIANGQAQ/OptiScaler-DLSS5-MFG-RTX40/tree/v10.0.0-pre2\n'
                                + (ui/'LICENSE.upstream').read_text(encoding='utf-8-sig'))
                sections.append('Embedded subset of Noto Sans CJK SC\n' + (ui/'OFL-Noto.txt').read_text(encoding='utf-8-sig'))
                data = '\n\n'.join(sections).encode('utf-8')
            elif name == 'OptiScaler.dll':
                data = args.localized_build.read_bytes()
            else:
                data = source.read(name)
            if name.startswith('OptiScaler/streamline/') and sha(data) != sl_hashes[Path(name).name]:
                raise ValueError(f'Private Streamline DLL differs from the NVIDIA manifest: {name}')
            if name.startswith('OptiScaler/streamline/'):
                continue
            if name in SIGNED and (name != 'OptiScaler.dll' or args.signed_entry):
                path = args.signed_entry if name == 'OptiScaler.dll' else args.signed/name
                signed = path.read_bytes()
                if image(signed) != image(data) or signed == data:
                    raise ValueError(f'Signed PE differs from original: {name}')
                script = "$s=Get-AuthenticodeSignature -LiteralPath $env:RTXFG_SIGNED_DLL; Write-Output ($s.Status.ToString() + ' ' + $s.SignerCertificate.Thumbprint)"
                shell = shutil.which('pwsh.exe') or 'powershell.exe'
                result = subprocess.check_output([shell,'-NoProfile','-NonInteractive','-Command',script],
                    env={**os.environ,'RTXFG_SIGNED_DLL':str(path.resolve())},text=True,timeout=60)
                if result.strip() != f'Valid {PUBLISHER}':
                    raise ValueError(f'Invalid publisher signature: {name}')
                data = signed
            files.append(dict(name=dest, bytes=len(data), sha256=sha(data)))
            archive.writestr(zipfile.ZipInfo(dest, (2026, 1, 1, 0, 0, 0)), data, compress_type=zipfile.ZIP_DEFLATED)
    digest = sha(staged.read_bytes())
    archive = f'optiscaler-nr-0.9.97-base-{digest[:12]}.zip'
    staged.replace(args.output/archive)
    catalog = json.loads(catalog_path.read_bytes())
    catalog['packages'] = [p for p in catalog['packages'] if p['scheme_id'] != SCHEME]
    label = 'OptiScaler-NR-PreSR-MFG · 0.9.97'
    reference = next(p for p in catalog['packages'] if p['scheme_id'] == 'anr-fg')
    pkg = dict(archive=archive, archive_prefix='', backends=['optiscaler_nr'], bytes=(args.output/archive).stat().st_size,
               component=False, files=files, id=f'optiscaler-nr-0.9.97-{digest[:12]}', label=label,
               labels={lang: label for lang in reference['labels']}, proxy='dxgi.dll', scheme_id=SCHEME,
               sha256=digest, upstream_version='0.9.97',version='0.9.97',
               urls={k:v.rsplit('/',1)[0]+'/'+archive for k,v in reference['urls'].items()})
    catalog['packages'].append(pkg)
    catalog['scheme_policies'][SCHEME] = dict(parameter_profile='optiscaler_nr_ini', ini_policy='optiscaler_nr',
                                               max_selected_proxies=1, gpu_paths=['SM75','SM86','SM89'], defaults={})
    (args.preview_catalog or catalog_path).write_bytes((json.dumps(catalog,ensure_ascii=False,indent=2,sort_keys=True)+'\n').encode())
    print(archive, digest, len(files))

if __name__ == '__main__':
    main()
