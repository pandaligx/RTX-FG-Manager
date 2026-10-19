import copy
import importlib.util
import json
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('names', ROOT / 'tools/build_game_names.py')
names = importlib.util.module_from_spec(spec)
spec.loader.exec_module(names)


class CatalogTests(unittest.TestCase):
    def setUp(self):
        self.catalog = json.loads((ROOT / 'rust/assets/game-names.json').read_text(encoding='utf-8'))

    def test_bundled_matches_immutable_manifest(self):
        manifest = json.loads((ROOT / 'cloud/game-names.json').read_text())
        data = (ROOT / 'cloud/names' / manifest['file']).read_bytes()
        self.assertEqual(data, (ROOT / 'rust/assets/game-names.json').read_bytes())
        self.assertEqual(names.hashlib.sha256(data).hexdigest(), manifest['sha256'])
        self.assertEqual(len(data), manifest['bytes'])
        names.validate(self.catalog)

    def test_truncated_source_and_mass_renames_are_rejected(self):
        next_catalog = copy.deepcopy(self.catalog)
        next_catalog['entries'] = next_catalog['entries'][:100]
        with self.assertRaises(ValueError): names.guard(self.catalog, next_catalog)
        next_catalog = copy.deepcopy(self.catalog)
        for game in next_catalog['entries'][:200]: game['zh'] = '错误名称'
        with self.assertRaises(ValueError): names.guard(self.catalog, next_catalog)

    def test_curated_corrections_survive_refresh(self):
        overrides = json.loads((ROOT / 'tools/game-name-overrides.json').read_text(encoding='utf-8'))
        next_catalog = copy.deepcopy(self.catalog)
        for game in next_catalog['entries']:
            if game['id'] == 'Q112307228': game['zh'] = '風暴之門'
        names.customize(next_catalog, overrides)
        self.assertEqual(next_catalog['entries'], self.catalog['entries'])
        names.guard(self.catalog, next_catalog)

    def test_paths_exe_only_and_conflicting_ids_rejected(self):
        for field, value in [('files', ['../other']), ('executables', ['../Game.exe'])]:
            catalog = copy.deepcopy(self.catalog)
            catalog['rules'][0][field] = value
            with self.assertRaises(ValueError): names.validate(catalog)
        catalog = copy.deepcopy(self.catalog)
        catalog['rules'][0].pop('files')
        with self.assertRaises(ValueError): names.validate(catalog)
        catalog = copy.deepcopy(self.catalog)
        catalog['entries'][1]['keys'].append(catalog['entries'][0]['keys'][0])
        with self.assertRaises(ValueError): names.validate(catalog)

    def test_automation_has_explicit_mirror_and_only_name_paths(self):
        workflow = (ROOT / '.github/workflows/update-game-names.yml').read_text()
        self.assertIn('python tools/gitcode_ci.py', workflow)
        self.assertIn('RELEASE_TAG: \'\'', workflow)
        self.assertIn('git add -- rust/assets/game-names.json cloud/game-names.json cloud/names', workflow)
        self.assertNotIn('--force', workflow)


if __name__ == '__main__': unittest.main()
