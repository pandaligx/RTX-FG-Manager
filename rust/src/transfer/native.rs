//! In-process HTTPS transfers; every request shares a process-wide connection budget.
use super::{Failure, Progress, Request, Source, check_cancel, checked_response, verify_file};
use crate::core;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    future::Future,
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::{sync::Semaphore, task::JoinSet};

const CONNECTIONS: usize = 30;
// A single package must leave connection capacity for startup metadata and other jobs.
const PER_FILE: usize = 8;
const ATTEMPTS: usize = 3;
const BLOCK: u64 = if cfg!(test) {
    64 * 1024
} else {
    4 * 1024 * 1024
};
const POLL: Duration = Duration::from_millis(100);

fn budget() -> &'static Arc<Semaphore> {
    static BUDGET: OnceLock<Arc<Semaphore>> = OnceLock::new();
    BUDGET.get_or_init(|| Arc::new(Semaphore::new(CONNECTIONS)))
}

pub(super) fn checkpoint_path(path: &Path) -> Result<PathBuf> {
    let name = path
        .file_name()
        .context("下载文件名无效")?
        .to_string_lossy();
    core::no_links(&path.with_file_name(format!("{name}.rtx-part.json")))
}

#[derive(Debug, Serialize, Deserialize)]
struct Checkpoint {
    schema: u8,
    bytes: u64,
    sha256: String,
    block: u64,
    complete: Vec<bool>,
}

impl Checkpoint {
    fn fresh(request: &Request) -> Result<Self> {
        let bytes = request.bytes.context("分段下载缺少文件大小")?;
        ensure!(bytes.div_ceil(BLOCK) <= 16384, Failure::Size);
        Ok(Self {
            schema: 1,
            bytes,
            sha256: request.sha256.clone().context("分段下载缺少文件摘要")?,
            block: BLOCK,
            complete: vec![false; bytes.div_ceil(BLOCK) as usize],
        })
    }

    fn range(&self, index: usize) -> (u64, u64) {
        let start = index as u64 * BLOCK;
        (start, (start + BLOCK).min(self.bytes) - 1)
    }

    fn completed_bytes(&self) -> u64 {
        self.complete
            .iter()
            .enumerate()
            .filter(|(_, done)| **done)
            .map(|(i, _)| {
                let (start, end) = self.range(i);
                end - start + 1
            })
            .sum()
    }

    fn load(path: &Path, request: &Request) -> Result<Self> {
        let fresh = Self::fresh(request)?;
        let marker = checkpoint_path(path)?;
        if path.is_file() && fs::metadata(path)?.len() == fresh.bytes && marker.is_file() {
            let mut data = Vec::new();
            fs::File::open(&marker)?
                .take(128 * 1024)
                .read_to_end(&mut data)?;
            if let Ok(old) = serde_json::from_slice::<Self>(&data)
                && old.schema == 1
                && old.bytes == fresh.bytes
                && old.block == BLOCK
                && old.sha256.eq_ignore_ascii_case(&fresh.sha256)
                && old.complete.len() == fresh.complete.len()
            {
                return Ok(old);
            }
        }
        // The marker describes committed blocks, never inferred from a sparse file's length.
        let file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(core::no_links(path)?)?;
        file.set_len(fresh.bytes)?;
        file.sync_all()?;
        core::atomic_json(&marker, &fresh)?;
        Ok(fresh)
    }
}

fn client(policy: super::UrlPolicy) -> Result<reqwest::Client> {
    let builder = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .read_timeout(Duration::from_secs(15))
        .user_agent("RTXFGManager-transfer/2")
        .redirect(reqwest::redirect::Policy::custom(move |next| {
            if next.previous().len() >= 8
                || super::validate_url(next.url().as_str(), policy).is_err()
            {
                next.error("Unsafe download redirect")
            } else {
                next.follow()
            }
        }));
    #[cfg(test)]
    let builder = if matches!(policy, super::UrlPolicy::TestLoopback) {
        builder.no_proxy()
    } else {
        builder
    };
    Ok(builder.build()?)
}

