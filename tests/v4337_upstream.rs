//! Offline integration against the signed candidate archives, without starting a game.
use anyhow::{Result, ensure};
use rtx_fg_manager::{
    anr, cleanup, cloud, core, encore, encore_install, presets, recommendation_presets, scanner,
    transfer, upstream040,
};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};

fn archived(catalog: &cloud::Catalog, scheme: &str, entry: &str) -> Result<cloud::Prepared> {
    let mut network = false;
    let result = cloud::prepare_with_progress(
        catalog,
        scheme,
        1,
        &[entry.into()],
        &AtomicBool::new(false),
        |event| network |= event.transfer.phase == transfer::Phase::Downloading,
    )?;
    ensure!(!network, "signed fixture was not preloaded");
    Ok(result)
}
fn install(target: &core::DeploymentTarget, payload: &cloud::Prepared, entry: &str) -> Result<()> {
    let ctx = presets::Context::new(&payload.scheme_id, &payload.policy, &target.game_exe);
    core::deploy_prepared_context_at_with_overrides(
        target,
        &payload.backend,
        &[entry.into()],
        None,
        payload.files.clone(),
        Some(&payload.version),
        Some(&ctx),
        Some(&presets::Values::new()),
        payload.upstream_version.as_deref(),
    )?;
    Ok(())
}
fn fixture() -> Result<(tempfile::TempDir, core::DeploymentTarget, cloud::Catalog)> {
    let qa = PathBuf::from(std::env::var("RTXFG_UPDATE_QA")?);
    ensure!(
        Path::new(&std::env::var("LOCALAPPDATA")?).starts_with(&qa),
        "use the isolated test data directory"
    );
    let dir = tempfile::tempdir_in(&qa)?;
    let exe = dir.path().join("ShippingGame.exe");
    fs::copy(std::env::current_exe()?, &exe)?;
    let old = serde_json::from_slice(&fs::read(qa.join("before-catalog.json"))?)?;
    Ok((dir, core::DeploymentTarget::for_game(&exe), old))
}

#[test]
#[ignore = "requires locally staged signed Pipo, standard, ANR and Encore packages"]
fn pipo_uses_its_own_defaults_after_install_share_uninstall_and_reinstall() -> Result<()> {
    let (_dir, target, _) = fixture()?;
    let catalog = cloud::bundled();
    let pipo = upstream040::PIPO_SCHEME;
    let entry = "version.dll";
    assert_eq!(catalog.proxies(pipo).len(), 6);
    assert_eq!(catalog.selection_limit(pipo), 1);
    let p = archived(&catalog, pipo, entry)?;
    let standard = archived(&catalog, upstream040::SCHEME, entry)?;
    for (payload, preset, kernel) in [(&p, "C", "2"), (&standard, "A", "1"), (&p, "C", "2")] {
        install(&target, payload, entry)?;
        let path = target.directory.join(core::INI);
        let ctx = presets::Context::new(&payload.scheme_id, &payload.policy, &target.game_exe);
        let values = upstream040::read_041(&fs::read(&path)?)?;
        assert_eq!(
            (&*values["preset"], &*values["optimized"]),
            (preset, kernel)
        );
        core::apply_parameters_at(
            &target,
            &ctx,
            &presets::Values::from([("optimized".into(), "0".into())]),
        )?;
        let card = recommendation_presets::capture(
            &target,
            &catalog,
            &payload.scheme_id,
            payload.upstream_version.as_deref().unwrap(),
        )?;
        assert_eq!(card.values["optimized"], "0");
        assert_eq!(card.defaults["optimized"], kernel);
        assert_eq!(card.defaults["preset"], preset);
        let before = fs::read(&path)?;
        if payload.scheme_id == pipo {
            assert!(install(&target, &standard, entry).is_err());
            assert_eq!(fs::read(&path)?, before);
        }
        assert!(cleanup::clean_outcome_at(&target)?.complete);
        assert!(!path.exists() && !target.directory.join(core::OWN).exists());
        assert!(target.game_exe.exists());
    }
    Ok(())
}

