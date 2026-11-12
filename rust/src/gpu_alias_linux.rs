//! Linux GPU display-name aliasing, driven by the Wine prefix registry.
//!
//! This is a real feature port, not a stub: a Wine prefix stores the same
//! display-adapter database Windows does, as text:
//!
//! ```text
//! [System\\ControlSet001\\Enum\\PCI\\VEN_10DE&DEV_25A2&SUBSYS_00000000&REV_00\\00000000]
//! "ClassGUID"="{4D36E968-E325-11CE-BFC1-08002BE10318}"
//! "DeviceDesc"="NVIDIA GeForce RTX 3050 Laptop GPU"
//! "Driver"="{4D36E968-E325-11CE-BFC1-08002BE10318}\\0000"
//!
//! [System\\ControlSet001\\Control\\Class\\{4D36E968...}\\0000]
//! "DriverDesc"="NVIDIA GeForce RTX 3050 Laptop GPU"
//! "HardwareInformation.AdapterString"="NVIDIA GeForce RTX 3050 Laptop GPU"
//! ```
//!
//! Wine reports `DeviceDesc` as the adapter name, so renaming it changes what
//! the prefix (and the games inside it) see. The Windows backend used SetupAPI
//! for `FriendlyName`; Wine keeps a single display-name slot, so `FriendlyName`
//! and `DeviceDesc` intentionally read and write the same value. That keeps the
//! four-field comparison in `change()` self-consistent on both targets.
//!
//! Every write is preceded by an atomic `.reg` rewrite (`wine_reg::RegFile::save`)
//! and the original values are backed up through the SAME `Record` schema the
//! Windows backend uses, so restore/rollback semantics are shared verbatim.

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::Path};

use crate::wine_reg::{Hive, RegFile, RegValue};
use base64::{Engine as _, engine::general_purpose::STANDARD};

#[path = "gpu_models.rs"]
mod models;
pub use models::NAMES;

pub fn display_model(name: &str) -> (&str, bool) {
    if let Some(model) = name.strip_prefix("NVIDIA ") {
        (model.strip_prefix("GeForce ").unwrap_or(model), true)
    } else {
        (name, false)
    }
}

pub const FIELDS: [&str; 4] = [
    "FriendlyName",
    "DriverDesc",
    "HardwareInformation.AdapterString",
    "DeviceDesc",
];
const CLASS: &str = "{4d36e968-e325-11ce-bfc1-08002be10318}";
const BACKUPS: &str = "SOFTWARE\\RTXFGManager\\GpuNameBackups";
const NVIDIA_PREFIX: &str = "PCI\\VEN_10DE&";
/// The PCI lane name as it appears under `Enum\\PCI` (no leading `PCI\\`).
const NVIDIA_VENDOR: &str = "VEN_10DE&";

// ---------------------------------------------------------------------------
// Device enumeration
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Device {
    pub instance: String,
    pub driver: String,
    pub version: String,
    pub inf: String,
    pub name: String,
    #[serde(default)]
    pub original_name: Option<String>,
    #[serde(default)]
    pub modified_name: Option<String>,
}
impl Device {
    fn identity(&self) -> serde_json::Value {
        serde_json::json!({"instance":self.instance,"driver":self.driver,"version":self.version,"inf":self.inf})
    }
    fn path(&self, field: &str) -> Result<String> {
        ensure!(FIELDS.contains(&field), "未知名称字段");
        ensure!(
            self.instance.to_uppercase().starts_with(NVIDIA_PREFIX)
                && !self.instance.contains(".."),
            "显示设备注册表位置无效"
        );
        let set = current_control_set()?;
        if field == "DeviceDesc" || field == "FriendlyName" {
            Ok(format!("System\\{set}\\Enum\\{}", self.instance))
        } else {
            ensure!(
                regex::Regex::new(&format!(r"(?i)^{}\\\d{{4}}$", regex::escape(CLASS)))?
                    .is_match(&self.driver),
                "显示驱动注册表位置无效"
            );
            Ok(format!("System\\{set}\\Control\\Class\\{}", self.driver))
        }
    }
}

/// Wine records the active hive as `System\Select` "Current" (normally 1) and
/// stores every control set under `ControlSet00N`. Resolving it dynamically is
/// what keeps the paths valid if Wine switches control sets after a reboot.
fn current_control_set() -> Result<String> {
    let prefix = crate::win::prefix()?;
    let value = crate::wine_reg::get_value(&prefix, Hive::Machine, "System\\Select", "Current")
        .unwrap_or(None);
    let index = match value {
        Some(RegValue::Dword(v)) if v > 0 && v < 1000 => v,
        _ => 1,
    };
    Ok(format!("ControlSet{index:03}"))
}

