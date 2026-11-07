//! Bounded in-process HTTPS downloads with shared concurrency and verified resumption.
//! Certificate checks, redirect policy, size limits and payload hashes remain mandatory.
mod native;
use crate::{assets, core};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug)]
pub enum UrlPolicy {
    /// Metadata and software updates hosted by the two official repositories.
    Official,
    /// Existing payload catalogs may also refer to the publisher's HTTPS CDN.
    Cloud,
    #[cfg(test)]
    TestLoopback,
}

pub fn validate_url(value: &str, policy: UrlPolicy) -> Result<reqwest::Url> {
    ensure!(
        value.len() <= 8192 && !value.chars().any(char::is_control),
        "下载地址无效"
    );
    let url = reqwest::Url::parse(value)?;
    #[cfg(test)]
    if matches!(policy, UrlPolicy::TestLoopback) {
        ensure!(
            url.scheme() == "http"
                && url.host_str() == Some("127.0.0.1")
                && url.username().is_empty()
                && url.password().is_none()
                && url.fragment().is_none(),
            "Invalid test URL"
        );
        return Ok(url);
    }
    ensure!(
        url.scheme() == "https"
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.fragment().is_none(),
        "下载地址必须使用 HTTPS"
    );
    if matches!(policy, UrlPolicy::Official) {
        let host = url.host_str().unwrap_or_default();
        ensure!(
            url.port().is_none_or(|p| p == 443)
                && ([
                    "github.com",
                    "api.github.com",
                    "raw.githubusercontent.com",
                    "objects.githubusercontent.com",
                    "release-assets.githubusercontent.com",
                    "gitee.com",
                    "api.gitee.com",
                    "gitee.cn",
                    "raw.giteeusercontent.com",
                    "gitcode.com",
                    "api.gitcode.com",
                    "raw.gitcode.com",
                    "file-cdn.gitcode.com",
                ]
                .contains(&host)
                    || host.ends_with(".gitee.com")),
            "下载地址不是官方发布站点"
        );
    }
    Ok(url)
}

#[derive(Clone, Debug)]
pub struct Source {
    pub name: String,
    pub url: String,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq)]
