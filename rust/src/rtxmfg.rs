//! RTXMFG Universal v1.3.3 HF2 / v1.4.1 HF1 use their own JSON protocol,
//! not dlssg_sm86.ini. Keep the profile ID stable for existing game settings.
use crate::presets::{Parameter, Values};
use anyhow::{Result, ensure};
use serde_json::{Value, json};

pub const PROFILE: &str = "rtxmfg_universal_133";
pub const BACKEND: &str = "rtx40mfg";
pub const CONFIG: &str = "RTXMFG-Universal.json";
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
pub fn parameters() -> Vec<Parameter> {
    vec![
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
            label: "UI 重组预设",
            default: "0",
            choices: vec![("0", "跟随游戏 / 驱动"), ("1", "A"), ("2", "B")],
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
    ]
}
pub fn configure(bytes: &[u8], values: &Values) -> Result<Vec<u8>> {
    let mut data: Value =
        serde_json::from_slice(bytes.strip_prefix(&[239, 187, 191]).unwrap_or(bytes))?;
    ensure!(data.is_object(), "RTXMFG JSON 必须是对象");
    crate::presets::validate(PROFILE, values)?;
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
    for p in parameters().into_iter().skip(1) {
        if let Some(value) = values.get(p.key) {
            data[p.ini_key] = json!(value.parse::<u32>()?);
        }
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
    for p in parameters().into_iter().skip(1) {
        if let Some(n) = data[p.ini_key].as_u64() {
            let n = n.to_string();
            if valid_value(p.key, &n) {
                values.insert(p.key.into(), n);
            }
        }
    }
    crate::presets::validate(PROFILE, &values)?;
    Ok(values)
}

pub fn valid_value(key: &str, value: &str) -> bool {
    (matches!(key, "rtx_target" | "rtx_reflex_limit")
        && !value.is_empty()
        && value.bytes().all(|b| b.is_ascii_digit())
        && value.parse::<u32>().is_ok_and(|n| n <= 1000))
        || parameters()
            .iter()
            .any(|p| p.key == key && p.choices.iter().any(|(v, _)| *v == value))
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
    vec![current, previous, interval]
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(a, a_case);
        assert_ne!(a, b);
        assert!(
            a.iter()
                .all(|p| p.parent() == Some(std::env::temp_dir().as_path()))
        );
        assert!(a[1].to_string_lossy().ends_with(".previous.log"));
        assert!(a[2].to_string_lossy().ends_with("-intervals.csv"));
    }
}
