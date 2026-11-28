//! Assemble OptiScaler using the same verified Streamline ZIP as the runtime window.
use super::{Catalog, Package, base_archive};
use crate::{optiscaler_nr, runtime_files::opti_streamline, transfer};
use anyhow::{Result, ensure};
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicBool, Ordering},
};

pub(super) fn public_components() -> impl Iterator<Item = &'static str> {
    optiscaler_nr::components()
        .iter()
        .copied()
        .filter(|n| !n.starts_with("OptiScaler/streamline/"))
}

pub(super) fn prepare(
    catalog: &Catalog,
    base: &Package,
    cancel: &AtomicBool,
    report: &mut impl FnMut(transfer::Progress),
) -> Result<BTreeMap<String, Vec<u8>>> {
    let mut files = base_archive(catalog, base, cancel, report)?;
    ensure!(!cancel.load(Ordering::Relaxed), "下载已取消");
    for (name, data) in opti_streamline::bundle_files(catalog.prefer_github, cancel, report)? {
        ensure!(
            files.insert(name, data).is_none(),
            "补丁包重复包含私有 Streamline 文件"
        );
    }
    ensure!(!cancel.load(Ordering::Relaxed), "下载已取消");
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Context;
    use std::collections::BTreeSet;

    #[test]
    fn public_and_shared_archives_cover_the_installed_files_once() -> Result<()> {
        let catalog = crate::cloud::bundled();
        let base = catalog
            .packages
            .iter()
            .find(|p| p.scheme_id == optiscaler_nr::SCHEME)
            .context("missing public base")?;
        let mut all: BTreeSet<String> = base.files.iter().map(|f| f.name.clone()).collect();
        let shared = crate::runtime_files::bundled_package("encorefg")?;
        for f in shared.files.iter().filter(|f| f.name.ends_with(".dll")) {
            assert!(all.insert(format!("OptiScaler/streamline/{}", f.name)));
        }
        let mut expected: BTreeSet<String> = optiscaler_nr::components()
            .iter()
            .map(|s| s.to_string())
            .collect();
        expected.extend([base.proxy.clone(), optiscaler_nr::CONFIG.into()]);
        assert_eq!(all, expected);
        Ok(())
    }
}
