//! Small, versioned parameter protocols. Display strings never become INI keys.
use anyhow::{Result, ensure};
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Clone, Debug)]
pub struct Context {
    pub scheme: String,
    pub profile: String,
    pub delta: bool,
    pub delta_capable: bool,
}
impl Context {
    pub fn new(scheme: &str, policy: &crate::cloud::Policy, exe: &Path) -> Self {
        Self {
            scheme: scheme.into(),
            profile: policy.parameter_profile.clone(),
            delta_capable: policy.capabilities.contains(crate::delta::CAPABILITY),
            delta: policy.parameter_profile == "upstream035"
                && policy.capabilities.contains(crate::delta::CAPABILITY)
                && crate::delta::is_game(exe),
        }
    }
    pub fn parameters(&self) -> Vec<Parameter> {
        let mut fields = parameters(&self.profile);
        if self.delta {
            let count = fields
                .iter_mut()
                .find(|p| p.key == "max_generated_frames")
                .unwrap();
            count.choices = vec![
                ("0", "跟随游戏"),
                ("1", "2X"),
                ("2", "3X"),
                ("3", "4X（推荐）"),
            ];
        }
        fields
    }
    pub fn normalize(&self, values: &mut Values) -> bool {
        let mut changed = normalize(&self.profile, values);
        if self.delta
            && matches!(
                values.get("max_generated_frames").map(String::as_str),
                Some("4" | "5")
            )
        {
            values.insert("max_generated_frames".into(), "3".into());
            changed = true;
        }
        changed
    }
    pub fn parameter_enabled(&self, key: &str, values: &Values) -> bool {
        parameter_enabled(&self.profile, key, values)
    }
    pub fn configure(&self, bytes: &[u8], values: &Values) -> Result<Vec<u8>> {
        let mut values = values.clone();
        self.normalize(&mut values);
        let mut out = configure(bytes, &self.profile, &values)?;
        if self.delta_capable {
            let n = if self.delta {
                values
                    .get("max_generated_frames")
                    .map(String::as_str)
                    .unwrap_or("3")
            } else {
                "0"
            };
            out = crate::diagnostics::edit_ini(
                &out,
                "Compatibility",
                "DeltaForceGeneratedFrames",
                n,
            )?;
            out = crate::diagnostics::edit_ini(
                &out,
                "Compatibility",
                "DeltaForcePrivateStreamline",
                if matches!(n, "2" | "3") { "1" } else { "0" },
            )?;
        }
        Ok(out)
    }
}

