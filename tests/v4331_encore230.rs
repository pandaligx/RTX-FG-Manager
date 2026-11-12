//! Versioned config tests against Encore 2.3's generated defaults, without a game.
use anyhow::Result;
use rtx_fg_manager::{
    encore,
    encore_install::{Choice, Frames, Reconstruction},
    presets::Values,
    recommendation_presets::Snapshot,
    scanner::Evidence,
};

#[test]
fn generated_230_template_matches_fresh_defaults_and_paths() -> Result<()> {
    let bytes = include_bytes!("fixtures/encore-2.3.0.jsonc");
    encore::validate_cloud_config(bytes)?;
    let actual = encore::read_for_version(bytes, Some("2.3.0"))?;
    let expected = encore::defaults_for_version(Some("2.3.0"));
    for f in encore::fields() {
        assert_eq!(actual[&f.key], expected[&f.key], "{}", f.key);
    }
    Ok(())
}

#[test]
fn old_files_keep_exposure_and_explicit_motion_while_fresh_defaults_change() -> Result<()> {
    let old = br#"{ // preserve notes
      "configVersion":4,"dlssSuperResolution":{"presentOpticalFlow":"quality"},
      "neuralRendering":{"core":{"nrPasses":3}},"vendor":{"nrExposureMode":"unknown"}}
    "#;
    let updated = encore::configure_for_version(old, &Values::new(), Some("2.3.0"))?;
    let read = encore::read_for_version(&updated, Some("2.3.0"))?;
    assert_eq!(read["nrExposureMode"], "game");
    assert_eq!(read["presentOpticalFlow"], "quality");
    assert_eq!(read["nrPasses"], "3");
    assert!(String::from_utf8_lossy(&updated).contains("preserve notes"));
    assert!(String::from_utf8_lossy(&updated).contains("\"unknown\""));
    assert_eq!(
        updated,
        encore::configure_for_version(&updated, &Values::new(), Some("2.3.0"))?
    );
    assert_eq!(
        encore::read_for_version(b"{}", Some("2.3.0"))?["presentOpticalFlow"],
        "game"
    );
    assert_eq!(
        encore::read_for_version(b"{}", Some("2.2.0"))?["presentOpticalFlow"],
        "encore"
    );
    let fresh = encore::defaults_for_version(Some("2.3.0"));
    assert_eq!(fresh["nrExposureMode"], "auto");
    assert_eq!(fresh["presentOpticalFlow"], "game");
    assert_eq!(
        encore::defaults_for_version(Some("2.2.0"))["nrExposureMode"],
        "game"
    );
    Ok(())
}

#[test]
fn new_menu_values_round_trip_and_cannot_be_applied_to_older_patches() -> Result<()> {
    let values = Values::from([
        ("nrPlacement".into(), "present".into()),
        ("presentOpticalFlow".into(), "game".into()),
        ("dlssWithoutGameDlssEngine".into(), "fsr1".into()),
        ("nrExposureMode".into(), "measured".into()),
        ("xefgSourcePacingMode".into(), "manual".into()),
        ("xefgSourcePacingLossPercent".into(), "30".into()),
        ("xefgSourcePacingManualDetection".into(), "0".into()),
        ("xefgSourcePacingManualAdjustments".into(), "0".into()),
    ]);
    for (k, v) in &values {
        assert!(!encore::value_available(k, v, Some("2.2.0")));
    }
    assert!(encore::configure_for_version(b"{}", &values, Some("2.2.0")).is_err());
    let bytes = encore::configure_for_version(b"{}", &values, Some("2.3.0"))?;
    let current = encore::read_for_version(&bytes, Some("2.3.0"))?;
    for (k, v) in &values {
        assert_eq!(&current[k], v);
    }
    assert!(!encore::valid_value("xefgSourcePacingLossPercent", "31"));
    assert!(!encore::valid_value("xefgSourcePacingLossPercent", "-1"));
    assert!(!encore::valid_value("xefgSourcePacingLossPercent", "4.5"));
    let shared = Snapshot {
        schema: 1,
        profile: encore::PROFILE.into(),
        values: values.clone(),
        defaults: values,
    };
    shared.validate(encore::SCHEME, "2.3.0")?;
    assert!(
        shared
            .for_version(encore::SCHEME, "2.3.0", "2.2.0")
            .is_err()
    );
    Ok(())
}

#[test]
fn fsr1_needs_no_neural_or_intel_dll_and_shares_spatial_controls() {
    let field = encore::field("dlssWithoutGameDlssEngine").unwrap();
    assert!(encore::requires_restart(field, Some("2.2.0")));
    assert!(!encore::requires_restart(field, Some("2.3.0")));
    let mut choice = Choice::new(
        "fixture.exe".into(),
        "Fixture".into(),
        &Evidence::default(),
        false,
    );
    choice.frames = Some(Frames::None);
    choice.reconstruction = Some(Reconstruction::Fsr1);
    choice.components.sr = false;
    choice.components.intel = false;
    choice.components.nr = false;
    assert!(choice.values().is_none());
    choice.fsr1_available = true;
    let values = choice.values().unwrap();
    assert_eq!(values["dlssWithoutGameDlssEngine"], "fsr1");
    for key in ["spatialSharpness", "spatialAa", "spatialAaPlacement"] {
        assert!(encore::field_enabled(key, &values, None));
    }
    let shared = Snapshot {
        schema: 1,
        profile: encore::PROFILE.into(),
        values: values.clone(),
        defaults: values,
    };
    assert!(shared.added_runtime_dependencies().is_empty());
}

