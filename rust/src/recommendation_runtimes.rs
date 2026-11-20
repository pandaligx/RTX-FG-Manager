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
impl Component {
    fn matches_native(&self, current: &runtime_files::Status) -> bool {
        current.available
            && current.present
            && current.warning.is_none()
            && current
                .versions
                .iter()
                .map(|v| normalized(v))
                .collect::<BTreeSet<_>>()
                == self.versions.iter().map(|v| normalized(v)).collect()
    }

    fn needs_install(&self, targets: &[core::DeploymentTarget]) -> bool {
        if self.source == "patch" || self.preserves_game_files() {
            return false;
        }
        !(self.source == "game"
            && self.kind != "nr"
            && targets
                .iter()
                .all(|target| self.matches_native(&runtime_files::inspect(target, &self.kind))))
    }
    fn preflight_target(
        &self,
        target: &core::DeploymentTarget,
        recipe: &crate::recommendations::Recipe,
    ) -> Result<Option<runtime_files::Status>> {
        let added = recipe.scheme == crate::encore::SCHEME
            && crate::encore::supports_200(Some(&recipe.patch_version));
        if self.kind == "encorefg" {
            ensure!(added, "新增帧生成组件需要 Encore 2.0+");
            runtime_files::preflight_added_fg(target, None)?;
        } else if self.source == "game" {
            if self.kind == "nr" {
                ensure!(
                    added || crate::scanner::inspect(&target.game_exe).neural_input(),
                    "未发现游戏的 DLSS 超分或光线重建组件；不会添加 NR 运行库"
                );
            } else {
                if added && matches!(self.kind.as_str(), "sr" | "rr") {
                    let evidence = crate::scanner::inspect_for_installation(&target.game_exe);
                    if !evidence.limited
                        && (self.kind == "rr"
                            || (!evidence.neural_input() && !evidence.super_resolution_plugin))
                    {
                        // Encore completion can supply a missing RR or eligible SR.
                        return Ok(None);
                    }
                }
                let status = runtime_files::inspect(target, &self.kind);
                ensure!(
                    status.available && status.present,
                    "游戏缺少分享所用的运行库文件；可仅安装方案，不会向未知位置写入"
                );
                return Ok(Some(status));
            }
        }
        Ok(None)
    }

    /// An old inventory can list several native versions without identifying
    /// their locations. It is reference information, not a safe replacement map.
    pub fn preserves_game_files(&self) -> bool {
        self.source == "game"
            && ["sr", "rr", "fg"].contains(&self.kind.as_str())
            && (self.versions.len() > 1
                || self.versions.first().is_some_and(|v| {
                    v != "unknown" && !runtime_files::versions(&self.kind).contains(&normalized(v))
                }))
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub schema: u32,
    pub components: Vec<Component>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuntimeStep {
    Keep,
    Install,
    Replace,
    Mixed,
    Verify,
    Unavailable,
}
impl RuntimeStep {
    pub fn label(self) -> &'static str {
        match self {
            Self::Keep => "保留本机",
            Self::Install => "需安装",
            Self::Replace => "将替换",
            Self::Mixed => "各目录操作不同",
            Self::Verify => "安装时核对",
            Self::Unavailable => "当前无法安装",
        }
    }
}

pub fn preview_action(
    source: &str,
    patch: bool,
    runtimes: bool,
    compatible: bool,
    local: Option<RuntimeStep>,
) -> &'static str {
    if runtimes && !compatible {
        return "当前无法安装";
    }
    if source == "patch" && patch {
        return "随补丁安装";
    }
    if !runtimes {
        return "未选择";
    }
    if source == "patch" {
        return "核对已安装补丁";
    }
    local.map_or("正在检查本机…", RuntimeStep::label)
}

/// One line per runtime kind; each version still belongs to its original source.
pub fn grouped(components: &[Component]) -> Vec<(&str, Vec<&Component>)> {
    [
        "sr", "rr", "fg", "nr", "xess", "xess_fg", "xell", "encorefg",
    ]
    .into_iter()
    .filter_map(|kind| {
        let group: Vec<_> = components.iter().filter(|c| c.kind == kind).collect();
        (!group.is_empty()).then_some((kind, group))
    })
    .collect()
}

