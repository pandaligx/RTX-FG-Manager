//! Optional NR runtime. Exact download digests identify upstream NVIDIA files;
//! they are not a deployment whitelist for the user's signed Encore proxies.
use crate::{assets, core, encore, scanner, transfer, win};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

const FILE: &str = "nvngx_dlssnr.dll";
const OWN: &str = ".rtx-fg-nr";
const LIMIT: u64 = 512 * 1024 * 1024;
/// The newest qualified runtime. Choosing it does not replace an installed file.
pub const RECOMMENDED: &str = "310.9.16";

#[derive(Deserialize)]
struct Runtime {
    version: String,
    size: u64,
    sha256: String,
    #[serde(rename = "archiveSize")]
    archive_size: u64,
    #[serde(rename = "archiveSha256")]
    archive_sha256: String,
    urls: Vec<String>,
}
fn runtime(version: &str) -> Result<Runtime> {
    let entries: Vec<Runtime> =
        serde_json::from_str(include_str!("../assets/neural-runtimes.json"))?;
    entries
        .into_iter()
        .find(|r| r.version == version)
        .context("不支持的 NR 运行库版本")
}

fn prepare(
    version: &str,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(transfer::Progress),
) -> Result<PathBuf> {
    let r = runtime(version)?;
    let root = core::no_links(&assets::cache_root()?.join("neural-rendering"))?;
    fs::create_dir_all(&root)?;
    let _lock = win::game_lock(&root)?;
    let destination = core::no_links(&root.join(format!("{}-{}.dll", version, r.sha256)))?;
    if matches_file(&destination, &r.sha256)? {
        return Ok(destination);
    }
    ensure!(!destination.exists(), "NR 缓存已变化，请移除损坏缓存后重试");
    let archive = core::no_links(&root.join(format!("{}.zip", r.archive_sha256)))?;
    transfer::download(
        &transfer::Request {
            sources: r
                .urls
                .iter()
                .enumerate()
                .map(|(i, url)| transfer::Source {
                    name: format!("NR {}", i + 1),
                    url: url.clone(),
                })
                .collect(),
            bytes: Some(r.archive_size),
            sha256: Some(r.archive_sha256),
            limit: LIMIT,
            metadata: false,
            policy: transfer::UrlPolicy::Cloud,
        },
        &archive,
        cancel,
        progress,
    )?;
    let mut zip = zip::ZipArchive::new(fs::File::open(&archive)?)?;
    ensure!(zip.len() == 1, "NR 压缩包内容异常");
    let mut entry = zip.by_index(0)?;
    ensure!(
        entry.name() == FILE
            && entry.size() == r.size
            && !entry.is_dir()
            && entry.unix_mode().is_none_or(|m| m & 0o170000 != 0o120000),
        "NR 压缩包文件异常"
    );
    let mut stage = tempfile::NamedTempFile::new_in(&root)?;
    std::io::copy(&mut (&mut entry).take(r.size + 1), &mut stage)?;
    stage.as_file().sync_all()?;
    ensure!(
        stage.as_file().metadata()?.len() == r.size && core::digest(stage.path())? == r.sha256,
        "NR 运行库摘要不匹配"
    );
    core::pe64(stage.path(), true)?;
    ensure!(!cancel.load(Ordering::Relaxed), "下载已取消");
    stage.persist_noclobber(&destination)?;
    Ok(destination)
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Ownership {
    schema: u32,
    game: String,
    directory: String,
    installed: String,
    previous: Option<String>,
    original: Option<String>,
}
/// Display-only metadata, read in the deployment worker. Never hashes a large
/// runtime on render; install/uninstall still perform full ownership validation.
#[derive(Clone, Debug, Default)]
pub struct Status {
    pub path: PathBuf,
    pub present: bool,
    pub version: Option<String>,
    pub managed: bool,
    pub warning: Option<String>,
    pub encore_version: Option<String>,
}
impl Status {
    pub fn label(&self) -> String {
        if let Some(warning) = &self.warning {
            return warning.clone();
        }
        if self.present {
            format!(
                "{} · {}",
                if self.managed {
                    "NR 已安装"
                } else {
                    "NR 已检测到（外部安装）"
                },
                self.version.as_deref().unwrap_or("版本未知")
            )
        } else {
            "NR 未安装".into()
        }
    }
}
pub fn inspect(target: &core::DeploymentTarget) -> Status {
    let path = target.directory.join(FILE);
    let mut status = Status {
        path: path.clone(),
        ..Default::default()
    };
    let result = (|| -> Result<()> {
        core::no_links(&path)?;
        let record = read(target)?;
        status.managed = record.is_some();
        status.present = path.is_file();
        if status.present {
            status.version = win::file_version(&path).ok().map(|v| v.0);
        } else if status.managed {
            status.warning = Some("NR 文件缺失，可重新安装".into());
        }
        Ok(())
    })();
    if result.is_err() {
        status.warning = Some("NR 状态异常，请检查安装目录".into());
    }
    status
}

fn matches_file(path: &Path, hash: &str) -> Result<bool> {
    core::no_links(path)?;
    Ok(path.is_file() && path.metadata()?.len() <= LIMIT && core::digest(path)? == hash)
}
fn read(target: &core::DeploymentTarget) -> Result<Option<Ownership>> {
    let p = core::no_links(&target.directory.join(OWN).join("ownership.json"))?;
    if !p.exists() {
        return Ok(None);
    }
    ensure!(
        p.is_file() && p.metadata()?.len() <= 16384,
        "NR 归属记录异常"
    );
    let r: Ownership = serde_json::from_slice(&fs::read(p)?)?;
    ensure!(
        r.schema == 1
            && core::key(Path::new(&r.game)) == core::key(&target.game_exe)
            && core::key(Path::new(&r.directory)) == core::key(&target.directory)
            && core::valid_hash(&r.installed)
            && r.original.as_ref().is_none_or(|h| core::valid_hash(h))
            && r.previous.as_ref().is_none_or(|h| core::valid_hash(h)),
        "NR 归属记录不属于此游戏"
    );
    Ok(Some(r))
}
fn original_valid(root: &Path, r: &Ownership) -> Result<()> {
    if let Some(hash) = &r.original {
        ensure!(
            matches_file(&root.join("original.dll"), hash)?,
            "NR 原文件备份已变化，已保留"
        );
    }
    Ok(())
}
fn accepted(path: &Path, r: &Ownership) -> Result<bool> {
    if !path.exists() {
        return Ok(true);
    }
    for hash in std::iter::once(&r.installed)
        .chain(r.previous.iter())
        .chain(r.original.iter())
    {
        if matches_file(path, hash)? {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Called in a worker; no GPU probe and no game binary is loaded.
pub fn install(
    target: &core::DeploymentTarget,
    version: &str,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(transfer::Progress),
) -> Result<()> {
    let target = target.validate(true)?;
    let _cache_lock = crate::cache::wait_for_operation(cancel, || {
        progress(transfer::Progress {
            phase: transfer::Phase::Waiting,
            ..Default::default()
        });
    })?;
    let _lock = win::game_lock(&target.directory)?;
    core::assert_target_stopped(&target)?;
    let record = core::record(&target.directory)?.context("请先安装 RTX Encore")?;
    target.validate_record(&record)?;
    ensure!(record.backend == encore::BACKEND, "请先安装 RTX Encore");
    ensure!(
        !record.cache_pending && !crate::encore_upgrade::has_pending(&target.directory)?,
        "请先完成补丁安装或恢复，再添加 NR 运行库"
    );
    if version == "310.9.16" {
        ensure!(
            record
                .upstream_version
                .as_deref()
                .or(record.payload_version.as_deref())
                .and_then(|v| semver::Version::parse(v).ok())
                .is_some_and(|v| v >= semver::Version::new(1, 1, 0)),
            "NR 310.9.16 需要 RTX Encore 1.1.0 或更新版本"
        );
    }
    ensure!(
        scanner::inspect(&target.game_exe).super_resolution,
        "未发现游戏的 DLSS 超分组件；不会添加 NR 运行库"
    );
    let source = prepare(version, cancel, progress)?;
    core::assert_target_stopped(&target)?;
    ensure!(!cancel.load(Ordering::Relaxed), "下载已取消");
    install_file_locked(&target, &source)
}
fn install_file_locked(target: &core::DeploymentTarget, source: &Path) -> Result<()> {
    let root = core::no_links(&target.directory.join(OWN))?;
    let dest = core::no_links(&target.directory.join(FILE))?;
    let next = core::digest(source)?;
    let prior = read(target)?;
    if let Some(r) = &prior {
        original_valid(&root, r)?;
        ensure!(
            accepted(&dest, r)?,
            "NR 文件已被其他程序修改，已保留；请先处理冲突"
        );
        if matches_file(&dest, &next)? {
            return Ok(());
        }
    } else {
        ensure!(!root.exists(), "发现未完成的 NR 备份，请保留并检查该目录");
        ensure!(
            !dest.exists() || (dest.is_file() && dest.metadata()?.len() <= LIMIT),
            "NR 文件类型或大小异常"
        );
        fs::create_dir(&root)?;
    }
    let before = if dest.exists() {
        Some(core::digest(&dest)?)
    } else {
        None
    };
    let original = if let Some(r) = &prior {
        r.original.clone()
    } else {
        before.clone()
    };
    if prior.is_none()
        && let Some(hash) = &original
    {
        fs::copy(&dest, root.join("original.dll"))?;
        ensure!(
            matches_file(&root.join("original.dll"), hash)?,
            "NR 备份不完整，原文件未替换"
        );
    }
    let r = Ownership {
        schema: 1,
        game: target.game_exe.to_string_lossy().into(),
        directory: target.directory.to_string_lossy().into(),
        installed: next.clone(),
        previous: before.clone(),
        original,
    };
    let mut stage = tempfile::NamedTempFile::new_in(&target.directory)?;
    std::io::copy(&mut fs::File::open(source)?, &mut stage)?;
    stage.as_file().sync_all()?;
    ensure!(core::digest(stage.path())? == next, "NR 暂存文件校验失败");
    let mut marker = tempfile::NamedTempFile::new_in(&root)?;
    marker.write_all(&serde_json::to_vec_pretty(&r)?)?;
    marker.as_file().sync_all()?;
    core::assert_target_stopped(target)?;
    ensure!(
        match &before {
            Some(h) => matches_file(&dest, h)?,
            None => !dest.exists(),
        },
        "NR 文件在操作期间发生变化"
    );
    marker.persist(root.join("ownership.json"))?;
    // Journal precedes the replacement: interrupted operations remain removable.
    stage.persist(&dest)?;
    Ok(())
}

/// Also used by the normal patch uninstaller while its directory lock is held.
/// Standalone removal uses the same restoration journal as full uninstall.
pub fn remove(target: &core::DeploymentTarget) -> Result<()> {
    let target = target.validate(false)?;
    let _lock = win::game_lock(&target.directory)?;
    core::assert_target_stopped(&target)?;
    ensure!(
        read(&target)?.is_some(),
        "此 NR 文件不是管理器安装的，已保留"
    );
    remove_locked(&target)
}

pub fn remove_locked(target: &core::DeploymentTarget) -> Result<()> {
    let Some(r) = read(target)? else {
        return Ok(());
    };
    core::assert_target_stopped(target)?;
    let root = core::no_links(&target.directory.join(OWN))?;
    let dest = core::no_links(&target.directory.join(FILE))?;
    original_valid(&root, &r)?;
    ensure!(
        accepted(&dest, &r)?,
        "NR 文件已被其他程序修改，已保留；请先处理冲突"
    );
    if let Some(hash) = &r.original {
        if !matches_file(&dest, hash)? {
            let mut stage = tempfile::NamedTempFile::new_in(&target.directory)?;
            std::io::copy(&mut fs::File::open(root.join("original.dll"))?, &mut stage)?;
            stage.as_file().sync_all()?;
            ensure!(core::digest(stage.path())? == *hash, "NR 恢复文件校验失败");
            stage.persist(&dest)?;
        }
    } else if dest.exists() {
        fs::remove_file(&dest)?;
    }
    // Remove the marker first: interruption leaves a harmless retained backup.
    fs::remove_file(root.join("ownership.json"))?;
    if r.original.is_some() {
        fs::remove_file(root.join("original.dll"))?;
    }
    let _ = fs::remove_dir(root); // Unknown files are never removed recursively.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standalone_removal_restores_original_and_preserves_other_patch_files() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let exe = dir.path().join("game.exe");
        let mut pe = vec![0u8; 512];
        pe[..2].copy_from_slice(b"MZ");
        pe[60] = 128;
        pe[128..132].copy_from_slice(b"PE\0\0");
        pe[132..134].copy_from_slice(&0x8664u16.to_le_bytes());
        pe[148..150].copy_from_slice(&240u16.to_le_bytes());
        pe[150..152].copy_from_slice(&2u16.to_le_bytes());
        pe[152..154].copy_from_slice(&0x20bu16.to_le_bytes());
        fs::write(&exe, pe)?;
        let target = core::DeploymentTarget::for_game(&exe);
        fs::write(dir.path().join(FILE), b"original NR")?;
        fs::write(dir.path().join("version.dll"), b"Encore proxy")?;
        fs::write(dir.path().join("rtx-encore.jsonc"), b"settings remain")?;
        assert!(remove(&target).is_err()); // external installation is preserved
        let source = dir.path().join("source.dll");
        fs::write(&source, b"new NR")?;
        install_file_locked(&target, &source)?;
        remove(&target)?;
        assert_eq!(fs::read(dir.path().join(FILE))?, b"original NR");
        assert_eq!(fs::read(dir.path().join("version.dll"))?, b"Encore proxy");
        assert_eq!(
            fs::read(dir.path().join("rtx-encore.jsonc"))?,
            b"settings remain"
        );
        assert!(!dir.path().join(OWN).exists());
        Ok(())
    }
    #[test]
    fn display_status_tracks_external_managed_missing_and_invalid_ownership() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let target = core::DeploymentTarget::for_game(&dir.path().join("game.exe"));
        fs::write(&target.game_exe, b"fixture")?;
        assert_eq!(inspect(&target).label(), "NR 未安装");
        let dll = dir.path().join(FILE);
        fs::write(&dll, b"external")?;
        let external = inspect(&target);
        assert!(external.present && !external.managed && external.version.is_none());
        let source = dir.path().join("source.dll");
        fs::write(&source, b"managed")?;
        install_file_locked(&target, &source)?;
        let managed = inspect(&target);
        assert!(managed.present && managed.managed);
        assert_eq!(managed.path, dll);
        fs::remove_file(&dll)?;
        let missing = inspect(&target);
        assert!(!missing.present && missing.managed && missing.warning.is_some());
        fs::write(dir.path().join(OWN).join("ownership.json"), b"invalid")?;
        assert!(inspect(&target).warning.is_some());
        Ok(())
    }
    #[test]
    fn fresh_install_and_interrupted_upgrade_remain_removable() -> Result<()> {
        for existing in [false, true] {
            let dir = tempfile::tempdir()?;
            let exe = dir.path().join("game.exe");
            fs::write(&exe, b"fixture")?;
            let target = core::DeploymentTarget::for_game(&exe);
            let dest = dir.path().join(FILE);
            if existing {
                fs::write(&dest, b"original")?;
            }
            let source = dir.path().join("source.dll");
            fs::write(&source, b"first")?;
            install_file_locked(&target, &source)?;
            fs::write(&source, b"second")?;
            install_file_locked(&target, &source)?;
            // Simulate interruption after journaling but before replacement.
            fs::write(&dest, b"first")?;
            remove_locked(&target)?;
            if existing {
                assert_eq!(fs::read(&dest)?, b"original");
            } else {
                assert!(!dest.exists());
            }
            assert!(!dir.path().join(OWN).exists());
        }
        Ok(())
    }
    #[test]
    fn different_game_and_corrupt_backup_cannot_restore() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let exe = dir.path().join("game.exe");
        fs::write(&exe, b"fixture")?;
        let target = core::DeploymentTarget::for_game(&exe);
        let dest = dir.path().join(FILE);
        fs::write(&dest, b"original")?;
        let source = dir.path().join("source.dll");
        fs::write(&source, b"new")?;
        install_file_locked(&target, &source)?;
        let other = core::DeploymentTarget::for_game(&dir.path().join("other.exe"));
        assert!(remove_locked(&other).is_err());
        fs::write(dir.path().join(OWN).join("original.dll"), b"changed")?;
        assert!(remove_locked(&target).is_err());
        assert_eq!(fs::read(&dest)?, b"new");
        Ok(())
    }
    #[test]
    fn backup_restore_and_external_changes_are_preserved() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let exe = dir.path().join("game.exe");
        fs::write(&exe, b"fixture")?;
        let target = core::DeploymentTarget::for_game(&exe);
        let original = dir.path().join(FILE);
        fs::write(&original, b"original")?;
        let source = dir.path().join("source.dll");
        fs::write(&source, b"new")?;
        install_file_locked(&target, &source)?;
        assert_eq!(fs::read(&original)?, b"new");
        fs::write(&original, b"external")?;
        assert!(remove_locked(&target).is_err());
        assert_eq!(fs::read(&original)?, b"external");
        fs::write(&original, b"new")?;
        remove_locked(&target)?;
        assert_eq!(fs::read(&original)?, b"original");
        assert!(!dir.path().join(OWN).exists());
        Ok(())
    }
}
