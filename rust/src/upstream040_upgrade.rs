//! Journaled same-entry version upgrade of one owned dlssg 0.4.x installation.
//! Call mutation helpers only with the deployment directory lock held. Recovery
//! never loads a DLL and never obtains paths from a configuration document.
use crate::{core, presets, upstream040, win};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, path::Path};

pub const SCHEME: &str = upstream040::SCHEME;
const JOURNAL: &str = "upstream040-upgrade.json";
const WORK: &str = "upstream040-upgrade";
const MAX_FILE: u64 = 128 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    schema: u32,
    game_exe: String,
    old: core::Record,
    new: core::Record,
    /// Actual old configuration digest may differ from the installation digest.
    before: BTreeMap<String, String>,
    old_marker_hash: String,
    new_marker_hash: String,
}

pub fn has_pending(directory: &Path) -> Result<bool> {
    Ok(core::no_links(&directory.join(core::OWN).join(JOURNAL))?.exists())
}

fn checked_bytes(path: &Path, maximum: u64) -> Result<Vec<u8>> {
    core::no_links(path)?;
    ensure!(
        path.is_file() && path.metadata()?.len() <= maximum,
        "升级文件类型或大小异常"
    );
    let bytes = fs::read(path)?;
    ensure!(bytes.len() as u64 <= maximum, "升级文件超过大小限制");
    Ok(bytes)
}

fn matches(path: &Path, hash: &str) -> Result<bool> {
    core::no_links(path)?;
    Ok(path.is_file() && path.metadata()?.len() <= MAX_FILE && core::digest(path)? == hash)
}

fn validate_journal(target: &core::DeploymentTarget, j: &Journal) -> Result<()> {
    ensure!(j.schema == 1, "升级恢复记录版本无效");
    ensure!(
        core::key(Path::new(&j.game_exe)) == core::key(&target.game_exe),
        "升级恢复记录属于其他游戏"
    );
    j.old.validate()?;
    j.new.validate()?;
    target.validate_record(&j.old)?;
    target.validate_record(&j.new)?;
    ensure!(
        matches!(
            j.old.backend.as_str(),
            "upstream_sm86" | upstream040::BACKEND
        ) && j.new.backend == upstream040::BACKEND
            && j.old.scheme_id.as_deref() == Some(SCHEME)
            && j.new.scheme_id.as_deref() == Some(SCHEME)
            && !j.old.cache_pending
            && !j.new.cache_pending,
        "升级恢复记录的方案不匹配"
    );
    ensure!(
        if j.old.backend == upstream040::BACKEND {
            j.new.selected() == j.old.selected()
        } else {
            j.new
                .selected()
                .iter()
                .all(|n| j.old.selected().contains(n))
        },
        "dlssg 0.4.x 更新须保留原 DLL 入口；换入口前请先卸载"
    );
    let old_version = j
        .old
        .payload_version
        .as_deref()
        .context("旧版缺少版本记录，请先核对原安装；不能自动升级")?;
    let new_version = j
        .new
        .payload_version
        .as_deref()
        .context("新版缺少版本记录")?;
    ensure!(
        crate::updater::version(new_version)? > crate::updater::version(old_version)?,
        "dlssg 0.4.x 只允许升级到更高版本；已有文件未覆盖"
    );
    ensure!(
        j.new
            .selected()
            .iter()
            .any(|n| j.old.hashes.get(n) != j.new.hashes.get(n)),
        "dlssg 0.4.x DLL 内容未变化"
    );
    ensure!(
        j.old.cleanup_dirs.is_empty()
            && j.old.delta_cache_ids.is_empty()
            && !j.old.delta_legacy_cache
            && j.new.cleanup_dirs.is_empty()
            && j.new.delta_cache_ids.is_empty()
            && !j.new.delta_legacy_cache,
        "升级恢复记录含有不适用的清理归属"
    );
    ensure!(
        j.before.len() == j.old.hashes.len()
            && j.before.keys().eq(j.old.hashes.keys())
            && j.before.values().all(|h| core::valid_hash(h))
            && core::valid_hash(&j.old_marker_hash)
            && j.new_marker_hash == core::hash(&serde_json::to_vec_pretty(&j.new)?),
        "升级恢复文件范围无效"
    );
    for (name, hash) in &j.before {
        ensure!(
            name == core::INI || j.old.hashes.get(name) == Some(hash),
            "原 DLL 与升级归属记录不一致"
        );
    }
    Ok(())
}

