//! RTXMFG Universal / MFG Unlocker use their own JSON protocol,
//! not dlssg_sm86.ini. Keep the profile ID stable for existing game settings.
use crate::presets::{Parameter, Values};
use anyhow::{Result, ensure};
use serde_json::{Value, json};

pub const PROFILE: &str = "rtxmfg_universal_133";
pub const BACKEND: &str = "rtx40mfg";
pub const CONFIG: &str = "RTXMFG-Universal.json";
pub const NOTICES: &str = "MFG-Unlocker-NOTICES.txt";
pub const UI_CONFIG: &str = "RTX40MFG-UI.ini";
/// Fixed upstream sidecars only; never derive deletion targets from JSON values.
pub const SIDECARS: [&str; 3] = [
    "RTXMFG-Universal.status.json",
    "RTX40MFG-UI.ini",
    "RTX40MFG-UI-layout.ini",
];
pub const PROXIES: [&str; 19] = [
    "version.dll",
    "dinput8.dll",
    "winmm.dll",
    "d3d9.dll",
    "d3d10.dll",
    "d3d11.dll",
    "d3d12.dll",
    "dxgi.dll",
    "dsound.dll",
    "wininet.dll",
    "winhttp.dll",
    "binkw64.dll",
    "bink2w64.dll",
    "xinput1_1.dll",
    "xinput1_2.dll",
    "xinput1_3.dll",
    "xinput1_4.dll",
    "xinput9_1_0.dll",
    "xinputuap.dll",
];
pub const ENTRIES: [&str; 20] = {
    let mut entries = [crate::mfg_plugin::ENTRY; 20];
    let mut i = 0;
    while i < PROXIES.len() {
        entries[i] = PROXIES[i];
        i += 1;
    }
    entries
};
pub fn parameters() -> Vec<Parameter> {
    let mut fields = vec![
        Parameter {
            key: "rtx_mode",
            section: "",
            ini_key: "",
            label: "请求倍率",
            default: "follow",
            choices: vec![
                ("follow", "跟随游戏"),
                ("1", "1X"),
                ("2", "2X"),
                ("3", "3X"),
                ("4", "4X"),
                ("5", "5X"),
                ("6", "6X"),
                ("dynamic", "动态多帧（仅 DX12）"),
            ],
        },
        Parameter {
            key: "rtx_target",
            section: "",
            ini_key: "dynamicTargetFrameRate",
            label: "动态目标帧率",
            default: "0",
            choices: vec![
                ("0", "跟随显示器刷新率"),
                ("60", "60 FPS"),
                ("90", "90 FPS"),
                ("120", "120 FPS"),
                ("144", "144 FPS"),
                ("165", "165 FPS"),
                ("180", "180 FPS"),
                ("240", "240 FPS"),
                ("360", "360 FPS"),
            ],
        },
        Parameter {
            key: "rtx_preset",
            section: "",
            ini_key: "dlssgPreset",
            label: "DLSS 帧生成预设",
            default: "0",
            choices: vec![
                ("0", "跟随游戏 / 驱动"),
                ("1", "A"),
                ("2", "B"),
                ("3", "C · 性能模式"),
            ],
        },
        Parameter {
            key: "rtx_vsync",
            section: "",
            ini_key: "vsyncMode",
            label: "垂直同步",
            default: "0",
            choices: vec![("0", "跟随游戏"), ("1", "关闭")],
        },
        Parameter {
            key: "rtx_sr_preset",
            section: "",
            ini_key: "dlssSrPreset",
            label: "DLSS 超分辨率预设",
            default: "0",
            choices: vec![
                ("0", "跟随游戏 / 驱动"),
                ("10", "J"),
                ("11", "K"),
                ("12", "L"),
                ("13", "M"),
            ],
        },
        Parameter {
            key: "rtx_rr_preset",
            section: "",
            ini_key: "dlssRrPreset",
            label: "DLSS 光线重建预设",
            default: "0",
            choices: vec![
                ("0", "跟随游戏 / 驱动"),
                ("4", "D"),
                ("5", "E"),
                ("6", "F（驱动 580+）"),
            ],
        },
        Parameter {
            key: "rtx_flip_metering",
            section: "Compatibility",
            ini_key: "Rtx30FlipMetering",
            label: "RTX 30 帧调度",
            default: "1",
            choices: vec![("1", "驱动调度（默认）"), ("0", "旧版 CPU 调度")],
        },
        Parameter {
            key: "rtx_gpu_override",
            section: "Compatibility",
            ini_key: "GpuNameOverride",
            label: "解锁游戏帧生成选项（实验）",
            default: "0",
            choices: vec![("0", "关闭"), ("1", "开启（仅当前游戏）")],
        },
        Parameter {
            key: "rtx_reflex_limit",
            section: "",
            ini_key: "reflexFrameLimitFps",
            label: "Reflex 帧率限制",
            default: "0",
            choices: vec![
                ("0", "关闭"),
                ("60", "60 FPS"),
                ("90", "90 FPS"),
                ("120", "120 FPS"),
                ("144", "144 FPS"),
                ("165", "165 FPS"),
                ("180", "180 FPS"),
                ("240", "240 FPS"),
                ("360", "360 FPS"),
            ],
        },
    ];
    fields.extend(parameters_170());
    fields
}