/// Older recipes can request NR in the original top-level field only.
pub fn displayed_components(recipe: &crate::recommendations::Recipe) -> Vec<Component> {
    let mut components = recipe
        .runtimes
        .as_ref()
        .map(|s| s.components.clone())
        .unwrap_or_default();
    if recipe.neural_runtime == "310.9.16" && !components.iter().any(|c| c.kind == "nr") {
        components.push(Component {
            kind: "nr".into(),
            versions: vec![recipe.neural_runtime.clone()],
            source: "game".into(),
            package_version: None,
        });
    }
    components
}

/// The worker inspects only the selected game's destinations. The installer
/// still repeats its preflight at the time the user actually chooses to apply.
pub fn preview(
    recipe: &crate::recommendations::Recipe,
    targets: &[core::DeploymentTarget],
) -> BTreeMap<(String, String), RuntimeStep> {
    let mut result = BTreeMap::new();
    let targets = targets
        .iter()
        .map(|t| t.validate(true))
        .collect::<Result<Vec<_>>>();
    for c in displayed_components(recipe) {
        if c.source == "patch" {
            continue;
        }
        let key = (c.kind.clone(), c.source.clone());
        let Ok(targets) = &targets else {
            result.insert(key, RuntimeStep::Unavailable);
            continue;
        };
        let mut combined = None;
        for target in targets {
            let recorded = recipe
                .runtimes
                .as_ref()
                .is_some_and(|s| s.components.contains(&c));
            let checked = if recorded {
                c.preflight_target(target, recipe)
            } else {
                Ok(None)
            };
            let Ok(checked) = checked else {
                combined = Some(RuntimeStep::Unavailable);
                break;
            };
            let step = if c.kind == "nr" {
                let status = neural_runtime::inspect(target);
                if status.present || status.warning.is_some() {
                    RuntimeStep::Verify
                } else {
                    RuntimeStep::Install
                }
            } else if c.source == "managed" {
                // The SDK installer verifies the complete package and file ownership.
                RuntimeStep::Verify
            } else {
                let status = checked.unwrap_or_else(|| runtime_files::inspect(target, &c.kind));
                if c.preserves_game_files() && !status.present {
                    RuntimeStep::Verify
                } else if c.preserves_game_files() || c.matches_native(&status) {
                    RuntimeStep::Keep
                } else if status.warning.is_some() && status.present {
                    RuntimeStep::Verify
                } else if status.present {
                    RuntimeStep::Replace
                } else {
                    RuntimeStep::Install
                }
            };
            combined = Some(match combined {
                Some(previous) if previous != step => RuntimeStep::Mixed,
                _ => step,
            });
        }
        result.insert(key, combined.unwrap_or(RuntimeStep::Unavailable));
    }
    result
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
        "encorefg" => "新增 DLSS 帧生成组件",
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
    pub(crate) fn download_files(
        &self,
        targets: &[core::DeploymentTarget],
    ) -> Result<Vec<crate::transfer::DownloadFile>> {
        let mut files = Vec::new();
        for c in self.components.iter().filter(|c| c.needs_install(targets)) {
            if c.source == "managed" {
                files.extend(runtime_files::download_files(
                    targets,
                    if c.kind == "encorefg" {
                        "encorefg"
                    } else {
                        "xemfg"
                    },
                    c.package_version.as_deref().context("运行库版本未知")?,
                )?);
            } else if c.kind == "nr" {
                files.push(neural_runtime::download_file("310.9.16")?);
            } else {
                files.extend(runtime_files::download_files(
                    targets,
                    &c.kind,
                    &normalized(c.versions.first().context("运行库版本未知")?),
                )?);
            }
        }
        Ok(files)
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.schema == 1 && self.components.len() <= 21,
            "运行库分享信息无效"
        );
        let mut seen = BTreeSet::new();
        for c in &self.components {
            ensure!(
                [
                    "sr", "rr", "fg", "nr", "xess", "xess_fg", "xell", "encorefg"
                ]
                .contains(&c.kind.as_str())
                    && ["game", "patch", "managed"].contains(&c.source.as_str())
                    && seen.insert((&c.kind, &c.source))
                    && !c.versions.is_empty()
                    && c.versions.len() <= 8
                    && c.versions.iter().all(|v| valid_version(v))
                    && c.versions.iter().collect::<BTreeSet<_>>().len() == c.versions.len()
                    && c.package_version
                        .as_ref()
                        .is_none_or(|v| c.source == "managed"
                            && ["xess_fg", "xell", "encorefg"].contains(&c.kind.as_str())
                            && valid_version(v)
                            && v != "unknown"),
                "运行库分享信息无效"
            );
            ensure!(
                c.source != "managed" || ["xess_fg", "xell", "encorefg"].contains(&c.kind.as_str()),
                "运行库来源无效"
            );
            ensure!(
                c.kind != "encorefg"
                    || c.source == "managed"
                        && c.package_version.as_deref() == Some("2.14.16")
                        && c.versions
                            .iter()
                            .map(|v| normalized(v))
                            .collect::<BTreeSet<_>>()
                            == BTreeSet::from(["2.14.16".into(), "310.9.16".into()]),
                "新增帧生成必须使用完整匹配的组件包"
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
            if c.preserves_game_files() {
                ensure!(c.kind != "nr", "分享包含多个 NR 版本，请手动核对");
                continue;
            }
            if c.source == "managed" {
                if c.kind == "encorefg" {
                    ensure!(
                        runtime_files::versions("encorefg")
                            .contains(c.package_version.as_ref().context("新增帧生成版本缺失")?),
                        "新增帧生成组件版本不可用"
                    );
                    continue;
                }
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
            .filter(|c| c.source == "managed" && c.kind != "encorefg")
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
                    file.as_file().sync_all()?;
                    // Windows version APIs cannot read a NamedTempFile while
                    // its delete-on-close handle is open. Keep the path guard,
                    // but close the writer before inspecting the PE resource.
                    let path = file.into_temp_path();
                    let version = win::file_version(&path)
                        .with_context(|| format!("无法读取配套运行库版本：{}", component.kind))?
                        .0;
                    Ok(normalized(&version))
                })
                .collect::<Result<BTreeSet<_>>>()?;
            ensure!(
                !component.versions.iter().any(|v| v == "unknown")
                    && versions == component.versions.iter().map(|v| normalized(v)).collect(),
                "当前补丁配套运行库与分享不同，尚未安装；可选择仅安装方案（{}：分享 {}，当前 {}）",
                label(&component.kind),
                component.versions.join(" / "),
                versions.iter().cloned().collect::<Vec<_>>().join(" / ")
            );
        }
        Ok(())
    }
    pub fn preflight(
        &self,
        targets: &[core::DeploymentTarget],
        recipe: &crate::recommendations::Recipe,
    ) -> Result<()> {
        self.compatible_with(&recipe.scheme, &recipe.entries)?;
        for c in &self.components {
            for target in targets {
                c.preflight_target(target, recipe)?;
            }
        }
        Ok(())
    }
    /// Check the shared components before downloads or filesystem inspection.
    pub fn compatible_with(&self, scheme: &str, entries: &[String]) -> Result<()> {
        self.installable()?;
        if self
            .components
            .iter()
            .any(|c| c.kind == "nr" && c.source == "game")
        {
            ensure_nr_scheme(scheme)?;
        }
        ensure!(
            !self
                .components
                .iter()
                .any(|c| c.source == "managed" && c.kind != "encorefg")
                || scheme == "rtx40mfg-1.3.3-hf2" && !entries.iter().any(|e| e.ends_with(".asi")),
            "此方案不能使用独立 XeSS 运行库"
        );
        ensure!(
            !self.components.iter().any(|c| c.kind == "encorefg")
                || scheme == crate::encore::SCHEME,
            "此方案不支持新增 DLSS 帧生成组件"
        );
        Ok(())
    }
    pub fn verify_installed(&self, targets: &[core::DeploymentTarget]) -> Result<()> {
        for target in targets {
            let actual = capture(target)?;
            for expected in &self.components {
                if expected.preserves_game_files() {
                    // Explicitly retained native files are not claimed to be
                    // an exact reproduction of the author's installation.
                    let status = runtime_files::inspect(target, &expected.kind);
                    ensure!(
                        status.available && status.present,
                        "游戏原有运行库缺失，请刷新后检查"
                    );
                    continue;
                }
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
            if !c.needs_install(targets) {
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
                if c.kind == "encorefg" {
                    runtime_files::install_version(
                        targets,
                        "encorefg",
                        c.package_version.as_deref().context("新增帧生成版本未知")?,
                        github,
                        cancel,
                        &mut report,
                    )?;
                } else if !intel {
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

/// A copied inventory may include NR left over from a different patch.
pub fn ensure_nr_scheme(scheme: &str) -> Result<()> {
    ensure!(
        [crate::encore::SCHEME, "rtx40mfg-1.3.3-hf2"].contains(&scheme),
        "此方案不支持分享中的 DLSSNR 运行库"
    );
    Ok(())
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
        if matches!(status.kind.as_str(), "xemfg" | "encorefg") {
            continue;
        }
        for path in status.paths {
            // A native FG transaction also owns its matched Streamline plugins
            // and license. Those are dependencies, not additional DLSS-G builds.
            if path.file_name().and_then(|n| n.to_str()).and_then(kind)
                == Some(status.kind.as_str())
            {
                add(&status.kind, "game", &path)?;
            }
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
    let mut components: Vec<Component> = groups
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
    if let Some(package_version) = runtime_files::installed_package_version(target, "encorefg")? {
        components.push(Component {
            kind: "encorefg".into(),
            source: "managed".into(),
            versions: vec!["2.14.16".into(), "310.9.16".into()],
            package_version: Some(package_version),
        });
    }
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
    fn grouped_inventory_preserves_game_and_patch_versions_and_selection() {
        let mut s = snapshot("310.5.2");
        for (kind, source, version) in [
            ("fg", "patch", "310.9.16"),
            ("sr", "patch", "310.9.16"),
            ("fg", "game", "310.5.2"),
        ] {
            s.components.push(Component {
                kind: kind.into(),
                source: source.into(),
                versions: vec![version.into()],
                package_version: None,
            });
        }
        let rows = grouped(&s.components);
        assert_eq!(
            rows.iter().map(|(k, _)| *k).collect::<Vec<_>>(),
            ["sr", "fg"]
        );
        for (_, group) in rows {
            assert_eq!(group.len(), 2);
            assert!(
                group
                    .iter()
                    .any(|c| c.source == "game" && c.versions == ["310.5.2"])
            );
            assert!(
                group
                    .iter()
                    .any(|c| c.source == "patch" && c.versions == ["310.9.16"])
            );
        }
        assert_eq!(
            preview_action("patch", true, false, true, None),
            "随补丁安装"
        );
        assert_eq!(
            preview_action("patch", false, true, true, None),
            "核对已安装补丁"
        );
        assert_eq!(
            preview_action("game", true, false, true, Some(RuntimeStep::Replace)),
            "未选择"
        );
        assert_eq!(
            preview_action("game", true, true, true, Some(RuntimeStep::Replace)),
            "将替换"
        );
        assert_eq!(
            preview_action("game", true, true, true, Some(RuntimeStep::Keep)),
            "保留本机"
        );
        assert_eq!(
            preview_action("patch", true, true, false, None),
            "当前无法安装"
        );
        assert_eq!(
            preview_action("game", true, true, true, None),
            "正在检查本机…"
        );
    }

    #[test]
    fn runtime_preview_uses_the_same_native_version_check_as_the_installer() {
        let c = snapshot("310.9.16").components.remove(0);
        let mut status = runtime_files::Status {
            available: true,
            present: true,
            versions: BTreeSet::from(["310.9.16.0".into()]),
            ..Default::default()
        };
        assert!(c.matches_native(&status));
        status.versions.insert("310.5.2".into());
        assert!(!c.matches_native(&status));
        status.versions.remove("310.5.2");
        status.present = false;
        assert!(!c.matches_native(&status));
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
        assert!(snapshot("999.1.0").components[0].preserves_game_files());
        assert!(snapshot("unknown").installable().is_err());
    }
    #[test]
    fn multi_version_native_inventory_does_not_download_or_replace_files() -> Result<()> {
        let mut s = snapshot("310.5.0");
        s.components[0].versions.push("310.7.0".into());
        assert!(s.components[0].preserves_game_files());
        s.installable()?;
        s.install(&[], false, &AtomicBool::new(false), &mut |_| {
            panic!("native reference inventory must never start a download");
        })?;
        // Unlisted native versions are retained, never substituted with latest.
        assert!(snapshot("999.1.0").components[0].preserves_game_files());
        s.components[0].kind = "nr".into();
        assert!(s.installable().is_err());
        Ok(())
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
    #[test]
    fn encore_reinstall_can_supply_removed_rr_but_other_schemes_cannot() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let exe = dir.path().join("FixtureGame.exe");
        std::fs::copy(std::env::current_exe()?, &exe)?;
        let target = core::DeploymentTarget::for_game(&exe);
        let mut shared = snapshot("310.9.16");
        shared.components[0].kind = "rr".into();
        let mut recipe: crate::recommendations::Recipe =
            serde_json::from_value(serde_json::json!({
                "schema":1,"game":{"id":"steam:2012510","name":"Fixture"},
                "title":"Fixture","notes":"","scheme":crate::encore::SCHEME,"patchVersion":"2.1.0",
                "entries":["version.dll"],"renderer":"dx12","gpuModel":"","driver":"",
                "gpuAlias":"unchanged","neuralRuntime":"none"
            }))?;
        shared.preflight(std::slice::from_ref(&target), &recipe)?;
        for scheme in [crate::upstream040::SCHEME, "rtx40mfg-1.3.3-hf2"] {
            recipe.scheme = scheme.into();
            assert!(
                shared
                    .preflight(std::slice::from_ref(&target), &recipe)
                    .is_err()
            );
        }
        assert!(
            !dir.path().join("nvngx_dlssd.dll").exists(),
            "preflight must not write files"
        );
        Ok(())
    }
    #[test]
    fn neural_runtime_compatibility_is_checked_before_game_access() -> Result<()> {
        let mut s = snapshot("310.9.16");
        s.components[0].kind = "nr".into();
        for scheme in [crate::encore::SCHEME, "rtx40mfg-1.3.3-hf2"] {
            s.compatible_with(scheme, &["dxgi.dll".into()])?;
        }
        for scheme in [
            crate::upstream040::SCHEME,
            crate::recommendation_presets::MFG_161_COMPAT,
            "rtxfg-0.3.5-dx12-vulkan",
        ] {
            let error = s.compatible_with(scheme, &["dxgi.dll".into()]).unwrap_err();
            assert!(error.to_string().contains("DLSSNR"));
        }
        // An ordinary SR inventory remains valid on the same older schemes.
        snapshot("310.9.16").compatible_with(crate::upstream040::SCHEME, &["dxgi.dll".into()])?;
        Ok(())
    }
    #[test]
    #[cfg(windows)]
    fn prepared_bytes_keep_the_same_windows_version_as_the_closed_source() -> Result<()> {
        // Use an existing Windows PE solely as a version-resource fixture;
        // nothing is loaded or installed. No signed patch fixtures are required.
        let source =
            std::path::PathBuf::from(std::env::var("SystemRoot")?).join("System32/kernel32.dll");
        // Windows can virtualize kernel32's version at its system location for
        // an unmanifested test harness. Establish the baseline from a copy.
        let fixture = tempfile::tempdir()?;
        let copied = fixture.path().join("version-fixture.dll");
        std::fs::copy(&source, &copied)?;
        let expected = file_version(&copied);
        assert_ne!(expected, "unknown");
        let mut snapshot = snapshot(&expected);
        snapshot.components[0].source = "patch".into();
        let files = BTreeMap::from([("components/nvngx_dlss.dll".into(), std::fs::read(source)?)]);
        snapshot.verify_prepared(&files)?;
        snapshot.components[0].versions[0].push_str(".0");
        snapshot.verify_prepared(&files)?;
        snapshot.components[0].versions[0] = "999.1.0".into();
        assert!(snapshot.verify_prepared(&files).is_err());
        Ok(())
    }
}
