"""Repack reviewed local archives into one immutable ZIP per scheme.

No publishing, DLL signing or DLL rebuilding. Logical entry ownership stays
unchanged; each entry has a separate archive prefix and is selected by the client.
"""
import argparse
import copy
import hashlib
import io
import json
import os
from pathlib import Path
import zipfile

ROOT = Path(__file__).resolve().parents[1]
BASES = {
    'domestic': 'https://gitcode.com/gcw_qWIjmve7/RTX-FG-Manager-payloads/releases/download/payloads/',
    'github': 'https://github.com/pandaligx/RTX-FG-Manager/releases/download/payloads/',
    'gitee': 'https://gitee.com/pandaligx/RTX-FG-Manager-payloads/releases/download/payloads/',
}
def canonical(value):
    return (json.dumps(value, ensure_ascii=False, indent=2, sort_keys=True)+'\n').encode()
def digest(data):
    return hashlib.sha256(data).hexdigest()
def immutable(path, data):
    path.parent.mkdir(parents=True,exist_ok=True)
    if path.exists() and path.read_bytes()!=data: raise ValueError(f'Existing artifact differs: {path}')
    path.write_bytes(data)

def build(output, baseline):
    catalog=json.loads(baseline.read_text(encoding='utf-8'))
    compact=json.loads((ROOT/'cloud/v3/catalog.json').read_text(encoding='utf-8'))
    packages=[p for p in catalog['packages'] if p['scheme_id']!='initial']
    if any(p.get('archive_prefix') for p in packages): raise ValueError('Use original unbundled input')
    wanted={p['archive'] for p in packages}|{p['sha256']+'.zip' for p in packages}
    paths={}
    cache=Path(os.environ.get('LOCALAPPDATA', str(ROOT/'runtime')))/'RTXFGManager/runtime-rust/cloud'
    for root in [ROOT/'development',ROOT/'runtime',ROOT/'payloads',cache]:
        if root.exists():
            for path in root.rglob('*.zip'):
                if path.name in wanted and path.name not in paths: paths[path.name]=path
    missing=[p['archive'] for p in packages if p['archive'] not in paths and p['sha256']+'.zip' not in paths]
    if missing: raise ValueError('Missing local source archives: '+', '.join(missing))
    receipts=[]
    for sid in dict.fromkeys(p['scheme_id'] for p in packages):
        members={}
        group=[p for p in packages if p['scheme_id']==sid]
        for p in group:
            path=paths.get(p['archive']) or paths[p['sha256']+'.zip']
            data=path.read_bytes()
            if len(data)!=p['bytes'] or digest(data)!=p['sha256']: raise ValueError(f'Archive mismatch: {path}')
            with zipfile.ZipFile(io.BytesIO(data)) as z:
                if set(z.namelist())!={f['name'] for f in p['files']}: raise ValueError('Unexpected source members')
                for f in p['files']:
                    members[p['id']+'/'+f['name']]=z.read(f['name'])
        buf=io.BytesIO()
        with zipfile.ZipFile(buf,'w',compression=zipfile.ZIP_DEFLATED,compresslevel=6) as z:
            for name,data in sorted(members.items()):
                info=zipfile.ZipInfo(name,(2026,10,16,0,0,0))
                info.compress_type=zipfile.ZIP_DEFLATED
                info.external_attr=0o100644<<16
                z.writestr(info,data)
        data=buf.getvalue(); sha=digest(data)
        if len(data)>512*1024*1024: raise ValueError('Bundle exceeds client bound')
        stem={'upstream-0.3.5-310-9':'dlssg-0.4.0', 'rtx40mfg-1.3.3-hf2':'mfg-unlocker-1.5.0',
              'dlssg-transfusion-1.4.5.3':'rtx-encore-1.0.0-rc.3'}.get(sid,sid)
        filename=f'{stem}-complete-{sha[:12]}.zip'
        immutable(output/'files'/filename,data)
        for p in group:
            p.update(archive_prefix=p['id'],archive=filename,bytes=len(data),sha256=sha,
                     urls={key:base+filename for key,base in BASES.items()
                           if key != 'gitee' or len(data) <= 100*1024*1024})
        receipts.append(dict(scheme=sid,file=filename,bytes=len(data),sha256=sha,members=len(members)))
    schemes=[copy.deepcopy(s) for s in compact['schemes'] if s['id']!='initial']
    for s in schemes:
        s['min_manager_version']='4.3.1'
        s['archives']=list(dict.fromkeys(p['archive'] for p in packages if p['scheme_id']==s['id']))
    index=canonical(dict(schema=2,packages=packages))
    revision='bundles-'+digest(index+canonical(schemes))[:20]
    catalog.update(revision=revision,packages=packages,sources={k:dict(base_url=v) for k,v in BASES.items()})
    catalog['scheme_policies'].pop('initial',None)
    compact.update(revision=revision,schemes=schemes,sources=catalog['sources'],index=dict(
        bytes=len(index),sha256=digest(index),url=BASES['domestic']+revision+'.json',
        fallback_url=BASES['github']+revision+'.json'))
    immutable(output/'files'/(revision+'.json'),index)
    immutable(output/'cloud/bundles/catalog.json',canonical(compact))
    immutable(output/'cloud/bundles/indexes'/(revision+'.json'),index)
    immutable(output/'rust/cloud-catalog.json',canonical(catalog))
    immutable(output/'BUILD.json',canonical(dict(revision=revision,bundles=receipts,
        note='Local candidate only. Signed payload bytes unchanged. Existing online catalogs and assets untouched.')))
    print(canonical(dict(revision=revision,bundles=receipts)).decode())

if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output',type=Path,required=True)
    parser.add_argument('--baseline',type=Path,default=ROOT/'rust/cloud-catalog.json')
    args=parser.parse_args()
    build(args.output,args.baseline)
