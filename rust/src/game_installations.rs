//! Local launcher records, separate from scanning/deployment. Every record must
//! match a library game's installation or its exact declared executable.
use crate::{core, game_name_rules::relative, game_names, scanner::Game};
use quick_xml::{Reader, events::Event};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

#[derive(Clone, Debug)]
pub(crate) struct Install {
    pub(crate) root: PathBuf,
    pub(crate) executable: Option<PathBuf>,
    pub(crate) keys: Vec<String>,
    pub(crate) title: String,
    pub(crate) source: String,
    pub(crate) priority: u8,
}
impl Install {
    pub(crate) fn new(root: PathBuf, source: &str, title: &str) -> Self {
        Self {
            root,
            executable: None,
            keys: vec![],
            title: if game_names::valid_title(title) {
                title.trim().into()
            } else {
                String::new()
            },
            source: source.into(),
            priority: 0,
        }
    }
    pub(crate) fn with_priority(mut self, p: u8) -> Self {
        self.priority = p;
        self
    }
    pub(crate) fn add_key(&mut self, key: String) {
        if game_names::valid_key(&key) && !self.keys.contains(&key) {
            self.keys.push(key);
        }
    }
    pub(crate) fn matches(&self, game: &Game) -> bool {
        let root = core::key(&self.root);
        let exe = core::key(Path::new(&game.exe));
        let game_root = core::key(Path::new(&game.root));
        if !exe.starts_with(&(root.trim_end_matches('\\').to_string() + "\\")) {
            return false;
        }
        if self
            .executable
            .as_ref()
            .is_some_and(|p| core::key(p) == exe)
        {
            return true;
        }
        if self.source == "Windows" && self.executable.is_some() {
            return false;
        }
        if root == game_root {
            return true;
        }
        // Allow a scanner's binary-only root, not an unrelated child game in a compilation.
        let prefix = root.trim_end_matches('\\').to_string() + "\\";
        game_root.strip_prefix(&prefix).is_some_and(|rest| {
            rest.split('\\').all(|part| {
                [
                    "bin", "binaries", "win64", "wingdk", "x64", "x64_dx12", "program",
                ]
                .contains(&part)
            })
        })
    }
}
pub(crate) fn read(path: &Path, limit: u64) -> Option<Vec<u8>> {
    core::no_links(path).ok()?;
    let f = fs::File::open(path).ok()?;
    if !f.metadata().ok()?.is_file() || f.metadata().ok()?.len() > limit {
        return None;
    }
    let mut data = Vec::new();
    f.take(limit + 1).read_to_end(&mut data).ok()?;
    (data.len() as u64 <= limit).then_some(data)
}
pub(crate) fn text(path: &Path) -> Option<String> {
    let bytes = read(path, 1024 * 1024)?;
    if bytes.starts_with(&[0xff, 0xfe]) {
        if bytes.len() % 2 != 0 {
            return None;
        }
        return String::from_utf16(
            &bytes[2..]
                .as_chunks::<2>()
                .0
                .iter()
                .map(|b| u16::from_le_bytes([b[0], b[1]]))
                .collect::<Vec<_>>(),
        )
        .ok();
    }
    String::from_utf8(bytes)
        .ok()
        .map(|s| s.trim_start_matches('\u{feff}').to_string())
}
pub(crate) fn ini(path: &Path) -> BTreeMap<String, String> {
    text(path)
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.trim_start().starts_with([';', '#']))
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| {
            (
                k.trim().to_ascii_lowercase(),
                v.trim().trim_matches('"').to_string(),
            )
        })
        .collect()
}
/// Legacy HoYoPlay/game config. A business ID is accepted only alongside the
/// matching declared Unity executable and its data folder; channel IDs alone
/// do not identify a game. No launcher account/cache database is read.
pub(crate) fn hoyo(exe: &Path, root: &Path) -> Option<Install> {
    let name = exe.file_name()?.to_string_lossy();
    let (prefix, title) = match name.to_ascii_lowercase().as_str() {
        "yuanshen.exe" | "genshinimpact.exe" => ("hk4e_", "原神"),
        "starrail.exe" => ("hkrpg_", "崩坏：星穹铁道"),
        "zenlesszonezero.exe" => ("nap_", "绝区零"),
        _ => return None,
    };
    let parent = exe.parent()?;
    if !parent
        .join(format!("{}_Data", exe.file_stem()?.to_string_lossy()))
        .is_dir()
    {
        return None;
    }
    let config = ini(&root.join("config.ini"));
    let biz = config.get("game_biz").or_else(|| config.get("biz"))?;
    if !biz.starts_with(prefix) {
        return None;
    }
    let mut i = Install::new(root.to_path_buf(), "HoYoPlay", title);
    i.executable = Some(exe.to_path_buf());
    i.add_key(format!("hoyo:{biz}"));
    (!i.keys.is_empty()).then_some(i)
}
/// Rail's per-install directory marker is an additional cue, never sufficient
/// on its own to assign a Chinese title (the catalog also checks the EXE).
pub(crate) fn rail_key(exe: &Path) -> Option<String> {
    for folder in exe.parent()?.ancestors().take(10) {
        if !folder
            .parent()
            .and_then(Path::file_name)
            .is_some_and(|s| s.eq_ignore_ascii_case("rail_apps"))
        {
            continue;
        }
        let name = folder.file_name()?.to_string_lossy();
        let (_, id) = name.rsplit_once('(')?;
        let id = id.strip_suffix(')')?;
        if id.is_empty() || id.len() > 12 || !id.bytes().all(|c| c.is_ascii_digit()) {
            return None;
        }
        return Some(format!("wegame:{id}"));
    }
    None
}

