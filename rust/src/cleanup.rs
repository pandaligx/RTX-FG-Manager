//! Deletion ownership only. No hash catalog is consulted to permit deployment.
use crate::{
    assets,
    core::{self, INI, MARKER, OWN, PROXIES, Record},
    win,
};
use anyhow::{Context, Result, bail, ensure};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
    sync::OnceLock,
};
pub const HELPER: &str = "rtxfg_vk_bridge.dll";
pub const VK_LOGS: [&str; 2] = ["rtxfg-vulkan-bridge.log", "rtxfg-vulkan-diag.jsonl"];
const DEFAULT_DIRS: [(&str, &str); 8] = [
    (".rtx-fg-v3/cache", "cache"),
    (".rtx-fg-v3/logs", "logs"),
    ("dlssg_sm75_cache_r2", "cache"),
    ("dlssg_sm75_logs_r2", "logs"),
    ("dlssg_sm75_cache", "cache"),
    ("dlssg_sm75_logs", "logs"),
    ("dlssg_sm86/logs", "logs"),
    ("dlssg_sm86/cache", "cache"),
];
fn catalog() -> &'static Value {
    static DATA: OnceLock<Value> = OnceLock::new();
    DATA.get_or_init(|| {
        serde_json::from_str(include_str!("../../app/assets/cleanup-catalog.json")).unwrap()
    })
}
fn listed(group: &str, hash: &str) -> bool {
    catalog()[group]
        .as_array()
        .is_some_and(|a| a.iter().any(|v| v.as_str() == Some(hash)))
}
pub fn image_digest(data: &[u8]) -> Option<String> {
    fn word(b: &[u8], i: usize) -> Option<usize> {
        Some(u16::from_le_bytes(b.get(i..i + 2)?.try_into().ok()?) as usize)
    }
    fn dword(b: &[u8], i: usize) -> Option<usize> {
        Some(u32::from_le_bytes(b.get(i..i + 4)?.try_into().ok()?) as usize)
    }
    if data.get(..2)? != b"MZ" {
        return None;
    }
    let pe = dword(data, 60)?;
    let opt = pe.checked_add(24)?;
    if data.get(pe..pe + 4)? != b"PE\0\0"
        || word(data, pe + 4)? != 0x8664
        || word(data, pe + 22)? & 0x2000 == 0
        || word(data, opt)? != 0x20b
    {
        return None;
    }
    let size = word(data, pe + 20)?;
    if size < 152 || opt.checked_add(size)? > data.len() {
        return None;
    }
    let cert = opt + 144;
    let start = dword(data, cert)?;
    let length = dword(data, cert + 4)?;
    let sections = word(data, pe + 6)?;
    let mut end = opt
        .checked_add(size)?
        .checked_add(40usize.checked_mul(sections)?)?;
    for i in 0..sections {
        let offset = opt + size + 40 * i;
        end = end.max(dword(data, offset + 20)?.checked_add(dword(data, offset + 16)?)?);
    }
    if end > data.len()
        || ((start != 0 || length != 0)
            && (start < end
                || start % 8 != 0
                || length < 8
                || start.checked_add(length)? > data.len()))
    {
        return None;
    }
    let mut bytes = data.to_vec();
    bytes.get_mut(opt + 64..opt + 68)?.fill(0);
    bytes.get_mut(cert..cert + 8)?.fill(0);
    if length > 0 {
        bytes.drain(start..start + length);
    }
    Some(core::hash(&bytes))
}
pub fn identity(path: &Path) -> Result<(String, Option<String>)> {
    core::no_links(path)?;
    ensure!(
        path.is_file() && path.metadata()?.len() <= 128 * 1024 * 1024,
        "文件类型或大小异常"
    );
    let mut data = Vec::new();
    fs::File::open(path)?
        .take(128 * 1024 * 1024 + 1)
        .read_to_end(&mut data)?;
    ensure!(data.len() <= 128 * 1024 * 1024, "文件类型或大小异常");
    Ok((core::hash(&data), image_digest(&data)))
}
pub fn known_proxy(path: &Path) -> Result<bool> {
    if !path.is_file() || path.metadata()?.len() > 128 * 1024 * 1024 {
        return Ok(false);
    }
    let (a, b) = identity(path)?;
    Ok(core::LEGACY.iter().any(|(h, _)| *h == a)
        || listed("proxy_sha256", &a)
        || b.as_ref().is_some_and(|h| listed("proxy_image_sha256", h))
        || assets::EMBEDDED
            .iter()
            .any(|r| r.name.ends_with(".dll") && r.sha256 == a)
        || b.is_some_and(|h| {
            bundled_images().contains(&h)
                || crate::cloud::known_image(&h)
                || crate::delta::known_image(&h)
        }))
}
fn bundled_images() -> &'static BTreeSet<String> {
    static IMAGES: OnceLock<BTreeSet<String>> = OnceLock::new();
    IMAGES.get_or_init(|| {
        assets::EMBEDDED
            .iter()
            .filter(|r| r.name.ends_with(".dll"))
            .filter_map(|r| {
                assets::bytes(r.name)
                    .ok()
                    .and_then(|bytes| image_digest(&bytes))
            })
            .collect()
    })
}
fn known_helper(path: &Path) -> Result<bool> {
    if !path.is_file() || path.metadata()?.len() > 128 * 1024 * 1024 {
        return Ok(false);
    }
    let (_, b) = identity(path)?;
    Ok(b.is_some_and(|h| listed("vulkan_helper_image_sha256", &h)))
}
pub fn parse_ini(text: &str) -> BTreeMap<String, BTreeMap<String, String>> {
    let mut result: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    let mut section = String::new();
    for line in text.trim_start_matches('\u{feff}').lines() {
        let line = line.trim();
        if line.starts_with([';', '#']) || line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            section = line[1..line.len() - 1].to_string();
            result.entry(section.clone()).or_default();
        } else if !section.is_empty()
            && let Some((k, v)) = line.split_once('=').or_else(|| line.split_once(':'))
        {
            result
                .entry(section.clone())
                .or_default()
                .insert(k.trim().into(), v.trim().into());
        }
    }
    result
}
pub fn relative_dir(directory: &Path, value: &str) -> Result<Option<PathBuf>> {
    let value = value.trim();
    if value.is_empty() || value.contains('%') || value.starts_with("\\\\") {
        return Ok(None);
    }
    let p = Path::new(value);
    if p.components().any(|c| c == Component::ParentDir) {
        return Ok(None);
    }
    let target = if p.is_absolute() {
        if value.get(2..).unwrap_or_default().contains(':') || !core::within(p, directory) {
            return Ok(None);
        }
        p.to_path_buf()
    } else {
        if p.has_root() || value.contains(':') || p.components().count() > 20 {
            return Ok(None);
        }
        directory.join(p)
    };
    for q in target.ancestors() {
        if core::key(q) == core::key(directory) {
            break;
        }
        if q.join(OWN).join(MARKER).exists() {
            return Ok(None);
        }
    }
    Ok(Some(core::no_links(&target)?))
}
fn merge(roots: &mut BTreeMap<PathBuf, String>, p: PathBuf, kind: &str) {
    let v = roots.entry(p).or_insert_with(|| kind.into());
    if v != kind {
        *v = "both".into();
    }
}
fn generated_dirs(dir: &Path, ini: &Path) -> Result<BTreeMap<PathBuf, String>> {
    let mut roots: BTreeMap<_, _> = DEFAULT_DIRS
        .iter()
        .map(|(p, k)| (dir.join(p), (*k).to_owned()))
        .collect();
    if ini.is_file()
        && ini.metadata()?.len() <= 1024 * 1024
        && let Ok(bytes) = fs::read(ini)
        && let Ok((s, _)) = crate::diagnostics::decode_ini(&bytes)
    {
        let c = parse_ini(&s);
        for (section, key, kind) in [
            ("Logging", "Directory", "logs"),
            ("Runtime", "CacheDirectory", "cache"),
        ] {
            if let Some(value) = c
                .iter()
                .find(|(s, _)| s.eq_ignore_ascii_case(section))
                .map(|(_, values)| values)
                .and_then(|s| s.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)))
                .map(|(_, v)| v)
                && let Some(p) = relative_dir(dir, value)?
            {
                merge(&mut roots, p, kind);
            }
        }
    }
    Ok(roots)
}
fn legacy_identity(journal: &Path) -> Result<Option<String>> {
    core::no_links(journal)?;
    if !journal.exists() {
        return Ok(None);
    }
    let state = core::read_json(journal, 500)?;
    let dll = state["dll"].as_str().unwrap_or_default();
    ensure!(
        state["schema"] == 1 && core::LEGACY.iter().any(|(h, _)| *h == dll),
        "旧版清理记录不匹配"
    );
    Ok(Some(dll.into()))
}
fn manual_record(exe: &Path, legacy: Option<String>) -> Result<Option<Record>> {
    let dir = exe.parent().context("无效游戏路径")?;
    let mut hashes = BTreeMap::new();
    let mut delta_runtime = false;
    for name in PROXIES {
        let p = core::no_links(&dir.join(name))?;
        if p.is_file() && known_proxy(&p)? {
            hashes.insert(name.into(), core::digest(&p)?);
            delta_runtime |= crate::delta::is_game(exe)
                && identity(&p)?
                    .1
                    .is_some_and(|h| crate::delta::known_image(&h));
        }
    }
    let helper = core::no_links(&dir.join(HELPER))?;
    if hashes.is_empty() && legacy.is_none() && !(helper.is_file() && known_helper(&helper)?) {
        return Ok(None);
    }
    if hashes.is_empty() {
        hashes.insert(
            "version.dll".into(),
            legacy.unwrap_or_else(|| core::hash(b"")),
        );
    }
    let selected = core::normalize_proxies(&hashes.keys().cloned().collect::<Vec<_>>())?;
    let ini = core::no_links(&dir.join(INI))?;
    hashes.insert(
        INI.into(),
        if ini.is_file() {
            core::digest(&ini)?
        } else {
            core::hash(b"")
        },
    );
    Ok(Some(Record {
        schema: 3,
        backend: "manual".into(),
        payload_version: None,
        proxy: selected[0].clone(),
        proxies: selected,
        hashes,
        cleanup_dirs: BTreeMap::new(),
        scheme_id: None,
        delta_cache_ids: if delta_runtime {
            vec![crate::delta::cache_id(exe)]
        } else {
            Vec::new()
        },
        delta_legacy_cache: delta_runtime,
        cache_pending: false,
    }))
}
fn is_log(name: &str) -> bool {
    ["loader_", "backend_", "native_"].iter().any(|p| {
        name.strip_prefix(p)
            .and_then(|n| n.strip_suffix(".jsonl"))
            .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
    })
}
fn cache_owned(path: &Path, name: &str) -> Result<bool> {
    if !path.is_file() || path.metadata()?.len() > 128 * 1024 * 1024 {
        return Ok(false);
    }
    let data: Value = serde_json::from_str(include_str!("../assets/cache-hashes.json"))?;
    let (a, b) = identity(path)?;
    Ok(data[name]
        .as_array()
        .is_some_and(|v| v.iter().any(|x| x.as_str() == Some(&a)))
        || b.is_some_and(|h| listed("cache_image_sha256", &h)))
}
fn scan_generated(
    root: &Path,
    kind: &str,
    kept: &mut BTreeSet<PathBuf>,
    targets: &mut BTreeMap<PathBuf, String>,
    dirs: &mut BTreeSet<PathBuf>,
) -> Result<()> {
    let root = core::no_links(root)?;
    if !root.exists() {
        return Ok(());
    }
    if !root.is_dir() {
        kept.insert(root);
        return Ok(());
    }
    let mut stack = vec![(root.clone(), 0)];
    while let Some((d, depth)) = stack.pop() {
        core::no_links(&d)?;
        for e in fs::read_dir(&d)? {
            let e = e?;
            let q = core::no_links(&e.path())?;
            let n = e.file_name().to_string_lossy().into_owned();
            if q.is_dir() {
                if depth == 0 && (kind == "cache" || kind == "both") && core::valid_hash(&n) {
                    stack.push((q, 1));
                } else {
                    kept.insert(q);
                }
                continue;
            }
            let owned = q.is_file()
                && (((kind == "logs" || kind == "both") && depth == 0 && is_log(&n))
                    || ((kind == "cache" || kind == "both")
                        && depth == 1
                        && ["nvngx_dlssg.dll", "sm75_backend.dll", "sm86_backend.dll"]
                            .contains(&n.as_str())
                        && cache_owned(&q, &n)?));
            if owned {
                targets.insert(q.clone(), core::digest(&q)?);
            } else {
                kept.insert(q);
            }
        }
        dirs.insert(d);
    }
    Ok(())
}
pub fn clean(exe: &Path) -> Result<String> {
    let p = core::location(exe, false)?;
    let dir = p.parent().context("无效路径")?;
    let _lock = win::game_lock(dir)?;
    core::assert_stopped(&p)?;
    let root = core::no_links(&dir.join(OWN))?;
    let journal = core::no_links(&dir.join(".rtx-fg-v3-legacy.json"))?;
    let legacy = legacy_identity(&journal)?;
    let journal_hash = if journal.is_file() {
        Some(core::digest(&journal)?)
    } else {
        None
    };
    ensure!(!root.exists() || root.is_dir(), "部署目录不是普通目录");
    let existing = core::record(dir)?;
    let created = existing.is_none();
    let mut record = match if existing.is_some() {
        existing
    } else {
        manual_record(&p, legacy)?
    } {
        Some(r) => r,
        None => return Ok("未发现可确认归属的补丁；游戏文件、其他 MOD 与未知文件已保留".into()),
    };
    let mut kept = BTreeSet::new();
    let mut targets = BTreeMap::new();
    let mut dirs = BTreeSet::new();
    let ini = core::no_links(&dir.join(INI))?;
    let mut roots = generated_dirs(dir, &ini)?;
    ensure!(record.cleanup_dirs.len() <= 64, "清理目录记录无效");
    for (name, kind) in &record.cleanup_dirs {
        ensure!(
            ["logs", "cache", "both"].contains(&kind.as_str()),
            "清理目录记录无效"
        );
        let p = relative_dir(dir, name)?.context("清理目录记录超出游戏范围")?;
        merge(&mut roots, p, kind);
    }
    for name in PROXIES {
        let q = core::no_links(&dir.join(name))?;
        if !q.exists() {
            continue;
        }
        if q.is_file() {
            let h = core::digest(&q)?;
            if record.hashes.get(name) == Some(&h) || known_proxy(&q)? {
                targets.insert(q, h);
                continue;
            }
        }
        kept.insert(q);
    }
    let helper = core::no_links(&dir.join(HELPER))?;
    if helper.exists() {
        if helper.is_file() && known_helper(&helper)? {
            targets.insert(helper.clone(), core::digest(&helper)?);
        } else {
            kept.insert(helper);
        }
    }
    for name in VK_LOGS.into_iter().chain([INI]) {
        let q = core::no_links(&dir.join(name))?;
        if q.exists() {
            if q.is_file() {
                targets.insert(q.clone(), core::digest(&q)?);
            } else {
                kept.insert(q);
            }
        }
    }
    let mut allowed: BTreeSet<String> = [MARKER, "cache", "logs"]
        .map(str::to_owned)
        .into_iter()
        .collect();
    for name in core::deployment_names(&record.backend, &record.selected())? {
        let n = format!("{name}.stage");
        allowed.insert(n.clone());
        let stage = core::no_links(&root.join(n))?;
        if !stage.exists() {
            continue;
        }
        ensure!(stage.is_file(), "临时文件不是普通文件");
        let h = core::digest(&stage)?;
        if record.hashes.get(&name) != Some(&h) {
            let package = core::package(&record.backend, &record.selected())?;
            let bytes = &package[&name];
            ensure!(
                record.hashes.get(&name) == Some(&core::hash(bytes))
                    && bytes.starts_with(&fs::read(&stage)?),
                "临时文件不完整且安装源已变化，请恢复原安装源后重试"
            );
        }
        targets.insert(stage, h);
    }
    let config_stage = core::no_links(&root.join(format!("{INI}.config-stage")))?;
    allowed.insert(format!("{INI}.config-stage"));
    if config_stage.is_file() && config_stage.metadata()?.len() <= 1024 * 1024 {
        // The marker and existing owned INI identify our interrupted config write.
        if ini.is_file() {
            targets.insert(config_stage.clone(), core::digest(&config_stage)?);
        } else {
            kept.insert(config_stage);
        }
    }
    if root.is_dir() {
        for e in fs::read_dir(&root)? {
            let e = e?;
            if !allowed.contains(&e.file_name().to_string_lossy().into_owned()) {
                kept.insert(e.path());
            }
        }
    }
    for (folder, kind) in &roots {
        scan_generated(folder, kind, &mut kept, &mut targets, &mut dirs)?;
    }
    for name in [".rtx-fg-script.json", ".rtx-fg-manager.json"] {
        let q = core::no_links(&dir.join(name))?;
        if !q.exists() {
            continue;
        }
        let matched = (|| -> Result<bool> {
            let state = core::read_json(&q, 20000)?;
            let values = state
                .get("Files")
                .or_else(|| state.get("files"))
                .context("无效旧版记录")?;
            if *values == serde_json::to_value(&record.hashes)? {
                return Ok(true);
            }
            let Some(m) = values.as_object() else {
                return Ok(false);
            };
            if m.len() != 2
                || !m
                    .get(INI)
                    .and_then(Value::as_str)
                    .is_some_and(core::valid_hash)
            {
                return Ok(false);
            }
            let Some(h) = m.get("version.dll").and_then(Value::as_str) else {
                return Ok(false);
            };
            Ok(core::valid_hash(h)
                && (listed("proxy_sha256", h)
                    || core::LEGACY.iter().any(|(a, _)| *a == h)
                    || record.hashes.get("version.dll").is_some_and(|a| a == h)))
        })();
        if matched.unwrap_or(false) {
            targets.insert(q.clone(), core::digest(&q)?);
        } else {
            kept.insert(q);
        }
    }
    record.cleanup_dirs = roots
        .iter()
        .map(|(p, k)| {
            Ok((
                p.strip_prefix(dir)?.to_string_lossy().into_owned(),
                k.clone(),
            ))
        })
        .collect::<Result<_>>()?;
    if created {
        fs::create_dir_all(&root)?;
        core::write_new(&root.join(MARKER), &serde_json::to_vec(&record)?)?;
    } else {
        core::atomic_json(&root.join(MARKER), &record)?;
    }
    let mut ordered = targets.keys().cloned().collect::<Vec<_>>();
    ordered.sort_by_key(|q| {
        (
            !(q.parent() == Some(dir)
                && q.file_name()
                    .is_some_and(|n| PROXIES.iter().any(|p| n == *p))),
            q.clone(),
        )
    });
    for q in ordered {
        core::no_links(&q)?;
        ensure!(
            core::digest(&q)? == targets[&q],
            "清理期间文件发生变化，请重试：{}",
            q.display()
        );
        fs::remove_file(&q)
            .with_context(|| format!("卸载未完成，恢复记录已保留：{}", q.display()))?;
    }
    dirs.insert(dir.join("dlssg_sm86"));
    let mut dirs = dirs.into_iter().collect::<Vec<_>>();
    dirs.sort_by_key(|p| std::cmp::Reverse(p.components().count()));
    for d in dirs {
        core::no_links(&d)?;
        if d != dir && d.is_dir() && fs::read_dir(&d)?.next().is_none() {
            fs::remove_dir(d)?;
        }
    }
    if journal.exists() {
        ensure!(
            Some(core::digest(&journal)?) == journal_hash,
            "清理期间旧版记录发生变化，请重试"
        );
        fs::remove_file(journal)?;
    }
    let cache = crate::delta::clean_game(&p, &record.delta_cache_ids, record.delta_legacy_cache);
    let pending = match cache {
        Ok(report) => report.pending,
        Err(e) => vec![e.to_string()],
    };
    if !pending.is_empty() {
        record.cache_pending = true;
        core::atomic_json(&root.join(MARKER), &record)?;
        return Ok(format!("补丁已移除，缓存待清理：{}", pending.join("；")));
    }
    let marker = core::no_links(&root.join(MARKER))?;
    let bytes = fs::read(&marker)?;
    fs::remove_file(&marker)?;
    if fs::read_dir(&root)?.next().is_none()
        && let Err(e) = fs::remove_dir(&root)
    {
        let _ = core::write_new(&marker, &bytes);
        bail!(e)
    }
    kept.retain(|q| q.exists());
    if kept.is_empty() {
        Ok("已清理新旧版补丁 DLL、INI、专属缓存、日志与部署记录".into())
    } else {
        Ok(format!(
            "已清理可确认归属的补丁、INI、缓存和日志；保留其他或未知文件：{}",
            kept.iter()
                .filter_map(|p| p.strip_prefix(dir).ok())
                .map(|p| p.to_string_lossy())
                .collect::<Vec<_>>()
                .join("、")
        ))
    }
}