pub enum Phase {
    #[default]
    Connecting,
    Waiting,
    Downloading,
    Switching,
    Restarting,
    Verifying,
    Complete,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Progress {
    pub completed: u64,
    pub total: u64,
    pub bytes_per_second: u64,
    pub connections: u32,
    pub source: String,
    pub phase: Phase,
    #[serde(default)]
    pub detail: String,
}

impl Progress {
    pub fn percent(&self) -> f32 {
        if self.phase == Phase::Complete {
            return 100.;
        }
        if self.total == 0 {
            return 0.;
        }
        (100. * self.completed as f64 / self.total as f64).clamp(0., 99.9) as f32
    }
    pub fn label(&self) -> &'static str {
        match self.phase {
            Phase::Waiting => "正在等待其他下载任务完成…",
            Phase::Connecting => "正在连接下载服务器…",
            Phase::Downloading => "正在下载更新…",
            Phase::Switching => "正在切换下载线路…",
            Phase::Restarting => "正在使用单连接重新下载…",
            Phase::Verifying => "正在校验文件与数字签名…",
            Phase::Complete => "更新下载完成，已验证签名",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Request {
    pub sources: Vec<Source>,
    pub bytes: Option<u64>,
    pub sha256: Option<String>,
    pub limit: u64,
    pub metadata: bool,
    pub policy: UrlPolicy,
}

impl Request {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.sources.is_empty() && self.sources.len() <= 4 && self.limit > 0,
            "下载任务无效"
        );
        ensure!(
            self.bytes.is_none_or(|n| n > 0 && n <= self.limit),
            "下载文件大小无效"
        );
        ensure!(
            self.sha256.as_ref().is_none_or(|s| core::valid_hash(s)),
            "下载文件摘要无效"
        );
        ensure!(
            self.metadata || (self.bytes.is_some() && self.sha256.is_some()),
            "文件下载缺少完整性信息"
        );
        for source in &self.sources {
            validate_url(&source.url, self.policy)?;
            ensure!(
                !source.name.is_empty() && source.name.len() <= 64,
                "下载线路无效"
            );
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Failure {
    Cancelled,
    Certificate,
    Range,
    RateLimited,
    Integrity,
    Connection,
    Stalled,
    Size,
    Timeout,
    Unauthorized,
    NotFound,
    Disk,
    UnsafeUrl,
}

impl Failure {
    pub fn label(self) -> &'static str {
        match self {
            Self::Cancelled => "下载已取消",
            Self::Certificate => "服务器证书验证失败",
            Self::Range => "服务器不支持分段下载",
            Self::RateLimited => "服务器请求限流",
            Self::Integrity => "文件完整性校验失败",
            Self::Connection => "下载连接失败",
            Self::Stalled => "下载持续无有效进展或速度过低",
            Self::Size => "下载文件超过大小限制",
            Self::Timeout => "下载超时",
            Self::Unauthorized => "下载服务器需要授权",
            Self::NotFound => "下载文件不存在（HTTP 404）",
            Self::Disk => "无法写入下载文件，请检查磁盘空间和权限",
            Self::UnsafeUrl => "下载重定向地址不符合安全要求",
        }
    }
}

fn checked_response(error: reqwest::Error) -> anyhow::Error {
    let mut certificate_error = false;
    let mut cause: Option<&(dyn std::error::Error + 'static)> = Some(&error);
    while let Some(current) = cause {
        let text = current.to_string().to_ascii_lowercase();
        certificate_error |= text.contains("certificate") || text.contains("unknownissuer");
        cause = current.source();
    }
    let failure = if error.is_timeout() {
        Failure::Timeout
    } else if error.status().is_some_and(|s| s.as_u16() == 429) {
        Failure::RateLimited
    } else if error.status().is_some_and(|s| s.as_u16() == 404) {
        Failure::NotFound
    } else if error.status().is_some_and(|s| s.as_u16() == 416) {
        Failure::Range
    } else if error
        .status()
        .is_some_and(|s| matches!(s.as_u16(), 401 | 403))
    {
        Failure::Unauthorized
    } else if error.is_redirect() {
        Failure::UnsafeUrl
    } else if certificate_error {
        Failure::Certificate
    } else {
        Failure::Connection
    };
    failure.into()
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}
impl std::error::Error for Failure {}

fn check_cancel(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::Relaxed) {
        return Err(Failure::Cancelled.into());
    }
    Ok(())
}

fn control_path(path: &Path) -> Result<PathBuf> {
    let name = path
        .file_name()
        .context("下载文件名无效")?
        .to_string_lossy();
    core::no_links(&path.with_file_name(format!("{name}.aria2")))
}

pub(crate) fn remove_partial(path: &Path) -> Result<()> {
    for p in [
        core::no_links(path)?,
        control_path(path)?,
        native::checkpoint_path(path)?,
    ] {
        if p.exists() {
            fs::remove_file(p)?;
        }
    }
    Ok(())
}

pub fn verify_file(path: &Path, request: &Request) -> Result<()> {
    let path = core::no_links(path)?;
    let size = fs::metadata(&path)?.len();
    ensure!(size <= request.limit, Failure::Size);
    ensure!(request.bytes.is_none_or(|n| n == size), Failure::Integrity);
    if let Some(hash) = &request.sha256 {
        ensure!(
            core::digest(&path)?.eq_ignore_ascii_case(hash),
            Failure::Integrity
        );
    }
    Ok(())
}

/// Sequential preferred-source failover; a successful source returns without
/// contacting its fallback. Cancellation stops only this invocation's requests.
pub fn download(
    request: &Request,
    path: &Path,
    cancel: &AtomicBool,
    mut progress: impl FnMut(Progress),
) -> Result<PathBuf> {
    request.validate()?;
    check_cancel(cancel)?;
    let path = core::no_links(path)?;
    let parent = core::no_links(path.parent().context("下载目录无效")?)?;
    fs::create_dir_all(parent)?;
    // Protect a shared destination even when two windows request the same package.
    let _lock = crate::win::cache_lock_wait(&path, cancel, || {
        progress(Progress {
            phase: Phase::Waiting,
            ..Default::default()
        });
    })?;
    if !request.metadata && path.is_file() && verify_file(&path, request).is_ok() {
        for marker in [control_path(&path)?, native::checkpoint_path(&path)?] {
            if marker.exists() {
                fs::remove_file(marker)?;
            }
        }
        progress(Progress {
            completed: request.bytes.unwrap_or(0),
            total: request.bytes.unwrap_or(0),
            source: "cache".into(),
            phase: Phase::Complete,
            ..Default::default()
        });
        return Ok(path);
    }
    if path.exists()
        && (request.metadata
            || !native::checkpoint_path(&path)?.exists()
            || control_path(&path)?.exists())
    {
        remove_partial(&path)?;
    }
    let deadline = Instant::now() + Duration::from_secs(if request.metadata { 90 } else { 600 });
    let mut errors = Vec::new();
    for (index, source) in request.sources.iter().enumerate() {
        check_cancel(cancel)?;
        if index > 0 {
            progress(Progress {
                source: source.name.clone(),
                phase: Phase::Switching,
                detail: errors.last().cloned().unwrap_or_default(),
                ..Default::default()
            });
        }
        let mut sequential = false;
        for _ in 0..2 {
            check_cancel(cancel)?;
            if sequential || request.metadata {
                remove_partial(&path)?;
            }
            match native::attempt(
                request,
                source,
                &path,
                sequential,
                cancel,
                &mut progress,
                deadline,
            ) {
                Ok(()) => return Ok(path),
                Err(error) => {
                    check_cancel(cancel)?;
                    let failure = error.downcast_ref::<Failure>().copied();
                    let reason = failure
                        .map(Failure::label)
                        .unwrap_or("下载连接或地址检查失败");
                    errors.push(format!("{}: {reason}", source.name));
                    progress(Progress {
                        source: source.name.clone(),
                        detail: reason.into(),
                        phase: Phase::Switching,
                        ..Default::default()
                    });
                    if matches!(
                        failure,
                        Some(Failure::Range | Failure::Size | Failure::Integrity)
                    ) {
                        remove_partial(&path)?;
                        sequential = true;
                    }
                    if matches!(
                        failure,
                        Some(
                            Failure::Certificate
                                | Failure::UnsafeUrl
                                | Failure::Disk
                                | Failure::Unauthorized
                                | Failure::NotFound
                        )
                    ) || Instant::now() >= deadline
                    {
                        break;
                    }
                }
            }
        }
        if Instant::now() >= deadline {
            break;
        }
    }
    bail!("下载失败，已保留现有文件。{}", errors.join("；"))
}

pub fn fetch_metadata(
    sources: Vec<Source>,
    policy: UrlPolicy,
    cancel: &AtomicBool,
    progress: impl FnMut(Progress),
) -> Result<Vec<u8>> {
    check_cancel(cancel)?;
    let root = core::no_links(&assets::cache_root()?)?;
    fs::create_dir_all(&root)?;
    let temp = tempfile::Builder::new()
        .prefix("metadata-")
        .tempdir_in(root)?;
    let request = Request {
        sources,
        bytes: None,
        sha256: None,
        limit: 1024 * 1024,
        metadata: true,
        policy,
    };
    let path = download(
        &request,
        &temp.path().join("metadata.json"),
        cancel,
        progress,
    )?;
    check_cancel(cancel)?;
    Ok(fs::read(path)?)
}