/// Legacy WeGame installs may identify the product in rail_files rather than
/// rail_apps or the registry. The ID alone is not a rendering-entry declaration:
/// pair the observed QQ Speed ID with its exact client layout and launcher.
pub(crate) fn wegame_renderer(exe: &Path) -> Option<Install> {
    if !exe.file_name()?.eq_ignore_ascii_case("GameApp.exe") {
        return None;
    }
    let bin = exe.parent()?;
    if !bin.file_name()?.eq_ignore_ascii_case("Releasephysx27_64") {
        return None;
    }
    let root = bin.parent()?;
    let data = read(&root.join("rail_files/rail_game_identify.json"), 4096)?;
    let value: Value = serde_json::from_slice(&data).ok()?;
    if value.get("game_id")?.as_u64()? != 17
        || !root.join("QQSpeedLauncher.exe").is_file()
        || core::no_links(&root.join("QQSpeedLauncher.exe")).is_err()
    {
        return None;
    }
    let mut record = Install::new(root.to_path_buf(), "WeGame", "QQ飞车").with_priority(1);
    record.executable = Some(exe.to_path_buf());
    record.add_key("wegame:17".into());
    Some(record)
}
fn absolute(value: &str) -> Option<PathBuf> {
    let p = PathBuf::from(value.replace('/', "\\"));
    (p.is_absolute() && !p.to_string_lossy().starts_with("\\\\") && core::no_links(&p).is_ok())
        .then_some(p)
}
fn epic_record(v: &Value) -> Option<Install> {
    if v["bIsApplication"].as_bool() == Some(false) || v["bIsExecutable"].as_bool() == Some(false) {
        return None;
    }
    if let (Some(main), Some(app)) = (v["MainGameAppName"].as_str(), v["AppName"].as_str())
        && !main.is_empty()
        && main != app
    {
        return None;
    }
    let root = absolute(v["InstallLocation"].as_str()?)?;
    let mut i = Install::new(root, "Epic", v["DisplayName"].as_str().unwrap_or_default());
    let namespace = v["CatalogNamespace"].as_str().or(v["NamespaceId"].as_str());
    let item = v["CatalogItemId"].as_str().or(v["ItemId"].as_str());
    if let (Some(ns), Some(id)) = (namespace, item) {
        i.add_key(format!("epic:{ns}/{id}").to_ascii_lowercase());
    }
    if let Some(exe) = v["LaunchExecutable"].as_str() {
        let exe = exe.replace('\\', "/");
        if relative(&exe) {
            i.executable = Some(i.root.join(exe));
        }
    }
    Some(i)
}
pub(crate) fn epic(program_data: &Path, cancel: &AtomicBool) -> Vec<Install> {
    let mut out = Vec::new();
    let folder = program_data.join("Epic/EpicGamesLauncher/Data/Manifests");
    if core::no_links(&folder).is_ok()
        && let Ok(files) = fs::read_dir(&folder)
    {
        for f in files.take(2048).flatten() {
            if cancel.load(Ordering::Relaxed) {
                return out;
            }
            if !f
                .path()
                .extension()
                .is_some_and(|s| s.eq_ignore_ascii_case("item"))
            {
                continue;
            }
            if let Some(data) = read(&f.path(), 1024 * 1024)
                && let Ok(v) = serde_json::from_slice(&data)
                && let Some(record) = epic_record(&v)
            {
                out.push(record);
            }
        }
    }
    if let Some(data) = read(
        &program_data.join("Epic/UnrealEngineLauncher/LauncherInstalled.dat"),
        2 * 1024 * 1024,
    ) && let Ok(v) = serde_json::from_slice::<Value>(&data)
        && let Some(list) = v["InstallationList"].as_array()
    {
        for row in list.iter().take(2048) {
            if let Some(mut i) = epic_record(row)
                && !out.iter().any(|e| core::key(&e.root) == core::key(&i.root))
            {
                i.priority = 1;
                out.push(i);
            }
        }
    }
    out
}
pub(crate) fn ea(root: &Path) -> Option<Install> {
    let data = text(&root.join("__Installer/installerdata.xml"))?;
    let mut r = Reader::from_str(&data);
    r.config_mut().trim_text(true);
    let mut stack = Vec::<(String, String)>::new();
    let mut title = String::new();
    let mut zh = String::new();
    let mut ids = Vec::new();
    let mut nodes = 0;
    loop {
        nodes += 1;
        if nodes > 20000 {
            return None;
        }
        match r.read_event().ok()? {
            Event::Start(e) => {
                if stack.len() > 24 {
                    return None;
                }
                let name = String::from_utf8_lossy(e.name().as_ref()).to_ascii_lowercase();
                let locale = e
                    .attributes()
                    .flatten()
                    .find(|a| a.key.as_ref().eq_ignore_ascii_case(b"locale"))
                    .and_then(|a| a.unescape_value().ok().map(|s| s.to_string()))
                    .unwrap_or_default();
                stack.push((name, locale));
            }
            Event::Text(t) => {
                let s = t.unescape().ok()?.into_owned();
                if let Some((name, locale)) = stack.last() {
                    if name == "contentid" && stack.iter().any(|(n, _)| n == "contentids") {
                        ids.push(s.clone());
                    }
                    if (name == "gametitle"
                        || name == "title" && stack.iter().any(|(n, _)| n == "gametitles"))
                        && game_names::valid_title(&s)
                    {
                        if title.is_empty() || locale.eq_ignore_ascii_case("en_US") {
                            title = s.clone();
                        }
                        if game_names::is_chinese(&s)
                            && (locale.to_lowercase().starts_with("zh") || locale.is_empty())
                        {
                            zh = s;
                        }
                    }
                }
            }
            Event::End(_) => {
                stack.pop();
            }
            Event::DocType(_) => return None,
            Event::Eof => break,
            _ => {}
        }
    }
    if ids.is_empty() {
        return None;
    }
    let mut i = Install::new(
        root.to_path_buf(),
        "EA",
        if zh.is_empty() { &title } else { &zh },
    );
    for id in ids.into_iter().take(16) {
        i.add_key(format!("ea:{}", id.to_ascii_lowercase()));
    }
    Some(i)
}
// Minimal protobuf wire reader for Battle.net's install database. Unknown fields
// are skipped; lengths, field counts, and the three schema levels are bounded.
fn varint(data: &[u8], pos: &mut usize) -> Option<u64> {
    let mut v = 0u64;
    for shift in (0..70).step_by(7) {
        let b = *data.get(*pos)?;
        *pos += 1;
        if shift == 63 && b > 1 {
            return None;
        }
        v |= u64::from(b & 127) << shift;
        if b & 128 == 0 {
            return Some(v);
        }
    }
    None
}
fn fields(data: &[u8]) -> Option<Vec<(u32, &[u8])>> {
    let mut pos = 0;
    let mut out = Vec::new();
    let mut count = 0;
    while pos < data.len() {
        count += 1;
        if count > 10000 {
            return None;
        }
        let tag = varint(data, &mut pos)?;
        if tag >> 3 == 0 {
            return None;
        }
        match tag & 7 {
            0 => {
                varint(data, &mut pos)?;
            }
            1 => pos = pos.checked_add(8)?,
            5 => pos = pos.checked_add(4)?,
            2 => {
                let len = usize::try_from(varint(data, &mut pos)?).ok()?;
                let end = pos.checked_add(len)?;
                out.push(((tag >> 3) as u32, data.get(pos..end)?));
                pos = end;
            }
            _ => return None,
        }
        if pos > data.len() {
            return None;
        }
    }
    Some(out)
}
pub(crate) fn battlenet(data: &[u8]) -> Vec<Install> {
    fields(data)
        .unwrap_or_default()
        .into_iter()
        .filter(|(n, _)| *n == 1)
        .take(2048)
        .filter_map(|(_, record)| {
            let values = fields(record)?;
            let id = std::str::from_utf8(values.iter().find(|(n, _)| *n == 1)?.1).ok()?;
            let detail = fields(values.iter().find(|(n, _)| *n == 3)?.1)?;
            let root =
                absolute(std::str::from_utf8(detail.iter().find(|(n, _)| *n == 1)?.1).ok()?)?;
            let mut i = Install::new(root, "Battle.net", "");
            i.add_key(format!("battlenet:{}", id.to_ascii_lowercase()));
            (!i.keys.is_empty()).then_some(i)
        })
        .collect()
}
pub(crate) fn build_info(root: &Path) -> Option<Install> {
    let text = text(&root.join(".build.info"))?;
    let mut lines = text.lines();
    let header: Vec<_> = lines
        .next()?
        .split('|')
        .map(|s| s.split('!').next().unwrap_or_default())
        .collect();
    let product = header.iter().position(|s| *s == "Product")?;
    let active = header.iter().position(|s| *s == "Active");
    let mut ids = Vec::new();
    for line in lines.take(64) {
        let parts: Vec<_> = line.split('|').collect();
        if active.is_some_and(|ix| parts.get(ix) != Some(&"1")) {
            continue;
        }
        if let Some(id) = parts.get(product) {
            ids.push(format!("battlenet:{}", id.to_ascii_lowercase()));
        }
    }
    ids.sort();
    ids.dedup();
    if ids.len() != 1 {
        return None;
    }
    let mut i = Install::new(root.to_path_buf(), "Battle.net", "");
    i.add_key(ids.pop()?);
    Some(i)
}
pub(crate) fn unity_products(exe: &Path) -> Vec<String> {
    let stem = exe.file_stem().unwrap_or_default().to_string_lossy();
    let p = exe
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .join(format!("{stem}_Data/app.info"));
    text(&p)
        .into_iter()
        .filter_map(|s| s.lines().nth(1).map(str::to_owned))
        .filter(|s| game_names::valid_title(s))
        .collect()
}

