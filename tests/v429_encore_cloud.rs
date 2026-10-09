//! Synthetic archive and catalog checks. These do not execute a DLL, contact
//! release hosts, or establish Windows signing / physical GPU compatibility.
use anyhow::Result;
use rtx_fg_manager::{cloud, core, encore};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Cursor, Write},
};

fn fixture(config: &[u8]) -> Result<(cloud::Catalog, Vec<u8>)> {
    let mut pe = vec![0; 512];
    pe[..2].copy_from_slice(b"MZ");
    pe[60..64].copy_from_slice(&128u32.to_le_bytes());
    pe[128..134].copy_from_slice(b"PE\0\0\x64\x86");
    pe[150..152].copy_from_slice(&0x2000u16.to_le_bytes());
    let entries = [
        ("version.dll", pe.as_slice()),
        (encore::CONFIG, config),
        (encore::NOTICES, b"Synthetic test notice.\n".as_slice()),
    ];
    let mut archive = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let mut files = Vec::new();
    for (name, bytes) in entries {
        archive.start_file(name, zip::write::FileOptions::default())?;
        archive.write_all(bytes)?;
        files.push(cloud::File {
            name: name.into(),
            bytes: bytes.len() as u64,
            sha256: core::hash(bytes),
        });
    }
    let bytes = archive.finish()?.into_inner();
    let package = cloud::Package {
        id: "rtx-encore-1.0.0-beta.2-v429-universal".into(),
        scheme_id: encore::SCHEME.into(),
        label: "RTX Encore · 1.0.0-beta.2".into(),
        labels: BTreeMap::new(),
        version: "1.0.0".into(),
        upstream_version: Some("1.0.0-beta.2".into()),
        backends: vec![encore::BACKEND.into()],
        proxy: "version.dll".into(),
        archive: "rtx-encore-1.0.0-beta.2-v429-universal.zip".into(),
        bytes: bytes.len() as u64,
        sha256: core::hash(&bytes),
        files,
    };
    let mut catalog = cloud::bundled();
    catalog.packages = vec![package];
    catalog.default_scheme = encore::SCHEME.into();
    catalog.scheme_policies = BTreeMap::from([(
        encore::SCHEME.into(),
        cloud::Policy {
            gpu_paths: vec!["SM75".into(), "SM86".into(), "SM89".into()],
            max_selected_proxies: 1,
            ini_policy: "encore_json".into(),
            parameter_profile: encore::PROFILE.into(),
            defaults: BTreeMap::new(),
            capabilities: BTreeSet::new(),
        },
    )]);
    catalog.validate()?;
    Ok((catalog, bytes))
}

const TEMPLATE: &[u8] = include_bytes!("../rust/assets/encore-defaults.jsonc");

#[test]
fn canonical_package_exposes_nineteen_single_choice_routes_and_three_gpu_series() -> Result<()> {
    let (catalog, bytes) = fixture(TEMPLATE)?;
    assert_eq!(catalog.packages.len(), 1);
    assert_eq!(catalog.packages[0].proxy, "version.dll");
    let proxies = catalog.proxies(encore::SCHEME);
    assert_eq!(proxies.len(), 19);
    assert_eq!(proxies.iter().collect::<BTreeSet<_>>().len(), 19);
    assert!(
        encore::PROXIES
            .iter()
            .all(|p| proxies.iter().any(|s| s == p))
    );
    assert_eq!(
        catalog.scheme_source_url(encore::SCHEME),
        Some(encore::SOURCE)
    );
    assert!((0..=2).all(|series| catalog.supports_series(encore::SCHEME, series)));
    assert!(!catalog.supports_series(encore::SCHEME, 3));
    let files = cloud::unpack(&catalog.packages[0], &bytes)?;
    assert_eq!(
        files.keys().map(String::as_str).collect::<BTreeSet<_>>(),
        BTreeSet::from(["version.dll", encore::CONFIG, encore::NOTICES])
    );
    assert_eq!(files[encore::CONFIG], TEMPLATE);

    let mut invalid = catalog.clone();
    invalid
        .scheme_policies
        .get_mut(encore::SCHEME)
        .unwrap()
        .max_selected_proxies = 2;
    assert!(invalid.validate().is_err());
    let mut invalid = catalog.clone();
    invalid.packages[0].proxy = "dxgi.dll".into();
    invalid.packages[0]
        .files
        .iter_mut()
        .find(|f| f.name == "version.dll")
        .unwrap()
        .name = "dxgi.dll".into();
    assert!(invalid.validate().is_err());
    let mut invalid = catalog.clone();
    invalid.packages.push(catalog.packages[0].clone());
    invalid.packages[1].id.push_str("-second");
    assert!(invalid.validate().is_err());
    Ok(())
}

