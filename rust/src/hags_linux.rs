//! Linux HAGS ("Hardware-accelerated GPU scheduling") reporting shim.
//!
//! HAGS is a Windows display-driver scheduler feature. A Wine prefix has no
//! such scheduler, so the manager cannot read or toggle it. What this module
//! keeps is the honest, useful half:
//!
//! * `classify`/`label`/`prompt` are pure and shared verbatim, so the UI shows
//!   the same wording on both platforms.
//! * `probe` reports the adapters it can actually enumerate and marks their
//!   scheduling support as unknown, yielding the "无法确认" state instead of
//!   inventing an enabled/disabled answer it cannot verify.
//! * `settings` fails with an explicit explanation rather than silently opening
//!   a Windows `ms-settings:` URI, which does not exist on Linux.

use crate::platform::CommandExt;
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    path::Path,
    process::{Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
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
    pub fn selected(&self) -> Option<bool> {
        None
    }
    pub fn needs_restart(&self) -> bool {
        false
    }
}
pub fn configure(_enabled: bool, _cancel: &AtomicBool) -> Result<State> {
    anyhow::bail!("硬件加速 GPU 调度是 Windows 功能，Linux 不支持此设置")
}
/// Shared, platform-neutral classification. Kept identical to the Windows
/// implementation so both builds describe the same conditions the same way.
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

/// Reads what the prefix can tell us about GPU scheduling.
///
/// Wine stores `HwSchMode` under `System\<ControlSet>\Control\GraphicsDrivers`
/// when a game/driver has set it, but it is normally absent. The adapters come
/// from the real CUDA driver, so the UI still names the GPUs; their scheduling
/// flags stay `None` because Wine does not expose them.
pub fn probe() -> Result<State> {
    let prefix = crate::win::prefix().unwrap_or_else(|_| crate::win::prefix_or_default());
    let configured = crate::wine_reg::get_value(
        &prefix,
        crate::wine_reg::Hive::Machine,
        "System\\ControlSet001\\Control\\GraphicsDrivers",
        "HwSchMode",
    )
    .ok()
    .flatten()
    .and_then(|v| match v {
        crate::wine_reg::RegValue::Dword(value) if [1, 2].contains(&value) => Some(value),
        _ => None,
    });
    let build = crate::win::build();
    let adapters = crate::win::gpu_names()
        .unwrap_or_default()
        .into_iter()
        .map(|(name, _)| Adapter {
            name,
            // Wine publishes no per-adapter scheduling capability, so an
            // explicit `None` is the truthful value and keeps `classify` from
            // claiming a state it could not observe.
            supported: None,
            enabled: None,
            flags: None,
        })
        .collect::<Vec<_>>();
    Ok(classify(adapters, configured, build))
}

/// Runs `--hags-probe` in a child so a wedged driver query cannot hang the UI.
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

/// There is no Linux/Wine settings page for HAGS. Explains this explicitly
/// rather than pretending to open a `ms-settings:` URI.
pub fn settings() -> Result<()> {
    let _ = Path::new(".");
    anyhow::bail!(
        "硬件加速 GPU 计划由 Windows 显示驱动提供，Wine 前缀无法配置；此开关不适用于 Linux 版"
    )
}