fn content_range(
    value: Option<&reqwest::header::HeaderValue>,
    start: u64,
    end: u64,
    total: u64,
) -> Result<()> {
    let expected = format!("bytes {start}-{end}/{total}");
    ensure!(
        value.and_then(|s| s.to_str().ok()) == Some(expected.as_str()),
        Failure::Range
    );
    Ok(())
}

fn encoding(response: &reqwest::Response) -> Result<()> {
    ensure!(
        response
            .headers()
            .get(reqwest::header::CONTENT_ENCODING)
            .is_none_or(|v| v.as_bytes().eq_ignore_ascii_case(b"identity")),
        Failure::Integrity
    );
    Ok(())
}

#[derive(Debug)]
struct RetryAfter(Duration);
impl std::fmt::Display for RetryAfter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("服务器要求稍后重试")
    }
}
impl std::error::Error for RetryAfter {}

async fn retry<T, F, Fut>(mut operation: F) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T>>,
{
    for attempt in 0..ATTEMPTS {
        match operation().await {
            Ok(value) => return Ok(value),
            Err(error) => {
                let transient = matches!(
                    error.downcast_ref::<Failure>(),
                    Some(Failure::Connection | Failure::Timeout | Failure::RateLimited)
                );
                if !transient || attempt + 1 == ATTEMPTS {
                    return Err(error);
                }
                let delay = error
                    .downcast_ref::<RetryAfter>()
                    .map(|hint| hint.0)
                    .unwrap_or(Duration::from_millis(250 << attempt));
                tokio::time::sleep(delay).await;
            }
        }
    }
    unreachable!("the last retry always returns")
}

async fn send(
    client: &reqwest::Client,
    url: &reqwest::Url,
    range: Option<(u64, u64)>,
) -> Result<reqwest::Response> {
    let mut request = client
        .get(url.clone())
        .header(reqwest::header::ACCEPT_ENCODING, "identity");
    if let Some((start, end)) = range {
        request = request.header(reqwest::header::RANGE, format!("bytes={start}-{end}"));
    }
    let response = request.send().await.map_err(checked_response)?;
    if response.status().as_u16() == 429 || response.status().as_u16() == 503 {
        // Keep server back-off bounded by the enclosing deadline and cancellable monitor.
        let delay = response
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok())
            .map(|n| Duration::from_secs(n.clamp(1, 10)))
            .unwrap_or(Duration::from_secs(1));
        return Err(anyhow::Error::new(RetryAfter(delay)).context(Failure::RateLimited));
    }
    response.error_for_status().map_err(checked_response)
}

#[derive(Default)]
struct Counters {
    received: AtomicU64,
    network_bytes: AtomicU64,
    active: AtomicUsize,
}
struct Active(Arc<Counters>);
impl Active {
    fn new(counters: &Arc<Counters>) -> Self {
        counters.active.fetch_add(1, Ordering::Relaxed);
        Self(counters.clone())
    }
}
impl Drop for Active {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::Relaxed);
    }
}

struct Written<'a> {
    counters: &'a Counters,
    bytes: u64,
    complete: bool,
}
impl Drop for Written<'_> {
    fn drop(&mut self) {
        if !self.complete {
            // A failed/retried block has not been committed. Do not count it twice.
            self.counters
                .received
                .fetch_sub(self.bytes, Ordering::Relaxed);
        }
    }
}

