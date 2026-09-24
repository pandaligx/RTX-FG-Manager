use anyhow::Result;
#[cfg(feature = "fixture-tests")]
use rtx_fg_manager::{cleanup, core};
use rtx_fg_manager::{cloud, delta, diagnostics, presets};
#[cfg(feature = "fixture-tests")]
use std::os::windows::fs::OpenOptionsExt;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};
const SCHEME: &str = "rtxfg-0.3.5-dx12-vulkan";
#[test]
#[cfg(feature = "fixture-tests")]
fn new_cloud_capability_is_optional_and_generic_packages_stay_off() -> Result<()> {
    let folder = root().join("tests/fixtures/catalog421");
    let bytes = fs::read(folder.join("catalog.json"))?;
    let index = fs::read(folder.join("index.json"))?;
    let catalog = serde_json::from_slice::<cloud::CompactCatalog>(&bytes)?.expand(&index)?;
    assert_eq!(catalog.schemes().len(), 4);
    assert_eq!(catalog.packages.len(), 19);
    assert_eq!(catalog.default_scheme, "upstream-0.3.5-310-9");
    assert!(
        catalog.scheme_policies[SCHEME]
            .capabilities
            .contains(delta::CAPABILITY)
    );
    assert!(
        catalog.scheme_policies[&catalog.default_scheme]
            .capabilities
            .is_empty()
    );
    for package in catalog.packages.iter().filter(|p| p.scheme_id == SCHEME) {
        let files = cloud::unpack(
            package,
            &fs::read(
                root()
                    .join("tests/fixtures/runtime/packages")
                    .join(&package.archive),
            )?,
        )?;
        let (ini, _) = diagnostics::decode_ini(&files[core::INI])?;
        assert_eq!(
            diagnostics::ini_value(&ini, "Compatibility", "DeltaForcePrivateStreamline")?
                .as_deref(),
            Some("0")
        );
        assert_eq!(
            diagnostics::ini_value(&ini, "Compatibility", "DeltaForceGeneratedFrames")?.as_deref(),
            Some("0")
        );
        assert!(diagnostics::ini_value(&ini, "Compatibility", delta::ID_KEY)?.is_none());
    }
    let mut old: serde_json::Value = serde_json::from_slice(&bytes)?;
    for s in old["schemes"].as_array_mut().unwrap() {
        s.as_object_mut().unwrap().remove("capabilities");
    }
    let compatible = serde_json::from_value::<cloud::CompactCatalog>(old)?.expand(&index)?;
    assert!(compatible.scheme_policies[SCHEME].capabilities.is_empty());
    Ok(())
}
#[test]
fn new_help_is_translated_in_all_supported_languages() -> Result<()> {
    let help: Vec<[String; 2]> =
        serde_json::from_slice(&fs::read(root().join("rust/assets/help.json"))?)?;
    for lang in ["en", "ru", "ja", "ko"] {
        let t = rtx_fg_manager::i18n::Translator::new(lang);
        for row in &help {
            for source in row {
                for line in source.split('\n') {
                    assert_ne!(t.t(line), line, "{lang}: {line}");
                }
            }
        }
        for source in [
            "跟随游戏",
            "三角洲专项：默认4X；可选跟随游戏、2X、3X、4X。",
            "补丁已移除，缓存待清理",
            "三角洲专项最高支持4X，原5X/6X已调整为4X",
        ] {
            assert_ne!(t.t(source), source);
        }
    }
    Ok(())
}
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}
fn context(exe: &Path) -> presets::Context {
    let c = cloud::bundled();
    let mut p = c.scheme_policies[&c.default_scheme].clone();
    p.capabilities.insert(delta::CAPABILITY.into());
    presets::Context::new(SCHEME, &p, exe)
}
fn game(base: &Path) -> Result<PathBuf> {
    let p = base
        .join("游戏 [中文]/DeltaForce/Binaries/Win64")
        .join(delta::GAME);
    fs::create_dir_all(p.parent().unwrap())?;
    let mut b = vec![0; 1024];
    b[..2].copy_from_slice(b"MZ");
    b[60..64].copy_from_slice(&128u32.to_le_bytes());
    b[128..132].copy_from_slice(b"PE\0\0");
    b[132..134].copy_from_slice(&0x8664u16.to_le_bytes());
    b[148..150].copy_from_slice(&240u16.to_le_bytes());
    b[150..152].copy_from_slice(&2u16.to_le_bytes());
    b[152..154].copy_from_slice(&0x20bu16.to_le_bytes());
    fs::write(&p, b)?;
    Ok(p)
}
#[cfg(feature = "fixture-tests")]
fn runtime(base: &Path, exe: &Path) -> Result<PathBuf> {
    let id = delta::cache_id(exe);
    delta::register_at(base, exe, &id)?;
    let out = base.join("games").join(id).join(delta::REVISION);
    fs::create_dir_all(&out)?;
    for file in fs::read_dir(root().join("tests/fixtures/runtime/streamline27"))? {
        let p = file?.path();
        if p.extension().is_some_and(|x| x == "dll") {
            fs::copy(&p, out.join(p.file_name().unwrap()))?;
        }
    }
    Ok(out)
}
#[test]
fn exact_scope_and_all_multiplier_mappings() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let exe = game(dir.path())?;
    let c = context(&exe);
    assert!(c.delta);
    for n in 0..=3 {
        let values = BTreeMap::from([("max_generated_frames".into(), n.to_string())]);
        let configured = c.configure(b"; keep\r\n[Custom]\r\nHello=world\r\n", &values)?;
        let text = String::from_utf8(configured)?;
        for (section, key, want) in [
            ("FrameGeneration", "MaxGeneratedFrames", n.to_string()),
            ("Compatibility", "DeltaForceGeneratedFrames", n.to_string()),
            (
                "Compatibility",
                "DeltaForcePrivateStreamline",
                u8::from(n >= 2).to_string(),
            ),
        ] {
            assert_eq!(
                diagnostics::ini_value(&text, section, key)?.as_deref(),
                Some(want.as_str())
            );
        }
        assert!(text.contains("; keep") && text.contains("Hello=world"));
    }
    for other in [
        exe.with_file_name("Other.exe"),
        dir.path().join(delta::GAME),
    ] {
        let c = context(&other);
        assert!(!c.delta);
        let bytes = c.configure(
            b"[Compatibility]\nDeltaForcePrivateStreamline=1\n",
            &presets::defaults("upstream035", &BTreeMap::new()),
        )?;
        assert_eq!(
            diagnostics::ini_value(
                &String::from_utf8(bytes)?,
                "Compatibility",
                "DeltaForcePrivateStreamline"
            )?
            .as_deref(),
            Some("0")
        );
    }
    let cat = cloud::bundled();
    let github = presets::Context::new(
        &cat.default_scheme,
        &cat.scheme_policies[&cat.default_scheme],
        &exe,
    );
    assert!(!github.delta);
    assert!(!String::from_utf8(github.configure(b"", &BTreeMap::new())?)?.contains("DeltaForce"));
    assert_eq!(
        c.parameters()
            .iter()
            .find(|p| p.key == "max_generated_frames")
            .unwrap()
            .choices
            .len(),
        4
    );
    Ok(())
}
#[test]
fn managed_merge_preserves_utf16_comments_and_unknown_sections() -> Result<()> {
    let current = "; 注释\r\n[Compatibility]\r\nDeltaForcePrivateStreamline=0 ; retain\r\nUserValue=42\r\n[Other]\r\nValue=ok\r\n";
    let bytes: Vec<u8> = [0xff, 0xfe]
        .into_iter()
        .chain(current.encode_utf16().flat_map(u16::to_le_bytes))
        .collect();
    let ctx = context(Path::new(
        "C:/Games/DeltaForce/Binaries/Win64/DeltaForceClient-Win64-Shipping.exe",
    ));
    let desired = ctx.configure(
        b"",
        &BTreeMap::from([("max_generated_frames".into(), "2".into())]),
    )?;
    let merged = presets::merge_context(&bytes, &desired, "upstream_sm86", Some(&ctx))?;
    assert_eq!(&merged[..2], &[0xff, 0xfe]);
    let (s, _) = diagnostics::decode_ini(&merged)?;
    assert!(
        s.contains("; 注释")
            && s.contains("UserValue=42")
            && s.contains("Value=ok")
            && s.contains("; retain")
    );
    assert_eq!(
        diagnostics::ini_value(&s, "Compatibility", "DeltaForceGeneratedFrames")?.as_deref(),
        Some("2")
    );
    Ok(())
}
#[test]
#[cfg(feature = "fixture-tests")]
fn cache_cleanup_is_scoped_retryable_and_does_not_delete_unknowns() -> Result<()> {
    let d = tempfile::tempdir()?;
    let exe = game(d.path())?;
    let other = game(&d.path().join("other"))?;
    let base = d.path().join("Hidden/AppData/Local/RTXFG-Delta4X");
    let cache = runtime(&base, &exe)?;
    let other_cache = runtime(&base, &other)?;
    let keep = d.path().join("original/sl.interposer.dll");
    fs::create_dir_all(keep.parent().unwrap())?;
    fs::write(&keep, b"game sentinel")?;
    fs::write(cache.join("unknown.dll"), b"leave me")?;
    fs::copy(
        cache.join("sl.common.dll"),
        cache.join("sl.common.dll.tmp-1-2-3"),
    )?;
    let held = fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(cache.join("sl.common.dll"))?;
    let ids = [delta::cache_id(&exe)];
    let r = delta::clean_at(&base, &exe, &ids, false);
    assert!(!r.pending.is_empty());
    assert_eq!(r.files, 0);
    assert!(cache.join("sl.interposer.dll").is_file());
    drop(held);
    let r = delta::clean_at(&base, &exe, &ids, false);
    assert_eq!(r.files, 8);
    assert!(!r.pending.is_empty());
    assert!(other_cache.join("sl.common.dll").is_file());
    assert_eq!(fs::read(&keep)?, b"game sentinel");
    assert_eq!(fs::read(cache.join("unknown.dll"))?, b"leave me");
    fs::remove_file(cache.join("unknown.dll"))?;
    assert!(delta::clean_at(&base, &exe, &ids, false).pending.is_empty());
    assert!(!cache.exists());
    assert!(delta::clean_at(&base, &exe, &ids, false).pending.is_empty());
    let invalid = delta::clean_at(&base, &exe, &[delta::cache_id(&other)], false);
    assert!(!invalid.pending.is_empty());
    assert!(other_cache.exists());
    for id in [
        "../outside",
        "C:/game",
        "FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF",
        "f",
    ] {
        assert!(!delta::valid_id(id));
    }
    Ok(())
}
#[test]
#[cfg(feature = "fixture-tests")]
fn reparse_cache_and_legacy_backups_are_preserved() -> Result<()> {
    let d = tempfile::tempdir()?;
    let exe = game(d.path())?;
    let base = d.path().join("cache");
    let cache = runtime(&base, &exe)?;
    let outside = d.path().join("outside");
    fs::create_dir_all(&outside)?;
    fs::write(outside.join("sl.common.dll"), b"original")?;
    let link = cache.join("linked");
    std::os::windows::fs::symlink_dir(&outside, &link)?;
    assert!(
        !delta::clean_at(&base, &exe, &[delta::cache_id(&exe)], false)
            .pending
            .is_empty()
    );
    assert_eq!(fs::read(outside.join("sl.common.dll"))?, b"original");
    // TempDir must never follow the reparse point during its own cleanup.
    fs::remove_dir(&link)?;
    let backup = base.join("test3-backup");
    fs::create_dir_all(&backup)?;
    fs::write(backup.join("sl.common.dll"), b"backup")?;
    delta::clean_at(&base, &exe, &[], true);
    assert_eq!(fs::read(backup.join("sl.common.dll"))?, b"backup");
    Ok(())
}
#[test]
#[cfg(feature = "fixture-tests")]
fn managed_uninstall_retains_retry_record_and_ignores_edited_id() -> Result<()> {
    if std::env::var_os("RTXFG_DELTA_TEST_CHILD").is_none() {
        let data = tempfile::tempdir()?;
        let output = std::process::Command::new(std::env::current_exe()?)
            .args([
                "--exact",
                "managed_uninstall_retains_retry_record_and_ignores_edited_id",
                "--nocapture",
            ])
            .env("RTXFG_DELTA_TEST_CHILD", "1")
            .env("LOCALAPPDATA", data.path())
            .output()?;
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return Ok(());
    }
    let d = tempfile::tempdir()?;
    let exe = game(d.path())?;
    let dir = exe.parent().unwrap();
    let ctx = context(&exe);
    let dll = fs::read(root().join("tests/fixtures/runtime/extended/version.dll"))?;
    let ini = ctx.configure(
        b"[Logging]\nDirectory=dlssg_sm86\\logs\n",
        &presets::defaults("upstream035", &BTreeMap::new()),
    )?;
    let data = BTreeMap::from([("version.dll".into(), dll.clone()), (core::INI.into(), ini)]);
    // Manual identical package adoption must retain comments.
    fs::write(dir.join("version.dll"), dll)?;
    fs::write(dir.join(core::INI), b"; user's INI\n[User]\nKeep=yes\n")?;
    core::deploy_prepared_context(
        &exe,
        "upstream_sm86",
        &["version.dll".into()],
        None,
        data,
        Some("0.3.5"),
        Some(&ctx),
    )?;
    assert!(fs::read_to_string(dir.join(core::INI))?.contains("; user's INI"));
    let cache = runtime(&delta::root()?, &exe)?;
    let other = game(&d.path().join("other"))?;
    let other_cache = runtime(&delta::root()?, &other)?;
    let edited = format!(
        "[Compatibility]\nDeltaForceRuntimeId={}\n[logging]\ndirectory=自定义日志\n",
        delta::cache_id(&other)
    );
    let edited: Vec<u8> = [0xff, 0xfe]
        .into_iter()
        .chain(edited.encode_utf16().flat_map(u16::to_le_bytes))
        .collect();
    fs::write(dir.join(core::INI), edited)?;
    fs::create_dir(dir.join("自定义日志"))?;
    fs::write(
        dir.join("自定义日志/native_123.jsonl"),
        b"{\"event\":\"configuration\"}\n",
    )?;
    fs::write(dir.join("自定义日志/notes.txt"), b"user notes")?;
    fs::write(dir.join("dxgi.dll"), b"original game dll")?;
    let held = fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(cache.join("sl.common.dll"))?;
    assert!(cleanup::clean(&exe)?.starts_with("补丁已移除，缓存待清理"));
    assert!(core::record(dir)?.unwrap().cache_pending);
    assert!(!dir.join("version.dll").exists());
    assert!(!dir.join("自定义日志/native_123.jsonl").exists());
    assert_eq!(fs::read(dir.join("自定义日志/notes.txt"))?, b"user notes");
    assert!(cache.join("sl.common.dll").exists());
    drop(held);
    cleanup::clean(&exe)?;
    cleanup::clean(&exe)?;
    assert!(core::record(dir)?.is_none());
    assert!(!cache.exists());
    assert!(other_cache.exists());
    assert_eq!(fs::read(dir.join("dxgi.dll"))?, b"original game dll");
    assert!(exe.exists());
    Ok(())
}