fn transaction_files(j: &Journal) -> BTreeMap<String, String> {
    let mut files = BTreeMap::from([("old.install.json".into(), j.old_marker_hash.clone())]);
    files.extend(
        j.before
            .iter()
            .map(|(n, h)| (format!("old.{}", n.replace('/', "__")), h.clone())),
    );
    files.extend(
        j.new
            .hashes
            .iter()
            .map(|(n, h)| (format!("next.{}", n.replace('/', "__")), h.clone())),
    );
    files
}

/// Remove only complete files with the exact journal identity. A partial write
/// or any external modification stays available for inspection and retry.
fn finish(target: &core::DeploymentTarget, j: &Journal, journal_hash: &str) -> Result<()> {
    let root = core::no_links(&target.directory.join(core::OWN))?;
    let work = core::no_links(&root.join(WORK))?;
    let allowed = transaction_files(j);
    if work.exists() {
        ensure!(work.is_dir(), "升级恢复目录已改变");
        let mut found = Vec::new();
        for entry in fs::read_dir(&work)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let hash = allowed
                .get(&name)
                .context("升级恢复目录含有未知文件，已保留")?;
            let path = core::no_links(&entry.path())?;
            ensure!(matches(&path, hash)?, "升级恢复文件已改变，已保留：{name}");
            found.push((path, hash));
        }
        for (path, hash) in found {
            ensure!(matches(&path, hash)?, "升级恢复文件在清理期间发生变化");
            fs::remove_file(path)?;
        }
        fs::remove_dir(&work)?;
    }
    let journal = root.join(JOURNAL);
    ensure!(
        matches(&journal, journal_hash)?,
        "升级恢复记录在清理期间发生变化"
    );
    fs::remove_file(journal)?;
    Ok(())
}

