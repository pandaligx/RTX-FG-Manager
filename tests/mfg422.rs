use anyhow::Result;
#[cfg(feature = "fixture-tests")]
use rtx_fg_manager::{cleanup, core, diagnostics};
use rtx_fg_manager::{cloud, i18n, presets};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

const SCHEME: &str = "pipotoufik-mfg-vulkan-sm86-7";
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}
fn context(exe: &Path) -> presets::Context {
    let c = cloud::bundled();
    presets::Context::new(SCHEME, &c.scheme_policies[SCHEME], exe)
}
#[cfg(feature = "fixture-tests")]
fn package() -> Result<BTreeMap<String, Vec<u8>>> {
    let c = cloud::bundled();
    let p = c.packages.iter().find(|p| p.scheme_id == SCHEME).unwrap();
    cloud::unpack(
        p,
        &fs::read(
            root()
                .join("tests/fixtures/runtime/packages")
                .join(&p.archive),
        )?,
    )
}
#[cfg(feature = "fixture-tests")]
fn game(dir: &Path) -> Result<PathBuf> {
    let exe = dir.join("测试游戏.exe");
    let mut b = vec![0; 1024];
    b[..2].copy_from_slice(b"MZ");
    b[60..64].copy_from_slice(&128u32.to_le_bytes());
    b[128..132].copy_from_slice(b"PE\0\0");
    b[132..134].copy_from_slice(&0x8664u16.to_le_bytes());
    b[148..150].copy_from_slice(&240u16.to_le_bytes());
    b[150..152].copy_from_slice(&2u16.to_le_bytes());
    b[152..154].copy_from_slice(&0x20bu16.to_le_bytes());
    fs::write(&exe, b)?;
    Ok(exe)
}
#[test]
fn new_catalog_retains_ids_and_default_and_limits_new_proxy() -> Result<()> {
    let folder = root().join("tests/fixtures/catalog422");
    let compact: cloud::CompactCatalog =
        serde_json::from_slice(&fs::read(folder.join("catalog.json"))?)?;
    let c = compact.expand(&fs::read(folder.join("index.json"))?)?;
    assert_eq!(c.packages.len(), 20);
    assert_eq!(c.schemes().len(), 5);
    assert_eq!(c.default_scheme, "upstream-0.3.5-310-9");
    assert_eq!(c.proxies(SCHEME), ["version.dll"]);
    assert_eq!(c.proxies("rtxfg-0.3.5-dx12-vulkan").len(), 6);
    assert!(c.scheme_policies[SCHEME].capabilities.is_empty());
    assert!(
        !context(Path::new(
            "C:/DeltaForce/Binaries/Win64/DeltaForceClient-Win64-Shipping.exe"
        ))
        .delta
    );
    let mut invalid = c.clone();
    invalid
        .scheme_policies
        .get_mut(SCHEME)
        .unwrap()
        .max_selected_proxies = 6;
    assert!(invalid.validate().is_err());
    assert_eq!(
        c.packages
            .iter()
            .map(|p| (&p.id, &p.sha256))
            .collect::<std::collections::BTreeMap<_, _>>(),
        cloud::bundled()
            .packages
            .iter()
            .map(|p| (&p.id, &p.sha256))
            .collect::<std::collections::BTreeMap<_, _>>()
    );
    Ok(())
}
#[test]
#[cfg(feature = "fixture-tests")]
fn signed_package_and_defaults_preserve_exact_upstream_ini() -> Result<()> {
    let files = package()?;
    for n in ["version.dll", core::INI] {
        assert_eq!(
            files[n],
            fs::read(root().join("tests/fixtures/runtime/mfg").join(n))?
        );
    }
    let ctx = context(Path::new("C:/Game.exe"));
    let values = presets::defaults(presets::MFG_VULKAN, &BTreeMap::new());
    let configured = ctx.configure(&files[core::INI], &values)?;
    assert_eq!(configured, files[core::INI]);
    let ini = cleanup::parse_ini(std::str::from_utf8(&configured)?);
    assert_eq!(ini["FrameGeneration"]["MaxInterpolatedFrames"], "5");
    assert_eq!(ini["FrameGeneration"]["ForceMultiplier"], "0");
    assert!(!ini["FrameGeneration"].contains_key("MaxGeneratedFrames"));
    assert!(!ini.contains_key("Compatibility"));
    assert!(!ini["Logging"].contains_key("Level"));
    assert!(!ini["Logging"].contains_key("Directory"));
    assert_eq!(ini["Experimental"]["BlackwellScatter"], "0");
    assert_eq!(ini["Experimental"]["BlackwellBlend"], "0");
    for key in [
        "optimized",
        "preset",
        "logging_level",
        "max_generated_frames",
    ] {
        assert!(
            presets::validate(
                presets::MFG_VULKAN,
                &BTreeMap::from([(key.into(), "1".into())])
            )
            .is_err()
        );
    }
    Ok(())
}
#[test]
#[cfg(feature = "fixture-tests")]
fn mfg_install_edit_reapply_and_cleanup_preserve_game_files() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let exe = game(temp.path())?;
    let ctx = context(&exe);
    let mut files = package()?;
    let proxies = vec!["version.dll".into()];
    let original_exe = fs::read(&exe)?;
    fs::write(temp.path().join("dxgi.dll"), b"original game library")?;
    let values = presets::defaults(presets::MFG_VULKAN, &BTreeMap::new());
    files.insert(core::INI.into(), ctx.configure(&files[core::INI], &values)?);
    core::deploy_prepared_context(
        &exe,
        "upstream_sm86",
        &proxies,
        None,
        files.clone(),
        Some("1.0.0"),
        Some(&ctx),
    )?;
    let path = temp.path().join(core::INI);
    let custom = diagnostics::edit_ini(&fs::read(&path)?, "Hotkeys", "ForceX4", "Alt+F4")?;
    let custom = String::from_utf8(custom)? + "\r\n; keep user comment\r\n[User]\r\nValue=keep\r\n";
    let utf16: Vec<u8> = [0xff, 0xfe]
        .into_iter()
        .chain(custom.encode_utf16().flat_map(u16::to_le_bytes))
        .collect();
    fs::write(&path, utf16)?;
    let mut changed = values;
    changed.insert("force_multiplier".into(), "4".into());
    changed.insert("mfg_logging".into(), "0".into());
    files.insert(
        core::INI.into(),
        ctx.configure(&files[core::INI], &changed)?,
    );
    assert!(
        core::deploy_prepared_context(
            &exe,
            "upstream_sm86",
            &proxies,
            None,
            files,
            Some("1.0.0"),
            Some(&ctx)
        )?
        .contains("配置已更新")
    );
    let updated = fs::read(&path)?;
    assert!(updated.starts_with(&[0xff, 0xfe]));
    let (text, _) = diagnostics::decode_ini(&updated)?;
    assert!(text.contains("; keep user comment"));
    assert_eq!(
        diagnostics::ini_value(&text, "Hotkeys", "ForceX4")?.as_deref(),
        Some("Alt+F4")
    );
    let observed = presets::read_values(&updated, presets::MFG_VULKAN)?;
    assert_eq!(observed["force_multiplier"], "4");
    assert_eq!(observed["mfg_logging"], "0");
    assert_eq!(
        presets::inspect(&exe, &cloud::bundled())?.unwrap().0,
        SCHEME
    );
    cleanup::clean(&exe)?;
    cleanup::clean(&exe)?;
    assert!(!path.exists() && !temp.path().join("version.dll").exists());
    assert_eq!(fs::read(&exe)?, original_exe);
    assert_eq!(
        fs::read(temp.path().join("dxgi.dll"))?,
        b"original game library"
    );
    Ok(())
}
#[test]
#[cfg(feature = "fixture-tests")]
fn manual_signed_mfg_is_recognized_without_deployment_record() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let exe = game(temp.path())?;
    for (name, bytes) in package()? {
        fs::write(temp.path().join(name), bytes)?;
    }
    assert!(cleanup::known_proxy(&temp.path().join("version.dll"))?);
    assert_eq!(
        presets::inspect(&exe, &cloud::bundled())?.unwrap().0,
        SCHEME
    );
    cleanup::clean(&exe)?;
    assert_eq!(fs::read_dir(temp.path())?.count(), 1);
    Ok(())
}
#[test]
fn mfg_labels_and_help_are_translated() -> Result<()> {
    let ui = fs::read_to_string(root().join("rust/src/ui.rs"))?;
    let start = ui
        .find("if profile == rtx_fg_manager::presets::MFG_VULKAN")
        .unwrap();
    let end = ui[start..].find("} else if profile.starts_with").unwrap() + start;
    let quotes = regex::Regex::new("self\\.t\\(\"([^\"]+)\"\\)")?;
    let translations: BTreeMap<String, [String; 4]> =
        serde_json::from_slice(&fs::read(root().join("rust/ui-translations.json"))?)?;
    for (index, lang) in ["en", "ru", "ja", "ko"].into_iter().enumerate() {
        let tr = i18n::Translator::new(lang);
        for p in presets::parameters(presets::MFG_VULKAN) {
            // Some correct Japanese labels are identical to Chinese.
            let expected = &translations[p.label][index];
            assert!(!expected.is_empty());
            assert_eq!(&tr.t(p.label), expected);
        }
        for c in quotes.captures_iter(&ui[start..end]) {
            assert_ne!(tr.t(&c[1]), c[1]);
        }
    }
    Ok(())
}

