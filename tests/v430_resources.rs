//! Metadata-only tests. Do not download, load, or repeatedly verify signed DLLs.
use anyhow::Result;
use rtx_fg_manager::{cloud, core, encore, mfg_plugin, rtxmfg, upstream040};
use std::collections::BTreeSet;

fn catalog() -> Result<cloud::Catalog> {
    Ok(serde_json::from_str(include_str!(
        "fixtures/cloud-v3-catalog.json"
    ))?)
}

#[test]
fn complete_schema3_candidate_keeps_legacy_routes_and_separates_new_entries() -> Result<()> {
    let c = catalog()?;
    c.validate()?;
    assert_eq!(c.schema, 3);
    assert_eq!(c.proxies(upstream040::SCHEME).len(), 6);
    assert_eq!(c.proxies(encore::SCHEME).len(), 20);
    assert_eq!(c.proxies(rtx_fg_manager::rtxmfg_upgrade::SCHEME).len(), 20);
    assert!(
        c.proxies(rtx_fg_manager::rtxmfg_upgrade::SCHEME)
            .contains(&mfg_plugin::ENTRY.into())
    );
    for series in 0..=2 {
        assert!(
            c.schemes()
                .iter()
                .all(|p| c.supports_series(&p.scheme_id, series))
        );
    }
    let components = c
        .packages
        .iter()
        .filter(|p| p.scheme_id == upstream040::SCHEME && p.component)
        .flat_map(|p| &p.files)
        .map(|f| f.name.as_str())
        .collect::<BTreeSet<_>>();
    assert_eq!(components, upstream040::COMPONENTS.into_iter().collect());
    assert!(
        c.packages
            .iter()
            .filter(|p| p.scheme_id == encore::SCHEME)
            .any(|p| p.proxy == "rtx-encore.asi")
    );
    assert!(
        c.packages
            .iter()
            .find(|p| p.scheme_id == rtx_fg_manager::rtxmfg_upgrade::SCHEME)
            .unwrap()
            .files
            .iter()
            .any(|f| f.name == rtxmfg::NOTICES)
    );
    assert!(
        c.packages
            .iter()
            .filter(|p| p.version == "0.4.0")
            .all(|p| p.urls["domestic"].starts_with("https://gitcode.com/"))
    );
    Ok(())
}

#[test]
fn component_omission_overlap_and_path_escape_are_rejected() -> Result<()> {
    let c = catalog()?;
    let n = c.packages.iter().position(|p| p.component).unwrap();
    let mut bad = c.clone();
    bad.packages.remove(n);
    assert!(bad.validate().is_err());
    let mut bad = c.clone();
    let mut duplicate = bad.packages[n].clone();
    duplicate.id.push_str("-duplicate");
    bad.packages.push(duplicate);
    assert!(bad.validate().is_err());
    let mut bad = c.clone();
    bad.packages[n].files[0].name = "../outside.dll".into();
    assert!(bad.validate().is_err());
    let mut bad = c;
    bad.packages[n].version = "0.4.1".into();
    assert!(bad.validate().is_err());
    assert!(
        core::deployment_names(
            rtxmfg::BACKEND,
            &[mfg_plugin::ENTRY.into(), "version.dll".into()]
        )
        .is_err()
    );
    assert!(
        core::deployment_names(
            encore::BACKEND,
            &["rtx-encore.asi".into(), "version.dll".into()]
        )
        .is_err()
    );
    Ok(())
}
