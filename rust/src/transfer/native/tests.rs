use super::*;
use crate::transfer::{self, UrlPolicy};
use std::{
    io::{BufRead, BufReader},
    net::{TcpListener, TcpStream},
    sync::Mutex,
    thread,
};

#[derive(Clone, Copy)]
enum Mode {
    Range,
    Ignore,
    BadRange,
    Corrupt,
    Missing,
    Slow,
    Redirect,
    Flaky,
    TruncatedOnce,
    Hanging,
    BlockFails,
}

type RequestedRange = Option<(u64, u64)>;
type Requests = Arc<Mutex<Vec<RequestedRange>>>;

#[test]
fn queued_files_follow_real_network_and_verified_cache_completion() -> Result<()> {
    let a = Server::new(Mode::Range, 16384);
    let b = Server::new(Mode::Range, 32768);
    let temp = tempfile::tempdir()?;
    let files = [&a, &b]
        .into_iter()
        .map(|server| {
            transfer::DownloadFile::new(
                &core::hash(&server.body),
                server.body.len() as u64,
                "runtime.zip".into(),
            )
        })
        .collect::<Vec<_>>();
    let mut task = crate::task_progress::Task::new("Fixture".into());
    task.download(String::new(), transfer::Progress::plan(files));
    let cancel = AtomicBool::new(false);
    transfer::download(&a.request(), &temp.path().join("a.zip"), &cancel, |p| {
        task.download(String::new(), p)
    })?;
    assert_eq!(task.completed_files(), 1);
    assert_eq!(task.files[1].label(), "等待");
    let requests = a.requests.lock().unwrap().len();
    transfer::download(&a.request(), &temp.path().join("a.zip"), &cancel, |p| {
        task.download(String::new(), p)
    })?;
    assert_eq!(
        a.requests.lock().unwrap().len(),
        requests,
        "Verified cache must not redownload"
    );
    assert_eq!(task.completed_files(), 1);
    transfer::download(&b.request(), &temp.path().join("b.zip"), &cancel, |p| {
        task.download(String::new(), p)
    })?;
    assert_eq!(task.completed_files(), 2);
    assert!(!task.is_downloading());
    Ok(())
}

struct Server {
    url: String,
    body: Arc<Vec<u8>>,
    requests: Requests,
    peak: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl Server {
    fn new(mode: Mode, size: usize) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/file", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let body = Arc::new((0..size).map(|n| (n % 251) as u8).collect::<Vec<_>>());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let peak = Arc::new(AtomicUsize::new(0));
        let active = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let worker = {
            let (body, requests, peak, stop) =
                (body.clone(), requests.clone(), peak.clone(), stop.clone());
            thread::spawn(move || {
                let mut workers = Vec::new();
                while !stop.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((socket, _)) => {
                            let (body, requests, peak, active, stop) = (
                                body.clone(),
                                requests.clone(),
                                peak.clone(),
                                active.clone(),
                                stop.clone(),
                            );
                            workers.push(thread::spawn(move || {
                                let count = active.fetch_add(1, Ordering::Relaxed) + 1;
                                peak.fetch_max(count, Ordering::Relaxed);
                                // Cancellation deliberately closes sockets while the fixture writes.
                                let _ = respond(socket, &body, mode, &requests, &stop);
                                active.fetch_sub(1, Ordering::Relaxed);
                            }));
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(2))
                        }
                        Err(error) => panic!("test listener: {error}"),
                    }
                }
                for worker in workers {
                    worker.join().unwrap();
                }
            })
        };
        Self {
            url,
            body,
            requests,
            peak,
            stop,
            thread: Some(worker),
        }
    }

    fn request(&self) -> Request {
        Request {
            sources: vec![Source {
                name: "test".into(),
                url: self.url.clone(),
            }],
            bytes: Some(self.body.len() as u64),
            sha256: Some(core::hash(&self.body)),
            limit: self.body.len() as u64,
            metadata: false,
            policy: UrlPolicy::TestLoopback,
        }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.thread.take().unwrap().join().unwrap();
    }
}

