"""Aggregation guards: pagination, resource releases and failure retention."""
from pathlib import Path
import copy
import json
import runpy
import tempfile
import unittest
from unittest.mock import patch
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

    def test_badge_uses_k_suffix_but_retains_exact_accessible_total(self):
        for total, visible in ((0, '0'), (999, '999'), (1000, '1k'), (1050, '1.1k'),
                               (1500, '1.5k'), (60466, '60.5k'), (682256, '682.3k')):
            with self.subTest(total=total):
                svg = ET.fromstring(stats['badge'](total))
                texts = svg.findall('.//{http://www.w3.org/2000/svg}text')
                self.assertEqual(texts[-1].text, visible)
                self.assertEqual(svg.attrib['aria-label'], f'downloads: {total:,}')

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

    def test_each_counter_regression_retains_both_files_and_timestamp(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / 'docs').mkdir()
            stats['refresh'](root, fixture)
            before = {p.name: p.read_bytes() for p in (root / 'docs').iterdir()}
            for target in ('github', *stats['REPOS']):
                for value in (0, 1):
                    def regression(url):
                        data = fixture(url)
                        if target == 'github' and '/assets?' in url:
                            for row in data:
                                row['download_count'] = value
                        elif url == GC + target + '/download_statistics':
                            data['download_statistics_history_total'] = value
                        elif '/assets?' in url:
                            # The grand total grows, but one GitCode count is lost.
                            for row in data:
                                row['download_count'] += 1000
                        return data
                    with self.subTest(source=target, value=value):
                        with self.assertRaises(stats['CounterRegression']):
                            stats['refresh'](root, regression, retry_delays=())
                        self.assertEqual(before, {p.name: p.read_bytes() for p in (root / 'docs').iterdir()})

    def test_transient_regression_retries_whole_snapshot_without_mixing_counts(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / 'docs').mkdir()
            stats['refresh'](root, fixture)
            before = {p.name: p.read_bytes() for p in (root / 'docs').iterdir()}
            attempts, waits = [], []
            def fluctuating(url):
                if url == GH + '/releases?per_page=100&page=1':
                    attempts.append(url)
                data = fixture(url)
                if len(attempts) < 3:
                    if url == GC + stats['REPOS'][1] + '/download_statistics':
                        data['download_statistics_history_total'] = 0
                    elif '/assets?' in url:
                        for row in data:
                            row['download_count'] += 1000
                else:
                    if url == GC + stats['REPOS'][0] + '/download_statistics':
                        data['download_statistics_history_total'] += 5
                return data
            def wait(delay):
                waits.append(delay)
                self.assertEqual(before, {p.name: p.read_bytes() for p in (root / 'docs').iterdir()})
            result = stats['refresh'](root, fluctuating, sleep=wait)
            self.assertEqual(len(attempts), 3)
            self.assertEqual(waits, [5, 15])
            self.assertEqual(result['total'], 272)
            self.assertEqual([s['downloads'] for s in result['sources']], [207, 35, 30])

    def test_persistent_regression_remains_failure_and_keeps_complete_snapshot(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / 'docs').mkdir()
            stats['refresh'](root, fixture)
            before = {p.name: p.read_bytes() for p in (root / 'docs').iterdir()}
            attempts, waits = [], []
            def regression(url):
                if url == GH + '/releases?per_page=100&page=1':
                    attempts.append(url)
                data = fixture(url)
                if url == GC + stats['REPOS'][0] + '/download_statistics':
                    data['download_statistics_history_total'] = 1
                return data
            with self.assertRaisesRegex(stats['CounterRegression'], r'GitCode gcw_qWIjmve7/RTX-FG-Manager: 30 -> 1'):
                stats['refresh'](root, regression, sleep=waits.append)
            self.assertEqual(len(attempts), 3)
            self.assertEqual(waits, [5, 15])
            self.assertEqual(before, {p.name: p.read_bytes() for p in (root / 'docs').iterdir()})

    def test_temporary_transport_failures_retry_without_exposing_response_details(self):
        errors = (TimeoutError('private request detail'),
                  stats['urllib'].error.URLError('private request detail'),
                  stats['urllib'].error.HTTPError(GC, 429, 'private response', {}, None),
                  stats['urllib'].error.HTTPError(GH, 503, 'private response', {}, None),
                  stats['http'].client.IncompleteRead(b'private response'))
        for error in errors:
            with self.subTest(error=type(error).__name__), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                (root / 'docs').mkdir()
                waits, attempts = [], []
                def fetch(url):
                    attempts.append(url)
                    if len(attempts) == 1:
                        raise error
                    return fixture(url)
                with patch('builtins.print') as output:
                    result = stats['refresh'](root, fetch, sleep=waits.append)
                self.assertEqual(result['total'], 267)
                self.assertEqual(waits, [5])
                self.assertNotIn('private', str(output.call_args_list))

    def test_authentication_and_malformed_responses_are_not_retried(self):
        errors = (stats['urllib'].error.HTTPError(GC, 401, 'Unauthorized', {}, None),
                  stats['urllib'].error.HTTPError(GC, 403, 'Forbidden', {}, None),
                  ValueError('Malformed response'))
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / 'docs').mkdir()
            stats['refresh'](root, fixture)
            before = {p.name: p.read_bytes() for p in (root / 'docs').iterdir()}
            for error in errors:
                waits, calls = [], []
                def fetch(url):
                    calls.append(url)
                    raise error
                with self.assertRaises(type(error)):
                    stats['refresh'](root, fetch, sleep=waits.append)
                self.assertEqual(len(calls), 1)
                self.assertEqual(waits, [])
                self.assertEqual(before, {p.name: p.read_bytes() for p in (root / 'docs').iterdir()})

    def test_unchanged_or_growing_counters_publish_one_correct_total(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / 'docs').mkdir()
            stats['refresh'](root, fixture)
            def increasing(url):
                data = fixture(url)
                if url == GC + stats['REPOS'][0] + '/download_statistics':
                    data['download_statistics_history_total'] += 5
                return data
            data = stats['refresh'](root, increasing)
            self.assertEqual(data['total'], 272)
            self.assertEqual(json.loads((root / 'docs/download-stats.json').read_bytes()), data)
            self.assertEqual(ET.parse(root / 'docs/downloads-total.svg').getroot().attrib['aria-label'],
                             'downloads: 272')
            self.assertEqual(stats['refresh'](root, increasing)['total'], 272)

    def test_changed_or_corrupt_baseline_requires_review(self):
        data = stats['collect'](fixture)
        invalid = []
        for field, value in (('total', 0), ('schema_version', 2)):
            before = copy.deepcopy(data)
            before[field] = value
            invalid.append(before)
        for change in ('missing', 'duplicate', 'scope'):
            before = copy.deepcopy(data)
            if change == 'missing':
                before['sources'].pop()
                before['total'] -= 30
            elif change == 'duplicate':
                before['sources'].append(before['sources'][0])
                before['total'] += 207
            else:
                before['sources'][0]['scope'] = 'different_counter'
            invalid.append(before)
        for before in invalid:
            with self.assertRaises(ValueError):
                stats['validate_refresh'](data, before)

    def test_credentials_are_headers_on_exact_platform_endpoints_only(self):
        with patch.dict(stats['os'].environ, {'GH_TOKEN': 'fake-gh', 'GITCODE_TOKEN': 'fake-gc'}):
            with patch.object(stats['urllib'].request, 'build_opener') as opener:
                opener.return_value.open.return_value.__enter__.return_value.read.return_value = b'{}'
                for url, token in ((GH + '/releases', 'fake-gh'),
                                   (GC + stats['REPOS'][0] + '/download_statistics', 'fake-gc'),
                                   (GC + stats['REPOS'][1] + '/download_statistics', 'fake-gc'),
                                   ('https://example.org/download_statistics', None),
                                   (GC + 'other/download_statistics', None)):
                    stats['get_json'](url)
                    request = opener.return_value.open.call_args.args[0]
                    self.assertEqual(request.full_url, url)
                    self.assertEqual(request.get_header('Authorization'), 'Bearer ' + token if token else None)
                    self.assertEqual(request.get_header('Cache-control'), 'no-cache')


if __name__ == '__main__':
    unittest.main()
