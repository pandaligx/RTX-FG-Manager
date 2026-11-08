//! No game/GPU code executes: signed payloads are only copied and parsed.
use anyhow::Result;
use rtx_fg_manager::{
    cleanup, cloud, core, encore, jsonc, neural_runtime, presets, recommendation_runtimes,
    runtime_files,
};
use std::{fs, path::PathBuf, sync::atomic::AtomicBool};

fn install(t: &core::DeploymentTarget, p: &cloud::Prepared, entry: &str) -> Result<()> {
    let c = presets::Context::new(&p.scheme_id, &p.policy, &t.game_exe);
    core::deploy_prepared_context_at_with_overrides(
        t,
        &p.backend,
        &[entry.into()],
        None,
        p.files.clone(),
        Some(&p.version),
        Some(&c),
        None,
        p.upstream_version.as_deref(),
    )?;
    Ok(())
}

#[test]
#[ignore = "reviewed signed local fixture; explicit isolated LOCALAPPDATA required"]
fn signed_upgrade_added_engines_multifolder_asi_and_owned_cleanup() -> Result<()> {
    assert!(std::env::var("LOCALAPPDATA")?.contains("v4326-review"));
    let fixture = PathBuf::from(std::env::var("RTXFG_4326_FIXTURE")?);
    let old: cloud::Catalog =
        serde_json::from_slice(&fs::read(fixture.join("before/rust/cloud-catalog.json"))?)?;
    let new = cloud::bundled();
    let cancel = AtomicBool::new(false);
    let mut network = false;
    let previous = cloud::prepare_with_progress(
        &old,
        encore::SCHEME,
        1,
        &["version.dll".into()],
        &cancel,
        |p| {
            network |= p.transfer.phase == rtx_fg_manager::transfer::Phase::Downloading;
        },
    )?;
    let current = cloud::prepare_with_progress(
        &new,
        encore::SCHEME,
        1,
        &["version.dll".into()],
        &cancel,
        |p| {
            network |= p.transfer.phase == rtx_fg_manager::transfer::Phase::Downloading;
        },
    )?;
    let asi = cloud::prepare_with_progress(
        &new,
        encore::SCHEME,
        1,
        &["rtx-encore.asi".into()],
        &cancel,
        |p| {
            network |= p.transfer.phase == rtx_fg_manager::transfer::Phase::Downloading;
        },
    )?;
    assert_eq!(previous.version, "2.0.1");
    assert_eq!(current.version, "2.1.0");
    assert!(!network);
    let tmp = tempfile::tempdir()?;
    let mut targets = Vec::new();
    for (i, name) in ["DX11", "DX12", "VulkanASI"].iter().enumerate() {
        let dir = tmp.path().join(name);
        fs::create_dir_all(&dir)?;
        let exe = dir.join("IsolatedGame.exe");
        fs::copy(std::env::current_exe()?, &exe)?;
        let t = if i == 2 {
            fs::create_dir(dir.join("plugins"))?;
            core::DeploymentTarget::custom(&exe, &dir.join("plugins"))
        } else {
            core::DeploymentTarget::for_game(&exe)
        };
        if i == 0 {
            install(&t, &previous, "version.dll")?;
            fs::write(dir.join(encore::CONFIG),br#"{
// retain player comment
"configVersion":4,"frameGeneration":{"flipMeteringBypass":true},"menuState":{"menuLanguage":"fr"},"vendor":{"keep":42}}
"#)?;
        }
        install(
            &t,
            if i == 2 { &asi } else { &current },
            if i == 2 {
                "rtx-encore.asi"
            } else {
                "version.dll"
            },
        )?;
        let config = fs::read(t.directory.join(encore::CONFIG))?;
        let values = encore::read_for_version(&config, Some("2.1.0"))?;
        assert_eq!(
            values["flipMeteringBypass"],
            if i == 0 { "1" } else { "auto" }
        );
        if i == 0 {
            let d = jsonc::Document::parse(&config)?;
            assert_eq!(d.value["vendor"]["keep"], 42);
            assert!(String::from_utf8_lossy(&config).contains("// retain player comment"));
        }
        for kind in ["sr", "encorefg"] {
            assert!(runtime_files::inspect(&t, kind).available, "{kind}");
        }
        // Missing dependencies must never change the user's configuration.
        let ctx = presets::Context::new(encore::SCHEME, &current.policy, &exe);
        assert!(
            core::apply_parameters_at(
                &t,
                &ctx,
                &presets::Values::from([("frameGenerationWithoutGameFg".into(), "1".into())])
            )
            .is_err()
        );
        assert_eq!(fs::read(t.directory.join(encore::CONFIG))?, config);
        targets.push(t);
    }
    for kind in ["sr", "encorefg"] {
        runtime_files::install_many(&targets, kind, false, &cancel, &mut |_| {})?;
    }
    neural_runtime::install_many(&targets, "310.9.16", false, &cancel, &mut |_| {})?;
    for t in &targets {
        let ctx = presets::Context::new(encore::SCHEME, &current.policy, &t.game_exe);
        core::apply_parameters_at(
            t,
            &ctx,
            &presets::Values::from([
                ("dlssWithoutGameDlss".into(), "1".into()),
                ("nrWithoutGameDlss".into(), "1".into()),
                ("frameGenerationWithoutGameFg".into(), "1".into()),
                ("tf_mode".into(), "3".into()),
            ]),
        )?;
        assert_eq!(
            runtime_files::installed_package_version(t, "encorefg")?.as_deref(),
            Some("2.14.16")
        );
        assert!(!runtime_files::inspect(t, "fg").available);
        let share = recommendation_runtimes::capture(t)?;
        assert!(share.components.iter().any(|c| c.kind == "encorefg"));
        share.verify_installed(std::slice::from_ref(t))?;
        let log = t.directory.join("rtx-encore-logs");
        fs::create_dir_all(&log)?;
        fs::write(log.join("rtx-encore.log"), b"session")?;
        fs::write(log.join("manual-capture.dmp"), b"user data")?;
        assert!(cleanup::clean_outcome_at(t)?.complete);
        assert!(!t.directory.join("nvngx_dlss.dll").exists());
        assert!(!t.directory.join("nvngx_dlssnr.dll").exists());
        assert!(
            !t.game_exe
                .parent()
                .unwrap()
                .join("sl.interposer.dll")
                .exists()
        );
        assert!(!log.join("rtx-encore.log").exists());
        assert_eq!(fs::read(log.join("manual-capture.dmp"))?, b"user data");
        assert!(t.game_exe.exists());
    }
    Ok(())
}

