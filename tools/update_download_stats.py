"""Refresh one combined README badge from public platform download counters.

GitHub counts published release assets (including Resources prereleases).
GitCode supplies historical repository totals, whose scope is platform-defined.
Gitee has no public attachment download counter and is explicitly excluded.
Any failed/incomplete response aborts before writing; the last snapshot survives.
"""
import argparse
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import urllib.request

GH = 'https://api.github.com/repos/pandaligx/RTX-FG-Manager'
GC = 'https://api.gitcode.com/api/v5/repos/gcw_qWIjmve7/'
REPOS = ('RTX-FG-Manager', 'RTX-FG-Manager-payloads')


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


def get_json(url):
    headers = {'User-Agent': 'RTXFG-download-stats', 'Accept': 'application/json'}
    if url.startswith(GH + '/') and os.environ.get('GH_TOKEN'):
        headers['Authorization'] = 'Bearer ' + os.environ['GH_TOKEN']
    request = urllib.request.Request(url, headers=headers)
    with urllib.request.build_opener(NoRedirect).open(request, timeout=30) as response:
        data = response.read(16 * 1024 * 1024 + 1)
    if len(data) > 16 * 1024 * 1024:
        raise ValueError('Statistics response too large')
    return json.loads(data)


def count(value):
    if type(value) is not int or value < 0:
        raise ValueError('Missing or invalid download counter')
    return value


def pages(fetch, url):
    for page in range(1, 101):
        rows = fetch(f'{url}?per_page=100&page={page}')
        if not isinstance(rows, list) or len(rows) > 100:
            raise ValueError('Invalid paginated statistics response')
        yield from rows
        if len(rows) < 100:
            return
    raise ValueError('Statistics pagination limit reached; retaining previous snapshot')


def collect(fetch=get_json):
    assets, releases = {}, set()
    for release in pages(fetch, GH + '/releases'):
        if type(release['draft']) is not bool:
            raise ValueError('Invalid release visibility')
        rid = count(release['id'])
        if release['draft'] or rid in releases:
            continue
        releases.add(rid)
        # Use the paginated asset endpoint, not the embedded preview list.
        for asset in pages(fetch, GH + f'/releases/{rid}/assets'):
            aid, downloads = count(asset['id']), count(asset['download_count'])
            if aid in assets and assets[aid] != downloads:
                raise ValueError('Asset counters changed across pages; retry next refresh')
            assets[aid] = downloads
    if not releases or not assets:
        raise ValueError('Unexpected empty release statistics')
    sources = [{'platform': 'GitHub', 'repository': 'pandaligx/RTX-FG-Manager',
                'downloads': sum(assets.values()), 'scope': 'published_release_assets'}]
    for repo in REPOS:
        data = fetch(GC + repo + '/download_statistics')
        sources.append({'platform': 'GitCode', 'repository': 'gcw_qWIjmve7/' + repo,
                        'downloads': count(data['download_statistics_history_total']),
                        'scope': 'platform_reported_repository_history'})
    return {'schema_version': 1, 'updated_at': datetime.now(timezone.utc).isoformat(timespec='seconds'),
            'total': sum(row['downloads'] for row in sources), 'sources': sources,
            'excluded': [{'platform': 'Gitee', 'reason': 'No public download counter available'}],
            'note': 'Sum of platform-reported downloads, not unique users or successful installations.'}


def badge(total):
    value = f'{count(total):,}'
    left, right = 70, len(value) * 7 + 12
    return f'''<svg xmlns="http://www.w3.org/2000/svg" width="{left + right}" height="20" role="img" aria-label="downloads: {value}">
  <title>Total reported downloads: {value}</title>
  <path fill="#555" d="M0 0h{left}v20H0z"/>
  <path fill="#38a7bc" d="M{left} 0h{right}v20H{left}z"/>
  <g fill="#fff" text-anchor="middle" font-family="Verdana,Geneva,DejaVu Sans,sans-serif" font-size="11">
    <text x="{left / 2}" y="14">downloads</text>
    <text x="{left + right / 2}" y="14">{value}</text>
  </g>
</svg>
'''


def refresh(root, fetch=get_json):
    snapshot = collect(fetch)
    outputs = {'download-stats.json': json.dumps(snapshot, ensure_ascii=False, indent=2) + '\n',
               'downloads-total.svg': badge(snapshot['total'])}
    # Both files enter a single Git commit only after a complete refresh succeeds.
    for name, content in outputs.items():
        path = root / 'docs' / name
        temporary = path.with_suffix(path.suffix + '.tmp')
        temporary.write_text(content, encoding='utf-8', newline='\n')
        temporary.replace(path)
    return snapshot


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, default=Path(__file__).resolve().parents[1])
    args = parser.parse_args()
    try:
        result = refresh(args.root)
        print(f"Combined reported downloads: {result['total']:,}")
    except Exception as exc:
        # Do not print authenticated request details or remote response bodies.
        print(f'{type(exc).__name__}: refresh failed; previous public snapshot retained')
        raise SystemExit(1)
