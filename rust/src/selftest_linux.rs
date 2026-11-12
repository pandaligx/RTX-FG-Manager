//! Linux self-test harness — the POSIX twin of the Windows-only `selftest.rs`.
//!
//! Windows contains every probe child in a Job Object created with
//! `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` and suppresses console creation through
//! `creation_flags(0x08000000)`. Both are replaced here by POSIX process
//! groups: every child is spawned with `CommandExt::process_group(0)`, so its
//! pid is also its process-group id, and a timeout or a cancel retires the
//! whole group with `kill(2)`. The public API, the JSON contract and the
//! report layout match the Windows module exactly.
//!
//! The `native-probes` feature is Windows-only (`build.rs` asserts it), so on
//! Linux [`adapters`] and [`child`] can never succeed. They still have to exist
//! and compile: `main.rs` and the shared modules call them unconditionally.

use crate::{assets, cleanup, core};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    fs,
    io::{Read, Seek, SeekFrom, Write},
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

/// Owns only the process launched for this test. Errors and cancellation retire it.
struct OwnedChild(Child);

/// Retire the process group led by `process`.
///
/// Only signal the group while the owned child is still its leader. Calling
/// the syscall directly avoids command-line ambiguity for negative PIDs and
/// never uses zero (the caller's group) as a fallback.
fn kill_group(process: &mut Child) {
    let signalled = i32::try_from(process.id()).is_ok_and(|pid| {
        // SAFETY: A positive child PID is checked against its current group.
        // The child has not been reaped, so its PID cannot have been reused.
        unsafe { pid > 1 && libc::getpgid(pid) == pid && libc::kill(-pid, libc::SIGKILL) == 0 }
    });
    if !signalled {
        let _ = process.kill();
    }
}

