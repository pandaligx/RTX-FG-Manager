pub const PUBLISHER: &str = "BF4DED3827EB26116EAE9E9ACACE0B0791D502BF";
#[cfg(not(target_os = "linux"))]
use crate::platform::CommandExt;
use crate::{core, win};
#[cfg(not(target_os = "linux"))]
use anyhow::Context;
use anyhow::{Result, bail, ensure};
use serde::{Deserialize, Serialize};
#[cfg(not(target_os = "linux"))]
use std::time::{Duration, Instant};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};
#[cfg(target_os = "linux")]
#[path = "linux/updater.rs"]
mod linux;
pub const LINUX_SCHEMA: u32 = 3;
pub const LINUX_METADATA: &str = "update-linux.json";
const ACTIVE_SCHEMA: u32 = if cfg!(target_os = "linux") {
    LINUX_SCHEMA
} else {
    2
};
const ACTIVE_METADATA: &str = if cfg!(target_os = "linux") {
    LINUX_METADATA
} else {
    "update-v2.json"
};
pub const REPO: &str = "pandaligx/RTX-FG-Manager";
pub const GITCODE_REPO: &str = "gcw_qWIjmve7/RTX-FG-Manager";
fn release_base(source: &str, version: &str) -> Result<String> {
    let repo = match source {
        "gitcode" => GITCODE_REPO,
        "github" | "gitee" => REPO,
        _ => bail!("更新来源无效"),
    };
    self::version(version)?;
    Ok(format!(
        "https://{source}.com/{repo}/releases/download/v{}",
        version.trim_start_matches('v')
    ))
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Manifest {
    pub schema: u32,
    pub version: String,
    pub file: String,
    pub sha256: String,
    pub bytes: u64,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub notes: std::collections::BTreeMap<String, String>,
}
pub fn version(s: &str) -> Result<[u16; 3]> {
    let s = s.strip_prefix('v').unwrap_or(s);
    let v = s.split('.').collect::<Vec<_>>();
    ensure!(
        v.len() == 3
            && v.iter()
                .all(|s| !s.is_empty() && s.len() <= 4 && s.bytes().all(|c| c.is_ascii_digit())),
        "更新版本号无效"
    );
    Ok([v[0].parse()?, v[1].parse()?, v[2].parse()?])
}
impl Manifest {
    pub fn validate(&self, tag: &str) -> Result<()> {
        ensure!(
            matches!(self.schema, 1 | 2 | LINUX_SCHEMA) && version(&self.version)? == version(tag)?,
            "更新清单与版本不匹配"
        );
        ensure!(
            self.file
                == match self.schema {
                    1 => format!(
                        "RTXManager-v{}-x64.exe",
                        self.version.trim_start_matches('v')
                    ),
                    2 => "RTXManager.exe".into(),
                    LINUX_SCHEMA => format!(
                        "RTXManager-{}-x86_64.AppImage",
                        self.version.trim_start_matches('v')
                    ),
                    _ => unreachable!("schema checked above"),
                }
                && core::valid_hash(&self.sha256.to_lowercase()),
            "更新清单文件或摘要无效"
        );
        ensure!(
            (1024 * 1024..=256 * 1024 * 1024).contains(&self.bytes),
            "更新文件大小无效"
        );
        ensure!(
            self.notes.len() <= 8 && self.notes.values().all(|n| n.len() <= 16384),
            "更新日志过长"
        );
        Ok(())
    }
    pub fn same_file(&self, other: &Self) -> bool {
        self.schema == other.schema
            && self.version == other.version
            && self.file == other.file
            && self.bytes == other.bytes
            && self.sha256.eq_ignore_ascii_case(&other.sha256)
    }
    fn metadata_file(&self) -> &'static str {
        if self.schema == LINUX_SCHEMA {
            LINUX_METADATA
        } else if self.schema == 2 {
            "update-v2.json"
        } else {
            "update.json"
        }
    }
}
pub fn safe_url(s: &str) -> Result<reqwest::Url> {
    crate::transfer::validate_url(s, crate::transfer::UrlPolicy::Official)
}
pub fn get_json(url: &str) -> Result<serde_json::Value> {
    get_json_with_cancel(url, &AtomicBool::new(false))
}
fn get_json_with_cancel(url: &str, cancel: &AtomicBool) -> Result<serde_json::Value> {
    let bytes = crate::transfer::fetch_metadata(
        vec![crate::transfer::Source {
            name: "metadata".into(),
            url: safe_url(url)?.to_string(),
        }],
        crate::transfer::UrlPolicy::Official,
        cancel,
        |_| {},
    )?;
    Ok(serde_json::from_slice(
        bytes.strip_prefix(&[239, 187, 191]).unwrap_or(&bytes),
    )?)
}
pub fn release(source: &str) -> Result<Manifest> {
    release_with_cancel(source, &AtomicBool::new(false))
}
fn release_with_cancel(source: &str, cancel: &AtomicBool) -> Result<Manifest> {
    release_with_fetch(source, cancel, &mut get_json_with_cancel)
}
fn release_with_fetch(
    source: &str,
    cancel: &AtomicBool,
    fetch: &mut impl FnMut(&str, &AtomicBool) -> Result<serde_json::Value>,
) -> Result<Manifest> {
    // Public static files need neither an API quota nor an embedded account token.
    // Only the release publisher promotes these after verifying both sites.
    let urls = match source {
        "gitcode" => vec![format!(
            "https://api.gitcode.com/api/v5/repos/{GITCODE_REPO}/raw/{ACTIVE_METADATA}?ref=main"
        )],
        "gitee" => vec![format!(
            "https://gitee.com/{REPO}/raw/main/{ACTIVE_METADATA}"
        )],
        "github" => vec![
            format!("https://raw.githubusercontent.com/{REPO}/main/{ACTIVE_METADATA}"),
            format!("https://github.com/{REPO}/releases/latest/download/{ACTIVE_METADATA}"),
        ],
        _ => bail!("更新来源无效"),
    };
    let mut errors = Vec::new();
    for url in urls {
        ensure!(!cancel.load(Ordering::Relaxed), "检查更新已取消");
        let result = (|| -> Result<Manifest> {
            let mut m: Manifest = serde_json::from_value(fetch(&url, cancel)?)?;
            m.validate(&m.version)?;
            // Legacy update.json is permanently pinned to the bridge. Falling
            // back to it would falsely report "up to date" during a v2 outage.
            // Schema 1 is accepted only for incoming legacy install tickets.
            ensure!(m.schema == ACTIVE_SCHEMA, "更新清单不适用于当前系统");
            m.version = m.version.trim_start_matches('v').into();
            // Never accept a manifest-supplied EXE URL or source, even if the
            // JSON came from an official host. Construct the versioned route.
            m.url = format!("{}/{}", release_base(source, &m.version)?, m.file);
            m.source = source.into();
            Ok(m)
        })();
        ensure!(!cancel.load(Ordering::Relaxed), "检查更新已取消");
        match result {
            Ok(m) => return Ok(m),
            Err(error) => errors.push(error.to_string()),
        }
    }
    bail!("{}", errors.join("；"))
}
pub fn check(choice: &str) -> Result<Option<Manifest>> {
    check_with_cancel(choice, &AtomicBool::new(false))
}
fn source_order(choice: &str) -> [&'static str; 2] {
    // An explicit GitHub preference is retained; auto/legacy settings now use
    // domestic first, independent of Windows region or VPN configuration.
    if choice == "github" {
        ["github", "gitcode"]
    } else {
        ["gitcode", "github"]
    }
}
pub fn check_with_cancel(choice: &str, cancel: &AtomicBool) -> Result<Option<Manifest>> {
    check_with_fetch(choice, cancel, &mut get_json_with_cancel)
}
fn check_with_fetch(
    choice: &str,
    cancel: &AtomicBool,
    fetch: &mut impl FnMut(&str, &AtomicBool) -> Result<serde_json::Value>,
) -> Result<Option<Manifest>> {
    let mut errors = Vec::new();
    for source in source_order(choice) {
        ensure!(!cancel.load(Ordering::Relaxed), "检查更新已取消");
        match release_with_fetch(source, cancel, fetch) {
            Ok(m) => return Ok((version(&m.version)? > version(crate::VERSION)?).then_some(m)),
            Err(error) => errors.push(format!("{source}: {error}")),
        }
    }
    ensure!(!cancel.load(Ordering::Relaxed), "检查更新已取消");
    bail!("检查更新失败，请稍后重试。{}", errors.join("；"))
}
fn mirror_release(m: &Manifest, source: &str, cancel: &AtomicBool) -> Result<Manifest> {
    let base = release_base(source, &m.version)?;
    let other: Manifest = serde_json::from_value(get_json_with_cancel(
        &format!("{base}/{}", m.metadata_file()),
        cancel,
    )?)?;
    matching_mirror(m, source, other)
}
fn matching_mirror(m: &Manifest, source: &str, mut other: Manifest) -> Result<Manifest> {
    let base = release_base(source, &m.version)?;
    other.validate(&m.version)?;
    ensure!(m.same_file(&other), "更新附件尚未同步完成");
    other.source = source.into();
    other.url = format!("{base}/{}", m.file);
    Ok(other)
}
pub fn verify(path: &Path, m: &Manifest) -> Result<()> {
    m.validate(&m.version)?;
    core::no_links(path)?;
    ensure!(
        fs::metadata(path)?.len() == m.bytes && core::digest(path)?.eq_ignore_ascii_case(&m.sha256),
        "更新文件大小或 SHA-256 不匹配"
    );
    #[cfg(target_os = "linux")]
    {
        ensure!(m.schema == LINUX_SCHEMA, "Linux 更新不能使用 Windows 程序");
        return verify_appimage(path);
    }
    #[cfg(not(target_os = "linux"))]
    {
        ensure!(m.schema != LINUX_SCHEMA, "Windows 更新不能使用 Linux 程序");
        core::pe64(path, false)?;
        let s = win::verify_signature(path)?;
        ensure!(
            version(&s.version)? == version(&m.version)?,
            "更新文件版本不匹配"
        );
        Ok(())
    }
}
/// Validate the type-2 x86_64 AppImage envelope after its full SHA-256 check.
/// This is format validation, not a Windows Authenticode signature.
pub fn verify_appimage(path: &Path) -> Result<()> {
    use std::io::Read;
    let mut header = [0u8; 64];
    fs::File::open(core::no_links(path)?)?.read_exact(&mut header)?;
    ensure!(
        &header[..6] == b"\x7fELF\x02\x01"
            && &header[8..11] == b"AI\x02"
            && u16::from_le_bytes([header[18], header[19]]) == 62,
        "更新文件不是 x86_64 Type 2 AppImage"
    );
    Ok(())
}
pub use crate::transfer::{Phase as DownloadPhase, Progress as DownloadProgress};

