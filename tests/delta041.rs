//! Protocol and migration tests. No game, driver or signed payload is executed.
use anyhow::Result;
use rtx_fg_manager::{cloud, delta, diagnostics, presets, recommendation_presets, upstream040};
use std::path::Path;

fn catalog() -> cloud::Catalog {
    let mut catalog = cloud::bundled();
    if catalog
        .scheme_policies
        .contains_key(cloud::DELTA_041_SCHEME)
    {
        return catalog;
    }
    let mut policy = catalog.scheme_policies[upstream040::SCHEME].clone();
    policy.capabilities.insert(delta::CAPABILITY.into());
    catalog
        .scheme_policies
        .insert(cloud::DELTA_041_SCHEME.into(), policy);
    let packages = catalog
        .packages
        .iter()
        .filter(|p| p.scheme_id == upstream040::SCHEME)
        .cloned()
        .map(|mut p| {
            p.id = format!("delta041-{}", p.id);
            p.scheme_id = cloud::DELTA_041_SCHEME.into();
            p
        })
        .collect::<Vec<_>>();
    catalog.packages.extend(packages);
    catalog
}

#[test]
fn delta041_directory_protocol_is_separate_and_capability_is_scoped() -> Result<()> {
    let mut c = catalog();
    c.validate()?;
    assert_eq!(
        c.proxies(cloud::DELTA_041_SCHEME),
        c.proxies(upstream040::SCHEME)
    );
    assert!(c.scheme_policies.contains_key(cloud::DELTA_SCHEME));
    assert_eq!(
        c.scheme_policies[cloud::DELTA_SCHEME].parameter_profile,
        "upstream035"
    );
    assert!(
        !c.scheme_policies[upstream040::SCHEME]
            .capabilities
            .contains(delta::CAPABILITY)
    );
    c.scheme_policies
        .get_mut(upstream040::SCHEME)
        .unwrap()
        .capabilities
        .insert(delta::CAPABILITY.into());
    assert!(c.validate().is_err());
    Ok(())
}

#[test]
fn delta041_migration_preserves_explicit_settings_but_fresh_install_uses_041_defaults() -> Result<()>
{
    let c = catalog();
    let policy = &c.scheme_policies[cloud::DELTA_041_SCHEME];
    let exe = Path::new("C:/Games/DeltaForce/Binaries/Win64").join(delta::GAME);
    let context = presets::Context::new(cloud::DELTA_041_SCHEME, policy, &exe);
    assert!(context.delta);
    let old = b"; custom\r\n[FrameGeneration]\r\nOptimized=2\r\nMaxGeneratedFrames=2\r\n[Compatibility]\r\nPreset=B\r\n[Runtime]\r\nMode=Auto\r\n[Other]\r\nKeep=42\r\n";
    let migrated = context.configure(old, &presets::Values::new())?;
    let text = String::from_utf8(migrated.clone())?;
    assert!(text.contains("; custom\r\n") && text.contains("Keep=42"));
    let values = upstream040::read_041(&migrated)?;
    assert_eq!(values["optimized"], "2");
    assert_eq!(values["preset"], "B");
    assert_eq!(values["max_generated_frames"], "2");
    assert_eq!(
        diagnostics::ini_value(&text, "Compatibility", "DeltaForceGeneratedFrames")?.as_deref(),
        Some("2")
    );
    assert_eq!(
        diagnostics::ini_value(&text, "Runtime", "Mode")?.as_deref(),
        Some("Bundled")
    );
    let fresh = context.configure(
        include_bytes!("fixtures/catalog041/dlssg_sm86.ini"),
        &presets::Values::new(),
    )?;
    let values = upstream040::read_041(&fresh)?;
    assert_eq!(values["optimized"], "1");
    assert_eq!(values["preset"], "A");
    assert_eq!(values["fg_engine"], "DLSS");
    Ok(())
}

