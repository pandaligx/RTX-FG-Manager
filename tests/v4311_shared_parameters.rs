//! Isolated file/transaction tests. Does not load a DLL or run a game/GPU.
use anyhow::Result;
use rtx_fg_manager::{
    cloud, core, encore, presets, recommendation_presets as shared, recommendations::Recipe,
    scanner::Game,
};
use std::{collections::BTreeMap, fs, sync::atomic::AtomicBool};
fn pe(dll: bool) -> Vec<u8> {
    let mut b = vec![0; 512];
    b[..2].copy_from_slice(b"MZ");
    b[60..64].copy_from_slice(&128u32.to_le_bytes());
    b[128..132].copy_from_slice(b"PE\0\0");
    b[132..134].copy_from_slice(&0x8664u16.to_le_bytes());
    b[148..150].copy_from_slice(&240u16.to_le_bytes());
    b[150..152].copy_from_slice(&(if dll { 0x2000u16 } else { 2 }).to_le_bytes());
    b[152..154].copy_from_slice(&0x20bu16.to_le_bytes());
    b
}
fn fixture() -> Result<(tempfile::TempDir, Game, cloud::Catalog, Recipe)> {
    let temp = tempfile::tempdir()?;
    let catalog = cloud::bundled();
    let mut targets = Vec::new();
    let scheme = shared::PROFILES[0].0;
    for name in ["dx12", "vulkan"] {
        let dir = temp.path().join(name);
        fs::create_dir(&dir)?;
        let exe = dir.join("RTXFG-Shared-Parameters-Test.exe");
        fs::write(&exe, pe(false))?;
        let target = core::DeploymentTarget::for_game(&exe);
        let context = presets::Context::new(scheme, &catalog.scheme_policies[scheme], &exe);
        core::deploy_prepared_context_at_with_overrides(&target,encore::BACKEND,&["dxgi.dll".into()],None,BTreeMap::from([
            ("dxgi.dll".into(),pe(true)),(encore::CONFIG.into(),b"{\n// keep comment\n\"configVersion\":4,\"vendor\":{\"keep\":42},\"menuState\":{\"menuKey\":36}}".to_vec()),(encore::NOTICES.into(),b"fixture".to_vec())
        ]).into_iter().chain(encore::INTEL_FILES.into_iter().map(|n|(n.to_owned(),pe(true)))).collect(),Some("1.4.2"),Some(&context),None,Some("1.4.2"))?;
        targets.push(exe.to_string_lossy().to_string());
    }
    let game: Game = serde_json::from_value(
        serde_json::json!({"exe":targets[0],"targets":targets,"title":"Test"}),
    )?;
    let recipe: Recipe = serde_json::from_value(
        serde_json::json!({"schema":1,"game":{"id":"steam:292030","name":"Test"},"title":"Test parameters","notes":"","scheme":scheme,"patchVersion":"1.4.2","entries":["dxgi.dll"],"renderer":"dx12","gpuModel":"RTX 3060","driver":"581.80","gpuAlias":"unchanged","neuralRuntime":"none","presets":{"schema":1,"profile":encore::PROFILE,"values":{"srSharpening":"custom","srSharpness":"45","nrEnabled":"0"},"defaults":{"srSharpening":"off","srSharpness":"30","nrEnabled":"0"}}}),
    )?;
    Ok((temp, game, catalog, recipe))
}
#[test]
fn shared_parameters_apply_to_all_directories_and_preserve_private_content() -> Result<()> {
    let (_temp, game, catalog, recipe) = fixture()?;
    shared::execute(
        &game,
        &catalog,
        &recipe,
        shared::Action::ParametersOnly,
        1,
        &AtomicBool::new(false),
        |_| {},
    )?;
    for target in game.deployment_targets() {
        let bytes = fs::read(target.directory.join(encore::CONFIG))?;
        assert_eq!(encore::read(&bytes)?["srSharpness"], "45");
        assert!(String::from_utf8_lossy(&bytes).contains("// keep comment"));
        let doc = rtx_fg_manager::jsonc::Document::parse(&bytes)?;
        assert_eq!(doc.value["vendor"]["keep"], 42);
        assert_eq!(doc.value["menuState"]["menuKey"], 36);
        assert_eq!(fs::read(target.directory.join("dxgi.dll"))?, pe(true));
        let snapshot = shared::capture(&target, &catalog, &recipe.scheme, &recipe.patch_version)?;
        assert_eq!(snapshot.values["srSharpness"], "45");
        assert!(!snapshot.values.contains_key("hotkeyToggleNr"));
        assert!(!snapshot.values.contains_key("logNr"));
    }
    Ok(())
}
#[test]
fn all_targets_and_dependencies_are_checked_before_any_parameter_write() -> Result<()> {
    let (_temp, game, catalog, mut recipe) = fixture()?;
    let targets = game.deployment_targets();
    let original = fs::read(targets[0].directory.join(encore::CONFIG))?;
    recipe
        .presets
        .as_mut()
        .unwrap()
        .values
        .insert("nrEnabled".into(), "1".into());
    assert!(
        shared::execute(
            &game,
            &catalog,
            &recipe,
            shared::Action::ParametersOnly,
            1,
            &AtomicBool::new(false),
            |_| {}
        )
        .is_err()
    );
    assert_eq!(
        fs::read(targets[0].directory.join(encore::CONFIG))?,
        original
    );
    recipe
        .presets
        .as_mut()
        .unwrap()
        .values
        .insert("nrEnabled".into(), "0".into());
    fs::write(
        targets[1].directory.join("dxgi.dll"),
        b"changed outside manager",
    )?;
    assert!(
        shared::execute(
            &game,
            &catalog,
            &recipe,
            shared::Action::ParametersOnly,
            1,
            &AtomicBool::new(false),
            |_| {}
        )
        .is_err()
    );
    assert_eq!(
        fs::read(targets[0].directory.join(encore::CONFIG))?,
        original
    );
    Ok(())
}
#[test]
fn cancellation_and_legacy_shares_do_not_write_parameters() -> Result<()> {
    let (_temp, game, catalog, mut recipe) = fixture()?;
    let target = &game.deployment_targets()[0];
    let original = fs::read(target.directory.join(encore::CONFIG))?;
    assert!(
        shared::execute(
            &game,
            &catalog,
            &recipe,
            shared::Action::ParametersOnly,
            1,
            &AtomicBool::new(true),
            |_| {}
        )
        .is_err()
    );
    recipe.presets = None;
    assert!(
        shared::execute(
            &game,
            &catalog,
            &recipe,
            shared::Action::InstallAndApply,
            1,
            &AtomicBool::new(false),
            |_| {}
        )
        .is_err()
    );
    assert_eq!(fs::read(target.directory.join(encore::CONFIG))?, original);
    Ok(())
}