pub type Values = BTreeMap<String, String>;
pub const MFG_VULKAN: &str = "mfg_vulkan_sm86_7";
/// Normalize dependent settings without discarding an inactive target FPS.
/// The INI cap counts generated frames; ForceMultiplier includes the real frame.
pub fn normalize(profile: &str, values: &mut Values) -> bool {
    if profile != MFG_VULKAN {
        return false;
    }
    let cap = values
        .get("max_interpolated_frames")
        .map(String::as_str)
        .unwrap_or("5")
        .parse::<u8>();
    let force = values
        .get("force_multiplier")
        .and_then(|v| v.parse::<u8>().ok());
    if let (Ok(cap @ 1..=5), Some(force @ 2..=6)) = (cap, force)
        && force > cap + 1
    {
        values.insert("force_multiplier".into(), (cap + 1).to_string());
        return true;
    }
    false
}
pub fn parameter_enabled(profile: &str, key: &str, values: &Values) -> bool {
    profile != MFG_VULKAN
        || key != "dynamic_target_fps"
        || values.get("dynamic_mfg").map(String::as_str) == Some("1")
}
pub struct Parameter {
    pub key: &'static str,
    pub section: &'static str,
    pub ini_key: &'static str,
    pub label: &'static str,
    pub default: &'static str,
    pub choices: Vec<(&'static str, &'static str)>,
}
pub fn parameters(profile: &str) -> Vec<Parameter> {
    if profile == MFG_VULKAN {
        return mfg_parameters();
    }
    let mut p = vec![Parameter {
        key: "enabled",
        section: "General",
        ini_key: "Enabled",
        label: "补丁开关",
        default: "1",
        choices: vec![("1", "开启"), ("0", "关闭")],
    }];
    if profile == "native026" {
        p.clear();
    }
    if profile.starts_with("upstream") {
        p.push(Parameter {
            key: "optimized",
            section: "FrameGeneration",
            ini_key: "Optimized",
            label: "内核模式",
            default: "1",
            choices: if profile == "upstream035" {
                vec![
                    ("0", "0 · 原厂数值"),
                    ("1", "1 · 逐位一致（推荐）"),
                    ("2", "2 · 更快（有损）"),
                    ("3", "3 · 最快（有损）"),
                ]
            } else {
                vec![("0", "原厂数值"), ("1", "优化（推荐）")]
            },
        });
    }
    let mut counts = vec![
        ("0", "运行库默认"),
        ("1", "2X"),
        ("2", "3X"),
        ("3", "4X（推荐）"),
    ];
    if profile == "native026" {
        counts.remove(0);
    }
    if profile.starts_with("upstream") {
        counts.extend([("4", "5X"), ("5", "6X")]);
    }
    p.push(Parameter {
        key: "max_generated_frames",
        section: "FrameGeneration",
        ini_key: "MaxGeneratedFrames",
        label: "倍率上限",
        default: "3",
        choices: counts,
    });
    if profile.starts_with("upstream") {
        p.push(Parameter {
            key: "preset",
            section: "Compatibility",
            ini_key: "Preset",
            label: "UI 重组预设",
            default: "Auto",
            choices: vec![
                ("Auto", "Auto · 自动"),
                ("A", "A · 关闭 UI 重组"),
                ("B", "B · 开启 UI 重组"),
            ],
        });
    }
    if profile == "native026" {
        p.push(Parameter {
            key: "hardware_bilinear",
            section: "Compatibility",
            ini_key: "HardwareBilinear",
            label: "采样方式（仅 SM86）",
            default: "0",
            choices: vec![("0", "精确（推荐）"), ("1", "近似采样")],
        });
    }
    p.push(Parameter {
        key: "logging_level",
        section: "Logging",
        ini_key: "Level",
        label: "日志级别",
        default: "1",
        choices: vec![
            ("0", "0 · 关闭"),
            ("1", "1 · 仅错误（推荐）"),
            ("2", "2 · 诊断"),
            ("3", "3 · 详细"),
        ],
    });
    p
}
pub fn validate(profile: &str, values: &Values) -> Result<()> {
    ensure!(
        matches!(
            profile,
            "upstream035" | "upstream031" | "native026" | "initial" | MFG_VULKAN
        ),
        "Unsupported parameter protocol"
    );
    let p = parameters(profile);
    for (key, value) in values {
        ensure!(
            p.iter()
                .any(|p| p.key == key && p.choices.iter().any(|(v, _)| v == value)),
            "Invalid preset parameter: {key}"
        );
    }
    if profile == MFG_VULKAN {
        let cap = values
            .get("max_interpolated_frames")
            .map(String::as_str)
            .unwrap_or("5")
            .parse::<u8>()?;
        let force = values
            .get("force_multiplier")
            .map(String::as_str)
            .unwrap_or("0")
            .parse::<u8>()?;
        ensure!(
            force == 0 || force <= cap + 1,
            "Requested multiplier exceeds the frame limit"
        );
    }
    Ok(())
}
pub fn defaults(profile: &str, overrides: &Values) -> Values {
    let mut values = parameters(profile)
        .iter()
        .map(|p| {
            (
                p.key.into(),
                overrides
                    .get(p.key)
                    .filter(|v| p.choices.iter().any(|(choice, _)| *choice == v.as_str()))
                    .cloned()
                    .unwrap_or_else(|| p.default.into()),
            )
        })
        .collect();
    normalize(profile, &mut values);
    values
}
pub fn configure(bytes: &[u8], profile: &str, values: &Values) -> Result<Vec<u8>> {
    let mut values = values.clone();
    normalize(profile, &mut values);
    validate(profile, &values)?;
    let mut out = bytes.to_vec();
    for p in parameters(profile) {
        if let Some(value) = values.get(p.key) {
            out = crate::diagnostics::edit_ini(&out, p.section, p.ini_key, value)?;
        }
    }
    Ok(out)
}
pub fn merge(current: &[u8], desired: &[u8], backend: &str) -> Result<Vec<u8>> {
    let profile = if backend == "upstream_sm86" {
        "upstream035"
    } else if backend.starts_with("native") {
        "native026"
    } else {
        "initial"
    };
    let (text, _) = crate::diagnostics::decode_ini(desired)?;
    let mut out = current.to_vec();
    for p in parameters(profile) {
        if let Some(value) = crate::diagnostics::ini_value(&text, p.section, p.ini_key)? {
            out = crate::diagnostics::edit_ini(&out, p.section, p.ini_key, &value)?;
        }
    }
    if backend == "upstream_sm86" {
        out = restore_upstream_paths(&out, desired)?;
    }
    Ok(out)
}

