"""Binary-release repository mirror. Contains no manager implementation or secrets."""
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import urllib.error
import urllib.parse
import urllib.request
import uuid

REPO='pandaligx/RTX-FG-Manager'
API='https://gitee.com/api/v5/repos/'+REPO
ALLOWED={'README.md','README.zh-CN.md','LICENSE','THIRD_PARTY_NOTICES.txt',
         'docs/screenshot-home.png','docs/screenshot-home-zh.png',
         '.github/workflows/mirror-gitee.yml','.github/scripts/mirror_gitee.py'}


def api(path, fields=None, file=None):
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
    h=hashlib.sha256();size=0
    with urllib.request.urlopen(url,timeout=60) as response:
        if not response.url.startswith('https://'):raise RuntimeError('Insecure mirror redirect')
        while chunk:=response.read(1024*1024):
            h.update(chunk);size+=len(chunk)
            if size>expected.stat().st_size:raise RuntimeError('Mirror asset too large')
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


def main():
    if not os.environ.get('GITEE_TOKEN'):
        raise RuntimeError('Configure repository secret GITEE_TOKEN; mirror has NOT completed')
    os.environ.setdefault('GITEE_USERNAME',REPO.split('/')[0])
    tracked=set(subprocess.check_output(['git','ls-files','-z']).decode().split('\0'))-{''}
    if tracked!=ALLOWED:raise RuntimeError('Publication repository allowlist mismatch')
    tag=os.environ.get('TAG_NAME','')
    if tag and not re.fullmatch(r'v\d{1,4}\.\d{1,4}\.\d{1,4}',tag):raise RuntimeError('Invalid stable tag')
    # Gitee repository must be created by its owner before enabling synchronization.
    api('')
    with tempfile.TemporaryDirectory(prefix='rtxfg-mirror-') as tmp:
        askpass=Path(tmp)/'askpass.py'
        askpass.write_text('#!/usr/bin/env python3\nimport os,sys\nprint(os.environ["GITEE_USERNAME"] if "username" in sys.argv[1].lower() else os.environ["GITEE_TOKEN"])\n')
        askpass.chmod(0o700)
        env=dict(os.environ,GIT_ASKPASS=str(askpass),GIT_TERMINAL_PROMPT='0')
        args=['git','-c','credential.helper=','push','https://gitee.com/'+REPO+'.git','refs/heads/main:refs/heads/main']
        if tag:
            remote_refs=subprocess.check_output(['git','-c','credential.helper=','ls-remote',
                'https://gitee.com/'+REPO+'.git',f'refs/tags/{tag}',f'refs/tags/{tag}^{{}}'],env=env,text=True,timeout=60)
            commit=subprocess.check_output(['git','rev-parse',f'{tag}^{{commit}}'],text=True).strip()
            if tag_push_required(remote_refs,tag,commit):args.append(f'refs/tags/{tag}:refs/tags/{tag}')
        subprocess.run(args,env=env,check=True,timeout=300)
        if not tag:
            print('Documentation mirror completed.');return
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
        if not isinstance(existing,list):raise RuntimeError('Invalid Gitee attachments')
        # Large EXEs are uploaded from the publisher machine, never from Actions.
        exe=assets/f'RTXManager-{tag}-x64.exe'
        if not verify_manual_exe(existing,exe):return
        for file in files:
            if file.suffix.lower()=='.exe':continue
            matches=[a for a in existing if a.get('name')==file.name]
            if len(matches)>1:raise RuntimeError('Duplicate mirror asset')
            if not matches:api(endpoint,{},file)
            latest=api(endpoint+'?per_page=100')
            found=[a for a in latest if a.get('name')==file.name]
            if len(found)!=1:raise RuntimeError('Attachment not visible after upload')
            verify_remote(found[0].get('browser_download_url') or found[0].get('download_url') or found[0].get('url',''),file)
            print('Verified mirror asset: '+file.name)
        print('Release mirror completed; GitHub and Gitee downloads match.')


if __name__=='__main__':main()