#[test]
#[ignore = "requires locally staged signed B16 and B20 archives"]
fn anr_upgrade_owns_every_new_model_and_preserves_unrelated_files() -> Result<()> {
    let (_dir, target, previous) = fixture()?;
    let catalog = cloud::bundled();
    let entry = "dxgi.dll";
    let old = archived(&previous, anr::SCHEME, entry)?;
    let new = archived(&catalog, anr::SCHEME, entry)?;
    assert_eq!(new.files.len(), old.files.len() + 11);
    install(&target, &old, entry)?;
    let ctx = presets::Context::new(anr::SCHEME, &new.policy, &target.game_exe);
    core::apply_parameters_at(
        &target,
        &ctx,
        &presets::Values::from([("scene_cut".into(), "0".into())]),
    )?;
    let original = fs::read(target.directory.join(anr::CONFIG))?;
    fs::create_dir_all(target.directory.join("OptiScaler/other-plugin"))?;
    let other = target.directory.join("OptiScaler/other-plugin/player.txt");
    fs::write(&other, b"owned by the player")?;
    install(&target, &new, entry)?;
    assert_eq!(fs::read(target.directory.join(anr::CONFIG))?, original);
    for (name, bytes) in &new.files {
        if name != anr::CONFIG {
            assert_eq!(fs::read(target.directory.join(name))?, *bytes, "{name}");
        }
    }
    let provider = target.directory.join(anr::PROVIDER_DIR);
    let logs = [
        provider.join("nr_fg-test.log"),
        provider.parent().unwrap().join("nr_fg-previous.log"),
    ];
    for log in &logs {
        fs::write(log, b"ANR log")?;
    }
    #[cfg(windows)]
    let loader_copy = {
        let path = provider.join("anr-vkloader.dll");
        fs::copy(
            PathBuf::from(std::env::var("SystemRoot")?).join("System32/vulkan-1.dll"),
            &path,
        )?;
        path
    };
    assert!(cleanup::clean_outcome_at(&target)?.complete);
    #[cfg(windows)]
    assert!(!loader_copy.exists());
    assert!(logs.iter().all(|p| !p.exists()));
    for name in new.files.keys() {
        assert!(!target.directory.join(name).exists(), "{name}");
    }
    assert_eq!(fs::read(&other)?, b"owned by the player");
    assert!(install(&target, &new, entry).is_err());
    fs::rename(
        target.directory.join("OptiScaler"),
        target.directory.join("saved-other-plugin"),
    )?;
    install(&target, &new, entry)?;
    assert_eq!(
        fs::read(target.directory.join(anr::CONFIG))?,
        new.files[anr::CONFIG]
    );
    assert!(cleanup::clean_outcome_at(&target)?.complete);
    Ok(())
}

#[cfg(windows)]
#[test]
#[ignore = "uses the system Vulkan loader, anr-b15-catalog.json and signed B15/B16/B20 archives"]
fn anr_vulkan_loader_uninstall_and_legacy_reinstall() -> Result<()> {
    let (_dir, target, old_catalog) = fixture()?;
    let system_loader = PathBuf::from(std::env::var("SystemRoot")?).join("System32/vulkan-1.dll");
    let opti = target.directory.join("OptiScaler");
    let provider = target.directory.join(anr::PROVIDER_DIR);
    let loader = provider.join("anr-vkloader.dll");
    let current_catalog = cloud::bundled();
    let current = archived(&current_catalog, anr::SCHEME, "version.dll")?;
    let b15 = serde_json::from_slice(&fs::read(
        PathBuf::from(std::env::var("RTXFG_UPDATE_QA")?).join("anr-b15-catalog.json"),
    )?)?;
    for catalog in [&b15, &old_catalog, &current_catalog] {
        let patch = archived(catalog, anr::SCHEME, "version.dll")?;
        install(&target, &patch, "version.dll")?;
        fs::copy(&system_loader, &loader)?;
        assert!(cleanup::clean_outcome_at(&target)?.complete);
        assert!(
            !opti.exists(),
            "ANR's private Vulkan loader survived uninstall"
        );
        assert_eq!(core::status_at(&target), "未部署");
    }
    // An older uninstall already removed the journal and everything except the loader.
    for action in ["reinstall", "normal_uninstall", "cleanup_retry"] {
        fs::create_dir_all(&provider)?;
        fs::copy(&system_loader, &loader)?;
        assert!(core::status_at(&target).contains("残留"), "{action}");
        match action {
            "reinstall" => install(&target, &current, "version.dll")?,
            "normal_uninstall" => assert!(cleanup::clean_outcome_at(&target)?.complete),
            _ => assert!(cleanup::retry_pending_at(&target)?.complete),
        }
        assert!(!loader.exists(), "{action}");
        if action == "reinstall" {
            // A stale retry must not uninstall a fresh, working deployment.
            assert!(cleanup::retry_pending_at(&target).is_err());
            assert!(target.directory.join("version.dll").exists());
            assert!(cleanup::clean_outcome_at(&target)?.complete);
        }
        assert!(!opti.exists(), "{action}");
    }
    assert!(target.game_exe.is_file());
    // A separate conflict must be reported before removing the old loader.
    fs::create_dir_all(&provider)?;
    fs::copy(&system_loader, &loader)?;
    let entry = target.directory.join("version.dll");
    fs::write(&entry, b"another mod")?;
    assert!(install(&target, &current, "version.dll").is_err());
    assert!(loader.is_file());
    assert_eq!(fs::read(&entry)?, b"another mod");
    fs::remove_file(entry)?;
    assert!(cleanup::retry_pending_at(&target)?.complete);
    Ok(())
}

