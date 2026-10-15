"""Build the same immutable static help artifact for desktop cache and web hosting."""
import argparse
import hashlib
import io
import json
from pathlib import Path
import re
import zipfile

ROOT = Path(__file__).resolve().parents[1]
FILES = ('index.html', 'style.css', 'content.js', 'app.js')


def build(destination, version, minimum):
    if not re.fullmatch(r'\d{1,4}\.\d{1,4}\.\d{1,4}\.\d{1,4}', version):
        raise ValueError('Help version must have four numeric parts')
    if not re.fullmatch(r'\d{1,4}\.\d{1,4}\.\d{1,4}', minimum):
        raise ValueError('Minimum client must have three numeric parts')
    sources = {name: (ROOT/'help'/name).read_bytes() for name in FILES}
    if version.encode() not in sources['app.js']:
        raise ValueError('Displayed content version differs from package version')
    buffer = io.BytesIO()
    with zipfile.ZipFile(buffer, 'w', compression=zipfile.ZIP_DEFLATED, compresslevel=9) as z:
        for name, data in sources.items():
            info = zipfile.ZipInfo(name, (2026, 1, 1, 0, 0, 0))
            info.compress_type = zipfile.ZIP_DEFLATED
            info.external_attr = 0o100644 << 16
            z.writestr(info, data)
    archive = buffer.getvalue()
    digest = hashlib.sha256(archive).hexdigest()
    name = f'rtxfg-help-{version}-{digest[:12]}.zip'
    manifest = dict(schema=1, version=version, min_client=minimum, max_client=None,
        languages=['zh','en','ru','ja','ko'], urls=[
            f'https://gitcode.com/gcw_qWIjmve7/RTX-FG-Manager-payloads/releases/download/help-{version}/{name}',
            f'https://github.com/pandaligx/RTX-FG-Manager/releases/download/help-{version}/{name}'],
        bytes=len(archive), sha256=digest,
        files={n:hashlib.sha256(b).hexdigest() for n,b in sources.items()})
    outputs = {name:archive, 'help.json':(json.dumps(manifest,ensure_ascii=False,indent=2)+'\n').encode(),
               **{'web/'+n:b for n,b in sources.items()}}
    for relative, data in outputs.items():
        path = destination/relative
        if path.exists() and path.read_bytes()!=data:
            raise ValueError('Existing help artifact differs; use a new version and destination')
    for relative, data in outputs.items():
        path=destination/relative
        path.parent.mkdir(parents=True,exist_ok=True)
        path.write_bytes(data)
    print(json.dumps({'version':version,'archive':name,'bytes':len(archive),'sha256':digest}))


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output',type=Path,required=True)
    parser.add_argument('--version',default='4.3.0.5')
    parser.add_argument('--min-client',default='4.3.0')
    args=parser.parse_args()
    build(args.output,args.version,args.min_client)
