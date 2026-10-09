//! One narrowly scoped, journaled migration from an owned Transfusion install.
//! Call mutation helpers only with the deployment directory lock held. Recovery
//! never loads a DLL and never obtains paths from a configuration document.
use crate::{core, encore, presets, transfusion, win};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, path::Path};

pub const SCHEME: &str = "dlssg-transfusion-1.4.5.3";
const JOURNAL: &str = "encore-upgrade.json";
const WORK: &str = "encore-upgrade";
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
        j.old.backend == transfusion::BACKEND
            && j.new.backend == encore::BACKEND
            && j.old.scheme_id.as_deref() == Some(SCHEME)
            && j.new.scheme_id.as_deref() == Some(SCHEME)
            && !j.old.cache_pending
            && !j.new.cache_pending,
        "升级恢复记录的方案不匹配"
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
            name == transfusion::CONFIG || j.old.hashes.get(name) == Some(hash),
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
            .map(|(n, h)| (format!("old.{n}"), h.clone())),
    );
    files.extend(
        j.new
            .hashes
            .iter()
            .map(|(n, h)| (format!("next.{n}"), h.clone())),
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
                matches(&work.join(format!("old.{name}")), hash)?,
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
            let backup = core::no_links(&work.join(format!("old.{name}")))?;
            ensure!(matches(&backup, hash)?, "原件备份在恢复期间改变：{name}");
            win::rename_no_replace(&backup, &path)?;
        }
        ensure!(matches(&path, hash)?, "原文件恢复核验失败：{name}");
    }
    finish(target, &j, &core::hash(&raw))?;
    Ok(true)
}