// Repair only the manager's former overrides, never arbitrary user paths.
// Empty CacheDirectory deliberately leaves the shared upstream user cache outside
// per-game uninstall ownership; DeltaForceRuntimeId has separate ownership rules.
fn restore_upstream_paths(current: &[u8], desired: &[u8]) -> Result<Vec<u8>> {
    let (text, _) = crate::diagnostics::decode_ini(current)?;
    let (defaults, _) = crate::diagnostics::decode_ini(desired)?;
    let mut out = current.to_vec();
    for (section, key, former) in [
        ("Runtime", "CacheDirectory", ".rtx-fg-v3/cache"),
        ("Logging", "Directory", ".rtx-fg-v3/logs"),
    ] {
        let value = crate::diagnostics::ini_value(&text, section, key)?;
        if value.is_some_and(|v| v.replace('\\', "/").eq_ignore_ascii_case(former))
            && let Some(default) = crate::diagnostics::ini_value(&defaults, section, key)?
        {
            out = crate::diagnostics::edit_ini(&out, section, key, &default)?;
        }
    }
    Ok(out)
}

pub fn merge_context(
    current: &[u8],
    desired: &[u8],
    backend: &str,
    context: Option<&Context>,
) -> Result<Vec<u8>> {
    // A shared proxy backend does not imply a shared INI protocol.
    let mut out = if let Some(context) = context {
        let (text, _) = crate::diagnostics::decode_ini(desired)?;
        let mut out = current.to_vec();
        for p in parameters(&context.profile) {
            if let Some(value) = crate::diagnostics::ini_value(&text, p.section, p.ini_key)? {
                out = crate::diagnostics::edit_ini(&out, p.section, p.ini_key, &value)?;
            }
        }
        out
    } else {
        merge(current, desired, backend)?
    };
    if context.is_some_and(|c| matches!(c.profile.as_str(), "upstream031" | "upstream035")) {
        out = restore_upstream_paths(&out, desired)?;
    }
    if context.is_some_and(|c| c.profile == "upstream035") {
        let (text, _) = crate::diagnostics::decode_ini(desired)?;
        for key in [
            "DeltaForceGeneratedFrames",
            "DeltaForcePrivateStreamline",
            "DeltaForceRuntimeId",
        ] {
            if let Some(value) = crate::diagnostics::ini_value(&text, "Compatibility", key)? {
                out = crate::diagnostics::edit_ini(&out, "Compatibility", key, &value)?;
            }
        }
    }
    Ok(out)
}