fn parameters_170() -> Vec<Parameter> {
    let switch = || vec![("0", "关闭"), ("1", "开启")];
    let field = |key, ini_key, label, default, choices| Parameter {
        key,
        section: "",
        ini_key,
        label,
        default,
        choices,
    };
    vec![
        field("rtx_nr", "dlss5", "DLSS 神经渲染", "0", switch()),
        field(
            "rtx_nr_stage",
            "dlss5HookStage",
            "NR 处理位置",
            "0",
            vec![
                ("0", "自动"),
                ("1", "超分前（DX12）"),
                ("2", "超分后"),
                ("3", "Present（DX12）"),
            ],
        ),
        field(
            "rtx_nr_preset",
            "dlss5Preset",
            "NR 模型预设",
            "0",
            vec![("0", "运行库默认"), ("1", "A"), ("2", "B")],
        ),
        field(
            "rtx_nr_scale_auto",
            "dlss5ResolutionScaleAuto",
            "NR 自动分辨率",
            "0",
            switch(),
        ),
        field(
            "rtx_nr_scale",
            "dlss5ResolutionScale",
            "NR 分辨率比例",
            "100",
            vec![("75", "75%"), ("85", "85%"), ("100", "100%")],
        ),
        field(
            "rtx_nr_intensity",
            "dlss5OverallIntensity",
            "NR 强度",
            "1",
            vec![
                ("0", "0%"),
                ("0.25", "25%"),
                ("0.5", "50%"),
                ("0.75", "75%"),
                ("1", "100%"),
            ],
        ),
        field(
            "rtx_nr_passes",
            "dlss5PassCount",
            "NR 处理遍数",
            "1",
            vec![
                ("1", "1"),
                ("2", "2"),
                ("3", "3"),
                ("4", "4"),
                ("5", "5"),
                ("6", "6"),
                ("7", "7"),
                ("8", "8"),
                ("9", "9"),
                ("10", "10"),
            ],
        ),
        field(
            "rtx_nr_half",
            "dlss5Boost",
            "NR 隔帧处理（DX12）",
            "0",
            switch(),
        ),
        field(
            "rtx_xemfg",
            "xemfgStandalone",
            "独立 XeSS 多帧生成（实验）",
            "0",
            switch(),
        ),
        field(
            "rtx_xemfg_frames",
            "xemfgFrames",
            "XeSS 最大倍率",
            "2",
            vec![
                ("2", "2X"),
                ("3", "3X"),
                ("4", "4X"),
                ("5", "5X"),
                ("6", "6X"),
            ],
        ),
        field(
            "rtx_xemfg_dynamic",
            "xemfgDynamic",
            "XeSS 动态倍率",
            "0",
            switch(),
        ),
        field(
            "rtx_xemfg_target",
            "xemfgTargetFps",
            "XeSS 动态目标帧率",
            "0",
            vec![
                ("0", "自动"),
                ("60", "60 FPS"),
                ("120", "120 FPS"),
                ("144", "144 FPS"),
                ("165", "165 FPS"),
                ("240", "240 FPS"),
            ],
        ),
        field(
            "rtx_xemfg_rounding",
            "xemfgRounding",
            "XeSS 倍率取整",
            "1",
            vec![("0", "不超过目标"), ("1", "最接近目标"), ("2", "达到目标")],
        ),
        field(
            "rtx_xemfg_allow1x",
            "xemfgAllow1x",
            "XeSS 允许回到 1X",
            "1",
            switch(),
        ),
        field(
            "rtx_xemfg_cap",
            "xemfgTargetAsCap",
            "XeSS 使用目标限帧",
            "0",
            switch(),
        ),
        field(
            "rtx_opti_ui",
            "optiScalerUi",
            "OptiScaler 菜单协作",
            "auto",
            vec![
                ("auto", "自动"),
                ("optiscaler", "仅 OptiScaler"),
                ("rtxmfg", "仅 MFG Unlocker"),
                ("shared", "共享菜单"),
            ],
        ),
    ]
}

