//! Filesystem-only regressions. The tiny PE fixture is parsed, never executed.
use anyhow::Result;
use rtx_fg_manager::{anr, cleanup, cloud, core, deployment, scanner::Game};
use std::fs;

fn vulkan_fixture() -> Vec<u8> {
    // One PE32+ section, with a module name and the two Vulkan dispatch exports.
    let mut bytes = vec![0; 0x600];
    fn put(bytes: &mut [u8], offset: usize, value: &[u8]) {
        bytes[offset..offset + value.len()].copy_from_slice(value);
    }
    let pe = 0x80;
    let opt = pe + 24;
    let section = opt + 240;
    put(&mut bytes, 0, b"MZ");
    put(&mut bytes, 0x3c, &(pe as u32).to_le_bytes());
    put(&mut bytes, pe, b"PE\0\0");
    for (offset, word) in [
        (pe + 4, 0x8664u16),
        (pe + 6, 1),
        (pe + 20, 240),
        (pe + 22, 0x2022),
        (opt, 0x20b),
    ] {
        put(&mut bytes, offset, &word.to_le_bytes());
    }
    put(&mut bytes, section, b".edata");
    for (offset, word) in [
        (opt + 108, 16u32),
        (opt + 112, 0x1000),
        (opt + 116, 0x200),
        (section + 8, 0x400),
        (section + 12, 0x1000),
        (section + 16, 0x400),
        (section + 20, 0x200),
        (0x20c, 0x1060),
        (0x210, 1),
        (0x214, 2),
        (0x218, 2),
        (0x21c, 0x1040),
        (0x220, 0x1048),
        (0x224, 0x1050),
        (0x240, 0x1200),
        (0x244, 0x1201),
        (0x248, 0x1080),
        (0x24c, 0x10a0),
    ] {
        put(&mut bytes, offset, &word.to_le_bytes());
    }
    put(&mut bytes, 0x252, &1u16.to_le_bytes());
    put(&mut bytes, 0x260, b"vulkan-1.dll\0");
    put(&mut bytes, 0x280, b"vkGetInstanceProcAddr\0");
    put(&mut bytes, 0x2a0, b"vkGetDeviceProcAddr\0");
    bytes
}

#[test]
fn legacy_game_status_and_cleanup_respect_file_ownership() -> Result<()> {
    let root = tempfile::tempdir()?;
    let folder = root.path().join("Stormgate/Binaries/Win64");
    let provider = folder.join(anr::PROVIDER_DIR);
    fs::create_dir_all(&provider)?;
    let exe = folder.join("Stormgate-Win64-Shipping.exe");
    fs::write(&exe, b"game")?;
    let target = core::DeploymentTarget::for_game(&exe);
    let loader = provider.join("anr-vkloader.dll");
    let game_loader = folder.join("vulkan-1.dll");
    let bytes = vulkan_fixture();
    fs::write(&game_loader, &bytes)?;
    fs::write(&loader, &bytes)?;
    let game = Game {
        exe: exe.to_string_lossy().into_owned(),
        ..Game::default()
    };
    let snapshot = deployment::inspect(&game, &cloud::bundled());
    assert_eq!(snapshot.status, anr::RESIDUE_STATUS);
    assert_eq!(snapshot.installed, 0);
    assert!(snapshot.issues()[0].kind().can_retry_cleanup());
    assert!(snapshot.issues()[0].detail().contains("anr-vkloader.dll"));

    // An independent host or any extra file invalidates markerless ownership.
    for foreign in [
        folder.join("OptiScaler.ini"),
        provider.join("capture.raw"),
        folder.join("OptiScaler/player-mod.ini"),
    ] {
        fs::write(&foreign, b"mine")?;
        assert!(anr::preflight(&target, false).is_err());
        assert_eq!(
            deployment::IssueKind::from_status(&core::status_at(&target)),
            Some(deployment::IssueKind::External)
        );
        assert!(cleanup::retry_pending_at(&target)?.complete);
        assert_eq!(fs::read(&foreign)?, b"mine");
        assert_eq!(fs::read(&loader)?, bytes);
        fs::remove_file(foreign)?;
    }
    fs::write(&loader, b"not a Vulkan loader")?;
    assert!(anr::preflight(&target, false).is_err());
    assert!(cleanup::retry_pending_at(&target)?.complete);
    assert_eq!(fs::read(&loader)?, b"not a Vulkan loader");

    let other = root.path().join("AnotherGame").join(anr::PROVIDER_DIR);
    fs::create_dir_all(&other)?;
    fs::write(other.join("anr-vkloader.dll"), &bytes)?;
    fs::write(&loader, &bytes)?;
    assert!(cleanup::retry_pending_at(&target)?.complete);
    assert!(!folder.join("OptiScaler").exists());
    assert_eq!(core::status_at(&target), "未部署");
    assert_eq!(fs::read(&game_loader)?, bytes);
    assert_eq!(fs::read(other.join("anr-vkloader.dll"))?, bytes);
    assert_eq!(fs::read(exe)?, b"game");
    Ok(())
}

#[cfg(windows)]
#[test]
fn locked_old_loader_remains_retryable_until_closed() -> Result<()> {
    use std::os::windows::fs::OpenOptionsExt;
    let root = tempfile::tempdir()?;
    let exe = root.path().join("Game.exe");
    fs::write(&exe, b"game")?;
    let target = core::DeploymentTarget::for_game(&exe);
    let provider = root.path().join(anr::PROVIDER_DIR);
    fs::create_dir_all(&provider)?;
    let path = provider.join("anr-vkloader.dll");
    fs::write(&path, vulkan_fixture())?;
    let held = fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(&path)?;
    let err = cleanup::retry_pending_at(&target).unwrap_err();
    let reason = format!("{err:#}");
    assert!(
        reason.contains("anr-vkloader.dll")
            || reason.contains(&format!("PID {}", std::process::id())),
        "locked loader should identify its path or holder: {reason}"
    );
    assert_eq!(core::status_at(&target), anr::RESIDUE_STATUS);
    drop(held);
    assert!(cleanup::retry_pending_at(&target)?.complete);
    assert!(!path.exists());
    Ok(())
}

#[cfg(windows)]
#[test]
fn a_junction_to_another_games_provider_is_never_cleaned() -> Result<()> {
    use std::{os::windows::process::CommandExt, process::Command};
    let owner = tempfile::tempdir()?;
    let other = tempfile::tempdir()?;
    let provider = other.path().join(anr::PROVIDER_DIR);
    fs::create_dir_all(&provider)?;
    let bytes = vulkan_fixture();
    let outside = provider.join("anr-vkloader.dll");
    fs::write(&outside, &bytes)?;
    let exe = owner.path().join("Game.exe");
    fs::write(&exe, b"game")?;
    let target = core::DeploymentTarget::for_game(&exe);
    let link = owner.path().join("OptiScaler");
    let status = Command::new(std::env::var_os("ComSpec").expect("Windows command processor"))
        .args(["/d", "/c", "mklink", "/J"])
        .arg(&link)
        .arg(other.path().join("OptiScaler"))
        .creation_flags(0x08000000)
        .output()?;
    assert!(
        status.status.success(),
        "junction {link:?}: {:?} / {:?}",
        String::from_utf8_lossy(&status.stdout),
        String::from_utf8_lossy(&status.stderr)
    );
    assert!(anr::preflight(&target, false).is_err());
    assert!(cleanup::retry_pending_at(&target)?.complete);
    assert_eq!(fs::read(&outside)?, bytes);
    fs::remove_dir(link)?;
    Ok(())
}