pub fn download(
    m: &Manifest,
    data: &Path,
    cancel: &AtomicBool,
    progress: impl FnMut(DownloadProgress),
) -> Result<PathBuf> {
    download_with_preference(m, data, &m.source, cancel, progress)
}
/// Apply the current preference to each new download attempt, even when the
/// checked manifest came from another source. A mirror must identify the same
/// version, size and hash before any executable is downloaded from it.
pub fn download_with_preference(
    m: &Manifest,
    data: &Path,
    preference: &str,
    cancel: &AtomicBool,
    mut progress: impl FnMut(DownloadProgress),
) -> Result<PathBuf> {
    m.validate(&m.version)?;
    safe_url(&m.url)?;
    let _cache_lock = crate::cache::wait_for_operation(cancel, || {
        progress(DownloadProgress {
            total: m.bytes,
            phase: DownloadPhase::Waiting,
            ..Default::default()
        });
    })?;
    ensure!(!cancel.load(Ordering::Relaxed), "更新下载已取消");
    let folder = core::no_links(&data.join("updates").join(&m.sha256[..32]))?;
    fs::create_dir_all(&folder)?;
    let _lock = win::game_lock(&folder)?;
    let path = core::no_links(&folder.join(&m.file))?;
    if path.exists() && verify(&path, m).is_ok() {
        progress(DownloadProgress {
            completed: m.bytes,
            total: m.bytes,
            source: "cache".into(),
            phase: DownloadPhase::Complete,
            ..Default::default()
        });
        return Ok(path);
    }
    let mut last_progress = DownloadProgress {
        total: m.bytes,
        ..Default::default()
    };
    let mut errors = Vec::new();
    for (index, source) in source_order(preference).into_iter().enumerate() {
        ensure!(!cancel.load(Ordering::Relaxed), "更新下载已取消");
        if index > 0 {
            progress(DownloadProgress {
                phase: DownloadPhase::Switching,
                source: source.into(),
                detail: "首选线路下载失败，正在核对备用文件".into(),
                ..last_progress.clone()
            });
        }
        let active = if source == m.source {
            m.clone()
        } else {
            if index == 0 {
                progress(DownloadProgress {
                    source: source.into(),
                    phase: DownloadPhase::Connecting,
                    ..last_progress.clone()
                });
            }
            match mirror_release(m, source, cancel) {
                Ok(other) => other,
                Err(_) => {
                    errors.push(format!("{source}: 更新附件尚未同步完成"));
                    continue;
                }
            }
        };
        let request = crate::transfer::Request {
            sources: vec![crate::transfer::Source {
                name: active.source.clone(),
                url: active.url,
            }],
            bytes: Some(m.bytes),
            sha256: Some(m.sha256.to_lowercase()),
            limit: 256 * 1024 * 1024,
            metadata: false,
            policy: crate::transfer::UrlPolicy::Official,
        };
        let result = crate::transfer::download(&request, &path, cancel, |mut p| {
            if let Some(file) = &mut p.file {
                file.name = m.file.clone();
            }
            // A valid transport checksum is not an Authenticode verification.
            if p.phase == DownloadPhase::Complete {
                p.phase = DownloadPhase::Verifying;
            }
            last_progress = p.clone();
            progress(p);
        });
        match result {
            Ok(_) => {
                ensure!(!cancel.load(Ordering::Relaxed), "更新下载已取消");
                verify(&path, m)?;
                ensure!(!cancel.load(Ordering::Relaxed), "更新下载已取消");
                progress(DownloadProgress {
                    completed: m.bytes,
                    total: m.bytes,
                    bytes_per_second: 0,
                    phase: DownloadPhase::Complete,
                    ..last_progress
                });
                return Ok(path);
            }
            Err(e) => errors.push(e.to_string()),
        }
    }
    ensure!(!cancel.load(Ordering::Relaxed), "更新下载已取消");
    bail!("下载失败，当前程序未修改。{}", errors.join("；"))
}
#[derive(Serialize, Deserialize)]
pub struct Ticket {
    pub schema: u32,
    pub target: PathBuf,
    pub old_sha256: String,
    pub manifest: Manifest,
    pub parent: u32,
    pub data_dir: PathBuf,
}
#[cfg(not(target_os = "linux"))]
fn launch(path: &Path, args: &[&std::ffi::OsStr]) -> Result<()> {
    crate::executable_name::command(path)
        .args(args)
        .current_dir(path.parent().context("无效路径")?)
        .creation_flags(0x08000000)
        .spawn()?;
    Ok(())
}
pub fn install(path: &Path, m: &Manifest, data: &Path) -> Result<()> {
    #[cfg(target_os = "linux")]
    return linux::install(path, m, data);
    #[cfg(not(target_os = "linux"))]
    {
        ensure!(
            cfg!(windows),
            "Linux AppImage 请手动更新，不使用 Windows 更新通道"
        );
        verify(path, m)?;
        let target = core::no_links(&std::env::current_exe()?)?;
        let s = win::verify_signature(&target)?;
        ensure!(
            version(&s.version)? < version(&m.version)?,
            "当前版本无需更新"
        );
        // Display-only notes must not enlarge the small, backward-compatible install ticket.
        let mut install_manifest = m.clone();
        install_manifest.notes.clear();
        let ticket = Ticket {
            schema: 1,
            old_sha256: core::digest(&target)?,
            target,
            manifest: install_manifest,
            parent: std::process::id(),
            data_dir: core::no_links(data)?,
        };
        // Download cache survives cancellation/retry; each installation gets a fresh
        // private ticket directory so a previous failed attempt cannot block retry.
        let folder = core::no_links(
            &data
                .join("updates")
                .join(uuid::Uuid::new_v4().simple().to_string()),
        )?;
        fs::create_dir(&folder)?;
        let executable = folder.join(&m.file);
        let mut input = fs::File::open(path)?;
        let mut output = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&executable)?;
        std::io::copy(&mut input, &mut output)?;
        output.sync_all()?;
        drop(output);
        verify(&executable, m)?;
        let out = folder.join("install.json");
        core::write_new(&out, &serde_json::to_vec(&ticket)?)?;
        launch(&executable, &["--apply-update".as_ref(), out.as_os_str()])
    }
}
pub fn replace_and_launch(
    target: &Path,
    stage: &Path,
    backup: &Path,
    old: &str,
    launch: impl FnOnce() -> Result<()>,
) -> Result<()> {
    replace_named_and_launch(target, target, stage, backup, old, launch)
}
pub fn replace_named_and_launch(
    target: &Path,
    destination: &Path,
    stage: &Path,
    backup: &Path,
    old: &str,
    launch: impl FnOnce() -> Result<()>,
) -> Result<()> {
    for p in [target, destination, stage, backup] {
        core::no_links(p)?;
    }
    let same_path = |a: &Path, b: &Path| {
        if cfg!(target_os = "linux") {
            a == b
        } else {
            a.to_string_lossy()
                .eq_ignore_ascii_case(&b.to_string_lossy())
        }
    };
    ensure!(
        target.parent() == destination.parent()
            && target.parent() == stage.parent()
            && target.parent() == backup.parent()
            && !same_path(stage, target)
            && !same_path(stage, destination)
            && !same_path(backup, target)
            && !same_path(backup, destination)
            && !same_path(backup, stage),
        "更新文件路径冲突"
    );
    ensure!(
        same_path(target, destination) || !destination.exists(),
        "新版文件名已被占用，未替换任何文件"
    );
    ensure!(
        !backup.exists() && core::digest(target)? == old,
        "更新目标已变化，已停止替换"
    );
    let new_hash = core::digest(stage)?;
    win::rename_no_replace(target, backup)?;
    let mut placed = false;
    let result = (|| {
        win::rename_no_replace(stage, destination)?;
        placed = true;
        launch()
    })();
    if result.is_err() {
        if placed && destination.exists() {
            ensure!(
                core::digest(destination)? == new_hash,
                "回滚目标已变化，备份已保留"
            );
            fs::remove_file(destination)?;
        }
        win::rename_no_replace(backup, target)?;
    } else {
        // Commit only after the supplied launcher acknowledged a usable new UI.
        // Delete exactly the old file we moved, never scan adjacent executables.
        ensure!(core::digest(backup)? == old, "旧版文件已变化，未删除");
        fs::remove_file(backup)?;
    }
    result
}

