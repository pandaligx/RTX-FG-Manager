//! RTX Encore beta.2 JSONC v4. Field metadata comes from an isolated upstream
//! configuration extraction; deployment never requires a fixed DLL hash.
use crate::{jsonc::Document, presets::Values};
use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{collections::BTreeSet, sync::OnceLock};

pub const PROFILE: &str = "rtx_encore_json_v4";
pub const BACKEND: &str = "encore";
pub const CONFIG: &str = "rtx-encore.jsonc";
pub const NOTICES: &str = "rtx-encore-THIRD-PARTY-NOTICES.md";
pub const SCHEME: &str = "dlssg-transfusion-1.4.5.3";
pub const SOURCE: &str = "https://github.com/SilyNoMeta/rtx-encore";
pub const PROXIES: [&str; 19] = [
    "version.dll",
    "dinput8.dll",
    "winmm.dll",
    "dxgi.dll",
    "d3d9.dll",
    "d3d10.dll",
    "d3d11.dll",
    "d3d12.dll",
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
pub const DEFAULT_CONFIG: &[u8] = include_bytes!("../assets/encore-defaults.jsonc");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FieldKind {
    Bool,
    Choice,
    Integer,
    Decimal,
    Hotkey,
}
#[derive(Debug, Clone, Deserialize)]
pub struct Choice {
    pub value: String,
    pub label: String,
}
#[derive(Debug, Clone, Deserialize)]
pub struct Field {
    pub key: String,
    pub path: Vec<String>,
    pub label: String,
    pub group: String,
    pub kind: FieldKind,
    pub default: String,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub choices: Vec<Choice>,
    pub restart: bool,
    pub dependencies: Vec<String>,
    pub help: String,
    pub optional: bool,
}
pub fn fields() -> &'static [Field] {
    static FIELDS: OnceLock<Vec<Field>> = OnceLock::new();
    FIELDS.get_or_init(|| {
        serde_json::from_str(include_str!("../assets/encore-fields.json"))
            .expect("reviewed Encore field metadata")
    })
}
pub fn defaults() -> Values {
    fields()
        .iter()
        .map(|f| (f.key.clone(), f.default.clone()))
        .collect()
}
pub fn default_config() -> &'static [u8] {
    DEFAULT_CONFIG
}
pub fn field(key: &str) -> Option<&'static Field> {
    fields().iter().find(|f| f.key == key)
}

