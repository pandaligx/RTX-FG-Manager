use anyhow::Result;
#[cfg(feature = "fixture-tests")]
use rtx_fg_manager::assets;
use rtx_fg_manager::{cleanup, core, gpu_alias, hags, i18n, preferences, scanner, updater, win};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};
fn pe(dll: bool) -> Vec<u8> {
    let mut b = vec![0; 1024];
    b[..2].copy_from_slice(b"MZ");
    b[60..64].copy_from_slice(&128u32.to_le_bytes());
    b[128..132].copy_from_slice(b"PE\0\0");
    b[132..134].copy_from_slice(&0x8664u16.to_le_bytes());
    b[148..150].copy_from_slice(&240u16.to_le_bytes());
    b[150..152].copy_from_slice(&(if dll { 0x2000u16 } else { 2 }).to_le_bytes());
    b[152..154].copy_from_slice(&0x20bu16.to_le_bytes());
    b
}
fn game(dir: &Path) -> PathBuf {
    let p = dir.join("Game.exe");
    fs::write(&p, pe(false)).unwrap();
    p
}
#[test]
#[cfg(feature = "fixture-tests")]
fn cloud_upstream_all_six_proxies_deploy_and_uninstall_offline() -> Result<()> {
    let c = rtx_fg_manager::cloud::bundled();
    for p in c
        .packages
        .iter()
        .filter(|p| p.scheme_id.starts_with("upstream"))
    {
        let dir = tempfile::tempdir()?;
        let exe = game(dir.path());
        let archive = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/runtime/packages")
            .join(&p.archive);
        let mut files = rtx_fg_manager::cloud::unpack(p, &fs::read(archive)?)?;
        core::configure_package("upstream_sm86", &mut files)?;
        let other = if p.proxy == "dbghelp.dll" {
            "d3d12.dll"
        } else {
            "dbghelp.dll"
        };
        fs::write(dir.path().join(other), b"original game library")?;
        core::deploy_prepared(
            &exe,
            "upstream_sm86",
            std::slice::from_ref(&p.proxy),
            Some(1),
            files,
            Some("0.3.5"),
        )?;
        assert!(core::status(&exe).contains("@0.3.5"));
        fs::write(dir.path().join(core::INI), b"[Logging]\nLevel=3\n")?;
        fs::write(dir.path().join("game-original.dat"), b"preserve")?;
        cleanup::clean(&exe)?;
        assert!(!dir.path().join(&p.proxy).exists());
        assert!(exe.exists());
        assert_eq!(fs::read(dir.path().join(other))?, b"original game library");
        assert_eq!(fs::read(dir.path().join("game-original.dat"))?, b"preserve");
    }
    Ok(())
}
#[test]
#[cfg(feature = "fixture-tests")]
fn cloud_upstream_multi_proxy_and_per_game_options_preserve_advanced_keys() -> Result<()> {
    let c = rtx_fg_manager::cloud::bundled();
    let packages = c
        .packages
        .iter()
        .filter(|p| p.scheme_id == "upstream-0.3.5-310-9")
        .collect::<Vec<_>>();
    assert_eq!(packages.len(), 6);
    let mut files = BTreeMap::new();
    let mut proxies = Vec::new();
    for p in packages {
        let archive = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/runtime/packages")
            .join(&p.archive);
        for (name, bytes) in rtx_fg_manager::cloud::unpack(p, &fs::read(archive)?)? {
            if let Some(existing) = files.insert(name.clone(), bytes.clone()) {
                assert_eq!(existing, bytes, "shared INI differs");
            }
        }
        proxies.push(p.proxy.clone());
    }
    core::configure_package("upstream_sm86", &mut files)?;
    let first = core::UpstreamOptions {
        optimized: true,
        max_generated_frames: 5,
        preset: "B".into(),
        logging_level: 2,
    };
    files.insert(
        core::INI.into(),
        core::configure_upstream_ini(&files[core::INI], &first)?,
    );
    let dir = tempfile::tempdir()?;
    let exe = game(dir.path());
    core::deploy_prepared(
        &exe,
        "upstream_sm86",
        &proxies,
        None,
        files.clone(),
        Some("0.3.5"),
    )?;
    for proxy in &proxies {
        assert!(dir.path().join(proxy).is_file());
    }
    let ini_path = dir.path().join(core::INI);
    let with_advanced = rtx_fg_manager::diagnostics::edit_ini(
        &fs::read(&ini_path)?,
        "Compatibility",
        "KernelImage",
        "PTX",
    )?;
    fs::write(&ini_path, with_advanced)?;
    let second = core::UpstreamOptions {
        optimized: false,
        max_generated_frames: 3,
        preset: "Auto".into(),
        logging_level: 1,
    };
    files.insert(
        core::INI.into(),
        core::configure_upstream_ini(&files[core::INI], &second)?,
    );
    let message =
        core::deploy_prepared(&exe, "upstream_sm86", &proxies, None, files, Some("0.3.5"))?;
    assert!(message.contains("配置已更新"));
    let ini = fs::read_to_string(&ini_path)?;
    let parsed = cleanup::parse_ini(&ini);
    assert_eq!(parsed["FrameGeneration"]["Optimized"], "0");
    assert_eq!(parsed["FrameGeneration"]["MaxGeneratedFrames"], "3");
    assert_eq!(parsed["Compatibility"]["Preset"], "Auto");
    assert_eq!(parsed["Compatibility"]["KernelImage"], "PTX");
    assert_eq!(parsed["Logging"]["Level"], "1");
    cleanup::clean(&exe)?;
    assert!(proxies.iter().all(|proxy| !dir.path().join(proxy).exists()));
    Ok(())
}
#[test]
#[cfg(feature = "fixture-tests")]
fn manually_added_new_cloud_proxies_clean_without_network_or_record() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let exe = game(dir.path());
    for p in rtx_fg_manager::cloud::bundled()
        .packages
        .iter()
        .filter(|p| {
            p.scheme_id.starts_with("upstream")
                && ["dbghelp.dll", "d3d12.dll"].contains(&p.proxy.as_str())
        })
    {
        let archive = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/runtime/packages")
            .join(&p.archive);
        let files = rtx_fg_manager::cloud::unpack(p, &fs::read(archive)?)?;
        fs::write(dir.path().join(&p.proxy), &files[&p.proxy])?;
    }
    assert!(core::status(&exe).contains("手动"));
    cleanup::clean(&exe)?;
    assert!(!dir.path().join("dbghelp.dll").exists() && !dir.path().join("d3d12.dll").exists());
    fs::write(dir.path().join("dbghelp.dll"), b"game original")?;
    assert_eq!(core::status(&exe), "未部署");
    assert!(exe.exists());
    Ok(())
}
#[test]
fn malformed_pe_and_certificate_bounds() {
    for n in 0..300 {
        assert!(cleanup::image_digest(&vec![0; n]).is_none());
    }
    let mut b = pe(true);
    let hash = cleanup::image_digest(&b).unwrap();
    b[216] = 50;
    assert_eq!(cleanup::image_digest(&b).unwrap(), hash);
    b[296..300].copy_from_slice(&1024u32.to_le_bytes());
    b[300..304].copy_from_slice(&16u32.to_le_bytes());
    b.extend([4; 16]);
    assert_eq!(cleanup::image_digest(&b).unwrap(), hash);
    b.extend([9; 8]);
    assert_ne!(cleanup::image_digest(&b).unwrap(), hash);
    b[296..300].copy_from_slice(&160u32.to_le_bytes());
    assert!(cleanup::image_digest(&b).is_none());
}
#[test]
fn pe_rejects_dll_exe_confusion() -> Result<()> {
    let d = tempfile::tempdir()?;
    let p = game(d.path());
    core::pe64(&p, false)?;
    assert!(core::pe64(&p, true).is_err());
    fs::write(&p, pe(true))?;
    assert!(core::location(&p, true).is_err());
    Ok(())
}
#[test]
fn unsupported_architecture_is_not_reported_as_a_non_game() -> Result<()> {
    let d = tempfile::tempdir()?;
    let p = d.path().join("Example.exe");
    for machine in [0x14cu16, 0xaa64] {
        let mut bytes = pe(false);
        bytes[132..134].copy_from_slice(&machine.to_le_bytes());
        fs::write(&p, bytes)?;
        assert_eq!(core::library_location(&p)?, p);
        assert!(core::location(&p, true).is_err());
        assert!(core::deploy(&p, "native30", &["version.dll".into()]).is_err());
        assert_eq!(
            fs::read_dir(d.path())?.count(),
            1,
            "incompatible deployment must not write files"
        );
        let message = core::pe64(&p, false).unwrap_err().to_string();
        assert!(message.contains("当前文件架构不匹配"));
        for language in ["en", "ru", "ja", "ko"] {
            assert_ne!(i18n::Translator::new(language).t(&message), message);
        }
    }
    let mut bytes = pe(false);
    bytes[128] = 0;
    fs::write(&p, bytes)?;
    assert_eq!(
        core::pe64(&p, false).unwrap_err().to_string(),
        "不是 Windows EXE 文件"
    );
    Ok(())
}
#[test]
fn scanner_finds_unreal_non_shipping_and_alternate_binary_layouts() -> Result<()> {
    let d = tempfile::tempdir()?;
    let root = d.path().join("燕云十六声/yysls_medium");
    for layout in ["Win64", "Win64r", "Win64rh", "Win64_shipping"] {
        let dir = root.join("Engine/Binaries").join(layout);
        fs::create_dir_all(&dir)?;
        fs::write(dir.join("yysls.exe"), pe(false))?;
        for tool in [
            "Launcher.exe",
            "CrashReportClient.exe",
            "UnrealPak.exe",
            "ShaderCompileWorker.exe",
            "UE4Editor.exe",
        ] {
            fs::write(dir.join(tool), pe(false))?;
        }
    }
    let r = scanner::scan(&[d.path().into()], &AtomicBool::new(false), |_, _, _| {})?;
    assert_eq!(r.rows.len(), 1);
    for row in r.rows {
        assert!(row.exe.ends_with("yysls.exe"));
        assert_eq!(row.targets.len(), 4);
        assert_eq!(core::key(Path::new(&row.root)), core::key(&root));
        assert!(row.reasons.iter().any(|r| r.contains("兼容性待确认")));
        assert!(!row.reasons.iter().any(|r| r.contains("帧生成组件")));
    }
    Ok(())
}
#[test]
fn scanner_unity_requires_matching_data_and_player() -> Result<()> {
    let d = tempfile::tempdir()?;
    let good = d.path().join("UnityGame");
    fs::create_dir_all(good.join("Example_Data"))?;
    fs::write(good.join("Example.exe"), pe(false))?;
    fs::write(good.join("UnityPlayer.dll"), b"evidence")?;
    fs::write(good.join("Aardvark.exe"), pe(false))?;
    let bad = d.path().join("Tool");
    fs::create_dir_all(bad.join("Other_Data"))?;
    fs::write(bad.join("Example.exe"), pe(false))?;
    fs::write(bad.join("UnityPlayer.dll"), b"evidence")?;
    let r = scanner::scan(&[d.path().into()], &AtomicBool::new(false), |_, _, _| {})?;
    assert_eq!(r.rows.len(), 1);
    assert_eq!(
        r.rows[0].exe,
        good.join("Example.exe").display().to_string()
    );
    Ok(())
}
#[test]
fn scanner_sr_evidence_stays_inside_game_and_preserves_architecture_filter() -> Result<()> {
    let d = tempfile::tempdir()?;
    let good = d.path().join("Example");
    fs::create_dir_all(good.join("plugins"))?;
    fs::write(good.join("plugins/nvngx_dlss.dll"), b"evidence")?;
    fs::write(good.join("Example.exe"), pe(false))?;
    let sibling = d.path().join("Unrelated");
    fs::create_dir_all(&sibling)?;
    fs::write(sibling.join("Example.exe"), pe(false))?;
    let x86 = d.path().join("32bit/Engine/Binaries/Win64");
    fs::create_dir_all(&x86)?;
    let mut b = pe(false);
    b[132..134].copy_from_slice(&0x14cu16.to_le_bytes());
    fs::write(x86.join("Example.exe"), b)?;
    let r = scanner::scan(&[d.path().into()], &AtomicBool::new(false), |_, _, _| {})?;
    assert_eq!(r.rows.len(), 1);
    assert_eq!(
        r.rows[0].exe,
        good.join("Example.exe").display().to_string()
    );
    assert!(
        r.rows[0]
            .reasons
            .iter()
            .any(|r| r.contains("不代表支持帧生成"))
    );
    Ok(())
}
#[test]
fn scanner_matches_evidence_and_ignores_launchers() -> Result<()> {
    let d = tempfile::tempdir()?;
    let root = d.path().join("Game");
    let dir = root.join("Project/Binaries/Win64");
    fs::create_dir_all(&dir)?;
    fs::create_dir_all(root.join("Engine/Plugins"))?;
    fs::write(root.join("Engine/Plugins/sl.dlss_g.dll"), b"evidence")?;
    fs::write(dir.join("Game-Win64-Shipping.exe"), pe(false))?;
    fs::write(dir.join("Launcher.exe"), pe(false))?;
    fs::write(dir.join("Other.exe"), pe(false))?;
    let r = scanner::scan(
        &[d.path().into(), d.path().into()],
        &AtomicBool::new(false),
        |_, _, _| {},
    )?;
    assert_eq!(r.rows.len(), 1);
    assert_eq!(r.rows[0].targets.len(), 1);
    assert!(r.rows.iter().all(|row| !row.exe.ends_with("Launcher.exe")));
    assert!(r.rows[0].exe.ends_with("Game-Win64-Shipping.exe"));
    assert_eq!(r.rows[0].rank, 105);
    assert_eq!(r.rows[0].root, root.display().to_string());
    let r = scanner::scan(&[d.path().into()], &AtomicBool::new(true), |_, _, _| {})?;
    assert!(r.cancelled);
    assert_eq!(r.directories, 0);
    Ok(())
}
#[test]
fn preferences_preserve_legacy_and_unknown_fields() -> Result<()> {
    let d = tempfile::tempdir()?;
    let v = serde_json::json!({"schema":3,"language":"ja","games":[{"exe":"D:\\Game\\game.exe","reason":"manual"}],"roots":["D:\\"],"future":{"a":1}});
    core::atomic_json(&d.path().join("games.json"), &v)?;
    assert_eq!(preferences::load(d.path())?, v);
    let s = preferences::Store::new(d.path().into());
    for i in 0..40 {
        let mut next = v.clone();
        next["future"]["a"] = serde_json::json!(i);
        s.save(next)?;
    }
    s.finish()?;
    assert_eq!(preferences::load(d.path())?["future"]["a"], 39);
    let p = d.path().join("games.json");
    fs::write(&p, b"corrupt")?;
    assert!(preferences::load(d.path()).is_err());
    assert_eq!(fs::read(p)?, b"corrupt");
    Ok(())
}
#[test]
fn all_languages_include_usage_sections() {
    let help: Vec<(String, String)> =
        serde_json::from_str(include_str!("../rust/assets/help.json")).unwrap();
    for lang in ["en", "ru", "ja", "ko"] {
        let tr = i18n::Translator::new(lang);
        for (t, b) in &help {
            assert_ne!(tr.t(t), *t, "{lang}: {t}");
            assert_ne!(tr.t(b), *b, "{lang}: {t}");
        }
    }
}
#[test]
#[cfg(feature = "fixture-tests")]
fn install_clean_edited_ini_and_manual_proxy() -> Result<()> {
    let d = tempfile::tempdir()?;
    let exe = game(d.path());
    core::deploy(&exe, "native20", &["version.dll".into()])?;
    fs::write(
        d.path().join(core::INI),
        "[Logging]\nDirectory=custom/log\nLevel=3\n",
    )?;
    fs::create_dir_all(d.path().join("custom/log"))?;
    fs::write(d.path().join("custom/log/native_123.jsonl"), b"test")?;
    fs::write(d.path().join("custom/log/keep.txt"), b"user")?;
    fs::write(
        d.path().join("winmm.dll"),
        assets::bytes("payloads/native/winmm.dll")?,
    )?;
    fs::write(d.path().join("dxgi.dll"), b"unrelated game mod")?;
    assert!(core::status(&exe).contains("配置已修改"));
    let result = cleanup::clean(&exe)?;
    assert!(result.contains("保留"));
    assert!(!d.path().join("version.dll").exists());
    assert!(!d.path().join("winmm.dll").exists());
    assert!(!d.path().join(core::INI).exists());
    assert!(!d.path().join("custom/log/native_123.jsonl").exists());
    assert_eq!(fs::read(d.path().join("dxgi.dll"))?, b"unrelated game mod");
    assert_eq!(fs::read(d.path().join("custom/log/keep.txt"))?, b"user");
    assert!(exe.exists());
    assert!(!d.path().join(core::OWN).join(core::MARKER).exists());
    Ok(())
}
#[test]
fn unknown_proxy_only_is_never_removed() -> Result<()> {
    let d = tempfile::tempdir()?;
    let exe = game(d.path());
    fs::write(d.path().join("version.dll"), b"game DLL")?;
    fs::write(d.path().join(core::INI), b"user ini")?;
    cleanup::clean(&exe)?;
    assert_eq!(fs::read(d.path().join("version.dll"))?, b"game DLL");
    assert!(d.path().join(core::INI).exists());
    assert!(core::deploy(&exe, "native30", &["version.dll".into()]).is_err());
    assert!(!d.path().join(core::OWN).exists());
    Ok(())
}
#[test]
fn custom_cleanup_stays_inside_game() -> Result<()> {
    let d = tempfile::tempdir()?;
    assert!(cleanup::relative_dir(d.path(), "../other")?.is_none());
    assert!(cleanup::relative_dir(d.path(), "C:\\Windows")?.is_none());
    assert!(cleanup::relative_dir(d.path(), "logs:stream")?.is_none());
    assert!(cleanup::relative_dir(d.path(), "%TEMP%")?.is_none());
    let nested = d.path().join("other");
    fs::create_dir_all(nested.join(core::OWN))?;
    fs::write(nested.join(core::OWN).join(core::MARKER), b"protected")?;
    assert!(cleanup::relative_dir(d.path(), "other/logs")?.is_none());
    Ok(())
}
#[test]
#[cfg(feature = "fixture-tests")]
fn dll_profiles_and_multiselect_keep_correct_routes() -> Result<()> {
    for backend in core::BACKENDS.into_iter().filter(|b| *b != "upstream_sm86") {
        let p = core::package(backend, &["version.dll".into()])?;
        let c = cleanup::parse_ini(std::str::from_utf8(&p[core::INI])?);
        if backend.starts_with("native") {
            assert_eq!(
                c["Compatibility"]["Router"],
                if backend.ends_with("20") {
                    "SM75"
                } else {
                    "SM86"
                }
            );
            assert_eq!(
                c["FrameGeneration"]["MaxGeneratedFrames"],
                if backend.contains("x6") { "5" } else { "3" }
            );
        }
        assert_eq!(
            core::hash(&p["version.dll"]),
            assets::EMBEDDED
                .iter()
                .find(|r| r.name
                    == format!("payloads/{}/version.dll", core::folder(backend).unwrap()))
                .unwrap()
                .sha256
        );
    }
    assert!(core::deployment_names("rtx20", &["winmm.dll".into()]).is_err());
    assert!(core::normalize_proxies(&[]).is_err());
    assert!(core::normalize_proxies(&["version.dll".into(), "version.dll".into()]).is_err());
    Ok(())
}