/// Called once after the new manager has drawn frames, with disk work off the UI thread.
pub fn signal_ready(data: &Path) -> Result<()> {
    #[cfg(target_os = "linux")]
    return linux::signal_ready(data);
    #[cfg(not(target_os = "linux"))]
    {
        crate::executable_name::signal_ready(data)?;
        let Ok(token) = std::env::var("RTXFG_UPDATE_READY") else {
            return Ok(());
        };
        ensure!(
            token.len() == 32 && token.bytes().all(|b| b.is_ascii_hexdigit()),
            "Invalid update readiness token"
        );
        let folder = core::no_links(&data.join("updates").join(token))?;
        let ticket: Ticket =
            serde_json::from_value(core::read_json(&folder.join("install.json"), 16384)?)?;
        let exe = core::no_links(&std::env::current_exe()?)?;
        ensure!(exe == ticket.target, "Unexpected updated executable");
        verify(&exe, &ticket.manifest)?;
        core::atomic_json(
            &folder.join("ready.json"),
            &serde_json::json!({"pid":std::process::id(),"sha256":ticket.manifest.sha256}),
        )
    }
}
pub fn apply(ticket: &Path) -> Result<()> {
    #[cfg(target_os = "linux")]
    return linux::apply(ticket);
    #[cfg(not(target_os = "linux"))]
    {
        let _cache_lock = crate::cache::operation_lock()?;
        let ticket = core::no_links(ticket)?;
        ensure!(
            ticket.file_name().is_some_and(|s| s == "install.json"),
            "更新安装记录无效"
        );
        let t: Ticket = serde_json::from_value(core::read_json(&ticket, 16384)?)?;
        ensure!(
            t.schema == 1
                && t.parent > 0
                && t.parent != std::process::id()
                && core::valid_hash(&t.old_sha256),
            "更新安装记录无效"
        );
        let folder = ticket.parent().context("无效路径")?;
        let token = folder.file_name().context("无效路径")?.to_string_lossy();
        ensure!(
            token.len() == 32
                && token.bytes().all(|c| c.is_ascii_hexdigit())
                && folder == core::no_links(&t.data_dir.join("updates").join(token.as_ref()))?,
            "更新暂存路径无效"
        );
        t.manifest.validate(crate::VERSION)?;
        let source = core::no_links(&std::env::current_exe()?)?;
        ensure!(source == folder.join(&t.manifest.file), "更新程序路径无效");
        verify(&source, &t.manifest)?;
        let target = core::no_links(&t.target)?;
        ensure!(
            target
                .extension()
                .is_some_and(|s| s.eq_ignore_ascii_case("exe"))
                && version(&win::verify_signature(&target)?.version)? < version(crate::VERSION)?,
            "更新目标无效"
        );
        win::wait_process(t.parent, 120000)?;
        ensure!(
            core::digest(&target)? == t.old_sha256,
            "更新目标已变化，已停止替换"
        );
        let name = target.file_name().context("无效路径")?.to_string_lossy();
        // Retain the user's filename so shortcuts and pinned launchers keep working.
        // The manifest filename identifies the verified download, not its destination.
        let destination = target.clone();
        let stage = target.with_file_name(format!("{name}.{token}.new"));
        let backup = target.with_file_name(format!("{name}.{token}.previous"));
        ensure!(!stage.exists() && !backup.exists(), "更新备份路径冲突");
        let mut src = fs::File::open(&source)?;
        let mut dst = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&stage)?;
        std::io::copy(&mut src, &mut dst)?;
        dst.sync_all()?;
        drop(dst);
        verify(&stage, &t.manifest)?;
        replace_named_and_launch(
            &target,
            &destination,
            &stage,
            &backup,
            &t.old_sha256,
            || {
                let mut child = crate::executable_name::command(&destination)
                    .args(["--data-dir".as_ref(), t.data_dir.as_os_str()])
                    .env("RTXFG_UPDATE_READY", token.as_ref())
                    .current_dir(destination.parent().context("无效路径")?)
                    .creation_flags(0x08000000)
                    .spawn()?;
                let start = Instant::now();
                loop {
                    if let Ok(ready) = core::read_json(&folder.join("ready.json"), 1024)
                        && ready["pid"] == child.id()
                        && ready["sha256"] == t.manifest.sha256
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
            },
        )?;
        core::atomic_json(
            &folder.join("installed.json"),
            &serde_json::json!({"version":crate::VERSION,"backup":backup,"target":destination,"previous_target":target}),
        )
    }
}

