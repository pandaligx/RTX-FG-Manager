//! Explicit, same-version filename migration. Normal update tickets stay unchanged.
use crate::{core, manager_shortcuts, updater, win};
use anyhow::{Context, Result, bail, ensure};
use std::{
    fs,
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};

pub const FILE_NAME: &str = "RTXManager.exe";
pub(crate) const READY_ENV: &str = "RTXFG_RENAME_READY";

pub(crate) fn command(path: &Path) -> Command {
    let mut command = Command::new(path);
    // Readiness tokens belong to one launch only. A later rename/update must
    // never inherit the previous operation's ticket or verified file hash.
    command
        .env_remove(READY_ENV)
        .env_remove("RTXFG_UPDATE_READY")
        .env_remove("RTXFG_RESET_READY");
    command
}

pub fn is_standard(path: &Path) -> bool {
    path.file_name()
        .is_some_and(|n| n.eq_ignore_ascii_case(FILE_NAME))
}
fn destination(target: &Path) -> Result<PathBuf> {
    ensure!(!is_standard(target), "当前已使用 RTXManager.exe，无需改名");
    ensure!(
        target
            .extension()
            .is_some_and(|s| s.eq_ignore_ascii_case("exe")),
        "管理器路径无效"
    );
    let destination = core::no_links(&target.with_file_name(FILE_NAME))?;
    ensure!(
        !destination.exists(),
        "同目录已有 RTXManager.exe，未改名。请先确认并移走重复文件。"
    );
    Ok(destination)
}
fn folder(data: &Path, token: &str) -> Result<PathBuf> {
    ensure!(
        token.len() == 32 && token.bytes().all(|b| b.is_ascii_hexdigit()),
        "改名记录无效"
    );
    core::no_links(&data.join("updates").join(token))
}

/// Runs on a worker after explicit confirmation; no installed file is changed here.
pub fn prepare(data: &Path) -> Result<()> {
    let target = core::no_links(&std::env::current_exe()?)?;
    let next = destination(&target)?;
    let signature = win::verify_signature(&target)?;
    ensure!(
        updater::version(&signature.version)? == updater::version(crate::VERSION)?,
        "管理器版本信息不匹配"
    );
    manager_shortcuts::Plan::inspect(&target, &next, &manager_shortcuts::roots()?)?;
    let hash = core::digest(&target)?;
    let ticket = updater::Ticket {
        schema: 1,
        target: target.clone(),
        old_sha256: hash.clone(),
        manifest: updater::Manifest {
            schema: 1,
            version: crate::VERSION.into(),
            file: format!("RTXManager-v{}-x64.exe", crate::VERSION),
            sha256: hash,
            bytes: fs::metadata(&target)?.len(),
            source: String::new(),
            url: String::new(),
            notes: Default::default(),
        },
        parent: std::process::id(),
        data_dir: core::no_links(data)?,
    };
    let token = uuid::Uuid::new_v4().simple().to_string();
    let folder = folder(data, &token)?;
    fs::create_dir_all(folder.parent().context("改名暂存路径无效")?)?;
    fs::create_dir(&folder)?;
    let helper = folder.join(FILE_NAME);
    copy_new(&target, &helper)?;
    updater::verify(&helper, &ticket.manifest)?;
    let path = folder.join("rename.json");
    core::write_new(&path, &serde_json::to_vec(&ticket)?)?;
    command(&helper)
        .args(["--rename-manager".as_ref(), path.as_os_str()])
        .current_dir(&folder)
        .creation_flags(0x08000000)
        .spawn()?;
    Ok(())
}
fn copy_new(source: &Path, target: &Path) -> Result<()> {
    let mut input = fs::File::open(core::no_links(source)?)?;
    let mut output = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(core::no_links(target)?)?;
    std::io::copy(&mut input, &mut output)?;
    output.sync_all()?;
    Ok(())
}

pub fn signal_ready(data: &Path) -> Result<()> {
    let Ok(token) = std::env::var(READY_ENV) else {
        return Ok(());
    };
    let folder = folder(data, &token)?;
    let ticket: updater::Ticket =
        serde_json::from_value(core::read_json(&folder.join("rename.json"), 16384)?)?;
    let exe = core::no_links(&std::env::current_exe()?)?;
    ensure!(
        core::key(&exe) == core::key(&ticket.target.with_file_name(FILE_NAME)),
        "改名后的程序路径不匹配"
    );
    updater::verify(&exe, &ticket.manifest)?;
    core::atomic_json(
        &folder.join("ready.json"),
        &serde_json::json!({"pid":std::process::id(),"sha256":ticket.manifest.sha256}),
    )
}