/// Complete a committed transaction, or restore an interrupted one. No operation
/// is performed when no journal exists. The caller holds the directory lock.
pub fn recover_locked(target: &core::DeploymentTarget) -> Result<bool> {
    let root = core::no_links(&target.directory.join(core::OWN))?;
    let journal = core::no_links(&root.join(JOURNAL))?;
    if !journal.exists() {
        return Ok(false);
    }
    core::assert_target_stopped(target)?;
    let raw = checked_bytes(&journal, 64 * 1024)?;
    let j: Journal = serde_json::from_slice(&raw)?;
    validate_journal(target, &j)?;
    let marker = core::no_links(&root.join(core::MARKER))?;
    if matches(&marker, &j.new_marker_hash)? {
        // The marker is the commit point. Never roll back a committed install
        // after a user or game has subsequently changed one of its files.
        for (name, hash) in &j.new.hashes {
            ensure!(
                matches(&target.directory.join(name), hash)?,
                "升级已提交，但文件随后改变；恢复材料已保留：{name}"
            );
        }
        finish(target, &j, &core::hash(&raw))?;
        return Ok(true);
    }
    ensure!(
        matches(&marker, &j.old_marker_hash)?,
        "部署记录在升级期间改变，原件备份已保留"
    );
    let work = core::no_links(&root.join(WORK))?;

    // Validate every destination before removing anything. Unknown files never
    // become ours merely because they occupy a name in the transaction.
    for (name, hash) in &j.new.hashes {
        let path = core::no_links(&target.directory.join(name))?;
        if path.exists() {
            ensure!(
                matches(&path, hash)?
                    || j.before
                        .get(name)
                        .is_some_and(|old| matches(&path, old).unwrap_or(false)),
                "升级目标已被其他文件替换，已保留：{name}"
            );
        }
    }
    for (name, hash) in &j.before {
        let path = core::no_links(&target.directory.join(name))?;
        if !matches(&path, hash)? {
            ensure!(
                !path.exists()
                    || j.new
                        .hashes
                        .get(name)
                        .is_some_and(|new| matches(&path, new).unwrap_or(false)),
                "原文件位置已被其他文件占用：{name}"
            );
            ensure!(
                matches(&work.join(format!("old.{}", name.replace('/', "__"))), hash)?,
                "原件备份缺失或已改变：{name}"
            );
        }
    }
    for (name, hash) in &j.new.hashes {
        let path = core::no_links(&target.directory.join(name))?;
        if path.exists()
            && !j
                .before
                .get(name)
                .is_some_and(|old| matches(&path, old).unwrap_or(false))
        {
            ensure!(matches(&path, hash)?, "升级目标在恢复期间改变：{name}");
            fs::remove_file(path)?;
        }
    }
    for (name, hash) in &j.before {
        let path = core::no_links(&target.directory.join(name))?;
        if !path.exists() {
            let backup = core::no_links(&work.join(format!("old.{}", name.replace('/', "__"))))?;
            ensure!(matches(&backup, hash)?, "原件备份在恢复期间改变：{name}");
            win::rename_no_replace(&backup, &path)?;
        }
        ensure!(matches(&path, hash)?, "原文件恢复核验失败：{name}");
    }
    finish(target, &j, &core::hash(&raw))?;
    Ok(true)
}

