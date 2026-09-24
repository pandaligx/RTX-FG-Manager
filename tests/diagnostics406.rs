use anyhow::Result;
#[cfg(feature = "fixture-tests")]
use rtx_fg_manager::{cleanup, core};
use rtx_fg_manager::{
    diagnostics::{self, edit_ini},
    selftest::{self, Adapter, Request},
};
use std::{fs, path::Path, sync::atomic::AtomicBool};

#[cfg(feature = "fixture-tests")]
fn fake_game(dir: &Path) -> Result<std::path::PathBuf> {
    let mut b = vec![0; 1024];
    b[..2].copy_from_slice(b"MZ");
    b[60..64].copy_from_slice(&128u32.to_le_bytes());
    b[128..132].copy_from_slice(b"PE\0\0");
    b[132..134].copy_from_slice(&0x8664u16.to_le_bytes());
    b[148..150].copy_from_slice(&240u16.to_le_bytes());
    b[150..152].copy_from_slice(&2u16.to_le_bytes());
    b[152..154].copy_from_slice(&0x20bu16.to_le_bytes());
    let p = dir.join("诊断游戏.exe");
    fs::write(&p, b)?;
    Ok(p)
}
#[test]
#[cfg(feature = "fixture-tests")]
fn per_game_log_apply_preserves_ini_reinstall_and_cleanup() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let game = fake_game(dir.path())?;
    assert_eq!(diagnostics::game_level(&game)?, None);
    assert!(!diagnostics::apply_level(&game, 2)?);
    core::deploy_with_level(&game, "native20", &["version.dll".into()], Some(1))?;
    let ini = dir.path().join(core::INI);
    let original = fs::read(&ini)?;
    assert_eq!(diagnostics::game_level(&game)?, Some(1));
    assert!(diagnostics::apply_level(&game, 3)?);
    assert_eq!(
        fs::read(&ini)?,
        edit_ini(&original, "Logging", "Level", "3")?
    );
    assert_eq!(diagnostics::game_level(&game)?, Some(3));
    assert!(
        core::deploy_with_level(&game, "native20", &["version.dll".into()], Some(3))?
            .contains("无需重复")
    );
    cleanup::clean(&game)?;
    assert!(!ini.exists());
    assert!(game.exists());
    fs::write(&ini, b"[Logging]\nLevel=2\n")?;
    assert!(diagnostics::apply_level(&game, 3).is_err());
    assert_eq!(fs::read(&ini)?, b"[Logging]\nLevel=2\n");
    Ok(())
}
#[test]
fn zip_keeps_tail_and_records_missing_files() -> Result<()> {
    use std::io::{Read, Seek, SeekFrom, Write};
    let dir = tempfile::tempdir()?;
    let log = dir.path().join("large.log");
    let mut f = fs::File::create(&log)?;
    f.set_len(33 * 1024 * 1024)?;
    f.seek(SeekFrom::End(-4))?;
    f.write_all(b"TAIL")?;
    drop(f);
    let req = diagnostics::ExportRequest {
        game: None,
        selftest: None,
        attachments: vec![log, dir.path().join("missing.log")],
        manager_log: vec!["test".into()],
    };
    let target = dir.path().join("诊断.zip");
    diagnostics::export(&target, &req, &AtomicBool::new(false), |_| {})?;
    let mut zip = zip::ZipArchive::new(fs::File::open(target)?)?;
    let mut manifest = String::new();
    zip.by_name("manifest.json")?
        .read_to_string(&mut manifest)?;
    let value: serde_json::Value = serde_json::from_str(&manifest)?;
    let files = value["files"].as_array().unwrap();
    assert_eq!(files.len(), 2);
    let large = files.iter().find(|v| v["truncated"] == true).unwrap();
    assert_eq!(large["exported_bytes"], 32 * 1024 * 1024);
    let mut bytes = Vec::new();
    zip.by_name(large["name"].as_str().unwrap())?
        .read_to_end(&mut bytes)?;
    assert!(bytes.ends_with(b"TAIL"));
    assert!(
        files
            .iter()
            .any(|v| v["note"].as_str().is_some_and(|s| !s.is_empty()))
    );
    Ok(())
}