pub fn is_170_key(key: &str) -> bool {
    parameters_170().iter().any(|p| p.key == key)
}
fn json_value(key: &str, value: &str) -> Result<Value> {
    Ok(match key {
        "rtx_nr" | "rtx_nr_scale_auto" | "rtx_nr_half" => json!(value == "1"),
        "rtx_nr_scale" | "rtx_nr_intensity" => json!(value.parse::<f64>()?),
        "rtx_opti_ui" => json!(value),
        _ => json!(value.parse::<u32>()?),
    })
}
pub fn configure(bytes: &[u8], values: &Values) -> Result<Vec<u8>> {
    let mut data: Value =
        serde_json::from_slice(bytes.strip_prefix(&[239, 187, 191]).unwrap_or(bytes))?;
    ensure!(data.is_object(), "RTXMFG JSON 必须是对象");
    crate::presets::validate(PROFILE, values)?;
    if values.keys().all(|key| is_sidecar_key(key)) {
        return Ok(bytes.to_vec());
    }
    if let Some(mode) = values.get("rtx_mode") {
        data["followGame"] = json!(mode == "follow");
        data["mode"] = json!(if mode == "follow" {
            "follow"
        } else if mode == "dynamic" {
            "dynamic"
        } else {
            "fixed"
        });
        data["multiplier"] = json!(mode.parse::<u32>().unwrap_or(2));
    }
    for p in parameters()
        .into_iter()
        .skip(1)
        .filter(|p| p.section.is_empty())
    {
        if let Some(value) = values.get(p.key) {
            data[p.ini_key] = json_value(p.key, value)?;
        }
    }
    // Freeze explicitly edited NR settings at the current generation so the
    // upstream migration cannot replace them. Preserve unedited advanced values
    // and any newer generation from a future patch.
    if values.keys().any(|k| k.starts_with("rtx_nr"))
        && data
            .get("dlss5DefaultsVersion")
            .and_then(Value::as_u64)
            .is_none_or(|v| v < 3)
    {
        data["dlss5DefaultsVersion"] = json!(3);
    }
    Ok(serde_json::to_vec_pretty(&data)?)
}
pub fn read(bytes: &[u8]) -> Result<Values> {
    let data: Value =
        serde_json::from_slice(bytes.strip_prefix(&[239, 187, 191]).unwrap_or(bytes))?;
    ensure!(data.is_object(), "RTXMFG JSON 必须是对象");
    let mode = if data["followGame"] == true {
        "follow".to_owned()
    } else if data["mode"] == "dynamic" {
        "dynamic".into()
    } else {
        data["multiplier"].as_u64().unwrap_or(2).to_string()
    };
    let mut values = Values::from([("rtx_mode".into(), mode)]);
    for p in parameters()
        .into_iter()
        .skip(1)
        .filter(|p| p.section.is_empty())
    {
        if let Some(value) = data.get(p.ini_key) {
            let n = match value {
                Value::Bool(b) => if *b { "1" } else { "0" }.to_owned(),
                Value::String(s) => s.clone(),
                Value::Number(n) => n.to_string(),
                _ => continue,
            };
            if valid_value(p.key, &n) {
                values.insert(p.key.into(), n);
            }
        }
    }
    crate::presets::validate(PROFILE, &values)?;
    Ok(values)
}

