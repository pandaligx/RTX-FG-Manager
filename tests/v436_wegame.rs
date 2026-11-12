//! Synthetic WeGame installation metadata; no game process or DLL is loaded.
use anyhow::Result;
use rtx_fg_manager::{core, scanner};
use std::{fs, path::Path, sync::atomic::AtomicBool};

fn executable(path: &Path, x64: bool) -> Result<()> {
    fs::create_dir_all(path.parent().unwrap())?;
    let mut pe = vec![0; 512];
    pe[..2].copy_from_slice(b"MZ");
    pe[60..64].copy_from_slice(&128u32.to_le_bytes());
    pe[128..132].copy_from_slice(b"PE\0\0");
    pe[132..134].copy_from_slice(&(if x64 { 0x8664u16 } else { 0x14c }).to_le_bytes());
    pe[148..150].copy_from_slice(&240u16.to_le_bytes());
    pe[150..152].copy_from_slice(&2u16.to_le_bytes());
    pe[152..154].copy_from_slice(&(if x64 { 0x20bu16 } else { 0x10b }).to_le_bytes());
    fs::write(path, pe)?;
    Ok(())
}

fn fixture(root: &Path, game_id: u64, x64: bool) -> Result<()> {
    executable(&root.join("Releasephysx27_64/GameApp.exe"), x64)?;
    executable(&root.join("QQSpeedLauncher.exe"), false)?;
    core::atomic_json(
        &root.join("rail_files/rail_game_identify.json"),
        &serde_json::json!({"game_id": game_id}),
    )
}

#[test]
fn wegame_qqspeed_without_dlss_finds_only_the_verified_x64_client() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let root = temp.path().join("Custom installation name");
    fixture(&root, 17, true)?;
    executable(&root.join("Releasephysx27/GameApp.exe"), false)?;
    executable(&root.join("Releasephysx27_64/IIPS/IIPSHostApp.exe"), true)?;
    executable(
        &root.join("Releasephysx27_64/FixDownloadHelper/FixDownload.exe"),
        true,
    )?;
    executable(&root.join("Other/GameApp.exe"), true)?;
    let report = scanner::scan(
        std::slice::from_ref(&root),
        &AtomicBool::new(false),
        |_, _, _| {},
    )?;
    assert_eq!(report.rows.len(), 1);
    let game = &report.rows[0];
    let expected = root.join("Releasephysx27_64/GameApp.exe");
    assert_eq!(core::key(Path::new(&game.exe)), core::key(&expected));
    assert_eq!(core::key(Path::new(&game.root)), core::key(&root));
    assert_eq!(game.targets.as_slice(), std::slice::from_ref(&game.exe));
    assert_eq!(game.title, "QQ飞车");
    assert!(!scanner::inspect(&expected).super_resolution);
    assert_eq!(core::key(&scanner::game_root(&expected)), core::key(&root));
    assert!(!expected.parent().unwrap().join(core::OWN).exists());
    Ok(())
}

#[test]
fn wegame_id_alone_wrong_architecture_and_missing_launcher_do_not_admit_gameapp() -> Result<()> {
    let temp = tempfile::tempdir()?;
    for (name, id, x64, launcher) in [
        ("wrong-id", 18, true, true),
        ("wrong-architecture", 17, false, true),
        ("missing-launcher", 17, true, false),
    ] {
        let root = temp.path().join(name);
        fixture(&root, id, x64)?;
        if !launcher {
            fs::remove_file(root.join("QQSpeedLauncher.exe"))?;
        }
        let report = scanner::scan(&[root], &AtomicBool::new(false), |_, _, _| {})?;
        assert!(report.rows.is_empty(), "{name}: {:?}", report.rows);
    }
    Ok(())
}