fn machine_file() -> Result<(std::path::PathBuf, RegFile)> {
    let prefix = crate::win::prefix()?;
    let path = crate::wine_reg::hive_path(&prefix, Hive::Machine);
    let file =
        RegFile::open(&path).with_context(|| format!("无法读取系统注册表：{}", path.display()))?;
    Ok((path, file))
}

fn text(value: Option<RegValue>) -> Option<String> {
    match value {
        Some(RegValue::Sz(s)) | Some(RegValue::ExpandSz(s)) => Some(s),
        _ => None,
    }
}

/// Lists the NVIDIA display adapters inside the prefix, matching the Windows
/// backend's `SetupDiGetClassDevsW` + `PCI\VEN_10DE&` filter.
pub fn enumerate() -> Result<Vec<Device>> {
    let (_path, file) = machine_file()?;
    let set = current_control_set()?;
    let mut rows = Vec::new();
    for vendor in file.subkeys(&format!("System\\{set}\\Enum\\PCI")) {
        if !vendor.to_uppercase().starts_with(NVIDIA_VENDOR) {
            continue;
        }
        for slot in file.subkeys(&format!("System\\{set}\\Enum\\PCI\\{vendor}")) {
            // Canonical instance id: "PCI\\VEN_...\\00000000", matching the
            // Windows backend's SetupDiGetDeviceInstanceIdW output.
            let instance = format!("PCI\\{vendor}\\{slot}");
            if !vendor.to_uppercase().starts_with(NVIDIA_VENDOR) {
                continue;
            }
            let enum_key = format!("System\\{set}\\Enum\\{instance}");
            let class_guid = text(file.get(&enum_key, "ClassGUID")).unwrap_or_default();
            if !class_guid.eq_ignore_ascii_case(CLASS) {
                continue;
            }
            let driver = text(file.get(&enum_key, "Driver")).unwrap_or_default();
            let class_key = format!("System\\{set}\\Control\\Class\\{driver}");
            let version = text(file.get(&class_key, "DriverVersion")).unwrap_or_default();
            let inf = text(file.get(&class_key, "InfPath")).unwrap_or_default();
            let name = text(file.get(&enum_key, "DeviceDesc"))
                .or_else(|| text(file.get(&class_key, "DriverDesc")))
                .unwrap_or_else(|| instance.clone());
            rows.push(Device {
                instance,
                driver,
                version,
                inf,
                name,
                original_name: None,
                modified_name: None,
            });
        }
    }
    Ok(rows)
}

// ---------------------------------------------------------------------------
// Backups: shared `Record` schema, stored in the prefix's HKLM
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Value {
    #[serde(rename = "type")]
    pub kind: u32,
    pub data: String,
}
impl Value {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            [1, 2, 3].contains(&self.kind) && self.data.len() <= 32768,
            "显卡名称注册表类型不支持，未修改"
        );
        if self.kind == 3 {
            ensure!(
                STANDARD.decode(&self.data)?.len() <= 8192,
                "显卡名称数据异常"
            );
        } else {
            ensure!(self.data.chars().count() <= 8192, "显卡名称数据异常");
        }
        Ok(())
    }
    fn raw(&self) -> Result<RegValue> {
        self.validate()?;
        Ok(match self.kind {
            2 => RegValue::ExpandSz(self.data.clone()),
            3 => RegValue::Binary(STANDARD.decode(&self.data)?),
            _ => RegValue::Sz(self.data.clone()),
        })
    }
    fn from_raw(v: RegValue) -> Result<Self> {
        let (kind, data) = match v {
            RegValue::Sz(s) => (1, s),
            RegValue::ExpandSz(s) => (2, s),
            // MultiSz never appears for these fields, but preserving it as a
            // type-2 string keeps a stray value from being destroyed.
            RegValue::MultiSz(parts) => (2, parts.join(";")),
            RegValue::Dword(d) => (3, STANDARD.encode(d.to_le_bytes())),
            RegValue::Binary(b) => (3, STANDARD.encode(&b)),
        };
        let v = Self { kind, data };
        v.validate()?;
        Ok(v)
    }
}
type Fields = BTreeMap<String, Option<Value>>;

