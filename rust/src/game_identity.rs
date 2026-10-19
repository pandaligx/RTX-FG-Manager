//! Bounded read-only identity adapters. No directory walking beyond known
//! metadata locations, no EXE hashes, no network calls and no ACL changes.
use crate::{
    core,
    game_installations::{self as installs, Install},
    game_name_windows as windows, game_names,
    scanner::Game,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::{
        OnceLock,
        atomic::{AtomicBool, Ordering},
    },
};

#[derive(Clone, Debug, Default)]
pub struct Identity {
    keys: Vec<String>,
    title: String,
    localized_title: String,
    source: String,
    catalog_id: String,
    revision: u64,
}
impl Identity {
    fn new(key: String, title: String) -> Self {
        Self {
            keys: if game_names::valid_key(&key) {
                vec![key]
            } else {
                vec![]
            },
            title: if game_names::valid_title(&title) {
                title
            } else {
                String::new()
            },
            ..Default::default()
        }
    }
    pub fn value(&self) -> Value {
        json!({"keys":self.keys,"title":self.title,"localized_title":self.localized_title,"source":self.source,"catalog_id":self.catalog_id,"revision":self.revision})
    }
}
fn read(path: &Path) -> Option<Vec<u8>> {
    core::no_links(path).ok()?;
    let file = fs::File::open(path).ok()?;
    if !file.metadata().ok()?.is_file() || file.metadata().ok()?.len() > 1024 * 1024 {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(1024 * 1024 + 1).read_to_end(&mut bytes).ok()?;
    (bytes.len() <= 1024 * 1024).then_some(bytes)
}
fn fields(text: &str) -> BTreeMap<String, String> {
    static FIELD: OnceLock<regex::Regex> = OnceLock::new();
    FIELD
        .get_or_init(|| regex::Regex::new(r#"(?m)^\s*"([A-Za-z]+)"\s*"([^"\r\n]*)""#).unwrap())
        .captures_iter(text)
        .map(|m| (m[1].to_ascii_lowercase(), m[2].to_string()))
        .collect()
}
fn steam_root(exe: &Path) -> Option<&Path> {
    exe.ancestors().find(|p| {
        p.parent().is_some_and(|p| {
            p.file_name()
                .is_some_and(|n| n.eq_ignore_ascii_case("common"))
                && p.parent()
                    .and_then(Path::file_name)
                    .is_some_and(|n| n.eq_ignore_ascii_case("steamapps"))
        })
    })
}
fn steam_installs(apps: &Path, cancel: &AtomicBool) -> BTreeMap<String, Identity> {
    let mut result = BTreeMap::new();
    let Ok(entries) = fs::read_dir(apps) else {
        return result;
    };
    let mut ambiguous = std::collections::BTreeSet::new();
    for entry in entries.take(4096).flatten() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let file = entry.file_name().to_string_lossy().to_lowercase();
        let Some(id) = file
            .strip_prefix("appmanifest_")
            .and_then(|s| s.strip_suffix(".acf"))
        else {
            continue;
        };
        let Some(bytes) = read(&entry.path()) else {
            continue;
        };
        let f = fields(&String::from_utf8_lossy(&bytes));
        if f.get("appid").is_none_or(|v| v != id) {
            continue;
        }
        let Some(folder) = f.get("installdir") else {
            continue;
        };
        if folder.is_empty() || folder == "." || folder == ".." || folder.contains(['/', '\\', ':'])
        {
            continue;
        }
        let root = apps.join("common").join(folder);
        let key = core::key(&root);
        if result
            .insert(
                key.clone(),
                Identity::new(
                    format!("steam:{id}"),
                    f.get("name").cloned().unwrap_or_default(),
                ),
            )
            .is_some()
        {
            ambiguous.insert(key);
        }
    }
    for key in ambiguous {
        result.remove(&key);
    }
    result
}
fn gog(exe: &Path, root: &Path) -> Option<Identity> {
    let entries = fs::read_dir(root).ok()?;
    let mut found = None;
    for entry in entries.take(1024).flatten() {
        let file = entry.file_name().to_string_lossy().to_lowercase();
        let Some(id) = file
            .strip_prefix("goggame_")
            .or_else(|| file.strip_prefix("goggame-"))
            .and_then(|s| s.strip_suffix(".info"))
        else {
            continue;
        };
        let Some(bytes) = read(&entry.path()) else {
            continue;
        };
        let Ok(data) = serde_json::from_slice::<Value>(&bytes) else {
            continue;
        };
        if data["gameId"].as_str() != Some(id)
            || data["rootGameId"].as_str().is_some_and(|v| v != id)
        {
            continue;
        }
        let declared = data["playTasks"].as_array().is_some_and(|a| {
            a.iter().any(|v| {
                if v["category"]
                    .as_str()
                    .is_some_and(|c| !["game", "launcher"].contains(&c))
                {
                    return false;
                }
                let Some(p) = v["path"].as_str() else {
                    return false;
                };
                let path = Path::new(p);
                path.components()
                    .all(|c| matches!(c, std::path::Component::Normal(_)))
                    && !p.contains(':')
                    && core::key(&root.join(path)) == core::key(exe)
            })
        });
        if !declared {
            continue;
        }
        if found.is_some() {
            return None;
        }
        found = Some(Identity::new(
            format!("gog:{id}"),
            data["name"].as_str().unwrap_or_default().into(),
        ));
    }
    found
}
pub fn identify(
    games: &[Game],
    catalog: &game_names::Catalog,
    cancel: &AtomicBool,
) -> Vec<(String, String, Value)> {
    let mut records = windows::registry(cancel);
    if let Some(data) = std::env::var_os("ProgramData").map(PathBuf::from) {
        records.extend(installs::epic(&data, cancel));
        if let Some(bytes) =
            installs::read(&data.join("Battle.net/Agent/product.db"), 8 * 1024 * 1024)
        {
            records.extend(installs::battlenet(&bytes));
        }
    }
    let shortcuts = windows::shortcuts(games, cancel);
    identify_records(games, catalog, &records, &shortcuts, cancel)
}
fn identify_records(
    games: &[Game],
    catalog: &game_names::Catalog,
    records: &[Install],
    shortcuts: &BTreeMap<String, Vec<String>>,
    cancel: &AtomicBool,
) -> Vec<(String, String, Value)> {
    let mut steam = BTreeMap::<String, BTreeMap<String, Identity>>::new();
    let mut rows = Vec::new();
    for game in games.iter().take(10000) {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let exe = Path::new(&game.exe);
        let root = Path::new(&game.root);
        // Retain identities when a library is unplugged, inaccessible or linked.
        if !exe.parent().is_some_and(Path::is_dir) || core::no_links(exe).is_err() {
            continue;
        }
        let mut identity = Identity::default();
        if let Some(steam_root) = steam_root(exe) {
            let apps = steam_root.parent().unwrap().parent().unwrap();
            let manifests = steam
                .entry(core::key(apps))
                .or_insert_with(|| steam_installs(apps, cancel));
            if Install::new(steam_root.to_path_buf(), "Steam", "").matches(game)
                && let Some(i) = manifests.get(&core::key(root))
            {
                identity = i.clone();
                identity.source = "Steam".into();
            }
        } else if let Some(xbox) = crate::xbox::declared(exe) {
            identity = Identity::new(format!("xbox:{}", xbox.store_id.to_lowercase()), xbox.title);
            identity.source = "Xbox".into();
        }
        let mut local = Vec::new();
        for folder in exe
            .parent()
            .into_iter()
            .flat_map(|p| p.ancestors().take(8))
            .take_while(|p| core::within(p, root))
        {
            if let Some(g) = gog(exe, folder) {
                let mut i = Install::new(folder.to_path_buf(), "GOG", &g.title);
                i.keys = g.keys;
                i.executable = Some(exe.to_path_buf());
                local.push(i);
            }
            if let Some(i) = installs::ea(folder).filter(|i| i.matches(game)) {
                local.push(i);
            }
            if let Some(i) = installs::build_info(folder).filter(|i| i.matches(game)) {
                local.push(i);
            }
            if let Some(i) = installs::hoyo(exe, folder).filter(|i| i.matches(game)) {
                local.push(i);
            }
        }
        let mut candidates: Vec<_> = local
            .iter()
            .chain(records.iter())
            .filter(|i| i.matches(game))
            .collect();
        candidates.sort_by_key(|i| i.priority);
        // Collect independent IDs so conflicting records cannot be resolved by order.
        // Lower-priority uninstall entries never override a launcher title.
        for i in &candidates {
            for key in &i.keys {
                if !identity.keys.contains(key) {
                    identity.keys.push(key.clone());
                }
            }
        }
        if identity.title.is_empty()
            && let Some(first) = candidates.iter().find(|i| !i.title.is_empty())
        {
            let titles: std::collections::BTreeSet<_> = candidates
                .iter()
                .filter(|i| i.priority == first.priority && !i.title.is_empty())
                .map(|i| i.title.as_str())
                .collect();
            if titles.len() == 1 {
                identity.title = first.title.clone();
                identity.source = first.source.clone();
            }
        }
        let mut products = windows::products(exe);
        products.extend(installs::unity_products(exe));
        if !game_names::is_chinese(&identity.title) {
            // Existing Chinese names must agree at the same evidence level.
            let registry_titles: Vec<_> = candidates
                .iter()
                .filter(|i| game_names::is_chinese(&i.title))
                .map(|i| i.title.clone())
                .collect();
            let shortcut_titles = shortcuts.get(&core::key(exe)).cloned().unwrap_or_default();
            let product_titles: Vec<_> = products
                .iter()
                .filter(|s| game_names::is_chinese(s))
                .cloned()
                .collect();
            for (names, source) in [
                (registry_titles, "Windows"),
                (shortcut_titles, "Shortcut"),
                (product_titles, "Product"),
            ] {
                let names: std::collections::BTreeSet<_> = names.iter().collect();
                if names.is_empty() {
                    continue;
                }
                if names.len() == 1 {
                    identity.localized_title = (*names.first().unwrap()).clone();
                    if identity.source.is_empty() {
                        identity.source = source.into();
                    }
                }
                break;
            }
        }
        identity.keys.sort();
        identity.keys.dedup();
        identity.keys.truncate(16);
        let mut cues = identity.keys.clone();
        if let Some(key) = installs::rail_key(exe) {
            cues.push(key);
        }
        if let Some(entry) = catalog.rule_match(exe, root, &products, &cues) {
            identity.catalog_id = entry.id().into();
            identity.revision = catalog.revision();
            if identity.source.is_empty() {
                identity.source = "Catalog rule".into();
            }
        }
        rows.push((game.exe.clone(), game.root.clone(), identity.value()));
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn screenshot_standalone_games_and_manual_override() {
        let t = tempfile::tempdir().unwrap();
        let catalog = game_names::bundled();
        for (folder, exe, title) in [
            ("ZenlessZoneZero Game", "ZenlessZoneZero.exe", "绝区零"),
            ("yysls/yysls_medium", "yysls.exe", "燕云十六声"),
            ("Arknights Endfield", "Endfield.exe", "明日方舟：终末地"),
            ("Naraka/program", "NarakaBladepoint.exe", "永劫无间"),
            (
                "WeGameApps/rail_apps/DeltaForce(2001918)/DeltaForce/Binaries/Win64",
                "DeltaForceClient-Win64-Shipping.exe",
                "三角洲行动",
            ),
            (
                "英雄联盟/TFT/Binaries/Win64",
                "TFTTencentClient-Win64-Shipping.exe",
                "云顶之弈",
            ),
        ] {
            let root = t.path().join(folder);
            fs::create_dir_all(&root).unwrap();
            let mut g = Game {
                exe: root.join(exe).display().to_string(),
                root: root.display().to_string(),
                title: exe.into(),
                ..Default::default()
            };
            let result = identify_records(
                std::slice::from_ref(&g),
                &catalog,
                &[],
                &BTreeMap::new(),
                &AtomicBool::new(false),
            );
            g.extra.insert("name_identity".into(), result[0].2.clone());
            assert_eq!(game_names::display(&g, &catalog, true), title, "{folder}");
            g.extra.insert("title_override".into(), json!("我的名字"));
            assert_eq!(game_names::display(&g, &catalog, true), "我的名字");
        }
        let root = t.path().join("Unknown");
        fs::create_dir(&root).unwrap();
        let mut g = Game {
            exe: root.join("Endfield.exe").display().to_string(),
            root: root.display().to_string(),
            ..Default::default()
        };
        g.extra.insert(
            "name_identity".into(),
            identify_records(
                std::slice::from_ref(&g),
                &catalog,
                &[],
                &BTreeMap::new(),
                &AtomicBool::new(false),
            )[0]
            .2
            .clone(),
        );
        assert_eq!(game_names::display(&g, &catalog, true), "Endfield");
    }
    #[test]
    fn localized_records_conflicts_and_cancellation() {
        let t = tempfile::tempdir().unwrap();
        let c = game_names::bundled();
        let mut g = Game {
            exe: t.path().join("game.exe").display().to_string(),
            root: t.path().display().to_string(),
            title: "Game.exe".into(),
            ..Default::default()
        };
        let records = [Install::new(t.path().to_path_buf(), "Windows", "中文游戏")];
        let found = identify_records(
            std::slice::from_ref(&g),
            &c,
            &records,
            &BTreeMap::new(),
            &AtomicBool::new(false),
        );
        g.extra.insert("name_identity".into(), found[0].2.clone());
        assert_eq!(game_names::display(&g, &c, true), "中文游戏");
        let records = [
            records[0].clone(),
            Install::new(t.path().to_path_buf(), "Windows", "其他游戏"),
        ];
        let found = identify_records(
            std::slice::from_ref(&g),
            &c,
            &records,
            &BTreeMap::new(),
            &AtomicBool::new(false),
        );
        assert_eq!(found[0].2["title"], "");
        assert!(
            identify_records(&[g], &c, &records, &BTreeMap::new(), &AtomicBool::new(true))
                .is_empty()
        );
    }
    fn identify_test(
        games: &[Game],
        epic_dir: Option<&Path>,
        cancel: &AtomicBool,
    ) -> Vec<(String, String, Value)> {
        let records = epic_dir
            .map(|dir| {
                fs::read_dir(dir)
                    .unwrap()
                    .flatten()
                    .filter_map(|e| {
                        let v: Value = serde_json::from_slice(&read(&e.path())?).ok()?;
                        let mut i = Install::new(
                            PathBuf::from(v["InstallLocation"].as_str()?),
                            "Epic",
                            v["DisplayName"].as_str().unwrap_or_default(),
                        );
                        i.add_key(format!(
                            "epic:{}/{}",
                            v["CatalogNamespace"].as_str()?,
                            v["CatalogItemId"].as_str()?
                        ));
                        Some(i)
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        identify_records(
            games,
            &game_names::bundled(),
            &records,
            &BTreeMap::new(),
            cancel,
        )
    }
    #[test]
    fn xbox_store_identity_and_epic_namespace_are_distinct() {
        let t = tempfile::tempdir().unwrap();
        let xbox = t.path().join("Xbox");
        fs::create_dir_all(&xbox).unwrap();
        fs::write(xbox.join("MicrosoftGame.config"),r#"<Game><StoreId>BX3M8L83BBRW</StoreId><ExecutableList><Executable Name="Game.exe" TargetDeviceFamily="PC"/></ExecutableList><ShellVisuals DefaultDisplayName="Cyberpunk 2077"/></Game>"#).unwrap();
        let game = Game {
            exe: xbox.join("Game.exe").display().to_string(),
            root: xbox.display().to_string(),
            ..Default::default()
        };
        assert_eq!(
            identify_test(&[game], None, &AtomicBool::new(false))[0].2["keys"],
            json!(["xbox:bx3m8l83bbrw"])
        );
        let epic = t.path().join("Epic");
        let manifests = t.path().join("Manifests");
        fs::create_dir_all(&epic).unwrap();
        fs::create_dir_all(&manifests).unwrap();
        fs::write(manifests.join("test.item"),json!({"InstallLocation":epic,"CatalogNamespace":"sandbox","CatalogItemId":"item","DisplayName":"Cyberpunk 2077"}).to_string()).unwrap();
        let game = Game {
            exe: epic.join("Game.exe").display().to_string(),
            root: epic.display().to_string(),
            ..Default::default()
        };
        let found = identify_test(&[game], Some(&manifests), &AtomicBool::new(false));
        assert_eq!(found[0].2["keys"], json!(["epic:sandbox/item"]));
        assert!(
            crate::game_names::bundled()
                .matched(&["epic:sandbox/item".into()])
                .is_none()
        );
    }
    #[test]
    fn steam_requires_filename_id_and_preserves_nested_game_identity() {
        let t = tempfile::tempdir().unwrap();
        let apps = t.path().join("steamapps");
        let root = apps.join("common/Game");
        fs::create_dir_all(root.join("Child")).unwrap();
        let manifest = apps.join("appmanifest_292030.acf");
        fs::write(
            &manifest,
            "\"appid\" \"292030\"\n\"installdir\" \"Game\"\n\"name\" \"The Witcher 3\"",
        )
        .unwrap();
        let game = Game {
            exe: root.join("Game.exe").display().to_string(),
            root: root.display().to_string(),
            ..Default::default()
        };
        let result = identify_test(std::slice::from_ref(&game), None, &AtomicBool::new(false));
        assert_eq!(result[0].2["keys"][0], "steam:292030");
        let child = Game {
            exe: root.join("Child/Game.exe").display().to_string(),
            root: root.join("Child").display().to_string(),
            ..Default::default()
        };
        assert_eq!(
            identify_test(&[child], None, &AtomicBool::new(false))[0].2["keys"],
            json!([])
        );
        fs::write(manifest, "\"appid\" \"999\"\n\"installdir\" \"Game\"").unwrap();
        assert_eq!(
            identify_test(&[game], None, &AtomicBool::new(false))[0].2["keys"],
            json!([])
        );
    }
    #[test]
    fn gog_dlc_and_path_traversal_are_not_game_identities() {
        let t = tempfile::tempdir().unwrap();
        let exe = t.path().join("game.exe");
        let path = t.path().join("goggame-1207664663.info");
        let mut v = json!({"gameId":"1207664663","rootGameId":"1207664663","name":"The Witcher 3","playTasks":[{"category":"game","path":"game.exe"}]});
        fs::write(&path, v.to_string()).unwrap();
        assert_eq!(gog(&exe, t.path()).unwrap().keys, vec!["gog:1207664663"]);
        v["rootGameId"] = json!("999");
        fs::write(&path, v.to_string()).unwrap();
        assert!(gog(&exe, t.path()).is_none());
        v["rootGameId"] = json!("1207664663");
        v["playTasks"][0]["path"] = json!("../game.exe");
        fs::write(path, v.to_string()).unwrap();
        assert!(gog(&exe, t.path()).is_none());
    }
}
