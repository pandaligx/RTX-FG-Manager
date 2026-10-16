"""CI entry point. Never upload an executable; update.json is always last."""
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import time
import gitcode_publish as publish


def main():
    tag = os.environ.get('RELEASE_TAG', '')
    if tag:
        if not re.fullmatch(r'[A-Za-z0-9._-]{1,100}', tag):
            raise RuntimeError('Invalid source tag')
        payload = os.environ.get('PAYLOADS') == 'true' or tag.startswith(('payloads', 'help'))
        destination = os.environ.get('DESTINATION_TAG') or ('payloads' if payload else tag)
        repo = publish.PAYLOAD_REPO if payload else publish.GC_REPO
        asset_name = os.environ.get('ASSET_NAME', '')
        if asset_name and (not payload or not re.fullmatch(r'[A-Za-z0-9._-]{1,160}',asset_name)):
            raise RuntimeError('Single-asset tests only support resource releases')
        release = json.loads(subprocess.check_output(['gh', 'release', 'view', tag, '--repo', publish.GH_REPO,
            '--json', 'name,body,targetCommitish,assets'], text=True))
        # The independent payload repository does not contain GitHub source SHAs.
        # A client release, however, must retain the exact reviewed source tag.
        if not payload:
            publish.sync_source(Path.cwd())
            publish.sync_release_tag(Path.cwd(), tag)
        publish.create_release(repo, destination, 'main' if payload else tag,
            'Resources (not a manager update)' if payload else release['name'], release['body'])
        with tempfile.TemporaryDirectory() as folder:
            started = time.monotonic()
            for asset in release['assets']:
                name = asset['name']
                if asset_name and name != asset_name:
                    continue
                if name.endswith('.exe'):
                    continue
                if not re.fullmatch(r'[A-Za-z0-9._-]{1,160}', name):
                    raise RuntimeError('Unsafe release asset name')
                if not payload and name not in ('update.json', 'SHA256SUMS.txt', 'third-party-info.json'):
                    raise RuntimeError('Unexpected stable release asset')
                if asset['size'] > (128 if payload else 2) * 1024 * 1024:
                    raise RuntimeError('Unexpected asset size')
                subprocess.run(['gh', 'release', 'download', tag, '--repo', publish.GH_REPO,
                    '--pattern', name, '--dir', folder], check=True)
            print(f'GitHub resource download: {time.monotonic()-started:.3f} seconds',flush=True)
            started = time.monotonic()
            publish.mirror_assets(Path(folder), repo, destination)
            print(f'GitCode upload and anonymous readback: {time.monotonic()-started:.3f} seconds',flush=True)
    publish.sync_source(Path.cwd())


if __name__ == '__main__':
    try:
        main()
    except Exception as e:
        print(str(e) if isinstance(e, RuntimeError) else type(e).__name__ + ': mirror failed; details withheld')
        raise SystemExit(1)