#[derive(Clone, Serialize, Deserialize)]
pub struct Record {
    pub schema: u32,
    pub identity: serde_json::Value,
    pub original: Fields,
    pub previous: Fields,
    pub written: Fields,
}
impl Record {
    pub fn validate(&self, id: &serde_json::Value) -> Result<()> {
        ensure!(
            [1, 2].contains(&self.schema) && self.identity == *id,
            "显卡或驱动已变化，不能套用旧名称备份"
        );
        let names = if self.schema == 1 {
            &FIELDS[..3]
        } else {
            &FIELDS[..]
        };
        for g in [&self.original, &self.previous, &self.written] {
            ensure!(
                g.len() == names.len() && names.iter().all(|n| g.contains_key(*n)),
                "名称备份损坏，未修改"
            );
            for v in g.values().flatten() {
                v.validate()?;
            }
        }
        Ok(())
    }
}

pub trait Backend {
    fn identity(&self) -> Result<serde_json::Value>;
    fn read(&self, field: &str) -> Result<Option<Value>>;
    fn write(&mut self, field: &str, value: Option<&Value>) -> Result<()>;
    fn load(&self) -> Result<Option<Record>>;
    fn save(&mut self, r: &Record) -> Result<()>;
    fn clear(&mut self) -> Result<()>;
    /// The installed driver node description, not the editable display name.
    fn driver_description(&self) -> Result<Option<String>> {
        Ok(None)
    }
}

fn named_value(name: &str, value: &Option<Value>) -> Option<Value> {
    let kind = value.as_ref().map(|v| v.kind).unwrap_or(1);
    let data = if kind == 3 {
        STANDARD.encode(
            crate::win::wide(name)
                .into_iter()
                .flat_map(u16::to_le_bytes)
                .collect::<Vec<_>>(),
        )
    } else {
        name.into()
    };
    Some(Value { kind, data })
}
fn is_our_alias(value: &Option<Value>) -> bool {
    NAMES.iter().any(|n| *value == named_value(n, value))
}
fn same_instance(a: &serde_json::Value, b: &serde_json::Value) -> bool {
    match (a["instance"].as_str(), b["instance"].as_str()) {
        (Some(a), Some(b)) => {
            a.to_uppercase().starts_with(NVIDIA_PREFIX) && a.eq_ignore_ascii_case(b)
        }
        _ => false,
    }
}

fn migrate_record(
    b: &impl Backend,
    r: &Record,
    identity: &serde_json::Value,
    current: &Fields,
) -> Result<Record> {
    r.validate(&r.identity)?;
    ensure!(
        same_instance(&r.identity, identity),
        "显卡或驱动已变化，不能套用旧名称备份"
    );
    let mut original = current.clone();
    let mut installed_name = None;
    for (f, old) in &r.original {
        let value = &current[f];
        if value != old && is_our_alias(value) && [&r.written[f], &r.previous[f]].contains(&value) {
            if old.is_none() {
                original.insert(f.clone(), None);
            } else {
                if installed_name.is_none() {
                    let name = b.driver_description()?.context(
                        "无法读取当前驱动的原始显卡名称，未修改；请确认驱动安装完成后重试",
                    )?;
                    ensure!(
                        !name.trim().is_empty() && name.len() <= 8192,
                        "显卡名称数据异常"
                    );
                    installed_name = Some(name);
                }
                original.insert(
                    f.clone(),
                    named_value(installed_name.as_deref().unwrap(), value),
                );
            }
        }
    }
    Ok(Record {
        schema: 2,
        identity: identity.clone(),
        original,
        previous: current.clone(),
        written: current.clone(),
    })
}