pub fn valid_hotkey(value: &str) -> bool {
    if value.is_empty() {
        return true;
    }
    if value.len() > 512 || value.split(',').count() > 16 {
        return false;
    }
    value.split(',').all(|combo| {
        let mut modifiers = BTreeSet::new();
        let parts: Vec<_> = combo
            .split('+')
            .map(|p| p.trim().to_ascii_lowercase())
            .collect();
        if parts.is_empty() || parts.iter().any(String::is_empty) {
            return false;
        }
        for modifier in &parts[..parts.len() - 1] {
            let normalized = if modifier == "control" {
                "ctrl"
            } else {
                modifier.as_str()
            };
            if !["ctrl", "alt", "shift", "win"].contains(&normalized)
                || !modifiers.insert(normalized)
            {
                return false;
            }
        }
        let key = parts.last().unwrap();
        (key.len() == 1 && key.bytes().all(|b| b.is_ascii_alphanumeric()))
            || key
                .strip_prefix('f')
                .and_then(|n| n.parse::<u32>().ok())
                .is_some_and(|n| (1..=24).contains(&n))
            || [
                "pageup",
                "pagedown",
                "end",
                "home",
                "left",
                "right",
                "up",
                "down",
                "insert",
                "delete",
                "space",
                "tab",
                "backspace",
                "enter",
                "pause",
                "esc",
                "escape",
                "plus",
                "comma",
                "minus",
                "period",
                "nummultiply",
                "numadd",
                "numsubtract",
                "numdecimal",
                "numdivide",
            ]
            .contains(&key.as_str())
            || key
                .strip_prefix("num")
                .is_some_and(|n| n.len() == 1 && n.as_bytes()[0].is_ascii_digit())
    })
}
pub fn valid_value(key: &str, value: &str) -> bool {
    let Some(f) = field(key) else {
        return false;
    };
    match f.kind {
        FieldKind::Bool => matches!(value, "0" | "1"),
        FieldKind::Choice => f.choices.iter().any(|c| c.value == value),
        FieldKind::Hotkey => valid_hotkey(value),
        FieldKind::Integer | FieldKind::Decimal => {
            if value.is_empty() || value.trim() != value {
                return false;
            }
            if f.kind == FieldKind::Integer
                && (!value.bytes().all(|b| b.is_ascii_digit()) || value.parse::<u64>().is_err())
            {
                return false;
            }
            value.parse::<f64>().is_ok_and(|n| {
                n.is_finite()
                    && f.min.is_none_or(|min| n >= min)
                    && f.max.is_none_or(|max| n <= max)
                    && (f.kind != FieldKind::Decimal || n.abs() <= f32::MAX as f64)
            })
        }
    }
}
pub fn validate_values(values: &Values) -> Result<()> {
    for (key, value) in values {
        ensure!(valid_value(key, value), "Invalid Encore setting: {key}");
    }
    Ok(())
}
pub fn field_enabled(key: &str, values: &Values, gpu: Option<i32>) -> bool {
    let Some(f) = field(key) else {
        return false;
    };
    let get = |k: &str| {
        values
            .get(k)
            .map(String::as_str)
            .or_else(|| field(k).map(|f| f.default.as_str()))
    };
    let dependencies = f.dependencies.iter().all(|dep| {
        if let Some((k, v)) = dep.split_once(">=") {
            get(k)
                .and_then(|n| n.parse::<u32>().ok())
                .zip(v.parse::<u32>().ok())
                .is_some_and(|(a, b)| a >= b)
        } else if let Some((k, v)) = dep.split_once('=') {
            get(k) == Some(v)
        } else {
            false
        }
    });
    dependencies
        && match (key, gpu) {
            (key, Some(0)) if key.starts_with("nrOpen") => false,
            ("smoothMotionEnabled" | "smoothMotionApi", Some(series)) => series == 1,
            ("nrPrecision", Some(series)) => series == 0 || series == 1,
            _ => true,
        }
}

