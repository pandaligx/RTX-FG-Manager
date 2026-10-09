//! Synthetic filesystem regressions; no game DLL is loaded or executed.
use anyhow::Result;
use rtx_fg_manager::{cleanup, cloud, core, deployment, presets, rtxmfg, rtxmfg_upgrade, scanner};
use std::{collections::BTreeMap, fs, os::windows::fs::OpenOptionsExt};

const ORIGINAL: &[u8] = b"\xef\xbb\xbf{\r\n  \"followGame\": false, \"mode\": \"dynamic\", \"multiplier\": 5,\r\n  \"dynamicTargetFrameRate\": 237, \"menuHotkey\": 121, \"menuShownOnce\": true, \"custom\": {\"keep\": true}\r\n}\r\n";
fn pe(dll: bool, identity: u8) -> Vec<u8> {
    let mut bytes = vec![0; 512];
    bytes[..2].copy_from_slice(b"MZ");
    bytes[60..64].copy_from_slice(&128u32.to_le_bytes());
    bytes[128..132].copy_from_slice(b"PE\0\0");
    bytes[132..134].copy_from_slice(&0x8664u16.to_le_bytes());
    bytes[148..150].copy_from_slice(&240u16.to_le_bytes());
    bytes[150..152].copy_from_slice(&(if dll { 0x2000u16 } else { 2 }).to_le_bytes());
    bytes[152..154].copy_from_slice(&0x20bu16.to_le_bytes());
    bytes[511] = identity;
    bytes
}
fn context() -> presets::Context {
    presets::Context {
        scheme: rtxmfg_upgrade::SCHEME.into(),
        profile: rtxmfg::PROFILE.into(),
        delta: false,
        delta_capable: false,
    }
}
fn fixture(custom: bool, proxy: &str) -> Result<(tempfile::TempDir, core::DeploymentTarget)> {
    let temp = tempfile::tempdir()?;
    let exe = temp.path().join("RTXFG-MFG-Upgrade-Test.exe");
    fs::write(&exe, pe(false, 0))?;
    let target = if custom {
        let dir = temp.path().join("独立插件");
        fs::create_dir(&dir)?;
        core::DeploymentTarget::custom(&exe, &dir)
    } else {
        core::DeploymentTarget::for_game(&exe)
    };
    if proxy == "binkw64.dll" {
        fs::write(target.directory.join("binkw64Hooked.dll"), pe(true, 99))?;
    }
    install(&target, proxy, "1.4.1", 1, None)?;
    // A later menu edit legitimately differs from the old installation digest.
    fs::write(target.directory.join(rtxmfg::CONFIG), ORIGINAL)?;
    Ok((temp, target))
}
fn install(
    target: &core::DeploymentTarget,
    proxy: &str,
    version: &str,
    identity: u8,
    values: Option<&presets::Values>,
) -> Result<String> {
    core::deploy_prepared_context_at_with_overrides(
        target,
        rtxmfg::BACKEND,
        &[proxy.into()],
        None,
        BTreeMap::from([
            (proxy.into(), pe(true, identity)),
            (
                rtxmfg::CONFIG.into(),
                br#"{"followGame":true,"mode":"follow","multiplier":2,"dynamicTargetFrameRate":0}"#
                    .to_vec(),
            ),
        ]),
        Some(version),
        Some(&context()),
        values,
        None,
    )
}
fn snapshot(target: &core::DeploymentTarget) -> Result<BTreeMap<String, Vec<u8>>> {
    let mut result = BTreeMap::new();
    for name in [
        "version.dll",
        "dxgi.dll",
        rtxmfg::CONFIG,
        ".rtx-fg-v3/install.json",
        "binkw64.dll",
        "binkw64Hooked.dll",
    ] {
        let path = target.directory.join(name);
        if path.is_file() {
            result.insert(name.into(), fs::read(path)?);
        }
    }
    Ok(result)
}

#[test]
fn rtxmfg_upgrade_retains_config_bytes_and_actual_version_until_dll_commit() -> Result<()> {
    let (_temp, target) = fixture(true, "version.dll")?;
    let mut catalog = cloud::bundled();
    catalog
        .packages
        .iter_mut()
        .filter(|p| p.scheme_id == rtxmfg_upgrade::SCHEME)
        .for_each(|p| {
            p.version = "1.4.2".into();
            p.label = "RTX40 MFG · 1.4.2".into();
        });
    let game = scanner::Game {
        exe: target.game_exe.display().to_string(),
        ..Default::default()
    };
    // Normal-target inspection below exercises the public actual-version label.
    let (_other, normal) = fixture(false, "version.dll")?;
    let normal_game = scanner::Game {
        exe: normal.game_exe.display().to_string(),
        ..game.clone()
    };
    let details = deployment::inspect(&normal_game, &catalog)
        .details
        .join("\n");
    assert!(details.contains("1.4.1") && !details.contains("1.4.2"));
    // Share-read permits reads but rejects config writes/removal. A no-edit
    // version upgrade must leave the original JSON physically in place.
    let _config_guard = fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(target.directory.join(rtxmfg::CONFIG))?;
    core::preflight_install_at(&target, &["version.dll".into()])?;
    install(&target, "version.dll", "1.4.2", 2, None)?;
    assert_eq!(fs::read(target.directory.join(rtxmfg::CONFIG))?, ORIGINAL);
    assert_eq!(fs::read(target.directory.join("version.dll"))?, pe(true, 2));
    let record = core::record(&target.directory)?.unwrap();
    assert_eq!(record.payload_version.as_deref(), Some("1.4.2"));
    assert_eq!(record.game_exe, target.record_exe());
    assert_eq!(record.hashes[rtxmfg::CONFIG], core::hash(ORIGINAL));
    install(
        &normal,
        "version.dll",
        "1.4.2",
        2,
        Some(&presets::Values::new()),
    )?;
    assert!(deployment::inspect(&normal_game, &catalog).details[0].contains("1.4.2"));
    assert_eq!(fs::read(normal.directory.join(rtxmfg::CONFIG))?, ORIGINAL);
    Ok(())
}