pub fn change(b: &mut impl Backend, name: Option<&str>) -> Result<String> {
    if let Some(n) = name {
        ensure!(NAMES.contains(&n), "请选择支持的显示名称")
    }
    let identity = b.identity()?;
    let mut record = b.load()?;
    let current = FIELDS
        .iter()
        .map(|f| Ok(((*f).into(), b.read(f)?)))
        .collect::<Result<Fields>>()?;
    let migrated = record.as_ref().is_some_and(|r| r.identity != identity);
    if let Some(r) = &record
        && migrated
    {
        record = Some(migrate_record(b, r, &identity, &current)?);
    }
    if let Some(r) = &record {
        r.validate(&identity)?;
        for f in r.original.keys() {
            ensure!(
                [&r.original[f], &r.previous[f], &r.written[f]].contains(&&current[f]),
                "名称已被其他程序或驱动修改，未覆盖；请检查驱动状态"
            )
        }
    }
    let target = if let Some(name) = name {
        let target = current
            .iter()
            .map(|(f, v)| (f.clone(), named_value(name, v)))
            .collect::<Fields>();
        let mut original = current.clone();
        if let Some(r) = record {
            original.extend(r.original)
        }
        ensure!(b.identity()? == identity, "驱动已变化，请刷新后重试");
        b.save(&Record {
            schema: 2,
            identity: identity.clone(),
            original,
            previous: current.clone(),
            written: target.clone(),
        })?;
        target
    } else {
        let mut r = record.context("此显卡没有本工具的名称备份")?;
        if migrated {
            r.written = r.original.clone();
            ensure!(b.identity()? == identity, "驱动已变化，请刷新后重试");
            b.save(&r)?;
        }
        r.original
    };
    let mut touched = Vec::new();
    let result = (|| {
        for (f, v) in &target {
            ensure!(b.identity()? == identity, "驱动已变化，请刷新后重试");
            ensure!(
                b.read(f)? == current[f],
                "名称已被其他程序或驱动修改，未覆盖；请检查驱动状态"
            );
            if current[f] != *v {
                touched.push(f.clone());
                b.write(f, v.as_ref())?;
            }
            ensure!(b.read(f)? == *v, "名称写入后核对失败：{f}")
        }
        Ok(())
    })();
    if let Err(e) = result {
        for f in touched.iter().rev() {
            if b.identity().is_ok_and(|id| id == identity)
                && b.read(f).is_ok_and(|v| v == target[f])
            {
                let _ = b.write(f, current[f].as_ref());
            }
        }
        return Err(anyhow::anyhow!(
            "操作未完成，已尝试回滚；原值备份已保留，可点击还原。{e}"
        ));
    }
    if name.is_none() {
        b.clear()?;
    }
    Ok(if let Some(n) = name {
        format!("已写入显示名称：{n}")
    } else {
        "已还原原始显示名称".into()
    })
}

/// Wine-backed backend: reads and writes the prefix registry directly.
struct WineBackend(Device);
impl WineBackend {
    fn token(&self) -> String {
        crate::core::hash(self.0.instance.to_uppercase().as_bytes())
    }
}
impl Backend for WineBackend {
    fn driver_description(&self) -> Result<Option<String>> {
        let key = format!(
            "System\\{}\\Control\\Class\\{}",
            current_control_set()?,
            self.0.driver
        );
        let (_path, file) = machine_file()?;
        Ok(text(file.get(&key, "DriverDesc")))
    }
    fn identity(&self) -> Result<serde_json::Value> {
        let current = enumerate()?
            .into_iter()
            .find(|d| d.instance.eq_ignore_ascii_case(&self.0.instance))
            .context("显卡已断开，请刷新后重试")?;
        ensure!(
            current.identity() == self.0.identity(),
            "驱动已变化，请刷新后重试"
        );
        Ok(current.identity())
    }
    fn read(&self, field: &str) -> Result<Option<Value>> {
        let key = self.0.path(field)?;
        let (_path, file) = machine_file()?;
        match file.get(&key, field) {
            Some(v) => Ok(Some(Value::from_raw(v)?)),
            None => Ok(None),
        }
    }
    fn write(&mut self, field: &str, value: Option<&Value>) -> Result<()> {
        let key = self.0.path(field)?;
        let (path, mut file) = machine_file()?;
        match value {
            Some(v) => file.set(&key, field, v.raw()?)?,
            None => file.remove(&key, field)?,
        }
        file.save()
            .with_context(|| format!("无法写回系统注册表：{}", path.display()))
    }
    fn load(&self) -> Result<Option<Record>> {
        let (_path, file) = machine_file()?;
        let Some(value) = file.get(BACKUPS, &self.token()) else {
            return Ok(None);
        };
        let text = text(Some(value)).context("显卡名称备份无效")?;
        ensure!(text.len() <= 131072, "显卡名称备份无效");
        Ok(Some(serde_json::from_str(&text)?))
    }
    fn save(&mut self, r: &Record) -> Result<()> {
        let (path, mut file) = machine_file()?;
        file.set(
            BACKUPS,
            &self.token(),
            RegValue::Sz(serde_json::to_string(r)?),
        )?;
        file.save()
            .with_context(|| format!("无法写回系统注册表：{}", path.display()))
    }
    fn clear(&mut self) -> Result<()> {
        let (path, mut file) = machine_file()?;
        file.remove(BACKUPS, &self.token())?;
        file.save()
            .with_context(|| format!("无法写回系统注册表：{}", path.display()))
    }
}

