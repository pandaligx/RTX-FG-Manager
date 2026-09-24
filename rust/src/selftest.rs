//! Isolated, portable synthetic GPU diagnostics. No game process is loaded.
#[cfg(feature = "native-probes")]
use crate::win;
use crate::{assets, cleanup, core};
#[cfg(feature = "native-probes")]
use anyhow::Context;
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    fs,
    io::{Read, Seek, SeekFrom, Write},
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
use windows::Win32::{
    Foundation::{CloseHandle, HANDLE},
    System::JobObjects::*,
};

/// Owns only the process launched for this test. Errors and cancellation retire it.
struct OwnedChild(std::process::Child);
struct TestJob(HANDLE);
impl TestJob {
    fn new() -> Result<Self> {
        // SAFETY: an unnamed, non-inheritable job; all structures live across these calls.
        unsafe {
            let job = Self(CreateJobObjectW(None, windows::core::PCWSTR::null())?);
            let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            SetInformationJobObject(
                job.0,
                JobObjectExtendedLimitInformation,
                (&info as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                std::mem::size_of_val(&info) as u32,
            )?;
            Ok(job)
        }
    }
    fn assign(&self, process: &std::process::Child) -> Result<()> {
        use std::os::windows::io::AsRawHandle;
        // SAFETY: both handles remain owned and valid; only this test child joins the job.
        unsafe {
            AssignProcessToJobObject(self.0, HANDLE(process.as_raw_handle()))?;
        }
        Ok(())
    }
}
impl Drop for TestJob {
    fn drop(&mut self) {
        // SAFETY: this wrapper uniquely owns the job handle. Closing kills any remaining test child.
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}
impl Drop for OwnedChild {
    fn drop(&mut self) {
        if !matches!(self.0.try_wait(), Ok(Some(_))) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
#[cfg(feature = "native-probes")]
struct RawAdapter {
    luid: u64,
    vram: u64,
    driver: u64,
    vendor: u32,
    device: u32,
    major: i32,
    minor: i32,
    name: [u16; 128],
}
#[cfg(feature = "native-probes")]
unsafe extern "C" {
    fn rtxfg_adapters(output: *mut RawAdapter, capacity: u32) -> i32;
    fn rtxfg_probe(
        api: *const u16,
        dll: *const u16,
        trace: *const u16,
        luid: u64,
        multiplier: u32,
        width: u32,
        height: u32,
        mode: *const u16,
        frame_max: u32,
    ) -> i32;
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
#[cfg(not(feature = "native-probes"))]
pub fn adapters() -> Result<Vec<Adapter>> {
    anyhow::bail!("Native GPU diagnostics are not included in this build; see BUILDING.md")
}
#[cfg(feature = "native-probes")]
pub fn adapters() -> Result<Vec<Adapter>> {
    let blank = RawAdapter {
        luid: 0,
        vram: 0,
        driver: 0,
        vendor: 0,
        device: 0,
        major: 0,
        minor: 0,
        name: [0; 128],
    };
    let mut list = [blank; 16];
    // SAFETY: the C ABI writes at most capacity fully initialized records of the matching repr(C) layout.
    let n = unsafe { rtxfg_adapters(list.as_mut_ptr(), list.len() as u32) };
    ensure!(n >= 0 && n as usize <= list.len(), "无法读取自检显卡信息");
    Ok(list[..n as usize]
        .iter()
        .map(|a| Adapter {
            luid: a.luid,
            name: win::from_wide(&a.name),
            vendor: a.vendor,
            device: a.device,
            vram: a.vram,
            driver: format!(
                "{}.{}.{}.{}",
                a.driver >> 48,
                (a.driver >> 32) & 65535,
                (a.driver >> 16) & 65535,
                a.driver & 65535
            ),
            major: a.major,
            minor: a.minor,
        })
        .collect())
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
fn monitor(
    process: &mut std::process::Child,
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
            let _ = process.kill();
            return Ok((process.wait()?.code(), timeout, cancelled));
        }
        tick()?;
        std::thread::sleep(Duration::from_millis(100));
    }
}
#[derive(Serialize, Deserialize)]
struct ChildRequest {
    case: Case,
    luid: u64,
    dll: PathBuf,
    frame_max: u32,
}
#[cfg(not(feature = "native-probes"))]
pub fn child(_path: &Path) -> Result<()> {
    anyhow::bail!("Native GPU diagnostics are not included in this build; see BUILDING.md")
}
#[cfg(feature = "native-probes")]
pub fn child(path: &Path) -> Result<()> {
    let p = core::no_links(path)?;
    let req: ChildRequest = serde_json::from_value(core::read_json(&p, 65536)?)?;
    let dir = p.parent().context("自检目录无效")?;
    let dll = core::no_links(&req.dll)?;
    ensure!(
        dll.parent() == Some(dir)
            && core::PROXIES.contains(&req.case.entry.as_str())
            && dll
                .file_name()
                .is_some_and(|n| n == req.case.entry.as_str()),
        "自检文件范围无效"
    );
    ensure!(
        ["load", "d3d12", "vulkan"].contains(&req.case.api.as_str())
            && (1..=3).contains(&req.case.multiplier)
            && (1..=1920).contains(&req.case.width)
            && (1..=1080).contains(&req.case.height)
            && [3, 5].contains(&req.frame_max)
            && [
                "basic",
                "normal",
                "typeless-stress",
                "rgba8-typeless",
                "rgba16",
                "bgra8",
                "reuse",
                "lifecycle",
                "noext"
            ]
            .contains(&req.case.mode.as_str()),
        "自检参数无效"
    );
    ensure!(cleanup::known_proxy(&dll)?, "未知 DLL 不参与自检");
    let trace = dir.join("trace.txt");
    let a = win::wide(&req.case.api);
    let d = win::wide(&dll);
    let t = win::wide(&trace);
    let m = win::wide(&req.case.mode);
    // SAFETY: all strings are terminated UTF-16, and the C++ boundary catches C++ exceptions.
    // The probe runs only in this child process; native faults cannot unwind into the GUI.
    let status = unsafe {
        rtxfg_probe(
            a.as_ptr(),
            d.as_ptr(),
            t.as_ptr(),
            req.luid,
            req.case.multiplier,
            req.case.width,
            req.case.height,
            m.as_ptr(),
            req.frame_max,
        )
    };
    std::process::exit(status)
}
pub fn run(
    data: &Path,
    request: Request,
    cancel: &AtomicBool,
    progress: impl Fn(String),
) -> Result<Report> {
    let folder = core::no_links(
        &data
            .join("diagnostics/selftests")
            .join(uuid::Uuid::new_v4().to_string()),
    )?;
    fs::create_dir_all(&folder)?;
    let mut report = Report {
        schema: 1,
        manager: crate::VERSION.into(),
        created: chrono::Utc::now().to_rfc3339(),
        request: request.clone(),
        scope: "Synthetic GPU output checks; not game compatibility or presented FPS".into(),
        cases: vec![],
        folder: folder.clone(),
    };
    let mut files = std::collections::BTreeMap::new();
    let source_ini;
    if let Some(game) = &request.game {
        let game = core::location(game, false)?;
        let dir = game.parent().unwrap();
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
        let mut ini = crate::diagnostics::edit_ini(&source_ini, "Logging", "Directory", "logs")?;
        ini = crate::diagnostics::edit_ini(&ini, "Runtime", "CacheDirectory", "cache")?;
        if request.game.is_none() {
            ini = crate::diagnostics::edit_ini(&ini, "Compatibility", "Router", &case.route)?;
            ini = crate::diagnostics::edit_ini(&ini, "Logging", "Level", "3")?;
        }
        let config_text = crate::diagnostics::decode_ini(&ini)?.0;
        let configured_router =
            crate::diagnostics::ini_value(&config_text, "Compatibility", "Router")?;
        let router = configured_router.as_deref().unwrap_or(&case.route);
        // Never execute a newer architecture on a physical Turing GPU, including game snapshots.
        if request.adapter.major == 7 && !router.eq_ignore_ascii_case("SM75") {
            report.cases.push(CaseResult {
                case: case.clone(),
                status: "skipped".into(),
                stage: "route".into(),
                exit_code: None,
                elapsed_ms: 0.,
                detail: "部署路由与实际显卡不兼容".into(),
                transport: String::new(),
                trace: String::new(),
                metrics: json!({}),
            });
            continue;
        }
        core::write_new(&dir.join(core::INI), &ini)?;
        let req = ChildRequest {
            case: case.clone(),
            luid: request.adapter.luid,
            dll,
            frame_max: if crate::diagnostics::ini_value(
                &config_text,
                "FrameGeneration",
                "MaxGeneratedFrames",
            )?
            .is_some_and(|s| s == "5")
            {
                5
            } else {
                3
            },
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
        let job = TestJob::new()?;
        let mut owned_process = OwnedChild(
            Command::new(std::env::current_exe()?)
                .arg("--selftest-child")
                .arg(&request_path)
                .current_dir(&dir)
                .creation_flags(0x08000000)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()?,
        );
        let process = &mut owned_process.0;
        job.assign(process)?;
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
        let mut actual_case = case.clone();
        actual_case.route = router.into();
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
                .creation_flags(0x08000000)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()?,
        ))
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
            let job = TestJob::new()?;
            let mut child = fixture("sleep")?;
            job.assign(&child.0)?;
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
            assert!(child.0.try_wait()?.is_some());
        }
        let mut child = fixture("sleep")?;
        let job = TestJob::new()?;
        job.assign(&child.0)?;
        drop(job);
        assert!(child.0.wait()?.code().is_some());
        Ok(())
    }
}
