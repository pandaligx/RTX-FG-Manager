//! Display/selection metadata. Decisions that modify files still use core validation.
use crate::{cloud::Catalog, core, scanner::Game};
use std::{collections::BTreeSet, path::Path};

#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub status: String,
    pub total: usize,
    pub installed: usize,
    pub schemes: BTreeSet<String>,
    pub proxies: BTreeSet<String>,
    pub details: Vec<String>,
    pub common: Option<(String, Vec<String>, Option<usize>)>,
}
pub fn inspect(game: &Game, catalog: &Catalog) -> Snapshot {
    let targets = if game.targets.is_empty() {
        vec![game.exe.clone()]
    } else {
        game.targets.clone()
    };
    let mut snapshot = Snapshot {
        total: targets.len(),
        ..Default::default()
    };
    let mut selections = Vec::new();
    let mut statuses = Vec::new();
    for exe in targets {
        let p = Path::new(&exe);
        let status = core::status(p);
        statuses.push(status.clone());
        let record = p.parent().and_then(|dir| core::record(dir).ok().flatten());
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
            let label = catalog
                .packages
                .iter()
                .find(|p| p.scheme_id == scheme)
                .map(|p| p.label.clone())
                .unwrap_or_else(|| {
                    format!(
                        "{} {}",
                        r.backend,
                        r.payload_version.as_deref().unwrap_or("")
                    )
                });
            snapshot.schemes.insert(scheme.clone());
            let proxies = r.selected();
            snapshot.proxies.extend(proxies.clone());
            snapshot.details.push(format!(
                "{}\n{} · {}",
                p.parent().unwrap().display(),
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
                .push(format!("{}\n{}", p.parent().unwrap_or(p).display(), status));
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
    pub fn can_apply(&self, scheme: &str) -> bool {
        self.total > 0
            && self.total == self.installed
            && self.common.as_ref().is_some_and(|c| c.0 == scheme)
    }
}