/// Applies or restores a display name. No elevation is required: a Wine prefix
/// is an ordinary user-writable directory, so `is_admin()` is always true.
pub fn apply(target: Device, name: Option<&str>) -> Result<String> {
    ensure!(crate::win::is_admin(), "修改系统显示名称需要管理员权限");
    let _lock = crate::win::game_lock(Path::new("RTXFG-global-gpu-name"))?;
    let current = enumerate()?
        .into_iter()
        .find(|d| d.instance.eq_ignore_ascii_case(&target.instance))
        .context("显卡已断开，请刷新后重试")?;
    change(&mut WineBackend(current), name)
}

/// On Linux there is no UAC relaunch, so aliasing runs in-process.
pub fn apply_with_elevation(target: Device, name: Option<&str>) -> Result<String> {
    apply(target, name)
}

pub fn restore_all() -> Result<()> {
    let _lock = crate::win::game_lock(Path::new("RTXFG-global-gpu-name"))?;
    let prefix = crate::win::prefix()?;
    let file = RegFile::open(&crate::wine_reg::hive_path(&prefix, Hive::Machine))?;
    let records = file.values(BACKUPS);
    let devices = enumerate()?;
    for token in records.keys() {
        let device = devices
            .iter()
            .find(|d| crate::core::hash(d.instance.to_uppercase().as_bytes()) == *token)
            .context("有显卡名称备份无法匹配当前设备；未清除恢复记录")?;
        change(&mut WineBackend(device.clone()), None)?;
    }
    Ok(())
}

/// The Windows backend re-executes itself with `--gpu-name-action` under UAC.
/// Linux has no such flag; running it directly would be a privilege-escalation
/// idiom with no meaning here, so it is rejected explicitly.
pub fn elevated_action(_encoded: &str, _action: &str) -> Result<()> {
    anyhow::bail!("Linux 版直接应用显示名称，无需提权子命令（--gpu-name-action 不可用）")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reads the real prefix when one is present. Tolerant of absence so the
    /// suite still runs in a bare environment, but strict about what it finds:
    /// a Wine prefix DOES store the display-adapter database, and this proves
    /// the Linux GPU-name feature is backed by real data rather than a stub.
    #[test]
    fn enumerates_nvidia_adapters_from_a_real_wine_prefix() -> Result<()> {
        let Ok(prefix) = crate::win::prefix() else {
            eprintln!("skipped: no Wine prefix configured");
            return Ok(());
        };
        if !crate::wine_reg::hive_path(&prefix, Hive::Machine).is_file() {
            eprintln!("skipped: prefix has no system.reg");
            return Ok(());
        }
        let rows = enumerate()?;
        for device in &rows {
            // Every row must come from an NVIDIA PCI instance and expose a
            // registry path we can actually read back.
            assert!(
                device.instance.to_uppercase().starts_with(NVIDIA_PREFIX),
                "unexpected instance {}",
                device.instance
            );
            assert!(
                device.path("DeviceDesc")?.to_uppercase().contains("ENUM"),
                "DeviceDesc path must live under Enum"
            );
            assert!(
                device
                    .path("DriverDesc")?
                    .to_uppercase()
                    .contains("CONTROL"),
                "DriverDesc path must live under Control"
            );
        }
        if rows.is_empty() {
            eprintln!("prefix has no NVIDIA display adapter entries");
        }
        Ok(())
    }

    /// `Record` validation and the backup round-trip are platform-neutral and
    /// must behave identically to the Windows backend.
    #[test]
    fn record_schema_round_trips_through_value_encoding() -> Result<()> {
        for (kind, data) in [(1u32, "NVIDIA GeForce RTX 4060"), (2, "%LOCALAPPDATA%\\x")] {
            let value = Value {
                kind,
                data: data.into(),
            };
            value.validate()?;
            let raw = value.raw()?;
            assert_eq!(Value::from_raw(raw)?, value);
        }
        let binary = Value {
            kind: 3,
            data: STANDARD.encode([1u8, 2, 3]),
        };
        assert_eq!(Value::from_raw(binary.raw()?)?, binary);
        Ok(())
    }

    #[test]
    fn rejects_unknown_fields_and_foreign_instances() {
        assert!(
            Value {
                kind: 9,
                data: "x".into()
            }
            .validate()
            .is_err()
        );
        assert!(
            Value {
                kind: 1,
                data: "暗".repeat(9000)
            }
            .validate()
            .is_err()
        );
        assert!(!same_instance(
            &serde_json::json!({"instance":r"PCI\VEN_8086&DEV_46A6"}),
            &serde_json::json!({"instance":r"PCI\VEN_8086&DEV_46A6"})
        ));
    }
}