/// Upgrade only a valid, same-entry dlssg 0.4.x record. The package configuration
/// is for fresh installs; upgrades keep the latest disk bytes and apply only
/// explicitly edited fields. The caller already holds the directory lock.
pub fn upgrade_locked(
    target: &core::DeploymentTarget,
    old: &core::Record,
    mut new: core::Record,
    mut files: BTreeMap<String, Vec<u8>>,
    explicit_overrides: Option<&presets::Values>,
) -> Result<String> {
    core::assert_target_stopped(target)?;
    old.validate()?;
    new.validate()?;
    target.validate_record(old)?;
    target.validate_record(&new)?;
    ensure!(
        matches!(old.backend.as_str(), "upstream_sm86" | upstream040::BACKEND)
            && new.backend == upstream040::BACKEND
            && old.scheme_id.as_deref() == Some(SCHEME)
            && new.scheme_id.as_deref() == Some(SCHEME),
        "只有本管理器记录的 dlssg 0.4.x 安装可以原位升级"
    );
    let dir = &target.directory;
    let root = core::no_links(&dir.join(core::OWN))?;
    let marker = core::no_links(&root.join(core::MARKER))?;
    let marker_bytes = checked_bytes(&marker, 20000)?;
    ensure!(
        serde_json::from_slice::<core::Record>(&marker_bytes)? == *old,
        "升级前部署记录已变化"
    );
    let mut originals = BTreeMap::new();
    for (name, hash) in &old.hashes {
        let bytes = checked_bytes(
            &dir.join(name),
            if name == core::INI {
                1024 * 1024
            } else {
                MAX_FILE
            },
        )?;
        ensure!(
            name == core::INI || core::hash(&bytes) == *hash,
            "旧补丁 DLL 已变化，请先检查再升级"
        );
        originals.insert(name.clone(), bytes);
    }
    let original_config = originals.get(core::INI).context("旧配置文件缺失")?;
    let updated = upstream040::migrate_versions(
        original_config,
        old.upstream_version
            .as_deref()
            .or(old.payload_version.as_deref()),
        new.upstream_version
            .as_deref()
            .or(new.payload_version.as_deref()),
        explicit_overrides,
    )?;
    ensure!(updated.len() <= 1024 * 1024, "更新后配置超过大小限制");
    files.insert(core::INI.into(), updated);
    ensure!(
        files.iter().all(|(name, bytes)| !bytes.is_empty()
            && bytes.len() as u64
                <= if name.ends_with(".dll") {
                    MAX_FILE
                } else {
                    1024 * 1024
                }),
        "升级文件大小无效"
    );
    new.hashes = files
        .iter()
        .map(|(n, b)| (n.clone(), core::hash(b)))
        .collect();
    let j = Journal {
        schema: 1,
        game_exe: target.game_exe.to_string_lossy().into_owned(),
        old: old.clone(),
        new,
        before: originals
            .iter()
            .map(|(n, b)| (n.clone(), core::hash(b)))
            .collect(),
        old_marker_hash: core::hash(&marker_bytes),
        new_marker_hash: String::new(),
    };
    let j = Journal {
        new_marker_hash: core::hash(&serde_json::to_vec_pretty(&j.new)?),
        ..j
    };
    validate_journal(target, &j)?;
    for name in &core::PROXIES {
        if !old.hashes.contains_key(*name) {
            let path = core::no_links(&dir.join(name))?;
            ensure!(
                !path.is_file() || !crate::cleanup::known_proxy(&path)?,
                "已有其他补丁入口，请先处理冲突"
            );
        }
    }
    for name in files.keys() {
        let path = core::no_links(&dir.join(name))?;
        ensure!(
            !path.exists() || j.before.contains_key(name),
            "已有 {name}，升级不会覆盖其他文件"
        );
    }
    let work = core::no_links(&root.join(WORK))?;
    let journal = core::no_links(&root.join(JOURNAL))?;
    ensure!(
        !work.exists() && !journal.exists(),
        "存在待检查的升级恢复材料"
    );
    let raw = serde_json::to_vec_pretty(&j)?;
    core::write_new(&journal, &raw)?;
    let operation = (|| -> Result<()> {
        checkpoint("journal")?;
        fs::create_dir(&work)?;
        core::write_new(&work.join("old.install.json"), &marker_bytes)?;
        checkpoint("marker-backup")?;
        for (name, bytes) in &originals {
            core::write_new(
                &work.join(format!("old.{}", name.replace('/', "__"))),
                bytes,
            )?;
            checkpoint(&format!("backup:{name}"))?;
        }
        for (name, bytes) in &files {
            core::write_new(
                &work.join(format!("next.{}", name.replace('/', "__"))),
                bytes,
            )?;
            checkpoint(&format!("stage:{name}"))?;
        }
        core::assert_target_stopped(target)?;
        ensure!(
            matches(&marker, &j.old_marker_hash)?,
            "升级前部署记录已变化"
        );
        for (name, hash) in &j.before {
            ensure!(
                matches(&dir.join(name), hash)?,
                "升级前原文件已变化：{name}"
            );
            ensure!(
                matches(&work.join(format!("old.{}", name.replace('/', "__"))), hash)?,
                "原件备份核验失败：{name}"
            );
        }
        for (name, hash) in &j.before {
            if j.new.hashes.get(name) == Some(hash) {
                // The untouched JSON never disappears or gets rewritten.
                continue;
            }
            let path = core::no_links(&dir.join(name))?;
            ensure!(matches(&path, hash)?, "原文件在升级期间改变：{name}");
            fs::remove_file(path)?;
            checkpoint(&format!("remove:{name}"))?;
        }
        // The DLL is exposed last so an accidental launch cannot observe the
        // new runtime with an absent configuration.
        let mut names = files.keys().collect::<Vec<_>>();
        names.sort_by_key(|n| j.new.proxies.contains(n));
        for name in names {
            if j.before.get(name) == j.new.hashes.get(name) {
                continue;
            }
            let stage = core::no_links(&work.join(format!("next.{}", name.replace('/', "__"))))?;
            ensure!(matches(&stage, &j.new.hashes[name])?, "升级暂存文件已变化");
            let destination = core::no_links(&dir.join(name))?;
            if let Some(parent) = destination.parent() {
                core::no_links(parent)?;
                fs::create_dir_all(parent)?;
                core::no_links(parent)?;
            }
            win::rename_no_replace(&stage, &destination)?;
            checkpoint(&format!("publish:{name}"))?;
        }
        core::assert_target_stopped(target)?;
        ensure!(
            matches(&marker, &j.old_marker_hash)?,
            "提交前部署记录已变化"
        );
        for (name, hash) in &j.new.hashes {
            ensure!(
                matches(&dir.join(name), hash)?,
                "提交前升级文件已变化：{name}"
            );
        }
        core::atomic_json(&marker, &j.new)?;
        checkpoint("committed")?;
        finish(target, &j, &core::hash(&raw))?;
        Ok(())
    })();
    if let Err(error) = operation {
        let committed = matches(&marker, &j.new_marker_hash).unwrap_or(false);
        return match recover_locked(target) {
            Ok(_) if committed => Ok("dlssg 0.4.x 升级已提交并完成恢复；已保留原配置".into()),
            Ok(_) => Err(error.context("升级失败，已恢复原 dlssg 0.4.x 文件和配置")),
            Err(recovery) => {
                bail!("升级未完成：{error}；自动恢复未完成，原件备份已保留：{recovery}")
            }
        };
    }
    Ok("dlssg 0.4.x 已升级；已保留原配置，重新启动游戏后生效".into())
}

