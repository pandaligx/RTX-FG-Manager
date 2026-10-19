use anyhow::Result;
use rtx_fg_manager::{encore, presets::Values, scanner};
use std::{fs, path::Path, sync::atomic::AtomicBool};
#[test]
fn nr_cache_cleanup_keeps_unknown_files_and_game_backups() -> Result<()> {
    let d = tempfile::tempdir()?;
    let runtime = d.path().join("runtime");
    let nr = runtime.join("neural-rendering");
    fs::create_dir_all(&nr)?;
    let data = d.path().join("data");
    fs::create_dir(&data)?;
    let hash = "a".repeat(64);
    fs::write(nr.join(format!("{hash}.zip")), b"cache")?;
    fs::write(nr.join(format!("310.9.16-{hash}.dll")), b"cache")?;
    fs::write(nr.join("user.dll"), b"keep")?;
    let backup = d.path().join("game/.rtx-fg-nr");
    fs::create_dir_all(&backup)?;
    fs::write(backup.join("original.dll"), b"original")?;
    let report = rtx_fg_manager::cache::clean_scoped(&runtime, &data)?;
    assert_eq!(report.files, 2);
    assert_eq!(fs::read(nr.join("user.dll"))?, b"keep");
    assert_eq!(fs::read(backup.join("original.dll"))?, b"original");
    Ok(())
}
#[test]
fn stale_catalog_cannot_downgrade_bundled_encore() -> Result<()> {
    use rtx_fg_manager::cloud;
    let scheme = "dlssg-transfusion-1.4.5.3";
    let mut catalog = cloud::bundled();
    let expected = catalog
        .packages
        .iter()
        .find(|p| p.scheme_id == scheme)
        .unwrap()
        .version
        .clone();
    for p in catalog
        .packages
        .iter_mut()
        .filter(|p| p.scheme_id == scheme)
    {
        p.version = "1.0.0".into();
        p.upstream_version = Some("1.0.0".into());
    }
    catalog
        .scheme_policies
        .get_mut(scheme)
        .unwrap()
        .defaults
        .clear();
    catalog.retain_bundled_versions();
    catalog.validate()?;
    assert!(
        catalog
            .packages
            .iter()
            .filter(|p| p.scheme_id == scheme)
            .all(|p| p.version == expected)
    );
    assert_eq!(
        catalog.scheme_policies[scheme].defaults["dlssNrPreset"],
        "b"
    );
    catalog.packages.retain(|p| p.scheme_id != scheme);
    catalog.retain_bundled_versions();
    assert!(!catalog.packages.iter().any(|p| p.scheme_id == scheme));
    Ok(())
}
fn exe(p: &Path) -> Result<()> {
    fs::create_dir_all(p.parent().unwrap())?;
    let mut b = vec![0; 512];
    b[..2].copy_from_slice(b"MZ");
    b[60..64].copy_from_slice(&128u32.to_le_bytes());
    b[128..132].copy_from_slice(b"PE\0\0");
    b[132..134].copy_from_slice(&0x8664u16.to_le_bytes());
    fs::write(p, b)?;
    Ok(())
}
#[test]
fn witcher_dx12_owns_icon_and_target_not_state_object_compiler() -> Result<()> {
    let d = tempfile::tempdir()?;
    let root = d.path().join("steamapps/common/The Witcher 3");
    let real = root.join("bin/x64_dx12/witcher3.exe");
    exe(&real)?;
    exe(&root.join("bin/x64/witcher3.exe"))?;
    let helper = root.join("bin/x64_dx12/D3D12_0/D3D12StateObjectCompiler.exe");
    exe(&helper)?;
    fs::write(real.parent().unwrap().join("nvngx_dlss.dll"), b"fixture")?;
    fs::write(
        d.path().join("steamapps/appmanifest_292030.acf"),
        b"\"name\" \"The Witcher 3\"\n\"installdir\" \"The Witcher 3\"",
    )?;
    let report = scanner::scan(&[d.path().into()], &AtomicBool::new(false), |_, _, _| {})?;
    assert_eq!(report.rows.len(), 1);
    assert_eq!(Path::new(&report.rows[0].exe), real);
    assert_eq!(report.rows[0].targets.len(), 1);
    let mut old = scanner::Game {
        exe: helper.display().to_string(),
        targets: vec![helper.display().to_string(), real.display().to_string()],
        ..Default::default()
    };
    fs::create_dir(helper.parent().unwrap().join(rtx_fg_manager::core::OWN))?;
    old.repair_renderer();
    assert_eq!(Path::new(&old.exe), real);
    assert_eq!(old.cleanup_only, vec![helper.display().to_string()]);
    Ok(())
}
#[test]
fn nr_preset_migration_retains_comments_unknowns_and_new_defaults() -> Result<()> {
    for (old, new) in [(0, "auto"), (1, "a"), (2, "b"), (3, "auto")] {
        let raw = format!(
            "{{\"configVersion\":4,// retain\n\"neuralRendering\":{{\"advanced\":{{\"nrPreset\":{old}}}}},\"vendor\":{{\"nrPreset\":99}}}}"
        );
        assert_eq!(encore::read(raw.as_bytes())?["dlssNrPreset"], new);
        let narrow = encore::migrate_nr(raw.as_bytes())?;
        assert_eq!(encore::read(&narrow)?["dlssNrPreset"], new);
        assert!(String::from_utf8_lossy(&narrow).contains("// retain"));
        let out = encore::migrate(raw.as_bytes(), &Values::new())?;
        assert!(String::from_utf8_lossy(&out).contains("// retain"));
        let doc = rtx_fg_manager::jsonc::Document::parse(&out)?;
        assert_eq!(doc.value["vendor"]["nrPreset"], 99);
        assert_eq!(doc.value["dlssPresets"]["dlssNrPreset"], new);
    }
    assert_eq!(encore::defaults()["dlssNrPreset"], "b");
    assert_eq!(encore::defaults()["fastSuperResolution"], "0");
    assert!(encore::field("fastSuperResolution").unwrap().restart);
    assert!(!encore::field("dlssNrPreset").unwrap().restart);
    let unchanged = b"{\"configVersion\":4,// exact\n\"vendor\":true}";
    assert_eq!(encore::migrate_nr(unchanged)?, unchanged);
    Ok(())
}