#[cfg(test)]
mod download_tests {
    use super::*;

    fn checked_github_manifest() -> Manifest {
        Manifest {
            schema: 1,
            version: "4.2.4".into(),
            file: "RTXManager-v4.2.4-x64.exe".into(),
            sha256: "a".repeat(64),
            bytes: 32 * 1024 * 1024,
            source: "github".into(),
            url: format!(
                "https://github.com/{REPO}/releases/download/v4.2.4/RTXManager-v4.2.4-x64.exe"
            ),
            notes: Default::default(),
        }
    }

    fn fixed_manifest() -> Manifest {
        Manifest {
            schema: ACTIVE_SCHEMA,
            version: crate::VERSION.into(),
            file: if cfg!(target_os = "linux") {
                format!("RTXManager-{}-x86_64.AppImage", crate::VERSION)
            } else {
                "RTXManager.exe".into()
            },
            ..checked_github_manifest()
        }
    }

    #[test]
    fn domestic_check_uses_anonymous_gitcode_file_without_github() -> Result<()> {
        let cancel = AtomicBool::new(false);
        let mut calls = Vec::new();
        let mut supplied = fixed_manifest();
        supplied.source = "untrusted".into();
        supplied.url = "https://example.invalid/untrusted.exe".into();
        let mut fetch = |url: &str, _: &AtomicBool| {
            calls.push(url.to_owned());
            Ok(serde_json::to_value(&supplied)?)
        };
        let actual = release_with_fetch("gitcode", &cancel, &mut fetch)?;
        assert_eq!(actual.source, "gitcode");
        assert_eq!(
            actual.url,
            format!(
                "https://gitcode.com/{GITCODE_REPO}/releases/download/v{}/{}",
                crate::VERSION,
                supplied.file
            )
        );
        // An older published manifest still counts as a successful check, and
        // must not fall through to the inaccessible overseas service.
        assert!(check_with_fetch("auto", &cancel, &mut fetch)?.is_none());
        assert_eq!(
            calls,
            vec![
                format!(
                    "https://api.gitcode.com/api/v5/repos/{GITCODE_REPO}/raw/{ACTIVE_METADATA}?ref=main"
                );
                2
            ]
        );
        Ok(())
    }