/// Load game-platform records once per scan. Generic Windows uninstall entries
/// include ordinary applications, so only entries with a platform ID qualify.
pub(crate) fn scan_records(cancel: &AtomicBool) -> Vec<Install> {
    let mut records = crate::game_name_windows::registry(cancel);
    records.retain(|r| r.source != "Windows" || !r.keys.is_empty());
    if let Some(data) = std::env::var_os("ProgramData").map(PathBuf::from) {
        records.extend(epic(&data, cancel));
        if let Some(bytes) = read(&data.join("Battle.net/Agent/product.db"), 8 * 1024 * 1024) {
            records.extend(battlenet(&bytes));
        }
    }
    records
}

/// Only a GOG game task is an entry point. Launcher/support tasks and DLC records
/// may describe the installation, but must not become a patch target.
pub(crate) fn gog_entries(path: &Path) -> Vec<Install> {
    let name = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_ascii_lowercase();
    let Some(id) = name
        .strip_prefix("goggame-")
        .or_else(|| name.strip_prefix("goggame_"))
        .and_then(|n| n.strip_suffix(".info"))
    else {
        return vec![];
    };
    let Some(root) = path.parent() else {
        return vec![];
    };
    let Some(data) = read(path, 1024 * 1024) else {
        return vec![];
    };
    let Ok(v) = serde_json::from_slice::<Value>(&data) else {
        return vec![];
    };
    if id.is_empty()
        || !id.bytes().all(|b| b.is_ascii_digit())
        || v["gameId"].as_str() != Some(id)
        || v["rootGameId"].as_str().is_some_and(|s| s != id)
    {
        return vec![];
    }
    v["playTasks"]
        .as_array()
        .into_iter()
        .flatten()
        .take(64)
        .filter_map(|task| {
            if task["category"].as_str().is_some_and(|s| s != "game")
                || task["type"].as_str().is_some_and(|s| s != "FileTask")
            {
                return None;
            }
            let exe = task["path"].as_str()?.replace('\\', "/");
            if !relative(&exe) || !exe.to_ascii_lowercase().ends_with(".exe") {
                return None;
            }
            let mut record =
                Install::new(root.into(), "GOG", v["name"].as_str().unwrap_or_default());
            record.executable = Some(root.join(exe));
            record.add_key(format!("gog:{id}"));
            Some(record)
        })
        .collect()
}