#[test]
fn normal_build_resources_and_proxy_selection_are_explicit() {
    assert!(core::deployment_names("rtx20", &["winmm.dll".into()]).is_err());
    assert!(core::normalize_proxies(&[]).is_err());
    assert!(core::normalize_proxies(&["version.dll".into(), "version.dll".into()]).is_err());
    #[cfg(not(feature = "fixture-tests"))]
    assert_eq!(
        rtx_fg_manager::assets::EMBEDDED
            .iter()
            .map(|r| r.name)
            .collect::<Vec<_>>(),
        ["app/tools/aria2c.exe", "app/tools/aria2.conf"]
    );
}
#[test]
fn hags_driver_state_beats_registry_intent() {
    let a = |enabled| hags::Adapter {
        name: "NVIDIA".into(),
        supported: Some(true),
        enabled: Some(enabled),
        flags: None,
    };
    assert_eq!(
        hags::classify(vec![a(false)], Some(2), 22631).state,
        "pending_restart"
    );
    assert_eq!(hags::classify(vec![a(true)], None, 22631).state, "enabled");
    assert_eq!(
        hags::classify(vec![a(true)], Some(1), 22631).state,
        "pending_disable"
    );
    assert_eq!(hags::classify(vec![], None, 22631).state, "unknown");
    assert!(!hags::prompt(
        &hags::classify(vec![a(true)], None, 22631),
        false
    ));
}
fn manifest() -> updater::Manifest {
    updater::Manifest{schema:1,version:"4.0.0".into(),file:"RTXManager-v4.0.0-x64.exe".into(),bytes:1048576,sha256:"a".repeat(64),source:"github".into(),url:"https://github.com/pandaligx/RTX-FG-Manager/releases/download/v4.0.0/RTXManager-v4.0.0-x64.exe".into()}
}
#[test]
fn malicious_updates_and_aria_options_rejected() {
    for url in [
        "http://github.com/a",
        "https://evil.test/a",
        "https://github.com.evil.test/a",
        "https://github.com@evil.test/a",
        "https://github.com:8443/a",
        "file:///C:/a",
        "https://github.com/a\n",
    ] {
        assert!(updater::safe_url(url).is_err(), "{url}")
    }
    assert!(updater::safe_url("https://release-assets.githubusercontent.com/a").is_ok());
    let mut m = manifest();
    m.validate("v4.0.0").unwrap();
    m.file = "../game.exe".into();
    assert!(m.validate("4.0.0").is_err());
    for v in ["4.0", "4.0.0.1", "4.-1.2", "4.00000.0", "4.0.0/x"] {
        assert!(updater::version(v).is_err())
    }
    let opts = updater::aria_options(
        "enable-rpc=true\ncheck-certificate=false\ndir=C:\\Windows\non-download-complete=evil\nsplit=99\nasync-dns-server=127.0.0.1,8.8.8.8",
    );
    assert!(opts.contains(&"--split=30".into()));
    assert!(!opts.iter().any(|s| s.contains("evil")
        || s.contains("certificate")
        || s.contains("rpc")
        || s.contains("Windows")));
}
#[test]
fn update_rename_rolls_back_without_overwriting_changed_target() -> Result<()> {
    let d = tempfile::tempdir()?;
    let target = d.path().join("app.exe");
    let stage = d.path().join("new.exe");
    let backup = d.path().join("previous.exe");
    fs::write(&target, b"old")?;
    fs::write(&stage, b"new")?;
    assert!(
        updater::replace_and_launch(
            &target,
            &stage,
            &backup,
            &core::hash(b"old"),
            || anyhow::bail!("launch failure")
        )
        .is_err()
    );
    assert_eq!(fs::read(&target)?, b"old");
    assert!(!backup.exists());
    fs::write(&stage, b"new")?;
    assert!(
        updater::replace_and_launch(&target, &stage, &backup, &core::hash(b"other"), || Ok(()))
            .is_err()
    );
    assert_eq!(fs::read(&target)?, b"old");
    assert!(stage.exists());
    Ok(())
}
#[test]
#[cfg(feature = "fixture-tests")]
fn released_exe_signature_is_accepted() -> Result<()> {
    // Use the retained current signed release, not a removable historical backup.
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/runtime/releases/RTXManager-v4.2.1-x64.exe");
    let s = win::verify_signature(&p)?;
    assert_eq!(s.version, "4.2.1");
    assert_eq!(
        core::digest(&p)?,
        "0a46be83b9a11b23bd1d90c9123d558f95b013459c3288c6019bcdd114ec6194"
    );
    let d = tempfile::tempdir()?;
    assert!(win::verify_signature(&game(d.path())).is_err());
    Ok(())
}

