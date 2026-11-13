use anyhow::Result;
use rtx_fg_manager::{core, presets::Values, recommendation_presets, rtxmfg};
use serde_json::{Value, json};

#[test]
fn named_looks_network_presets_and_unknown_values_stay_independent() -> Result<()> {
    let values = Values::from([
        ("rtx_nr_model".into(), "2".into()),
        ("rtx_nr_preset".into(), "1".into()),
        ("rtx_nr_structure".into(), "1.5".into()),
        ("rtx_nr_tone".into(), "0.75".into()),
        ("rtx_nr_character_mask".into(), "1".into()),
        ("rtx_nr_character".into(), "-1".into()),
    ]);
    let original = br#"{"dlss5DefaultsVersion":3,"future":{"keep":true},"dlss5LaterModel":1}"#;
    let configured = rtxmfg::configure(original, &values)?;
    let document: Value = serde_json::from_slice(&configured)?;
    assert_eq!(document["dlss5Model"], 2);
    assert_eq!(document["dlss5Preset"], 1);
    assert_eq!(document["dlss5StructureIntensity"], 1.5);
    assert_eq!(document["dlss5CharacterIntensity"], -1.0);
    assert_eq!(document["dlss5CharacterMask"], true);
    assert_eq!(document["future"], json!({"keep":true}));
    assert_eq!(document["dlss5LaterModel"], 1);
    let read = rtxmfg::read(&configured)?;
    for (key, value) in values {
        if key == "rtx_nr_character" {
            assert_eq!(read[&key].parse::<f64>()?, -1.);
        } else {
            assert_eq!(read[&key], value);
        }
    }
    Ok(())
}

#[test]
fn new_keys_are_version_gated_and_diagnostics_never_shared() -> Result<()> {
    for key in [
        "rtx_nr_model",
        "rtx_nr_structure",
        "rtx_nr_character",
        "rtx_diagnostic_logging",
    ] {
        let values = Values::from([(key.into(), "0".into())]);
        rtxmfg::validate_version(Some("1.7.1"), &values)?;
        for version in ["1.6.1", "1.7.0"] {
            assert!(rtxmfg::validate_version(Some(version), &values).is_err());
        }
    }
    assert!(!recommendation_presets::shareable(
        rtxmfg::PROFILE,
        "rtx_diagnostic_logging"
    ));
    for (key, value) in [
        ("rtx_nr_model", "3"),
        ("rtx_nr_structure", "2.1"),
        ("rtx_nr_tone", "NaN"),
        ("rtx_nr_character", "-1.1"),
    ] {
        assert!(!rtxmfg::valid_value(key, value));
    }
    let original = b"[Keybinds]\r\nMenu=8\r\n[Diagnostics]\r\nLogging=0\r\n";
    let values = Values::from([("rtx_diagnostic_logging".into(), "1".into())]);
    let changed = rtxmfg::configure_ui(original, &values)?;
    assert!(String::from_utf8_lossy(&changed).contains("Menu=8"));
    assert_eq!(rtxmfg::read_ui(&changed)?["rtx_diagnostic_logging"], "1");
    assert_eq!(
        rtxmfg::configure(b"{\"keep\":1}", &values)?,
        b"{\"keep\":1}"
    );
    Ok(())
}

#[test]
fn linux_channel_envelope_cannot_masquerade_as_windows_or_another_architecture() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("test.AppImage");
    let mut data = vec![0; 64];
    data[..6].copy_from_slice(b"\x7fELF\x02\x01");
    data[8..11].copy_from_slice(b"AI\x02");
    data[18] = 62;
    std::fs::write(&path, &data)?;
    rtx_fg_manager::updater::verify_appimage(&path)?;
    data[18] = 183;
    std::fs::write(&path, &data)?;
    assert!(rtx_fg_manager::updater::verify_appimage(&path).is_err());
    let manifest = rtx_fg_manager::updater::Manifest {
        schema: 3,
        version: "4.3.32".into(),
        file: "RTXManager-4.3.32-x86_64.AppImage".into(),
        sha256: "a".repeat(64),
        bytes: 1048576,
        source: String::new(),
        url: String::new(),
        notes: Default::default(),
    };
    manifest.validate("4.3.32")?;
    assert!(
        rtx_fg_manager::updater::Manifest {
            schema: 2,
            ..manifest
        }
        .validate("4.3.32")
        .is_err()
    );
    Ok(())
}