    #[test]
    fn failed_or_malformed_domestic_metadata_falls_back_to_github() -> Result<()> {
        for malformed in [false, true] {
            let cancel = AtomicBool::new(false);
            let mut calls = Vec::new();
            let mut m = fixed_manifest();
            m.version = "9999.0.0".into();
            if m.schema == LINUX_SCHEMA {
                m.file = "RTXManager-9999.0.0-x86_64.AppImage".into();
            }
            let mut fetch = |url: &str, _: &AtomicBool| {
                calls.push(url.to_owned());
                if url.starts_with("https://api.gitcode.com/") {
                    if malformed {
                        return Ok(serde_json::json!({"message": "403 Forbidden"}));
                    }
                    bail!("HTTP 403 Forbidden (Rate Limit Exceeded)");
                }
                Ok(serde_json::to_value(&m)?)
            };
            let result = check_with_fetch("domestic", &cancel, &mut fetch)?.unwrap();
            assert_eq!(result.source, "github");
            assert_eq!(calls.len(), 2);
            assert!(calls[0].contains("api.gitcode.com"));
            assert!(calls[1].contains("raw.githubusercontent.com"));
            assert!(!calls.iter().any(|u| u.contains("access_token")));
        }
        Ok(())
    }

    #[test]
    fn github_release_asset_remains_a_fallback() -> Result<()> {
        let mut calls = Vec::new();
        let mut fetch = |url: &str, _: &AtomicBool| {
            calls.push(url.to_owned());
            if url.contains("raw.githubusercontent.com") {
                bail!("TLS connection failed");
            }
            Ok(serde_json::to_value(fixed_manifest())?)
        };
        release_with_fetch("github", &AtomicBool::new(false), &mut fetch)?;
        assert_eq!(calls.len(), 2);
        assert!(calls[1].ends_with(&format!("/releases/latest/download/{ACTIVE_METADATA}")));
        Ok(())
    }

