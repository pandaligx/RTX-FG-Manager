use anyhow::{Context, Result, ensure};
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
impl State {
    /// The saved choice, distinct from the scheduler currently running in WDDM.
    pub fn selected(&self) -> Option<bool> {
        match self.state.as_str() {
            "enabled" | "pending_restart" => Some(true),
            "disabled" | "pending_disable" => Some(false),
            _ => None,
        }
    }
    pub fn needs_restart(&self) -> bool {
        matches!(self.state.as_str(), "pending_restart" | "pending_disable")
    }
}
pub fn classify(adapters: Vec<Adapter>, configured: Option<u32>, build: u32) -> State {
    let eligible: Vec<_> = adapters
        .iter()
        .filter(|a| a.supported == Some(true))
        .collect();
    let current = eligible
        .first()
        .and_then(|a| a.enabled)
        .filter(|enabled| eligible.iter().all(|a| a.enabled == Some(*enabled)));
    let desired = match configured {
        Some(1) => Some(false),
        Some(2) => Some(true),
        _ => current,
    };
    let state = if build < 19041 {
        "unsupported"
    } else if adapters.is_empty() {
        "unknown"
    } else if adapters.iter().all(|a| a.supported == Some(false)) {
        "unsupported"
    } else if !eligible.is_empty() && eligible.iter().all(|a| a.enabled.is_some()) {
        match desired {
            Some(true) if eligible.iter().any(|a| a.enabled == Some(false)) => "pending_restart",
            Some(false) if eligible.iter().any(|a| a.enabled == Some(true)) => "pending_disable",
            Some(true) => "enabled",
            Some(false) => "disabled",
            None => "unknown",
        }
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
const REGISTRY_KEY: &str = "SYSTEM\\CurrentControlSet\\Control\\GraphicsDrivers";
const REGISTRY_VALUE: &str = "HwSchMode";

fn read_mode(key: &winreg::RegKey) -> Result<Option<u32>> {
    match key.get_value::<u32, _>(REGISTRY_VALUE) {
        Ok(0) => Ok(None), // Windows/driver default.
        Ok(value @ (1 | 2)) => Ok(Some(value)),
        Ok(_) => anyhow::bail!("GPU 调度设置值无法识别，请在 Windows 设置中检查"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).context("无法读取 GPU 调度设置"),
    }
}

fn write_mode(key: &winreg::RegKey, value: u32) -> Result<()> {
    // winreg calls RegSetValueExW directly: no shell, helper service or driver patch.
    key.set_value(REGISTRY_VALUE, &value)
        .context("无法保存 GPU 调度设置，请以管理员身份运行后重试")?;
    ensure!(
        read_mode(key)? == Some(value),
        "GPU 调度设置未保存成功，请刷新后重试"
    );
    Ok(())
}

fn configure_with(
    state: State,
    enabled: bool,
    cancel: &AtomicBool,
    write: impl FnOnce(u32) -> Result<()>,
) -> Result<State> {
    ensure!(!cancel.load(Ordering::Relaxed), "GPU 调度设置已取消");
    ensure!(
        state.selected().is_some() && state.build >= 19041,
        "无法确认系统和驱动支持 GPU 调度，未修改设置"
    );
    if state.selected() == Some(enabled) {
        return Ok(state);
    }
    let value = if enabled { 2 } else { 1 };
    write(value)?;
    Ok(classify(state.adapters, Some(value), state.build))
}

pub fn configure(enabled: bool, cancel: &AtomicBool) -> Result<State> {
    use winreg::{RegKey, enums::*};
    // Query again on activation. Never enable from a stale or unsupported capability.
    let state = detect(cancel)?;
    configure_with(state, enabled, cancel, |value| {
        let key = RegKey::predef(HKEY_LOCAL_MACHINE)
            .open_subkey_with_flags(
                REGISTRY_KEY,
                KEY_QUERY_VALUE | KEY_SET_VALUE | KEY_WOW64_64KEY,
            )
            .context("无法保存 GPU 调度设置，请以管理员身份运行后重试")?;
        write_mode(&key, value)
    })
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
    let key = RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey_with_flags(REGISTRY_KEY, KEY_READ | KEY_WOW64_64KEY)
        .context("无法读取 GPU 调度设置")?;
    let configured = read_mode(&key)?;
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
            if d.Flags & 2 != 0 {
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
                    // KMTQAITYPE_WDDM_2_7_CAPS. Reserved by Microsoft; failures
                    // remain unknown, never treated as disabled or as permission to write.
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

#[cfg(test)]
mod tests {
    use super::*;
    fn adapter(enabled: bool) -> Adapter {
        Adapter {
            name: "Test GPU".into(),
            supported: Some(true),
            enabled: Some(enabled),
            flags: None,
        }
    }
    #[test]
    fn hags_roundtrip_uses_running_driver_not_first_checkbox_value() -> Result<()> {
        for build in [19041, 19045, 22000, 26100] {
            for running in [false, true] {
                for configured in [None, Some(if running { 2 } else { 1 })] {
                    let cancel = AtomicBool::new(false);
                    let initial = classify(vec![adapter(running)], configured, build);
                    assert_eq!(initial.selected(), Some(running));
                    assert!(!initial.needs_restart());
                    let changed = configure_with(initial, !running, &cancel, |value| {
                        assert_eq!(value, if running { 1 } else { 2 });
                        Ok(())
                    })?;
                    assert_eq!(changed.selected(), Some(!running));
                    assert!(changed.needs_restart());
                    // A new manager process reads the saved choice and the still-running driver.
                    let reopened = classify(changed.adapters, changed.configured, build);
                    assert_eq!(reopened.selected(), Some(!running));
                    assert!(reopened.needs_restart());
                    let restored = configure_with(reopened, running, &cancel, |_| Ok(()))?;
                    assert_eq!(restored.selected(), Some(running));
                    assert!(!restored.needs_restart());
                    let rebooted = classify(
                        vec![adapter(!running)],
                        Some(if running { 1 } else { 2 }),
                        build,
                    );
                    assert!(!rebooted.needs_restart());
                }
            }
        }
        Ok(())
    }
    #[test]
    fn hags_unavailable_and_cancelled_queries_never_write() {
        for state in [
            classify(vec![], Some(2), 26100),
            classify(vec![adapter(false)], None, 18362),
            classify(
                vec![Adapter {
                    supported: Some(false),
                    ..Default::default()
                }],
                Some(2),
                26100,
            ),
            classify(
                vec![Adapter {
                    supported: Some(true),
                    ..Default::default()
                }],
                Some(2),
                26100,
            ),
        ] {
            assert_eq!(state.selected(), None);
            assert!(
                configure_with(state, true, &AtomicBool::new(false), |_| panic!(
                    "must not write"
                ))
                .is_err()
            );
        }
        assert!(
            configure_with(
                classify(vec![adapter(false)], None, 26100),
                true,
                &AtomicBool::new(true),
                |_| panic!("cancelled write")
            )
            .is_err()
        );
    }
    #[test]
    fn hags_multi_adapter_ignores_unsupported_gpu_but_preserves_mixed_state() {
        let unsupported = Adapter {
            supported: Some(false),
            enabled: Some(false),
            ..Default::default()
        };
        assert_eq!(
            classify(vec![adapter(true), unsupported], None, 26100).selected(),
            Some(true)
        );
        let mixed = vec![adapter(true), adapter(false)];
        assert_eq!(classify(mixed.clone(), None, 26100).selected(), None);
        for value in [1, 2] {
            assert!(classify(mixed.clone(), Some(value), 26100).needs_restart());
        }
    }
    #[test]
    fn hags_noop_and_failed_save_do_not_claim_success() {
        let state = classify(vec![adapter(false)], None, 26100);
        assert!(
            configure_with(state.clone(), false, &AtomicBool::new(false), |_| panic!(
                "no-op write"
            ))
            .is_ok()
        );
        let result = configure_with(state, true, &AtomicBool::new(false), |_| {
            anyhow::bail!("access denied")
        });
        assert!(result.is_err());
    }
    #[test]
    fn hags_registry_roundtrip_in_isolated_user_key() -> Result<()> {
        use winreg::{RegKey, enums::*};
        // Never touch HKLM GraphicsDrivers in tests. Exercise the same Win32
        // registry calls in a unique, disposable per-user key instead.
        let root = RegKey::predef(HKEY_CURRENT_USER);
        let path = format!("Software\\RTXFG-Hags-Test-{}", uuid::Uuid::new_v4());
        let result = (|| -> Result<()> {
            let (key, _) = root.create_subkey(&path)?;
            assert_eq!(read_mode(&key)?, None);
            for value in [2, 1] {
                write_mode(&key, value)?;
                assert_eq!(read_mode(&key)?, Some(value));
            }
            let readonly = root.open_subkey_with_flags(&path, KEY_QUERY_VALUE)?;
            assert!(write_mode(&readonly, 2).is_err());
            assert_eq!(read_mode(&key)?, Some(1));
            key.set_value(REGISTRY_VALUE, &0u32)?;
            assert_eq!(read_mode(&key)?, None);
            key.set_value(REGISTRY_VALUE, &99u32)?;
            assert!(read_mode(&key).is_err());
            key.set_value(REGISTRY_VALUE, &"invalid type")?;
            assert!(read_mode(&key).is_err());
            Ok(())
        })();
        root.delete_subkey(&path)?;
        result
    }
}