#[test]
fn delta041_mfg_is_enabled_only_for_the_exact_game() -> Result<()> {
    let c = catalog();
    let mut p = c.scheme_policies[cloud::DELTA_041_SCHEME].clone();
    p.capabilities.remove(delta::SIX_X_CAPABILITY);
    let game = Path::new("C:/DeltaForce/Binaries/Win64").join(delta::GAME);
    for (exe, expected) in [(game.as_path(), true), (Path::new("C:/Other.exe"), false)] {
        let context = presets::Context::new(cloud::DELTA_041_SCHEME, &p, exe);
        assert_eq!(context.delta, expected);
        for n in 0..=5 {
            let values = presets::Values::from([("max_generated_frames".into(), n.to_string())]);
            let text = String::from_utf8(context.configure(b"", &values)?)?;
            let requested = if expected { n.min(3) } else { 0 };
            assert_eq!(
                diagnostics::ini_value(&text, "Compatibility", "DeltaForceGeneratedFrames")?,
                Some(requested.to_string())
            );
            assert_eq!(
                diagnostics::ini_value(&text, "Compatibility", "DeltaForcePrivateStreamline")?,
                // 0.4.1 also needs the newer plugin for 2X / game-controlled
                // frames; otherwise its generation-A guard disables FG.
                Some(u8::from(expected).to_string())
            );
        }
    }
    Ok(())
}

#[test]
fn delta041_sharing_is_distinct_from_legacy_and_keeps_paths_private() -> Result<()> {
    let values = presets::defaults(upstream040::PROFILE_041, &presets::Values::new());
    let mut snapshot = recommendation_presets::Snapshot {
        schema: 1,
        profile: upstream040::PROFILE_041.into(),
        values: values.clone(),
        defaults: values,
    };
    snapshot.validate(cloud::DELTA_041_SCHEME, "0.4.1")?;
    assert!(snapshot.validate(cloud::DELTA_SCHEME, "0.3.5").is_err());
    snapshot
        .values
        .insert("DeltaForceRuntimeId".into(), "1234".into());
    snapshot
        .defaults
        .insert("DeltaForceRuntimeId".into(), "1234".into());
    assert!(snapshot.validate(cloud::DELTA_041_SCHEME, "0.4.1").is_err());
    let contracts = recommendation_presets::contracts();
    assert_eq!(
        contracts[cloud::DELTA_041_SCHEME]["profile"],
        upstream040::PROFILE_041
    );
    assert_eq!(
        contracts[upstream040::SCHEME]["profile"],
        upstream040::PROFILE
    );
    assert_eq!(
        contracts[upstream040::SCHEME]["from_041"]["profile"],
        upstream040::PROFILE_041
    );
    Ok(())
}