#[test]
fn versioned_update_changes_filename_and_rolls_back_safely() -> Result<()> {
    let d = tempfile::tempdir()?;
    let old = d.path().join("RTXManager-v4.0.8-x64.exe");
    let new = d.path().join("RTXManager-v4.0.9-x64.exe");
    let stage = d.path().join("staged.new");
    let backup = d.path().join("old.previous");
    fs::write(&old, b"old")?;
    fs::write(&stage, b"new")?;
    fs::write(&new, b"unrelated")?;
    assert!(
        updater::replace_named_and_launch(&old, &new, &stage, &backup, &core::hash(b"old"), || Ok(
            ()
        ))
        .is_err()
    );
    assert_eq!(fs::read(&old)?, b"old");
    assert_eq!(fs::read(&new)?, b"unrelated");
    assert!(!backup.exists());
    fs::remove_file(&new)?;
    assert!(
        updater::replace_named_and_launch(
            &old,
            &new,
            &stage,
            &backup,
            &core::hash(b"old"),
            || anyhow::bail!("launch failed")
        )
        .is_err()
    );
    assert_eq!(fs::read(&old)?, b"old");
    assert!(!new.exists());
    assert!(!backup.exists());
    fs::write(&stage, b"new")?;
    updater::replace_named_and_launch(&old, &new, &stage, &backup, &core::hash(b"old"), || {
        assert!(!old.exists());
        assert_eq!(fs::read(&new)?, b"new");
        Ok(())
    })?;
    assert!(!old.exists());
    assert_eq!(fs::read(&new)?, b"new");
    assert!(!backup.exists());
    Ok(())
}

