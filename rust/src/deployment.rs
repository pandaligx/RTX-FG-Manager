//! Display/selection metadata. Decisions that modify files still use core validation.
use crate::{cloud::Catalog, core, scanner::Game};
use std::{collections::BTreeSet, path::Path};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IssueKind {
    Residuals,
    Interrupted,
    Modified,
    External,
    Unavailable,
}
impl IssueKind {
    pub fn from_status(status: &str) -> Option<Self> {
        if status.starts_with("补丁已移除，缓存待清理") {
            Some(Self::Residuals)
        } else if status.starts_with("升级未完成") || status.starts_with("未完成 /") {
            Some(Self::Interrupted)
        } else if status.starts_with("DLL 已调整") || status.contains("参数待恢复") {
            Some(Self::Modified)
        } else if status.starts_with("已有手动补丁或其他 MOD") {
            Some(Self::External)
        } else if status.starts_with("需检查") {
            Some(Self::Unavailable)
        } else {
            None
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Residuals => "残留文件待清理",
            Self::Interrupted => "补丁安装未完成",
            Self::Modified => "补丁文件已变化",
            Self::External => "发现未受管理的文件",
            Self::Unavailable => "无法读取补丁状态",
        }
    }
    pub fn help(self) -> &'static str {
        match self {
            Self::Residuals => {
                "补丁已移除。可清理当前游戏留下的缓存和安装临时文件，无需清空全部下载缓存。"
            }
            Self::Interrupted => "安装或升级未完成。可先卸载并恢复原文件，再重新安装所需方案。",
            Self::Modified => {
                "文件与安装记录不一致，可能由手动修改或其他 MOD 引起。请先查看详情，勿直接删除 DLL。"
            }
            Self::External => {
                "此目录有手动补丁、其他 MOD 或旧配置。未确认归属前保留文件；这不代表游戏损坏。"
            }
            Self::Unavailable => "暂时无法确认状态。请核对目录是否存在及能否访问，再刷新检测。",
        }
    }
}
#[derive(Clone, Debug)]
pub struct Issue {
    target: core::DeploymentTarget,
    kind: IssueKind,
    detail: String,
}
impl Issue {
    pub fn target(&self) -> &core::DeploymentTarget {
        &self.target
    }
    pub fn kind(&self) -> IssueKind {
        self.kind
    }
    pub fn detail(&self) -> &str {
        &self.detail
    }
}
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub status: String,
    pub cleanup_pending: bool,
    pub cleanup_details: Vec<String>,
    pub cleanup_paths: Vec<std::path::PathBuf>,
    pub neural: Vec<crate::neural_runtime::Status>,
    pub runtimes: Vec<crate::runtime_files::Status>,
    pub total: usize,
    pub installed: usize,
    pub schemes: BTreeSet<String>,
    /// Installed release identities, shared by the row and per-directory details.
    /// A stable scheme ID is not an installed version: the catalog may be newer.
    pub installed_labels: BTreeSet<String>,
    pub proxies: BTreeSet<String>,
    pub details: Vec<String>,
    pub common: Option<(String, Vec<String>, Option<usize>)>,
    pub incompatible_protocol: bool,
    issues: Vec<Issue>,
}
pub fn inspect(game: &Game, catalog: &Catalog) -> Snapshot {
    let targets = game.deployment_targets();
    let mut snapshot = Snapshot {
        total: targets.len(),
        ..Default::default()
    };
    let mut selections = Vec::new();
    let mut statuses = Vec::new();
    let mut seen = BTreeSet::new();
    // Old deployment locations must also expose their cleanup/recovery state.
    let targets = targets.into_iter().map(|t| (t, false)).chain(
        game.cleanup_only
            .iter()
            .map(|exe| (game.deployment_target(Path::new(exe)), true)),
    );
    for (target, cleanup_only) in targets {
        if !seen.insert(core::key(&target.directory)) {
            continue;
        }
        let status = core::status_at(&target);
        if !cleanup_only {
            statuses.push(status.clone());
        }
        if let Some(kind) = IssueKind::from_status(&status) {
            let mut detail = status.clone();
            if kind == IssueKind::External {
                for name in core::PROXIES.iter().chain(
                    [
                        core::INI,
                        crate::encore::CONFIG,
                        crate::rtxmfg::CONFIG,
                        ".rtx-fg-script.json",
                        ".rtx-fg-manager.json",
                    ]
                    .iter(),
                ) {
                    if target.directory.join(name).exists() {
                        detail.push_str(&format!("\n{name}"));
                    }
                }
            }
            snapshot.issues.push(Issue {
                target: target.clone(),
                kind,
                detail,
            });
        }
        let record = core::record(&target.directory).ok().flatten();
        if let Some(r) = record.as_ref()
            && r.cache_pending
            && status.starts_with("补丁已移除，缓存待清理")
        {
            snapshot.cleanup_pending = true;
            if let Ok(root) = crate::delta::root() {
                snapshot.cleanup_paths.extend(
                    r.delta_cache_ids
                        .iter()
                        .filter(|id| crate::delta::valid_id(id))
                        .map(|id| root.join("games").join(id)),
                );
                if r.delta_legacy_cache {
                    snapshot
                        .cleanup_paths
                        .push(root.join(crate::delta::REVISION));
                }
            }
            snapshot.cleanup_paths.extend(
                r.hashes
                    .keys()
                    .map(|name| {
                        target
                            .directory
                            .join(core::OWN)
                            .join(core::stage_name(name))
                    })
                    .filter(|p| p.exists()),
            );
            snapshot.cleanup_details.push(format!(
                "{}\n{}",
                target.directory.display(),
                if r.cleanup_pending_reasons.is_empty() {
                    "旧记录未保存具体原因；点击重新清理可重新检测。".into()
                } else {
                    r.cleanup_pending_reasons.join("\n")
                }
            ));
        }
        if cleanup_only {
            snapshot
                .details
                .push(format!("{}\n{}", target.directory.display(), status));
            continue;
        }
        let mut nr = crate::neural_runtime::inspect(&target);
        if status.starts_with("已部署")
            && let Some(r) = record.as_ref()
            && matches!(
                r.backend.as_str(),
                crate::encore::BACKEND | crate::rtxmfg::BACKEND
            )
            && target.validate_record(r).is_ok()
        {
            let version = Some(
                r.upstream_version
                    .as_deref()
                    .or(r.payload_version.as_deref())
                    .unwrap_or("0.0.0")
                    .into(),
            );
            if r.backend == crate::rtxmfg::BACKEND {
                nr.mfg_version = version;
            } else {
                nr.encore_version = version;
            }
        }
        snapshot.neural.push(nr);
        snapshot
            .runtimes
            .extend(crate::runtime_files::inspect_all(&target));
        if status.starts_with("已部署")
            && let Some(r) = record
        {
            snapshot.installed += 1;
            let scheme = r
                .scheme_id
                .clone()
                .unwrap_or_else(|| match r.backend.as_str() {
                    "rtx20" | "rtx30" => "initial".into(),
                    b if b.starts_with("native") => "native-0.2.6-stable".into(),
                    _ => "unknown".into(),
                });
            let catalog_label = catalog
                .packages
                .iter()
                .find(|p| {
                    p.scheme_id == scheme
                        && Some(p.upstream_version.as_deref().unwrap_or(&p.version))
                            == r.upstream_version
                                .as_deref()
                                .or(r.payload_version.as_deref())
                })
                .map(|p| p.label.clone())
                .unwrap_or_else(|| {
                    format!(
                        "{} {}",
                        r.backend,
                        r.upstream_version
                            .as_deref()
                            .or(r.payload_version.as_deref())
                            .unwrap_or("?")
                    )
                });
            let label = if r.backend == crate::transfusion::BACKEND {
                "DLSSG-Transfusion · 1.4.5.3".to_owned()
            } else if r.backend == crate::encore::BACKEND {
                format!(
                    "RTX Encore · {}",
                    r.upstream_version
                        .as_deref()
                        .or(r.payload_version.as_deref())
                        .unwrap_or("")
                )
            } else if r.backend == crate::rtxmfg::BACKEND {
                format!(
                    "MFG Unlocker · {}",
                    r.upstream_version
                        .as_deref()
                        .or(r.payload_version.as_deref())
                        .unwrap_or("?")
                )
            } else if scheme == crate::upstream040::SCHEME {
                // The stable scheme ID spans 0.3.5 and 0.4.0. Never label an
                // installed old DLL with the available catalog's new version.
                format!(
                    "{} · Github-sdli1995",
                    r.upstream_version
                        .as_deref()
                        .or(r.payload_version.as_deref())
                        .unwrap_or("?")
                )
            } else {
                catalog_label
            };
            snapshot.incompatible_protocol |=
                catalog.scheme_policies.get(&scheme).is_none_or(|policy| {
                    !core::backend_matches_profile(&r.backend, &policy.parameter_profile)
                });
            snapshot.schemes.insert(scheme.clone());
            snapshot.installed_labels.insert(label.clone());
            let proxies = r.selected();
            snapshot.proxies.extend(proxies.clone());
            snapshot.details.push(format!(
                "{}\n{} · {}",
                target.directory.display(),
                label,
                proxies.join(", ")
            ));
            let series = if r.backend == crate::rtxmfg::BACKEND {
                Some(2)
            } else if r.backend.ends_with("20") {
                Some(0)
            } else if r.backend.ends_with("30") {
                Some(1)
            } else {
                None
            };
            selections.push((scheme, proxies, series));
        } else {
            snapshot
                .details
                .push(format!("{}\n{}", target.directory.display(), status));
        }
    }
    if let Some(first) = selections.first()
        && selections.iter().all(|s| s == first)
    {
        snapshot.common = Some(first.clone());
    }
    snapshot.status = if snapshot.total == 1 && game.cleanup_only.is_empty() {
        statuses[0].clone()
    } else if snapshot.installed == snapshot.total {
        format!("已部署 {0}/{0} 个目录", snapshot.total)
    } else if snapshot.installed > 0 {
        format!(
            "已部署 {}/{} 个目录，请检查未完成项",
            snapshot.installed, snapshot.total
        )
    } else if game
        .cleanup_only
        .iter()
        .any(|e| core::status(Path::new(e)) != "未部署")
    {
        "旧路径有补丁，可卸载清理".into()
    } else {
        statuses
            .into_iter()
            .find(|s| s != "未部署")
            .unwrap_or_else(|| "未部署".into())
    };
    snapshot
}
impl Snapshot {
    pub fn issues(&self) -> &[Issue] {
        &self.issues
    }
    pub fn issue_label(&self) -> Option<&'static str> {
        let first = self.issues.first()?.kind;
        Some(if self.issues.iter().all(|i| i.kind == first) {
            first.label()
        } else {
            "多个目录需要处理"
        })
    }
    pub fn can_apply(&self, scheme: &str) -> bool {
        self.total > 0
            && self.total == self.installed
            && self.common.as_ref().is_some_and(|c| c.0 == scheme)
            && !self.incompatible_protocol
    }
}