fn mfg_parameters() -> Vec<Parameter> {
    let toggle = || vec![("0", "关闭"), ("1", "开启")];
    let mut fields = vec![
        Parameter {
            key: "max_interpolated_frames",
            section: "FrameGeneration",
            ini_key: "MaxInterpolatedFrames",
            label: "倍率上限",
            default: "5",
            choices: vec![
                ("1", "2X"),
                ("2", "3X"),
                ("3", "4X"),
                ("4", "5X"),
                ("5", "6X（默认）"),
            ],
        },
        Parameter {
            key: "force_multiplier",
            section: "FrameGeneration",
            ini_key: "ForceMultiplier",
            label: "请求倍率",
            default: "0",
            choices: vec![
                ("0", "跟随游戏"),
                ("2", "2X"),
                ("3", "3X"),
                ("4", "4X"),
                ("5", "5X"),
                ("6", "6X"),
            ],
        },
        Parameter {
            key: "dynamic_mfg",
            section: "FrameGeneration",
            ini_key: "DynamicMFG",
            label: "动态多帧（兼容 DX12）",
            default: "0",
            choices: toggle(),
        },
        Parameter {
            key: "dynamic_target_fps",
            section: "FrameGeneration",
            ini_key: "DynamicTargetFPS",
            label: "动态目标帧率",
            default: "0",
            choices: vec![
                ("0", "跟随游戏"),
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
            key: "mfg_logging",
            section: "Logging",
            ini_key: "Enabled",
            label: "补丁日志",
            default: "1",
            choices: toggle(),
        },
    ];
    for (key, ini_key, label) in [
        ("mfg_bilinear", "HardwareBilinear", "双线性采样优化"),
        ("conv13_shared", "Conv13SharedInput", "卷积输入复用 · k13"),
        ("conv0_shared", "Conv0SharedInput", "卷积输入复用 · k0"),
        ("residual_vector", "ResidualVectorLoads", "残差向量读取优化"),
    ] {
        fields.push(Parameter {
            key,
            section: "Optimizations",
            ini_key,
            label,
            default: "1",
            choices: toggle(),
        });
    }
    fields
}

pub fn read_values(bytes: &[u8], profile: &str) -> Result<Values> {
    let (text, _) = crate::diagnostics::decode_ini(bytes)?;
    let mut values = Values::new();
    for p in parameters(profile) {
        if let Some(value) = crate::diagnostics::ini_value(&text, p.section, p.ini_key)?
            && p.choices.iter().any(|(v, _)| *v == value)
        {
            values.insert(p.key.into(), value);
        }
    }
    normalize(profile, &mut values);
    Ok(values)
}

/// Read only recognized deployment INIs, off the UI thread.
pub fn inspect(exe: &Path, catalog: &crate::cloud::Catalog) -> Result<Option<(String, Values)>> {
    let exe = crate::core::location(exe, false)?;
    let dir = exe.parent().unwrap();
    let record = crate::core::record(dir)?;
    let mut scheme = record.as_ref().and_then(|r| r.scheme_id.clone());
    if scheme.is_none() {
        for name in crate::core::PROXIES {
            let path = crate::core::no_links(&dir.join(name))?;
            if !path.is_file() || path.metadata()?.len() > 128 * 1024 * 1024 {
                continue;
            }
            let bytes = std::fs::read(&path)?;
            if crate::cleanup::image_digest(&bytes).is_some_and(|h| crate::delta::known_image(&h)) {
                scheme = Some("rtxfg-0.3.5-dx12-vulkan".into());
                break;
            }
            let sha = crate::core::hash(&bytes);
            scheme = catalog
                .packages
                .iter()
                .find(|p| p.files.iter().any(|f| f.name == name && f.sha256 == sha))
                .map(|p| p.scheme_id.clone());
            if scheme.is_some() {
                break;
            }
        }
    }
    let Some(scheme) = scheme else {
        return Ok(None);
    };
    let Some(policy) = catalog.scheme_policies.get(&scheme) else {
        return Ok(None);
    };
    let ini = crate::core::no_links(&dir.join(crate::core::INI))?;
    if !ini.is_file() {
        return Ok(None);
    }
    ensure!(ini.metadata()?.len() <= 1024 * 1024, "INI 文件过大");
    let bytes = std::fs::read(ini)?;
    let mut values = read_values(&bytes, &policy.parameter_profile)?;
    if Context::new(&scheme, policy, &exe).delta {
        let (text, _) = crate::diagnostics::decode_ini(&bytes)?;
        if let Some(n) =
            crate::diagnostics::ini_value(&text, "Compatibility", "DeltaForceGeneratedFrames")?
            && matches!(n.as_str(), "0" | "1" | "2" | "3")
        {
            let enabled = crate::diagnostics::ini_value(
                &text,
                "Compatibility",
                "DeltaForcePrivateStreamline",
            )?
            .as_deref()
                == Some("1");
            // A manual test INI may request fewer frames than its generic cap.
            let cap: u32 = values
                .get("max_generated_frames")
                .and_then(|s| s.parse().ok())
                .unwrap_or(3);
            let count: u32 = n.parse()?;
            let effective = if count > 1 && !enabled {
                1
            } else if cap > 0 {
                count.min(cap)
            } else {
                count
            };
            values.insert("max_generated_frames".into(), effective.to_string());
        }
    }
    Ok(Some((scheme, values)))
}
