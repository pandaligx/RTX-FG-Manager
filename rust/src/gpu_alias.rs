//! Reversible GPU display-name changes, preserving existing registry backups.
use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::Path};
use windows::{
    Win32::Devices::DeviceAndDriverInstallation::*,
    core::{GUID, PCWSTR},
};
use winreg::{RegKey, RegValue, enums::*};
pub const NAMES: [&str; 2] = ["NVIDIA GeForce RTX 4060", "NVIDIA GeForce RTX 5090"];

/// UAC helper accepts only a serialized device identity and a fixed action.
/// No caller-supplied output path, registry path or executable is accepted.
pub fn elevated_action(encoded: &str, action: &str) -> Result<()> {
    ensure!(crate::win::is_admin(), "未获得管理员权限，操作未完成");
    ensure!(encoded.len() <= 8192, "显示设备参数过长");
    let target: Device = serde_json::from_slice(&STANDARD.decode(encoded)?)?;
    let name = match action {
        "restore" => None,
        "4060" => Some(NAMES[0]),
        "5090" => Some(NAMES[1]),
        _ => anyhow::bail!("请选择支持的显示名称"),
    };
    let current = enumerate()?
        .into_iter()
        .find(|d| d.instance.eq_ignore_ascii_case(&target.instance))
        .context("显卡或驱动已变化，不能套用旧名称备份")?;
    apply(current, name)?;
    Ok(())
}

