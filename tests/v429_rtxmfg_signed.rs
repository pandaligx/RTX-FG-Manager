//! Opt-in catalog-verified signed archive regressions. Real DLL bytes are copied
//! but never loaded, and the synthetic game EXE is never launched. This does
//! not test Authenticode trust, a game, a graphics API or GPU compatibility.
//! No downloads or production-cache writes occur; callers supply both ZIPs.
use anyhow::{Context, Result, ensure};
use rtx_fg_manager::{cleanup, cloud, core, deployment, presets, rtxmfg, rtxmfg_upgrade, scanner};
use serde_json::json;
use std::{collections::BTreeMap, fs, os::windows::fs::OpenOptionsExt};

fn synthetic_pe(dll: bool) -> Vec<u8> {
    let mut bytes = vec![0; 512];
    bytes[..2].copy_from_slice(b"MZ");
    bytes[60..64].copy_from_slice(&128u32.to_le_bytes());
    bytes[128..132].copy_from_slice(b"PE\0\0");
    bytes[132..134].copy_from_slice(&0x8664u16.to_le_bytes());
    bytes[148..150].copy_from_slice(&240u16.to_le_bytes());
    bytes[150..152].copy_from_slice(&(if dll { 0x2000u16 } else { 2 }).to_le_bytes());
    bytes[152..154].copy_from_slice(&0x20bu16.to_le_bytes());
    bytes
}

fn fixture() -> Result<(tempfile::TempDir, core::DeploymentTarget)> {
    let temp = tempfile::Builder::new()
        .prefix("rtxfg-mfg-signed-archive-")
        .tempdir()?;
    let exe = temp.path().join("RTXFG-Never-Run-MFG-Archive-Test.exe");
    fs::write(&exe, synthetic_pe(false))?;
    fs::write(temp.path().join("unrelated-user-file.txt"), b"keep me")?;
    fs::write(temp.path().join(core::INI), b"other mod INI")?;
    Ok((temp, core::DeploymentTarget::for_game(&exe)))
}

fn local_archive(variable: &str) -> Result<Vec<u8>> {
    let path = std::env::var_os(variable)
        .with_context(|| format!("Set {variable} to an existing local signed release ZIP"))?;
    let path = core::no_links(&std::path::PathBuf::from(path))?;
    ensure!(path.is_file(), "{variable} must name a local ZIP file");
    ensure!(path.metadata()?.len() <= 128 * 1024 * 1024, "ZIP too large");
    fs::read(&path).with_context(|| format!("Read supplied archive: {}", path.display()))
}

struct Archives {
    old: cloud::Package,
    old_files: BTreeMap<String, Vec<u8>>,
    new: cloud::Package,
    new_files: BTreeMap<String, Vec<u8>>,
}
fn archives() -> Result<Archives> {
    let old_zip = local_archive("RTXFG_MFG_OLD_TEST_ARCHIVE")?;
    let old_hash = core::hash(&old_zip);
    let historical: serde_json::Value = serde_json::from_str(include_str!(
        "../cloud/indexes/payload-index-r-27e17f68f3e68de7a12c.json"
    ))?;
    let old: cloud::Package = serde_json::from_value(
        historical["packages"]
            .as_array()
            .context("Historical package array")?
            .iter()
            .find(|p| p["scheme_id"] == rtxmfg_upgrade::SCHEME && p["sha256"] == old_hash)
            .context("Supplied old ZIP does not match the published RTX40 MFG package")?
            .clone(),
    )?;
    ensure!(
        old.version == "1.4.1" && old.backends == [rtxmfg::BACKEND],
        "Expected RTX40 MFG 1.4.1 Hotfix 1"
    );
    let old_files = cloud::unpack(&old, &old_zip)?;
    let new = cloud::bundled()
        .packages
        .into_iter()
        .find(|p| p.scheme_id == rtxmfg_upgrade::SCHEME && p.backends == [rtxmfg::BACKEND])
        .context("Bundled catalog has no RTX40 MFG package")?;
    ensure!(new.version == "1.4.2", "Expected RTX40 MFG 1.4.2 candidate");
    let new_files = cloud::unpack(&new, &local_archive("RTXFG_MFG_TEST_ARCHIVE")?)?;
    ensure!(
        core::hash(&old_files[&old.proxy]) != core::hash(&new_files[&new.proxy]),
        "Old and new DLLs must differ"
    );
    Ok(Archives {
        old,
        old_files,
        new,
        new_files,
    })
}

