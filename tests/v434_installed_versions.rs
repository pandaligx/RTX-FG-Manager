//! Issue #11: synthetic files only; no real DLL is loaded or game modified.
use anyhow::Result;
use rtx_fg_manager::{cloud, core, deployment, encore, rtxmfg, scanner};
use serde_json::json;
use std::{collections::BTreeMap, fs, path::Path};

fn install(dir: &Path, backend: &str, scheme: &str, version: Option<&str>) -> Result<String> {
    fs::create_dir_all(dir)?;
    let mut pe = vec![0; 512];
    pe[..2].copy_from_slice(b"MZ");
    pe[60..64].copy_from_slice(&128u32.to_le_bytes());
    pe[128..132].copy_from_slice(b"PE\0\0");
    pe[132..134].copy_from_slice(&0x8664u16.to_le_bytes());
    pe[148..150].copy_from_slice(&240u16.to_le_bytes());
    pe[150..152].copy_from_slice(&2u16.to_le_bytes());
    pe[152..154].copy_from_slice(&0x20bu16.to_le_bytes());
    let exe = dir.join("Game.exe");
    fs::write(&exe, pe)?;
    let mut hashes = BTreeMap::new();
    for name in core::deployment_names(backend, &["version.dll".into()])? {
        let path = dir.join(&name);
        fs::write(&path, b"synthetic installed bytes")?;
        hashes.insert(name, core::digest(&path)?);
    }
    core::atomic_json(
        &dir.join(core::OWN).join(core::MARKER),
        &json!({"schema":3,"backend":backend,"scheme_id":scheme,
            "payload_version":version,"proxy":"version.dll","proxies":["version.dll"],"hashes":hashes}),
    )?;
    Ok(exe.display().to_string())
}

fn future_catalog() -> cloud::Catalog {
    let mut catalog = cloud::bundled();
    for package in &mut catalog.packages {
        package.version = "99.0.0".into();
        package.upstream_version = Some("99.0.0".into());
        package.label = "Future patch 99.0.0".into();
        package.labels.clear();
    }
    catalog
}

#[test]
fn catalog_refresh_cannot_promote_any_installed_patch_version() -> Result<()> {
    let temp = tempfile::tempdir()?;
    for (backend, scheme, version, label) in [
        (
            encore::BACKEND,
            encore::SCHEME,
            "1.1.1",
            "RTX Encore · 1.1.1",
        ),
        (
            rtxmfg::BACKEND,
            "rtx40mfg-1.3.3-hf2",
            "1.5.0",
            "MFG Unlocker · 1.5.0",
        ),
        (
            "upstream_sm86",
            "upstream-0.3.5-310-9",
            "0.3.5",
            "0.3.5 · Github-sdli1995",
        ),
        (
            "upstream_sm86",
            "rtxfg-0.3.5-dx12-vulkan",
            "0.3.5",
            "upstream_sm86 0.3.5",
        ),
    ] {
        let dir = temp.path().join(scheme);
        let game = scanner::Game {
            exe: install(&dir, backend, scheme, Some(version))?,
            ..Default::default()
        };
        let before = fs::read(dir.join(core::OWN).join(core::MARKER))?;
        let snapshot = deployment::inspect(&game, &future_catalog());
        assert_eq!(snapshot.installed, 1);
        assert_eq!(
            snapshot.installed_labels.iter().collect::<Vec<_>>(),
            vec![label]
        );
        assert!(snapshot.details[0].contains(label));
        assert!(!snapshot.details[0].contains("99.0.0"));
        assert_eq!(before, fs::read(dir.join(core::OWN).join(core::MARKER))?);
    }
    Ok(())
}

#[test]
fn actual_record_update_changes_label_and_retains_release_suffix() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let game = scanner::Game {
        exe: install(temp.path(), encore::BACKEND, encore::SCHEME, Some("1.1.1"))?,
        ..Default::default()
    };
    let catalog = future_catalog();
    assert!(
        deployment::inspect(&game, &catalog)
            .installed_labels
            .contains("RTX Encore · 1.1.1")
    );
    let mut record = core::record(temp.path())?.unwrap();
    record.payload_version = Some("1.1.2".into());
    record.upstream_version = Some("1.1.2-hotfix.1".into());
    core::atomic_json(&temp.path().join(core::OWN).join(core::MARKER), &record)?;
    let snapshot = deployment::inspect(&game, &catalog);
    assert!(
        snapshot
            .installed_labels
            .contains("RTX Encore · 1.1.2-hotfix.1")
    );
    assert!(!snapshot.installed_labels.contains("RTX Encore · 1.1.1"));
    assert!(snapshot.details[0].contains("RTX Encore · 1.1.2-hotfix.1"));
    Ok(())
}

#[test]
fn grouped_directories_retain_distinct_versions_and_exclude_changed_files() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let first = install(
        &temp.path().join("A"),
        encore::BACKEND,
        encore::SCHEME,
        Some("1.1.1"),
    )?;
    let second = install(
        &temp.path().join("B"),
        encore::BACKEND,
        encore::SCHEME,
        Some("1.1.2"),
    )?;
    let game = scanner::Game {
        exe: first.clone(),
        targets: vec![first, second],
        ..Default::default()
    };
    let catalog = future_catalog();
    let snapshot = deployment::inspect(&game, &catalog);
    assert_eq!((snapshot.installed, snapshot.total), (2, 2));
    assert_eq!(
        snapshot.installed_labels.into_iter().collect::<Vec<_>>(),
        vec!["RTX Encore · 1.1.1", "RTX Encore · 1.1.2"]
    );
    fs::write(temp.path().join("B/version.dll"), b"externally changed DLL")?;
    let partial = deployment::inspect(&game, &catalog);
    assert_eq!((partial.installed, partial.total), (1, 2));
    assert_eq!(
        partial.installed_labels.into_iter().collect::<Vec<_>>(),
        vec!["RTX Encore · 1.1.1"]
    );
    assert!(partial.details.iter().any(|s| s.contains("DLL 已调整")));
    Ok(())
}

#[test]
fn legacy_missing_version_does_not_borrow_the_available_version() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let game = scanner::Game {
        exe: install(
            temp.path(),
            "upstream_sm86",
            "rtxfg-0.3.5-dx12-vulkan",
            None,
        )?,
        ..Default::default()
    };
    let snapshot = deployment::inspect(&game, &future_catalog());
    assert_eq!(snapshot.installed, 1);
    assert!(snapshot.installed_labels.contains("upstream_sm86 ?"));
    assert!(!snapshot.details[0].contains("99.0.0"));
    Ok(())
}
