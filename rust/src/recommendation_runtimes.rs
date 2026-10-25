//! Public runtime inventory: versions and provenance, never local paths or URLs.
use crate::{cloud, core, neural_runtime, runtime_files, win};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Write,
    sync::atomic::AtomicBool,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Component {
    pub kind: String,
    pub versions: Vec<String>,
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_version: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub schema: u32,
    pub components: Vec<Component>,
}
pub fn label(kind: &str) -> &'static str {
    match kind {
        "sr" => "DLSS SR",
        "rr" => "DLSS RR",
        "fg" => "DLSS FG",
        "nr" => "DLSSNR",
        "xess" => "XeSS SR",
        "xess_fg" => "XeSS FG",
        "xell" => "XeLL",
        _ => "运行库",
    }
}
fn kind(name: &str) -> Option<&'static str> {
    match name.to_ascii_lowercase().as_str() {
        "nvngx_dlss.dll" => Some("sr"),
        "nvngx_dlssd.dll" => Some("rr"),
        "nvngx_dlssg.dll" => Some("fg"),
        "nvngx_dlssnr.dll" => Some("nr"),
        "libxess.dll" => Some("xess"),
        "libxess_fg.dll" => Some("xess_fg"),
        "libxell.dll" => Some("xell"),
        _ => None,
    }
}
fn file_version(path: &std::path::Path) -> String {
    win::file_version(path)
        .ok()
        .map(|v| v.0)
        .filter(|v| valid_version(v))
        .unwrap_or("unknown".into())
}
fn valid_version(version: &str) -> bool {
    version == "unknown"
        || version.len() <= 40
            && (3..=4).contains(&version.split('.').count())
            && version
                .split('.')
                .all(|n| !n.is_empty() && n.len() <= 10 && n.bytes().all(|b| b.is_ascii_digit()))
}
fn normalized(version: &str) -> String {
    version
        .strip_suffix(".0")
        .filter(|_| version.split('.').count() == 4)
        .unwrap_or(version)
        .to_owned()
}
impl Snapshot {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.schema == 1 && self.components.len() <= 21,
            "运行库分享信息无效"
        );
        let mut seen = BTreeSet::new();
        for c in &self.components {
            ensure!(
                ["sr", "rr", "fg", "nr", "xess", "xess_fg", "xell"].contains(&c.kind.as_str())
                    && ["game", "patch", "managed"].contains(&c.source.as_str())
                    && seen.insert((&c.kind, &c.source))
                    && !c.versions.is_empty()
                    && c.versions.len() <= 8
                    && c.versions.iter().all(|v| valid_version(v))
                    && c.versions.iter().collect::<BTreeSet<_>>().len() == c.versions.len()
                    && c.package_version
                        .as_ref()
                        .is_none_or(|v| c.source == "managed"
                            && ["xess_fg", "xell"].contains(&c.kind.as_str())
                            && valid_version(v)
                            && v != "unknown"),
                "运行库分享信息无效"
            );
            ensure!(
                c.source != "managed" || ["xess_fg", "xell"].contains(&c.kind.as_str()),
                "运行库来源无效"
            );
        }
        Ok(())
    }
    pub fn installable(&self) -> Result<()> {
        self.validate()?;
        for c in &self.components {
            if c.source == "patch" {
                continue;
            }
            if c.source == "managed" {
                ensure!(
                    c.package_version
                        .as_ref()
                        .is_some_and(|v| runtime_files::versions("xemfg").contains(v)),
                    "分享包含未收录的 XeSS 版本，可仅安装方案并手动核对运行库"
                );
                continue;
            }
            ensure!(
                c.versions.len() == 1,
                "分享包含多个运行库版本，可仅安装方案并手动核对运行库"
            );
            let version = normalized(&c.versions[0]);
            let known = if c.kind == "nr" {
                version == "310.9.16"
            } else {
                ["sr", "rr", "fg"].contains(&c.kind.as_str())
                    && runtime_files::versions(&c.kind).contains(&version)
            };
            ensure!(
                known,
                "分享中的运行库版本未收录，可仅安装方案并手动核对运行库"
            );
        }
        let managed = self
            .components
            .iter()
            .filter(|c| c.source == "managed")
            .collect::<Vec<_>>();
        ensure!(
            managed.is_empty()
                || managed.len() == 2
                    && managed.iter().any(|c| c.kind == "xess_fg")
                    && managed.iter().any(|c| c.kind == "xell")
                    && managed[0].package_version == managed[1].package_version,
            "分享的 XeSS 组件不完整或版本不一致"
        );
        Ok(())
    }
    pub fn verify_prepared(&self, files: &BTreeMap<String, Vec<u8>>) -> Result<()> {
        for component in self.components.iter().filter(|c| c.source == "patch") {
            let versions = files
                .iter()
                .filter(|(name, _)| {
                    kind(name.rsplit('/').next().unwrap_or(name)) == Some(component.kind.as_str())
                })
                .map(|(_, bytes)| {
                    let mut file = tempfile::Builder::new()
                        .prefix("rtxfg-version-")
                        .suffix(".dll")
                        .tempfile()?;
                    file.write_all(bytes)?;
                    Ok(file_version(file.path()))
                })
                .collect::<Result<BTreeSet<_>>>()?;
            ensure!(
                !component.versions.iter().any(|v| v == "unknown")
                    && versions == component.versions.iter().cloned().collect(),
                "当前补丁配套运行库与分享不同，尚未安装；可选择仅安装方案"
            );
        }
        Ok(())
    }
    pub fn preflight(
        &self,
        targets: &[core::DeploymentTarget],
        recipe: &crate::recommendations::Recipe,
    ) -> Result<()> {
        self.installable()?;
        for c in &self.components {
            if c.kind == "nr" && c.source == "game" {
                ensure!(
                    ["dlssg-transfusion-1.4.5.3", "rtx40mfg-1.3.3-hf2"]
                        .contains(&recipe.scheme.as_str()),
                    "此方案不支持分享中的 DLSSNR 运行库"
                );
                for target in targets {
                    ensure!(
                        crate::scanner::inspect(&target.game_exe).neural_input(),
                        "未发现游戏的 DLSS 超分或光线重建组件；不会添加 NR 运行库"
                    );
                }
            }
            if c.source == "managed" {
                ensure!(
                    recipe.scheme == "rtx40mfg-1.3.3-hf2"
                        && !recipe.entries.iter().any(|e| e.ends_with(".asi")),
                    "此方案不能使用独立 XeSS 运行库"
                );
            }
            if c.source != "game" || c.kind == "nr" {
                continue;
            }
            for target in targets {
                let status = runtime_files::inspect(target, &c.kind);
                ensure!(
                    status.available && status.present,
                    "游戏缺少分享所用的运行库文件；可仅安装方案，不会向未知位置写入"
                );
            }
        }
        Ok(())
    }
    pub fn verify_installed(&self, targets: &[core::DeploymentTarget]) -> Result<()> {
        for target in targets {
            let actual = capture(target)?;
            for expected in &self.components {
                ensure!(
                    actual.components.iter().any(|c| c.kind == expected.kind
                        && c.source == expected.source
                        && c.versions
                            .iter()
                            .map(|v| normalized(v))
                            .collect::<BTreeSet<_>>()
                            == expected.versions.iter().map(|v| normalized(v)).collect()),
                    "运行库版本未完整匹配分享，参数尚未应用；请刷新后检查"
                );
            }
        }
        Ok(())
    }
    pub fn install(
        &self,
        targets: &[core::DeploymentTarget],
        github: bool,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(cloud::CloudProgress),
    ) -> Result<()> {
        let mut intel = false;
        for c in &self.components {
            if c.source == "patch" {
                continue;
            }
            let mut report = |transfer| {
                progress(cloud::CloudProgress {
                    transfer,
                    proxy: label(&c.kind).into(),
                    package_index: 1,
                    package_count: 1,
                })
            };
            if c.source == "managed" {
                if !intel {
                    runtime_files::install_version(
                        targets,
                        "xemfg",
                        c.package_version.as_deref().context("XeSS 版本未知")?,
                        github,
                        cancel,
                        &mut report,
                    )?;
                    intel = true;
                }
            } else if c.kind == "nr" {
                neural_runtime::install_many(targets, "310.9.16", github, cancel, &mut report)?;
            } else {
                runtime_files::install_version(
                    targets,
                    &c.kind,
                    &normalized(&c.versions[0]),
                    github,
                    cancel,
                    &mut report,
                )?;
            }
        }
        Ok(())
    }
}

