//! Update an existing OptiScaler 10 host only when its MFG plugin is selected.
//! The original host is retained until uninstall, independently of plugin files.
use crate::core;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

pub const PAYLOAD: &str = "OptiScaler.dll";
pub const JOURNAL: &str = "mfg-optiscaler-host.json";
pub const BACKUP: &str = "mfg-optiscaler-host.original";
const LIMIT: u64 = 64 * 1024 * 1024;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    schema: u8,
    game_exe: String,
    name: String,
    before: String,
    applied: String,
}

fn valid_name(name: &str) -> bool {
    crate::mfg_plugin::host_names().any(|candidate| candidate == name)
}

pub fn host(target: &core::DeploymentTarget) -> Result<PathBuf> {
    crate::mfg_plugin::host_file(target)
}

pub fn preflight(target: &core::DeploymentTarget, bytes: &[u8], upgrading: bool) -> Result<()> {
    ensure!(
        !bytes.is_empty() && bytes.len() as u64 <= LIMIT,
        "OptiScaler 配套文件大小无效"
    );
    let path = host(target)?;
    ensure!(path.metadata()?.len() <= LIMIT, "OptiScaler 原文件过大");
    if upgrading {
        plan_upgrade(target, &path, bytes)?;
    }
    Ok(())
}

/// The enclosing MFG transaction owns the commit point. Keep the pre-install
/// host backup independent from the copy used to roll back this one upgrade.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Upgrade {
    previous: Option<Journal>,
    next: Journal,
    pub(crate) before: String,
}

pub(crate) fn plan_upgrade(
    target: &core::DeploymentTarget,
    path: &Path,
    bytes: &[u8],
) -> Result<Option<Upgrade>> {
    let root = core::no_links(&target.directory.join(core::OWN))?;
    let receipt = core::no_links(&root.join(JOURNAL))?;
    let backup = core::no_links(&root.join(BACKUP))?;
    let before = core::digest(path)?;
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .context("主文件名无效")?
        .to_owned();
    let previous: Option<Journal> = if receipt.try_exists()? {
        let old: Journal = serde_json::from_value(core::read_json(&receipt, 16384)?)?;
        ensure!(
            old.schema == 1
                && old.game_exe == target.game_exe.to_string_lossy()
                && old.name == name
                && old.applied == before
                && backup.metadata()?.len() <= LIMIT
                && core::digest(&backup)? == old.before,
            "OptiScaler 主文件或原件备份已变化，请先检查"
        );
        Some(old)
    } else {
        ensure!(!backup.try_exists()?, "发现待恢复的 OptiScaler 备份");
        None
    };
    let after = core::hash(bytes);
    if before == after {
        return Ok(None);
    }
    let next = Journal {
        schema: 1,
        game_exe: target.game_exe.to_string_lossy().into_owned(),
        name,
        before: previous
            .as_ref()
            .map_or_else(|| before.clone(), |old| old.before.clone()),
        applied: after,
    };
    let plan = Upgrade {
        previous,
        next,
        before,
    };
    plan.validate(target)?;
    Ok(Some(plan))
}

impl Upgrade {
    pub(crate) fn validate(&self, target: &core::DeploymentTarget) -> Result<()> {
        let n = &self.next;
        ensure!(
            n.schema == 1
                && n.game_exe == target.game_exe.to_string_lossy()
                && valid_name(&n.name)
                && core::valid_hash(&n.applied)
                && core::valid_hash(&n.before)
                && core::valid_hash(&self.before)
                && self.before != n.applied
                && self
                    .previous
                    .as_ref()
                    .map_or(self.before == n.before, |old| old.schema == 1
                        && old.game_exe == n.game_exe
                        && old.name == n.name
                        && old.before == n.before
                        && old.applied == self.before),
            "OptiScaler 升级记录无效"
        );
        Ok(())
    }
    pub(crate) fn path(&self, target: &core::DeploymentTarget) -> Result<PathBuf> {
        self.validate(target)?;
        core::no_links(&target.directory.join(&self.next.name))
    }
    fn state(&self, root: &Path) -> Result<Option<Journal>> {
        let path = core::no_links(&root.join(JOURNAL))?;
        let current = if path.try_exists()? {
            Some(serde_json::from_value(core::read_json(&path, 16384)?)?)
        } else {
            None
        };
        ensure!(
            current == self.previous || current.as_ref() == Some(&self.next),
            "OptiScaler 恢复记录已变化"
        );
        Ok(current)
    }
    pub(crate) fn apply(&self, target: &core::DeploymentTarget, bytes: &[u8]) -> Result<()> {
        let path = self.path(target)?;
        let root = core::no_links(&target.directory.join(core::OWN))?;
        ensure!(
            core::hash(bytes) == self.next.applied
                && core::digest(&path)? == self.before
                && self.state(&root)? == self.previous,
            "OptiScaler 升级前文件已改变"
        );
        let backup = core::no_links(&root.join(BACKUP))?;
        if self.previous.is_none() {
            core::write_new(&backup, &fs::read(&path)?)?;
        }
        ensure!(
            core::digest(&backup)? == self.next.before,
            "OptiScaler 原件备份不匹配"
        );
        core::atomic_json(&root.join(JOURNAL), &self.next)?;
        core::assert_target_stopped(target)?;
        replace(&path, &self.before, bytes)
    }
    pub(crate) fn recover(
        &self,
        target: &core::DeploymentTarget,
        work: &Path,
        committed: bool,
    ) -> Result<()> {
        let path = self.path(target)?;
        let root = core::no_links(&target.directory.join(core::OWN))?;
        let backup = core::no_links(&root.join(BACKUP))?;
        let current = core::digest(&path)?;
        let receipt = self.state(&root)?;
        let has_backup = backup.try_exists()?;
        if self.previous.is_some() || has_backup || committed {
            ensure!(
                has_backup
                    && backup.metadata()?.len() <= LIMIT
                    && core::digest(&backup)? == self.next.before,
                "OptiScaler 原件备份已变化，已保留"
            );
        }
        if committed {
            ensure!(
                current == self.next.applied && receipt.as_ref() == Some(&self.next),
                "升级后 OptiScaler 已变化，恢复材料已保留"
            );
            return Ok(());
        }
        if current != self.before {
            ensure!(
                current == self.next.applied,
                "OptiScaler 主文件已被其他程序修改"
            );
            let prior = core::no_links(&work.join("old.host"))?;
            ensure!(
                prior.metadata()?.len() <= LIMIT && core::digest(&prior)? == self.before,
                "OptiScaler 升级前的文件副本已变化"
            );
            replace(&path, &current, &fs::read(prior)?)?;
        }
        if receipt != self.previous {
            if let Some(old) = &self.previous {
                core::atomic_json(&root.join(JOURNAL), old)?;
            } else {
                fs::remove_file(core::no_links(&root.join(JOURNAL))?)?;
            }
        }
        if self.previous.is_none() && has_backup {
            fs::remove_file(backup)?;
        }
        Ok(())
    }
}