#[test]
#[cfg(feature = "fixture-tests")]
fn upstream_d3d12_keeps_author_paths_and_repairs_old_deployment() -> Result<()> {
    let c = cloud::bundled();
    for scheme in ["upstream-0.3.5-310-9", "rtxfg-0.3.5-dx12-vulkan"] {
        let temp = tempfile::tempdir()?;
        let exe = game(temp.path())?;
        let p = c
            .packages
            .iter()
            .find(|p| p.scheme_id == scheme && p.proxy == "d3d12.dll")
            .unwrap();
        let mut files = cloud::unpack(
            p,
            &fs::read(
                root()
                    .join("tests/fixtures/runtime/packages")
                    .join(&p.archive),
            )?,
        )?;
        let author_ini = files[core::INI].clone();
        core::configure_package("upstream_sm86", &mut files)?;
        assert_eq!(files[core::INI], author_ini);
        let ctx = presets::Context::new(scheme, &c.scheme_policies[scheme], &exe);
        let proxies = vec!["d3d12.dll".into()];
        core::deploy_prepared_context(
            &exe,
            "upstream_sm86",
            &proxies,
            None,
            files.clone(),
            None,
            Some(&ctx),
        )?;
        let path = temp.path().join(core::INI);
        let old = diagnostics::edit_ini(
            &fs::read(&path)?,
            "Runtime",
            "CacheDirectory",
            ".rtx-fg-v3\\cache",
        )?;
        let old = diagnostics::edit_ini(&old, "Logging", "Directory", ".rtx-fg-v3\\logs")?;
        let old = String::from_utf8(old)? + "\r\n; user comment stays\r\n[User]\r\nKeep=1\r\n";
        fs::write(
            &path,
            [0xff, 0xfe]
                .into_iter()
                .chain(old.encode_utf16().flat_map(u16::to_le_bytes))
                .collect::<Vec<_>>(),
        )?;
        let orphan = temp.path().join(".rtx-fg-v3/cache/unknown.dll");
        fs::create_dir_all(orphan.parent().unwrap())?;
        fs::write(&orphan, b"unknown: retain")?;
        core::deploy_prepared_context(
            &exe,
            "upstream_sm86",
            &proxies,
            None,
            files.clone(),
            None,
            Some(&ctx),
        )?;
        let repaired = fs::read(&path)?;
        assert!(repaired.starts_with(&[0xff, 0xfe]));
        let (text, _) = diagnostics::decode_ini(&repaired)?;
        assert_eq!(
            diagnostics::ini_value(&text, "Runtime", "CacheDirectory")?.as_deref(),
            Some("")
        );
        assert_eq!(
            diagnostics::ini_value(&text, "Logging", "Directory")?.as_deref(),
            Some("dlssg_sm86\\logs")
        );
        assert!(text.contains("; user comment stays"));
        assert_eq!(fs::read(temp.path().join("d3d12.dll"))?, files["d3d12.dll"]);
        let custom =
            diagnostics::edit_ini(&repaired, "Runtime", "CacheDirectory", "MyCustomRuntime")?;
        let custom = diagnostics::edit_ini(&custom, "Logging", "Directory", "MyCustomLogs")?;
        fs::write(&path, &custom)?;
        core::deploy_prepared_context(
            &exe,
            "upstream_sm86",
            &proxies,
            None,
            files,
            None,
            Some(&ctx),
        )?;
        assert_eq!(fs::read(&path)?, custom);
        cleanup::clean(&exe)?;
        cleanup::clean(&exe)?;
        assert!(!path.exists() && !temp.path().join("d3d12.dll").exists());
        assert_eq!(fs::read(orphan)?, b"unknown: retain");
        assert!(exe.exists());
    }
    Ok(())
}
#[test]
fn path_migration_does_not_create_keys_or_change_other_protocols() -> Result<()> {
    let desired = b"[Runtime]\r\nCacheDirectory=\r\n[Logging]\r\nDirectory=dlssg_sm86\\logs\r\n";
    let old = b"[Runtime]\r\nCacheDirectory=.RTX-FG-V3/cache\r\n[Logging]\r\nDirectory=.rtx-fg-v3/logs\r\n";
    let repaired = presets::merge(old, desired, "upstream_sm86")?;
    assert_eq!(repaired, desired);
    let custom =
        b"[Runtime]\r\nCacheDirectory=C:\\games\\cache\r\n[Logging]\r\nDirectory=custom\r\n";
    assert_eq!(presets::merge(custom, desired, "upstream_sm86")?, custom);
    let missing = b"; empty ini\r\n";
    assert_eq!(presets::merge(missing, desired, "upstream_sm86")?, missing);
    assert_eq!(presets::merge(old, desired, "native_sm75")?, old);
    let ctx = context(Path::new("C:/Game.exe"));
    assert_eq!(
        presets::merge_context(old, desired, "upstream_sm86", Some(&ctx))?,
        old
    );
    Ok(())
}
