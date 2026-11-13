"""Exercise a real AppImage installer in isolated folders under Xvfb (Linux CI)."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import tempfile
import time
import uuid


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def run(image, version, success):
    with tempfile.TemporaryDirectory(prefix='rtxfg-update-') as folder:
        root = Path(folder)
        data = root/'data'
        data.mkdir()
        (data/'games.json').write_text(json.dumps(dict(schema=3, games=[], roots=[],
            language='en', theme='深色', auto_update=False, auto_download=False,
            initial_scan_pending=False, welcome_pending=False)), encoding='utf-8')
        target = root/'My manager.AppImage'
        shutil.copyfile(image, target)
        with target.open('ab') as out:
            out.write(b'previous-version-fixture')
        target.chmod(0o755)
        old = digest(target)
        stage = data/'updates'/uuid.uuid4().hex
        stage.mkdir(parents=True, mode=0o700)
        source = stage/f'RTXManager-{version}-x86_64.AppImage'
        shutil.copyfile(image, source)
        source.chmod(0o755)
        parent = subprocess.Popen(['sleep', '120'])
        manifest = dict(schema=3, version=version, file=source.name,
            sha256=digest(source), bytes=source.stat().st_size, notes={})
        ticket = stage/'install-linux.json'
        ticket.write_text(json.dumps(dict(schema=1, target=str(target),
            old_sha256=old, manifest=manifest, parent=parent.pid,
            data_dir=str(data), wine_prefix=None)), encoding='utf-8')
        env = dict(os.environ, APPIMAGE_EXTRACT_AND_RUN='1', HOME=str(root/'home'),
                   XDG_DATA_HOME=str(root/'xdg'), XDG_RUNTIME_DIR=str(root/'runtime'))
        Path(env['HOME']).mkdir()
        Path(env['XDG_RUNTIME_DIR']).mkdir(mode=0o700)
        if not success:
            env.pop('DISPLAY', None)
            env.pop('WAYLAND_DISPLAY', None)
        with (root/'output.log').open('w+') as log:
            helper = subprocess.Popen([str(source), '--apply-update', str(ticket)],
                env=env, stdout=log, stderr=log, start_new_session=True)
            try:
                until = time.monotonic()+45
                while not (stage/'helper-ready.json').exists():
                    if helper.poll() is not None or time.monotonic() > until:
                        raise RuntimeError('Installer did not become ready')
                    time.sleep(.1)
                assert digest(target) == old, 'Old image changed before parent exited'
                parent.terminate()
                parent.wait(timeout=10)
                result = helper.wait(timeout=90)
                if success:
                    assert result == 0 and (stage/'installed.json').exists()
                    assert (stage/'ready.json').exists(), 'New UI did not acknowledge startup'
                    assert digest(target) == manifest['sha256']
                    assert target.stat().st_mode & 0o111
                    assert not list(root.glob('*.previous'))
                    assert not list(root.glob('*.new'))
                else:
                    assert result != 0 and (stage/'failed.json').exists()
                    assert digest(target) == old, 'Failed startup did not restore old image'
                print(json.dumps(dict(case='restart' if success else 'rollback', passed=True)))
            except Exception:
                log.flush()
                log.seek(0)
                print(log.read(), file=sys.stderr)
                raise
            finally:
                if parent.poll() is None:
                    parent.terminate()
                    parent.wait(timeout=10)
                try:
                    os.killpg(helper.pid, signal.SIGTERM)
                except ProcessLookupError:
                    pass
                if helper.poll() is None:
                    helper.wait(timeout=10)


if __name__ == '__main__':
    image = Path(sys.argv[1]).resolve()
    version = image.name.removeprefix('RTXManager-').removesuffix('-x86_64.AppImage')
    run(image, version, False)
    run(image, version, True)
