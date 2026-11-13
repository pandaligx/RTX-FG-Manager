//! Encore 2.4 configuration/upgrade contracts; these are not GPU feature tests.
use anyhow::Result;
use rtx_fg_manager::{encore, presets::Values, recommendation_presets::Snapshot};
#[test]
#[ignore = "uses local signed 2.3/2.4 archives in an isolated cache; does not run a game"]
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
    assert_eq!(old.upstream_version.as_deref(), Some("2.3.0"));
    assert_eq!(new.upstream_version.as_deref(), Some("2.4.0"));
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
            Some("2.3.0"),
        )?;
        let document = rtx_fg_manager::jsonc::Document::parse(&prior)?;
        let prior = document.set(
            &["vendor".into(), "preserve".into()],
            serde_json::json!(true),
        )?;
        fs::write(&config, prior)?;
        install(&new, &Values::new())?;
        let bytes = fs::read(&config)?;
        let values = encore::read_for_version(&bytes, Some("2.4.0"))?;
        assert_eq!(values["nrExposureMode"], "auto");
        assert_eq!(values["nrPass1Placement"], "inherit");
        assert_eq!(values["presentOpticalFlow"], "quality");
        assert_eq!(values["nrPasses"], "3");
        assert!(
            rtx_fg_manager::jsonc::Document::parse(&bytes)?.value["vendor"]["preserve"] == true
        );
        let shared = Values::from([
            ("nrExposureMode".into(), "measured".into()),
            ("nrPass1Placement".into(), "before-sr".into()),
            ("nrPass2Placement".into(), "present".into()),
            ("dlssCustomScale".into(), "33".into()),
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
        assert_eq!(fresh["nrPass1Placement"], "inherit");
        assert_eq!(fresh["nrPass2Placement"], "inherit");
        assert_eq!(fresh["dlssCustomScale"], "67");
        assert_eq!(fresh["presentOpticalFlow"], "game");
        assert_eq!(fresh["nrPlacement"], "after-sr");
        assert_eq!(fresh["dlssWithoutGameDlssEngine"], "dlss");
        assert!(cleanup::clean_outcome_at(&target)?.complete);
    }
    let ui_dir = output.join("ui-game");
    fs::create_dir_all(&ui_dir)?;
    let exe = ui_dir.join("Encore24Fixture.exe");
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
        &serde_json::json!({"old":"2.3.0","new":"2.4.0","directories":2,"upgrade_preserves_settings":true,"shared_parameters_cleared_after_uninstall":true,"network_download":network,"dll_execution":false}),
    )?;
    Ok(())
}

#[test]
fn generated_defaults_match_and_every_new_placement_has_a_version_gate() -> Result<()> {
    let bytes = include_bytes!("fixtures/encore-2.4.0.jsonc");
    encore::validate_cloud_config(bytes)?;
    let actual = encore::read_for_version(bytes, Some("2.4.0"))?;
    let defaults = encore::defaults_for_version(Some("2.4.0"));
    for f in encore::fields() {
        assert_eq!(actual[&f.key], defaults[&f.key], "{}", f.key);
    }
    for n in 1..=4 {
        let key = format!("nrPass{n}Placement");
        for value in ["inherit", "before-sr", "after-sr", "present"] {
            assert!(encore::value_available(&key, value, Some("2.4.0")));
            assert!(!encore::value_available(&key, value, Some("2.3.0")));
        }
    }
    Ok(())
}

#[test]
fn mixed_placements_preserve_old_comments_unknowns_and_pass_appearance() -> Result<()> {
    let old = include_bytes!("fixtures/encore-2.3.0.jsonc");
    let base = rtx_fg_manager::jsonc::Document::parse(old)?.set(
        &["customVendor".into()],
        serde_json::json!({"untouched":true}),
    )?;
    let edits = Values::from([
        ("nrPlacement".into(), "after-sr".into()),
        ("nrPass1Placement".into(), "before-sr".into()),
        ("nrPass2Placement".into(), "present".into()),
        ("nrPass3Placement".into(), "inherit".into()),
        ("nrPass1Style".into(), "cinematic".into()),
        ("nrPass1Preset".into(), "b".into()),
        ("nrPasses".into(), "3".into()),
        ("nrEnabled".into(), "1".into()),
        ("dlssCustomScale".into(), "33".into()),
    ]);
    assert!(encore::configure_for_version(&base, &edits, Some("2.3.0")).is_err());
    let updated = encore::configure_for_version(&base, &edits, Some("2.4.0"))?;
    let actual = encore::read_for_version(&updated, Some("2.4.0"))?;
    for (k, v) in &edits {
        assert_eq!(&actual[k], v, "{k}");
    }
    assert_eq!(
        rtx_fg_manager::jsonc::Document::parse(&updated)?.value["customVendor"]["untouched"],
        true
    );
    assert!(String::from_utf8_lossy(&updated).contains("//"));
    assert_eq!(
        updated,
        encore::configure_for_version(&updated, &edits, Some("2.4.0"))?
    );
    assert!(encore::field_enabled("nrPass1Preset", &actual, None));
    assert!(!encore::field_enabled("nrPass2Preset", &actual, None));
    assert!(!encore::field_enabled("nrPass4Placement", &actual, None));
    let untouched = encore::read_for_version(&base, Some("2.4.0"))?;
    for n in 1..=4 {
        assert_eq!(untouched[&format!("nrPass{n}Placement")], "inherit");
    }
    let share = Snapshot {
        schema: 1,
        profile: encore::PROFILE.into(),
        values: edits.clone(),
        defaults: edits,
    };
    share.validate(encore::SCHEME, "2.4.0")?;
    assert!(share.for_version(encore::SCHEME, "2.4.0", "2.3.0").is_err());
    Ok(())
}

#[test]
fn custom_scale_minimum_is_versioned_in_client_and_exported_contract() -> Result<()> {
    let field = encore::field("dlssCustomScale").unwrap();
    assert_eq!(encore::minimum_for_version(field, Some("2.4.0")), Some(33.));
    assert_eq!(encore::minimum_for_version(field, Some("2.3.0")), Some(50.));
    assert_eq!(encore::minimum_for_version(field, None), Some(50.));
    for value in ["33", "49", "50", "100"] {
        let values = Values::from([("dlssCustomScale".into(), value.into())]);
        encore::configure_for_version(b"{}", &values, Some("2.4.0"))?;
        assert_eq!(
            encore::configure_for_version(b"{}", &values, Some("2.3.0")).is_ok(),
            value.parse::<u32>()? >= 50
        );
    }
    for invalid in ["32", "101", "NaN", "inf", "33.5"] {
        assert!(!encore::valid_value("dlssCustomScale", invalid));
    }
    let contract = rtx_fg_manager::recommendation_presets::contracts();
    assert_eq!(
        contract[encore::SCHEME]["fields"]["dlssCustomScale"]["minBefore"]["version"],
        "2.4.0"
    );
    Ok(())
}