#[test]
fn notices_are_required_only_for_encore_and_cannot_be_replaced_by_arbitrary_files() -> Result<()> {
    let (catalog, _) = fixture(TEMPLATE)?;
    for filename in [core::INI, "game.log", "../notice.md"] {
        let mut invalid = catalog.clone();
        invalid.packages[0]
            .files
            .iter_mut()
            .find(|f| f.name == encore::NOTICES)
            .unwrap()
            .name = filename.into();
        assert!(invalid.validate().is_err(), "{filename}");
    }
    let mut missing = catalog.clone();
    missing.packages[0]
        .files
        .retain(|f| f.name != encore::NOTICES);
    assert!(missing.validate().is_err());
    let mut legacy = catalog;
    let policy = legacy.scheme_policies.get_mut(encore::SCHEME).unwrap();
    policy.parameter_profile = rtx_fg_manager::transfusion::PROFILE.into();
    policy.ini_policy = "transfusion_json".into();
    legacy.packages[0].backends = vec![rtx_fg_manager::transfusion::BACKEND.into()];
    legacy.packages[0]
        .files
        .iter_mut()
        .find(|f| f.name == encore::CONFIG)
        .unwrap()
        .name = rtx_fg_manager::transfusion::CONFIG.into();
    assert!(legacy.validate().is_err());
    legacy.packages[0]
        .files
        .retain(|f| f.name != encore::NOTICES);
    legacy.validate()?;
    Ok(())
}

#[test]
fn semantic_config_validation_rejects_wrong_nesting_even_with_matching_hashes() -> Result<()> {
    let mutations: &[fn(&mut Value)] = &[
        |c| c["configVersion"] = json!(5),
        |c| c["neuralRendering"] = c["neuralRendering"]["core"].clone(),
        |c| c["neuralRendering"]["core"]["nrEnabled"] = json!("false"),
        |c| c["neuralRendering"]["core"]["nrEnabled"] = json!(true),
        |c| c["smoothMotion"]["smoothMotionEnabled"] = json!(true),
        |c| c["general"]["gpuSeries"] = json!("rtx30"),
        |c| c["frameGeneration"]["mode"] = json!("fixed"),
        |c| c["neuralRendering"]["openExperimental"]["nrOpenUltraFastGhostTolerance"] = json!(0.31),
    ];
    for mutate in mutations {
        // The parser accepts JSONC, and configure preserves a valid schema4
        // document. A no-op configure provides JSON for deliberate corruption.
        let canonical = encore::configure(TEMPLATE, &BTreeMap::new())?;
        let mut value: Value = rtx_fg_manager::jsonc::Document::parse(&canonical)?.value;
        mutate(&mut value);
        let (catalog, bytes) = fixture(&serde_json::to_vec(&value)?)?;
        assert!(cloud::unpack(&catalog.packages[0], &bytes).is_err());
    }
    Ok(())
}

#[test]
fn full_upstream_version_is_optional_but_bounded_and_matches_numeric_version() -> Result<()> {
    let (catalog, _) = fixture(TEMPLATE)?;
    let encoded = serde_json::to_value(&catalog.packages[0])?;
    assert_eq!(encoded["version"], "1.0.0");
    assert_eq!(encoded["upstream_version"], "1.0.0-beta.2");
    for bad in ["1.0.1-beta.2", "1.0.0-", "1.0.0/evil", "1.0.0-beta.2\n"] {
        let mut invalid = catalog.clone();
        invalid.packages[0].upstream_version = Some(bad.into());
        assert!(invalid.validate().is_err(), "{bad}");
    }
    let mut previous = encoded;
    previous.as_object_mut().unwrap().remove("upstream_version");
    let previous: cloud::Package = serde_json::from_value(previous)?;
    assert!(previous.upstream_version.is_none());
    assert!(
        serde_json::to_value(previous)?
            .get("upstream_version")
            .is_none()
    );
    Ok(())
}

#[test]
fn full_version_survives_compact_expansion_and_file_integrity_stays_mandatory() -> Result<()> {
    let (catalog, bytes) = fixture(TEMPLATE)?;
    let index = serde_json::to_vec(&json!({"schema": 1, "packages": catalog.packages}))?;
    let compact: cloud::CompactCatalog = serde_json::from_value(json!({
        "schema": 2, "revision": "test-encore", "default_scheme": encore::SCHEME,
        "sources": catalog.sources,
        "index": {"url": "https://example.org/index.json", "fallback_url": "https://example.org/index.json",
                  "bytes": index.len(), "sha256": core::hash(&index)},
        "schemes": [{"id": encore::SCHEME, "name": "RTX Encore", "profile": encore::PROFILE,
                     "min_manager_version": "4.2.9", "archives": [catalog.packages[0].archive]}]
    }))?;
    let expanded = compact.expand(&index)?;
    assert_eq!(
        expanded.packages[0].upstream_version.as_deref(),
        Some("1.0.0-beta.2")
    );
    assert_eq!(
        expanded.scheme_policies[encore::SCHEME].gpu_paths,
        ["SM75", "SM86", "SM89"]
    );
    let mut corrupt = bytes.clone();
    corrupt[0] ^= 1;
    assert!(cloud::unpack(&expanded.packages[0], &corrupt).is_err());
    let mut wrong_digest = expanded.packages[0].clone();
    wrong_digest.files[0].sha256 = "f".repeat(64);
    assert!(cloud::unpack(&wrong_digest, &bytes).is_err());
    Ok(())
}
