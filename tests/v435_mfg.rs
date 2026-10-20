//! Filesystem/protocol tests only. Synthetic binaries are never executed.
use anyhow::Result;
use base64::{Engine, engine::general_purpose::STANDARD};
use rtx_fg_manager::{cleanup, core, presets, rtxmfg, rtxmfg_settings};
use std::{collections::BTreeMap, fs, os::windows::fs::OpenOptionsExt};

const JSON: &[u8] = b"\xef\xbb\xbf{\r\n\"followGame\":true,\"mode\":\"follow\",\"custom\":7,\"menuShownOnce\":true\r\n}";
const INI: &[u8] = b"; keep comment\r\n[Overlay]\r\nPinnedFps=1\r\n[Compatibility]\r\nOther=keep\r\nGpuNameOverride=0\r\n";
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
fn context() -> presets::Context {
    presets::Context {
        scheme: "rtx40mfg-1.3.3-hf2".into(),
        profile: rtxmfg::PROFILE.into(),
        delta: false,
        delta_capable: false,
    }
}
fn fixture(custom: bool, version: &str) -> Result<(tempfile::TempDir, core::DeploymentTarget)> {
    let temp = tempfile::tempdir()?;
    let exe = temp.path().join("MFG435-Test.exe");
    fs::write(&exe, pe(false))?;
    let dir = if custom {
        temp.path().join("custom")
    } else {
        temp.path().to_owned()
    };
    fs::create_dir_all(&dir)?;
    let target = core::DeploymentTarget::custom(&exe, &dir);
    core::deploy_prepared_context_at_with_overrides(
        &target,
        rtxmfg::BACKEND,
        &["version.dll".into()],
        None,
        BTreeMap::from([
            ("version.dll".into(), pe(true)),
            (rtxmfg::CONFIG.into(), JSON.to_vec()),
            (rtxmfg::NOTICES.into(), b"synthetic notices".to_vec()),
        ]),
        Some(version),
        Some(&context()),
        None,
        Some(version),
    )?;
    Ok((temp, target))
}
fn values() -> presets::Values {
    BTreeMap::from([
        ("rtx_sr_preset".into(), "13".into()),
        ("rtx_rr_preset".into(), "6".into()),
        ("rtx_gpu_override".into(), "1".into()),
    ])
}

#[test]
fn numeric_presets_are_distinct_from_ui_ini_and_preserve_unknown_values() -> Result<()> {
    let edited = rtxmfg::configure(JSON, &values())?;
    let j: serde_json::Value = serde_json::from_slice(&edited)?;
    assert_eq!(j["dlssSrPreset"], 13);
    assert_eq!(j["dlssRrPreset"], 6);
    assert_eq!(j["custom"], 7);
    assert_eq!(j["menuShownOnce"], true);
    assert!(j.get("GpuNameOverride").is_none());
    assert_eq!(rtxmfg::read(&edited)?["rtx_sr_preset"], "13");
    assert!(!rtxmfg::valid_value("rtx_rr_preset", "13"));
    assert!(!rtxmfg::valid_value("rtx_sr_preset", "9"));
    let defaults = presets::defaults(rtxmfg::PROFILE, &BTreeMap::new());
    assert_eq!(defaults["rtx_flip_metering"], "1");
    assert_eq!(defaults["rtx_gpu_override"], "0");
    let ui = rtxmfg::configure_ui(INI, &values())?;
    assert!(String::from_utf8(ui.clone())?.contains("Other=keep\r\n"));
    assert!(String::from_utf8(ui.clone())?.starts_with("; keep comment\r\n"));
    assert_eq!(rtxmfg::read_ui(&ui)?["rtx_gpu_override"], "1");
    Ok(())
}

#[test]
fn applies_both_files_in_correct_locations_and_rejects_old_version() -> Result<()> {
    let (_temp, target) = fixture(true, "1.6.1")?;
    let ui = target.game_exe.parent().unwrap().join(rtxmfg::UI_CONFIG);
    fs::write(&ui, INI)?;
    core::apply_parameters_at(&target, &context(), &values())?;
    assert_eq!(rtxmfg::read_ui(&fs::read(ui)?)?["rtx_gpu_override"], "1");
    assert!(!target.directory.join(rtxmfg::UI_CONFIG).exists());
    assert_eq!(
        rtxmfg::read(&fs::read(target.directory.join(rtxmfg::CONFIG))?)?["rtx_sr_preset"],
        "13"
    );
    let (_old, old) = fixture(false, "1.5.0")?;
    assert!(
        core::apply_parameters_at(&old, &context(), &values())
            .unwrap_err()
            .to_string()
            .contains("1.6.0")
    );
    assert_eq!(fs::read(old.directory.join(rtxmfg::CONFIG))?, JSON);
    assert!(!old.directory.join(rtxmfg::UI_CONFIG).exists());
    Ok(())
}