    #[test]
    fn legacy_bridge_is_accepted_for_install_but_never_hides_a_new_update() -> Result<()> {
        let legacy = checked_github_manifest();
        legacy.validate(&legacy.version)?;
        let mut calls = Vec::new();
        let mut fetch = |url: &str, _: &AtomicBool| {
            calls.push(url.to_owned());
            Ok(serde_json::to_value(&legacy)?)
        };
        assert!(check_with_fetch("auto", &AtomicBool::new(false), &mut fetch).is_err());
        assert!(calls.iter().all(|url| url.contains(ACTIVE_METADATA)));
        // Both supported install formats use the old ticket envelope, so the
        // signed bridge installer can be launched by an unmodified old client.
        for manifest in [legacy, fixed_manifest()] {
            let json = serde_json::json!({"schema":1,"target":"game/manager.exe",
                "old_sha256":"b".repeat(64),"manifest":manifest,"parent":123,
                "data_dir":"data"});
            let ticket: Ticket = serde_json::from_value(json)?;
            assert_eq!(ticket.schema, 1);
            ticket.manifest.validate(&ticket.manifest.version)?;
        }
        Ok(())
    }

    #[test]
    fn fixed_filename_keeps_version_digest_and_schema_bound_to_the_mirror() -> Result<()> {
        let current = fixed_manifest();
        current.validate(&current.version)?;
        assert_eq!(current.metadata_file(), "update-v2.json");
        assert_eq!(checked_github_manifest().metadata_file(), "update.json");
        let other = matching_mirror(&current, "gitcode", current.clone())?;
        assert!(
            other
                .url
                .ends_with(&format!("/v{}/RTXManager.exe", crate::VERSION))
        );
        for change in ["schema", "version", "hash", "bytes", "file"] {
            let mut other = current.clone();
            match change {
                "schema" => other.schema = 1,
                "version" => other.version = "9999.0.0".into(),
                "hash" => other.sha256 = "b".repeat(64),
                "bytes" => other.bytes += 1,
                "file" => other.file = "../RTXManager.exe".into(),
                _ => unreachable!(),
            }
            assert!(
                matching_mirror(&current, "gitcode", other).is_err(),
                "{change}"
            );
        }
        for schema in [0, 1, 3, u32::MAX] {
            let mut invalid = current.clone();
            invalid.schema = schema;
            assert!(invalid.validate(&invalid.version).is_err());
        }
        Ok(())
    }

