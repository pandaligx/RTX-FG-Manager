//! Maintainer fixture. Signed payloads are copied, never loaded or executed.
use anyhow::Result;
use rtx_fg_manager::{cleanup, cloud, core, encore, jsonc, presets};
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
#[ignore = "requires reviewed signed ZIPs in isolated LOCALAPPDATA and RTXFG_4318_FIXTURE"]
fn signed_upgrade_configuration_and_owned_cleanup() -> Result<()> {
    assert!(std::env::var("LOCALAPPDATA")?.contains("v4318"));
    let fixture = PathBuf::from(std::env::var("RTXFG_4318_FIXTURE")?);
    let previous: cloud::Catalog =
        serde_json::from_slice(&fs::read(fixture.join("before/cloud-catalog.json"))?)?;
    let current = cloud::bundled();
    let cancel = AtomicBool::new(false);
    let entries = ["version.dll".into()];
    let old = cloud::prepare(&previous, encore::SCHEME, 1, &entries, &cancel, |_| {})?;
    let new = cloud::prepare(&current, encore::SCHEME, 1, &entries, &cancel, |_| {})?;
    assert_eq!(old.version, "1.5.0");
    assert_eq!(new.version, "1.5.3");
    let temp = tempfile::tempdir()?;
    for directory in ["dx11", "dx12"] {
        let dir = temp.path().join(directory);
        fs::create_dir_all(&dir)?;
        let exe = dir.join("RTXFG-Isolated-Game.exe");
        fs::copy(std::env::current_exe()?, &exe)?;
        let target = core::DeploymentTarget::for_game(&exe);
        install(&target, &old)?;
        let original = b"{\r\n// player comment\r\n\"configVersion\":4,\"menuState\":{\"menuKey\":36},\"vendor\":{\"keep\":99},\"nativeMenuFeatures\":{\"vsyncOff\":true},\"smoothMotion\":{\"smoothMotionTuringVariant\":\"v1\"},\"frameGeneration\":{\"mode\":\"fixed\",\"multiplier\":3}}\r\n";
        fs::write(dir.join(encore::CONFIG), original)?;
        fs::write(dir.join("libxess.dll"), b"game-owned Intel runtime")?;
        install(&target, &new)?;
        assert_eq!(fs::read(dir.join("version.dll"))?, new.files["version.dll"]);
        let config = fs::read(dir.join(encore::CONFIG))?;
        let parsed = jsonc::Document::parse(&config)?;
        assert!(String::from_utf8_lossy(&config).contains("// player comment"));
        assert_eq!(parsed.value["vendor"]["keep"], 99);
        assert_eq!(parsed.value["menuState"]["menuKey"], 36);
        assert_eq!(
            encore::read_for_version(&config, Some("1.5.3"))?["vsync"],
            "off"
        );
        let context = presets::Context::new(encore::SCHEME, &new.policy, &exe);
        core::apply_parameters_at(
            &target,
            &context,
            &presets::Values::from([
                ("vsync".into(), "on".into()),
                ("dlssRrPreset".into(), "f".into()),
            ]),
        )?;
        let after = fs::read(dir.join(encore::CONFIG))?;
        let values = encore::read_for_version(&after, Some("1.5.3"))?;
        assert_eq!(values["vsync"], "on");
        assert_eq!(values["dlssRrPreset"], "f");
        let doc = jsonc::Document::parse(&after)?;
        assert!(doc.value["nativeMenuFeatures"].get("vsyncOff").is_none());
        assert!(
            doc.value["smoothMotion"]
                .get("smoothMotionTuringVariant")
                .is_none()
        );
        let logs = dir.join("rtx-encore-logs");
        fs::create_dir_all(&logs)?;
        for name in [
            "rtx-encore.log",
            "rtx-encore-status.json",
            "rtx-encore_perf.csv",
            "rtx-encore-13512-134365138303574349.gpu.dmp",
        ] {
            fs::write(logs.join(name), b"generated")?;
        }
        fs::write(logs.join("manual-capture.dmp"), b"user data")?;
        assert!(cleanup::clean_outcome_at(&target)?.complete);
        assert!(!dir.join("version.dll").exists());
        assert!(!dir.join("rtx-encore-runtimes").exists());
        assert!(!logs.join("rtx-encore.log").exists());
        assert!(
            !logs
                .join("rtx-encore-13512-134365138303574349.gpu.dmp")
                .exists()
        );
        assert_eq!(fs::read(logs.join("manual-capture.dmp"))?, b"user data");
        assert_eq!(
            fs::read(dir.join("libxess.dll"))?,
            b"game-owned Intel runtime"
        );
        assert!(exe.exists());
    }
    Ok(())
}