/// Upgrade exactly one valid manager-owned Transfusion deployment. The prepared
/// new configuration supplies fresh-install defaults only; migration starts from
/// the user's old document, then applies explicit edits.
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
        old.backend == transfusion::BACKEND
            && new.backend == encore::BACKEND
            && old.scheme_id.as_deref() == Some(SCHEME)
            && new.scheme_id.as_deref() == Some(SCHEME),
        "只有本管理器记录的 Transfusion 安装可以原位升级"
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
            if name == transfusion::CONFIG {
                1024 * 1024
            } else {
                MAX_FILE
            },
        )?;
        ensure!(
            name == transfusion::CONFIG || core::hash(&bytes) == *hash,
            "旧补丁 DLL 已变化，请先检查再升级"
        );
        originals.insert(name.clone(), bytes);
    }
    let empty = presets::Values::new();
    let migrated = encore::migrate(
        originals
            .get(transfusion::CONFIG)
            .context("旧配置文件缺失")?,
        explicit_overrides.unwrap_or(&empty),
    )?;
    ensure!(migrated.len() <= 1024 * 1024, "迁移后配置超过大小限制");
    files.insert(encore::CONFIG.into(), migrated);
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
            core::write_new(&work.join(format!("old.{name}")), bytes)?;
            checkpoint(&format!("backup:{name}"))?;
        }
        for (name, bytes) in &files {
            core::write_new(&work.join(format!("next.{name}")), bytes)?;
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
                matches(&work.join(format!("old.{name}")), hash)?,
                "原件备份核验失败：{name}"
            );
        }
        for (name, hash) in &j.before {
            let path = core::no_links(&dir.join(name))?;
            ensure!(matches(&path, hash)?, "原文件在升级期间改变：{name}");
            fs::remove_file(path)?;
            checkpoint(&format!("remove:{name}"))?;
        }
        // The DLL is exposed last so an accidental launch cannot observe the
        // new runtime with an absent configuration or license notice.
        let mut names = files.keys().collect::<Vec<_>>();
        names.sort_by_key(|n| n.ends_with(".dll"));
        for name in names {
            let stage = core::no_links(&work.join(format!("next.{name}")))?;
            ensure!(matches(&stage, &j.new.hashes[name])?, "升级暂存文件已变化");
            win::rename_no_replace(&stage, &core::no_links(&dir.join(name))?)?;
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
            Ok(_) if committed => Ok("RTX Encore 升级已提交并完成恢复；原配置已迁移".into()),
            Ok(_) => Err(error.context("升级失败，已恢复原 Transfusion 文件和配置")),
            Err(recovery) => {
                bail!("升级未完成：{error}；自动恢复未完成，原件备份已保留：{recovery}")
            }
        };
    }
    Ok("已升级至 RTX Encore；保留并迁移原配置，重新启动游戏后生效".into())
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

    fn pe(dll: bool, identity: u8) -> Vec<u8> {
        let mut b = vec![0; 512];
        b[..2].copy_from_slice(b"MZ");
        b[60..64].copy_from_slice(&128u32.to_le_bytes());
        b[128..132].copy_from_slice(b"PE\0\0");
        b[132..134].copy_from_slice(&0x8664u16.to_le_bytes());
        b[148..150].copy_from_slice(&240u16.to_le_bytes());
        b[150..152].copy_from_slice(&(if dll { 0x2000u16 } else { 2 }).to_le_bytes());
        b[152..154].copy_from_slice(&0x20bu16.to_le_bytes());
        b[511] = identity;
        b
    }
    fn context(profile: &str) -> presets::Context {
        presets::Context {
            scheme: SCHEME.into(),
            profile: profile.into(),
            delta: false,
            delta_capable: false,
        }
    }
    fn fixture() -> Result<(tempfile::TempDir, core::DeploymentTarget)> {
        let temp = tempfile::tempdir()?;
        let exe = temp.path().join("RTXFG-Encore-Recovery-Test.exe");
        fs::write(&exe, pe(false, 0))?;
        let target = core::DeploymentTarget::for_game(&exe);
        core::deploy_prepared_context_at(&target, transfusion::BACKEND, &["version.dll".into()], None,
            BTreeMap::from([
                ("version.dll".into(), pe(true, 1)),
                (transfusion::CONFIG.into(), b"{\n// preserve\n\"configVersion\":3,\"frameGeneration\":{\"mode\":\"fixed\",\"multiplier\":3},\"vendor\":true}".to_vec()),
            ]), Some("1.4.5"), Some(&context(transfusion::PROFILE)))?;
        Ok((temp, target))
    }
    fn install(target: &core::DeploymentTarget) -> Result<String> {
        core::deploy_prepared_context_at_with_overrides(
            target,
            encore::BACKEND,
            &["dinput8.dll".into()],
            None,
            BTreeMap::from([
                ("dinput8.dll".into(), pe(true, 2)),
                (encore::CONFIG.into(), b"{\"configVersion\":4}".to_vec()),
                (encore::NOTICES.into(), b"license fixture".to_vec()),
            ]),
            Some("1.0.0"),
            Some(&context(encore::PROFILE)),
            None,
            Some("1.0.0-beta.2"),
        )
    }
    fn snapshot(root: &Path) -> Result<BTreeMap<PathBuf, Vec<u8>>> {
        fn visit(root: &Path, dir: &Path, map: &mut BTreeMap<PathBuf, Vec<u8>>) -> Result<()> {
            for item in fs::read_dir(dir)? {
                let path = item?.path();
                if path.is_dir() {
                    visit(root, &path, map)?;
                } else {
                    map.insert(path.strip_prefix(root)?.into(), fs::read(path)?);
                }
            }
            Ok(())
        }
        let mut map = BTreeMap::new();
        visit(root, root, &mut map)?;
        Ok(map)
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

    #[test]
    fn failures_and_interruptions_at_every_transaction_boundary_restore_exact_originals()
    -> Result<()> {
        let points = [
            "journal",
            "marker-backup",
            "backup:DLSSG-Transfusion.json",
            "backup:version.dll",
            "stage:dinput8.dll",
            "stage:rtx-encore.jsonc",
            "stage:rtx-encore-THIRD-PARTY-NOTICES.md",
            "remove:DLSSG-Transfusion.json",
            "remove:version.dll",
            "publish:rtx-encore.jsonc",
            "publish:rtx-encore-THIRD-PARTY-NOTICES.md",
            "publish:dinput8.dll",
            "committed",
        ];
        for crash in [false, true] {
            for point in points {
                let (_temp, target) = fixture()?;
                let before = snapshot(&target.directory)?;
                fault(point, crash);
                let result =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| install(&target)));
                clear_fault();
                if crash {
                    assert!(result.is_err(), "{point}");
                    assert!(has_pending(&target.directory)?);
                    assert!(core::status_at(&target).contains("升级未完成"));
                    assert!(recover(&target)?, "{point}");
                } else if point == "committed" {
                    assert!(result.unwrap().is_ok());
                } else {
                    assert!(result.unwrap().is_err(), "{point}");
                }
                assert!(!has_pending(&target.directory)?);
                assert!(!recover(&target)?);
                if point == "committed" {
                    assert_eq!(
                        core::record(&target.directory)?.unwrap().backend,
                        encore::BACKEND
                    );
                    assert_eq!(fs::read(target.directory.join("dinput8.dll"))?, pe(true, 2));
                    assert!(!target.directory.join("version.dll").exists());
                } else {
                    assert_eq!(
                        snapshot(&target.directory)?,
                        before,
                        "crash={crash}, point={point}"
                    );
                }
            }
        }
        Ok(())
    }

    #[test]
    fn recovery_keeps_a_foreign_replacement_and_all_original_backups() -> Result<()> {
        let (_temp, target) = fixture()?;
        let before = snapshot(&target.directory)?;
        fault("publish:dinput8.dll", true);
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| install(&target))).is_err()
        );
        clear_fault();
        let replacement = target.directory.join("dinput8.dll");
        fs::write(&replacement, b"unrelated replacement")?;
        let interrupted = snapshot(&target.directory)?;
        assert!(recover(&target).is_err());
        assert_eq!(snapshot(&target.directory)?, interrupted);
        assert!(
            target
                .directory
                .join(core::OWN)
                .join(WORK)
                .join("old.version.dll")
                .is_file()
        );
        // Once the exact staged runtime is restored, recovery is idempotent.
        fs::write(&replacement, pe(true, 2))?;
        assert!(recover(&target)?);
        assert_eq!(snapshot(&target.directory)?, before);
        Ok(())
    }

    #[test]
    fn tampered_backup_or_journal_does_not_authorize_restore_or_path_escape() -> Result<()> {
        let (_temp, target) = fixture()?;
        fault("remove:version.dll", true);
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| install(&target))).is_err()
        );
        clear_fault();
        let backup = target
            .directory
            .join(core::OWN)
            .join(WORK)
            .join("old.version.dll");
        fs::write(&backup, b"changed backup")?;
        let before = snapshot(&target.directory)?;
        assert!(recover(&target).is_err());
        assert_eq!(snapshot(&target.directory)?, before);
        fs::write(&backup, pe(true, 1))?;
        let journal = target.directory.join(core::OWN).join(JOURNAL);
        let original = fs::read(&journal)?;
        let mut data: serde_json::Value = serde_json::from_slice(&original)?;
        data["before"]["../outside.dll"] = serde_json::json!(core::hash(b"outside"));
        fs::write(&journal, serde_json::to_vec(&data)?)?;
        let before = snapshot(&target.directory)?;
        assert!(recover(&target).is_err());
        assert_eq!(snapshot(&target.directory)?, before);
        fs::write(&journal, original)?;
        assert!(recover(&target)?);
        Ok(())
    }

    #[test]
    fn committed_transaction_never_rolls_back_files_changed_after_commit() -> Result<()> {
        let (_temp, target) = fixture()?;
        fault("committed", true);
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| install(&target))).is_err()
        );
        clear_fault();
        let config = target.directory.join(encore::CONFIG);
        fs::write(&config, b"user changed after commit")?;
        let before = snapshot(&target.directory)?;
        assert!(recover(&target).is_err());
        assert_eq!(snapshot(&target.directory)?, before);
        assert_eq!(
            core::record(&target.directory)?.unwrap().backend,
            encore::BACKEND
        );
        Ok(())
    }
}