async fn write_body(
    mut response: reqwest::Response,
    mut file: fs::File,
    limit: u64,
    exact: Option<u64>,
    counters: &Arc<Counters>,
) -> Result<u64> {
    encoding(&response)?;
    if let Some(size) = response.content_length() {
        ensure!(
            size <= limit && exact.is_none_or(|n| n == size),
            Failure::Size
        );
    }
    let mut written = Written {
        counters,
        bytes: 0,
        complete: false,
    };
    while let Some(chunk) = response.chunk().await.map_err(checked_response)? {
        let next = written
            .bytes
            .checked_add(chunk.len() as u64)
            .context(Failure::Size)?;
        ensure!(next <= limit, Failure::Size);
        file.write_all(&chunk).map_err(|_| Failure::Disk)?;
        written.bytes = next;
        counters
            .received
            .fetch_add(chunk.len() as u64, Ordering::Relaxed);
        counters
            .network_bytes
            .fetch_add(chunk.len() as u64, Ordering::Relaxed);
    }
    ensure!(exact.is_none_or(|n| n == written.bytes), Failure::Integrity);
    file.sync_all().map_err(|_| Failure::Disk)?;
    written.complete = true;
    Ok(written.bytes)
}

struct Segment {
    client: reqwest::Client,
    url: reqwest::Url,
    path: PathBuf,
    index: usize,
    start: u64,
    end: u64,
    total: u64,
    counters: Arc<Counters>,
}

impl Segment {
    async fn download(self) -> Result<usize> {
        retry(|| self.once()).await
    }

    async fn once(&self) -> Result<usize> {
        let _permit = budget().acquire().await.context("下载队列已关闭")?;
        let _active = Active::new(&self.counters);
        let response = send(&self.client, &self.url, Some((self.start, self.end))).await?;
        ensure!(
            response.status() == reqwest::StatusCode::PARTIAL_CONTENT,
            Failure::Range
        );
        content_range(
            response.headers().get(reqwest::header::CONTENT_RANGE),
            self.start,
            self.end,
            self.total,
        )?;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .open(core::no_links(&self.path)?)
            .map_err(|_| Failure::Disk)?;
        file.seek(SeekFrom::Start(self.start))
            .map_err(|_| Failure::Disk)?;
        let count = self.end - self.start + 1;
        write_body(response, file, count, Some(count), &self.counters).await?;
        Ok(self.index)
    }
}