pub fn valid_value(key: &str, value: &str) -> bool {
    (matches!(key, "rtx_target" | "rtx_reflex_limit" | "rtx_xemfg_target")
        && !value.is_empty()
        && value.bytes().all(|b| b.is_ascii_digit())
        && value.parse::<u32>().is_ok_and(|n| n <= 1000))
        || (matches!(key, "rtx_nr_scale" | "rtx_nr_intensity")
            && value.parse::<f64>().is_ok_and(|n| {
                n.is_finite() && n >= 0. && n <= if key == "rtx_nr_scale" { 200. } else { 1. }
            }))
        || parameters()
            .iter()
            .any(|p| p.key == key && p.choices.iter().any(|(v, _)| *v == value))
}

pub fn is_sidecar_key(key: &str) -> bool {
    matches!(key, "rtx_flip_metering" | "rtx_gpu_override")
}

pub fn help(key: &str) -> Option<&'static str> {
    match key {
        "rtx_nr" => Some(
            "需要 MFG 1.7+ 和 NR 运行库；右侧可单独下载安装。游戏需启用 DLSS SR、DLAA 或 RR；Backspace 打开菜单。",
        ),
        "rtx_nr_stage" => Some(
            "Vulkan 仅支持自动或超分后；隔帧处理仅适用于 DX12 超分前/后。高级画质控制可在游戏内菜单调整。",
        ),
        "rtx_xemfg" => Some(
            "需要右侧安装 XeSS 3.0.2 运行库，重启游戏生效。独立模式不需要 OptiScaler；插件模式请在 OptiScaler 中选择 XeFG。",
        ),
        "rtx_xemfg_dynamic" => Some(
            "独立 XeMFG 支持 DX12/Vulkan；Vulkan 仅使用 DLSS 超分输入。游戏原生 FSR/XeSS 帧生成应关闭。",
        ),
        "rtx_sr_preset" | "rtx_rr_preset" => {
            Some("需要 MFG 1.6+ 及游戏支持的 DLSS 功能；预设不会安装 SR/RR 运行库。")
        }
        "rtx_flip_metering" => Some("默认使用驱动调度；遇到兼容问题可恢复旧版。重启游戏生效。"),
        "rtx_gpu_override" => Some(
            "仅向当前游戏报告 RTX 4090 身份，尝试显示帧生成选项；不修改系统显卡名称。默认关闭，重启游戏生效。",
        ),
        _ => None,
    }
}

pub fn validate_version(version: Option<&str>, values: &Values) -> Result<()> {
    ensure!(
        version
            .and_then(|v| semver::Version::parse(v).ok())
            .is_some_and(|v| v >= semver::Version::new(1, 7, 0))
            || !values.keys().any(|k| is_170_key(k)),
        "这些参数需要 MFG Unlocker 1.7.0 或更新版本，请先升级补丁"
    );
    let supported = version
        .and_then(|v| semver::Version::parse(v).ok())
        .is_some_and(|v| (v.major, v.minor, v.patch) >= (1, 6, 0));
    ensure!(
        supported
            || !values
                .keys()
                .any(|k| is_sidecar_key(k)
                    || matches!(k.as_str(), "rtx_sr_preset" | "rtx_rr_preset")),
        "这些参数需要 MFG Unlocker 1.6.0 或更新版本，请先升级补丁"
    );
    Ok(())
}

pub fn configure_ui(bytes: &[u8], values: &Values) -> Result<Vec<u8>> {
    crate::presets::validate(PROFILE, values)?;
    let mut out = bytes.to_vec();
    for p in parameters().into_iter().filter(|p| is_sidecar_key(p.key)) {
        if let Some(v) = values.get(p.key) {
            out = crate::diagnostics::edit_ini(&out, p.section, p.ini_key, v)?;
        }
    }
    Ok(out)
}

pub fn read_ui(bytes: &[u8]) -> Result<Values> {
    let (text, _) = crate::diagnostics::decode_ini(bytes)?;
    let mut values = Values::new();
    for p in parameters().into_iter().filter(|p| is_sidecar_key(p.key)) {
        if let Some(v) = crate::diagnostics::ini_value(&text, p.section, p.ini_key)? {
            let normalized = match v.to_ascii_lowercase().as_str() {
                "1" | "true" | "on" => Some("1"),
                "0" | "false" | "off" => Some("0"),
                _ => None,
            };
            if let Some(v) = normalized {
                values.insert(p.key.into(), v.into());
            }
        }
    }
    Ok(values)
}

