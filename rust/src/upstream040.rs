//! The 0.4 bundle is a directory protocol, not a renamed single-file 0.3 proxy.
use crate::{
    diagnostics,
    presets::{Parameter, Values},
};
use anyhow::{Result, ensure};
pub const PROFILE: &str = "upstream040";
pub const PROFILE_041: &str = "upstream041";
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
pub fn is_profile(profile: &str) -> bool {
    matches!(profile, PROFILE | PROFILE_041)
}
pub fn profile_for_version(version: Option<&str>) -> &'static str {
    if version
        .and_then(|v| semver::Version::parse(v).ok())
        .is_some_and(|v| v >= semver::Version::new(0, 4, 1))
    {
        PROFILE_041
    } else {
        PROFILE
    }
}
pub fn parameters_041() -> Vec<Parameter> {
    let mut fields = parameters();
    fields.retain(|p| p.key != "sr_tier");
    for p in &mut fields {
        match p.key {
            "optimized" => {
                p.default = "1";
                p.choices = vec![
                    ("0", "0 · 原厂数值"),
                    ("1", "1 · 逐位一致（默认）"),
                    ("2", "2 · 有损加速"),
                    ("3", "3 · 最快（有损）"),
                ];
            }
            "preset" => {
                p.default = "A";
                p.choices.retain(|(v, _)| *v != "Auto");
            }
            "fg_engine" => {
                p.choices = vec![("DLSS", "DLSS · 2X–6X"), ("XeSS", "XeSS · 2X–4X")];
            }
            "max_generated_frames" => p.label = "可用帧生成倍率",
            "sr_mode" => p.choices.retain(|(v, _)| *v != "On"),
            "xess_flow" => p.choices.push(("CoarseAt4K", "4K 起使用粗光流")),
            "xess_hudless" => {
                p.default = "Auto";
                p.choices = vec![
                    ("Auto", "自动（推荐）"),
                    ("1", "严格要求无 HUD 输入"),
                    ("0", "关闭"),
                ];
            }
            _ => {}
        }
    }
    fields
}
pub fn validate_041(values: &Values) -> Result<()> {
    ensure!(
        !(values.get("fg_engine").is_some_and(|v| v == "XeSS")
            && values
                .get("max_generated_frames")
                .is_some_and(|v| matches!(v.as_str(), "4" | "5"))),
        "XeSS 帧生成最高支持 4X，请将可用倍率设为 4X 或更低"
    );
    Ok(())
}
pub fn help(key: &str) -> Option<&'static str> {
    Some(match key {
        "fg_engine" => {
            "需要 R580 或更新驱动。XeSS 使用随包后端，无需另装 Intel DLL；RGBA16F 后缓冲不支持，不会自动切回 DLSS。"
        }
        "sr_mode" => {
            "默认关闭。M/L 会覆盖全部画质档，质量、平衡和 DLAA 下可能更慢。旧版 On 和作用档位已取消。"
        }
        "xess_hudless" => "严格模式下，游戏缺少无 HUD 输入可能直接退出；通常使用自动。",
        "max_generated_frames" => {
            "游戏仍可选择更低倍率；旧代仅 2X 的 DLSS-G 接口可能不受支持，换新 DLL 不等于更新游戏接口。"
        }
        _ => return None,
    })
}
/// Upgrade the meaning of old values exactly once, before any game files move.
/// An SR mode without an equivalent requires a player choice, never a guess.
pub fn migrate_versions(
    bytes: &[u8],
    from: Option<&str>,
    to: Option<&str>,
    overrides: Option<&Values>,
) -> Result<Vec<u8>> {
    let old_profile = profile_for_version(from);
    let new_profile = profile_for_version(to);
    let mut out = if let Some(values) = overrides.filter(|v| !v.is_empty()) {
        crate::presets::configure(bytes, old_profile, values)?
    } else {
        migrate(bytes)?
    };
    if new_profile == PROFILE_041 && old_profile != PROFILE_041 {
        let (text, _) = diagnostics::decode_ini(&out)?;
        let sr = diagnostics::ini_value(&text, "SuperResolution", "Mode")?
            .unwrap_or_else(|| "Off".into());
        ensure!(
            !sr.eq_ignore_ascii_case("On"),
            "0.4.1 已取消超分 On，无法等价迁移。请先在当前补丁参数中选择关闭或明确的 M/L 模式，再升级；原安装尚未修改。"
        );
        let tier = diagnostics::ini_value(&text, "SuperResolution", "Tier")?;
        ensure!(
            sr.eq_ignore_ascii_case("Off")
                || tier
                    .as_deref()
                    .is_none_or(|v| v.eq_ignore_ascii_case("Auto")),
            "0.4.1 的 M/L 模式作用于全部画质档。请先将旧版超分作用档位设为全部档位，或关闭超分后再升级；原安装尚未修改。"
        );
        if diagnostics::ini_value(&text, "XeSSFG", "UseHudless")?
            .as_deref()
            .is_none_or(|v| v == "1")
        {
            out = diagnostics::edit_ini(&out, "XeSSFG", "UseHudless", "Auto")?;
        }
        if diagnostics::ini_value(&text, "DLSSG", "Preset")?
            .is_some_and(|v| v.eq_ignore_ascii_case("Auto"))
        {
            out = diagnostics::edit_ini(&out, "DLSSG", "Preset", "A")?;
        }
        for (section, key) in [("SuperResolution", "Tier"), ("XeSSFG", "Model")] {
            out = diagnostics::remove_ini_key(&out, section, key)?;
        }
    }
    if new_profile == PROFILE_041 {
        read_041(&out)?;
    }
    Ok(out)
}
pub fn read_041(bytes: &[u8]) -> Result<Values> {
    let (text, _) = diagnostics::decode_ini(bytes)?;
    let mut values = crate::presets::defaults(PROFILE_041, &Values::new());
    for p in parameters_041() {
        if let Some(mut value) = diagnostics::ini_value(&text, p.section, p.ini_key)? {
            // Upstream still accepts Auto with a warning and runs A.
            if p.key == "preset" && value.eq_ignore_ascii_case("Auto") {
                value = "A".into();
            }
            if let Some((canonical, _)) = p
                .choices
                .iter()
                .find(|(v, _)| v.eq_ignore_ascii_case(&value))
            {
                values.insert(p.key.into(), (*canonical).into());
            } else {
                anyhow::bail!(
                    "0.4.1 不支持参数 [{}] {}={}，请修正配置后重试",
                    p.section,
                    p.ini_key,
                    value
                );
            }
        }
    }
    validate_041(&values)?;
    Ok(values)
}
pub fn configure_041(bytes: &[u8], values: &Values) -> Result<Vec<u8>> {
    crate::presets::validate(PROFILE_041, values)?;
    let mut out = migrate(bytes)?;
    for p in parameters_041() {
        if let Some(value) = values.get(p.key) {
            out = diagnostics::edit_ini(&out, p.section, p.ini_key, value)?;
        }
    }
    read_041(&out)?;
    Ok(out)
}
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
        ("Compatibility", "OptimizedKernels"),
        ("Compatibility", "Preset"),
    ] {
        let target_key = if key == "OptimizedKernels" {
            "Optimized"
        } else {
            key
        };
        let (latest, _) = diagnostics::decode_ini(&out)?;
        if diagnostics::ini_value(&latest, "DLSSG", target_key)?.is_none()
            && let Some(value) = diagnostics::ini_value(&text, old_section, key)?
        {
            out = diagnostics::edit_ini(&out, "DLSSG", target_key, &value)?;
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
    #[test]
    fn current_parameter_labels_have_english_translations() {
        let tr = crate::i18n::Translator::new("en");
        let mut missing = Vec::new();
        for parameter in super::parameters_041() {
            for label in std::iter::once(parameter.label)
                .chain(parameter.choices.into_iter().map(|(_, label)| label))
            {
                if tr
                    .t(label)
                    .chars()
                    .any(|c| ('\u{3400}'..='\u{9fff}').contains(&c))
                {
                    missing.push(label);
                }
            }
        }
        assert!(
            missing.is_empty(),
            "Missing English translations: {missing:?}"
        );
    }
    use super::*;
    #[test]
    fn release_041_defaults_match_signed_package_ini() {
        let ini = include_bytes!("../../tests/fixtures/catalog041/dlssg_sm86.ini");
        assert_eq!(
            read_041(ini).unwrap(),
            crate::presets::defaults(PROFILE_041, &Values::new())
        );
        assert_eq!(
            crate::presets::defaults(PROFILE, &Values::new())["optimized"],
            "2"
        );
    }
    #[test]
    fn upgrade_converts_hudless_once_and_preserves_custom_values_and_encoding() {
        let old = b"\xef\xbb\xbf; keep comment\r\n[DLSSG]\r\nOptimized=3\r\nPreset=Auto\r\n[XeSSFG]\r\nUseHudless=1\r\nModel=18\r\n[SuperResolution]\r\nMode=Off\r\nTier=Quality\r\n[Custom]\r\nKeep=42\r\n";
        let new = migrate_versions(old, Some("0.4.0"), Some("0.4.1"), None).unwrap();
        let (text, _) = diagnostics::decode_ini(&new).unwrap();
        assert!(new.starts_with(&[0xef, 0xbb, 0xbf]));
        assert!(text.contains("; keep comment\r\n") && text.contains("Keep=42"));
        assert_eq!(read_041(&new).unwrap()["optimized"], "3");
        assert_eq!(read_041(&new).unwrap()["xess_hudless"], "Auto");
        assert_eq!(read_041(&new).unwrap()["preset"], "A");
        assert!(
            diagnostics::ini_value(&text, "SuperResolution", "Tier")
                .unwrap()
                .is_none()
        );
        let strict =
            configure_041(&new, &Values::from([("xess_hudless".into(), "1".into())])).unwrap();
        assert_eq!(
            read_041(&migrate_versions(&strict, Some("0.4.1"), Some("0.4.2"), None).unwrap())
                .unwrap()["xess_hudless"],
            "1"
        );
    }
    #[test]
    fn ambiguous_sr_upgrade_and_invalid_xess_combinations_are_rejected() {
        for ini in [
            "[SuperResolution]\nMode=On\n",
            "[SuperResolution]\nMode=M-INT8\nTier=Quality\n",
            "[FrameGeneration]\nEngine=XeSS\nMaxGeneratedFrames=5\n",
        ] {
            assert!(migrate_versions(ini.as_bytes(), Some("0.4.0"), Some("0.4.1"), None).is_err());
        }
        assert!(
            configure_041(
                b"[FrameGeneration]\nEngine=XeSS\n",
                &Values::from([("max_generated_frames".into(), "5".into())])
            )
            .is_err()
        );
        assert!(read_041(b"[SuperResolution]\nMode=On\n").is_err());
        assert!(read_041(b"[XeSSFG]\nUseHudless=wrong\n").is_err());
    }
    #[test]
    fn old_install_can_resolve_sr_choice_and_engine_switch_updates_cap() {
        let values = Values::from([("sr_mode".into(), "Off".into())]);
        let ini = migrate_versions(
            b"[SuperResolution]\nMode=On\n",
            Some("0.4.0"),
            Some("0.4.1"),
            Some(&values),
        )
        .unwrap();
        assert_eq!(read_041(&ini).unwrap()["sr_mode"], "Off");
        let mut values = Values::from([
            ("fg_engine".into(), "XeSS".into()),
            ("max_generated_frames".into(), "5".into()),
        ]);
        assert!(crate::presets::normalize(PROFILE_041, &mut values));
        assert_eq!(values["max_generated_frames"], "3");
        assert!(crate::presets::validate(PROFILE_041, &values).is_ok());
        assert!(!crate::presets::parameter_enabled(
            PROFILE_041,
            "optimized",
            &values
        ));
        assert!(crate::presets::parameter_enabled(
            PROFILE_041,
            "xess_hudless",
            &values
        ));
    }
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