    #[test]
    fn failed_check_never_reports_up_to_date_and_cancellation_stops_fallback() {
        let cancel = AtomicBool::new(false);
        let mut calls = 0;
        let mut failed = |_: &str, _: &AtomicBool| -> Result<serde_json::Value> {
            calls += 1;
            bail!("network unavailable");
        };
        assert!(check_with_fetch("auto", &cancel, &mut failed).is_err());
        assert_eq!(calls, 3);
        let mut calls = 0;
        let mut cancelled = |_: &str, flag: &AtomicBool| -> Result<serde_json::Value> {
            calls += 1;
            flag.store(true, Ordering::Relaxed);
            bail!("cancelled");
        };
        assert!(check_with_fetch("auto", &cancel, &mut cancelled).is_err());
        assert!(check_with_fetch("auto", &cancel, &mut cancelled).is_err());
        assert_eq!(calls, 1);
    }

    #[test]
    fn unsafe_static_manifest_is_rejected_before_offering_an_update() {
        let mut m = checked_github_manifest();
        m.file = "../untrusted.exe".into();
        let mut fetch = |_: &str, _: &AtomicBool| Ok(serde_json::to_value(&m).unwrap());
        assert!(check_with_fetch("auto", &AtomicBool::new(false), &mut fetch).is_err());
        m.file = "RTXManager-v4.2.4-x64.exe".into();
        m.sha256 = "invalid".into();
        let mut fetch = |_: &str, _: &AtomicBool| Ok(serde_json::to_value(&m).unwrap());
        assert!(check_with_fetch("auto", &AtomicBool::new(false), &mut fetch).is_err());
    }

