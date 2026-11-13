//! AppImage-only updater. The Windows install ticket and trust policy are separate.
use super::{Manifest, verify, verify_appimage, version};
use crate::{core, win};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Ticket {
    schema: u8,
    target: PathBuf,
    old_sha256: String,
    manifest: Manifest,
    parent: u32,
    data_dir: PathBuf,
    wine_prefix: Option<PathBuf>,
}

fn image_from_context(image: &Path, appdir: &Path, exe: &Path) -> Result<PathBuf> {
    ensure!(
        image.is_absolute() && appdir.is_absolute(),
        "AppImage 路径无效"
    );
    let image = core::no_links(image)?;
    ensure!(image.is_file(), "当前 AppImage 不存在，请重新下载");
    ensure!(
        fs::canonicalize(appdir.join("usr/bin/RTXManager"))? == fs::canonicalize(exe)?,
        "当前程序不是从 AppImage 启动，请手动替换安装文件"
    );
    verify_appimage(&image)?;
    Ok(image)
}

fn current_image() -> Result<PathBuf> {
    let image = std::env::var_os("APPIMAGE")
        .context("当前为解包版或手动安装版，请下载新版 AppImage 后手动替换")?;
    let appdir = std::env::var_os("APPDIR").context("缺少 AppImage 运行目录")?;
    image_from_context(
        Path::new(&image),
        Path::new(&appdir),
        &std::env::current_exe()?,
    )
}

/// A child AppImage must not retain paths inside the parent's temporary mount.
fn command(path: &Path) -> Result<Command> {
    let mut command = Command::new(path);
    command.current_dir(path.parent().context("AppImage 路径无效")?);
    if let Some(appdir) = std::env::var_os("APPDIR") {
        let mount = Path::new(&appdir);
        for key in [
            "PATH",
            "LD_LIBRARY_PATH",
            "XDG_DATA_DIRS",
            "GSETTINGS_SCHEMA_DIR",
        ] {
            if let Some(value) = std::env::var_os(key) {
                let paths = std::env::split_paths(&value)
                    .filter(|p| !p.starts_with(mount))
                    .collect::<Vec<_>>();
                if paths.is_empty() {
                    command.env_remove(key);
                } else {
                    command.env(key, std::env::join_paths(paths)?);
                }
            }
        }
    }
    for key in ["APPIMAGE", "APPDIR", "ARGV0", "OWD", "RTXFG_UPDATE_READY"] {
        command.env_remove(key);
    }
    Ok(command)
}

fn copy_executable(source: &Path, target: &Path) -> Result<()> {
    let mut input = fs::File::open(core::no_links(source)?)?;
    let mut output = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(core::no_links(target)?)?;
    std::io::copy(&mut input, &mut output)?;
    output.set_permissions(fs::Permissions::from_mode(0o755))?;
    output.sync_all()?;
    Ok(())
}