fn respond(
    mut socket: TcpStream,
    body: &[u8],
    mode: Mode,
    requests: &Mutex<Vec<Option<(u64, u64)>>>,
    stop: &AtomicBool,
) -> std::io::Result<()> {
    // Windows accepts inherit the listener's nonblocking mode; this fixture
    // uses blocking readers, unlike the production async HTTP client.
    socket.set_nonblocking(false)?;
    socket.set_nodelay(true)?;
    socket.set_read_timeout(Some(Duration::from_secs(15)))?;
    socket.set_write_timeout(Some(Duration::from_secs(5)))?;
    let mut reader = BufReader::new(socket.try_clone()?);
    let mut range = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 || line == "\r\n" {
            break;
        }
        if let Some(value) = line
            .to_ascii_lowercase()
            .trim()
            .strip_prefix("range: bytes=")
        {
            let (start, end) = value.split_once('-').unwrap();
            range = Some((start.parse::<u64>().unwrap(), end.parse::<u64>().unwrap()));
        }
    }
    let first = {
        let mut requests = requests.lock().unwrap();
        let first = !requests.contains(&range);
        requests.push(range);
        first
    };
    if matches!(mode, Mode::Flaky) && first && range != Some((0, 0)) {
        return socket.write_all(
            b"HTTP/1.1 500 Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        );
    }
    if matches!(mode, Mode::BlockFails) && range == Some((0, BLOCK - 1)) {
        return socket.write_all(
            b"HTTP/1.1 500 Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        );
    }
    if matches!(mode, Mode::Missing) {
        return socket.write_all(
            b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        );
    }
    if matches!(mode, Mode::Redirect) {
        return socket.write_all(b"HTTP/1.1 302 Found\r\nLocation: http://localhost:1/forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    }
    if matches!(mode, Mode::Ignore) {
        range = None;
    }
    let (start, end) = range.unwrap_or((0, body.len() as u64 - 1));
    let count = end - start + 1;
    if range.is_some() {
        let claimed_start = if matches!(mode, Mode::BadRange) {
            start + 1
        } else {
            start
        };
        write!(
            socket,
            "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {claimed_start}-{end}/{}\r\nContent-Length: {count}\r\nConnection: close\r\n\r\n",
            body.len()
        )?;
    } else {
        write!(
            socket,
            "HTTP/1.1 200 OK\r\nContent-Length: {count}\r\nConnection: close\r\n\r\n"
        )?;
    }
    if matches!(mode, Mode::Hanging) {
        while !stop.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_millis(2));
        }
        return Ok(());
    }
    for (i, chunk) in body[start as usize..=end as usize].chunks(4096).enumerate() {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        if matches!(mode, Mode::Slow) {
            thread::sleep(Duration::from_millis(15));
        }
        if matches!(mode, Mode::TruncatedOnce) && first && i == 1 {
            break;
        }
        if matches!(mode, Mode::Corrupt) {
            socket.write_all(&vec![0; chunk.len()])?;
        } else {
            socket.write_all(chunk)?;
        }
    }
    Ok(())
}

#[test]
fn range_download_completes_without_sequential_fallback() -> Result<()> {
    let server = Server::new(Mode::Slow, BLOCK as usize * 4 + 13);
    let temp = tempfile::tempdir()?;
    let request = server.request();
    attempt(
        &request,
        &request.sources[0],
        &temp.path().join("range.zip"),
        false,
        &AtomicBool::new(false),
        &mut |_| {},
        Instant::now() + Duration::from_secs(30),
    )?;
    let requests = server.requests.lock().unwrap();
    assert!(requests.len() > 2 && requests.iter().all(Option::is_some));
    Ok(())
}

#[test]
fn concurrent_files_share_connection_limit_and_verify_bytes() -> Result<()> {
    let server = Server::new(Mode::Slow, BLOCK as usize * 35 + 31);
    let temp = tempfile::tempdir()?;
    thread::scope(|scope| -> Result<()> {
        let mut workers = Vec::new();
        for i in 0..4 {
            let request = server.request();
            let path = temp.path().join(format!("{i}.zip"));
            workers.push(scope.spawn(move || {
                transfer::download(&request, &path, &AtomicBool::new(false), |_| {})
            }));
        }
        for worker in workers {
            let path = worker.join().unwrap()?;
            assert_eq!(fs::read(path)?, *server.body);
        }
        Ok(())
    })?;
    let peak = server.peak.load(Ordering::Relaxed);
    assert!(
        (2..=CONNECTIONS).contains(&peak),
        "concurrent connections: {peak}"
    );
    Ok(())
}

#[test]
fn cancel_preserves_committed_blocks_and_resume_skips_them() -> Result<()> {
    let server = Server::new(Mode::Slow, BLOCK as usize * 65 + 9);
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("file.zip");
    let marker = checkpoint_path(&path)?;
    let cancel = AtomicBool::new(false);
    let finished = AtomicBool::new(false);
    let (sent, received) = std::sync::mpsc::channel();
    let mut failures = Vec::new();
    let result = thread::scope(|scope| {
        scope.spawn(|| {
            while !finished.load(Ordering::Relaxed) {
                if fs::read(&marker)
                    .ok()
                    .and_then(|b| serde_json::from_slice::<Checkpoint>(&b).ok())
                    .is_some_and(|c| c.complete.iter().any(|v| *v))
                {
                    cancel.store(true, Ordering::Relaxed);
                    sent.send(Instant::now()).unwrap();
                    break;
                }
                thread::sleep(Duration::from_millis(2));
            }
        });
        let result = transfer::download(&server.request(), &path, &cancel, |p| {
            if !p.detail.is_empty() {
                failures.push(p.detail);
            }
        });
        finished.store(true, Ordering::Relaxed);
        result
    });
    assert!(
        result.is_err(),
        "fallback reasons: {failures:?}; requests: {:?}",
        server.requests.lock().unwrap()
    );
    assert!(cancel.load(Ordering::Relaxed));
    assert!(received.recv()?.elapsed() < Duration::from_secs(3));
    let saved: Checkpoint = serde_json::from_slice(&fs::read(&marker)?)?;
    let count = server.requests.lock().unwrap().len();
    transfer::download(&server.request(), &path, &AtomicBool::new(false), |_| {})?;
    let requests = server.requests.lock().unwrap();
    for (i, done) in saved.complete.iter().enumerate() {
        if *done {
            assert!(
                !requests[count..].contains(&Some(saved.range(i))),
                "repeated committed block {i}"
            );
        }
    }
    assert_eq!(fs::read(path)?, *server.body);
    assert!(!marker.exists());
    Ok(())
}

#[test]
fn missing_ranges_and_invalid_ranges_fall_back_without_corrupting_output() -> Result<()> {
    for mode in [Mode::Ignore, Mode::BadRange] {
        let server = Server::new(mode, BLOCK as usize * 2 + 17);
        let temp = tempfile::tempdir()?;
        let path = transfer::download(
            &server.request(),
            &temp.path().join("file.zip"),
            &AtomicBool::new(false),
            |_| {},
        )?;
        assert_eq!(fs::read(path)?, *server.body);
    }
    Ok(())
}

#[test]
fn checksum_failure_uses_verified_fallback_and_cached_file_needs_no_network() -> Result<()> {
    let bad = Server::new(Mode::Corrupt, BLOCK as usize * 2 + 31);
    let good = Server::new(Mode::Range, bad.body.len());
    let mut request = bad.request();
    request.sources.extend(good.request().sources);
    let temp = tempfile::tempdir()?;
    let path = transfer::download(
        &request,
        &temp.path().join("file.zip"),
        &AtomicBool::new(false),
        |_| {},
    )?;
    assert_eq!(fs::read(&path)?, *good.body);
    request.sources[0].url = "http://127.0.0.1:1/offline".into();
    request.sources.truncate(1);
    transfer::download(&request, &path, &AtomicBool::new(false), |_| {})?;
    Ok(())
}

#[test]
fn metadata_limits_and_redirects_fail_closed_without_any_external_process() -> Result<()> {
    for mode in [Mode::Range, Mode::Missing, Mode::Redirect] {
        let server = Server::new(mode, 1024);
        let mut request = server.request();
        request.metadata = true;
        request.bytes = None;
        request.sha256 = None;
        request.limit = 10;
        let temp = tempfile::tempdir()?;
        assert!(
            transfer::download(
                &request,
                &temp.path().join("metadata.json"),
                &AtomicBool::new(false),
                |_| {}
            )
            .is_err()
        );
    }
    Ok(())
}

#[test]
fn stale_or_wrong_identity_checkpoint_never_skips_bytes() -> Result<()> {
    let server = Server::new(Mode::Range, BLOCK as usize * 2 + 1);
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("file.zip");
    fs::File::create(&path)?.set_len(server.body.len() as u64)?;
    let mut stale = Checkpoint::fresh(&server.request())?;
    stale.sha256 = "0".repeat(64);
    stale.complete.fill(true);
    core::atomic_json(&checkpoint_path(&path)?, &stale)?;
    transfer::download(&server.request(), &path, &AtomicBool::new(false), |_| {})?;
    assert_eq!(fs::read(path)?, *server.body);
    Ok(())
}

#[test]
fn transient_status_and_cut_connections_retry_only_failed_blocks() -> Result<()> {
    for mode in [Mode::Flaky, Mode::TruncatedOnce] {
        let server = Server::new(mode, BLOCK as usize * 4 + 31);
        let temp = tempfile::tempdir()?;
        let request = server.request();
        let mut events = Vec::new();
        let path = transfer::download(
            &request,
            &temp.path().join("retry.zip"),
            &AtomicBool::new(false),
            |p| events.push(p),
        )?;
        assert_eq!(fs::read(path)?, *server.body);
        assert!(events.iter().all(|p| p.completed <= request.bytes.unwrap()));
        assert!(
            !events
                .iter()
                .any(|p| p.phase == transfer::Phase::Restarting)
        );
        let requests = server.requests.lock().unwrap();
        assert!(
            requests.iter().all(Option::is_some),
            "fell back to full download"
        );
        assert_eq!(
            requests
                .iter()
                .filter(|r| **r == Some((0, BLOCK - 1)))
                .count(),
            2
        );
    }
    Ok(())
}

#[test]
fn hung_body_is_cancelled_promptly() -> Result<()> {
    let server = Server::new(Mode::Hanging, 1024);
    let temp = tempfile::tempdir()?;
    let cancel = AtomicBool::new(false);
    let started = Instant::now();
    let result = transfer::download(
        &server.request(),
        &temp.path().join("hang.zip"),
        &cancel,
        |p| {
            if p.connections > 0 {
                cancel.store(true, Ordering::Relaxed);
            }
        },
    );
    assert!(result.is_err());
    assert!(cancel.load(Ordering::Relaxed));
    assert!(started.elapsed() < Duration::from_secs(2));
    Ok(())
}

#[test]
fn same_destination_is_serialized_and_second_caller_uses_verified_cache() -> Result<()> {
    let server = Server::new(Mode::Slow, BLOCK as usize * 2 + 11);
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("shared.zip");
    let request = server.request();
    thread::scope(|scope| -> Result<()> {
        let worker = || transfer::download(&request, &path, &AtomicBool::new(false), |_| {});
        let first = scope.spawn(worker);
        let second = scope.spawn(worker);
        first.join().unwrap()?;
        second.join().unwrap()?;
        Ok(())
    })?;
    assert_eq!(fs::read(path)?, *server.body);
    assert_eq!(
        server.requests.lock().unwrap().len(),
        4,
        "one probe plus three blocks"
    );
    Ok(())
}

#[test]
fn exhausted_retries_keep_completed_blocks_for_next_download() -> Result<()> {
    let bad = Server::new(Mode::BlockFails, BLOCK as usize * 4 + 19);
    let good = Server::new(Mode::Range, bad.body.len());
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("resume.zip");
    assert!(transfer::download(&bad.request(), &path, &AtomicBool::new(false), |_| {}).is_err());
    let saved: Checkpoint = serde_json::from_slice(&fs::read(checkpoint_path(&path)?)?)?;
    assert!(!saved.complete[0]);
    assert!(saved.complete[1..].iter().all(|done| *done));
    assert_eq!(
        bad.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|r| **r == Some((0, BLOCK - 1)))
            .count(),
        6,
        "three segment attempts in each of two bounded source attempts"
    );
    transfer::download(&good.request(), &path, &AtomicBool::new(false), |_| {})?;
    assert_eq!(
        *good.requests.lock().unwrap(),
        [Some((0, 0)), Some((0, BLOCK - 1))]
    );
    assert_eq!(fs::read(path)?, *good.body);
    Ok(())
}
