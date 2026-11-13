//! Update an existing OptiScaler 10 host only when its MFG plugin is selected.
//! The original host is retained until uninstall, independently of plugin files.
use crate::{core, win};
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

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    schema: u8,
    game_exe: String,
    name: String,
    before: String,
    applied: String,
}

fn valid_name(name: &str) -> bool {
    name == PAYLOAD || name == "OptiScaler.asi" || crate::rtxmfg::PROXIES.contains(&name)
}

pub fn host(target: &core::DeploymentTarget) -> Result<PathBuf> {
    crate::mfg_plugin::validate_host(target)?;
    let mut paths = Vec::new();
    for name in crate::rtxmfg::PROXIES
        .into_iter()
        .chain([PAYLOAD, "OptiScaler.asi"])
    {
        let path = core::no_links(&target.directory.join(name))?;
        if path.is_file()
            && let Ok((version, internal)) = win::file_version(&path)
            && internal.eq_ignore_ascii_case("OptiScaler")
            && version.starts_with("10.")
        {
            paths.push(path);
        }
    }
    ensure!(
        paths.len() == 1,
        "检测到多个 OptiScaler 主文件，请先保留一个加载入口"
    );
    Ok(paths.remove(0))
}

pub fn preflight(target: &core::DeploymentTarget, bytes: &[u8], upgrading: bool) -> Result<()> {
    ensure!(
        !bytes.is_empty() && bytes.len() as u64 <= LIMIT,
        "OptiScaler 配套文件大小无效"
    );
    let path = host(target)?;
    ensure!(path.metadata()?.len() <= LIMIT, "OptiScaler 原文件过大");
    if upgrading {
        ensure!(
            core::digest(&path)? == core::hash(bytes),
            "插件升级还需更换配套 OptiScaler；请先卸载 MFG 插件，再选择插件模式安装。原 OptiScaler 会保留"
        );
    }
    Ok(())
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
