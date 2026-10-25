"""Build a reproducible, CC0, ID-only name catalog from a saved Wikidata query.

Fetch explicitly with --fetch; publication remains a separate reviewed operation.
English titles are search hints, NEVER automatic cross-platform identity keys.
"""
import argparse
import hashlib
import json
import re
import urllib.parse
import urllib.request
import datetime
import time
from pathlib import Path

QUERY = '''SELECT ?item ?property ?id ?zh ?en WHERE {
 hint:Query hint:optimizer "None".
 { VALUES ?property { wdt:P1733 wdt:P5885 wdt:P2725 wdt:P6278 wdt:P12727 } ?item ?property ?id. }
 UNION { ?item p:P2725 ?statement. ?statement pq:P12727 ?id. BIND(wdt:P12727 AS ?property) }
 ?item rdfs:label ?zh.
 FILTER(LANG(?zh) IN ("zh-cn","zh-hans","zh"))
 OPTIONAL { ?item rdfs:label ?en FILTER(LANG(?en) IN ("en", "mul")) }
}'''
# Keep platform-ID selection before multilingual label joins. Blazegraph's
# automatic reordering can turn this bounded query into a label-wide scan (504).
PROPERTIES = {"P1733": "steam", "P5885": "xbox", "P2725": "gog_slug", "P6278": "epic_slug", "P12727": "gog"}


def valid_text(s):
    return 0 < len(s) <= 160 and not any(ord(c) < 32 or ord(c) == 127 or c in "\u202a\u202b\u202c\u202d\u202e\u2066\u2067\u2068\u2069" for c in s)


def build(rows, revision):
    english = {}
    for row in rows:
        value = row.get("en", {}).get("value", "").strip()
        if valid_text(value):
            key = row["item"]["value"]
            if key not in english or row["en"].get("xml:lang") == "en":
                english[key] = value
    games = {}
    for row in rows:
        qid = row["item"]["value"].rsplit("/", 1)[-1]
        prop = row["property"]["value"].rsplit("/", 1)[-1]
        zh, en = row["zh"]["value"].strip(), english.get(row["item"]["value"], "")
        if not valid_text(zh) or not valid_text(en) or not re.search(r"[\u3400-\u9fff]", zh):
            continue
        value = row["id"]["value"].lower()
        if prop not in PROPERTIES or not re.fullmatch(r"[a-z0-9][a-z0-9_./-]{0,159}", value):
            continue
        if prop in ("P1733", "P12727") and not re.fullmatch(r"[1-9][0-9]{0,11}", value):
            continue
        if prop == "P5885" and not re.fullmatch(r"[a-z0-9]{12}", value):
            continue
        if prop == "P2725":
            if not value.startswith("game/"):
                continue
            value = value[5:]
        rank = {"zh-cn": 0, "zh-hans": 1, "zh": 2}[row["zh"]["xml:lang"]]
        game = games.setdefault(qid, {"id": qid, "zh": zh, "en": en, "keys": set(), "rank": rank})
        if rank < game["rank"]:
            game.update(zh=zh, rank=rank)
        game["keys"].add(f"{PROPERTIES[prop]}:{value}")
    owners = {}
    for qid, game in games.items():
        for key in game["keys"]:
            owners.setdefault(key, set()).add(qid)
    conflicts = sorted(k for k, ids in owners.items() if len(ids) != 1)
    entries = []
    for game in sorted(games.values(), key=lambda g: g["id"]):
        game["keys"] = sorted(k for k in game["keys"] if k not in conflicts)
        game.pop("rank")
        if game["keys"]:
            entries.append(game)
    return {"schema": 1, "revision": revision, "source": "Wikidata (CC0)", "entries": entries}, conflicts


def customize(catalog, overrides):
    if overrides.get("schema") != 1:
        raise ValueError("Unsupported curated rules")
    entries = {e['id']: e for e in catalog['entries']}
    for qid, title in overrides.get('titles', {}).items():
        if qid not in entries or not valid_text(title):
            raise ValueError("Invalid curated title")
        entries[qid]['zh'] = title
    for qid, keys in overrides.get('keys', {}).items():
        if qid not in entries:
            raise ValueError("Unknown curated identity")
        entries[qid]['keys'] = sorted(set(entries[qid]['keys']) | set(keys))
    catalog.update(schema=2, source="Wikidata (CC0) + reviewed local identification rules", rules=overrides['rules'])
    validate(catalog)
    return catalog