#[test]
fn xefg_manual_controls_follow_mode_and_enable_state() {
    let mut values = encore::defaults_for_version(Some("2.3.0"));
    values.insert("frameGenerationEngine".into(), "xefg".into());
    assert!(!encore::field_enabled(
        "xefgSourcePacingLossPercent",
        &values,
        None
    ));
    values.insert("xefgSourcePacingMode".into(), "manual".into());
    assert!(encore::field_enabled(
        "xefgSourcePacingLossPercent",
        &values,
        None
    ));
    values.insert("xefgSourcePacing".into(), "0".into());
    assert!(!encore::field_enabled(
        "xefgSourcePacingLossPercent",
        &values,
        None
    ));
}

#[test]
#[ignore = "uses local signed 2.2/2.3 archives in an isolated cache; does not run a game"]
fn signed_upgrade_share_uninstall_and_fresh_reinstall() -> Result<()> {
    use rtx_fg_manager::{cleanup, cloud, core, encore_components, presets, transfer};
    use std::{fs, path::PathBuf, sync::atomic::AtomicBool};
    let output = PathBuf::from(std::env::var("RTXFG_COMPONENT_QA")?);
    anyhow::ensure!(std::env::var("LOCALAPPDATA")?.contains("development"));
    let old_catalog: cloud::Catalog =
        serde_json::from_slice(&fs::read(output.join("before/rust/cloud-catalog.json"))?)?;
    let catalog = cloud::bundled();
    let cancel = AtomicBool::new(false);
    let mut network = false;
    let mut prepare = |c: &cloud::Catalog| {
        cloud::prepare_with_progress(
            c,
            encore::SCHEME,
            1,
            &["version.dll".into()],
            &cancel,
            |p| network |= p.transfer.phase == transfer::Phase::Downloading,
        )
    };
    let old = prepare(&old_catalog)?;
    let new = prepare(&catalog)?;
    anyhow::ensure!(!network, "use existing local archives only");
    assert_eq!(old.upstream_version.as_deref(), Some("2.2.0"));
    assert_eq!(new.upstream_version.as_deref(), Some("2.3.0"));
    let tmp = tempfile::tempdir_in(&output)?;
    let patch_only = encore_components::Components {
        patch: true,
        sr: false,
        rr: false,
        fg: false,
        nr: false,
        intel: false,
        streamline: false,
        added_fg: false,
    };
    for folder in ["Primary", "SecondExecutable"] {
        let dir = tmp.path().join(folder);
        fs::create_dir(&dir)?;
        let exe = dir.join("EncoreMigrationTest.exe");
        fs::copy(std::env::current_exe()?, &exe)?;
        let target = core::DeploymentTarget::for_game(&exe);
        let context = presets::Context::new(encore::SCHEME, &new.policy, &exe);
        let install = |payload: &cloud::Prepared, values: &Values| {
            encore_components::deploy(
                &target,
                payload,
                &["version.dll".into()],
                payload.files.clone(),
                &context,
                values,
                patch_only,
            )
        };
        install(&old, &Values::new())?;
        let config = dir.join(encore::CONFIG);
        let prior = encore::configure_for_version(
            &fs::read(&config)?,
            &Values::from([
                ("presentOpticalFlow".into(), "quality".into()),
                ("nrPasses".into(), "3".into()),
            ]),
            Some("2.2.0"),
        )?;
        let document = rtx_fg_manager::jsonc::Document::parse(&prior)?;
        let prior = document.set(
            &["vendor".into(), "preserve".into()],
            serde_json::json!(true),
        )?;
        fs::write(&config, prior)?;
        install(&new, &Values::new())?;
        let bytes = fs::read(&config)?;
        let values = encore::read_for_version(&bytes, Some("2.3.0"))?;
        assert_eq!(values["nrExposureMode"], "game");
        assert_eq!(values["presentOpticalFlow"], "quality");
        assert_eq!(values["nrPasses"], "3");
        assert!(
            rtx_fg_manager::jsonc::Document::parse(&bytes)?.value["vendor"]["preserve"] == true
        );
        let shared = Values::from([
            ("nrExposureMode".into(), "measured".into()),
            ("dlssWithoutGameDlssEngine".into(), "fsr1".into()),
            ("nrPlacement".into(), "present".into()),
        ]);
        install(&new, &shared)?;
        assert_eq!(
            encore::read(&fs::read(&config)?)?["nrExposureMode"],
            "measured"
        );
        assert!(cleanup::clean_outcome_at(&target)?.complete);
        assert!(!config.exists() && !dir.join(core::OWN).exists());
        install(&new, &Values::new())?;
        let fresh = encore::read(&fs::read(&config)?)?;
        assert_eq!(fresh["nrExposureMode"], "auto");
        assert_eq!(fresh["presentOpticalFlow"], "game");
        assert_eq!(fresh["nrPlacement"], "after-sr");
        assert_eq!(fresh["dlssWithoutGameDlssEngine"], "dlss");
        assert!(cleanup::clean_outcome_at(&target)?.complete);
    }
    let ui_dir = output.join("ui-game");
    fs::create_dir_all(&ui_dir)?;
    let exe = ui_dir.join("Encore23Fixture.exe");
    fs::copy(std::env::current_exe()?, &exe)?;
    let target = core::DeploymentTarget::for_game(&exe);
    let context = presets::Context::new(encore::SCHEME, &new.policy, &exe);
    encore_components::deploy(
        &target,
        &new,
        &["version.dll".into()],
        new.files.clone(),
        &context,
        &Values::new(),
        patch_only,
    )?;
    core::atomic_json(
        &output.join("signed-upgrade.json"),
        &serde_json::json!({"old":"2.2.0","new":"2.3.0","directories":2,"upgrade_preserves_settings":true,"shared_parameters_cleared_after_uninstall":true,"network_download":network,"dll_execution":false}),
    )?;
    Ok(())
}