#[test]
fn locked_ui_rolls_back_json_and_preserves_original_bytes() -> Result<()> {
    let (_temp, target) = fixture(false, "1.6.1")?;
    let ui = target.directory.join(rtxmfg::UI_CONFIG);
    fs::write(&ui, INI)?;
    let _guard = fs::OpenOptions::new().read(true).share_mode(1).open(&ui)?;
    assert!(core::apply_parameters_at(&target, &context(), &values()).is_err());
    assert_eq!(fs::read(target.directory.join(rtxmfg::CONFIG))?, JSON);
    assert_eq!(fs::read(ui)?, INI);
    assert!(
        !target
            .directory
            .join(core::OWN)
            .join("mfg-settings.json")
            .exists()
    );
    Ok(())
}

#[test]
fn upgrade_and_reapply_keep_user_config_and_apply_compatibility_settings() -> Result<()> {
    let (_temp, target) = fixture(true, "1.5.0")?;
    let ui = target.game_exe.parent().unwrap().join(rtxmfg::UI_CONFIG);
    fs::write(&ui, INI)?;
    let mut dll = pe(true);
    dll[511] = 1;
    let files = BTreeMap::from([
        ("version.dll".into(), dll),
        (rtxmfg::CONFIG.into(), br#"{"followGame":true}"#.to_vec()),
        (rtxmfg::NOTICES.into(), b"synthetic notices".to_vec()),
    ]);
    for override_value in ["1", "0"] {
        let mut edits = values();
        edits.insert("rtx_gpu_override".into(), override_value.into());
        core::deploy_prepared_context_at_with_overrides(
            &target,
            rtxmfg::BACKEND,
            &["version.dll".into()],
            None,
            files.clone(),
            Some("1.6.1"),
            Some(&context()),
            Some(&edits),
            Some("1.6.1"),
        )?;
        assert_eq!(
            rtxmfg_settings::read_ui_at(&target)?["rtx_gpu_override"],
            override_value
        );
        let data: serde_json::Value =
            serde_json::from_slice(&fs::read(target.directory.join(rtxmfg::CONFIG))?)?;
        assert_eq!(data["custom"], 7);
        assert_eq!(data["dlssSrPreset"], 13);
        assert_eq!(
            core::record(&target.directory)?
                .unwrap()
                .upstream_version
                .as_deref(),
            Some("1.6.1")
        );
    }
    assert!(String::from_utf8(fs::read(ui)?)?.contains("PinnedFps=1"));
    Ok(())
}

#[test]
fn interrupted_edit_recovers_but_never_overwrites_later_user_edits() -> Result<()> {
    for conflict in [false, true] {
        let (_temp, target) = fixture(false, "1.6.1")?;
        let json = target.directory.join(rtxmfg::CONFIG);
        let after = rtxmfg::configure(JSON, &values())?;
        let ini_after = rtxmfg::configure_ui(INI, &values())?;
        fs::write(target.directory.join(rtxmfg::UI_CONFIG), INI)?;
        fs::write(
            &json,
            if conflict {
                b"{\"later\":true}".as_slice()
            } else {
                &after
            },
        )?;
        let journal = target.directory.join(core::OWN).join("mfg-settings.json");
        core::write_new(
            &journal,
            &serde_json::to_vec(
                &serde_json::json!({"schema":1,"exe":target.game_exe,"directory":target.directory,"edits":[
            {"name":rtxmfg::CONFIG,"before":STANDARD.encode(JSON),"after":STANDARD.encode(&after)},
            {"name":rtxmfg::UI_CONFIG,"before":STANDARD.encode(INI),"after":STANDARD.encode(&ini_after)}]}),
            )?,
        )?;
        let _lock = rtx_fg_manager::win::game_lock(&target.directory)?;
        let result = rtxmfg_settings::recover_locked(&target);
        assert_eq!(result.is_err(), conflict);
        assert_eq!(journal.exists(), conflict);
        assert_eq!(
            fs::read(json)?,
            if conflict {
                b"{\"later\":true}".as_slice()
            } else {
                JSON
            }
        );
    }
    Ok(())
}

#[test]
fn cleanup_covers_game_and_temp_logs_without_other_game_or_unknown_files() -> Result<()> {
    let (_temp, target) = fixture(true, "1.6.1")?;
    let own = rtxmfg::log_paths(&target.game_exe);
    let other = rtxmfg::log_paths(&target.game_exe.with_file_name("OtherGame.exe"));
    for p in own.iter().chain(&other) {
        fs::write(p, b"synthetic log")?;
    }
    let unknown = target
        .game_exe
        .parent()
        .unwrap()
        .join("RTXMFG-manual-capture.csv");
    fs::write(&unknown, b"keep")?;
    let ui = target.game_exe.parent().unwrap().join(rtxmfg::UI_CONFIG);
    fs::write(&ui, INI)?;
    let result = cleanup::clean_outcome_at(&target)?;
    assert!(result.complete, "{}", result.message);
    assert!(own.iter().all(|p| !p.exists()));
    assert!(!ui.exists());
    assert!(other.iter().all(|p| p.exists()));
    assert!(unknown.exists());
    for p in other {
        fs::remove_file(p)?;
    }
    Ok(())
}