pub(super) fn install(path: &Path, manifest: &Manifest, data: &Path) -> Result<()> {
    verify(path, manifest)?;
    ensure!(
        version(crate::VERSION)? < version(&manifest.version)?,
        "当前版本无需更新"
    );
    let target = current_image()?;
    // Fail while the original UI is still open, rather than after it exits.
    drop(
        tempfile::NamedTempFile::new_in(target.parent().context("安装目录无效")?)
            .context("AppImage 所在目录不可写，请将文件移到个人目录后再更新")?,
    );
    let data = core::no_links(data)?;
    let updates = core::no_links(&data.join("updates"))?;
    fs::create_dir_all(&updates)?;
    let folder = updates.join(uuid::Uuid::new_v4().simple().to_string());
    fs::create_dir(&folder)?;
    fs::set_permissions(&folder, fs::Permissions::from_mode(0o700))?;
    let executable = folder.join(&manifest.file);
    copy_executable(path, &executable)?;
    verify(&executable, manifest)?;
    let mut manifest = manifest.clone();
    manifest.notes.clear();
    let ticket = Ticket {
        schema: 1,
        old_sha256: core::digest(&target)?,
        target,
        manifest,
        parent: std::process::id(),
        data_dir: data,
        wine_prefix: win::prefix().ok(),
    };
    let out = folder.join("install-linux.json");
    core::write_new(&out, &serde_json::to_vec(&ticket)?)?;
    let mut child = command(&executable)?
        .arg("--apply-update")
        .arg(out)
        .spawn()?;
    let start = Instant::now();
    loop {
        if let Ok(ready) = core::read_json(&folder.join("helper-ready.json"), 1024)
            && ready["pid"] == child.id()
            && ready["sha256"] == ticket.manifest.sha256
        {
            return Ok(());
        }
        if let Some(status) = child.try_wait()? {
            bail!("AppImage 更新程序无法启动，当前程序未退出：{status}");
        }
        if start.elapsed() > Duration::from_secs(30) {
            let _ = child.kill();
            let _ = child.wait();
            bail!("AppImage 更新程序启动超时，当前程序未退出");
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn read_ticket(path: &Path) -> Result<Ticket> {
    let path = core::no_links(path)?;
    ensure!(
        path.file_name().is_some_and(|s| s == "install-linux.json"),
        "更新安装记录无效"
    );
    let ticket: Ticket = serde_json::from_value(core::read_json(&path, 16384)?)?;
    let folder = path.parent().context("更新目录无效")?;
    let token = folder
        .file_name()
        .context("更新目录无效")?
        .to_string_lossy();
    ensure!(
        ticket.schema == 1
            && ticket.parent > 0
            && core::valid_hash(&ticket.old_sha256)
            && token.len() == 32
            && token.bytes().all(|c| c.is_ascii_hexdigit())
            && ticket.target.is_absolute()
            && ticket.data_dir.is_absolute()
            && folder == core::no_links(&ticket.data_dir.join("updates").join(token.as_ref()))?,
        "更新安装记录或路径无效"
    );
    ticket.manifest.validate(crate::VERSION)?;
    ensure!(
        ticket.manifest.schema == super::LINUX_SCHEMA,
        "Linux 更新清单无效"
    );
    Ok(ticket)
}

pub(super) fn signal_ready(data: &Path) -> Result<()> {
    let Ok(token) = std::env::var("RTXFG_UPDATE_READY") else {
        return Ok(());
    };
    ensure!(
        token.len() == 32 && token.bytes().all(|c| c.is_ascii_hexdigit()),
        "更新确认标识无效"
    );
    let folder = core::no_links(&data.join("updates").join(token))?;
    let ticket = read_ticket(&folder.join("install-linux.json"))?;
    let image = current_image()?;
    ensure!(image == ticket.target, "更新程序路径不匹配");
    verify(&image, &ticket.manifest)?;
    core::atomic_json(
        &folder.join("ready.json"),
        &serde_json::json!({"pid":std::process::id(),"sha256":ticket.manifest.sha256}),
    )
}

pub(super) fn apply(path: &Path) -> Result<()> {
    let _lock = crate::cache::operation_lock()?;
    let ticket = read_ticket(path)?;
    ensure!(ticket.parent != std::process::id(), "更新进程无效");
    let folder = core::no_links(path)?
        .parent()
        .context("更新目录无效")?
        .to_path_buf();
    let token = folder
        .file_name()
        .context("更新目录无效")?
        .to_string_lossy();
    let source = current_image()?;
    ensure!(
        source == folder.join(&ticket.manifest.file),
        "更新程序路径不匹配"
    );
    verify(&source, &ticket.manifest)?;
    let target = core::no_links(&ticket.target)?;
    ensure!(target != source, "不能更新临时安装程序");
    verify_appimage(&target)?;
    ensure!(
        core::digest(&target)? == ticket.old_sha256,
        "原程序已变化，未替换"
    );
    core::atomic_json(
        &folder.join("helper-ready.json"),
        &serde_json::json!({"pid":std::process::id(),"sha256":ticket.manifest.sha256}),
    )?;
    win::wait_process(ticket.parent, 120000)?;
    ensure!(
        core::digest(&target)? == ticket.old_sha256,
        "原程序已变化，未替换"
    );
    let name = target
        .file_name()
        .context("更新目标无效")?
        .to_string_lossy();
    let stage = target.with_file_name(format!("{name}.{token}.new"));
    let backup = target.with_file_name(format!("{name}.{token}.previous"));
    ensure!(!stage.exists() && !backup.exists(), "更新备份路径冲突");
    copy_executable(&source, &stage)?;
    verify(&stage, &ticket.manifest)?;
    let result = super::replace_and_launch(&target, &stage, &backup, &ticket.old_sha256, || {
        let mut cmd = command(&target)?;
        cmd.arg("--data-dir")
            .arg(&ticket.data_dir)
            .env("RTXFG_UPDATE_READY", token.as_ref());
        if let Some(prefix) = &ticket.wine_prefix {
            cmd.arg("--wine-prefix").arg(prefix);
        }
        let mut child = cmd.spawn()?;
        let start = Instant::now();
        loop {
            if let Ok(ready) = core::read_json(&folder.join("ready.json"), 1024)
                && ready["pid"] == child.id()
                && ready["sha256"] == ticket.manifest.sha256
            {
                return Ok(());
            }
            if let Some(status) = child.try_wait()? {
                bail!("新版未能启动：{status}");
            }
            if start.elapsed() > Duration::from_secs(60) {
                let _ = child.kill();
                let _ = child.wait();
                bail!("新版启动超时，已恢复旧版");
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    });
    if let Err(error) = &result {
        core::atomic_json(
            &folder.join("failed.json"),
            &serde_json::json!({"error":error.to_string()}),
        )?;
    }
    result?;
    core::atomic_json(
        &folder.join("installed.json"),
        &serde_json::json!({"version":crate::VERSION,"target":target}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn envelope(path: &Path) -> Result<()> {
        let mut bytes = vec![0u8; 1024 * 1024];
        bytes[..6].copy_from_slice(b"\x7fELF\x02\x01");
        bytes[8..11].copy_from_slice(b"AI\x02");
        bytes[18] = 62;
        fs::write(path, bytes)?;
        Ok(())
    }

    #[test]
    fn image_context_rejects_extracted_binary_and_symlinks() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let image = dir.path().join("Manager.AppImage");
        let appdir = dir.path().join("mount");
        fs::create_dir_all(appdir.join("usr/bin"))?;
        let exe = appdir.join("usr/bin/RTXManager");
        fs::write(&exe, b"executable")?;
        envelope(&image)?;
        assert_eq!(image_from_context(&image, &appdir, &exe)?, image);
        assert!(image_from_context(&image, &appdir, &image).is_err());
        let link = dir.path().join("link.AppImage");
        std::os::unix::fs::symlink(&image, &link)?;
        assert!(image_from_context(&link, &appdir, &exe).is_err());
        Ok(())
    }

    #[test]
    fn executable_copy_and_failed_launch_restore_original_bytes_and_mode() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let target = dir.path().join("My manager.AppImage");
        let stage = dir.path().join("staged");
        let backup = dir.path().join("backup");
        fs::write(&target, b"old")?;
        fs::set_permissions(&target, fs::Permissions::from_mode(0o750))?;
        let source = dir.path().join("new");
        fs::write(&source, b"new")?;
        copy_executable(&source, &stage)?;
        assert_eq!(fs::metadata(&stage)?.permissions().mode() & 0o777, 0o755);
        let digest = core::digest(&target)?;
        assert!(
            super::super::replace_and_launch(&target, &stage, &backup, &digest, || bail!(
                "simulated startup failure"
            ))
            .is_err()
        );
        assert_eq!(fs::read(&target)?, b"old");
        assert_eq!(fs::metadata(&target)?.permissions().mode() & 0o777, 0o750);
        assert!(!backup.exists());
        Ok(())
    }
}
