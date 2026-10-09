//! Offline compatibility checks: immutable previous cloud resources and update
//! metadata. These do not claim network, signature, UAC or physical GPU testing.
use anyhow::Result;
use rtx_fg_manager::{cloud, core, release_notes, updater};
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[test]
fn v426_only_replaces_rtxmfg_and_keeps_previous_resources_immutable() -> Result<()> {
    let bytes = include_bytes!("fixtures/catalog425/index.json");
    assert_eq!(
        core::hash(bytes),
        "7d1cba1c8cd03ed75ea02b895ff684fb3336fd025a1785d05de15fffadba85cb"
    );
    let prior: Value = serde_json::from_slice(bytes)?;
    let packages = prior["packages"].as_array().unwrap();
    assert_eq!(packages.len(), 21);
    let current = cloud::bundled();
    current.validate()?;
    let mut preserved = 0;
    for old in packages {
        let old: cloud::Package = serde_json::from_value(old.clone())?;
        if old.scheme_id == "rtx40mfg-1.3.3-hf2" {
            let updated = current
                .packages
                .iter()
                .find(|p| p.scheme_id == old.scheme_id)
                .unwrap();
            assert_eq!(updated.version, "1.4.2");
            assert_ne!(updated.archive, old.archive);
            assert_ne!(updated.sha256, old.sha256);
            assert_eq!(updated.proxy, old.proxy);
            assert_eq!(updated.backends, old.backends);
            continue;
        }
        let new = current.packages.iter().find(|p| p.id == old.id).unwrap();
        // Display names may evolve; every deployment/download identity must
        // remain exact, including archive/file sizes, hashes and proxy routing.
        let identity = |p: &cloud::Package| -> Result<Value> {
            let mut p = p.clone();
            p.files.sort_by(|a, b| a.name.cmp(&b.name));
            let mut value = serde_json::to_value(p)?;
            let object = value.as_object_mut().unwrap();
            object.remove("label");
            object.remove("labels");
            Ok(value)
        };
        assert_eq!(identity(new)?, identity(&old)?, "{}", old.id);
        preserved += 1;
    }
    assert_eq!(preserved, 20);
    // These four exact proxy packages belonged to 4.2.6; a later stable-ID
    // upgrade must not rewrite that historical download/cleanup evidence.
    let historical: Value = serde_json::from_str(include_str!(
        "../cloud/indexes/payload-index-r-27e17f68f3e68de7a12c.json"
    ))?;
    let historical_mfg = historical["packages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["scheme_id"] == "rtx40mfg-1.3.3-hf2")
        .unwrap();
    assert_eq!(historical_mfg["version"], "1.4.1");
    assert_eq!(
        historical_mfg["archive"],
        "rtxmfg-1.4.1-hf1-v426-universal.zip"
    );
    let legacy: Vec<_> = historical["packages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|p| p["scheme_id"] == "dlssg-transfusion-1.4.5.3")
        .collect();
    assert_eq!(legacy.len(), 4);
    assert!(
        legacy
            .iter()
            .all(|p| p["backends"] == json!(["transfusion"]))
    );
    Ok(())
}

fn legacy_manifest() -> Result<updater::Manifest> {
    Ok(serde_json::from_value(json!({
        "schema": 1,
        "version": "4.2.5",
        "file": "RTXManager-v4.2.5-x64.exe",
        "sha256": "9a80d83a5474a634ec5bc84bb7dee5ac665b0dadb0ac2aa7642a8b7f643adc1a",
        "bytes": 29079544
    }))?)
}

#[test]
fn updater_accepts_old_manifests_without_notes_and_keeps_file_identity_separate() -> Result<()> {
    let old = legacy_manifest()?;
    old.validate("v4.2.5")?;
    assert!(old.notes.is_empty());
    assert!(release_notes::select(&old.notes, "zh-CN").is_none());
    let mut localized = old.clone();
    localized.notes = BTreeMap::from([
        ("zh-CN".into(), "更新说明\n第二行".into()),
        ("en".into(), "Release notes".into()),
    ]);
    localized.validate("4.2.5")?;
    assert!(old.same_file(&localized));
    assert_eq!(
        release_notes::select(&localized.notes, "ru"),
        Some("Release notes")
    );
    localized.notes.insert("zh-CN".into(), " \n ".into());
    assert_eq!(
        release_notes::select(&localized.notes, "zh-CN"),
        Some("Release notes")
    );
    localized.sha256 = "a".repeat(64);
    assert!(!old.same_file(&localized));
    localized.file = "unrelated.exe".into();
    assert!(localized.validate("4.2.5").is_err());
    Ok(())
}

#[test]
fn updater_rejects_oversized_notes_by_utf8_bytes_and_excess_languages() -> Result<()> {
    let mut manifest = legacy_manifest()?;
    manifest.notes.insert("en".into(), "a".repeat(16384));
    manifest.validate("4.2.5")?;
    manifest.notes.get_mut("en").unwrap().push('a');
    assert!(manifest.validate("4.2.5").is_err());
    manifest.notes = BTreeMap::from([("zh-CN".into(), "汉".repeat(5461))]);
    manifest.validate("4.2.5")?;
    manifest.notes.get_mut("zh-CN").unwrap().push('汉');
    assert!(manifest.validate("4.2.5").is_err());
    manifest.notes = (0..8)
        .map(|n| (format!("language-{n}"), "note".into()))
        .collect();
    manifest.validate("4.2.5")?;
    manifest.notes.insert("language-8".into(), "note".into());
    assert!(manifest.validate("4.2.5").is_err());
    Ok(())
}