fn replace(path: &Path, before: &str, bytes: &[u8]) -> Result<()> {
    let mut temporary = tempfile::NamedTempFile::new_in(path.parent().context("主文件目录无效")?)?;
    temporary.write_all(bytes)?;
    temporary.as_file().sync_all()?;
    ensure!(
        core::digest(path)? == before,
        "OptiScaler 文件已变化，未覆盖"
    );
    temporary.persist(path)?;
    Ok(())
}

/// Caller holds the deployment lock, has checked the game is stopped and saved its marker.
pub fn install_locked(target: &core::DeploymentTarget, bytes: &[u8]) -> Result<()> {
    preflight(target, bytes, false)?;
    let path = host(target)?;
    let before = core::digest(&path)?;
    let applied = core::hash(bytes);
    if before == applied {
        return Ok(());
    }
    let root = core::no_links(&target.directory.join(core::OWN))?;
    let journal = root.join(JOURNAL);
    let backup = root.join(BACKUP);
    ensure!(
        !journal.exists() && !backup.exists(),
        "存在待恢复的 OptiScaler 主文件记录，请先卸载 MFG 插件"
    );
    let j = Journal {
        schema: 1,
        game_exe: target.game_exe.to_string_lossy().into_owned(),
        name: path
            .file_name()
            .context("主文件名无效")?
            .to_string_lossy()
            .into_owned(),
        before,
        applied,
    };
    // Save the journal first: interruption before the backup cannot leave an unowned file.
    core::write_new(&journal, &serde_json::to_vec(&j)?)?;
    core::write_new(&backup, &fs::read(&path)?)?;
    ensure!(
        core::digest(&backup)? == j.before,
        "OptiScaler 原件备份不匹配"
    );
    core::assert_target_stopped(target)?;
    replace(&path, &j.before, bytes)
}

pub fn restore_locked(target: &core::DeploymentTarget) -> Result<()> {
    let root = core::no_links(&target.directory.join(core::OWN))?;
    let journal = core::no_links(&root.join(JOURNAL))?;
    if !journal.exists() {
        return Ok(());
    }
    let j: Journal = serde_json::from_value(core::read_json(&journal, 16384)?)?;
    ensure!(
        j.schema == 1
            && j.game_exe == target.game_exe.to_string_lossy()
            && valid_name(&j.name)
            && core::valid_hash(&j.before)
            && core::valid_hash(&j.applied),
        "OptiScaler 恢复记录无效"
    );
    let path = core::no_links(&target.directory.join(&j.name))?;
    let backup = core::no_links(&root.join(BACKUP))?;
    let current = core::digest(&path)?;
    ensure!(
        current == j.before || current == j.applied,
        "OptiScaler 主文件后来已被修改，原件备份已保留，请检查后重试"
    );
    if current != j.before {
        ensure!(
            backup.metadata()?.len() <= LIMIT && core::digest(&backup)? == j.before,
            "OptiScaler 原件备份缺失或已变化，未恢复"
        );
        core::assert_target_stopped(target)?;
        replace(&path, &j.applied, &fs::read(&backup)?)?;
    }
    if backup.exists() {
        ensure!(
            core::digest(&backup)? == j.before,
            "OptiScaler 备份已变化，已保留"
        );
        fs::remove_file(backup)?;
    }
    fs::remove_file(journal)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn restore_keeps_user_changes_and_restores_exact_original() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let target = core::DeploymentTarget::for_game(&dir.path().join("game.exe"));
        fs::create_dir(dir.path().join(core::OWN))?;
        let root = dir.path().join(core::OWN);
        let path = dir.path().join("version.dll");
        fs::write(&path, b"user modified")?;
        fs::write(root.join(BACKUP), b"original")?;
        let j = Journal {
            schema: 1,
            game_exe: target.game_exe.to_string_lossy().into_owned(),
            name: "version.dll".into(),
            before: core::hash(b"original"),
            applied: core::hash(b"paired"),
        };
        core::write_new(&root.join(JOURNAL), &serde_json::to_vec(&j)?)?;
        assert!(restore_locked(&target).is_err());
        assert_eq!(fs::read(&path)?, b"user modified");
        fs::write(&path, b"original")?;
        restore_locked(&target)?;
        assert!(!root.join(JOURNAL).exists());
        assert!(!root.join(BACKUP).exists());
        Ok(())
    }
}