pub fn capture(target: &core::DeploymentTarget) -> Result<Snapshot> {
    let mut groups: BTreeMap<(String, String), BTreeSet<String>> = BTreeMap::new();
    let mut add = |kind: &str, source: &str, path: &std::path::Path| -> Result<()> {
        let p = core::no_links(path)?;
        if p.is_file() {
            groups
                .entry((source.into(), kind.into()))
                .or_default()
                .insert(file_version(&p));
        }
        Ok(())
    };
    for status in runtime_files::inspect_all(target) {
        if status.kind == "xemfg" {
            continue;
        }
        for path in status.paths {
            add(&status.kind, "game", &path)?;
        }
    }
    let nr = neural_runtime::inspect(target);
    if nr.present {
        add("nr", "game", &nr.path)?;
    }
    if let Some(record) = core::record(&target.directory)? {
        target.validate_record(&record)?;
        for name in record.hashes.keys() {
            if let Some(k) = kind(name.rsplit('/').next().unwrap_or(name)) {
                add(k, "patch", &target.directory.join(name))?;
            }
        }
    }
    let intel = runtime_files::inspect(target, "xemfg");
    for path in &intel.paths {
        if let Some(k) = path.file_name().and_then(|n| n.to_str()).and_then(kind) {
            add(k, "managed", path)?;
        }
    }
    let package = if intel.present {
        runtime_files::installed_package_version(target, "xemfg")?
    } else {
        None
    };
    let components = groups
        .into_iter()
        .map(|((source, kind), versions)| Component {
            package_version: if source == "managed" {
                package.clone()
            } else {
                None
            },
            kind,
            source,
            versions: versions.into_iter().collect(),
        })
        .collect();
    let snapshot = Snapshot {
        schema: 1,
        components,
    };
    snapshot.validate()?;
    Ok(snapshot)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn snapshot(version: &str) -> Snapshot {
        Snapshot {
            schema: 1,
            components: vec![Component {
                kind: "sr".into(),
                source: "game".into(),
                versions: vec![version.into()],
                package_version: None,
            }],
        }
    }
    #[test]
    fn version_inventory_is_bounded_and_never_accepts_paths() {
        for v in ["310.9.16", "310.9.16.0", "unknown"] {
            snapshot(v).validate().unwrap();
        }
        for v in ["C:/Users/me", "https://x/y", "", "NaN", "1.2", "1.2.3.4.5"] {
            assert!(snapshot(v).validate().is_err());
        }
        let mut s = snapshot("310.9.16");
        s.components.push(s.components[0].clone());
        assert!(s.validate().is_err());
    }
    #[test]
    fn unavailable_versions_are_not_replaced_with_latest() {
        snapshot("310.9.16.0").installable().unwrap();
        assert!(snapshot("999.1.0").installable().is_err());
        assert!(snapshot("unknown").installable().is_err());
    }
    #[test]
    fn intel_is_a_complete_same_version_sdk_pair() {
        let mut s = Snapshot {
            schema: 1,
            components: vec![Component {
                kind: "xess_fg".into(),
                source: "managed".into(),
                versions: vec!["2.1.0.0".into()],
                package_version: Some("3.0.2".into()),
            }],
        };
        assert!(s.installable().is_err());
        let mut xell = s.components[0].clone();
        xell.kind = "xell".into();
        s.components.push(xell);
        s.installable().unwrap();
        s.components[1].package_version = Some("3.0.1".into());
        assert!(s.installable().is_err());
    }
    #[test]
    fn missing_patch_runtime_is_rejected_before_install() {
        let mut s = snapshot("310.9.16");
        s.components[0].source = "patch".into();
        assert!(s.verify_prepared(&BTreeMap::new()).is_err());
    }
}