#[test]
#[ignore = "requires locally staged signed Encore 2.4.1 and 2.5.0 archives"]
fn encore_upgrade_preserves_jsonc_and_first_install_mwsr_remains_opt_in() -> Result<()> {
    let (_dir, target, previous) = fixture()?;
    let catalog = cloud::bundled();
    let entry = "version.dll";
    let old = archived(&previous, encore::SCHEME, entry)?;
    let new = archived(&catalog, encore::SCHEME, entry)?;
    install(&target, &old, entry)?;
    let path = target.directory.join(encore::CONFIG);
    let mut text = fs::read_to_string(&path)?;
    let i = text.find('{').unwrap() + 1;
    text.insert_str(i, "\n  // my menu settings\n  \"privateExtra\": 37,");
    fs::write(&path, text)?;
    install(&target, &new, entry)?;
    assert!(fs::read_to_string(&path)?.contains("// my menu settings"));
    assert!(fs::read_to_string(&path)?.contains("\"privateExtra\": 37"));
    let ctx = presets::Context::new(encore::SCHEME, &new.policy, &target.game_exe);
    let values = presets::Values::from([
        ("dlssWithoutGameDlss".into(), "1".into()),
        ("dlssWithoutGameDlssEngine".into(), "mwsr".into()),
        ("menuLanguage".into(), "ko".into()),
    ]);
    core::apply_parameters_at(&target, &ctx, &values)?;
    let card = recommendation_presets::capture(&target, &catalog, encore::SCHEME, "2.5.0")?;
    assert_eq!(card.values["dlssWithoutGameDlssEngine"], "mwsr");
    assert!(!card.values.contains_key("menuLanguage"));
    assert!(!card.added_runtime_dependencies().contains(&"sr"));
    assert!(cleanup::clean_outcome_at(&target)?.complete);
    install(&target, &new, entry)?;
    let clean = encore::read_for_version(&fs::read(path)?, Some("2.5.0"))?;
    assert_eq!(clean["dlssWithoutGameDlss"], "0");
    assert_eq!(clean["dlssWithoutGameDlssEngine"], "dlss");
    assert!(cleanup::clean_outcome_at(&target)?.complete);
    Ok(())
}

#[test]
fn encore_feature_gates_and_runtime_requirements_match_the_installed_version() {
    for (version, mwsr, korean) in [
        ("2.4.1", false, false),
        ("2.4.2", false, true),
        ("2.5.0", true, true),
    ] {
        assert_eq!(
            encore::value_available("dlssWithoutGameDlssEngine", "mwsr", Some(version)),
            mwsr
        );
        assert_eq!(
            encore::value_available("menuLanguage", "ko", Some(version)),
            korean
        );
    }
    let mut choice = encore_install::Choice::new(
        "test.exe".into(),
        "Test".into(),
        &scanner::Evidence::default(),
        false,
    );
    choice.frames = Some(encore_install::Frames::None);
    assert!(choice.values().is_none());
    choice.reconstruction = Some(encore_install::Reconstruction::Mwsr);
    choice.components.sr = false;
    choice.components.intel = false;
    assert!(choice.values().is_none());
    choice.mwsr_available = true;
    let values = choice.values().unwrap();
    assert_eq!(values["dlssWithoutGameDlssEngine"], "mwsr");
    assert!(encore::field_enabled("spatialSharpness", &values, None));
    let four = "F1, F2, F3, F4";
    assert!(encore::valid_hotkey(four));
    assert!(!encore::value_available(
        "hotkeyFixed2",
        four,
        Some("2.5.0")
    ));
    assert!(encore::value_available(
        "hotkeyFixed2",
        "F1, F2, F3",
        Some("2.5.0")
    ));
}
