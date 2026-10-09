//! Synthetic filesystem/configuration tests. No game DLL is loaded or GPU used.
use anyhow::Result;
use rtx_fg_manager::{
    cleanup, cloud, core, deployment, encore, encore_upgrade, presets, scanner, transfusion,
};
use std::{collections::BTreeMap, fs};

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

fn context(profile: &str) -> presets::Context {
    presets::Context {
        scheme: encore_upgrade::SCHEME.into(),
        profile: profile.into(),
        delta: false,
        delta_capable: false,
    }
}

fn fixture(custom: bool) -> Result<(tempfile::TempDir, core::DeploymentTarget)> {
    let temp = tempfile::tempdir()?;
    let exe = temp.path().join("RTXFG-Encore-Transaction-Test.exe");
    fs::write(&exe, pe(false, 0))?;
    let target = if custom {
        let folder = temp.path().join("独立插件目录");
        fs::create_dir(&folder)?;
        core::DeploymentTarget::custom(&exe, &folder)
    } else {
        core::DeploymentTarget::for_game(&exe)
    };
    Ok((temp, target))
}

fn old_install(target: &core::DeploymentTarget) -> Result<()> {
    core::deploy_prepared_context_at(
        target, transfusion::BACKEND, &["version.dll".into()], None,
        BTreeMap::from([
            ("version.dll".into(), pe(true, 1)),
            (transfusion::CONFIG.into(), b"\xef\xbb\xbf{\r\n// keep my settings\r\n\"configVersion\":3,\"frameGeneration\":{\"mode\":\"fixed\",\"multiplier\":3,\"dynamicTargetFrameRate\":237},\"overlay\":{\"showOverlay\":true},\"menuState\":{\"menuKey\":123},\"vendor\":{\"keep\":true}}".to_vec()),
        ]), Some("1.4.5"), Some(&context(transfusion::PROFILE)),
    )?;
    Ok(())
}

