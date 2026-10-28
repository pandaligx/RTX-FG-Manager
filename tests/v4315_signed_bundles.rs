//! Maintainer-only integration against the existing signed MFG resource cache.
//! This exercises files and transactions, without loading a DLL or running a game.
use anyhow::Result;
use rtx_fg_manager::{
    cleanup, cloud, core, presets, recommendation_presets as shared, recommendations, rtxmfg,
    scanner::Game,
};
use std::{collections::BTreeMap, fs, sync::atomic::AtomicBool};

#[test]
#[ignore = "requires the existing MFG bundles in isolated LOCALAPPDATA"]
fn compatibility_share_install_apply_and_switch_across_game_folders() -> Result<()> {
    assert!(std::env::var("LOCALAPPDATA")?.contains("v4315"));
    let catalog = cloud::bundled();
    let cancel = AtomicBool::new(false);
    let temp = tempfile::tempdir()?;
    let mut paths = Vec::new();
    for name in ["DX12", "Alternative"] {
        let dir = temp.path().join(name);
        fs::create_dir_all(&dir)?;
        let exe = dir.join("FixtureGame.exe");
        fs::copy(std::env::current_exe()?, &exe)?;
        paths.push(exe.to_string_lossy().to_string());
        fs::write(dir.join("other-mod.txt"), b"preserve")?;
    }
    let game = Game {
        exe: paths[0].clone(),
        targets: paths.clone(),
        ..Game::default()
    };
    let targets = game.deployment_targets();
    assert_eq!(targets.len(), 2);
    let entries = vec!["version.dll".into()];
    let prepared = cloud::prepare(
        &catalog,
        shared::MFG_161_COMPAT,
        1,
        &entries,
        &cancel,
        |_| {},
    )?;
    for target in &targets {
        let context =
            presets::Context::new(shared::MFG_161_COMPAT, &prepared.policy, &target.game_exe);
        core::deploy_prepared_context_at_with_overrides(
            target,
            &prepared.backend,
            &entries,
            None,
            prepared.files.clone(),
            Some(&prepared.version),
            Some(&context),
            None,
            prepared.upstream_version.as_deref(),
        )?;
        let config = target.directory.join(rtxmfg::CONFIG);
        let mut json: serde_json::Value = serde_json::from_slice(&fs::read(&config)?)?;
        json["unrelatedCustomSetting"] = serde_json::json!(123);
        fs::write(&config, serde_json::to_vec_pretty(&json)?)?;
        core::apply_parameters_at(
            target,
            &context,
            &BTreeMap::from([("rtx_mode".into(), "3".into())]),
        )?;
    }
    let identity =
        serde_json::from_value(serde_json::json!({"id":"steam:292030","name":"Fixture"}))?;
    let recipe = recommendations::installed_recipe(
        &game,
        identity,
        &catalog,
        "RTX 3060".into(),
        "581.80".into(),
    )?;
    assert_eq!(recipe.patch_version, "1.6.1");
    let snapshot = recipe.presets.as_ref().unwrap();
    assert_eq!(snapshot.values["rtx_mode"], "3");
    assert!(
        snapshot
            .changes()
            .iter()
            .any(|(key, _, value)| *key == "rtx_mode" && *value == "3")
    );
    for key in [
        "rtx_nr",
        "rtx_xemfg",
        "rtx_opti_ui",
        "logging",
        "unrelatedCustomSetting",
    ] {
        assert!(!snapshot.values.contains_key(key), "{key}");
    }
    let bytes = serde_json::to_vec(&recipe)?;
    assert!(!String::from_utf8_lossy(&bytes).contains(&paths[0]));
    let mut imported: recommendations::Recipe = serde_json::from_slice(&bytes)?;
    imported.validate(&catalog)?;
    imported
        .presets
        .as_mut()
        .unwrap()
        .values
        .insert("rtx_mode".into(), "4".into());

    // Scheme-only does not overwrite the applied parameter file.
    let original = targets
        .iter()
        .map(|t| fs::read(t.directory.join(rtxmfg::CONFIG)).unwrap())
        .collect::<Vec<_>>();
    shared::execute(
        &game,
        &catalog,
        &imported,
        shared::Action::InstallOnly,
        1,
        &cancel,
        |_| {},
    )?;
    for (target, previous) in targets.iter().zip(&original) {
        assert_eq!(&fs::read(target.directory.join(rtxmfg::CONFIG))?, previous);
    }
    shared::execute(
        &game,
        &catalog,
        &imported,
        shared::Action::ParametersOnly,
        1,
        &cancel,
        |_| {},
    )?;
    for target in &targets {
        let config = fs::read(target.directory.join(rtxmfg::CONFIG))?;
        assert_eq!(rtxmfg::read(&config)?["rtx_mode"], "4");
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&config)?["unrelatedCustomSetting"],
            123
        );
        assert!(cleanup::clean_outcome_at(target)?.complete);
    }
    // A fresh complete setup installs the exact compatibility branch in both folders.
    shared::execute(
        &game,
        &catalog,
        &recipe,
        shared::Action::InstallAndApply,
        1,
        &cancel,
        |_| {},
    )?;
    for target in &targets {
        assert_eq!(
            rtxmfg::read(&fs::read(target.directory.join(rtxmfg::CONFIG))?)?["rtx_mode"],
            "3"
        );
        assert_eq!(
            core::record(&target.directory)?
                .unwrap()
                .scheme_id
                .as_deref(),
            Some(shared::MFG_161_COMPAT)
        );
    }
    // Cross-version cards fail before changing the installed binary or parameters.
    let before = fs::read(targets[0].directory.join("version.dll"))?;
    let mut invalid = recipe.clone();
    invalid.patch_version = "1.7.0".into();
    assert!(
        shared::execute(
            &game,
            &catalog,
            &invalid,
            shared::Action::InstallAndApply,
            1,
            &cancel,
            |_| {}
        )
        .is_err()
    );
    assert_eq!(fs::read(targets[0].directory.join("version.dll"))?, before);
    let mut current = recipe.clone();
    current.scheme = "rtx40mfg-1.3.3-hf2".into();
    current.patch_version = "1.7.0".into();
    assert!(
        shared::execute(
            &game,
            &catalog,
            &current,
            shared::Action::ParametersOnly,
            1,
            &cancel,
            |_| {}
        )
        .is_err()
    );
    for target in &targets {
        assert!(cleanup::clean_outcome_at(target)?.complete);
        assert_eq!(
            fs::read(target.directory.join("other-mod.txt"))?,
            b"preserve"
        );
        assert!(target.game_exe.is_file());
    }
    shared::execute(
        &game,
        &catalog,
        &current,
        shared::Action::InstallAndApply,
        1,
        &cancel,
        |_| {},
    )?;
    for target in &targets {
        assert_eq!(
            core::record(&target.directory)?
                .unwrap()
                .payload_version
                .as_deref(),
            Some("1.7.0")
        );
        assert!(cleanup::clean_outcome_at(target)?.complete);
    }
    Ok(())
}