    #[test]
    fn changed_preference_selects_domestic_without_rewriting_checked_identity() -> Result<()> {
        let checked = checked_github_manifest();
        let original = checked.clone();
        assert_eq!(source_order("github"), ["github", "gitcode"]);
        // The check has already completed on GitHub. A later source change
        // (including a cancelled download's retry) must not use checked.source.
        for current_preference in ["domestic", "gitee", "gitcode", "auto"] {
            let [first, fallback] = source_order(current_preference);
            assert_eq!((first, fallback), ("gitcode", "github"));
            let mut mirror_json = checked.clone();
            mirror_json.source.clear();
            mirror_json.url.clear();
            let active = matching_mirror(&checked, first, mirror_json)?;
            assert_eq!(active.source, "gitcode");
            assert_eq!(
                active.url,
                format!(
                    "https://gitcode.com/{GITCODE_REPO}/releases/download/v4.2.4/RTXManager-v4.2.4-x64.exe"
                )
            );
            assert!(checked.same_file(&active));
            assert_eq!(checked, original); // Existing verified cache identity is unchanged.
        }
        assert_eq!(source_order("github")[0], "github");
        Ok(())
    }

    #[test]
    fn preferred_mirror_must_match_checked_version_size_and_digest() -> Result<()> {
        let checked = checked_github_manifest();
        let mut other = checked.clone();
        other.sha256 = "b".repeat(64);
        assert!(matching_mirror(&checked, "gitee", other).is_err());
        let mut other = checked.clone();
        other.bytes += 1;
        assert!(matching_mirror(&checked, "gitee", other).is_err());
        let mut other = checked.clone();
        other.version = "4.2.5".into();
        other.file = "RTXManager-v4.2.5-x64.exe".into();
        assert!(matching_mirror(&checked, "gitee", other).is_err());
        let mut other = checked.clone();
        other.sha256.make_ascii_uppercase();
        assert!(checked.same_file(&matching_mirror(&checked, "gitee", other)?));
        Ok(())
    }

    #[test]
    fn completion_waits_for_integrity_and_signature_verification() {
        let mut p = DownloadProgress {
            completed: 100,
            total: 100,
            phase: DownloadPhase::Verifying,
            ..Default::default()
        };
        assert!(p.percent() < 100.);
        p.phase = DownloadPhase::Complete;
        assert_eq!(p.percent(), 100.);
        p = DownloadProgress::default();
        assert_eq!(p.percent(), 0.);
    }
}
