pub const PUBLISHER: &str = "BF4DED3827EB26116EAE9E9ACACE0B0791D502BF";
use crate::{core, win};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
pub const REPO: &str = "pandaligx/RTX-FG-Manager";
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
            self.schema == 1 && version(&self.version)? == version(tag)?,
            "更新清单与版本不匹配"
        );
        ensure!(
            self.file
                == format!(
                    "RTXManager-v{}-x64.exe",
                    self.version.trim_start_matches('v')
                )
                && core::valid_hash(&self.sha256.to_lowercase()),
            "更新清单文件或摘要无效"
        );
        ensure!(
            (1024 * 1024..=256 * 1024 * 1024).contains(&self.bytes),
            "更新文件大小无效"
        );
        Ok(())
    }
    pub fn same_file(&self, other: &Self) -> bool {
        self.version == other.version
            && self.file == other.file
            && self.bytes == other.bytes
            && self.sha256.eq_ignore_ascii_case(&other.sha256)
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
    if source == "github" {
        let mut m: Manifest = serde_json::from_value(get_json_with_cancel(
            &format!("https://github.com/{REPO}/releases/latest/download/update.json"),
            cancel,
        )?)?;
        m.validate(&m.version)?;
        m.version = m.version.trim_start_matches('v').into();
        m.url = format!(
            "https://github.com/{REPO}/releases/download/v{}/{}",
            m.version, m.file
        );
        m.source = source.into();
        return Ok(m);
    }
    ensure!(source == "gitee", "更新来源无效");
    let base = format!("https://gitee.com/api/v5/repos/{REPO}");
    let r = get_json_with_cancel(&format!("{base}/releases/latest"), cancel)?;
    ensure!(
        !r["draft"].as_bool().unwrap_or(false) && !r["prerelease"].as_bool().unwrap_or(false),
        "没有可用的正式版本"
    );
    let tag = r["tag_name"].as_str().context("更新版本号无效")?;
    let id = r["id"]
        .as_u64()
        .filter(|v| *v > 0)
        .context("更新信息无效")?;
    let a = get_json_with_cancel(
        &format!("{base}/releases/{id}/attach_files?per_page=100"),
        cancel,
    )?;
    let a = a.as_array().context("更新附件列表无效")?;
    let asset = |name: &str| -> Result<String> {
        let found = a.iter().filter(|v| v["name"] == name).collect::<Vec<_>>();
        ensure!(found.len() == 1, "更新附件尚未同步完成");
        let s = found[0]["browser_download_url"]
            .as_str()
            .context("更新地址无效")?;
        safe_url(s)?;
        Ok(s.into())
    };
    let mut m: Manifest =
        serde_json::from_value(get_json_with_cancel(&asset("update.json")?, cancel)?)?;
    m.validate(tag)?;
    m.url = asset(&m.file)?;
    m.source = source.into();
    Ok(m)
}
pub fn check(choice: &str) -> Result<Option<Manifest>> {
    check_with_cancel(choice, &AtomicBool::new(false))
}
fn source_order(choice: &str) -> [&'static str; 2] {
    // An explicit GitHub preference is retained; auto/legacy settings now use
    // domestic first, independent of Windows region or VPN configuration.
    if choice == "github" {
        ["github", "gitee"]
    } else {
        ["gitee", "github"]
    }
}
pub fn check_with_cancel(choice: &str, cancel: &AtomicBool) -> Result<Option<Manifest>> {
    let mut errors = Vec::new();
    for source in source_order(choice) {
        ensure!(!cancel.load(Ordering::Relaxed), "检查更新已取消");
        match release_with_cancel(source, cancel) {
            Ok(m) => return Ok((version(&m.version)? > version(crate::VERSION)?).then_some(m)),
            Err(_) => errors.push(format!("{source}: 更新信息暂时不可用")),
        }
    }
    ensure!(!cancel.load(Ordering::Relaxed), "检查更新已取消");
    bail!("检查更新失败，请稍后重试。{}", errors.join("；"))
}
fn mirror_release(m: &Manifest, source: &str, cancel: &AtomicBool) -> Result<Manifest> {
    ensure!(["github", "gitee"].contains(&source), "更新来源无效");
    let base = format!(
        "https://{source}.com/{REPO}/releases/download/v{}",
        m.version
    );
    let other: Manifest = serde_json::from_value(get_json_with_cancel(
        &format!("{base}/update.json"),
        cancel,
    )?)?;
    matching_mirror(m, source, other)
}
fn matching_mirror(m: &Manifest, source: &str, mut other: Manifest) -> Result<Manifest> {
    ensure!(["github", "gitee"].contains(&source), "更新来源无效");
    other.validate(&m.version)?;
    ensure!(m.same_file(&other), "更新附件尚未同步完成");
    other.source = source.into();
    other.url = format!(
        "https://{source}.com/{REPO}/releases/download/v{}/{}",
        m.version, m.file
    );
    Ok(other)
}
pub fn verify(path: &Path, m: &Manifest) -> Result<()> {
    m.validate(&m.version)?;
    core::no_links(path)?;
    ensure!(
        fs::metadata(path)?.len() == m.bytes && core::digest(path)?.eq_ignore_ascii_case(&m.sha256),
        "更新文件大小或 SHA-256 不匹配"
    );
    core::pe64(path, false)?;
    let s = win::verify_signature(path)?;
    ensure!(
        version(&s.version)? == version(&m.version)?,
        "更新文件版本不匹配"
    );
    Ok(())
}
pub fn aria_options(text: &str) -> Vec<String> {
    let size_pattern = regex::Regex::new(r"^[1-9]\d?[KM]$").expect("fixed size pattern");
    let mut result: std::collections::BTreeMap<String, String> =
        std::collections::BTreeMap::from([
            ("max-connection-per-server".into(), "30".into()),
            ("split".into(), "30".into()),
        ]);
    for l in text.lines() {
        let Some((k, v)) = l.split_once('=') else {
            continue;
        };
        let (k, v) = (k.trim(), v.trim());
        let value = match k {
            "async-dns"
            | "disable-ipv6"
            | "enable-http2"
            | "enable-doh-http2"
            | "enable-http-pipelining"
                if ["true", "false"].contains(&v) =>
            {
                Some(v.into())
            }
            "max-connection-per-server" | "split" => {
                v.parse::<u32>().ok().map(|n| n.clamp(1, 30).to_string())
            }
            "min-split-size" | "piece-length" if size_pattern.is_match(v) => Some(v.into()),
            "async-dns-mode" if ["multi", "single"].contains(&v) => Some(v.into()),
            "uri-selector" if ["inorder", "feedback", "adaptive"].contains(&v) => Some(v.into()),
            "async-dns-server" => {
                let v = v
                    .split(',')
                    .map(|s| s.trim().parse::<std::net::IpAddr>())
                    .collect::<std::result::Result<Vec<_>, _>>();
                v.ok().filter(|v| !v.is_empty() && v.len() <= 8).map(|v| {
                    v.iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(",")
                })
            }
            _ => None,
        };
        if let Some(v) = value {
            result.insert(k.into(), v);
        }
    }
    // The bundled customized aria2 accepts 1..1024 connections (verified --help).
    // Keep its signed configuration unchanged; update-specific settings are explicit.
    for (key, value) in [
        ("async-dns", "false"),
        ("disable-ipv6", "false"),
        ("split", "30"),
        ("max-connection-per-server", "30"),
        ("min-split-size", "1M"),
    ] {
        result.insert(key.into(), value.into());
    }
    result
        .into_iter()
        .map(|(k, v)| format!("--{k}={v}"))
        .collect()
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
    let _cache_lock = crate::cache::operation_lock()?;
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
fn launch(path: &Path, args: &[&std::ffi::OsStr]) -> Result<()> {
    Command::new(path)
        .args(args)
        .current_dir(path.parent().context("无效路径")?)
        .creation_flags(0x08000000)
        .spawn()?;
    Ok(())
}
pub fn install(path: &Path, m: &Manifest, data: &Path) -> Result<()> {
    verify(path, m)?;
    let target = core::no_links(&std::env::current_exe()?)?;
    let s = win::verify_signature(&target)?;
    ensure!(
        version(&s.version)? < version(&m.version)?,
        "当前版本无需更新"
    );
    let destination = target.with_file_name(&m.file);
    ensure!(
        target
            .to_string_lossy()
            .eq_ignore_ascii_case(&destination.to_string_lossy())
            || !destination.exists(),
        "新版文件名已被占用，未替换任何文件"
    );
    let ticket = Ticket {
        schema: 1,
        old_sha256: core::digest(&target)?,
        target,
        manifest: m.clone(),
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
        a.to_string_lossy()
            .eq_ignore_ascii_case(&b.to_string_lossy())
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
    ensure!(
        exe == ticket.target.with_file_name(&ticket.manifest.file),
        "Unexpected updated executable"
    );
    verify(&exe, &ticket.manifest)?;
    core::atomic_json(
        &folder.join("ready.json"),
        &serde_json::json!({"pid":std::process::id(),"sha256":ticket.manifest.sha256}),
    )
}
pub fn apply(ticket: &Path) -> Result<()> {
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
    let destination = target.with_file_name(&t.manifest.file);
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
            let mut child = Command::new(&destination)
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

#[cfg(test)]
mod download_tests {
    use super::*;
    use crate::transfer::Readout;
    use std::io::Write;

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
        }
    }

    #[test]
    fn changed_preference_selects_domestic_without_rewriting_checked_identity() -> Result<()> {
        let checked = checked_github_manifest();
        let original = checked.clone();
        assert_eq!(source_order("github"), ["github", "gitee"]);
        // The check has already completed on GitHub. A later source change
        // (including a cancelled download's retry) must not use checked.source.
        for current_preference in ["domestic", "gitee", "auto"] {
            let [first, fallback] = source_order(current_preference);
            assert_eq!((first, fallback), ("gitee", "github"));
            let mut mirror_json = checked.clone();
            mirror_json.source.clear();
            mirror_json.url.clear();
            let active = matching_mirror(&checked, first, mirror_json)?;
            assert_eq!(active.source, "gitee");
            assert_eq!(
                active.url,
                format!(
                    "https://gitee.com/{REPO}/releases/download/v4.2.4/RTXManager-v4.2.4-x64.exe"
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
    fn readout_tracks_real_bytes_and_speed_with_fragmented_lines() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("console.log");
        fs::write(&path, b"old session should not be replayed\n")?;
        let mut readout = Readout::new(&path)?;
        let mut log = fs::OpenOptions::new().append(true).open(&path)?;
        // A sparse output can already be full length while only 1 MiB arrived.
        fs::File::create(temp.path().join("sparse.exe"))?.set_len(90524944)?;
        log.write_all(b"[#abcdef 1048576B/90524944B(1%) CN:30 DL:123")?;
        assert!(readout.poll(90524944)?.is_none());
        log.write_all(b"4567B ETA:1m]\r\n")?;
        assert_eq!(readout.poll(90524944)?, Some((1048576, 1234567, 30)));
        assert!(readout.poll(90524944)?.is_none());
        log.write_all(b"[#abcdef 100/90524944(0%) CN:1 DL:0]\n")?;
        assert_eq!(readout.poll(90524944)?, Some((100, 0, 1)));
        // Invalid totals, overflowing integers and diagnostics are not progress.
        for line in [
            "[#abcdef 999/1(100%) CN:1 DL:10]",
            "[#abcdef 1/2(50%) CN:1 DL:10]",
            "ERROR 123/90524944 DL:1",
            "[#abcdef 999999999999999999999999999999/90524944(0%) CN:1 DL:0]",
        ] {
            assert!(readout.parse(line, 90524944).is_none());
        }
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

    #[test]
    fn update_profile_uses_custom_aria30_without_untrusted_execution_options() {
        let options = aria_options(include_str!("../../app/tools/aria2.conf"));
        for expected in [
            "--async-dns=false",
            "--disable-ipv6=false",
            "--split=30",
            "--max-connection-per-server=30",
            "--min-split-size=1M",
        ] {
            assert!(options.iter().any(|s| s == expected), "{expected}");
        }
        assert!(
            !options
                .iter()
                .any(|s| s.contains("certificate") || s.contains("dot://"))
        );
    }
}