#[test]
fn ini_preserves_other_settings_comments_and_encodings() -> Result<()> {
    let source = "; 参数\r\n[Logging]\r\nLevel = 1 ; keep\r\nDirectory=logs\r\n[Runtime]\r\nKernelImage=ptx\r\n";
    let edited = edit_ini(source.as_bytes(), "Logging", "Level", "3")?;
    assert_eq!(
        String::from_utf8(edited)?,
        source.replace("Level = 1", "Level =3")
    );
    for le in [true, false] {
        let mut bytes = if le { vec![255, 254] } else { vec![254, 255] };
        for c in source.encode_utf16() {
            bytes.extend(if le { c.to_le_bytes() } else { c.to_be_bytes() })
        }
        let edited = edit_ini(&bytes, "Logging", "Level", "2")?;
        assert_eq!(&edited[..2], &bytes[..2]);
        assert!(
            diagnostics::decode_ini(&edited)?
                .0
                .contains("Level =2 ; keep\r\n")
        );
        assert!(
            diagnostics::decode_ini(&edited)?
                .0
                .ends_with("KernelImage=ptx\r\n")
        );
    }
    let bom = [&[239, 187, 191][..], source.as_bytes()].concat();
    assert!(edit_ini(&bom, "Logging", "Level", "0")?.starts_with(&[239, 187, 191]));
    assert!(edit_ini(b"[Logging]\nLevel=1\nlevel=2", "Logging", "Level", "3").is_err());
    assert!(edit_ini(b"[Logging]\n[Logging]\n", "Logging", "Level", "3").is_err());
    assert!(edit_ini(b"", "Logging", "Level", "3\nOther=1").is_err());
    assert_eq!(
        edit_ini(b"[Runtime]\nKey=1", "Logging", "Level", "1")?,
        b"[Runtime]\nKey=1\n[Logging]\nLevel=1\n"
    );
    Ok(())
}
#[test]
fn event_matching_requires_exact_exe_not_filename() {
    let xml = r#"<Events><Event><EventData><Data Name="AppPath">D:\游戏\A&amp;B\Game.exe</Data></EventData></Event><Event><Data>D:\Other\Game.exe</Data></Event><Event><Data>Game.exe</Data></Event><Event><Data>D:\游戏\A&amp;B\Game.exe.bak</Data></Event></Events>"#;
    let matches = diagnostics::matching_events(xml, Path::new(r"D:\游戏\A&B\Game.exe"));
    assert_eq!(matches.len(), 1);
    assert!(matches[0].contains("AppPath"));
}
fn request(major: i32, minor: i32, full: bool, forward: bool) -> Request {
    Request {
        adapter: Adapter {
            luid: 1,
            name: "Spoofed NVIDIA RTX 5090".into(),
            vendor: 0x10de,
            device: 1,
            vram: 0,
            driver: "test".into(),
            major,
            minor,
        },
        complete: full,
        forward_sm75: forward,
        game: None,
    }
}
#[test]
fn routes_use_actual_compute_capability_and_cases_are_serial_units() {
    #[cfg(not(feature = "native-probes"))]
    assert!(
        selftest::adapters()
            .unwrap_err()
            .to_string()
            .contains("not included")
    );
    let turing = request(7, 5, false, true);
    let standard = selftest::cases(&turing, &["version.dll".into()]);
    assert_eq!(standard.len(), 7);
    assert!(standard.iter().all(|c| c.route == "SM75"));
    assert_eq!(
        selftest::cases(&request(8, 6, true, false), &["version.dll".into(); 1]).len(),
        14
    );
    assert_eq!(
        selftest::cases(&request(8, 9, true, true), &["version.dll".into()]).len(),
        28
    );
    assert!(selftest::cases(&request(0, 0, true, true), &["version.dll".into()]).is_empty());
    let mut game = request(8, 6, true, true);
    game.game = Some("Game.exe".into());
    assert_eq!(selftest::cases(&game, &["version.dll".into()]).len(), 14);
}
#[test]
fn selftest_cannot_pass_on_crash_timeout_cancel_or_bad_output() {
    for (code, trace, timeout, cancel, expected) in [
        (Some(0), "PASS output\n", false, false, "passed"),
        (Some(0), "STAGE=evaluate\n", false, false, "failed"),
        (
            Some(0),
            "FAIL output mismatch\nPASS output",
            false,
            false,
            "failed",
        ),
        (
            Some(0xc0000005u32 as i32),
            "PASS partial",
            false,
            false,
            "failed",
        ),
        (Some(0), "PASS output", true, false, "failed"),
        (Some(0), "PASS output", false, true, "cancelled"),
        (
            Some(77),
            "SKIP system Vulkan loader unavailable",
            false,
            false,
            "skipped",
        ),
        (Some(77), "", false, false, "failed"),
        (None, "FAIL device lost VkResult=-4", false, false, "failed"),
    ] {
        assert_eq!(selftest::classify(code, trace, timeout, cancel).0, expected);
    }
}
#[test]
fn zip_cancellation_and_source_overwrite_are_rejected() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let log = dir.path().join("用户.log");
    fs::write(&log, "log evidence")?;
    let req = diagnostics::ExportRequest {
        game: None,
        selftest: None,
        attachments: vec![log.clone()],
        manager_log: vec![],
    };
    let target = dir.path().join("result.zip");
    fs::write(&target, b"previous export")?;
    assert!(diagnostics::export(&target, &req, &AtomicBool::new(true), |_| {}).is_err());
    assert_eq!(fs::read(&target)?, b"previous export");
    assert!(diagnostics::export(&log, &req, &AtomicBool::new(false), |_| {}).is_err());
    assert_eq!(fs::read_to_string(&log)?, "log evidence");
    Ok(())
}

#[test]
fn full_report_is_kept_even_when_log_count_exceeds_cap() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let root = dir.path().join("selftest");
    fs::create_dir(&root)?;
    for name in ["report.json", "files.json", "events.jsonl"] {
        fs::write(root.join(name), b"{}")?;
    }
    for i in 0..270 {
        fs::write(root.join(format!("{i:03}.log")), b"test\n")?;
    }
    let req = diagnostics::ExportRequest {
        game: None,
        selftest: Some(root),
        attachments: vec![],
        manager_log: vec![],
    };
    let path = dir.path().join("report.zip");
    diagnostics::export(&path, &req, &AtomicBool::new(false), |_| {})?;
    let z = zip::ZipArchive::new(fs::File::open(path)?)?;
    assert!(z.len() <= 256);
    for name in ["report.json", "files.json", "events.jsonl"] {
        assert!(z.file_names().any(|n| n.ends_with(name)), "missing {name}");
    }
    Ok(())
}
