"""Aggregation guards: pagination, resource releases and failure retention."""
from pathlib import Path
import runpy
import tempfile
import unittest
import xml.etree.ElementTree as ET

stats = runpy.run_path(str(Path(__file__).resolve().parents[1] / 'tools/update_download_stats.py'))
GH, GC = stats['GH'], stats['GC']


def fixture(url):
    if url == GH + '/releases?per_page=100&page=1':
        return [{'id': 1, 'draft': False, 'prerelease': True}, {'id': 2, 'draft': True}]
    if url == GH + '/releases/1/assets?per_page=100&page=1':
        return [{'id': n, 'download_count': 2} for n in range(100)]
    if url == GH + '/releases/1/assets?per_page=100&page=2':
        return [{'id': 99, 'download_count': 2}, {'id': 100, 'download_count': 7}]
    if url.startswith(GC):
        return {'download_statistics_history_total': 30, 'download_statistics_total': 10,
                'download_statistics_detail': [{'total_dl_cnt': 30}]}
    raise AssertionError('Unexpected endpoint: ' + url)


class DownloadStats(unittest.TestCase):
    def test_complete_total_includes_resources_excludes_drafts_deduplicates_assets(self):
        data = stats['collect'](fixture)
        self.assertEqual([s['downloads'] for s in data['sources']], [207, 30, 30])
        self.assertEqual(data['total'], 267)
        self.assertEqual(data['excluded'][0]['platform'], 'Gitee')
        self.assertIn('267', ET.fromstring(stats['badge'](data['total'])).attrib['aria-label'])

    def test_releases_are_paginated_too(self):
        calls = []
        def fetch(url):
            calls.append(url)
            return list(range(100)) if url.endswith('page=1') else [100]
        self.assertEqual(len(list(stats['pages'](fetch, GH + '/releases'))), 101)
        self.assertEqual(len(calls), 2)

    def test_partial_response_never_overwrites_last_complete_snapshot(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / 'docs').mkdir()
            stats['refresh'](root, fixture)
            before = {p.name: p.read_bytes() for p in (root / 'docs').iterdir()}
            for invalid in ({}, {'download_statistics_history_total': None},
                            {'download_statistics_history_total': -1},
                            {'download_statistics_history_total': True}):
                def incomplete(url):
                    return invalid if 'payloads/download_statistics' in url else fixture(url)
                with self.assertRaises((KeyError, ValueError)):
                    stats['refresh'](root, incomplete)
                self.assertEqual(before, {p.name: p.read_bytes() for p in (root / 'docs').iterdir()})


if __name__ == '__main__':
    unittest.main()
