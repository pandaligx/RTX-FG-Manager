//! Opt-in local archive regressions. These copy catalog-verified signed release
//! DLL bytes, but never load a DLL or launch the synthetic EXE. They do not test
//! Authenticode trust, a game, a graphics API, or GPU compatibility. No downloads
//! or production cache writes occur: callers explicitly supply both ZIP paths.
use anyhow::{Context, Result, ensure};
use rtx_fg_manager::{
    cleanup, cloud, core, encore, encore_upgrade, jsonc::Document, presets, transfusion,
};
use serde_json::json;
use std::{collections::BTreeMap, fs};

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
        .prefix("rtxfg-encore-local-archive-")
        .tempdir()?;
    let exe = temp.path().join("RTXFG-Never-Run-Signed-Archive-Test.exe");
    fs::write(&exe, synthetic_pe(false))?;
    fs::write(temp.path().join("unrelated-user-file.txt"), b"keep me")?;
    Ok((temp, core::DeploymentTarget::for_game(&exe)))
}

fn context(profile: &str) -> presets::Context {
    presets::Context {
        scheme: encore::SCHEME.into(),
        profile: profile.into(),
        delta: false,
        delta_capable: false,
    }
}

fn local_archive(variable: &str) -> Result<Vec<u8>> {
    let path = std::env::var_os(variable)
        .with_context(|| format!("Set {variable} to an existing local release ZIP"))?;
    let path = std::path::PathBuf::from(path);
    let path = core::no_links(&path)?;
    ensure!(path.is_file(), "{variable} must name a local ZIP file");
    ensure!(
        fs::metadata(&path)?.len() <= 128 * 1024 * 1024,
        "ZIP too large"
    );
    fs::read(&path).with_context(|| format!("Read supplied local ZIP: {}", path.display()))
}

fn encore_archive() -> Result<(cloud::Package, BTreeMap<String, Vec<u8>>)> {
    let catalog = cloud::bundled();
    let package = catalog
        .packages
        .into_iter()
        .find(|p| p.scheme_id == encore::SCHEME && p.backends == [encore::BACKEND])
        .context("Bundled catalog has no Encore package")?;
    let files = cloud::unpack(&package, &local_archive("RTXFG_ENCORE_TEST_ARCHIVE")?)?;
    encore::validate_cloud_config(&files[encore::CONFIG])?;
    Ok((package, files))
}

fn named_encore_files(
    canonical: &BTreeMap<String, Vec<u8>>,
    proxy: &str,
) -> BTreeMap<String, Vec<u8>> {
    let mut files = canonical.clone();
    let dll = files.remove("version.dll").expect("canonical archive DLL");
    files.insert(proxy.into(), dll);
    files
}

fn deploy_encore(
    target: &core::DeploymentTarget,
    package: &cloud::Package,
    files: &BTreeMap<String, Vec<u8>>,
    proxy: &str,
    overrides: &presets::Values,
) -> Result<()> {
    core::deploy_prepared_context_at_with_overrides(
        target,
        encore::BACKEND,
        &[proxy.into()],
        None,
        named_encore_files(files, proxy),
        Some(&package.version),
        Some(&context(encore::PROFILE)),
        Some(overrides),
        package.upstream_version.as_deref(),
    )?;
    Ok(())
}

fn assert_clean(target: &core::DeploymentTarget, proxy: &str) -> Result<()> {
    let result = cleanup::clean_outcome_at(target)?;
    assert!(result.complete, "{}", result.message);
    for name in [proxy, encore::CONFIG, encore::NOTICES, transfusion::CONFIG] {
        assert!(!target.directory.join(name).exists(), "retained {name}");
    }
    assert!(core::record(&target.directory)?.is_none());
    assert_eq!(fs::read(&target.game_exe)?, synthetic_pe(false));
    assert_eq!(
        fs::read(target.directory.join("unrelated-user-file.txt"))?,
        b"keep me"
    );
    Ok(())
}