#[test]
#[ignore = "reviewed signed local fixture; explicit isolated LOCALAPPDATA required"]
fn historical_patch_only_card_completes_base_runtimes() -> Result<()> {
    assert!(std::env::var("LOCALAPPDATA")?.contains("v4326-review"));
    let tmp = tempfile::tempdir()?;
    let exe = tmp.path().join("IsolatedGame.exe");
    fs::copy(std::env::current_exe()?, &exe)?;
    let t = &core::DeploymentTarget::for_game(&exe);
    let new = cloud::bundled();
    let cancel = AtomicBool::new(false);
    // Historical cards may contain no shared runtime snapshot. Patch-only still
    // installs Encore's base dependencies, without enabling unrequested features.
    let recipe = serde_json::from_value(serde_json::json!({
        "schema":1,"game":{"id":"steam:2012510","name":"Fixture"},
        "title":"Old card","notes":"","scheme":encore::SCHEME,"patchVersion":"1.4.2",
        "entries":["version.dll"],"renderer":"dx12","gpuModel":"","driver":"",
        "gpuAlias":"unchanged","neuralRuntime":"none"
    }))?;
    let game = rtx_fg_manager::scanner::Game {
        exe: t.game_exe.to_string_lossy().into_owned(),
        ..Default::default()
    };
    rtx_fg_manager::recommendation_presets::execute(
        &game,
        &new,
        &recipe,
        rtx_fg_manager::recommendation_presets::Action::InstallOnly,
        1,
        &cancel,
        |_| {},
    )?;
    assert!(neural_runtime::inspect(t).present);
    assert!(runtime_files::inspect(t, "sr").managed);
    assert!(runtime_files::inspect(t, "rr").managed);
    assert!(runtime_files::inspect(t, "encorefg").managed);
    let values =
        encore::read_for_version(&fs::read(t.directory.join(encore::CONFIG))?, Some("2.1.0"))?;
    assert_eq!(values["dlssWithoutGameDlss"], "0");
    assert_eq!(values["frameGenerationWithoutGameFg"], "0");
    assert!(cleanup::clean_outcome_at(t)?.complete);
    Ok(())
}