pub fn apply_with_elevation(target: Device, name: Option<&str>) -> Result<String> {
    if crate::win::is_admin() {
        return apply(target, name);
    }
    let action = match name {
        None => "restore",
        Some(n) if n == NAMES[0] => "4060",
        Some(n) if n == NAMES[1] => "5090",
        _ => anyhow::bail!("请选择支持的显示名称"),
    };
    let encoded = STANDARD.encode(serde_json::to_vec(&target)?);
    crate::win::elevated_gpu_action(&encoded, action)?;
    Ok(if let Some(n) = name {
        format!("已写入显示名称：{n}")
    } else {
        "已还原原始显示名称".into()
    })
}
pub const FIELDS: [&str; 4] = [
    "FriendlyName",
    "DriverDesc",
    "HardwareInformation.AdapterString",
    "DeviceDesc",
];
const CLASS: &str = "{4d36e968-e325-11ce-bfc1-08002be10318}";
const BACKUPS: &str = "SOFTWARE\\RTXFGManager\\GpuNameBackups";
fn flush(key: &RegKey) -> Result<()> {
    // SAFETY: The borrowed live registry key owns this handle throughout the flush.
    unsafe {
        windows::Win32::System::Registry::RegFlushKey(windows::Win32::System::Registry::HKEY(
            key.raw_handle().cast(),
        ))
        .ok()?;
    }
    Ok(())
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Device {
    pub instance: String,
    pub driver: String,
    pub version: String,
    pub inf: String,
    pub name: String,
}
impl Device {
    fn identity(&self) -> serde_json::Value {
        serde_json::json!({"instance":self.instance,"driver":self.driver,"version":self.version,"inf":self.inf})
    }
    fn path(&self, field: &str) -> Result<String> {
        ensure!(FIELDS.contains(&field), "未知名称字段");
        if field == "DeviceDesc" {
            ensure!(
                regex::Regex::new(r"(?i)^PCI\\VEN_10DE&[A-Z0-9_&]+\\[A-Z0-9_&]+$")?
                    .is_match(&self.instance),
                "显示设备注册表位置无效"
            );
            Ok(format!(
                "SYSTEM\\CurrentControlSet\\Enum\\{}",
                self.instance
            ))
        } else {
            ensure!(
                regex::Regex::new(&format!(r"(?i)^{}\\\d{{4}}$", regex::escape(CLASS)))?
                    .is_match(&self.driver),
                "显示驱动注册表位置无效"
            );
            Ok(format!(
                "SYSTEM\\CurrentControlSet\\Control\\Class\\{}",
                self.driver
            ))
        }
    }
}
struct Set(HDEVINFO);
impl Drop for Set {
    fn drop(&mut self) {
        // SAFETY: This set is created once and destroyed once by this owner.
        unsafe {
            let _ = SetupDiDestroyDeviceInfoList(self.0);
        }
    }
}
fn devices(mut f: impl FnMut(HDEVINFO, &mut SP_DEVINFO_DATA, &str) -> Result<()>) -> Result<()> {
    let guid = GUID::from_u128(0x4d36e968_e325_11ce_bfc1_08002be10318);
    // SAFETY: SetupAPI structures use ABI sizes, bounded UTF-16 buffers and a valid owned device set.
    unsafe {
        let set = Set(SetupDiGetClassDevsW(
            Some(&guid),
            PCWSTR::null(),
            None,
            DIGCF_PRESENT,
        )?);
        for i in 0..128 {
            let mut dev = SP_DEVINFO_DATA {
                cbSize: std::mem::size_of::<SP_DEVINFO_DATA>() as u32,
                ..Default::default()
            };
            match SetupDiEnumDeviceInfo(set.0, i, &mut dev) {
                Ok(()) => {}
                Err(e) if e.code() == windows::core::HRESULT::from_win32(259) => break,
                Err(e) => return Err(e.into()),
            }
            let mut buf = [0; 4096];
            SetupDiGetDeviceInstanceIdW(set.0, &dev, Some(&mut buf), None)?;
            let id = crate::win::from_wide(&buf);
            if id.to_uppercase().starts_with("PCI\\VEN_10DE&") {
                f(set.0, &mut dev, &id)?;
            }
        }
    }
    Ok(())
}
fn property(
    set: HDEVINFO,
    dev: &SP_DEVINFO_DATA,
    p: SETUP_DI_REGISTRY_PROPERTY,
) -> Result<Option<String>> {
    let mut buf = [0; 8192];
    let (mut kind, mut size) = (0, 0);
    // SAFETY: The property destination is sized and mutable for the duration of the call.
    match unsafe {
        SetupDiGetDeviceRegistryPropertyW(
            set,
            dev,
            p,
            Some(&mut kind),
            Some(&mut buf),
            Some(&mut size),
        )
    } {
        Ok(()) => {
            ensure!(
                kind == 1 && size <= 8192 && size % 2 == 0,
                "设备属性类型异常"
            );
            Ok(Some(crate::win::from_wide(
                &buf[..size as usize]
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|b| u16::from_le_bytes([b[0], b[1]]))
                    .collect::<Vec<_>>(),
            )))
        }
        Err(e)
            if [2, 13, 1168]
                .iter()
                .any(|c| e.code() == windows::core::HRESULT::from_win32(*c)) =>
        {
            Ok(None)
        }
        Err(e) => Err(e.into()),
    }
}
pub fn enumerate() -> Result<Vec<Device>> {
    let mut rows = Vec::new();
    devices(|set, dev, id| {
        let driver = property(set, dev, SPDRP_DRIVER)?.context("显示驱动注册表位置无效")?;
        let mut d = Device {
            instance: id.into(),
            driver,
            version: String::new(),
            inf: String::new(),
            name: String::new(),
        };
        let key = RegKey::predef(HKEY_LOCAL_MACHINE)
            .open_subkey_with_flags(d.path("DriverDesc")?, KEY_READ | KEY_WOW64_64KEY)?;
        d.version = key.get_value("DriverVersion")?;
        d.inf = key.get_value("InfPath")?;
        d.name = property(set, dev, SPDRP_FRIENDLYNAME)?
            .or(property(set, dev, SPDRP_DEVICEDESC)?)
            .unwrap_or_else(|| id.into());
        rows.push(d);
        Ok(())
    })?;
    Ok(rows)
}
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
            )
        } else {
            ensure!(self.data.chars().count() <= 8192, "显卡名称数据异常")
        }
        Ok(())
    }
    fn raw(&self) -> Result<RegValue> {
        self.validate()?;
        let bytes = if self.kind == 3 {
            STANDARD.decode(&self.data)?
        } else {
            crate::win::wide(&self.data)
                .into_iter()
                .flat_map(u16::to_le_bytes)
                .collect()
        };
        Ok(RegValue {
            bytes,
            vtype: match self.kind {
                2 => REG_EXPAND_SZ,
                3 => REG_BINARY,
                _ => REG_SZ,
            },
        })
    }
    fn from_raw(v: RegValue) -> Result<Self> {
        let kind = v.vtype as u32;
        let data = if kind == 3 {
            STANDARD.encode(&v.bytes)
        } else {
            ensure!(v.bytes.len().is_multiple_of(2), "显卡名称数据异常");
            crate::win::from_wide(
                &v.bytes
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|b| u16::from_le_bytes([b[0], b[1]]))
                    .collect::<Vec<_>>(),
            )
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
            a.to_uppercase().starts_with("PCI\\VEN_10DE&") && a.eq_ignore_ascii_case(b)
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
    // Validate the old record before considering driver migration. Never move a
    // backup to another PnP instance, even if the model/slot looks similar.
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
        // Other values belong to the newly installed driver (or another tool).
        // Preserve them as the new baseline instead of restoring stale fields.
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
struct WindowsBackend(Device);
impl WindowsBackend {
    fn token(&self) -> String {
        crate::core::hash(self.0.instance.to_uppercase().as_bytes())
    }
    fn friendly<T>(
        &self,
        mut f: impl FnMut(HDEVINFO, &mut SP_DEVINFO_DATA) -> Result<T>,
    ) -> Result<T> {
        let mut result = None;
        devices(|set, dev, id| {
            if id == self.0.instance {
                result = Some(f(set, dev)?);
            }
            Ok(())
        })?;
        result.context("目标显卡不再存在")
    }
}
impl Backend for WindowsBackend {
    fn driver_description(&self) -> Result<Option<String>> {
        self.friendly(|set, dev| {
            // SAFETY: Operates on an owned temporary SetupAPI set. Building the
            // installed-driver list only reads metadata; no install/restart API.
            unsafe {
                let mut params = SP_DEVINSTALL_PARAMS_W {
                    cbSize: std::mem::size_of::<SP_DEVINSTALL_PARAMS_W>() as u32,
                    ..Default::default()
                };
                SetupDiGetDeviceInstallParamsW(set, Some(dev), &mut params)?;
                params.FlagsEx |= DI_FLAGSEX_INSTALLEDDRIVER | DI_FLAGSEX_ALLOWEXCLUDEDDRVS;
                SetupDiSetDeviceInstallParamsW(set, Some(dev), &params)?;
                SetupDiBuildDriverInfoList(set, Some(dev), SPDIT_CLASSDRIVER)?;
                let mut info = SP_DRVINFO_DATA_V2_W {
                    cbSize: std::mem::size_of::<SP_DRVINFO_DATA_V2_W>() as u32,
                    ..Default::default()
                };
                let result =
                    SetupDiEnumDriverInfoW(set, Some(dev), SPDIT_CLASSDRIVER, 0, &mut info);
                let _ = SetupDiDestroyDriverInfoList(set, Some(dev), SPDIT_CLASSDRIVER);
                result?;
                Ok(Some(crate::win::from_wide(&info.Description)))
            }
        })
    }
    fn identity(&self) -> Result<serde_json::Value> {
        let current = enumerate()?
            .into_iter()
            .find(|d| d.instance == self.0.instance)
            .context("显卡已断开，请刷新后重试")?;
        ensure!(
            current.identity() == self.0.identity(),
            "驱动已变化，请刷新后重试"
        );
        Ok(current.identity())
    }
    fn read(&self, field: &str) -> Result<Option<Value>> {
        if field == "FriendlyName" {
            return self.friendly(|s, d| {
                Ok(property(s, d, SPDRP_FRIENDLYNAME)?.map(|data| Value { kind: 1, data }))
            });
        }
        let key = RegKey::predef(HKEY_LOCAL_MACHINE)
            .open_subkey_with_flags(self.0.path(field)?, KEY_READ | KEY_WOW64_64KEY)?;
        match key.get_raw_value(field) {
            Ok(v) => Ok(Some(Value::from_raw(v)?)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
    fn write(&mut self, field: &str, value: Option<&Value>) -> Result<()> {
        if field == "FriendlyName" {
            let raw = value.map(Value::raw).transpose()?;
            return self.friendly(|s, d| {
                // SAFETY: Only the enumerated GPU's FriendlyName property is changed, with a stable encoded buffer.
                unsafe {
                    SetupDiSetDeviceRegistryPropertyW(
                        s,
                        d,
                        SPDRP_FRIENDLYNAME,
                        raw.as_ref().map(|v| v.bytes.as_slice()),
                    )?;
                }
                Ok(())
            });
        }
        let key = RegKey::predef(HKEY_LOCAL_MACHINE)
            .open_subkey_with_flags(self.0.path(field)?, KEY_SET_VALUE | KEY_WOW64_64KEY)?;
        if let Some(v) = value {
            key.set_raw_value(field, &v.raw()?)?;
        } else {
            match key.delete_value(field) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
        }
        flush(&key)?;
        Ok(())
    }
    fn load(&self) -> Result<Option<Record>> {
        let result = RegKey::predef(HKEY_LOCAL_MACHINE)
            .open_subkey_with_flags(BACKUPS, KEY_READ | KEY_WOW64_64KEY)
            .and_then(|k| k.get_raw_value(self.token()));
        match result {
            Ok(v) => {
                ensure!(
                    v.vtype == REG_SZ && v.bytes.len() <= 131072,
                    "显卡名称备份无效"
                );
                let text = crate::win::from_wide(
                    &v.bytes
                        .as_chunks::<2>()
                        .0
                        .iter()
                        .map(|b| u16::from_le_bytes([b[0], b[1]]))
                        .collect::<Vec<_>>(),
                );
                Ok(Some(serde_json::from_str(&text)?))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
    fn save(&mut self, r: &Record) -> Result<()> {
        let (k, _) = RegKey::predef(HKEY_LOCAL_MACHINE)
            .create_subkey_with_flags(BACKUPS, KEY_SET_VALUE | KEY_WOW64_64KEY)?;
        k.set_value(self.token(), &serde_json::to_string(r)?)?;
        flush(&k)?;
        Ok(())
    }
    fn clear(&mut self) -> Result<()> {
        let k = RegKey::predef(HKEY_LOCAL_MACHINE)
            .open_subkey_with_flags(BACKUPS, KEY_SET_VALUE | KEY_WOW64_64KEY)?;
        k.delete_value(self.token())?;
        flush(&k)?;
        Ok(())
    }
}
pub fn apply(target: Device, name: Option<&str>) -> Result<String> {
    ensure!(crate::win::is_admin(), "修改系统显示名称需要管理员权限");
    let _lock = crate::win::game_lock(Path::new("RTXFG-global-gpu-name"))?;
    let current = enumerate()?
        .into_iter()
        .find(|d| d.instance.eq_ignore_ascii_case(&target.instance))
        .context("显卡已断开，请刷新后重试")?;
    change(&mut WindowsBackend(current), name)
}

#[cfg(test)]
mod windows_read_only_tests {
    use super::*;
    #[test]
    #[ignore = "Explicit Windows installed-driver metadata read; no registry writes"]
    fn installed_driver_description_does_not_change_display_names() -> Result<()> {
        let devices = enumerate()?;
        ensure!(
            !devices.is_empty(),
            "No NVIDIA device for this explicit check"
        );
        for device in devices {
            let backend = WindowsBackend(device);
            let before = FIELDS
                .iter()
                .map(|f| backend.read(f))
                .collect::<Result<Vec<_>>>()?;
            let description = backend
                .driver_description()?
                .context("No installed-driver description")?;
            ensure!(
                !description.is_empty(),
                "Empty installed-driver description"
            );
            println!("installed_driver_description={description}");
            assert_eq!(
                before,
                FIELDS
                    .iter()
                    .map(|f| backend.read(f))
                    .collect::<Result<Vec<_>>>()?
            );
        }
        Ok(())
    }
}