#[test]
fn parameter_only_does_not_install_shared_runtimes_or_require_their_downloads() -> Result<()> {
    let (_temp, game, catalog, mut recipe) = fixture()?;
    recipe.runtimes = Some(serde_json::from_value(
        serde_json::json!({"schema":1,"components":[{"kind":"sr","source":"game","versions":["999.0.0"]}]}),
    )?);
    assert!(recipe.runtimes.as_ref().unwrap().components[0].preserves_game_files());
    shared::execute(
        &game,
        &catalog,
        &recipe,
        shared::Action::ParametersOnly,
        1,
        &AtomicBool::new(false),
        |_| {},
    )?;
    for t in game.deployment_targets() {
        assert_eq!(
            encore::read(&fs::read(t.directory.join(encore::CONFIG))?)?["srSharpness"],
            "45"
        );
        assert!(!t.directory.join("nvngx_dlss.dll").exists());
        assert_eq!(fs::read(t.directory.join("dxgi.dll"))?, pe(true));
    }
    Ok(())
}

#[test]
fn unsupported_shared_runtime_fails_before_patch_or_parameter_changes() -> Result<()> {
    let (_temp, game, catalog, mut recipe) = fixture()?;
    recipe.runtimes = Some(serde_json::from_value(
        serde_json::json!({"schema":1,"components":[{"kind":"sr","source":"game","versions":["999.0.0"]}]}),
    )?);
    let target = &game.deployment_targets()[0];
    let original = fs::read(target.directory.join(encore::CONFIG))?;
    assert!(
        shared::execute(
            &game,
            &catalog,
            &recipe,
            shared::Action::InstallAndApply,
            1,
            &AtomicBool::new(false),
            |_| {}
        )
        .is_err()
    );
    assert_eq!(fs::read(target.directory.join(encore::CONFIG))?, original);
    assert_eq!(fs::read(target.directory.join("dxgi.dll"))?, pe(true));
    Ok(())
}

#[test]
fn sharing_captures_patch_runtimes_without_paths_or_proxy_dlls() -> Result<()> {
    let (_temp, game, _catalog, _recipe) = fixture()?;
    let snapshot = rtx_fg_manager::recommendation_runtimes::capture(&game.deployment_targets()[0])?;
    assert!(!snapshot.components.is_empty());
    assert!(
        snapshot
            .components
            .iter()
            .all(|c| c.source == "patch" && ["xess", "xess_fg", "xell"].contains(&c.kind.as_str()))
    );
    let public = serde_json::to_string(&snapshot)?;
    assert!(
        !public.contains("dxgi.dll") && !public.contains("path") && !public.contains("Test.exe")
    );
    // The synthetic PE fixture deliberately has no version resource; never claim a known SDK.
    assert!(
        snapshot
            .components
            .iter()
            .all(|c| c.versions == ["unknown"])
    );
    Ok(())
}