fn encore_files(proxy: &str) -> BTreeMap<String, Vec<u8>> {
    BTreeMap::from([
        (proxy.into(), pe(true, 2)),
        (encore::CONFIG.into(), br#"{"configVersion":4,"frameGeneration":{"mode":"game","multiplier":4},"overlay":{"showOverlay":false}}"#.to_vec()),
        (encore::NOTICES.into(), b"RTX Encore third-party license notices fixture\n".to_vec()),
    ])
}

fn upgrade(
    target: &core::DeploymentTarget,
    proxy: &str,
    overrides: &presets::Values,
) -> Result<String> {
    core::deploy_prepared_context_at_with_overrides(
        target,
        encore::BACKEND,
        &[proxy.into()],
        None,
        encore_files(proxy),
        Some("1.0.0"),
        Some(&context(encore::PROFILE)),
        Some(overrides),
        Some("1.0.0-beta.2"),
    )
}

#[test]
fn migration_keeps_disk_settings_and_comments_and_accepts_only_explicit_overrides() -> Result<()> {
    let (_temp, target) = fixture(true)?;
    old_install(&target)?;
    let old_config = target.directory.join(transfusion::CONFIG);
    // A game-side edit after installation is legitimate, even though the
    // installation record still has the first configuration digest.
    let edited = String::from_utf8(fs::read(&old_config)?[3..].to_vec())?.replace("237", "165");
    fs::write(&old_config, edited.as_bytes())?;
    fs::write(
        target.directory.join("other-plugin.dll"),
        b"keep unrelated mod",
    )?;
    upgrade(
        &target,
        "dinput8.dll",
        &BTreeMap::from([("tf_target".into(), "144".into())]),
    )?;
    let text = fs::read_to_string(target.directory.join(encore::CONFIG))?;
    assert!(text.contains("// keep my settings\r\n"));
    assert!(text.contains("\"vendor\":{\"keep\":true}"));
    assert!(text.contains("\"menuKey\":123"));
    let values = encore::read(text.as_bytes())?;
    assert_eq!(values["tf_mode"], "3");
    assert_eq!(values["tf_target"], "144");
    assert_eq!(values["tf_overlay"], "1");
    assert!(!old_config.exists());
    assert!(!target.directory.join("version.dll").exists());
    assert_eq!(fs::read(target.directory.join("dinput8.dll"))?, pe(true, 2));
    assert_eq!(
        fs::read(target.directory.join("other-plugin.dll"))?,
        b"keep unrelated mod"
    );
    let record = core::record(&target.directory)?.unwrap();
    assert_eq!(record.backend, encore::BACKEND);
    assert_eq!(record.upstream_version.as_deref(), Some("1.0.0-beta.2"));
    assert_eq!(record.game_exe.as_deref(), target.game_exe.to_str());
    assert_eq!(record.hashes.len(), 3);
    assert!(!encore_upgrade::has_pending(&target.directory)?);
    assert!(core::status_at(&target).contains("1.0.0-beta.2"));
    Ok(())
}

#[test]
fn stable_scheme_id_does_not_apply_new_protocol_or_mislabel_old_dll() -> Result<()> {
    let (_temp, target) = fixture(false)?;
    old_install(&target)?;
    let before = fs::read(target.directory.join(transfusion::CONFIG))?;
    assert!(
        core::apply_parameters_at(&target, &context(encore::PROFILE), &BTreeMap::new()).is_err()
    );
    assert_eq!(
        fs::read(target.directory.join(transfusion::CONFIG))?,
        before
    );
    let mut catalog = cloud::bundled();
    catalog
        .scheme_policies
        .get_mut(encore_upgrade::SCHEME)
        .unwrap()
        .parameter_profile = encore::PROFILE.into();
    for p in &mut catalog.packages {
        if p.scheme_id == encore_upgrade::SCHEME {
            p.label = "RTX Encore · future release".into();
        }
    }
    let game = scanner::Game {
        exe: target.game_exe.display().to_string(),
        ..Default::default()
    };
    let snapshot = deployment::inspect(&game, &catalog);
    assert!(snapshot.details[0].contains("DLSSG-Transfusion · 1.4.5.3"));
    assert!(!snapshot.details[0].contains("future release"));
    assert!(!snapshot.can_apply(encore_upgrade::SCHEME));
    upgrade(&target, "version.dll", &BTreeMap::new())?;
    let snapshot = deployment::inspect(&game, &catalog);
    assert!(snapshot.can_apply(encore_upgrade::SCHEME));
    assert!(snapshot.details[0].contains("1.0.0-beta.2"));
    Ok(())
}

#[test]
fn repeated_encore_install_uses_latest_disk_document_and_does_not_replace_dll() -> Result<()> {
    let (_temp, target) = fixture(false)?;
    upgrade(&target, "version.dll", &BTreeMap::new())?;
    let config = target.directory.join(encore::CONFIG);
    let updated = encore::configure(
        &fs::read(&config)?,
        &BTreeMap::from([
            ("tf_mode".into(), "3".into()),
            ("tf_target".into(), "237".into()),
        ]),
    )?;
    fs::write(&config, &updated)?;
    upgrade(&target, "version.dll", &BTreeMap::new())?;
    assert_eq!(fs::read(&config)?, updated);
    upgrade(
        &target,
        "version.dll",
        &BTreeMap::from([("tf_overlay".into(), "1".into())]),
    )?;
    let values = encore::read(&fs::read(&config)?)?;
    assert_eq!(values["tf_mode"], "3");
    assert_eq!(values["tf_target"], "237");
    assert_eq!(values["tf_overlay"], "1");
    let mut different = encore_files("version.dll");
    different.insert("version.dll".into(), pe(true, 3));
    assert!(
        core::deploy_prepared_context_at_with_overrides(
            &target,
            encore::BACKEND,
            &["version.dll".into()],
            None,
            different,
            Some("1.0.0"),
            Some(&context(encore::PROFILE)),
            None,
            Some("1.0.0-beta.3"),
        )
        .is_err()
    );
    assert_eq!(fs::read(target.directory.join("version.dll"))?, pe(true, 2));
    Ok(())
}

#[test]
fn all_nineteen_entries_deploy_same_bytes_and_bink_originals_survive_removal() -> Result<()> {
    for proxy in encore::PROXIES {
        let (_temp, target) = fixture(false)?;
        let hooked = match proxy {
            "binkw64.dll" => Some("binkw64Hooked.dll"),
            "bink2w64.dll" => Some("bink2w64Hooked.dll"),
            _ => None,
        };
        if let Some(name) = hooked {
            assert!(upgrade(&target, proxy, &BTreeMap::new()).is_err());
            fs::write(target.directory.join(name), pe(true, 7))?;
        }
        upgrade(&target, proxy, &BTreeMap::new())?;
        assert_eq!(fs::read(target.directory.join(proxy))?, pe(true, 2));
        assert!(target.directory.join(encore::NOTICES).is_file());
        cleanup::clean_outcome_at(&target)?;
        assert!(!target.directory.join(proxy).exists());
        assert!(!target.directory.join(encore::CONFIG).exists());
        assert!(!target.directory.join(encore::NOTICES).exists());
        if let Some(name) = hooked {
            assert_eq!(fs::read(target.directory.join(name))?, pe(true, 7));
        }
    }
    assert!(core::deployment_names(encore::BACKEND, &["rtx-encore.asi".into()]).is_err());
    assert!(
        core::deployment_names(encore::BACKEND, &["version.dll".into(), "dxgi.dll".into()])
            .is_err()
    );
    Ok(())
}

#[test]
fn unknown_files_and_changed_old_dll_block_upgrade_without_touching_originals() -> Result<()> {
    for collision in [encore::CONFIG, encore::NOTICES, "dinput8.dll"] {
        let (_temp, target) = fixture(false)?;
        old_install(&target)?;
        let marker = target.directory.join(core::OWN).join(core::MARKER);
        let before = fs::read(&marker)?;
        fs::write(target.directory.join(collision), b"unrelated user file")?;
        assert!(upgrade(&target, "dinput8.dll", &BTreeMap::new()).is_err());
        assert_eq!(fs::read(&marker)?, before);
        assert_eq!(fs::read(target.directory.join("version.dll"))?, pe(true, 1));
        assert_eq!(
            fs::read(target.directory.join(collision))?,
            b"unrelated user file"
        );
        assert!(!encore_upgrade::has_pending(&target.directory)?);
    }
    let (_temp, target) = fixture(false)?;
    old_install(&target)?;
    fs::write(target.directory.join("version.dll"), pe(true, 8))?;
    assert!(upgrade(&target, "version.dll", &BTreeMap::new()).is_err());
    assert_eq!(fs::read(target.directory.join("version.dll"))?, pe(true, 8));
    Ok(())
}

#[test]
fn cleanup_retains_modified_notice_and_unrecognized_files_inside_logs() -> Result<()> {
    let (_temp, target) = fixture(false)?;
    upgrade(&target, "version.dll", &BTreeMap::new())?;
    fs::write(
        target.directory.join(encore::NOTICES),
        b"user edited notices",
    )?;
    let logs = target.directory.join("rtx-encore-logs");
    fs::create_dir(&logs)?;
    fs::write(logs.join("rtx-encore.log"), b"owned session")?;
    fs::write(logs.join("rtx-encore_perf.csv"), b"owned metrics")?;
    let rotated = "rtx-encore.log.previous-20261009-015707-625-12888.log";
    fs::write(logs.join(rotated), b"owned rotated session")?;
    fs::write(
        logs.join("rtx-encore.log.previous-user-notes.log"),
        b"user notes",
    )?;
    fs::write(logs.join("unrelated.log"), b"user log")?;
    fs::create_dir(logs.join("user-data"))?;
    fs::write(
        logs.join("user-data/rtx-encore.log"),
        b"not owned nested file",
    )?;
    let outcome = cleanup::clean_outcome_at(&target)?;
    assert!(outcome.complete);
    assert!(!target.directory.join("version.dll").exists());
    assert!(!logs.join("rtx-encore.log").exists());
    assert!(!logs.join("rtx-encore_perf.csv").exists());
    assert!(!logs.join(rotated).exists());
    assert_eq!(
        fs::read(logs.join("rtx-encore.log.previous-user-notes.log"))?,
        b"user notes"
    );
    assert_eq!(fs::read(logs.join("unrelated.log"))?, b"user log");
    assert_eq!(
        fs::read(logs.join("user-data/rtx-encore.log"))?,
        b"not owned nested file"
    );
    assert_eq!(
        fs::read(target.directory.join(encore::NOTICES))?,
        b"user edited notices"
    );
    Ok(())
}

#[test]
fn old_record_without_full_version_remains_readable() -> Result<()> {
    let (_temp, target) = fixture(false)?;
    old_install(&target)?;
    let marker = target.directory.join(core::OWN).join(core::MARKER);
    let text = fs::read_to_string(&marker)?;
    assert!(!text.contains("upstream_version"));
    let record = core::record(&target.directory)?.unwrap();
    assert!(record.upstream_version.is_none());
    record.validate()?;
    Ok(())
}
