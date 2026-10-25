//! RTX Encore JSONC v4, including 1.3 foveated NR and Turing Smooth Motion.
//! Legacy paths remain writable for games whose patch has not been upgraded.
//! Deployment never requires a fixed DLL hash.
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
pub const INTEL_FILES: [&str; 4] = [
    "rtx-encore-runtimes/intel/libxess.dll",
    "rtx-encore-runtimes/intel/libxess_fg.dll",
    "rtx-encore-runtimes/intel/libxell.dll",
    "rtx-encore-runtimes/intel/LICENSE.txt",
];
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
pub const ENTRIES: [&str; 20] = {
    let mut entries = ["rtx-encore.asi"; 20];
    let mut i = 0;
    while i < PROXIES.len() {
        entries[i] = PROXIES[i];
        i += 1;
    }
    entries
};
pub const DEFAULT_CONFIG: &[u8] = include_bytes!("../assets/encore-defaults.jsonc");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FieldKind {
    Bool,
    Choice,
    Integer,
    Decimal,
    Hotkey,
    InheritDecimal,
    InheritBool,
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
    #[serde(default)]
    pub since: Option<String>,
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
pub fn supports_120(version: Option<&str>) -> bool {
    version
        .and_then(|v| semver::Version::parse(v).ok())
        .is_some_and(|v| v >= semver::Version::parse("1.2.0-beta.1").expect("constant"))
}
pub fn supports_130(version: Option<&str>) -> bool {
    version
        .and_then(|v| semver::Version::parse(v).ok())
        .is_some_and(|v| v >= semver::Version::new(1, 3, 0))
}
fn removed_open_field(key: &str) -> bool {
    key == "nrEngine" || key.starts_with("nrOpen")
}
pub fn field_available(field: &Field, version: Option<&str>) -> bool {
    if supports_130(version) && removed_open_field(&field.key) {
        return false;
    }
    if field.key == "smoothMotionForce" && supports_120(version) {
        return false;
    }
    if field.key == "providerPatchPolicy"
        && version
            .and_then(|v| semver::Version::parse(v).ok())
            .is_some_and(|v| v >= semver::Version::new(1, 2, 1))
    {
        return false;
    }
    field.since.as_deref().is_none_or(|since| {
        version
            .and_then(|v| semver::Version::parse(v).ok())
            .zip(semver::Version::parse(since).ok())
            .is_some_and(|(v, min)| v >= min)
    })
}
pub fn requires_restart(field: &Field, version: Option<&str>) -> bool {
    field.restart
        && !(supports_120(version) && matches!(field.key.as_str(), "dlssSrPreset" | "dlssRrPreset"))
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
    if key == "nrPreset" {
        return ["0", "1", "2", "3"].contains(&value);
    }
    let Some(f) = field(key) else {
        return false;
    };
    match f.kind {
        FieldKind::Bool => matches!(value, "0" | "1"),
        FieldKind::Choice => f.choices.iter().any(|c| c.value == value),
        FieldKind::Hotkey => valid_hotkey(value),
        FieldKind::InheritBool => matches!(value, "inherit" | "0" | "1"),
        FieldKind::InheritDecimal if value == "inherit" => true,
        FieldKind::Integer | FieldKind::Decimal | FieldKind::InheritDecimal => {
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
pub fn field_enabled(key: &str, values: &Values, _gpu: Option<i32>) -> bool {
    let Some(f) = field(key) else {
        return false;
    };
    let get = |k: &str| {
        values
            .get(k)
            .map(String::as_str)
            .or_else(|| field(k).map(|f| f.default.as_str()))
    };
    f.dependencies.iter().all(|dep| {
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
    })
}

/// Keep the four custom values out of the normal preset view. The values are
/// retained when another preset is selected, and reset still covers them.
pub fn field_visible(field: &Field, values: &Values) -> bool {
    !matches!(
        field.key.as_str(),
        "nrFoveaWidth" | "nrFoveaHeight" | "nrFoveaFalloff" | "nrFoveaEdge"
    ) || values.get("nrFoveaPreset").is_some_and(|v| v == "custom")
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
    ("nrPreset", "dlssNrPreset"),
];
fn normalized(key: &str, value: &Value) -> Value {
    if key == "dlssNrPreset"
        && let Some(n) = value.as_u64()
    {
        return json!(match n {
            0 | 3 => "auto",
            1 => "a",
            2 => "b",
            _ => return value.clone(),
        });
    }
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
        "dlssPresets",
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
        "renderingEngines",
        "xefg",
        "superResolutionInputs",
        "superResolutionSharpening",
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
                "passTuning",
                "passModels",
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
    let mut canonical = doc.get(path)?.cloned();
    let key = path.last().context("Empty field path")?;
    if key == "flipMeteringBypass"
        && let Some(value) = doc.get(&self::path(&["frameGeneration", "flipMeteringBypass"]))?
    {
        canonical = Some(value.clone());
    }
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
        FieldKind::InheritDecimal | FieldKind::InheritBool if value.as_str() == Some("inherit") => {
            "inherit".into()
        }
        FieldKind::InheritDecimal => value
            .as_f64()
            .context("Invalid inherited decimal")?
            .to_string(),
        FieldKind::InheritBool => if value.as_bool().context("Invalid inherited boolean")? {
            "1"
        } else {
            "0"
        }
        .into(),
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
        FieldKind::InheritDecimal | FieldKind::InheritBool if value == "inherit" => {
            json!("inherit")
        }
        FieldKind::InheritDecimal => json!(value.parse::<f64>()?),
        FieldKind::InheritBool => json!(value == "1"),
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
        if key == "nrPreset" {
            if !values.contains_key("dlssNrPreset") {
                let mapped = normalized("dlssNrPreset", &json!(value.parse::<u64>()?));
                out = document(&out)?.set(&path(&["dlssPresets", "dlssNrPreset"]), mapped)?;
            }
            continue;
        }
        let f = field(key).context("Unknown Encore field")?;
        if key == "flipMeteringBypass" {
            let doc = document(&out)?;
            let modern = path(&["frameGeneration", key]);
            if doc.get(&modern)?.is_some() {
                let (_, locations) = resolve(&doc, &f.path)?;
                for old in locations.into_iter().filter(|p| p != &modern) {
                    out = document(&out)?.remove(&old)?;
                }
                out = document(&out)?.set(&modern, encode(f, value)?)?;
            } else {
                // An old game still needs the legacy section until its mod is upgraded.
                out = doc.set(&f.path, encode(f, value)?)?;
            }
            continue;
        }
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

pub fn used_open_nr(bytes: &[u8]) -> Result<bool> {
    let f = field("nrEngine").expect("legacy engine metadata");
    Ok(resolve(&document(bytes)?, &f.path)?
        .0
        .is_some_and(|v| v == "opendlss"))
}

/// 1.3 removed OpenNR. Retire only known keys inside documented containers;
/// unrelated objects, comments, shortcuts and menu state are not rewritten.
/// The upgrade journal retains the original bytes for rollback.
pub fn migrate_130(bytes: &[u8]) -> Result<Vec<u8>> {
    let open = used_open_nr(bytes)?;
    let doc = document(bytes)?;
    let mut retired = Vec::new();
    for f in fields().iter().filter(|f| removed_open_field(&f.key)) {
        retired.extend(resolve(&doc, &f.path)?.1);
    }
    let mut out = bytes.to_vec();
    for p in retired {
        out = document(&out)?.remove(&p)?;
    }
    if open {
        // Remove flat/legacy aliases as well so the DLL cannot read a stale true.
        let f = field("nrEnabled").expect("NR activation metadata");
        let (_, aliases) = resolve(&document(&out)?, &f.path)?;
        for p in aliases.into_iter().filter(|p| p != &f.path) {
            out = document(&out)?.remove(&p)?;
        }
        out = document(&out)?.set(&f.path, json!(false))?;
    }
    read(&out)?;
    Ok(out)
}

pub fn read_for_version(bytes: &[u8], version: Option<&str>) -> Result<Values> {
    if supports_130(version) {
        read(&migrate_130(bytes)?)
    } else {
        read(bytes)
    }
}

pub fn configure_for_version(
    bytes: &[u8],
    values: &Values,
    version: Option<&str>,
) -> Result<Vec<u8>> {
    for key in values.keys() {
        let f = field(if key == "nrPreset" {
            "dlssNrPreset"
        } else {
            key
        })
        .context("Unknown Encore field")?;
        ensure!(
            field_available(f, version),
            "Encore setting unavailable in installed version: {key}"
        );
    }
    if !supports_130(version) {
        return configure(bytes, values);
    }
    let open = used_open_nr(bytes)?;
    let migrated = migrate_130(bytes)?;
    let mut values = values.clone();
    if open {
        // Even a stale draft from the previous engine must not silently turn
        // NVIDIA NR on. Once migrated, an explicit later enable works normally.
        values.insert("nrEnabled".into(), "0".into());
    }
    configure(&migrated, &values)
}
/// 1.1.0 moves only the legacy NR model setting. Keep all unrelated bytes as-is.
pub fn migrate_nr(bytes: &[u8]) -> Result<Vec<u8>> {
    let doc = document(bytes)?;
    let canonical = path(&["dlssPresets", "dlssNrPreset"]);
    let (value, locations) = resolve(&doc, &canonical)?;
    let old: Vec<_> = locations.into_iter().filter(|p| p != &canonical).collect();
    if old.is_empty() {
        return Ok(bytes.to_vec());
    }
    read(bytes)?;
    let mut out = bytes.to_vec();
    for p in old {
        out = document(&out)?.remove(&p)?;
    }
    if let Some(value) = value {
        out = document(&out)?.set(&canonical, value)?;
    }
    Ok(out)
}
/// Called only when deploying Encore 1.1.2 or newer; preserves comments and
/// unknown fields, and prevents a stale legacy key from overriding the new one.
pub fn migrate_frame_pacing(bytes: &[u8]) -> Result<Vec<u8>> {
    read(bytes)?;
    let doc = document(bytes)?;
    let canonical = path(&["frameGeneration", "flipMeteringBypass"]);
    let (value, locations) = resolve(&doc, &canonical)?;
    let mut out = bytes.to_vec();
    for old in locations.into_iter().filter(|p| p != &canonical) {
        out = document(&out)?.remove(&old)?;
    }
    document(&out)?.set(&canonical, value.unwrap_or(json!(false)))
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
            if f.key == "flipMeteringBypass" {
                resolve(&doc, &f.path)?.0.is_some()
            } else {
                doc.get(&f.path)?.is_some()
            },
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
        ("nrFoveation", "0"),
    ] {
        ensure!(
            values.get(key).is_some_and(|v| v == value),
            "Unsafe Encore package default: {key}"
        );
    }
    Ok(())
}