#[test]
fn delta041_six_x_revision_keeps_four_x_default_and_xess_limit() -> Result<()> {
    let c = catalog();
    let mut policy = c.scheme_policies[cloud::DELTA_041_SCHEME].clone();
    policy.capabilities.insert(delta::SIX_X_CAPABILITY.into());
    let exe = Path::new("C:/DeltaForce/Binaries/Win64").join(delta::GAME);
    let context = presets::Context::new(cloud::DELTA_041_SCHEME, &policy, &exe);
    let defaults = presets::defaults(&context.profile, &policy.defaults);
    assert_eq!(defaults["max_generated_frames"], "3");
    assert!(
        context
            .parameters()
            .iter()
            .find(|p| p.key == "max_generated_frames")
            .unwrap()
            .choices
            .contains(&("5", "6X"))
    );
    for n in 0..=5 {
        let values = presets::Values::from([("max_generated_frames".into(), n.to_string())]);
        let text = String::from_utf8(context.configure(b"", &values)?)?;
        assert_eq!(
            diagnostics::ini_value(&text, "Compatibility", "DeltaForceGeneratedFrames")?,
            Some(n.to_string())
        );
        assert_eq!(
            diagnostics::ini_value(&text, "Compatibility", "DeltaForcePrivateStreamline")?
                .as_deref(),
            Some("1")
        );
    }
    for n in [4, 5] {
        let values = presets::Values::from([
            ("max_generated_frames".into(), n.to_string()),
            ("fg_engine".into(), "XeSS".into()),
        ]);
        let text = String::from_utf8(context.configure(b"", &values)?)?;
        assert_eq!(
            diagnostics::ini_value(&text, "FrameGeneration", "MaxGeneratedFrames")?.as_deref(),
            Some("3")
        );
    }
    let mut shared = recommendation_presets::Snapshot {
        schema: 1,
        profile: context.profile.clone(),
        defaults: defaults.clone(),
        values: defaults,
    };
    shared
        .values
        .insert("max_generated_frames".into(), "5".into());
    shared.validate(cloud::DELTA_041_SCHEME, delta::SIX_X_VERSION)?;
    assert!(
        context
            .validate_installed(Some("0.4.1"), &shared.values)
            .is_err()
    );
    context.validate_installed(Some(delta::SIX_X_VERSION), &shared.values)?;
    for version in ["0.4.0", "0.4.1", delta::SIX_X_VERSION] {
        let installed = presets::policy_for_version(&policy, Some(version));
        let context = presets::Context::new(cloud::DELTA_041_SCHEME, &installed, &exe);
        assert_eq!(context.delta_six, version == delta::SIX_X_VERSION);
    }
    Ok(())
}

#[test]
#[cfg(windows)]
#[ignore = "requires local official Streamline 2.14.16 files"]
fn delta214_cleanup_retains_busy_unknown_and_other_game_files() -> Result<()> {
    use std::{fs, os::windows::fs::OpenOptionsExt};
    let source = std::path::PathBuf::from(std::env::var("RTXFG_DELTA214_INPUT")?);
    let d = tempfile::tempdir()?;
    let root = d.path().join("cache");
    let revision = "Streamline-2.14.16-13c73ca2332d375e";
    let mut games = Vec::new();
    for name in ["main", "other"] {
        let exe = d
            .path()
            .join(name)
            .join("DeltaForce/Binaries/Win64")
            .join(delta::GAME);
        fs::create_dir_all(exe.parent().unwrap())?;
        fs::write(&exe, b"fixture")?;
        let id = delta::cache_id(&exe);
        delta::register_at(&root, &exe, &id)?;
        let cache = root.join("games").join(&id).join(revision);
        fs::create_dir_all(&cache)?;
        for file in fs::read_dir(&source)? {
            let file = file?.path();
            if file.extension().is_some_and(|e| e == "dll") {
                fs::copy(&file, cache.join(file.file_name().unwrap()))?;
            }
        }
        games.push((exe, id, cache));
    }
    let (exe, id, cache) = &games[0];
    let held = fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(cache.join("sl.common.dll"))?;
    fs::write(cache.join("unknown.txt"), b"keep")?;
    let report = delta::clean_at(&root, exe, std::slice::from_ref(id), false);
    assert_eq!(report.files, 0);
    assert!(!report.pending.is_empty());
    assert!(cache.join("sl.interposer.dll").is_file());
    drop(held);
    let report = delta::clean_at(&root, exe, std::slice::from_ref(id), false);
    assert_eq!(report.files, 7);
    assert!(!report.pending.is_empty());
    assert_eq!(fs::read(cache.join("unknown.txt"))?, b"keep");
    assert!(games[1].2.join("sl.common.dll").is_file());
    fs::remove_file(cache.join("unknown.txt"))?;
    let report = delta::clean_at(&root, exe, std::slice::from_ref(id), false);
    assert!(report.pending.is_empty(), "{:?}", report.pending);
    assert!(!cache.parent().unwrap().exists());
    assert!(
        delta::clean_at(&root, exe, std::slice::from_ref(id), false)
            .pending
            .is_empty()
    );
    Ok(())
}