impl Drop for OwnedChild {
    fn drop(&mut self) {
        if !matches!(self.0.try_wait(), Ok(Some(_))) {
            kill_group(&mut self.0);
            let _ = self.0.wait();
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Adapter {
    pub luid: u64,
    pub name: String,
    pub vendor: u32,
    pub device: u32,
    pub vram: u64,
    pub driver: String,
    pub major: i32,
    pub minor: i32,
}
impl Adapter {
    pub fn route(&self) -> Option<&'static str> {
        match (self.major, self.minor) {
            (7, 5) => Some("SM75"),
            (8, 6..=9) | (9.., _) => Some("SM86"),
            _ => None,
        }
    }
    pub fn label(&self) -> String {
        format!(
            "{} · {:04X}:{:04X} · {:.1} GiB · SM{}{}",
            self.name,
            self.vendor,
            self.device,
            self.vram as f64 / 1073741824.,
            self.major,
            self.minor
        )
    }
}
/// The native probe extension is a Windows-only C++ build, so this can never
/// succeed on Linux. The message is the one the Windows build uses when the
/// feature is off, verbatim.
pub fn adapters() -> Result<Vec<Adapter>> {
    anyhow::bail!("Native GPU diagnostics are not included in this build; see BUILDING.md")
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Request {
    pub adapter: Adapter,
    pub complete: bool,
    pub forward_sm75: bool,
    pub game: Option<PathBuf>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Case {
    pub api: String,
    pub entry: String,
    pub route: String,
    pub mode: String,
    pub width: u32,
    pub height: u32,
    pub multiplier: u32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CaseResult {
    pub case: Case,
    pub status: String,
    pub stage: String,
    pub exit_code: Option<i32>,
    pub elapsed_ms: f64,
    pub detail: String,
    pub transport: String,
    pub trace: String,
    #[serde(default)]
    pub metrics: serde_json::Value,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Report {
    pub schema: u32,
    pub manager: String,
    pub created: String,
    pub request: Request,
    pub scope: String,
    pub cases: Vec<CaseResult>,
    pub folder: PathBuf,
}
/// The case matrix is platform-neutral, so its body is identical to the Windows
/// build (`selftest.rs:208-251`).
pub fn cases(request: &Request, entries: &[String]) -> Vec<Case> {
    let Some(route) = request.adapter.route() else {
        return vec![];
    };
    let mut routes = vec![route];
    if request.forward_sm75 && route != "SM75" && request.game.is_none() {
        routes.push("SM75")
    }
    let mut cases = Vec::new();
    for entry in entries {
        for route in &routes {
            let mut add = |api: &str, mode: &str, width, height, multiplier| {
                cases.push(Case {
                    api: api.into(),
                    entry: entry.clone(),
                    route: (*route).into(),
                    mode: mode.into(),
                    width,
                    height,
                    multiplier,
                })
            };
            add("load", "basic", 1280, 720, 1);
            for multiplier in 1..=3 {
                add("d3d12", "basic", 1280, 720, multiplier);
                add("vulkan", "normal", 1280, 720, multiplier);
            }
            if request.complete {
                for (api, mode, w, h) in [
                    ("d3d12", "typeless-stress", 1920, 1080),
                    ("d3d12", "rgba8-typeless", 1920, 1080),
                    ("vulkan", "rgba16", 1920, 1080),
                    ("vulkan", "bgra8", 1280, 720),
                    ("vulkan", "reuse", 1280, 720),
                    ("vulkan", "lifecycle", 1280, 720),
                    ("vulkan", "noext", 1280, 720),
                ] {
                    add(api, mode, w, h, 3);
                }
            }
        }
    }
    cases
}
/// Last `max` bytes of `path`, lossily decoded. Never propagates: any error
/// yields the empty string.
pub fn tail(path: &Path, max: u64) -> String {
    (|| -> Result<String> {
        let mut f = fs::File::open(path)?;
        let n = f.metadata()?.len();
        f.seek(SeekFrom::Start(n.saturating_sub(max)))?;
        let mut b = Vec::new();
        f.take(max).read_to_end(&mut b)?;
        Ok(String::from_utf8_lossy(&b).into_owned())
    })()
    .unwrap_or_default()
}
pub fn classify(
    code: Option<i32>,
    trace: &str,
    timed_out: bool,
    cancelled: bool,
) -> (&'static str, String) {
    if cancelled {
        return ("cancelled", "已取消".into());
    }
    if timed_out {
        return ("failed", "自检超时".into());
    }
    if code == Some(77) && trace.contains("SKIP ") {
        return (
            "skipped",
            trace
                .lines()
                .find(|s| s.starts_with("SKIP "))
                .unwrap_or_default()
                .into(),
        );
    }
    if code == Some(0) && trace.lines().any(|s| s.starts_with("PASS ")) && !trace.contains("FAIL ")
    {
        return ("passed", String::new());
    }
    (
        "failed",
        trace
            .lines()
            .rev()
            .find(|s| s.contains("FAIL") || s.contains("DEVICE_REMOVED"))
            .map(str::to_owned)
            .unwrap_or_else(|| format!("子进程退出：0x{:08X}", code.unwrap_or(-1) as u32)),
    )
}
fn write_event(file: &mut fs::File, value: serde_json::Value) -> Result<()> {
    serde_json::to_writer(&mut *file, &value)?;
    file.write_all(b"\n")?;
    file.flush()?;
    Ok(())
}
/// Poll a child until it exits, is cancelled, or reaches `limit`. On cancel or
/// timeout the whole process group is killed — the POSIX stand-in for closing
/// the kill-on-close Job Object.
fn monitor(
    process: &mut Child,
    cancel: &AtomicBool,
    limit: Duration,
    mut tick: impl FnMut() -> Result<()>,
) -> Result<(Option<i32>, bool, bool)> {
    let start = Instant::now();
    loop {
        if let Some(status) = process.try_wait()? {
            return Ok((status.code(), false, false));
        }
        let cancelled = cancel.load(Ordering::Relaxed);
        let timeout = start.elapsed() >= limit;
        if cancelled || timeout {
            kill_group(process);
            return Ok((process.wait()?.code(), timeout, cancelled));
        }
        tick()?;
        std::thread::sleep(Duration::from_millis(100));
    }
}
/// Payload written to `request.json` and consumed by the probe child. Present
/// for API parity with the Windows module; on Linux [`child`] refuses before it
/// is ever parsed.
#[allow(dead_code)]
#[derive(Serialize, Deserialize)]
struct ChildRequest {
    case: Case,
    luid: u64,
    dll: PathBuf,
    frame_max: u32,
}
pub fn child(_path: &Path) -> Result<()> {
    anyhow::bail!("Native GPU diagnostics are not included in this build; see BUILDING.md")
}
/// Runs the synthetic case matrix in `folder`, rewriting `report.json` after
/// every case so a crash still leaves partial evidence behind.
///
/// The Windows module derives its folder from the data directory plus a fresh
/// UUID; the caller passes the already-resolved folder here.
pub fn run(
    folder: &Path,
    request: Request,
    cancel: &AtomicBool,
    progress: impl Fn(String),
) -> Result<Report> {
    fs::create_dir_all(folder)?;
    let mut report = Report {
        schema: 1,
        manager: crate::VERSION.into(),
        created: chrono::Utc::now().to_rfc3339(),
        request: request.clone(),
        scope: "Local diagnostic evidence, not proof of game compatibility".into(),
        cases: vec![],
        folder: folder.to_path_buf(),
    };
    let mut files = std::collections::BTreeMap::new();
    let source_ini;
    if let Some(game) = &request.game {
        let game = core::location(game, false)?;
        let dir = game
            .parent()
            .ok_or_else(|| anyhow::anyhow!("游戏路径无效"))?;
        for entry in core::PROXIES {
            let path = dir.join(entry);
            if path.is_file() && cleanup::known_proxy(&path)? {
                core::no_links(&path)?;
                files.insert(entry.to_string(), fs::read(path)?);
            }
        }
        ensure!(!files.is_empty(), "未发现可识别的本项目 DLL");
        let ini = core::no_links(&dir.join(core::INI))?;
        ensure!(ini.metadata()?.len() <= 1024 * 1024, "INI 文件过大");
        source_ini = fs::read(ini)?;
    } else {
        for name in core::PROXIES {
            if request.complete || name == "version.dll" {
                files.insert(
                    name.to_string(),
                    assets::bytes(&format!("payloads/native/{name}"))?,
                );
            }
        }
        source_ini = assets::bytes("payloads/native/dlssg_sm86.ini")?;
    }
    core::atomic_json(
        &folder.join("files.json"),
        &files
            .iter()
            .map(|(n, b)| json!({"name":n,"bytes":b.len(),"sha256":core::hash(b)}))
            .collect::<Vec<_>>(),
    )?;
    let all = cases(&request, &files.keys().cloned().collect::<Vec<_>>());
    let start = Instant::now();
    let mut events = fs::File::create(folder.join("events.jsonl"))?;
    if all.is_empty() {
        write_event(
            &mut events,
            json!({"status":"skipped","reason":"unsupported_compute_capability"}),
        )?;
    }
    let mut stop = false;
    for (index, case) in all.iter().enumerate() {
        if stop || cancel.load(Ordering::Relaxed) || start.elapsed() > Duration::from_secs(1800) {
            report.cases.push(CaseResult {
                case: case.clone(),
                status: if cancel.load(Ordering::Relaxed) {
                    "cancelled"
                } else {
                    "skipped"
                }
                .into(),
                stage: "not_started".into(),
                exit_code: None,
                elapsed_ms: 0.,
                detail: "自检已停止".into(),
                transport: String::new(),
                trace: String::new(),
                metrics: json!({}),
            });
            continue;
        }
        let dir = folder.join(format!("{:03}", index + 1));
        fs::create_dir(&dir)?;
        let dll = dir.join(&case.entry);
        core::write_new(&dll, &files[&case.entry])?;
        // The Windows build rewrites Logging.Directory, Runtime.CacheDirectory,
        // Compatibility.Router and Logging.Level through `crate::diagnostics`,
        // then reads `MaxGeneratedFrames` back out of the INI. On Linux the
        // probe child refuses before reading any of that, so the collected INI
        // is copied verbatim and `frame_max` keeps the documented default of 3.
        core::write_new(&dir.join(core::INI), &source_ini)?;
        let req = ChildRequest {
            case: case.clone(),
            luid: request.adapter.luid,
            dll,
            frame_max: 3,
        };
        let request_path = dir.join("request.json");
        core::atomic_json(&request_path, &req)?;
        let message = format!(
            "{}/{} · {} · {} · {} · {}X",
            index + 1,
            all.len(),
            case.entry,
            case.route,
            case.api,
            case.multiplier + 1
        );
        progress(message.clone());
        write_event(
            &mut events,
            json!({"case":index+1,"stage":"starting","description":message}),
        )?;
        let mut owned_process = OwnedChild(
            Command::new(std::env::current_exe()?)
                .arg("--selftest-child")
                .arg(&request_path)
                .current_dir(&dir)
                // Own process group: pid == pgid, so `kill -9 -<pid>` retires the
                // probe and anything it spawned. Replaces creation_flags(0x08000000).
                .process_group(0)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()?,
        );
        let process = &mut owned_process.0;
        let began = Instant::now();
        let mut phase = String::new();
        let trace_path = dir.join("trace.txt");
        let limit =
            Duration::from_secs(120).min(Duration::from_secs(1800).saturating_sub(start.elapsed()));
        let (code, timed_out, cancelled) = monitor(process, cancel, limit, || {
            let trace = tail(&trace_path, 32768);
            if let Some(new) = trace.lines().rev().find_map(|l| l.strip_prefix("STAGE="))
                && phase != new
            {
                phase = new.into();
                write_event(&mut events, json!({"case":index+1,"stage":phase}))?;
                progress(format!("{message} · {phase}"));
            }
            Ok(())
        })?;
        let trace = tail(&trace_path, 1024 * 1024);
        // Remove only the fresh per-case copy after the process has exited.
        if core::no_links(&req.dll).is_ok() {
            let _ = fs::remove_file(&req.dll);
        }
        if let Some(last) = trace.lines().rev().find_map(|l| l.strip_prefix("STAGE=")) {
            phase = last.into();
        }
        let (status, detail) = classify(code, &trace, timed_out, cancelled);
        let bridge = tail(&dir.join("rtxfg-vulkan-bridge.log"), 2 * 1024 * 1024);
        let transport =
            if bridge.contains("shared_fallback") || bridge.contains("cached_host fallback") {
                "cached_host"
            } else if bridge.contains("shared_transfer") {
                "gpu_shared_buffer"
            } else {
                "not_reported"
            };
        stop = timed_out
            || trace.contains("DEVICE_REMOVED_REASON=887a")
            || trace.contains("VkResult=-4")
            || trace.contains("HRESULT=887a0005")
            || trace.contains("HRESULT=887a0006")
            || trace.contains("result=-4")
            || bridge.contains("device lost") && !bridge.contains("shutdown");
        // The Windows build substitutes the router actually read back from the
        // INI here; on Linux the INI is never rewritten, so the case keeps the
        // route the matrix assigned to it.
        let actual_case = case.clone();
        let gpu_queue_ms = trace
            .lines()
            .filter_map(|l| l.strip_prefix("gpu_queue_ms=")?.parse::<f64>().ok())
            .collect::<Vec<_>>();
        let vram = trace
            .lines()
            .filter(|l| l.starts_with("vram["))
            .collect::<Vec<_>>();
        let result = CaseResult {
            case: actual_case,
            status: status.into(),
            stage: phase,
            exit_code: code,
            elapsed_ms: began.elapsed().as_secs_f64() * 1000.,
            detail,
            transport: transport.into(),
            trace: format!("{:03}/trace.txt", index + 1),
            metrics: json!({"gpu_queue_ms":gpu_queue_ms,"gpu_timing_scope":"Queue timestamps include synchronization; not pure model time or FPS","wall_clock_ms":began.elapsed().as_secs_f64()*1000.,"vram":vram,"output_checks":trace.lines().filter(|l|l.contains("verified")||l.starts_with("valid_generated_outputs=")).collect::<Vec<_>>()}),
        };
        write_event(&mut events, json!({"case":index+1,"result":result}))?;
        report.cases.push(result);
        core::atomic_json(&folder.join("report.json"), &report)?;
    }
    core::atomic_json(&folder.join("report.json"), &report)?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    /// One counter per test process keeps concurrent tests from sharing a
    /// scratch file.
    static FIXTURE_FILE: AtomicUsize = AtomicUsize::new(0);

    #[test]
    fn child_fixture() {
        match std::env::var("RTXFG_UNIT_FIXTURE").as_deref() {
            Ok("exit") => std::process::exit(19),
            Ok("sleep") => std::thread::sleep(Duration::from_secs(10)),
            _ => (),
        }
    }
    fn fixture(mode: &str) -> Result<OwnedChild> {
        Ok(OwnedChild(
            Command::new(std::env::current_exe()?)
                .args(["--exact", "selftest::tests::child_fixture", "--nocapture"])
                .env("RTXFG_UNIT_FIXTURE", mode)
                // Same isolation the harness uses for real probe children.
                .process_group(0)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()?,
        ))
    }
    #[test]
    fn classify_reaches_every_terminal_state() {
        assert_eq!(
            classify(None, "", true, true),
            ("cancelled", String::from("已取消"))
        );
        assert_eq!(
            classify(None, "", true, false),
            ("failed", String::from("自检超时"))
        );
        assert_eq!(
            classify(Some(77), "boot\nSKIP no adapter\n", false, false),
            ("skipped", String::from("SKIP no adapter"))
        );
        assert_eq!(
            classify(Some(0), "PASS load\nPASS frame\n", false, false),
            ("passed", String::new())
        );
        let (status, detail) = classify(Some(0), "PASS load\nFAIL frame 887a0005\n", false, false);
        assert_eq!(status, "failed");
        assert_eq!(detail, "FAIL frame 887a0005");
        // DEVICE_REMOVED becomes the detail only when the PASS-only rule cannot
        // claim the run: with a PASS line and no FAIL, exit code 0 still passes.
        let (status, detail) = classify(Some(0), "boot\nDEVICE_REMOVED\n", false, false);
        assert_eq!(status, "failed");
        assert_eq!(detail, "DEVICE_REMOVED");
        assert_eq!(
            classify(Some(9), "no marker\n", false, false),
            ("failed", String::from("子进程退出：0x00000009"))
        );
        assert_eq!(
            classify(None, "no marker\n", false, false),
            ("failed", String::from("子进程退出：0xFFFFFFFF"))
        );
    }
    #[test]
    fn tail_reads_the_last_bytes_and_never_propagates_errors() {
        let n = FIXTURE_FILE.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir();
        let path = dir.join(format!("rtxfg-selftest-{}-{n}.txt", std::process::id()));
        fs::write(&path, b"alpha\nbeta\ngamma\n").unwrap();
        assert_eq!(tail(&path, 0), "");
        assert_eq!(tail(&path, 5), "amma\n");
        assert_eq!(tail(&path, 4096), "alpha\nbeta\ngamma\n");
        fs::write(&path, b"ok\xff").unwrap();
        assert!(tail(&path, 1).contains('\u{FFFD}'));
        assert_eq!(tail(&dir.join("rtxfg-selftest-absent-file"), 16), "");
        // A directory cannot be read as a file: still the empty string, no panic.
        assert_eq!(tail(&dir, 16), "");
        let _ = fs::remove_file(&path);
    }
    #[test]
    fn cases_build_the_shared_matrix() {
        let adapter = |major, minor| Adapter {
            luid: 1,
            name: "GPU".into(),
            vendor: 0x10DE,
            device: 0x2504,
            vram: 8 << 30,
            driver: "560.0".into(),
            major,
            minor,
        };
        let request = |complete, forward_sm75| Request {
            adapter: adapter(8, 6),
            complete,
            forward_sm75,
            game: None,
        };
        let entries = |name: &str| vec![name.to_string()];
        // Unsupported compute capability yields an empty matrix.
        let unsupported = Request {
            adapter: adapter(7, 0),
            complete: true,
            forward_sm75: false,
            game: None,
        };
        assert!(cases(&unsupported, &entries("version.dll")).is_empty());
        // 1 load + 3 multipliers * (d3d12 + vulkan).
        assert_eq!(
            cases(&request(false, false), &entries("version.dll")).len(),
            7
        );
        // The complete profile adds the seven extended cases.
        assert_eq!(
            cases(&request(true, false), &entries("version.dll")).len(),
            14
        );
        // forward_sm75 doubles the matrix once per extra route.
        assert_eq!(
            cases(&request(true, true), &entries("version.dll")).len(),
            28
        );
        // A game snapshot pins SM86 even when forwarding is requested.
        let snapshot = Request {
            adapter: adapter(8, 6),
            complete: false,
            forward_sm75: true,
            game: Some(PathBuf::from("/tmp/game")),
        };
        assert_eq!(cases(&snapshot, &entries("version.dll")).len(), 7);
    }
    #[test]
    fn child_exit_timeout_cancel_and_job_cleanup() -> Result<()> {
        let mut exit = fixture("exit")?;
        let result = monitor(
            &mut exit.0,
            &AtomicBool::new(false),
            Duration::from_secs(5),
            || Ok(()),
        )?;
        assert_eq!(result, (Some(19), false, false));
        for cancel in [false, true] {
            let mut child = fixture("sleep")?;
            let result = monitor(
                &mut child.0,
                &AtomicBool::new(cancel),
                Duration::from_millis(200),
                || Ok(()),
            )?;
            assert_eq!(result.2, cancel);
            if !cancel {
                assert!(result.1);
            }
            // The child was signalled and reaped by the monitor itself.
            assert!(child.0.try_wait()?.is_some());
        }
        // Dropping the guard retires the process group: the POSIX analogue of
        // closing the kill-on-close Job Object.
        let child = fixture("sleep")?;
        let pid = child.0.id();
        drop(child);
        assert!(
            !Path::new(&format!("/proc/{pid}")).exists(),
            "process-group cleanup left pid {pid} running"
        );
        Ok(())
    }
}