def validate(catalog):
    owners, rules = set(), set()
    qids = {e['id'] for e in catalog['entries']}
    if len(qids) != len(catalog['entries']):
        raise ValueError("Duplicate games")
    for entry in catalog['entries']:
        if not re.fullmatch(r'Q[0-9]{1,19}', entry['id']) or not all(valid_text(entry[k]) for k in ('zh', 'en')):
            raise ValueError("Invalid entry")
        if not 1 <= len(entry['keys']) <= 64:
            raise ValueError("Invalid identity count")
        for key in entry['keys']:
            if key in owners or not re.fullmatch(r'(steam|xbox|gog|gog_slug|epic|epic_slug|ea|ubisoft|battlenet|wegame|hoyo|cube|sonkwo):[a-z0-9][a-z0-9_./-]{0,159}', key):
                raise ValueError("Invalid or conflicting identity")
            owners.add(key)
    def relative(value):
        return bool(value) and len(value) <= 240 and not any(c in value for c in ':\\*?') and all(p not in ('', '.', '..') for p in value.split('/'))
    for rule in catalog.get('rules', []):
        if set(rule) - {'id', 'game', 'executables', 'directories', 'files', 'products', 'keys'}:
            raise ValueError('Unknown rule condition')
        if rule['game'] not in qids or rule['id'] in rules or not re.fullmatch(r'[A-Za-z0-9_-]{1,80}', rule['id']):
            raise ValueError('Invalid rule identity')
        rules.add(rule['id'])
        if not rule['executables'] or not any(rule.get(k) for k in ('directories', 'files', 'products', 'keys')):
            raise ValueError('EXE-only rules are forbidden')
        for group in ('executables', 'directories', 'files', 'products', 'keys'):
            values = rule.get(group, [])
            if len(values) > 16 or any(not isinstance(v, str) for v in values):
                raise ValueError('Invalid rule conditions')
            if group in ('executables', 'directories', 'files') and not all(relative(v) and (group == 'files' or '/' not in v) for v in values):
                raise ValueError('Unsafe rule path')
            if group == 'executables' and any(not v.lower().endswith('.exe') for v in values):
                raise ValueError('Invalid executable')
            if group == 'products' and not all(valid_text(v) for v in values):
                raise ValueError('Invalid product')


def guard(previous, catalog):
    """An incomplete upstream response must not replace a healthy offline library."""
    old = {e['id']: e for e in previous['entries']}
    new = {e['id']: e for e in catalog['entries']}
    if not new or len(new) < len(old) * .98 or len(new) > max(len(old) * 1.3, len(old) + 200):
        raise ValueError('Unexpected catalog size; manual review required')
    if len(old.keys() - new.keys()) > max(2, len(old) * .01):
        raise ValueError('Unexpected missing games; retaining previous catalog')
    changed = sum(old[k]['zh'] != new[k]['zh'] for k in old.keys() & new.keys())
    if changed > max(20, len(old) * .02):
        raise ValueError('Unexpected translation changes; manual review required')
    for platform in PROPERTIES.values():
        count = lambda entries: sum(k.startswith(platform + ':') for e in entries for k in e['keys'])
        if count(catalog['entries']) < count(previous['entries']) * .98:
            raise ValueError('Incomplete platform identity data')


def fetch():
    url = "https://query.wikidata.org/sparql?" + urllib.parse.urlencode({"format": "json", "query": QUERY})
    req = urllib.request.Request(url, headers={"User-Agent": "RTX-FG-Manager (https://github.com/pandaligx/RTX-FG-Manager)", "Accept": "application/sparql-results+json"})
    for attempt in range(3):
        try:
            with urllib.request.urlopen(req, timeout=90) as response:
                data = response.read(32 * 1024 * 1024 + 1)
            if len(data) > 32 * 1024 * 1024:
                raise ValueError("Source too large")
            rows = json.loads(data)['results']['bindings']
            if not isinstance(rows, list) or not rows:
                raise ValueError('Empty source')
            return data
        except (OSError, ValueError, KeyError):
            if attempt == 2:
                raise
            time.sleep(5 * (attempt + 1))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("input", type=Path)
    parser.add_argument("--revision", type=int)
    parser.add_argument("--fetch", action="store_true")
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    previous = json.loads((root / 'rust/assets/game-names.json').read_text(encoding='utf-8'))
    if args.fetch:
        args.input.write_bytes(fetch())
    revision = args.revision or max(previous['revision'] + 1, int(datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%d%H%M')))
    if revision <= previous['revision']:
        raise ValueError('Revision must increase')
    catalog, conflicts = build(json.loads(args.input.read_text(encoding="utf-8-sig"))["results"]["bindings"], revision)
    customize(catalog, json.loads((root / 'tools/game-name-overrides.json').read_text(encoding='utf-8')))
    guard(previous, catalog)
    if all(previous.get(k) == catalog.get(k) for k in ('schema', 'entries', 'rules')):
        print('No catalog changes; no new revision or publication needed')
        return
    data = (json.dumps(catalog, ensure_ascii=False, separators=(",", ":")) + "\n").encode()
    digest = hashlib.sha256(data).hexdigest()
    (root / "rust/assets/game-names.json").write_bytes(data)
    target = root / "cloud/names" / f"names-{digest[:20]}.json"
    target.parent.mkdir(parents=True, exist_ok=True)
    if target.exists() and target.read_bytes() != data:
        raise ValueError("Immutable filename conflict")
    target.write_bytes(data)
    manifest = {"schema": 1, "revision": revision, "file": target.name, "bytes": len(data), "sha256": digest}
    (root / "cloud/game-names.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"entries": len(catalog["entries"]), "keys": sum(len(g["keys"]) for g in catalog["entries"]), "bytes": len(data), "excluded_conflicts": conflicts}, ensure_ascii=False))


if __name__ == "__main__":
    main()