pub fn apply(path: &Path) -> Result<()> {
    let _lock = crate::cache::operation_lock()?;
    let path = core::no_links(path)?;
    ensure!(
        path.file_name().is_some_and(|s| s == "rename.json"),
        "改名记录无效"
    );
    let t: updater::Ticket = serde_json::from_value(core::read_json(&path, 16384)?)?;
    ensure!(
        t.schema == 1
            && t.parent > 0
            && t.parent != std::process::id()
            && core::valid_hash(&t.old_sha256),
        "改名记录无效"
    );
    let parent = path.parent().context("改名暂存路径无效")?;
    let token = parent
        .file_name()
        .context("改名暂存路径无效")?
        .to_string_lossy();
    ensure!(folder(&t.data_dir, &token)? == parent, "改名暂存路径无效");
    let source = core::no_links(&std::env::current_exe()?)?;
    ensure!(source == parent.join(FILE_NAME), "改名辅助程序路径无效");
    t.manifest.validate(crate::VERSION)?;
    updater::verify(&source, &t.manifest)?;
    let target = core::no_links(&t.target)?;
    updater::verify(&target, &t.manifest)?;
    ensure!(t.old_sha256 == t.manifest.sha256, "改名不允许更换程序内容");
    let next = destination(&target)?;
    win::wait_process(t.parent, 120000)?;
    ensure!(
        core::digest(&target)? == t.old_sha256,
        "原程序已变化，未改名"
    );
    let shortcuts = manager_shortcuts::Plan::inspect(&target, &next, &manager_shortcuts::roots()?)?;
    shortcuts.save_backup(parent)?;
    let stage = target.with_file_name(format!("RTXManager.{token}.new"));
    let backup = target.with_file_name(format!("RTXManager.{token}.previous"));
    copy_new(&source, &stage)?;
    updater::verify(&stage, &t.manifest)?;
    updater::replace_named_and_launch(&target, &next, &stage, &backup, &t.old_sha256, || {
        shortcuts.with_changes(|| launch_and_wait(&next, &t, &token, parent))
    })?;
    core::atomic_json(
        &parent.join("installed.json"),
        &serde_json::json!({"operation":"rename", "target":next, "previous_target":target, "version":crate::VERSION}),
    )
}
fn launch_and_wait(next: &Path, t: &updater::Ticket, token: &str, folder: &Path) -> Result<()> {
    let mut child = command(next)
        .args(["--data-dir".as_ref(), t.data_dir.as_os_str()])
        .env(READY_ENV, token)
        .env_remove("RTXFG_UPDATE_READY")
        .current_dir(next.parent().context("管理器路径无效")?)
        .creation_flags(0x08000000)
        .spawn()?;
    let result = (|| {
        let start = Instant::now();
        loop {
            if let Ok(ready) = core::read_json(&folder.join("ready.json"), 1024)
                && ready["pid"] == child.id()
                && ready["sha256"] == t.manifest.sha256
            {
                return Ok(());
            }
            if let Some(status) = child.try_wait()? {
                bail!("改名后的程序未能启动：{status}");
            }
            ensure!(
                start.elapsed() <= Duration::from_secs(60),
                "改名后的程序启动超时，恢复原文件名"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
    })();
    if result.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_restart_clears_previous_operation_tokens() {
        let mut next = command(Path::new("RTXManager.exe"));
        let values: std::collections::BTreeMap<_, _> = next.get_envs().collect();
        assert_eq!(values[std::ffi::OsStr::new(READY_ENV)], None);
        assert_eq!(values[std::ffi::OsStr::new("RTXFG_UPDATE_READY")], None);
        next.env("RTXFG_UPDATE_READY", "new-ticket");
        let values: std::collections::BTreeMap<_, _> = next.get_envs().collect();
        assert_eq!(values[std::ffi::OsStr::new(READY_ENV)], None);
        assert_eq!(
            values[std::ffi::OsStr::new("RTXFG_UPDATE_READY")],
            Some(std::ffi::OsStr::new("new-ticket"))
        );
    }
    #[test]
    fn canonical_name_conflict_never_overwrites_another_executable() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let old = temp.path().join("RTXManager-v4.3.18-x64.exe");
        let fixed = temp.path().join(FILE_NAME);
        fs::write(&old, b"old")?;
        assert_eq!(destination(&old)?, fixed);
        fs::write(&fixed, b"unrelated")?;
        assert!(destination(&old).is_err());
        assert_eq!(fs::read(fixed)?, b"unrelated");
        assert_eq!(fs::read(old)?, b"old");
        assert!(is_standard(Path::new("c:/App/RTXMANAGER.EXE")));
        assert!(!is_standard(Path::new("c:/App/RTXManager-v4.3.18-x64.exe")));
        Ok(())
    }
    #[test]
    fn rename_failure_restores_old_name_and_leaves_other_files() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let old = temp.path().join("我的管理器.exe");
        let next = temp.path().join(FILE_NAME);
        let stage = temp.path().join("rename.new");
        let backup = temp.path().join("rename.previous");
        let other = temp.path().join("unrelated.exe");
        fs::write(&old, b"manager")?;
        fs::write(&stage, b"manager")?;
        fs::write(&other, b"keep")?;
        assert!(
            updater::replace_named_and_launch(
                &old,
                &next,
                &stage,
                &backup,
                &core::digest(&old)?,
                || bail!("injected failure")
            )
            .is_err()
        );
        assert_eq!(fs::read(old)?, b"manager");
        assert!(!next.exists());
        assert!(!backup.exists());
        assert_eq!(fs::read(other)?, b"keep");
        Ok(())
    }
}
