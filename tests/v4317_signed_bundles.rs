//! Maintainer fixtures: signed files are copied and never loaded or executed.
use anyhow::Result;
use rtx_fg_manager::{cleanup, cloud, core, encore, jsonc, presets, upstream040};
use std::{fs, path::PathBuf, sync::atomic::AtomicBool};

fn install(target: &core::DeploymentTarget, prepared: &cloud::Prepared) -> Result<()> {
    let context = presets::Context::new(&prepared.scheme_id, &prepared.policy, &target.game_exe);
    core::deploy_prepared_context_at_with_overrides(
        target,
        &prepared.backend,
        &["version.dll".into()],
        None,
        prepared.files.clone(),
        Some(&prepared.version),
        Some(&context),
        None,
        prepared.upstream_version.as_deref(),
    )?;
    Ok(())
}

#[test]
#[ignore = "requires the reviewed signed ZIPs in isolated LOCALAPPDATA and RTXFG_4317_FIXTURE"]
fn signed_upgrade_orphan_recovery_and_reinstallation() -> Result<()> {
    assert!(std::env::var("LOCALAPPDATA")?.contains("v4317"));
    let fixture = PathBuf::from(std::env::var("RTXFG_4317_FIXTURE")?);
    let previous: cloud::Catalog =
        serde_json::from_slice(&fs::read(fixture.join("before/cloud-catalog.json"))?)?;
    let current = cloud::bundled();
    let cancel = AtomicBool::new(false);
    let entries = ["version.dll".into()];
    let old = cloud::prepare(&previous, encore::SCHEME, 1, &entries, &cancel, |_| {})?;
    let new = cloud::prepare(&current, encore::SCHEME, 1, &entries, &cancel, |_| {})?;
    assert_eq!(old.version, "1.4.5");
    assert_eq!(new.version, "1.5.0");
    let temp = tempfile::tempdir()?;
    let exe = temp.path().join("RTXFG-Isolated-Game.exe");
    fs::copy(std::env::current_exe()?, &exe)?;
    let target = core::DeploymentTarget::for_game(&exe);
    install(&target, &old)?;
    let original = b"{\r\n// player comment\r\n\"configVersion\":4,\"menuState\":{\"menuKey\":36},\"vendor\":{\"keep\":99},\"frameGeneration\":{\"mode\":\"fixed\",\"multiplier\":3}}\r\n";
    fs::write(target.directory.join(encore::CONFIG), original)?;
    install(&target, &new)?;
    assert_eq!(
        fs::read(target.directory.join("version.dll"))?,
        new.files["version.dll"]
    );
    let config = fs::read(target.directory.join(encore::CONFIG))?;
    assert!(String::from_utf8_lossy(&config).contains("// player comment"));
    let parsed = jsonc::Document::parse(&config)?;
    assert_eq!(parsed.value["vendor"]["keep"], 99);
    assert_eq!(parsed.value["menuState"]["menuKey"], 36);
    assert_eq!(encore::read(&config)?["xessTensorCores"], "0");
    let context = presets::Context::new(encore::SCHEME, &new.policy, &exe);
    core::apply_parameters_at(
        &target,
        &context,
        &presets::Values::from([
            ("xessTensorCores".into(), "1".into()),
            ("xefgTensorCores".into(), "0".into()),
            ("reflexMode".into(), "on".into()),
        ]),
    )?;
    assert_eq!(
        encore::read(&fs::read(target.directory.join(encore::CONFIG))?)?["xessTensorCores"],
        "1"
    );
    assert!(cleanup::clean_outcome_at(&target)?.complete);
    assert!(!target.directory.join("rtx-encore-runtimes").exists());

    // Reproduce old managers losing ownership after a private component changes.
    // This is an actual signed DLL, not a fabricated DLL digest or blanket folder deletion.
    let dlssg = cloud::prepare(&current, upstream040::SCHEME, 1, &entries, &cancel, |_| {})?;
    let sr = "dlssg_sm86/sr/nvngx_dlss.dll";
    let residual = target.directory.join(sr);
    fs::create_dir_all(residual.parent().unwrap())?;
    fs::write(&residual, &dlssg.files[sr])?;
    let capture = residual.parent().unwrap().join("player-capture.bin");
    fs::write(&capture, b"unrelated manual capture")?;
    assert!(core::record(&target.directory)?.is_none());
    assert!(cleanup::orphaned_components(&target.directory));
    assert!(cleanup::known_component(&residual, sr)?);
    assert!(cleanup::clean_outcome_at(&target)?.complete);
    assert!(!residual.exists());
    assert!(capture.is_file());
    install(&target, &dlssg)?;
    assert_eq!(
        core::record(&target.directory)?
            .unwrap()
            .payload_version
            .as_deref(),
        Some("0.4.1")
    );
    assert_eq!(fs::read(&residual)?, dlssg.files[sr]);
    assert!(cleanup::clean_outcome_at(&target)?.complete);
    assert!(capture.is_file());
    assert!(exe.is_file());
    Ok(())
}