fn context() -> presets::Context {
    presets::Context {
        scheme: rtxmfg_upgrade::SCHEME.into(),
        profile: rtxmfg::PROFILE.into(),
        delta: false,
        delta_capable: false,
    }
}

fn deploy(
    target: &core::DeploymentTarget,
    package: &cloud::Package,
    canonical: &BTreeMap<String, Vec<u8>>,
    proxy: &str,
    values: Option<&presets::Values>,
) -> Result<()> {
    let mut files = canonical.clone();
    let dll = files.remove(&package.proxy).context("Canonical DLL")?;
    files.insert(proxy.into(), dll);
    core::deploy_prepared_context_at_with_overrides(
        target,
        rtxmfg::BACKEND,
        &[proxy.into()],
        None,
        files,
        Some(&package.version),
        Some(&context()),
        values,
        package.upstream_version.as_deref(),
    )?;
    Ok(())
}

fn custom_config(template: &[u8]) -> Result<Vec<u8>> {
    let mut value: serde_json::Value =
        serde_json::from_slice(template.strip_prefix(&[239, 187, 191]).unwrap_or(template))?;
    value["followGame"] = json!(false);
    value["mode"] = json!("dynamic");
    value["multiplier"] = json!(5);
    value["dynamicTargetFrameRate"] = json!(237);
    value["menuHotkey"] = json!(121);
    value["menuShownOnce"] = json!(true);
    value["userFixture"] = json!({"keep":true,"nested":[1,2,3]});
    let text = format!(
        "{}\r\n",
        serde_json::to_string_pretty(&value)?.replace('\n', "\r\n")
    );
    let mut bytes = vec![239, 187, 191];
    bytes.extend_from_slice(text.as_bytes());
    Ok(bytes)
}

fn assert_clean(target: &core::DeploymentTarget, proxy: &str) -> Result<()> {
    let outcome = cleanup::clean_outcome_at(target)?;
    assert!(outcome.complete, "{}", outcome.message);
    assert!(!target.directory.join(proxy).exists());
    assert!(!target.directory.join(rtxmfg::CONFIG).exists());
    assert!(core::record(&target.directory)?.is_none());
    assert!(!rtxmfg_upgrade::has_pending(&target.directory)?);
    assert_eq!(fs::read(&target.game_exe)?, synthetic_pe(false));
    assert_eq!(
        fs::read(target.directory.join("unrelated-user-file.txt"))?,
        b"keep me"
    );
    assert_eq!(
        fs::read(target.directory.join(core::INI))?,
        b"other mod INI"
    );
    Ok(())
}