fn checkpoint(_name: &str) -> Result<()> {
    #[cfg(test)]
    FAULT.with(|fault| -> Result<()> {
        if let Some((name, crash)) = fault.borrow().as_ref()
            && name == _name
        {
            if *crash {
                panic!("simulated interrupted upgrade at {_name}");
            }
            bail!("simulated IO failure at {_name}");
        }
        Ok(())
    })?;
    Ok(())
}

#[cfg(test)]
thread_local! {
    static FAULT: std::cell::RefCell<Option<(String, bool)>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const ORIGINAL: &[u8] = b"; preserve user comment\r\n[FrameGeneration]\r\nOptimized=1\r\n[Compatibility]\r\nPreset=B\r\n[Runtime]\r\nMode=Auto\r\n[Custom]\r\nMine=42\r\n";

    fn pe(dll: bool, identity: u8) -> Vec<u8> {
        let mut bytes = vec![0; 512];
        bytes[..2].copy_from_slice(b"MZ");
        bytes[60..64].copy_from_slice(&128u32.to_le_bytes());
        bytes[128..132].copy_from_slice(b"PE\0\0");
        bytes[132..134].copy_from_slice(&0x8664u16.to_le_bytes());
        bytes[148..150].copy_from_slice(&240u16.to_le_bytes());
        bytes[150..152].copy_from_slice(&(if dll { 0x2000u16 } else { 2 }).to_le_bytes());
        bytes[152..154].copy_from_slice(&0x20bu16.to_le_bytes());
        bytes[511] = identity;
        bytes
    }
    fn context() -> presets::Context {
        presets::Context {
            scheme: SCHEME.into(),
            profile: upstream040::PROFILE.into(),
            delta: false,
            delta_capable: false,
        }
    }
    fn fixture() -> Result<(tempfile::TempDir, core::DeploymentTarget)> {
        let temp = tempfile::tempdir()?;
        let exe = temp.path().join("RTXFG-MFG-Transaction-Test.exe");
        fs::write(&exe, pe(false, 0))?;
        let target = core::DeploymentTarget::for_game(&exe);
        core::deploy_prepared_context_at(
            &target,
            "upstream_sm86",
            &["version.dll".into()],
            None,
            BTreeMap::from([
                ("version.dll".into(), pe(true, 1)),
                (core::INI.into(), ORIGINAL.to_vec()),
            ]),
            Some("0.3.5"),
            Some(&context()),
        )?;
        fs::write(temp.path().join("unrelated-mod.json"), b"preserve")?;
        Ok((temp, target))
    }
    fn install(target: &core::DeploymentTarget, edit: bool) -> Result<String> {
        let values = if edit {
            presets::Values::from([("fg_engine".into(), "XeSS".into())])
        } else {
            presets::Values::new()
        };
        core::deploy_prepared_context_at_with_overrides(
            target,
            upstream040::BACKEND,
            &["version.dll".into()],
            None,
            bundle(),
            Some("0.4.0"),
            Some(&context()),
            Some(&values),
            None,
        )
    }
    fn snapshot(root: &Path) -> Result<BTreeMap<PathBuf, Vec<u8>>> {
        fn visit(root: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) -> Result<()> {
            for entry in fs::read_dir(dir)? {
                let path = entry?.path();
                if path.is_dir() {
                    visit(root, &path, out)?;
                } else {
                    out.insert(path.strip_prefix(root)?.into(), fs::read(path)?);
                }
            }
            Ok(())
        }
        let mut out = BTreeMap::new();
        visit(root, root, &mut out)?;
        Ok(out)
    }
    fn fault(point: &str, crash: bool) {
        FAULT.with(|f| *f.borrow_mut() = Some((point.into(), crash)));
    }
    fn clear_fault() {
        FAULT.with(|f| *f.borrow_mut() = None);
    }
    fn recover(target: &core::DeploymentTarget) -> Result<bool> {
        let _lock = win::game_lock(&target.directory)?;
        recover_locked(target)
    }

    fn bundle() -> BTreeMap<String, Vec<u8>> {
        let mut files = BTreeMap::from([
            ("version.dll".into(), pe(true, 2)),
            (core::INI.into(), b"[General]\nEnabled=1\n".to_vec()),
        ]);
        for name in upstream040::COMPONENTS {
            files.insert(
                name.into(),
                if name.ends_with(".dll") {
                    pe(true, 3)
                } else {
                    b"license text".to_vec()
                },
            );
        }
        files
    }

    fn install041(target: &core::DeploymentTarget) -> Result<String> {
        let mut files = bundle();
        for (name, bytes) in &mut files {
            if name.ends_with(".dll") {
                *bytes = pe(true, 4);
            }
        }
        files.insert(
            core::INI.into(),
            include_bytes!("../../tests/fixtures/catalog041/dlssg_sm86.ini").to_vec(),
        );
        let mut context = context();
        context.profile = upstream040::PROFILE_041.into();
        core::deploy_prepared_context_at_with_overrides(
            target,
            upstream040::BACKEND,
            &["version.dll".into()],
            None,
            files,
            Some("0.4.1"),
            Some(&context),
            None,
            Some("0.4.1"),
        )
    }
    #[test]
    fn upgrade041_is_transactional_and_uninstall_cleans_new_logs_only() -> Result<()> {
        let (_temp, target) = fixture()?;
        install(&target, false)?;
        let ini = target.directory.join(core::INI);
        let original=b"; original\n[DLSSG]\nOptimized=2\nPreset=Auto\n[XeSSFG]\nUseHudless=1\n[Other]\nKeep=custom\n";
        fs::write(&ini, original)?;
        let before = snapshot(&target.directory)?;
        for point in [
            "journal",
            "marker-backup",
            "publish:version.dll",
            "publish:dlssg_sm86/sr/sr_kernels.dll",
        ] {
            for crash in [false, true] {
                fault(point, crash);
                let result =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| install041(&target)));
                clear_fault();
                if crash {
                    assert!(result.is_err());
                    recover(&target)?;
                } else {
                    assert!(result.unwrap().is_err());
                }
                assert_eq!(snapshot(&target.directory)?, before, "{point}");
            }
        }
        install041(&target)?;
        let values = upstream040::read_041(&fs::read(&ini)?)?;
        assert_eq!(values["xess_hudless"], "Auto");
        assert_eq!(values["optimized"], "2");
        assert!(fs::read_to_string(&ini)?.contains("Keep=custom"));
        let logs = target.directory.join("dlssg_sm86/logs");
        fs::create_dir_all(&logs)?;
        for name in [
            "dlssg_sm86.log",
            "frame_timing.txt",
            "frames_123.jsonl",
            "manual-capture.csv",
        ] {
            fs::write(logs.join(name), name)?;
        }
        let outcome = crate::cleanup::clean_outcome_at(&target)?;
        assert!(outcome.complete);
        for name in ["dlssg_sm86.log", "frame_timing.txt", "frames_123.jsonl"] {
            assert!(!logs.join(name).exists());
        }
        assert!(logs.join("manual-capture.csv").exists());
        assert!(target.directory.join("unrelated-mod.json").exists());
        Ok(())
    }
    #[test]
    fn unsupported_sr_upgrade_makes_no_changes() -> Result<()> {
        let (_temp, target) = fixture()?;
        install(&target, false)?;
        fs::write(
            target.directory.join(core::INI),
            b"[SuperResolution]\nMode=On\n",
        )?;
        let before = snapshot(&target.directory)?;
        assert!(install041(&target).unwrap_err().to_string().contains("On"));
        assert_eq!(snapshot(&target.directory)?, before);
        Ok(())
    }

    #[test]
    fn directory_upgrade_rolls_back_each_component_and_preserves_config() -> Result<()> {
        let mut points = vec![
            "journal".to_owned(),
            "marker-backup".into(),
            "remove:version.dll".into(),
            "publish:version.dll".into(),
            "committed".into(),
        ];
        points.extend(bundle().keys().map(|n| format!("publish:{n}")));
        for crash in [false, true] {
            for point in &points {
                let (_temp, target) = fixture()?;
                let before = snapshot(&target.directory)?;
                fault(point, crash);
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    install(&target, true)
                }));
                clear_fault();
                if crash {
                    assert!(result.is_err(), "{point}");
                    assert!(core::status_at(&target).contains("升级未完成"));
                    core::preflight_install_at(&target, &["version.dll".into()])?;
                    assert!(recover(&target)?);
                } else if point == "committed" {
                    assert!(result.unwrap().is_ok());
                } else {
                    assert!(result.unwrap().is_err(), "{point}");
                }
                assert!(!has_pending(&target.directory)?);
                if point == "committed" {
                    assert_eq!(
                        core::record(&target.directory)?.unwrap().backend,
                        upstream040::BACKEND
                    );
                    let config = fs::read_to_string(target.directory.join(core::INI))?;
                    assert!(config.contains("Mine=42") && config.contains("preserve user comment"));
                    for name in upstream040::COMPONENTS {
                        assert!(target.directory.join(name).is_file());
                    }
                } else {
                    assert_eq!(snapshot(&target.directory)?, before, "{point}");
                }
            }
        }
        Ok(())
    }

    #[test]
    fn directory_uninstall_keeps_unknown_and_modified_components() -> Result<()> {
        let (_temp, target) = fixture()?;
        install(&target, false)?;
        let changed = target.directory.join(upstream040::COMPONENTS[0]);
        fs::write(&changed, b"another mod replaced this component")?;
        let unknown = target.directory.join("dlssg_sm86/sr/user-capture.bin");
        fs::write(&unknown, b"manual capture")?;
        crate::cleanup::clean_at(&target)?;
        assert!(changed.exists() && unknown.exists());
        assert!(!target.directory.join("version.dll").exists());
        for name in &upstream040::COMPONENTS[1..] {
            assert!(!target.directory.join(name).exists());
        }
        assert_eq!(
            fs::read(target.directory.join("unrelated-mod.json"))?,
            b"preserve"
        );
        Ok(())
    }
}
