//! Restore managed games and GPU names before resetting local application data.
use crate::{cleanup, core, executable_name, scanner::Game, win};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
const NOTE: &str = "reset-backup.json";
const READY_ENV: &str = "RTXFG_RESET_READY";
const RESULT: &str = "reset-result.json";

#[derive(Serialize, Deserialize)]
struct Ticket {
    schema: u32,
    parent: u32,
    data: PathBuf,
    executable: PathBuf,
    sha256: String,
    #[serde(default)]
    games: Vec<Game>,
    #[serde(default)]
    archives: Vec<PathBuf>,
}
fn validate_data(data: &Path, executable: &Path) -> Result<PathBuf> {
    let data = core::no_links(data)?;
    ensure!(
        data.file_name()
            .is_some_and(|n| n.eq_ignore_ascii_case("RTXFGManager")),
        "仅可重置 RTXFGManager 数据目录"
    );
    ensure!(
        data.parent().and_then(Path::parent).is_some(),
        "数据目录无效"
    );
    ensure!(
        !core::no_links(executable)?.starts_with(&data),
        "请先将管理器移出缓存目录，再恢复初始状态"
    );
    ensure!(data.is_dir(), "数据目录不存在");
    Ok(data)
}
pub fn available(data: &Path) -> bool {
    std::env::current_exe().is_ok_and(|exe| validate_data(data, &exe).is_ok())
}
fn reset_folder(data: &Path, folder: &Path) -> Result<PathBuf> {
    let folder = core::no_links(folder)?;
    let token = folder
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(|n| n.strip_prefix("RTXFGManager-reset-"))
        .context("重置备份路径无效")?;
    ensure!(
        folder.parent() == data.parent()
            && token.len() == 32
            && token.bytes().all(|b| b.is_ascii_hexdigit()),
        "重置备份路径无效"
    );
    Ok(folder)
}
fn archive_ticket(data: &Path, folder: &Path) -> Result<Ticket> {
    let folder = reset_folder(data, folder)?;
    let t: Ticket = serde_json::from_value(core::read_json(
        &folder.join("reset.json"),
        16 * 1024 * 1024,
    )?)?;
    ensure!(
        [1, 2].contains(&t.schema)
            && core::key(&t.data) == core::key(data)
            && core::valid_hash(&t.sha256),
        "重置备份归属无法确认：{}",
        folder.display()
    );
    Ok(t)
}
fn saved_games(data: &Path) -> Result<Vec<Game>> {
    let mut games = Vec::new();
    // Last-good can retain a game removed from the current library.
    for name in ["games.json", "games.last-good.json"] {
        let path = core::no_links(&data.join(name))?;
        if path.try_exists()? {
            let v = core::read_json(&path, 4 * 1024 * 1024)
                .with_context(|| format!("无法读取游戏恢复记录：{}", path.display()))?;
            ensure!(
                v["schema"] == 3,
                "游戏恢复记录版本不受支持：{}",
                path.display()
            );
            let rows: Vec<Game> = serde_json::from_value(v["games"].clone())?;
            ensure!(rows.len() <= 10000, "游戏恢复记录过多");
            games.extend(rows);
        }
    }
    Ok(games)
}
fn previous_archives(data: &Path) -> Result<Vec<PathBuf>> {
    let mut archives = Vec::new();
    for entry in fs::read_dir(data.parent().context("数据目录无效")?)? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(token) = name
            .to_str()
            .and_then(|n| n.strip_prefix("RTXFGManager-reset-"))
        else {
            continue;
        };
        if token.len() != 32 || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
            continue;
        }
        let folder = reset_folder(data, &entry.path())?;
        // An unrelated similarly named folder without an ownership ticket stays.
        if !folder.join("reset.json").try_exists()? {
            continue;
        }
        archive_ticket(data, &folder)?;
        ensure!(archives.len() < 128, "重置备份过多，请先检查备份目录");
        archives.push(folder);
    }
    Ok(archives)
}
fn targets(games: &[Game]) -> Result<Vec<core::DeploymentTarget>> {
    let mut rows: BTreeMap<String, core::DeploymentTarget> = BTreeMap::new();
    for g in games {
        for exe in std::iter::once(&g.exe)
            .chain(&g.targets)
            .chain(&g.cleanup_only)
        {
            let t = g.deployment_target(Path::new(exe));
            if let Some(old) = rows.get_mut(&core::key(&t.directory)) {
                old.guard_exes.push(t.game_exe);
                old.guard_exes.extend(t.guard_exes);
            } else {
                rows.insert(core::key(&t.directory), t);
            }
        }
    }
    rows.into_values()
        .map(|t| {
            t.validate(false).with_context(|| {
                format!(
                    "无法检查游戏目录，请确认磁盘已连接：{}",
                    t.directory.display()
                )
            })
        })
        .collect()
}
fn no_recovery_records(t: &core::DeploymentTarget) -> Result<()> {
    for name in [
        core::OWN,
        ".rtx-fg-nr",
        ".rtx-fg-runtimes",
        ".rtx-fg-v3-legacy.json",
    ] {
        let p = core::no_links(&t.directory.join(name))?;
        ensure!(
            !p.try_exists()?,
            "仍有恢复记录或未确认文件，未清空管理器数据：{}",
            p.display()
        );
    }
    Ok(())
}
fn check_tree(root: &Path) -> Result<()> {
    let mut pending = vec![core::no_links(root)?];
    let mut count = 0;
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(dir)? {
            let path = entry?.path();
            let m = fs::symlink_metadata(&path)?;
            ensure!(
                !core::is_link(&m),
                "备份内有链接，未删除：{}",
                path.display()
            );
            ensure!(m.is_file() || m.is_dir(), "备份内有未知文件类型");
            count += 1;
            ensure!(count <= 100000, "备份文件过多，未自动删除");
            if m.is_dir() {
                pending.push(path);
            }
        }
    }
    Ok(())
}
fn restore_targets(
    targets: &[core::DeploymentTarget],
    mut uninstall: impl FnMut(&core::DeploymentTarget) -> Result<cleanup::CleanOutcome>,
    restore_names: impl FnOnce() -> Result<()>,
) -> Result<()> {
    for t in targets {
        let out = uninstall(t)
            .with_context(|| format!("{}：还原未完成，记录已保留", t.directory.display()))?;
        ensure!(out.complete, "{}：{}", t.directory.display(), out.message);
        no_recovery_records(t)?;
    }
    restore_names().context("显卡名称未全部还原，未清空管理器数据")
}
/// Restore while the UI can report failures; the helper clears state only after
/// every parent writer exits. Games absent from all saved records are not guessed.
pub fn prepare(data: &Path, current: &[Game], mut progress: impl FnMut(String)) -> Result<()> {
    let executable = core::no_links(&std::env::current_exe()?)?;
    let data = validate_data(data, &executable)?;
    let _operation = crate::cache::operation_lock()?;
    let archives = previous_archives(&data)?;
    let mut games = current.to_vec();
    games.extend(saved_games(&data)?);
    check_tree(&data)?;
    for folder in &archives {
        ensure!(
            !core::within(&executable, folder),
            "请先将管理器移出重置备份目录"
        );
        check_tree(folder)?;
        games.extend(archive_ticket(&data, folder)?.games);
        for name in ["previous", "failed-start"] {
            if folder.join(name).is_dir() {
                games.extend(saved_games(&folder.join(name))?);
            }
        }
    }
    let rows = targets(&games)?;
    for t in &rows {
        ensure!(
            !core::within(&t.directory, &data)
                && !archives.iter().any(|p| core::within(&t.directory, p)),
            "游戏目录位于待重置数据内，未重置"
        );
        core::assert_target_stopped(t)?;
    }
    progress("正在卸载补丁并恢复游戏原件…".into());
    restore_targets(
        &rows,
        cleanup::clean_outcome_at,
        crate::gpu_alias::restore_all,
    )?;
    let t = Ticket {
        schema: 2,
        parent: std::process::id(),
        data: data.clone(),
        sha256: core::digest(&executable)?,
        executable: executable.clone(),
        games,
        archives,
    };
    let folder = data.with_file_name(format!(
        "RTXFGManager-reset-{}",
        uuid::Uuid::new_v4().simple()
    ));
    core::no_links(&folder)?;
    fs::create_dir(&folder)?;
    let path = folder.join("reset.json");
    core::write_new(&path, &serde_json::to_vec(&t)?)?;
    progress("正在清空设置并重启…".into());
    executable_name::command(&executable)
        .args(["--reset-manager".as_ref(), path.as_os_str()])
        .current_dir(executable.parent().context("程序路径无效")?)
        .creation_flags(0x08000000)
        .spawn()?;
    Ok(())
}
fn rotate(data: &Path, archive: &Path, launch: impl FnOnce() -> Result<()>) -> Result<()> {
    let data = core::no_links(data)?;
    let archive = core::no_links(archive)?;
    ensure!(
        !archive.try_exists()? && !archive.starts_with(&data) && !data.starts_with(&archive),
        "重置备份路径无效或已存在"
    );
    win::rename_no_replace(&data, &archive)?;
    let result = (|| {
        fs::create_dir(&data)?;
        core::write_new(
            &data.join(NOTE),
            &serde_json::to_vec(&serde_json::json!({"archive":archive}))?,
        )?;
        launch()
    })();
    if let Err(e) = result {
        // The failed child may already have saved new files; preserve them too.
        if data.try_exists()? {
            win::rename_no_replace(&data, &archive.with_file_name("failed-start"))?;
        }
        win::rename_no_replace(&archive, &data)?;
        return Err(e);
    }
    Ok(())
}
fn remove_tree(path: &Path) -> Result<()> {
    let path = core::no_links(path)?;
    if path.is_dir() {
        for e in fs::read_dir(&path)? {
            remove_tree(&e?.path())?;
        }
        fs::remove_dir(path)?;
    } else {
        fs::remove_file(path)?;
    }
    Ok(())
}
fn remove_archive(data: &Path, folder: &Path) -> Result<()> {
    let folder = reset_folder(data, folder)?;
    archive_ticket(data, &folder)?;
    check_tree(&folder)?;
    // Retain ownership until all contents are gone, so a file lock can be retried.
    for e in fs::read_dir(&folder)? {
        let path = e?.path();
        if path.file_name().is_some_and(|n| n == "reset.json") {
            continue;
        }
        remove_tree(&path)?;
    }
    fs::remove_file(folder.join("reset.json"))?;
    fs::remove_dir(folder)?;
    Ok(())
}
pub fn apply(path: &Path) -> Result<()> {
    let path = core::no_links(path)?;
    ensure!(
        path.file_name().is_some_and(|n| n == "reset.json"),
        "重置记录无效"
    );
    let t: Ticket = serde_json::from_value(core::read_json(&path, 16 * 1024 * 1024)?)?;
    ensure!(
        t.schema == 2 && t.parent > 0 && t.parent != std::process::id(),
        "重置记录无效"
    );
    let exe = core::no_links(&std::env::current_exe()?)?;
    ensure!(
        exe == t.executable && core::valid_hash(&t.sha256) && core::digest(&exe)? == t.sha256,
        "程序已变化，未重置"
    );
    let data = validate_data(&t.data, &exe)?;
    let folder = reset_folder(&data, path.parent().context("重置备份路径无效")?)?;
    win::wait_process(t.parent, 120000)?;
    let instance = win::instance_lock(&data)?;
    let operation = crate::cache::operation_lock()?;
    for target in targets(&t.games)? {
        core::assert_target_stopped(&target)?;
        no_recovery_records(&target)?;
    }
    rotate(&data, &folder.join("previous"), || {
        drop(instance);
        let mut child = executable_name::command(&exe)
            .args(["--data-dir".as_ref(), data.as_os_str()])
            .env(READY_ENV, &folder)
            .env_remove("RTXFG_UPDATE_READY")
            .current_dir(exe.parent().context("程序路径无效")?)
            .creation_flags(0x08000000)
            .spawn()?;
        let result = (|| {
            let start = Instant::now();
            loop {
                if let Ok(v) = core::read_json(&folder.join("ready.json"), 1024)
                    && v["pid"] == child.id()
                    && v["sha256"] == t.sha256
                {
                    return Ok(());
                }
                ensure!(
                    child.try_wait()?.is_none(),
                    "重置后的程序未能启动，已保留原设置"
                );
                ensure!(
                    start.elapsed() < Duration::from_secs(60),
                    "重置后的程序启动超时，已保留原设置"
                );
                std::thread::sleep(Duration::from_millis(100));
            }
        })();
        if result.is_err() {
            let _ = child.kill();
            let _ = child.wait();
        }
        result
    })?;
    let result = (|| -> Result<()> {
        for old in &t.archives {
            remove_archive(&data, old)?;
        }
        remove_archive(&data, &folder)?;
        fs::remove_file(data.join(NOTE))?;
        Ok(())
    })();
    drop(operation);
    core::atomic_json(
        &data.join(RESULT),
        &serde_json::json!({"error":result.err().map(|e|format!("重置已完成，部分备份未能删除：{e:#}"))}),
    )
}
/// Startup acknowledgment runs off the UI thread. Backup deletion waits for the
/// rendered child's PID/hash; completion updates the backup entry in that child.
pub fn finish_startup(data: &Path) -> Result<()> {
    let Some(folder) = std::env::var_os(READY_ENV) else {
        return Ok(());
    };
    let folder = reset_folder(data, Path::new(&folder))?;
    let t = archive_ticket(data, &folder)?;
    let exe = core::no_links(&std::env::current_exe()?)?;
    ensure!(
        exe == t.executable && core::digest(&exe)? == t.sha256,
        "重置启动确认不匹配"
    );
    core::atomic_json(
        &folder.join("ready.json"),
        &serde_json::json!({"pid":std::process::id(),"sha256":t.sha256}),
    )?;
    let start = Instant::now();
    loop {
        if let Ok(v) = core::read_json(&data.join(RESULT), 16384) {
            fs::remove_file(data.join(RESULT))?;
            if let Some(e) = v["error"].as_str() {
                anyhow::bail!("{e}");
            }
            return Ok(());
        }
        ensure!(
            start.elapsed() < Duration::from_secs(120),
            "备份清理尚未完成，恢复文件已保留"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}
pub fn backup(data: &Path) -> Option<PathBuf> {
    let v = core::read_json(&data.join(NOTE), 16384).ok()?;
    let path = core::no_links(Path::new(v["archive"].as_str()?)).ok()?;
    reset_folder(data, path.parent()?).ok()?;
    (path.file_name().is_some_and(|n| n == "previous") && path.is_dir()).then_some(path)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn ticket(data: &Path, folder: &Path) {
        fs::create_dir_all(folder).unwrap();
        core::atomic_json(
            &folder.join("reset.json"),
            &Ticket {
                schema: 1,
                parent: 1,
                data: data.into(),
                executable: "test.exe".into(),
                sha256: "a".repeat(64),
                games: vec![],
                archives: vec![],
            },
        )
        .unwrap();
    }
    #[test]
    fn success_purges_multiple_owned_archives_and_preserves_unrelated_sibling() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("RTXFGManager");
        fs::create_dir(&data).unwrap();
        let other = tmp.path().join("RTXFGManager-reset-unrelated");
        fs::create_dir(&other).unwrap();
        fs::write(other.join("keep"), b"keep").unwrap();
        let mut old = vec![];
        for ch in ['a', 'b'] {
            let folder = tmp
                .path()
                .join(format!("RTXFGManager-reset-{}", ch.to_string().repeat(32)));
            ticket(&data, &folder);
            fs::create_dir(folder.join("previous")).unwrap();
            fs::write(folder.join("previous/session"), b"old").unwrap();
            old.push(folder);
        }
        assert_eq!(previous_archives(&data).unwrap().len(), 2);
        for folder in old {
            remove_archive(&data, &folder).unwrap();
            assert!(!folder.exists());
        }
        assert_eq!(fs::read(other.join("keep")).unwrap(), b"keep");
    }
    #[test]
    fn pending_cleanup_prevents_gpu_restore_and_data_purge() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("RTXFGManager");
        fs::create_dir(&data).unwrap();
        fs::write(data.join("session"), b"keep").unwrap();
        let target = core::DeploymentTarget::for_game(&tmp.path().join("game.exe"));
        let mut restored = false;
        assert!(
            restore_targets(
                &[target],
                |_| Ok(cleanup::CleanOutcome {
                    message: "locked log".into(),
                    complete: false
                }),
                || {
                    restored = true;
                    Ok(())
                }
            )
            .is_err()
        );
        assert!(!restored);
        assert_eq!(fs::read(data.join("session")).unwrap(), b"keep");
    }
    #[test]
    fn failed_restart_restores_settings_even_after_child_writes() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("RTXFGManager");
        let archive = tmp.path().join("previous");
        fs::create_dir(&data).unwrap();
        fs::write(data.join("games.json"), b"original").unwrap();
        assert!(
            rotate(&data, &archive, || {
                fs::write(data.join("new"), b"child")?;
                anyhow::bail!("startup failed")
            })
            .is_err()
        );
        assert_eq!(fs::read(data.join("games.json")).unwrap(), b"original");
        assert_eq!(
            fs::read(tmp.path().join("failed-start/new")).unwrap(),
            b"child"
        );
        assert!(!archive.exists());
    }
    #[test]
    fn targets_include_old_custom_and_grouped_directories() {
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("game");
        let b = tmp.path().join("old");
        let c = tmp.path().join("plugin");
        for p in [&a, &b, &c] {
            fs::create_dir(p).unwrap();
        }
        let g = Game {
            exe: a.join("game.exe").display().to_string(),
            targets: vec![a.join("renderer.exe").display().to_string()],
            cleanup_only: vec![b.join("old.exe").display().to_string()],
            ..Default::default()
        };
        let plugin = Game {
            deployment_dir: c.display().to_string(),
            ..g.clone()
        };
        let rows = targets(&[g, plugin]).unwrap();
        assert_eq!(rows.len(), 3);
        assert!(rows.iter().all(|t| !t.guard_exes.is_empty()));
    }
    #[test]
    fn unowned_and_nested_archives_are_not_deleted() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("RTXFGManager");
        fs::create_dir(&data).unwrap();
        let folder = data.join(format!("RTXFGManager-reset-{}", "a".repeat(32)));
        ticket(&data, &folder);
        assert!(remove_archive(&data, &folder).is_err());
        assert!(folder.exists());
        assert!(validate_data(&data, &data.join("RTXManager.exe")).is_err());
    }

    #[test]
    fn restored_games_do_not_allow_purge_when_gpu_restore_fails() {
        let tmp = tempfile::tempdir().unwrap();
        let target = core::DeploymentTarget::for_game(&tmp.path().join("game.exe"));
        let result = restore_targets(
            &[target],
            |_| {
                Ok(cleanup::CleanOutcome {
                    message: "restored".into(),
                    complete: true,
                })
            },
            || anyhow::bail!("driver changed"),
        );
        assert!(format!("{:#}", result.unwrap_err()).contains("driver changed"));
    }

    #[test]
    fn backup_junction_is_rejected_without_touching_its_destination() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("backup");
        let external = tmp.path().join("external");
        fs::create_dir(&root).unwrap();
        fs::create_dir(&external).unwrap();
        fs::write(external.join("keep"), b"original").unwrap();
        std::os::windows::fs::symlink_dir(&external, root.join("link")).unwrap();
        assert!(check_tree(&root).is_err());
        assert_eq!(fs::read(external.join("keep")).unwrap(), b"original");
    }
}