/// Match upstream diagnostic_paths.h; never accept a deletion path from JSON.
pub fn log_path(exe: &std::path::Path) -> std::path::PathBuf {
    let path = exe.to_string_lossy();
    let mut hash = 14695981039346656037u64;
    for mut c in path.encode_utf16() {
        if c == b'/' as u16 {
            c = b'\\' as u16;
        }
        if (65..=90).contains(&c) {
            c += 32;
        }
        hash ^= u64::from(c);
        hash = hash.wrapping_mul(1099511628211);
    }
    let stem = exe
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .encode_utf16()
        .take(40)
        .map(|c| {
            let c = u8::try_from(c).unwrap_or(b'_').to_ascii_lowercase();
            if c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_') {
                c as char
            } else {
                '_'
            }
        })
        .collect::<String>();
    std::env::temp_dir().join(format!("RTXMFG-{stem}-{hash:016X}.log"))
}

/// Per-installation runtime artifacts use the exact upstream path hash. No
/// wildcard TEMP scanning or game-controlled paths are involved.
pub fn log_paths(exe: &std::path::Path) -> Vec<std::path::PathBuf> {
    let current = log_path(exe);
    let previous = current.with_extension("previous.log");
    let mut interval = current.file_stem().unwrap_or_default().to_os_string();
    interval.push("-intervals.csv");
    let interval = current.with_file_name(interval);
    let previous2 = current.with_extension("previous2.log");
    let crash = current.with_extension("crash.dmp");
    let mut paths = vec![current, previous, previous2, crash, interval];
    if let Some(dir) = exe.parent() {
        for path in paths.clone() {
            let local = dir.join(path.file_name().unwrap());
            if !paths
                .iter()
                .any(|p| crate::core::key(p) == crate::core::key(&local))
            {
                paths.push(local);
            }
        }
    }
    paths
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_explicit_edits_preserve_original_json_bytes() -> Result<()> {
        let before = b"\xef\xbb\xbf{\r\n  \"followGame\": true, \"custom\": {\"keep\": 7}\r\n}\r\n";
        assert_eq!(configure(before, &Values::new())?, before);
        assert!(configure(b"[]", &Values::new()).is_err());
        Ok(())
    }

    #[test]
    fn presentation_fields_preserve_other_settings_and_custom_frame_limit() -> Result<()> {
        let before = br#"{"followGame":false,"mode":"dynamic","multiplier":3,"dlssgPreset":1,"reflexFrameLimitFps":200,"custom":{"keep":true}}"#;
        let mut settings = read(before)?;
        assert_eq!(settings["rtx_reflex_limit"], "200");
        settings.insert("rtx_vsync".into(), "1".into());
        let after: Value = serde_json::from_slice(&configure(before, &settings)?)?;
        assert_eq!(after["mode"], "dynamic");
        assert_eq!(after["vsyncMode"], 1);
        assert_eq!(after["reflexFrameLimitFps"], 200);
        assert_eq!(after["dlssgPreset"], 1);
        assert_eq!(after["custom"]["keep"], true);
        assert!(!valid_value("rtx_vsync", "99"));
        assert!(!valid_value("rtx_reflex_limit", "1001"));
        assert!(!valid_value("rtx_reflex_limit", "../60"));
        Ok(())
    }

    #[test]
    fn diagnostic_paths_are_scoped_to_the_full_executable_path() {
        let a = log_paths(std::path::Path::new("D:/游戏 A/Game.exe"));
        let a_case = log_paths(std::path::Path::new("d:/游戏 a/game.EXE"));
        let b = log_paths(std::path::Path::new("D:/游戏 B/Game.exe"));
        assert_eq!(
            a.iter().map(|p| crate::core::key(p)).collect::<Vec<_>>(),
            a_case
                .iter()
                .map(|p| crate::core::key(p))
                .collect::<Vec<_>>()
        );
        assert_ne!(a, b);
        assert!(
            a[..5]
                .iter()
                .all(|p| p.parent() == Some(std::env::temp_dir().as_path()))
        );
        assert!(a[1].to_string_lossy().ends_with(".previous.log"));
        assert!(a[2].to_string_lossy().ends_with(".previous2.log"));
        assert!(a[3].to_string_lossy().ends_with(".crash.dmp"));
        assert!(a[4].to_string_lossy().ends_with("-intervals.csv"));
        assert_eq!(a.len(), 10);
        assert_eq!(a[5].parent(), Some(std::path::Path::new("D:/游戏 A")));
    }
}