#[test]
fn versioned_update_preserves_modified_destination_on_failed_launch() -> Result<()> {
    let d = tempfile::tempdir()?;
    let old = d.path().join("old.exe");
    let new = d.path().join("new.exe");
    let stage = d.path().join("staged.new");
    let backup = d.path().join("old.previous");
    fs::write(&old, b"old")?;
    fs::write(&stage, b"new")?;
    assert!(
        updater::replace_named_and_launch(&old, &new, &stage, &backup, &core::hash(b"old"), || {
            fs::write(&new, b"external change")?;
            anyhow::bail!("launch failed")
        })
        .is_err()
    );
    assert_eq!(fs::read(&new)?, b"external change");
    assert_eq!(fs::read(&backup)?, b"old");
    Ok(())
}

#[test]
#[cfg(feature = "fixture-tests")]
fn renamed_update_launches_verified_windows_baseline_from_new_path() -> Result<()> {
    use std::os::windows::process::CommandExt;
    let d = tempfile::tempdir()?;
    let old = d.path().join("old-manager.exe");
    let new = d.path().join("RTXManager-v4.0.9-x64.exe");
    let stage = d.path().join("verified.new");
    let backup = d.path().join("old.previous");
    let report = d.path().join("payload-report.json");
    // Exercise the rename/launch transaction with a signed, known baseline.
    // This does not claim that the unsigned candidate passed update verification.
    let baseline = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/runtime/releases/RTXManager-v4.0.8-x64.exe");
    assert_eq!(win::verify_signature(&baseline)?.version, "4.0.8");
    fs::copy(&baseline, &stage)?;
    fs::write(&old, b"old fixture")?;
    updater::replace_named_and_launch(
        &old,
        &new,
        &stage,
        &backup,
        &core::hash(b"old fixture"),
        || {
            let mut child = std::process::Command::new(&new)
                .args(["--payload-report".as_ref(), report.as_os_str()])
                .creation_flags(0x08000000)
                .spawn()?;
            let start = std::time::Instant::now();
            loop {
                if let Some(status) = child.try_wait()? {
                    anyhow::ensure!(status.success(), "baseline process failed");
                    break;
                }
                if start.elapsed() > std::time::Duration::from_secs(20) {
                    let _ = child.kill();
                    let _ = child.wait();
                    anyhow::bail!("baseline process timed out");
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            let resources: serde_json::Value = serde_json::from_slice(&fs::read(&report)?)?;
            assert_eq!(resources.as_array().unwrap().len(), 18);
            Ok(())
        },
    )?;
    assert!(!old.exists());
    assert!(!backup.exists());
    assert_eq!(core::digest(&new)?, core::digest(&baseline)?);
    Ok(())
}
#[derive(Default)]
struct FakeGpu {
    fields: BTreeMap<String, Option<gpu_alias::Value>>,
    record: Option<gpu_alias::Record>,
    fail: Option<String>,
    id: Option<serde_json::Value>,
    installed_name: Option<String>,
}
impl gpu_alias::Backend for FakeGpu {
    fn identity(&self) -> Result<serde_json::Value> {
        Ok(self.id.clone().unwrap_or(serde_json::json!({"gpu":"test"})))
    }
    fn read(&self, f: &str) -> Result<Option<gpu_alias::Value>> {
        Ok(self.fields.get(f).cloned().flatten())
    }
    fn write(&mut self, f: &str, v: Option<&gpu_alias::Value>) -> Result<()> {
        if self.fail.as_deref() == Some(f) {
            self.fail = None;
            anyhow::bail!("failure")
        }
        self.fields.insert(f.into(), v.cloned());
        Ok(())
    }
    fn load(&self) -> Result<Option<gpu_alias::Record>> {
        Ok(self.record.clone())
    }
    fn save(&mut self, r: &gpu_alias::Record) -> Result<()> {
        self.record = Some(r.clone());
        Ok(())
    }
    fn clear(&mut self) -> Result<()> {
        self.record = None;
        Ok(())
    }
    fn driver_description(&self) -> Result<Option<String>> {
        Ok(self.installed_name.clone())
    }
}

fn driver_identity(version: &str) -> serde_json::Value {
    serde_json::json!({"instance":"PCI\\VEN_10DE&DEV_2503&SUBSYS_000110DE\\4&123&0&0008","driver":format!("{{4d36e968-e325-11ce-bfc1-08002be10318}}\\{version}"),"version":version,"inf":format!("oem{version}.inf")})
}
fn registry_name(name: &str) -> Option<gpu_alias::Value> {
    Some(gpu_alias::Value {
        kind: 1,
        data: name.into(),
    })
}
fn renamed_gpu() -> Result<FakeGpu> {
    let mut b = FakeGpu {
        id: Some(driver_identity("0001")),
        installed_name: Some("NVIDIA GeForce RTX 3060".into()),
        ..Default::default()
    };
    for f in gpu_alias::FIELDS {
        b.fields
            .insert(f.into(), registry_name("NVIDIA GeForce RTX 3060"));
    }
    b.fields.insert("FriendlyName".into(), None);
    b.fields.insert(
        "DeviceDesc".into(),
        registry_name("@oem0001.inf,%gpu%;NVIDIA GeForce RTX 3060"),
    );
    gpu_alias::change(&mut b, Some(gpu_alias::NAMES[1]))?;
    b.id = Some(driver_identity("0002"));
    Ok(b)
}
#[test]
fn gpu_driver_upgrade_restore_preserves_new_driver_values() -> Result<()> {
    let mut b = renamed_gpu()?;
    b.fields.insert(
        "DriverDesc".into(),
        registry_name("NVIDIA GeForce RTX 3060"),
    );
    b.fields.insert(
        "HardwareInformation.AdapterString".into(),
        registry_name("new driver adapter string"),
    );
    b.fields.insert(
        "DeviceDesc".into(),
        registry_name("@oem0002.inf,%new%;NVIDIA GeForce RTX 3060"),
    );
    let mut expected = b.fields.clone();
    expected.insert("FriendlyName".into(), None);
    gpu_alias::change(&mut b, None)?;
    assert_eq!(b.fields, expected);
    assert!(b.record.is_none());
    Ok(())
}
#[test]
fn gpu_driver_upgrade_rebases_surviving_alias_and_can_rename_again() -> Result<()> {
    let mut b = renamed_gpu()?;
    gpu_alias::change(&mut b, Some(gpu_alias::NAMES[0]))?;
    assert_eq!(b.record.as_ref().unwrap().identity, driver_identity("0002"));
    gpu_alias::change(&mut b, None)?;
    assert_eq!(b.fields["FriendlyName"], None);
    for f in &gpu_alias::FIELDS[1..] {
        assert_eq!(b.fields[*f], registry_name("NVIDIA GeForce RTX 3060"));
    }
    assert!(b.record.is_none());
    Ok(())
}
#[test]
fn gpu_driver_upgrade_preserves_external_values_and_fully_reset_driver() -> Result<()> {
    let mut b = renamed_gpu()?;
    for f in gpu_alias::FIELDS {
        b.fields
            .insert(f.into(), registry_name("driver or external value"));
    }
    let expected = b.fields.clone();
    b.installed_name = None;
    gpu_alias::change(&mut b, None)?;
    assert_eq!(b.fields, expected);
    assert!(b.record.is_none());
    Ok(())
}
#[test]
fn gpu_driver_upgrade_failure_retains_recoverable_new_baseline() -> Result<()> {
    let mut b = renamed_gpu()?;
    let current = b.fields.clone();
    b.fail = Some("DriverDesc".into());
    assert!(gpu_alias::change(&mut b, None).is_err());
    assert_eq!(b.fields, current);
    assert_eq!(b.record.as_ref().unwrap().identity, driver_identity("0002"));
    gpu_alias::change(&mut b, None)?;
    assert_eq!(
        b.fields["DeviceDesc"],
        registry_name("NVIDIA GeForce RTX 3060")
    );
    Ok(())
}
#[test]
fn gpu_driver_migration_rejects_other_hardware_corrupt_backup_and_missing_driver_name() -> Result<()>
{
    let mut b = renamed_gpu()?;
    let current = b.fields.clone();
    let old = b.record.clone();
    b.id.as_mut().unwrap()["instance"] = serde_json::json!("PCI\\VEN_10DE&DEV_9999\\4&123&0&0008");
    assert!(gpu_alias::change(&mut b, None).is_err());
    assert_eq!(b.fields, current);
    b.id = Some(driver_identity("0002"));
    b.record.as_mut().unwrap().original.remove("DriverDesc");
    assert!(gpu_alias::change(&mut b, None).is_err());
    assert_eq!(b.fields, current);
    b.record = old;
    b.installed_name = None;
    assert!(gpu_alias::change(&mut b, None).is_err());
    assert_eq!(b.fields, current);
    assert_eq!(b.record.as_ref().unwrap().identity, driver_identity("0001"));
    Ok(())
}
#[test]
fn gpu_legacy_python_schema1_migrates_without_claiming_device_desc() -> Result<()> {
    let mut b = renamed_gpu()?;
    let r = b.record.as_mut().unwrap();
    r.schema = 1;
    for map in [&mut r.original, &mut r.previous, &mut r.written] {
        map.remove("DeviceDesc");
    }
    b.fields.insert(
        "DeviceDesc".into(),
        registry_name("new driver untouched DeviceDesc"),
    );
    gpu_alias::change(&mut b, None)?;
    assert_eq!(
        b.fields["DeviceDesc"],
        registry_name("new driver untouched DeviceDesc")
    );
    assert_eq!(b.fields["FriendlyName"], None);
    Ok(())
}

#[test]
#[cfg(feature = "fixture-tests")]
fn installed_payload_version_is_recorded_without_relabeling_legacy_records() -> Result<()> {
    let d = tempfile::tempdir()?;
    let exe = game(d.path());
    core::deploy(&exe, "native30", &["version.dll".into()])?;
    assert!(core::status(&exe).contains("@0.2.6"));
    let marker = d.path().join(core::OWN).join(core::MARKER);
    let mut legacy = core::read_json(&marker, 20000)?;
    legacy.as_object_mut().unwrap().remove("payload_version");
    core::atomic_json(&marker, &legacy)?;
    assert!(!core::status(&exe).contains("@0.2.6"));
    cleanup::clean(&exe)?;
    assert!(exe.exists());
    Ok(())
}

#[test]
#[cfg(feature = "fixture-tests")]
fn native026_all_proxy_cleanup_survives_certificate_layout_changes() -> Result<()> {
    for backend in ["native20", "native_x6_30"] {
        let d = tempfile::tempdir()?;
        let exe = game(d.path());
        let native_proxies = &core::PROXIES[..5];
        core::deploy(
            &exe,
            backend,
            &native_proxies
                .iter()
                .map(|s| (*s).to_string())
                .collect::<Vec<_>>(),
        )?;
        for name in native_proxies {
            let path = d.path().join(name);
            let mut b = fs::read(&path)?;
            let expected = cleanup::image_digest(&b);
            let pe = u32::from_le_bytes(b[60..64].try_into().unwrap()) as usize;
            let certificate = pe + 24 + 144;
            let old =
                u32::from_le_bytes(b[certificate..certificate + 4].try_into().unwrap()) as usize;
            if old != 0 {
                b.truncate(old);
            }
            let end = b.len() as u32;
            b[certificate..certificate + 4].copy_from_slice(&end.to_le_bytes());
            b[certificate + 4..certificate + 8].copy_from_slice(&16u32.to_le_bytes());
            b.extend([0u8; 16]); // Certificate-layout fixture, not an actual signature.
            assert_eq!(cleanup::image_digest(&b), expected);
            fs::write(&path, b)?;
            assert!(cleanup::known_proxy(&path)?);
        }
        fs::write(d.path().join(core::INI), "[Logging]\nLevel=3\n")?;
        cleanup::clean(&exe)?;
        assert!(exe.exists());
        for name in core::PROXIES {
            assert!(!d.path().join(name).exists());
        }
    }
    Ok(())
}
#[test]
fn gpu_alias_preserves_originals_and_recovers_partial_failures() -> Result<()> {
    let mut b = FakeGpu::default();
    for f in gpu_alias::FIELDS {
        b.fields.insert(
            f.into(),
            Some(gpu_alias::Value {
                kind: 1,
                data: format!("original {f}"),
            }),
        );
    }
    let original = b.fields.clone();
    b.fail = Some("DriverDesc".into());
    assert!(gpu_alias::change(&mut b, Some(gpu_alias::NAMES[0])).is_err());
    assert_eq!(b.fields, original);
    assert!(b.record.is_some());
    gpu_alias::change(&mut b, Some(gpu_alias::NAMES[1]))?;
    assert_eq!(
        b.fields["DeviceDesc"].as_ref().unwrap().data,
        gpu_alias::NAMES[1]
    );
    gpu_alias::change(&mut b, None)?;
    assert_eq!(b.fields, original);
    assert!(b.record.is_none());
    Ok(())
}

#[test]
#[cfg(feature = "fixture-tests")]
fn a_running_game_blocks_uninstall_before_any_changes() -> Result<()> {
    use std::{
        os::windows::process::CommandExt,
        process::{Command, Stdio},
    };
    let d = tempfile::tempdir()?;
    let exe = game(d.path());
    core::deploy(&exe, "native30", &["version.dll".into()])?;
    let running = d.path().join("owned-process-check.exe");
    let system = PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("System32/PING.EXE");
    fs::copy(system, &running)?;
    let mut child = Command::new(&running)
        .args(["127.0.0.1", "-n", "20"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(0x08000000)
        .spawn()?;
    std::thread::sleep(std::time::Duration::from_millis(100));
    let result = cleanup::clean(&exe);
    let _ = child.kill();
    let _ = child.wait();
    assert!(result.unwrap_err().to_string().contains("请先完全退出游戏"));
    assert!(d.path().join("version.dll").exists());
    assert!(d.path().join(core::OWN).join(core::MARKER).exists());
    cleanup::clean(&exe)?;
    assert!(!d.path().join("version.dll").exists());
    Ok(())
}

#[test]
fn same_named_process_in_another_directory_does_not_block_game() -> Result<()> {
    use std::{
        os::windows::process::CommandExt,
        process::{Command, Stdio},
    };
    let target = tempfile::tempdir()?;
    let elsewhere = tempfile::tempdir()?;
    let exe = game(target.path());
    let other = elsewhere.path().join("Game.exe");
    fs::copy(
        PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("System32/PING.EXE"),
        &other,
    )?;
    let mut child = Command::new(&other)
        .args(["127.0.0.1", "-n", "20"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(0x08000000)
        .spawn()?;
    std::thread::sleep(std::time::Duration::from_millis(100));
    let result = core::assert_stopped(&exe);
    let _ = child.kill();
    let _ = child.wait();
    result
}

#[test]
#[cfg(feature = "fixture-tests")]
fn invalid_record_and_interrupted_stage_are_handled_without_guessing() -> Result<()> {
    let d = tempfile::tempdir()?;
    let exe = game(d.path());
    core::deploy(&exe, "native30", &["version.dll".into()])?;
    let marker = d.path().join(core::OWN).join(core::MARKER);
    let mut record = core::read_json(&marker, 20000)?;
    record["proxies"] = serde_json::json!([]);
    core::atomic_json(&marker, &record)?;
    assert!(cleanup::clean(&exe).is_err());
    assert!(d.path().join("version.dll").exists());
    record["proxies"] = serde_json::json!(["version.dll"]);
    core::atomic_json(&marker, &record)?;
    let bytes = assets::bytes("payloads/native/version.dll")?;
    fs::write(
        d.path().join(core::OWN).join("version.dll.stage"),
        &bytes[..120],
    )?;
    cleanup::clean(&exe)?;
    assert!(!marker.exists());
    assert!(!d.path().join(core::OWN).exists());
    Ok(())
}

#[test]
fn per_game_lock_blocks_other_threads() -> Result<()> {
    let d = tempfile::tempdir()?;
    let p = d.path().to_path_buf();
    let _lock = win::game_lock(&p)?;
    let blocked = std::thread::spawn(move || win::game_lock(&p).is_err())
        .join()
        .unwrap();
    assert!(blocked);
    Ok(())
}

#[test]
fn cleanup_retains_large_unrelated_proxy() -> Result<()> {
    let d = tempfile::tempdir()?;
    let exe = game(d.path());
    let p = d.path().join("version.dll");
    let f = fs::File::create(&p)?;
    f.set_len(129 * 1024 * 1024)?;
    drop(f);
    cleanup::clean(&exe)?;
    assert_eq!(fs::metadata(&p)?.len(), 129 * 1024 * 1024);
    Ok(())
}

#[test]
fn directory_junctions_are_rejected_and_never_traversed() -> Result<()> {
    use std::{os::windows::process::CommandExt, process::Command};
    let d = tempfile::tempdir()?;
    let outside = tempfile::tempdir()?;
    let exe = game(outside.path());
    fs::write(outside.path().join("nvngx_dlssg.dll"), b"evidence")?;
    let link = d.path().join("junction");
    let status = Command::new("C:\\Windows\\System32\\cmd.exe")
        .args(["/d", "/c", "mklink", "/J"])
        .arg(&link)
        .arg(outside.path())
        .creation_flags(0x08000000)
        .output()?;
    assert!(
        status.status.success(),
        "{}",
        String::from_utf8_lossy(&status.stderr)
    );
    let check = core::no_links(&link.join("Game.exe"));
    let scan = scanner::scan(&[d.path().into()], &AtomicBool::new(false), |_, _, _| {});
    fs::remove_dir(&link)?;
    assert!(check.is_err());
    assert!(scan?.rows.is_empty());
    assert!(exe.exists());
    Ok(())
}

#[test]
fn rust_status_and_error_catalog_covers_all_four_translations() -> Result<()> {
    let catalog: BTreeMap<String, [String; 4]> =
        serde_json::from_str(include_str!("../rust/ui-translations.json"))?;
    for (i, code) in ["en", "ru", "ja", "ko"].iter().enumerate() {
        let translator = i18n::Translator::new(code);
        for (source, values) in &catalog {
            assert!(!values[i].trim().is_empty(), "{code}: {source}");
            assert_eq!(translator.t(source), values[i], "{code}: {source}");
        }
        let status = translator.t("已部署 NATIVE_X6_30 / version.dll / 配置已修改，可正常卸载");
        assert!(status.contains(&catalog["配置已修改，可正常卸载"][i]));
        assert!(status.contains("NATIVE_X6_30 / version.dll"));
    }
    Ok(())
}

#[test]
fn component_probe_distinguishes_super_resolution_and_frame_generation() {
    let dir = tempfile::tempdir().unwrap();
    let exe = game(dir.path());
    fs::write(dir.path().join("nvngx_dlss.dll"), b"unloaded fixture").unwrap();
    let p = scanner::inspect(&exe);
    assert!(p.super_resolution && !p.frame_generation && !p.anti_cheat);
    let plugin = dir.path().join("plugins");
    fs::create_dir(&plugin).unwrap();
    fs::write(plugin.join("sl.dlss_g.dll"), b"unloaded fixture").unwrap();
    fs::create_dir(dir.path().join("EasyAntiCheat")).unwrap();
    let p = scanner::inspect(&exe);
    assert!(p.super_resolution && p.frame_generation && p.anti_cheat);
}

#[test]
fn welcome_only_for_new_users_and_dismissal_persists() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let mut state = preferences::load(dir.path())?;
    assert_eq!(state["welcome_pending"], true);
    state["welcome_pending"] = serde_json::json!(false);
    let store = preferences::Store::new(dir.path().into());
    store.save(state)?;
    store.finish()?;
    assert_eq!(preferences::load(dir.path())?["welcome_pending"], false);
    core::atomic_json(
        &dir.path().join("games.json"),
        &serde_json::json!({"schema":3,"games":[],"language":"ja"}),
    )?;
    let old = preferences::load(dir.path())?;
    assert!(old.get("welcome_pending").is_none());
    assert_eq!(old["language"], "ja");
    Ok(())
}