#[test]
#[ignore = "requires RTXFG_MFG_OLD_TEST_ARCHIVE and RTXFG_MFG_TEST_ARCHIVE; no DLL execution"]
fn local_signed_mfg_141_upgrades_to_142_in_all_nineteen_existing_entries() -> Result<()> {
    let archives = archives()?;
    let catalog = cloud::bundled();
    let old_dll = &archives.old_files[&archives.old.proxy];
    let new_dll = &archives.new_files[&archives.new.proxy];
    let latest = custom_config(&archives.old_files[rtxmfg::CONFIG])?;
    for proxy in rtxmfg::PROXIES {
        let (_temp, target) = fixture()?;
        let hooked = match proxy {
            "binkw64.dll" => Some("binkw64Hooked.dll"),
            "bink2w64.dll" => Some("bink2w64Hooked.dll"),
            _ => None,
        };
        if let Some(name) = hooked {
            fs::write(target.directory.join(name), synthetic_pe(true))?;
        }
        deploy(&target, &archives.old, &archives.old_files, proxy, None)?;
        assert_eq!(fs::read(target.directory.join(proxy))?, *old_dll);
        let config = target.directory.join(rtxmfg::CONFIG);
        fs::write(&config, &latest)?;
        let game = scanner::Game {
            exe: target.game_exe.display().to_string(),
            ..Default::default()
        };
        let before = deployment::inspect(&game, &catalog).details.join("\n");
        assert!(
            before.contains("1.4.1") && !before.contains("1.4.2"),
            "{proxy}: {before}"
        );
        {
            let _no_config_writes = fs::OpenOptions::new()
                .read(true)
                .share_mode(1)
                .open(&config)?;
            core::preflight_install_at(&target, &[proxy.into()])?;
            deploy(
                &target,
                &archives.new,
                &archives.new_files,
                proxy,
                Some(&presets::Values::new()),
            )
            .with_context(|| format!("Upgrade signed RTX40 MFG at {proxy}"))?;
        }
        assert_eq!(
            fs::read(&config)?,
            latest,
            "All config bytes must survive for {proxy}"
        );
        assert_eq!(fs::read(target.directory.join(proxy))?, *new_dll);
        let record = core::record(&target.directory)?.context("Upgraded ownership record")?;
        assert_eq!(record.backend, rtxmfg::BACKEND);
        assert_eq!(record.scheme_id.as_deref(), Some(rtxmfg_upgrade::SCHEME));
        assert_eq!(record.payload_version.as_deref(), Some("1.4.2"));
        assert_eq!(record.selected(), vec![proxy.to_owned()]);
        assert_eq!(record.hashes[rtxmfg::CONFIG], core::hash(&latest));
        assert_eq!(record.hashes[proxy], core::hash(new_dll));
        assert!(!rtxmfg_upgrade::has_pending(&target.directory)?);
        let after = deployment::inspect(&game, &catalog).details.join("\n");
        assert!(after.contains("1.4.2") && after.contains(proxy));
        assert_clean(&target, proxy)?;
        if let Some(name) = hooked {
            assert_eq!(fs::read(target.directory.join(name))?, synthetic_pe(true));
        }
    }
    Ok(())
}

#[test]
#[ignore = "requires RTXFG_MFG_OLD_TEST_ARCHIVE and RTXFG_MFG_TEST_ARCHIVE; no DLL execution"]
fn local_signed_mfg_upgrade_writes_only_explicit_parameters_then_retains_new_menu_edits()
-> Result<()> {
    let archives = archives()?;
    let (_temp, target) = fixture()?;
    deploy(
        &target,
        &archives.old,
        &archives.old_files,
        "version.dll",
        None,
    )?;
    let config = target.directory.join(rtxmfg::CONFIG);
    fs::write(&config, custom_config(&archives.old_files[rtxmfg::CONFIG])?)?;
    deploy(
        &target,
        &archives.new,
        &archives.new_files,
        "version.dll",
        Some(&presets::Values::from([("rtx_vsync".into(), "1".into())])),
    )?;
    let after: serde_json::Value = serde_json::from_slice(&fs::read(&config)?)?;
    assert_eq!(after["vsyncMode"], 1);
    assert_eq!(after["mode"], "dynamic");
    assert_eq!(after["multiplier"], 5);
    assert_eq!(after["dynamicTargetFrameRate"], 237);
    assert_eq!(after["menuHotkey"], 121);
    assert_eq!(after["menuShownOnce"], true);
    assert_eq!(after["userFixture"], json!({"keep":true,"nested":[1,2,3]}));
    let latest = custom_config(&fs::read(&config)?)?;
    fs::write(&config, &latest)?;
    deploy(
        &target,
        &archives.new,
        &archives.new_files,
        "version.dll",
        None,
    )?;
    assert_eq!(fs::read(&config)?, latest);
    assert_eq!(
        fs::read(target.directory.join("version.dll"))?,
        archives.new_files[&archives.new.proxy]
    );
    assert_clean(&target, "version.dll")?;
    Ok(())
}
