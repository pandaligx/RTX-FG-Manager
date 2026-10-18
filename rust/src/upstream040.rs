//! The 0.4 bundle is a directory protocol, not a renamed single-file 0.3 proxy.
use crate::{
    diagnostics,
    presets::{Parameter, Values},
};
use anyhow::Result;
pub const PROFILE: &str = "upstream040";
pub const BACKEND: &str = "upstream040";
pub const SCHEME: &str = "upstream-0.3.5-310-9";
pub const PROXIES: [&str; 6] = [
    "version.dll",
    "winmm.dll",
    "dbghelp.dll",
    "dinput8.dll",
    "dxgi.dll",
    "d3d12.dll",
];
pub const COMPONENTS: [&str; 8] = [
    "dlssg_sm86/backend/sm86_backend.dll",
    "dlssg_sm86/kernels/fg_kernels.dll",
    "dlssg_sm86/runtime/nvngx_dlssg.dll",
    "dlssg_sm86/xefg/xefg_kernels.dll",
    "dlssg_sm86/sr/nvngx_dlss.dll",
    "dlssg_sm86/sr/sr_kernels.dll",
    "dlssg_sm86/LICENSE.md",
    "dlssg_sm86/THIRD_PARTY_NOTICES.txt",
];
pub fn parameters() -> Vec<Parameter> {
    let mut p = crate::presets::parameters("upstream035");
    for f in &mut p {
        if f.key == "optimized" {
            f.section = "DLSSG";
            f.default = "2";
            f.choices = vec![
                ("0", "0 · 原厂数值"),
                ("1", "1 · 逐位一致"),
                ("2", "2 · 上游默认（有损加速）"),
                ("3", "3 · 最快（有损）"),
            ];
        }
        if f.key == "preset" {
            f.section = "DLSSG";
            f.label = "DLSS 帧生成预设";
            f.choices.push(("C", "C · 性能模式"));
        }
    }
    p.extend([
        Parameter {
            key: "fg_engine",
            section: "FrameGeneration",
            ini_key: "Engine",
            label: "帧生成引擎",
            default: "DLSS",
            choices: vec![("DLSS", "DLSS"), ("XeSS", "XeSS · 2X")],
        },
        Parameter {
            key: "sr_mode",
            section: "SuperResolution",
            ini_key: "Mode",
            label: "超分模式",
            default: "Off",
            choices: vec![
                ("Off", "关闭"),
                ("On", "INT8 · 保留游戏预设"),
                ("M-INT8", "强制 M · INT8"),
                ("L-INT8", "强制 L · INT8"),
                ("M-FP16", "强制 M · FP16 对照"),
                ("L-FP16", "强制 L · FP16 对照"),
            ],
        },
        Parameter {
            key: "sr_tier",
            section: "SuperResolution",
            ini_key: "Tier",
            label: "超分作用档位",
            default: "Auto",
            choices: vec![
                ("Auto", "全部档位"),
                ("Quality", "质量"),
                ("Balanced", "平衡"),
                ("Performance", "性能"),
                ("UltraPerformance", "极致性能"),
            ],
        },
        Parameter {
            key: "xess_flow",
            section: "XeSSFG",
            ini_key: "FlowResolution",
            label: "XeSS 光流分辨率",
            default: "Middle",
            choices: vec![("Coarse", "粗"), ("Middle", "中"), ("Fine", "细")],
        },
        Parameter {
            key: "xess_hudless",
            section: "XeSSFG",
            ini_key: "UseHudless",
            label: "XeSS 使用无 HUD 输入",
            default: "1",
            choices: vec![("1", "开启"), ("0", "关闭")],
        },
        Parameter {
            key: "frame_timing",
            section: "Logging",
            ini_key: "FrameTiming",
            label: "帧时间日志",
            default: "0",
            choices: vec![("0", "关闭"), ("1", "开启")],
        },
    ]);
    p
}
/// Preserve unknown options/comments. New spelling wins if both exist. Runtime
/// Auto disappeared upstream. The obsolete cache setting is preserved as inert
/// user text; it no longer controls extraction or authorizes global cleanup.
pub fn migrate(bytes: &[u8]) -> Result<Vec<u8>> {
    let (text, _) = diagnostics::decode_ini(bytes)?;
    let mut out = bytes.to_vec();
    for (old_section, key) in [
        ("FrameGeneration", "Optimized"),
        ("Compatibility", "Preset"),
    ] {
        if diagnostics::ini_value(&text, "DLSSG", key)?.is_none()
            && let Some(value) = diagnostics::ini_value(&text, old_section, key)?
        {
            out = diagnostics::edit_ini(&out, "DLSSG", key, &value)?;
        }
    }
    for key in ["Mode", "SRMode"] {
        if diagnostics::ini_value(&text, "Runtime", key)?
            .is_some_and(|v| v.eq_ignore_ascii_case("Auto"))
        {
            out = diagnostics::edit_ini(&out, "Runtime", key, "Bundled")?;
        }
    }
    Ok(out)
}
pub fn configure(bytes: &[u8], values: &Values) -> Result<Vec<u8>> {
    crate::presets::validate(PROFILE, values)?;
    let mut out = migrate(bytes)?;
    for p in parameters() {
        if let Some(value) = values.get(p.key) {
            out = diagnostics::edit_ini(&out, p.section, p.ini_key, value)?;
        }
    }
    Ok(out)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn migration_preserves_custom_settings_and_new_spelling() {
        let b=b"; custom\n[FrameGeneration]\nOptimized=1\n[DLSSG]\nOptimized=3\n[Compatibility]\nPreset=B\n[Runtime]\nMode=Auto\n[Other]\nMine=42\n";
        let s = String::from_utf8(migrate(b).unwrap()).unwrap();
        assert_eq!(
            diagnostics::ini_value(&s, "DLSSG", "Optimized")
                .unwrap()
                .as_deref(),
            Some("3")
        );
        assert_eq!(
            diagnostics::ini_value(&s, "DLSSG", "Preset")
                .unwrap()
                .as_deref(),
            Some("B")
        );
        assert_eq!(
            diagnostics::ini_value(&s, "Runtime", "Mode")
                .unwrap()
                .as_deref(),
            Some("Bundled")
        );
        assert!(s.contains("Mine=42") && s.contains("; custom"));
    }
}
