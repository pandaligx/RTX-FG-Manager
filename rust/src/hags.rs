use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    ffi::c_void,
    os::windows::process::CommandExt,
    path::PathBuf,
    process::{Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
use windows::Win32::{
    Foundation::LUID,
    Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1},
};
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Adapter {
    pub name: String,
    pub supported: Option<bool>,
    pub enabled: Option<bool>,
    pub flags: Option<u32>,
}
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct State {
    pub state: String,
    pub adapters: Vec<Adapter>,
    pub configured: Option<u32>,
    pub build: u32,
}
pub fn classify(adapters: Vec<Adapter>, configured: Option<u32>, build: u32) -> State {
    let state = if build < 19041 {
        "unsupported"
    } else if adapters.is_empty() {
        "unknown"
    } else if adapters
        .iter()
        .any(|a| a.supported == Some(true) && a.enabled == Some(false))
    {
        if configured == Some(2) {
            "pending_restart"
        } else {
            "disabled"
        }
    } else if adapters.iter().all(|a| a.enabled == Some(true)) {
        if configured == Some(1) {
            "pending_disable"
        } else {
            "enabled"
        }
    } else if adapters.iter().all(|a| a.supported == Some(false)) {
        "unsupported"
    } else {
        "unknown"
    };
    State {
        state: state.into(),
        adapters,
        configured,
        build,
    }
}
pub fn label(state: &str) -> &'static str {
    match state {
        "enabled" => "硬件加速 GPU 计划：已开启",
        "disabled" => "硬件加速 GPU 计划：未开启",
        "pending_restart" => "硬件加速 GPU 计划：已设置开启，需重启",
        "pending_disable" => "硬件加速 GPU 计划：当前已开启，重启后将关闭",
        "unsupported" => "硬件加速 GPU 计划：系统或驱动暂不支持",
        "checking" => "硬件加速 GPU 计划：正在检测…",
        _ => "硬件加速 GPU 计划：无法确认，请在系统设置中检查",
    }
}
pub fn prompt(state: &State, already: bool) -> bool {
    !already && matches!(state.state.as_str(), "disabled" | "pending_restart")
}
pub fn probe() -> Result<State> {
    use winreg::{RegKey, enums::*};
    let configured = RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey_with_flags(
            "SYSTEM\\CurrentControlSet\\Control\\GraphicsDrivers",
            KEY_READ | KEY_WOW64_64KEY,
        )
        .and_then(|k| k.get_value::<u32, _>("HwSchMode"))
        .ok()
        .filter(|v| [1, 2].contains(v));
    let build = crate::win::build();
    if build < 19041 {
        return Ok(classify(vec![], configured, build));
    }
    #[repr(C)]
    struct Open {
        luid: LUID,
        handle: u32,
    }
    #[repr(C)]
    struct Query {
        handle: u32,
        kind: u32,
        data: *mut c_void,
        size: u32,
    }
    #[repr(C)]
    struct Close {
        handle: u32,
    }
    let path =
        PathBuf::from(std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into()))
            .join("System32/gdi32.dll");
    let mut rows = Vec::new();
    // SAFETY: The system GDI module exports these WDDM ABI functions. Structures and
    // DWORD output are correctly sized. DXGI interfaces and adapter handles are released.
    unsafe {
        let lib = libloading::Library::new(path)?;
        let open: libloading::Symbol<unsafe extern "system" fn(*mut Open) -> i32> =
            lib.get(b"D3DKMTOpenAdapterFromLuid\0")?;
        let query: libloading::Symbol<unsafe extern "system" fn(*mut Query) -> i32> =
            lib.get(b"D3DKMTQueryAdapterInfo\0")?;
        let close: libloading::Symbol<unsafe extern "system" fn(*const Close) -> i32> =
            lib.get(b"D3DKMTCloseAdapter\0")?;
        let factory: IDXGIFactory1 = CreateDXGIFactory1()?;
        for i in 0..32 {
            let adapter = match factory.EnumAdapters1(i) {
                Ok(a) => a,
                Err(e) if e.code().0 as u32 == 0x887a0002 => break,
                Err(e) => return Err(e.into()),
            };
            let d = adapter.GetDesc1()?;
            if d.VendorId != 0x10de || d.Flags & 2 != 0 {
                continue;
            }
            let mut row = Adapter {
                name: crate::win::from_wide(&d.Description),
                ..Default::default()
            };
            let mut h = Open {
                luid: d.AdapterLuid,
                handle: 0,
            };
            if open(&mut h) >= 0 {
                let mut flags = 0u32;
                let mut q = Query {
                    handle: h.handle,
                    kind: 70,
                    data: (&mut flags as *mut u32).cast(),
                    size: 4,
                };
                if query(&mut q) >= 0 {
                    row.supported = Some(flags & 1 != 0);
                    row.enabled = Some(flags & 2 != 0);
                    row.flags = Some(flags);
                }
                close(&Close { handle: h.handle });
            }
            rows.push(row);
        }
    }
    Ok(classify(rows, configured, build))
}
pub fn detect(cancel: &AtomicBool) -> Result<State> {
    let dir = tempfile::tempdir()?;
    let out = dir.path().join("hags.json");
    let mut child = Command::new(std::env::current_exe()?)
        .arg("--hags-probe")
        .arg(&out)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(0x08000000)
        .spawn()?;
    let start = Instant::now();
    loop {
        if let Some(exit) = child.try_wait()? {
            ensure!(exit.success(), "GPU 计划查询失败");
            break;
        }
        if cancel.load(Ordering::Relaxed) || start.elapsed() > Duration::from_secs(12) {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!("GPU 计划查询超时或已取消")
        }
        std::thread::sleep(Duration::from_millis(40));
    }
    let data = crate::core::read_json(&out, 65536)?;
    let state: State = serde_json::from_value(data)?;
    ensure!(
        [
            "enabled",
            "disabled",
            "pending_restart",
            "pending_disable",
            "unsupported",
            "unknown"
        ]
        .contains(&state.state.as_str()),
        "GPU 计划查询状态无效"
    );
    Ok(state)
}
pub fn settings() -> Result<()> {
    let target = if crate::win::build() >= 22000 {
        "ms-settings:display-advancedgraphics-default"
    } else {
        "ms-settings:display-advancedgraphics"
    };
    crate::win::open(target).or_else(|_| crate::win::open("ms-settings:display"))
}