fn document(bytes: &[u8]) -> Result<Document> {
    let doc = Document::parse(bytes)?;
    if let Some(v) = doc.value.get("configVersion") {
        ensure!(
            v.as_u64().is_some_and(|v| (1..=4).contains(&v)),
            "Unsupported Encore configuration version"
        );
    }
    Ok(doc)
}
const RENAMES: &[(&str, &str)] = &[
    ("blackwellTransfusion", "highMultiplierQuality"),
    ("optimizedKernels", "fastFrameGeneration"),
    ("qualityValidWarp", "frameProtection"),
    ("qualityPolicy", "protectionTuning"),
    ("smoothMotionSm86", "smoothMotionEnabled"),
    ("smoothMotionSm86Api", "smoothMotionApi"),
    ("nrReplaceKernels", "nrFastProcessing"),
    ("nrOpenF16Weights", "nrOpenVramForSpeed"),
    ("nrOpenSm86Gemm32", "nrOpenFastProjection"),
    ("nrOpenSm86Fast", "nrOpenFast"),
    ("nrOpenNativeQkvPairs", "nrOpenSharedLoads"),
    ("gpuArchitecture", "gpuSeries"),
    ("patchFlipMetering", "flipMeteringBypass"),
];
fn normalized(key: &str, value: &Value) -> Value {
    let mapped = match (key, value.as_str()) {
        ("protectionTuning", Some("explained-warp")) => Some("refined"),
        ("protectionTuning", Some("transfusion")) => Some("classic"),
        ("gpuSeries", Some("ada")) => Some("rtx40"),
        ("gpuSeries", Some("ampere")) => Some("rtx30"),
        ("gpuSeries", Some("turing")) => Some("rtx20"),
        ("nrOpenBackend", Some("native")) => Some("rtx40"),
        ("nrOpenBackend", Some("sm86")) => Some("rtx30"),
        _ => None,
    };
    mapped.map_or_else(|| value.clone(), |s| json!(s))
}
// Only documented configuration containers are searched; similarly named keys
// in unknown vendor objects must remain entirely untouched.
fn supported_container(path: &[String]) -> bool {
    if path.is_empty() {
        return true;
    }
    let roots = [
        "frameGeneration",
        "smoothMotion",
        "dlssSuperResolution",
        "nativeMenuFeatures",
        "neuralRendering",
        "overlay",
        "imageQuality",
        "hudUi",
        "general",
        "compatibility",
        "diagnostics",
        "keyboardShortcuts",
        "menuState",
    ];
    if path.len() == 1 {
        return roots.contains(&path[0].as_str());
    }
    if path.len() == 2 {
        return (path[0] == "neuralRendering"
            && [
                "core",
                "appearance",
                "passStyles",
                "optimizations",
                "experimental",
                "openExperimental",
                "advanced",
            ]
            .contains(&path[1].as_str()))
            || (path[0] == "overlay" && ["metrics", "diagnostics"].contains(&path[1].as_str()));
    }
    false
}
fn locations(
    value: &Value,
    key: &str,
    path: &mut Vec<String>,
    out: &mut Vec<(Vec<String>, Value)>,
) {
    let Some(obj) = value.as_object() else {
        return;
    };
    if !supported_container(path) {
        return;
    }
    for (name, v) in obj {
        path.push(name.clone());
        if name == key || RENAMES.iter().any(|(old, new)| *old == name && *new == key) {
            out.push((path.clone(), normalized(key, v)));
        }
        if v.is_object() {
            locations(v, key, path, out);
        }
        path.pop();
    }
}
fn resolve(doc: &Document, path: &[String]) -> Result<(Option<Value>, Vec<Vec<String>>)> {
    // Checking canonical parents also rejects a malformed known section even
    // when an equivalent flat legacy key is available.
    let canonical = doc.get(path)?.cloned();
    let key = path.last().context("Empty field path")?;
    let mut candidates = Vec::new();
    locations(&doc.value, key, &mut Vec::new(), &mut candidates);
    let aliases = candidates.iter().map(|(p, _)| p.clone()).collect();
    if let Some(v) = canonical {
        return Ok((Some(normalized(key, &v)), aliases));
    }
    let value = candidates.first().map(|(_, v)| v.clone());
    ensure!(
        candidates.iter().all(|(_, v)| Some(v) == value.as_ref()),
        "Conflicting legacy Encore values: {key}"
    );
    Ok((value, aliases))
}
fn path(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| (*s).into()).collect()
}
fn multiplier(doc: &Document) -> Result<u64> {
    let v = resolve(doc, &path(&["frameGeneration", "multiplier"]))?
        .0
        .unwrap_or(json!(4));
    let n = v.as_u64().context("Invalid Encore multiplier type")?;
    ensure!((2..=6).contains(&n), "Invalid Encore multiplier");
    Ok(n)
}
fn decode(f: &Field, value: &Value) -> Result<String> {
    let result = match f.kind {
        FieldKind::Bool => if value.as_bool().context("Invalid Encore boolean type")? {
            "1"
        } else {
            "0"
        }
        .to_string(),
        FieldKind::Integer => value
            .as_u64()
            .context("Invalid Encore integer type")?
            .to_string(),
        FieldKind::Decimal => value
            .as_f64()
            .context("Invalid Encore decimal type")?
            .to_string(),
        FieldKind::Choice if matches!(f.key.as_str(), "nrStyle" | "nrPreset") => value
            .as_u64()
            .context("Invalid Encore numeric choice")?
            .to_string(),
        FieldKind::Choice | FieldKind::Hotkey => value
            .as_str()
            .context("Invalid Encore string type")?
            .to_owned(),
    };
    ensure!(
        valid_value(&f.key, &result),
        "Invalid Encore value: {}",
        f.key
    );
    Ok(result)
}
fn encode(f: &Field, value: &str) -> Result<Value> {
    Ok(match f.kind {
        FieldKind::Bool => json!(value == "1"),
        FieldKind::Integer => json!(value.parse::<u64>()?),
        FieldKind::Decimal => json!(value.parse::<f64>()?),
        FieldKind::Choice if matches!(f.key.as_str(), "nrStyle" | "nrPreset") => {
            json!(value.parse::<u64>()?)
        }
        FieldKind::Choice | FieldKind::Hotkey => json!(value),
    })
}
pub fn read(bytes: &[u8]) -> Result<Values> {
    let doc = document(bytes)?;
    let mut values = defaults();
    let mult = multiplier(&doc)?;
    for f in fields() {
        if let Some(v) = resolve(&doc, &f.path)?.0 {
            let result = if f.key == "tf_mode" {
                match v.as_str().context("Invalid Encore mode type")? {
                    "fixed" => mult.to_string(),
                    "game" => "game".into(),
                    "dynamic" => "dynamic".into(),
                    _ => bail!("Invalid Encore mode"),
                }
            } else {
                decode(f, &v)?
            };
            values.insert(f.key.clone(), result);
        }
    }
    validate_values(&values)?;
    Ok(values)
}
pub fn configure(bytes: &[u8], values: &Values) -> Result<Vec<u8>> {
    validate_values(values)?;
    read(bytes)?;
    let mut out = bytes.to_vec();
    for (key, value) in values {
        let f = field(key).context("Unknown Encore field")?;
        if key == "tf_mode" {
            out = document(&out)?.set(
                &f.path,
                json!(if matches!(value.as_str(), "game" | "dynamic") {
                    value.as_str()
                } else {
                    "fixed"
                }),
            )?;
            if let Ok(n) = value.parse::<u64>() {
                out = document(&out)?.set(&path(&["frameGeneration", "multiplier"]), json!(n))?;
            }
        } else {
            out = document(&out)?.set(&f.path, encode(f, value)?)?;
        }
    }
    read(&out)?;
    Ok(out)
}
pub fn migrate(bytes: &[u8], explicit_overrides: &Values) -> Result<Vec<u8>> {
    validate_values(explicit_overrides)?;
    let doc = document(bytes)?;
    read(bytes)?;
    let mut paths: Vec<Vec<String>> = fields().iter().map(|f| f.path.clone()).collect();
    paths.push(path(&["frameGeneration", "multiplier"]));
    let mut changes = Vec::new();
    for p in paths {
        let (value, locations) = resolve(&doc, &p)?;
        if let Some(v) = value {
            changes.push((p, v, locations));
        }
    }
    let mut out = bytes.to_vec();
    for (canonical, value, locations) in changes {
        for old in locations {
            if old != canonical {
                out = document(&out)?.remove(&old)?;
            }
        }
        out = document(&out)?.set(&canonical, value)?;
    }
    out = document(&out)?.set(&path(&["configVersion"]), json!(4))?;
    configure(&out, explicit_overrides)
}
pub fn validate_template(bytes: &[u8]) -> Result<()> {
    let doc = document(bytes)?;
    ensure!(
        doc.value["configVersion"] == 4,
        "Encore package requires configuration schema 4"
    );
    for f in fields().iter().filter(|f| !f.optional) {
        ensure!(
            doc.get(&f.path)?.is_some(),
            "Encore package missing {}",
            f.path.join(".")
        );
    }
    ensure!(
        doc.get(&path(&["frameGeneration", "multiplier"]))?
            .is_some(),
        "Encore package missing multiplier"
    );
    read(bytes)?;
    Ok(())
}
pub fn validate_cloud_config(bytes: &[u8]) -> Result<()> {
    validate_template(bytes)?;
    let values = read(bytes)?;
    for (key, value) in [
        ("tf_mode", "game"),
        ("gpuSeries", "auto"),
        ("smoothMotionEnabled", "0"),
        ("nrEnabled", "0"),
    ] {
        ensure!(
            values.get(key).is_some_and(|v| v == value),
            "Unsafe Encore package default: {key}"
        );
    }
    Ok(())
}