/// Resolve conflicting metadata conservatively; an exact entry outranks a
/// name/root match, but the caller still excludes utility and bootstrap roles.
pub(crate) fn scan_match<'a>(game: &Game, records: &'a [Install]) -> Option<(&'a Install, bool)> {
    let exe = Path::new(&game.exe);
    let stem = crate::game_engines::normalized(&exe.file_stem()?.to_string_lossy());
    let mut found: Option<(&Install, bool)> = None;
    let mut conflict = false;
    let mut products = None;
    for record in records.iter().filter(|r| r.matches(game)) {
        let exact = record
            .executable
            .as_ref()
            .is_some_and(|p| core::key(p) == core::key(exe));
        // Do not guess another EXE if a platform already declared one.
        if !exact && record.executable.is_some() {
            continue;
        }
        let title = crate::game_engines::normalized(&record.title);
        let folder = crate::game_engines::normalized(
            &record
                .root
                .file_name()
                .unwrap_or_default()
                .to_string_lossy(),
        );
        let named = stem.len() >= 3 && (stem == title || stem == folder);
        if !exact && !named {
            let products = products.get_or_insert_with(|| {
                if crate::game_engines::graphics_import(exe) {
                    crate::game_name_windows::products(exe)
                        .iter()
                        .map(|s| crate::game_engines::normalized(s))
                        .collect::<Vec<_>>()
                } else {
                    vec![]
                }
            });
            if title.len() < 3 || !products.contains(&title) {
                continue;
            }
        }
        if let Some((prior, prior_exact)) = found {
            if prior_exact && !exact {
                continue;
            }
            if exact == prior_exact && core::key(&prior.root) != core::key(&record.root) {
                conflict = true;
                continue;
            }
            if exact == prior_exact && !prior.title.is_empty() {
                continue;
            }
            if exact && !prior_exact {
                conflict = false;
            }
        }
        found = Some((record, exact));
    }
    if conflict { None } else { found }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn installation_boundaries_and_exact_uninstall_target() {
        let root = PathBuf::from("C:/Games/Title");
        let mut record = Install::new(root, "Windows", "中文游戏");
        let mut game = Game {
            exe: "C:/Games/Title/bin/x64/game.exe".into(),
            root: "C:/Games/Title/bin/x64".into(),
            ..Default::default()
        };
        assert!(record.matches(&game));
        game.root = "C:/Games/Title/OtherGame".into();
        game.exe = "C:/Games/Title/OtherGame/game.exe".into();
        assert!(!record.matches(&game));
        game.root = "C:/Games/Title2".into();
        game.exe = "C:/Games/Title2/game.exe".into();
        assert!(!record.matches(&game));
        game.root = "C:/Games/Title".into();
        game.exe = "C:/Games/Title/game.exe".into();
        record.executable = Some(PathBuf::from("C:/Games/Title/launcher.exe"));
        assert!(!record.matches(&game));
    }
    #[test]
    fn epic_skips_dlc_and_keeps_namespace_identity() {
        let t = tempfile::tempdir().unwrap();
        let mut v = json!({"InstallLocation":t.path(),"AppName":"game","MainGameAppName":"game","CatalogNamespace":"sandBox","CatalogItemId":"item","DisplayName":"中文游戏","LaunchExecutable":"Binaries/Win64/game.exe"});
        let i = epic_record(&v).unwrap();
        assert_eq!(i.keys, ["epic:sandbox/item"]);
        assert_eq!(i.title, "中文游戏");
        assert_eq!(i.executable, Some(t.path().join("Binaries/Win64/game.exe")));
        v["MainGameAppName"] = json!("another-game");
        assert!(epic_record(&v).is_none());
        v["MainGameAppName"] = json!("game");
        v["bIsExecutable"] = json!(false);
        assert!(epic_record(&v).is_none());
        v["bIsExecutable"] = json!(true);
        v["LaunchExecutable"] = json!("../outside.exe");
        assert!(epic_record(&v).unwrap().executable.is_none());
    }
    #[test]
    fn ea_localized_install_xml_and_external_entities() {
        let t = tempfile::tempdir().unwrap();
        fs::create_dir(t.path().join("__Installer")).unwrap();
        let p = t.path().join("__Installer/installerdata.xml");
        fs::write(&p,r#"<game><contentIDs><contentID>Origin.OFR.50.000000</contentID></contentIDs><gameTitles><title locale="en_US">Game</title><title locale="zh_CN">中文游戏</title></gameTitles></game>"#).unwrap();
        let i = ea(t.path()).unwrap();
        assert_eq!(i.title, "中文游戏");
        assert_eq!(i.keys, ["ea:origin.ofr.50.000000"]);
        fs::write(
            p,
            r#"<!DOCTYPE game [<!ENTITY x SYSTEM "file:///secret">]><game>&x;</game>"#,
        )
        .unwrap();
        assert!(ea(t.path()).is_none());
    }
    #[test]
    fn battlenet_bounded_wire_and_build_info() {
        fn field(tag: u8, data: &[u8]) -> Vec<u8> {
            assert!(data.len() < 128);
            let mut b = vec![tag, data.len() as u8];
            b.extend(data);
            b
        }
        let t = tempfile::tempdir().unwrap();
        let mut row = field(10, b"diablo3");
        row.extend(field(26, &field(10, t.path().to_str().unwrap().as_bytes())));
        let db = field(10, &row);
        let records = battlenet(&db);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].keys, ["battlenet:diablo3"]);
        for n in 1..db.len() {
            assert!(battlenet(&db[..n]).is_empty());
        }
        assert!(battlenet(&[255; 128]).is_empty());
        fs::write(
            t.path().join(".build.info"),
            "Active!DEC:1|Product!STRING:0\n1|wow\n0|wow_classic\n",
        )
        .unwrap();
        assert_eq!(build_info(t.path()).unwrap().keys, ["battlenet:wow"]);
        fs::write(
            t.path().join(".build.info"),
            "Product!STRING:0\nwow\nwow_classic\n",
        )
        .unwrap();
        assert!(build_info(t.path()).is_none());
    }
    #[test]
    fn hoyo_requires_business_and_matching_game_files() {
        let t = tempfile::tempdir().unwrap();
        let exe = t.path().join("ZenlessZoneZero.exe");
        fs::write(t.path().join("config.ini"), "[General]\ngame_biz=nap_cn\n").unwrap();
        assert!(hoyo(&exe, t.path()).is_none());
        fs::create_dir(t.path().join("ZenlessZoneZero_Data")).unwrap();
        assert_eq!(hoyo(&exe, t.path()).unwrap().keys, ["hoyo:nap_cn"]);
        fs::write(t.path().join("config.ini"), "game_biz=hk4e_cn").unwrap();
        assert!(hoyo(&exe, t.path()).is_none());
        assert_eq!(
            rail_key(Path::new(
                "C:/WeGameApps/rail_apps/DeltaForce(2001918)/DeltaForce/game.exe"
            ))
            .as_deref(),
            Some("wegame:2001918")
        );
        assert!(rail_key(Path::new("C:/Other/DeltaForce(2001918)/game.exe")).is_none());
    }
}