#[test]
fn rtxmfg_updates_apply_only_explicit_fields_over_latest_disk_settings() -> Result<()> {
    let (_temp, target) = fixture(false, "version.dll")?;
    for (version, identity) in [("1.4.2", 2), ("1.4.2", 2)] {
        fs::write(target.directory.join(rtxmfg::CONFIG), ORIGINAL)?;
        install(
            &target,
            "version.dll",
            version,
            identity,
            Some(&presets::Values::from([("rtx_vsync".into(), "1".into())])),
        )?;
        let updated: serde_json::Value =
            serde_json::from_slice(&fs::read(target.directory.join(rtxmfg::CONFIG))?)?;
        assert_eq!(updated["vsyncMode"], 1);
        assert_eq!(updated["dynamicTargetFrameRate"], 237);
        assert_eq!(updated["mode"], "dynamic");
        assert_eq!(updated["multiplier"], 5);
        assert_eq!(updated["menuHotkey"], 121);
        assert_eq!(updated["menuShownOnce"], true);
        assert_eq!(updated["custom"]["keep"], true);
    }
    fs::write(target.directory.join(rtxmfg::CONFIG), ORIGINAL)?;
    install(&target, "version.dll", "1.4.2", 2, None)?;
    assert_eq!(fs::read(target.directory.join(rtxmfg::CONFIG))?, ORIGINAL);
    Ok(())
}

#[test]
fn rtxmfg_upgrade_requires_owned_matching_scheme_same_entry_and_newer_version() -> Result<()> {
    for case in [
        "other-scheme",
        "missing-version",
        "changed-dll",
        "same-version",
        "downgrade",
        "new-entry",
        "bad-overrides",
    ] {
        let (_temp, target) = fixture(false, "version.dll")?;
        let marker = target.directory.join(core::OWN).join(core::MARKER);
        let mut record = core::record(&target.directory)?.unwrap();
        if case == "other-scheme" {
            record.scheme_id = Some("another-scheme".into());
        }
        if case == "missing-version" {
            record.payload_version = None;
        }
        core::atomic_json(&marker, &record)?;
        if case == "changed-dll" {
            fs::write(target.directory.join("version.dll"), pe(true, 99))?;
        }
        let before = snapshot(&target)?;
        let version = match case {
            "same-version" => "1.4.1",
            "downgrade" => "1.3.3",
            _ => "1.4.2",
        };
        let proxy = if case == "new-entry" {
            "dxgi.dll"
        } else {
            "version.dll"
        };
        let values = if case == "bad-overrides" {
            presets::Values::from([("rtx_mode".into(), "invalid".into())])
        } else {
            presets::Values::new()
        };
        assert!(
            install(&target, proxy, version, 2, Some(&values)).is_err(),
            "{case}"
        );
        assert_eq!(snapshot(&target)?, before, "{case}");
        assert!(!rtxmfg_upgrade::has_pending(&target.directory)?);
    }
    Ok(())
}

#[test]
fn rtxmfg_bink_upgrade_requires_and_preserves_original_and_unrelated_files() -> Result<()> {
    let (_temp, target) = fixture(false, "binkw64.dll")?;
    let original = target.directory.join("binkw64Hooked.dll");
    fs::remove_file(&original)?;
    let before = snapshot(&target)?;
    assert!(install(&target, "binkw64.dll", "1.4.2", 2, None).is_err());
    assert_eq!(snapshot(&target)?, before);
    fs::write(&original, pe(true, 99))?;
    let unrelated = target.directory.join(core::INI);
    fs::write(&unrelated, b"other mod configuration")?;
    install(&target, "binkw64.dll", "1.4.2", 2, None)?;
    cleanup::clean_at(&target)?;
    assert_eq!(fs::read(&original)?, pe(true, 99));
    assert_eq!(fs::read(&unrelated)?, b"other mod configuration");
    assert!(!target.directory.join("binkw64.dll").exists());
    assert!(!target.directory.join(rtxmfg::CONFIG).exists());
    Ok(())
}