async fn transfer(
    request: &Request,
    source: &Source,
    path: &Path,
    sequential: bool,
    counters: Arc<Counters>,
) -> Result<()> {
    let client = client(request.policy)?;
    let url = super::validate_url(&source.url, request.policy)?;
    let ranged = !sequential && !request.metadata && request.bytes.is_some_and(|n| n > BLOCK);
    let (mut response, permit, active) = retry(|| async {
        let permit = budget().acquire().await.context("下载队列已关闭")?;
        let active = Active::new(&counters);
        let response = send(&client, &url, ranged.then_some((0, 0))).await?;
        Ok((response, permit, active))
    })
    .await?;
    encoding(&response)?;
    if ranged && response.status() == reqwest::StatusCode::PARTIAL_CONTENT {
        let mut checkpoint = Checkpoint::load(path, request)?;
        content_range(
            response.headers().get(reqwest::header::CONTENT_RANGE),
            0,
            0,
            checkpoint.bytes,
        )?;
        // Drain the one-byte probe so the connection can be reused without buffering a full file.
        let mut probe_bytes = 0;
        while let Some(chunk) = response.chunk().await.map_err(checked_response)? {
            probe_bytes += chunk.len();
            ensure!(probe_bytes <= 1, Failure::Range);
        }
        ensure!(probe_bytes == 1, Failure::Range);
        // The actual GET and each redirect are checked; no separate unchecked downloader follows.
        let url = response.url().clone();
        drop(response);
        drop(active);
        drop(permit);
        counters
            .received
            .store(checkpoint.completed_bytes(), Ordering::Relaxed);
        let mut pending = checkpoint
            .complete
            .iter()
            .enumerate()
            .filter_map(|(i, done)| (!done).then_some(i))
            .collect::<std::collections::VecDeque<_>>();
        let mut tasks = JoinSet::new();
        loop {
            while tasks.len() < PER_FILE {
                let Some(index) = pending.pop_front() else {
                    break;
                };
                let (start, end) = checkpoint.range(index);
                tasks.spawn(
                    Segment {
                        client: client.clone(),
                        url: url.clone(),
                        path: path.to_owned(),
                        index,
                        start,
                        end,
                        total: checkpoint.bytes,
                        counters: counters.clone(),
                    }
                    .download(),
                );
            }
            let Some(result) = tasks.join_next().await else {
                break;
            };
            let index = result.context("下载任务异常退出")??;
            checkpoint.complete[index] = true;
            core::atomic_json(&checkpoint_path(path)?, &checkpoint).map_err(|_| Failure::Disk)?;
        }
    } else {
        ensure!(
            response.status() == reqwest::StatusCode::OK,
            Failure::Connection
        );
        let marker = checkpoint_path(path)?;
        if marker.exists() {
            fs::remove_file(marker)?;
        }
        let file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(core::no_links(path)?)
            .map_err(|_| Failure::Disk)?;
        write_body(
            response,
            file,
            request.bytes.unwrap_or(request.limit),
            request.bytes,
            &counters,
        )
        .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;

async fn monitor<T>(
    future: impl Future<Output = Result<T>>,
    cancel: &AtomicBool,
    deadline: Instant,
    mut tick: impl FnMut() -> Result<()>,
) -> Result<T> {
    let mut future = std::pin::pin!(future);
    loop {
        check_cancel(cancel)?;
        ensure!(Instant::now() < deadline, Failure::Timeout);
        match tokio::time::timeout(POLL, &mut future).await {
            Ok(result) => {
                check_cancel(cancel)?;
                return result;
            }
            Err(_) => tick()?,
        }
    }
}

pub(super) fn attempt(
    request: &Request,
    source: &Source,
    path: &Path,
    sequential: bool,
    cancel: &AtomicBool,
    progress: &mut impl FnMut(Progress),
    deadline: Instant,
) -> Result<()> {
    use super::Phase;
    let counters = Arc::new(Counters::default());
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
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let mut sample = (Instant::now(), 0u64);
    let mut stalled = (Instant::now(), 0u64);
    let deadline = if request.metadata {
        deadline.min(Instant::now() + Duration::from_secs(20))
    } else {
        deadline
    };
    runtime.block_on(monitor(
        transfer(request, source, path, sequential, counters.clone()),
        cancel,
        deadline,
        || {
            let received = counters.received.load(Ordering::Relaxed);
            status.completed = received;
            status.connections = counters.active.load(Ordering::Relaxed) as u32;
            status.phase = if status.connections == 0 && received == 0 {
                Phase::Waiting
            } else {
                Phase::Downloading
            };
            if sample.0.elapsed() >= Duration::from_millis(500) {
                let downloaded = counters.network_bytes.load(Ordering::Relaxed);
                status.bytes_per_second = (downloaded.saturating_sub(sample.1) as f64
                    / sample.0.elapsed().as_secs_f64())
                    as u64;
                sample = (Instant::now(), downloaded);
            }
            if counters.active.load(Ordering::Relaxed) == 0 || received != stalled.1 {
                stalled = (Instant::now(), received);
            }
            ensure!(
                stalled.0.elapsed() < Duration::from_secs(30),
                Failure::Stalled
            );
            progress(status.clone());
            Ok(())
        },
    ))?;
    status.phase = Phase::Verifying;
    status.bytes_per_second = 0;
    status.connections = 0;
    progress(status.clone());
    verify_file(path, request)?;
    check_cancel(cancel)?;
    let marker = checkpoint_path(path)?;
    if marker.exists() {
        fs::remove_file(marker)?;
    }
    status.completed = fs::metadata(path)?.len();
    status.total = status.completed;
    status.phase = Phase::Complete;
    progress(status);
    Ok(())
}
