//! Shared bounded aria2 transfers. HTTP headers are inspected before starting
//! aria2; response bodies are always downloaded by the bundled downloader.
//!
//! The stock aria2 CLI has no per-redirect policy callback. The preflight checks
//! every redirect it observes, but cannot bind a later server response to that
//! response. Hashes/signatures remain mandatory for executable payloads; HTTPS
//! certificate verification is never disabled.
use crate::{assets, core};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Read, Seek, SeekFrom},
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug)]
pub enum UrlPolicy {
    /// Metadata and software updates hosted by the two official repositories.
    Official,
    /// Existing payload catalogs may also refer to the publisher's HTTPS CDN.
    Cloud,
}

pub fn validate_url(value: &str, policy: UrlPolicy) -> Result<reqwest::Url> {
    ensure!(
        value.len() <= 8192 && !value.chars().any(char::is_control),
        "下载地址无效"
    );
    let url = reqwest::Url::parse(value)?;
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
            Phase::Connecting => "正在连接下载服务器…",
            Phase::Downloading => "正在下载更新…",
            Phase::Switching => "正在切换下载线路…",
            Phase::Restarting => "正在使用单连接重新下载…",
            Phase::Verifying => "正在校验文件与数字签名…",
            Phase::Complete => "更新下载完成，已验证签名",
        }
    }
    /// Only aria2's completed byte count is progress; file length can describe
    /// sparse, unwritten ranges and must never be used as downloaded bytes.
    pub fn parse_aria_line(line: &str, expected: u64) -> Option<(u64, u64, u32)> {
        static PATTERN: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
        let pattern = PATTERN.get_or_init(|| {
            regex::Regex::new(r"\[#[0-9a-f]+ (\d+)B?/(\d+)B?\([^)]*\) CN:(\d+) DL:(\d+)B?(?: |\])")
                .expect("fixed aria2 progress expression")
        });
        let c = pattern.captures(line)?;
        let completed = c[1].parse::<u64>().ok()?;
        let total = c[2].parse::<u64>().ok()?;
        ((expected == 0 || total == expected) && completed <= total).then_some((
            completed,
            c[4].parse().ok()?,
            c[3].parse().ok()?,
        ))
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
            Self::Disk => "无法写入下载文件，请检查磁盘空间和权限",
            Self::UnsafeUrl => "下载重定向地址不符合安全要求",
        }
    }
    pub fn from_aria_log(code: Option<i32>, text: &str) -> Self {
        if code == Some(8)
            || text.contains("Invalid range header")
            || text.contains("errorCode=8 ")
            || text.contains("status=416")
            || text.contains("416 Requested Range")
        {
            Self::Range
        } else if text.contains("status=429") || text.contains("429 Too Many") {
            Self::RateLimited
        } else if text.contains("certificate verification")
            || text.contains("certificate verify")
            || text.contains("Certificate verification")
        {
            Self::Certificate
        } else if code == Some(32) || text.contains("Checksum error") {
            Self::Integrity
        } else if code == Some(2) {
            Self::Timeout
        } else if code == Some(24) {
            Self::Unauthorized
        } else if matches!(code, Some(9 | 14..=18)) {
            Self::Disk
        } else if code == Some(23) {
            Self::UnsafeUrl
        } else if code == Some(5) {
            Self::Stalled
        } else {
            Self::Connection
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

/// This is a headers-only preflight, not a second file downloader. Do not claim
/// that an aria2 request made afterwards is cryptographically bound to it.
fn preflight(url: &str, policy: UrlPolicy, cancel: &AtomicBool) -> Result<reqwest::Url> {
    check_cancel(cancel)?;
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(10))
        .user_agent("RTXFGManager-transfer/1")
        .redirect(reqwest::redirect::Policy::custom(move |a| {
            if a.previous().len() >= 8 || validate_url(a.url().as_str(), policy).is_err() {
                a.error("Unsafe download redirect")
            } else {
                a.follow()
            }
        }))
        .build()?;
    let initial = validate_url(url, policy)?;
    let mut response = client
        .head(initial.clone())
        .send()
        .map_err(checked_response)?;
    // Some public CDNs reject HEAD while allowing ordinary anonymous GET. Do
    // not consume its body; aria2 receives the final checked URL below.
    if matches!(response.status().as_u16(), 403 | 405 | 501) {
        response = client.get(initial).send().map_err(checked_response)?;
    }
    check_cancel(cancel)?;
    let response = response.error_for_status().map_err(checked_response)?;
    validate_url(response.url().as_str(), policy)
}

pub(crate) struct Readout {
    file: fs::File,
    pending: Vec<u8>,
}
impl Readout {
    pub(crate) fn new(path: &Path) -> Result<Self> {
        let mut file = fs::File::open(path)?;
        file.seek(SeekFrom::End(0))?;
        Ok(Self {
            file,
            pending: Vec::new(),
        })
    }
    pub(crate) fn parse(&self, line: &str, expected: u64) -> Option<(u64, u64, u32)> {
        Progress::parse_aria_line(line, expected)
    }
    pub(crate) fn poll(&mut self, expected: u64) -> Result<Option<(u64, u64, u32)>> {
        let mut buffer = [0; 16384];
        let n = self.file.read(&mut buffer)?;
        self.pending.extend_from_slice(&buffer[..n]);
        let mut result = None;
        if let Some(last) = self
            .pending
            .iter()
            .rposition(|b| matches!(b, b'\r' | b'\n'))
        {
            for line in self.pending[..=last].split(|b| matches!(b, b'\r' | b'\n')) {
                if let Some(value) = self.parse(&String::from_utf8_lossy(line), expected) {
                    result = Some(value);
                }
            }
            self.pending.drain(..=last);
        }
        if self.pending.len() > 32768 {
            self.pending.clear();
        }
        Ok(result)
    }
}

struct DownloadChild(Child);
impl Drop for DownloadChild {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

fn control_path(path: &Path) -> Result<PathBuf> {
    let name = path
        .file_name()
        .context("下载文件名无效")?
        .to_string_lossy();
    core::no_links(&path.with_file_name(format!("{name}.aria2")))
}

fn remove_partial(path: &Path) -> Result<()> {
    for p in [core::no_links(path)?, control_path(path)?] {
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

fn attempt(
    request: &Request,
    source: &Source,
    path: &Path,
    sequential: bool,
    cancel: &AtomicBool,
    progress: &mut impl FnMut(Progress),
    deadline: Instant,
) -> Result<()> {
    check_cancel(cancel)?;
    let mut status = Progress {
        total: request.bytes.unwrap_or(0),
        source: source.name.clone(),
        phase: if sequential {
            Phase::Restarting
        } else {
            Phase::Connecting
        },
        ..Default::default()
    };
    progress(status.clone());
    let url = preflight(&source.url, request.policy, cancel)?;
    check_cancel(cancel)?;
    ensure!(Instant::now() < deadline, Failure::Timeout);
    let tool = assets::tool()?;
    let parent = path.parent().context("下载目录无效")?;
    let mut log = tempfile::NamedTempFile::new_in(parent)?;
    let mut readout = Readout::new(log.path())?;
    let mut command = Command::new(tool.clone());
    command
        .args(crate::updater::aria_options(&fs::read_to_string(
            tool.with_file_name("aria2.conf"),
        )?))
        .args([
            "--no-conf=true",
            "--enable-rpc=false",
            "--enable-dht=false",
            "--enable-dht6=false",
            "--enable-peer-exchange=false",
            "--follow-torrent=false",
            "--follow-metalink=false",
            "--check-certificate=true",
            "--allow-overwrite=false",
            "--auto-file-renaming=false",
            "--continue=true",
            "--file-allocation=none",
            "--connect-timeout=8",
            "--timeout=15",
            "--max-tries=1",
            "--retry-wait=1",
            "--summary-interval=1",
            "--show-console-readout=true",
            "--human-readable=false",
            "--enable-color=false",
            "--auto-save-interval=1",
            "--no-netrc=true",
            "--console-log-level=warn",
            "--download-result=hide",
        ]);
    if sequential || request.metadata {
        command.args([
            "--split=1",
            "--max-connection-per-server=1",
            "--continue=false",
        ]);
    }
    if let Some(hash) = &request.sha256 {
        command.arg(format!("--checksum=sha-256={hash}"));
    }
    // process::exit / Windows termination may end worker threads without
    // unwinding Drop. The downloader also watches this manager's PID so it
    // cannot remain as an orphan after the application exits unexpectedly.
    command.arg(format!("--stop-with-process={}", std::process::id()));
    let mut child = DownloadChild(
        command
            .arg(format!("--dir={}", parent.display()))
            .arg(format!(
                "--out={}",
                path.file_name()
                    .context("下载文件名无效")?
                    .to_string_lossy()
            ))
            .arg(url.as_str())
            .stdin(Stdio::null())
            .stdout(log.as_file().try_clone()?)
            .stderr(log.as_file().try_clone()?)
            .creation_flags(0x08000000)
            .spawn()?,
    );
    let started = Instant::now();
    let mut last_sample = started;
    let mut sample_window = (started, 0u64);
    let outcome = loop {
        check_cancel(cancel)?;
        ensure!(Instant::now() < deadline, Failure::Timeout);
        if let Ok(meta) = fs::metadata(path) {
            ensure!(
                meta.len() <= request.limit && request.bytes.is_none_or(|n| meta.len() <= n),
                Failure::Size
            );
        }
        if let Some((completed, speed, connections)) = readout.poll(request.bytes.unwrap_or(0))? {
            ensure!(completed <= request.limit, Failure::Size);
            status.completed = completed;
            status.bytes_per_second = speed;
            status.connections = connections;
            status.phase = Phase::Downloading;
            last_sample = Instant::now();
        } else if last_sample.elapsed() > Duration::from_secs(3) {
            status.bytes_per_second = 0;
        }
        progress(status.clone());
        if let Some(exit) = child.0.try_wait()? {
            break exit;
        }
        if request.metadata && started.elapsed() > Duration::from_secs(20) {
            return Err(Failure::Timeout.into());
        }
        if !request.metadata && sample_window.0.elapsed() >= Duration::from_secs(30) {
            let received = status.completed.saturating_sub(sample_window.1);
            if status.completed < status.total
                && received as f64 / sample_window.0.elapsed().as_secs_f64() < 256. * 1024.
            {
                return Err(Failure::Stalled.into());
            }
            sample_window = (Instant::now(), status.completed);
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    drop(child);
    check_cancel(cancel)?;
    if !outcome.success() {
        log.as_file_mut().seek(SeekFrom::Start(0))?;
        let mut bytes = Vec::new();
        log.as_file_mut()
            .take(1024 * 1024)
            .read_to_end(&mut bytes)?;
        // Never display raw downloader output: signed CDN URLs can contain tokens.
        return Err(
            Failure::from_aria_log(outcome.code(), &String::from_utf8_lossy(&bytes)).into(),
        );
    }
    status.phase = Phase::Verifying;
    status.bytes_per_second = 0;
    progress(status.clone());
    verify_file(path, request)?;
    check_cancel(cancel)?;
    status.completed = fs::metadata(path)?.len();
    status.total = status.completed;
    status.phase = Phase::Complete;
    progress(status);
    Ok(())
}

/// Sequential preferred-source failover; a successful source returns without
/// contacting its fallback. Only this invocation's child is stopped on cancel.
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
    if !request.metadata && path.is_file() && verify_file(&path, request).is_ok() {
        progress(Progress {
            completed: request.bytes.unwrap_or(0),
            total: request.bytes.unwrap_or(0),
            source: "cache".into(),
            phase: Phase::Complete,
            ..Default::default()
        });
        return Ok(path);
    }
    if path.exists() && (request.metadata || !control_path(&path)?.exists()) {
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
        for sequential in [false, true] {
            check_cancel(cancel)?;
            if sequential || request.metadata {
                remove_partial(&path)?;
            }
            match attempt(
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
                    if matches!(failure, Some(Failure::Size | Failure::Integrity)) {
                        remove_partial(&path)?;
                    }
                    if matches!(
                        failure,
                        Some(
                            Failure::Certificate
                                | Failure::UnsafeUrl
                                | Failure::Disk
                                | Failure::Unauthorized
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