#[test]
#[ignore = "requires the two user-signed MFG 1.7.1 artifacts; no game or DLL execution"]
fn signed_standalone_and_existing_host_plugin_restore_without_stale_presets() -> Result<()> {
    use rtx_fg_manager::{cleanup, mfg_host, mfg_plugin};
    use std::{collections::BTreeMap, fs, path::PathBuf};
    let input = PathBuf::from(std::env::var("RTXFG_MFG171_INPUT")?);
    let mfg = fs::read(input.join("MFGUnlocker.dll"))?;
    let opti = fs::read(input.join("OptiScaler.dll"))?;
    for plugin in [false, true] {
        let dir = tempfile::tempdir()?;
        // Minimal PE game identity; it is never executed.
        let mut exe = vec![0u8; 512];
        exe[..2].copy_from_slice(b"MZ");
        exe[60..64].copy_from_slice(&128u32.to_le_bytes());
        exe[128..134].copy_from_slice(b"PE\0\0\x64\x86");
        exe[148..150].copy_from_slice(&240u16.to_le_bytes());
        exe[150..152].copy_from_slice(&2u16.to_le_bytes());
        exe[152..154].copy_from_slice(&0x20bu16.to_le_bytes());
        let game = dir.path().join("synthetic.exe");
        fs::write(&game, exe)?;
        let target = core::DeploymentTarget::for_game(&game);
        let entry = if plugin {
            mfg_plugin::ENTRY
        } else {
            "version.dll"
        };
        let mut original_host = opti.clone();
        original_host.extend_from_slice(b"test original host overlay");
        let original_ini = b"; user settings\r\n[Plugins]\r\nLoadAsiPlugins=false\r\n[FrameGen]\r\nFGOutput=DLSSG\r\n";
        if plugin {
            fs::write(dir.path().join("winmm.dll"), &original_host)?;
            fs::write(dir.path().join("OptiScaler.ini"), original_ini)?;
        }
        let files = BTreeMap::from([
            (entry.into(), mfg.clone()),
            (mfg_host::PAYLOAD.into(), opti.clone()),
            (
                rtxmfg::CONFIG.into(),
                b"{\"followGame\":true,\"mode\":\"follow\",\"multiplier\":2}".to_vec(),
            ),
            (rtxmfg::NOTICES.into(), b"test notice".to_vec()),
        ]);
        core::deploy_prepared_context_at_with_overrides(
            &target,
            rtxmfg::BACKEND,
            &[entry.into()],
            None,
            files.clone(),
            Some("1.7.1"),
            None,
            None,
            Some("1.7.1"),
        )?;
        assert!(!dir.path().join(mfg_host::PAYLOAD).exists());
        if plugin {
            assert_eq!(fs::read(dir.path().join("winmm.dll"))?, opti);
        }
        fs::write(
            dir.path().join("RTXMFG-Dlss5Presets.txt"),
            b"saved test look",
        )?;
        let values = Values::from([("rtx_nr_model".into(), "2".into())]);
        let before = fs::read(dir.path().join(rtxmfg::CONFIG))?;
        let after = rtxmfg::configure(&before, &values)?;
        {
            let _lock = rtx_fg_manager::win::game_lock(dir.path())?;
            rtx_fg_manager::rtxmfg_settings::apply_locked(
                &target,
                Some((&before, &after)),
                &values,
            )?;
        }
        cleanup::clean(&game)?;
        assert!(!dir.path().join(rtxmfg::CONFIG).exists());
        assert!(!dir.path().join("RTXMFG-Dlss5Presets.txt").exists());
        if plugin {
            assert_eq!(fs::read(dir.path().join("winmm.dll"))?, original_host);
            assert_eq!(fs::read(dir.path().join("OptiScaler.ini"))?, original_ini);
        }
        core::deploy_prepared_context_at_with_overrides(
            &target,
            rtxmfg::BACKEND,
            &[entry.into()],
            None,
            files,
            Some("1.7.1"),
            None,
            None,
            Some("1.7.1"),
        )?;
        let fresh = rtxmfg::read(&fs::read(dir.path().join(rtxmfg::CONFIG))?)?;
        assert_eq!(
            fresh.get("rtx_nr_model").map(String::as_str).unwrap_or("0"),
            "0"
        );
        cleanup::clean(&game)?;
        assert!(!dir.path().join(core::OWN).exists());
    }
    Ok(())
}
