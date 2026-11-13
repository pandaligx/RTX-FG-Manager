"""Prepare the independent Linux AppImage update channel; never touch Windows metadata."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil

CHANNEL = 'update-linux.json'


def checked(raw):
    m = json.loads(raw)
    if not isinstance(m, dict):
        raise RuntimeError('Invalid Linux manifest')
    version = m.get('version', '')
    if (m.get('schema') != 3 or type(m.get('schema')) is not int
            or not isinstance(version, str) or not re.fullmatch(r'[0-9]{1,4}\.[0-9]{1,4}\.[0-9]{1,4}', version)
            or m.get('file') != f'RTXManager-{version}-x86_64.AppImage'
            or type(m.get('bytes')) is not int or not 1048576 <= m['bytes'] <= 268435456
            or not isinstance(m.get('sha256'), str) or not re.fullmatch('[0-9a-f]{64}', m['sha256'])
            or not isinstance(m.get('notes', {}), dict) or len(m.get('notes', {})) > 8
            or any(not isinstance(v, str) or len(v.encode('utf-8')) > 16384 for v in m.get('notes', {}).values())):
        raise RuntimeError('Invalid Linux manifest')
    return m


def transition(previous, raw):
    new = checked(raw)
    if previous is not None:
        old = checked(previous)
        number = lambda m: tuple(map(int, m['version'].split('.')))
        if number(new) < number(old) or number(new) == number(old) and new != old:
            raise RuntimeError('Linux channel cannot roll back or replace a published version')
    return new


def prepare(image, version, notes, output):
    image, output = Path(image), Path(output)
    if image.is_symlink() or not image.is_file() or output.exists():
        raise RuntimeError('Use a regular AppImage and a new output directory')
    data = image.read_bytes()
    if (len(data) < 64 or data[:6] != b'\x7fELF\x02\x01' or data[8:11] != b'AI\x02'
            or int.from_bytes(data[18:20], 'little') != 62):
        raise RuntimeError('Expected an x86_64 Type 2 AppImage')
    m = dict(schema=3, version=version, file=f'RTXManager-{version}-x86_64.AppImage',
             bytes=len(data), sha256=hashlib.sha256(data).hexdigest(), notes=notes)
    raw = (json.dumps(m, ensure_ascii=False, indent=2) + '\n').encode()
    checked(raw)
    output.mkdir(parents=True)
    shutil.copyfile(image, output / m['file'])
    (output / m['file']).chmod(0o755)
    (output / CHANNEL).write_bytes(raw)
    (output / 'SHA256SUMS-linux.txt').write_text(f"{m['sha256']}  {m['file']}\n", encoding='ascii')
    return m


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--appimage', required=True, type=Path)
    p.add_argument('--version', required=True)
    p.add_argument('--notes', required=True, type=Path)
    p.add_argument('--output', required=True, type=Path)
    a = p.parse_args()
    print(json.dumps(prepare(a.appimage, a.version, json.loads(a.notes.read_text(encoding='utf-8')), a.output), ensure_ascii=False))
