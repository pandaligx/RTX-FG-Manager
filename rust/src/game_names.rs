//! Offline name presentation. Store identities never change deployment paths.
//! Titles are search hints, not identity keys; only an explicit user choice can
//! apply a name when no unambiguous platform ID is available.
use crate::{core, scanner::Game, transfer};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

const LIMIT: u64 = 8 * 1024 * 1024;
const CACHE: &str = "game-names.json";
const DOMESTIC: &str =
    "https://api.gitcode.com/api/v5/repos/gcw_qWIjmve7/RTX-FG-Manager/raw/cloud/";
const GITHUB: &str = "https://raw.githubusercontent.com/pandaligx/RTX-FG-Manager/main/cloud/";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    id: String,
    zh: String,
    en: String,
    keys: Vec<String>,
}
impl Entry {
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn chinese(&self) -> &str {
        &self.zh
    }
    pub fn original(&self) -> &str {
        &self.en
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    schema: u32,
    revision: u64,
    source: String,
    entries: Vec<Entry>,
    #[serde(default)]
    rules: Vec<crate::game_name_rules::Rule>,
    #[serde(skip)]
    by_key: BTreeMap<String, usize>,
}
pub fn valid_title(value: &str) -> bool {
    !value.trim().is_empty()
        && value.chars().count() <= 160
        && !value.chars().any(|c| {
            c.is_control() || matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        })
}
pub(crate) fn is_chinese(value: &str) -> bool {
    value
        .chars()
        .any(|c| ('\u{3400}'..='\u{9fff}').contains(&c))
}
pub fn valid_key(key: &str) -> bool {
    let Some((platform, id)) = key.split_once(':') else {
        return false;
    };
    if id.is_empty()
        || id.len() > 160
        || !id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_.-/".contains(&b))
    {
        return false;
    }
    match platform {
        "steam" | "gog" => {
            id.len() <= 12 && !id.starts_with('0') && id.bytes().all(|b| b.is_ascii_digit())
        }
        "xbox" => id.len() == 12 && id.bytes().all(|b| b.is_ascii_alphanumeric()),
        "epic" => id
            .split_once('/')
            .is_some_and(|(a, b)| !a.is_empty() && !b.is_empty()),
        "gog_slug" | "epic_slug" | "ea" | "ubisoft" | "battlenet" | "wegame" | "hoyo" | "cube"
        | "sonkwo" => !id.contains('/') && !id.starts_with('.'),
        _ => false,
    }
}
impl Catalog {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        ensure!(bytes.len() as u64 <= LIMIT, "名称库文件过大");
        let mut catalog: Self = serde_json::from_slice(bytes)?;
        ensure!(
            (catalog.schema == 1 || catalog.schema == 2)
                && catalog.revision > 0
                && !catalog.entries.is_empty()
                && catalog.entries.len() <= 50000,
            "名称库格式无效"
        );
        ensure!(valid_title(&catalog.source), "名称库来源无效");
        let mut ids = BTreeSet::new();
        for (ix, entry) in catalog.entries.iter().enumerate() {
            ensure!(
                entry.id.starts_with('Q')
                    && entry.id.len() <= 20
                    && entry.id.len() > 1
                    && entry.id[1..].bytes().all(|b| b.is_ascii_digit())
                    && ids.insert(&entry.id),
                "名称库条目 ID 无效或重复"
            );
            ensure!(
                valid_title(&entry.zh)
                    && valid_title(&entry.en)
                    && !entry.keys.is_empty()
                    && entry.keys.len() <= 64,
                "名称库条目无效"
            );
            for key in &entry.keys {
                ensure!(
                    valid_key(key) && catalog.by_key.insert(key.clone(), ix).is_none(),
                    "名称库平台 ID 无效或冲突"
                );
            }
        }
        ensure!(catalog.rules.len() <= 5000, "名称规则过多");
        let mut rules = BTreeSet::new();
        for rule in &catalog.rules {
            rule.validate()?;
            ensure!(
                ids.contains(&rule.game().to_string()) && rules.insert(rule.id()),
                "名称规则重复或目标无效"
            );
        }
        Ok(catalog)
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    pub fn entry(&self, id: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.id == id)
    }
    pub fn matched(&self, keys: &[String]) -> Option<&Entry> {
        let indices: BTreeSet<usize> = keys
            .iter()
            .filter_map(|key| self.by_key.get(key).copied())
            .collect();
        // Conflicting independently detected platforms are never resolved by order.
        (indices.len() == 1).then(|| &self.entries[*indices.first().unwrap()])
    }
    pub fn search(&self, query: &str) -> Vec<Entry> {
        let query = query.trim().to_lowercase();
        if query.chars().count() < 2 {
            return Vec::new();
        }
        let mut hits: Vec<_> = self
            .entries
            .iter()
            .filter_map(|e| {
                let en = e.en.to_lowercase();
                let zh = e.zh.to_lowercase();
                (en.contains(&query) || zh.contains(&query))
                    .then_some((en != query && zh != query, e))
            })
            .collect();
        hits.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.en.cmp(&b.1.en)));
        hits.into_iter().take(8).map(|(_, e)| e.clone()).collect()
    }
    pub(crate) fn rule_match(
        &self,
        exe: &Path,
        root: &Path,
        products: &[String],
        keys: &[String],
    ) -> Option<&Entry> {
        let known: BTreeSet<_> = keys.iter().filter_map(|k| self.by_key.get(k)).collect();
        if known.len() > 1 {
            return None;
        }
        let ids: BTreeSet<_> = self
            .rules
            .iter()
            .filter(|r| r.matches(exe, root, products, keys))
            .map(|r| r.game())
            .collect();
        if ids.len() != 1 {
            return None;
        }
        let entry = self.entry(ids.first().copied()?)?;
        if self.matched(keys).is_some_and(|known| known.id != entry.id) {
            return None;
        }
        Some(entry)
    }
}
pub fn bundled() -> Catalog {
    Catalog::parse(include_bytes!("../assets/game-names.json")).expect("bundled game names")
}
pub fn cached(data: &Path) -> Catalog {
    let bundled = bundled();
    core::read_json(&data.join(CACHE), LIMIT * 2)
        .ok()
        .and_then(|v| serde_json::to_vec(&v).ok())
        .and_then(|v| Catalog::parse(&v).ok())
        .filter(|c| c.revision >= bundled.revision)
        .unwrap_or(bundled)
}
pub fn refresh_due(data: &Path) -> bool {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let path = data.join("game-names-check.json");
    let last = core::read_json(&path, 1024)
        .ok()
        .and_then(|v| v["attempt"].as_u64())
        .unwrap_or(0);
    if last <= now && now - last < 24 * 60 * 60 {
        return false;
    }
    // Attempt time also backs off offline failures; never retry on every launch.
    core::atomic_json(&path, &serde_json::json!({"attempt":now})).is_ok()
}
pub fn keys(game: &Game) -> Vec<String> {
    game.extra
        .get("name_identity")
        .and_then(|m| m.get("keys"))
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .take(16)
        .filter_map(|v| v.as_str())
        .filter(|v| valid_key(v))
        .map(str::to_owned)
        .collect()
}
pub fn original(game: &Game) -> String {
    let title = game.title.trim();
    if valid_title(title) && is_chinese(title) {
        return clean_name(title);
    }
    let metadata = game
        .extra
        .get("name_identity")
        .and_then(|m| m.get("title"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if valid_title(metadata) {
        return clean_name(metadata);
    }
    if valid_title(title) {
        return clean_name(title);
    }
    let leaf = game.exe.rsplit(['/', '\\']).next().unwrap_or(&game.exe);
    clean_name(leaf)
}
fn clean_name(name: &str) -> String {
    let name = name.trim();
    name.get(..name.len().saturating_sub(4))
        .filter(|_| name.to_ascii_lowercase().ends_with(".exe"))
        .unwrap_or(name)
        .into()
}
pub fn manual(game: &Game) -> Option<&str> {
    game.extra
        .get("title_override")
        .and_then(|v| v.as_str())
        .filter(|v| valid_title(v))
}
pub fn automatic(game: &Game, catalog: &Catalog, chinese: bool) -> String {
    let original = original(game);
    // Preserve already localized platform titles, including edition qualifiers.
    if !chinese
        || original
            .chars()
            .any(|c| ('\u{3400}'..='\u{9fff}').contains(&c))
    {
        return original;
    }
    if let Some(localized) = game
        .extra
        .get("name_identity")
        .and_then(|i| i.get("localized_title"))
        .and_then(|v| v.as_str())
        .filter(|s| valid_title(s) && is_chinese(s))
    {
        return localized.into();
    }
    identified(game, catalog)
        .map(|e| e.zh.clone())
        .unwrap_or(original)
}
fn identified<'a>(game: &Game, catalog: &'a Catalog) -> Option<&'a Entry> {
    catalog.matched(&keys(game)).or_else(|| {
        let identity = game.extra.get("name_identity")?;
        // Rule results are valid only for the catalog that evaluated them.
        if identity["revision"].as_u64()? != catalog.revision {
            return None;
        }
        catalog.entry(identity["catalog_id"].as_str()?)
    })
}
pub fn display(game: &Game, catalog: &Catalog, chinese: bool) -> String {
    manual(game)
        .map(str::to_owned)
        .unwrap_or_else(|| automatic(game, catalog, chinese))
}
pub fn matches_query(game: &Game, catalog: &Catalog, query: &str) -> bool {
    [
        original(game),
        display(game, catalog, true),
        game.title.clone(),
    ]
    .iter()
    .any(|s| s.to_lowercase().contains(query))
        || identified(game, catalog).is_some_and(|e| {
            e.en.to_lowercase().contains(query) || e.zh.to_lowercase().contains(query)
        })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema: u32,
    revision: u64,
    file: String,
    bytes: u64,
    sha256: String,
}
impl Manifest {
    fn validate(&self) -> Result<()> {
        ensure!(
            self.schema == 1
                && self.revision > 0
                && self.bytes > 0
                && self.bytes <= LIMIT
                && core::valid_hash(&self.sha256),
            "名称库更新清单无效"
        );
        ensure!(
            self.file == format!("names-{}.json", &self.sha256[..20]),
            "名称库文件名无效"
        );
        Ok(())
    }
}
fn sources(file: &str, prefer_github: bool) -> Vec<transfer::Source> {
    let mut pairs = [("gitcode", DOMESTIC), ("github", GITHUB)];
    if prefer_github {
        pairs.reverse()
    }
    pairs
        .into_iter()
        .map(|(name, base)| transfer::Source {
            name: name.into(),
            url: format!(
                "{base}{file}{}",
                if name == "gitcode" { "?ref=main" } else { "" }
            ),
        })
        .collect()
}
pub fn refresh(
    data: &Path,
    current: &Catalog,
    prefer_github: bool,
    cancel: &AtomicBool,
) -> Result<Catalog> {
    let bytes = transfer::fetch_metadata(
        sources("game-names.json", prefer_github),
        transfer::UrlPolicy::Official,
        cancel,
        |_| {},
    )?;
    let manifest: Manifest = serde_json::from_slice(&bytes)?;
    manifest.validate()?;
    if manifest.revision <= current.revision {
        return Ok(current.clone());
    }
    core::no_links(data)?;
    fs::create_dir_all(data)?;
    let temp = tempfile::Builder::new().prefix("names-").tempdir_in(data)?;
    let request = transfer::Request {
        sources: sources(&format!("names/{}", manifest.file), prefer_github),
        bytes: Some(manifest.bytes),
        sha256: Some(manifest.sha256),
        limit: LIMIT,
        metadata: true,
        policy: transfer::UrlPolicy::Official,
    };
    let path = transfer::download(&request, &temp.path().join("names.json"), cancel, |_| {})?;
    ensure!(!cancel.load(Ordering::Relaxed), "名称库更新已取消");
    replace_cache(data, &fs::read(path)?, manifest.revision)
}
fn replace_cache(data: &Path, bytes: &[u8], revision: u64) -> Result<Catalog> {
    let next = Catalog::parse(bytes)?;
    ensure!(next.revision == revision, "名称库更新版本不符");
    core::atomic_json(&data.join(CACHE), &next)?;
    Ok(next)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn rules_reject_conflicting_platforms_unsafe_paths_and_stale_results() {
        let t = tempfile::tempdir().unwrap();
        let exe = t.path().join("Game.exe");
        let mut value = serde_json::to_value(catalog()).unwrap();
        value["rules"] = json!([{"id":"game","game":"Q1","executables":["Game.exe"],"products":["The Witcher 3"]}]);
        let c = Catalog::parse(&serde_json::to_vec(&value).unwrap()).unwrap();
        let product = vec!["The Witcher 3".into()];
        assert!(
            c.rule_match(&exe, t.path(), &product, &["steam:378648".into()])
                .is_none()
        );
        assert!(
            c.rule_match(
                &exe,
                t.path(),
                &product,
                &["steam:292030".into(), "steam:378648".into()]
            )
            .is_none()
        );
        assert_eq!(
            c.rule_match(&exe, t.path(), &product, &[]).unwrap().id(),
            "Q1"
        );
        let mut g = Game {
            title: "Game".into(),
            ..Default::default()
        };
        g.extra.insert(
            "name_identity".into(),
            json!({"catalog_id":"Q1","revision":1}),
        );
        assert_eq!(display(&g, &c, true), "巫师3");
        g.extra.get_mut("name_identity").unwrap()["revision"] = json!(2);
        assert_eq!(display(&g, &c, true), "Game");
        value["rules"][0]["files"] = json!(["../outside"]);
        assert!(Catalog::parse(&serde_json::to_vec(&value).unwrap()).is_err());
        value["rules"][0] = json!({"id":"weak","game":"Q1","executables":["Game.exe"]});
        assert!(Catalog::parse(&serde_json::to_vec(&value).unwrap()).is_err());
    }
    #[test]
    fn imported_executable_titles_keep_the_previous_suffix_cleanup() {
        for title in ["Game.exe", "Game.EXE", "Game.Exe", "  Game.exe  "] {
            assert_eq!(
                original(&Game {
                    title: title.into(),
                    ..Default::default()
                }),
                "Game"
            );
        }
        assert_eq!(
            original(&Game {
                exe: "C:/Games/Game.Exe".into(),
                ..Default::default()
            }),
            "Game"
        );
        assert_eq!(
            original(&Game {
                title: "游戏".into(),
                ..Default::default()
            }),
            "游戏"
        );
    }
    #[test]
    fn broken_update_retains_cache_and_daily_backoff_works() {
        let t = tempfile::tempdir().unwrap();
        let bytes = serde_json::to_vec(&catalog()).unwrap();
        replace_cache(t.path(), &bytes, 1).unwrap();
        let before = fs::read(t.path().join(CACHE)).unwrap();
        assert!(replace_cache(t.path(), b"broken", 2).is_err());
        assert!(replace_cache(t.path(), &bytes, 2).is_err());
        assert_eq!(fs::read(t.path().join(CACHE)).unwrap(), before);
        assert!(refresh_due(t.path()));
        assert!(!refresh_due(t.path()));
    }
    fn catalog() -> Catalog {
        Catalog::parse(&serde_json::to_vec(&json!({"schema":1,"revision":1,"source":"test","entries":[
        {"id":"Q1","zh":"巫师3","en":"The Witcher 3","keys":["steam:292030","gog:1207664663"]},
        {"id":"Q2","zh":"巫师3：资料片","en":"The Witcher 3 DLC","keys":["steam:378648"]}
    ]})).unwrap()).unwrap()
    }
    #[test]
    fn exact_ids_only_and_conflicts_do_not_guess() {
        let c = catalog();
        assert!(c.matched(&["steam:99".into()]).is_none());
        assert!(
            c.matched(&["steam:292030".into(), "steam:378648".into()])
                .is_none()
        );
        assert_eq!(c.matched(&["gog:1207664663".into()]).unwrap().id(), "Q1");
        let game = Game {
            title: "The Witcher 3".into(),
            ..Default::default()
        };
        assert_eq!(display(&game, &c, true), "The Witcher 3");
    }
    #[test]
    fn manual_names_and_original_language_are_preserved() {
        let c = catalog();
        let mut g = Game {
            title: "The Witcher 3".into(),
            ..Default::default()
        };
        g.extra
            .insert("name_identity".into(), json!({"keys":["steam:292030"]}));
        assert_eq!(display(&g, &c, true), "巫师3");
        assert_eq!(display(&g, &c, false), "The Witcher 3");
        g.extra.get_mut("name_identity").unwrap()["localized_title"] = json!("巫师3 重制版");
        assert_eq!(display(&g, &c, true), "巫师3 重制版");
        assert_eq!(display(&g, &c, false), "The Witcher 3");
        g.extra.insert("title_override".into(), json!("我的猎魔人"));
        assert_eq!(display(&g, &c, false), "我的猎魔人");
        assert!(matches_query(&g, &c, "witcher"));
        assert!(matches_query(&g, &c, "巫师"));
        g.extra.remove("title_override");
        g.title = "巫师3 重制版".into();
        assert_eq!(display(&g, &c, true), "巫师3 重制版");
    }
    #[test]
    fn malformed_or_conflicting_catalog_is_rejected() {
        let mut c = serde_json::to_value(catalog()).unwrap();
        c["entries"][1]["keys"] = json!(["steam:292030"]);
        assert!(Catalog::parse(&serde_json::to_vec(&c).unwrap()).is_err());
        assert!(!valid_title("bad\u{202e}exe"));
        assert!(!valid_key("xbox:../hello"));
        let m = Manifest {
            schema: 1,
            revision: 2,
            file: "../../outside".into(),
            bytes: 100,
            sha256: "a".repeat(64),
        };
        assert!(m.validate().is_err());
    }
    #[test]
    fn bad_cache_falls_back_and_bundled_ids_are_unique() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(CACHE), "broken").unwrap();
        assert_eq!(cached(dir.path()).revision(), bundled().revision());
        assert!(bundled().len() > 1000);
        assert_eq!(
            bundled().matched(&["steam:292030".into()]).unwrap().id(),
            "Q4267401"
        );
    }
}