#[test]
#[ignore = "requires RTXFG_ENCORE_TEST_ARCHIVE; local signed bytes, no DLL execution"]
fn local_signed_encore_deploys_edits_and_cleans_all_nineteen_names() -> Result<()> {
    let (package, files) = encore_archive()?;
    let original_hash = core::hash(&files["version.dll"]);
    for proxy in encore::PROXIES {
        let (_temp, target) = fixture()?;
        let hooked = match proxy {
            "binkw64.dll" => Some("binkw64Hooked.dll"),
            "bink2w64.dll" => Some("bink2w64Hooked.dll"),
            _ => None,
        };
        if let Some(name) = hooked {
            fs::write(target.directory.join(name), synthetic_pe(true))?;
        }
        deploy_encore(&target, &package, &files, proxy, &BTreeMap::new())
            .with_context(|| format!("Deploy signed Encore as {proxy}"))?;
        assert_eq!(core::digest(&target.directory.join(proxy))?, original_hash);
        assert_eq!(
            fs::read(target.directory.join(encore::NOTICES))?,
            files[encore::NOTICES]
        );
        let record = core::record(&target.directory)?.context("Encore ownership record")?;
        assert_eq!(record.backend, encore::BACKEND);
        assert_eq!(record.upstream_version, package.upstream_version);
        assert_eq!(record.hashes.len(), 3);
        assert!(core::status_at(&target).starts_with("已部署"));

        // Simulate an in-game edit after the manager's initial template load.
        let config = target.directory.join(encore::CONFIG);
        let latest = encore::configure(
            &fs::read(&config)?,
            &BTreeMap::from([
                ("tf_mode".into(), "3".into()),
                ("tf_target".into(), "237".into()),
            ]),
        )?;
        let mut latest =
            Document::parse(&latest)?.set(&["userFixture".into(), "keep".into()], json!(true))?;
        latest.extend_from_slice(b"\n// Preserve this local fixture comment.\n");
        fs::write(&config, latest)?;
        core::apply_parameters_at(
            &target,
            &context(encore::PROFILE),
            &BTreeMap::from([
                ("tf_overlay".into(), "1".into()),
                ("hotkeyFixed2".into(), "Ctrl+Alt+2".into()),
            ]),
        )?;
        let changed = fs::read(&config)?;
        let values = encore::read(&changed)?;
        assert_eq!(values["tf_mode"], "3");
        assert_eq!(values["tf_target"], "237");
        assert_eq!(values["tf_overlay"], "1");
        assert_eq!(values["hotkeyFixed2"], "Ctrl+Alt+2");
        assert_eq!(
            Document::parse(&changed)?.value["userFixture"]["keep"],
            true
        );
        assert!(String::from_utf8_lossy(&changed).contains("Preserve this local fixture comment."));
        assert_eq!(core::digest(&target.directory.join(proxy))?, original_hash);
        assert_clean(&target, proxy).with_context(|| format!("Clean signed Encore {proxy}"))?;
        if let Some(name) = hooked {
            assert_eq!(fs::read(target.directory.join(name))?, synthetic_pe(true));
        }
    }
    Ok(())
}

#[test]
#[ignore = "requires RTXFG_ENCORE_TEST_ARCHIVE and RTXFG_TRANSFUSION_TEST_ARCHIVE; no execution"]
fn local_signed_transfusion_upgrades_in_place_and_with_another_encore_entry() -> Result<()> {
    let (package, files) = encore_archive()?;
    let old_zip = local_archive("RTXFG_TRANSFUSION_TEST_ARCHIVE")?;
    let old_hash = core::hash(&old_zip);
    let historical: serde_json::Value = serde_json::from_str(include_str!(
        "../cloud/indexes/payload-index-r-27e17f68f3e68de7a12c.json"
    ))?;
    let old_package: cloud::Package = serde_json::from_value(
        historical["packages"]
            .as_array()
            .context("Historical package array")?
            .iter()
            .find(|p| p["scheme_id"] == encore::SCHEME && p["sha256"] == old_hash)
            .context("Supplied ZIP does not match a published historical Transfusion package")?
            .clone(),
    )?;
    ensure!(
        old_package.backends == [transfusion::BACKEND],
        "Expected Transfusion backend"
    );
    let old_files = cloud::unpack(&old_package, &old_zip)?;
    let alternate = if old_package.proxy == "version.dll" {
        "dinput8.dll"
    } else {
        "version.dll"
    };
    for proxy in [old_package.proxy.as_str(), alternate] {
        let (_temp, target) = fixture()?;
        core::deploy_prepared_context_at(
            &target,
            transfusion::BACKEND,
            std::slice::from_ref(&old_package.proxy),
            None,
            old_files.clone(),
            Some(&old_package.version),
            Some(&context(transfusion::PROFILE)),
        )?;
        let old_config = target.directory.join(transfusion::CONFIG);
        let mut latest = transfusion::configure(
            &fs::read(&old_config)?,
            &BTreeMap::from([
                ("tf_mode".into(), "3".into()),
                ("tf_target".into(), "237".into()),
                ("tf_overlay".into(), "1".into()),
            ]),
        )?;
        latest.extend_from_slice(b"\n// Preserve the old signed installation settings.\n");
        fs::write(&old_config, &latest)?;
        assert!(
            core::apply_parameters_at(&target, &context(encore::PROFILE), &BTreeMap::new())
                .is_err()
        );
        assert_eq!(fs::read(&old_config)?, latest);
        assert_eq!(
            fs::read(target.directory.join(&old_package.proxy))?,
            old_files[&old_package.proxy]
        );
        deploy_encore(
            &target,
            &package,
            &files,
            proxy,
            &BTreeMap::from([("tf_target".into(), "144".into())]),
        )?;
        let migrated = fs::read(target.directory.join(encore::CONFIG))?;
        let values = encore::read(&migrated)?;
        assert_eq!(values["tf_mode"], "3");
        assert_eq!(values["tf_target"], "144");
        assert_eq!(values["tf_overlay"], "1");
        assert_eq!(Document::parse(&migrated)?.value["configVersion"], 4);
        assert!(
            String::from_utf8_lossy(&migrated)
                .contains("Preserve the old signed installation settings.")
        );
        assert_eq!(
            fs::read(target.directory.join(proxy))?,
            files["version.dll"]
        );
        assert_eq!(
            fs::read(target.directory.join(encore::NOTICES))?,
            files[encore::NOTICES]
        );
        assert!(!old_config.exists());
        if proxy != old_package.proxy {
            assert!(!target.directory.join(&old_package.proxy).exists());
        }
        assert!(!encore_upgrade::has_pending(&target.directory)?);
        let record = core::record(&target.directory)?.context("Migrated ownership record")?;
        assert_eq!(record.backend, encore::BACKEND);
        assert_eq!(record.upstream_version, package.upstream_version);
        assert_clean(&target, proxy)?;
    }
    Ok(())
}
