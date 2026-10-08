//! Grouped process guards and scanner role regressions. Only the test binary's
//! sleeper branch is executed; synthetic PE fixtures never run as games.
use anyhow::Result;
use rtx_fg_manager::{cleanup, core, presets, scanner, win};
use std::{
    collections::BTreeMap,
    fs,
    os::windows::process::CommandExt,
    path::Path,
    process::{Child, Command, Stdio},
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};

fn pe(dll: bool) -> Vec<u8> {
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
fn write_exe(path: &Path) -> Result<()> {
    fs::create_dir_all(path.parent().unwrap())?;
    fs::write(path, pe(false))?;
    Ok(())
}
fn context() -> presets::Context {
    presets::Context {
        scheme: "upstream-0.3.5-310-9".into(),
        profile: "upstream035".into(),
        delta: false,
        delta_capable: false,
    }
}
fn install(target: &core::DeploymentTarget) -> Result<String> {
    core::deploy_prepared_context_at(
        target,
        "upstream_sm86",
        &["version.dll".into()],
        None,
        BTreeMap::from([
            ("version.dll".into(), pe(true)),
            (core::INI.into(), b"[Logging]\r\nLevel=1\r\n".to_vec()),
        ]),
        Some("0.3.5"),
        Some(&context()),
    )
}
struct OwnedChild(Child);
impl Drop for OwnedChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
#[test]
fn owned_guard_sleeper() -> Result<()> {
    if let Some(ready) = std::env::var_os("RTXFG_V428_GUARD_READY") {
        fs::write(ready, b"ready")?;
        std::thread::sleep(Duration::from_secs(30));
    }
    Ok(())
}

#[test]
fn grouped_alternate_blocks_every_mutation_in_shared_plugin_folder() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let primary = temp.path().join("Binaries/Win64/RTXFG-V428-Primary.exe");
    let alternate = temp.path().join("Binaries/Win64r/RTXFG-V428-Alternate.exe");
    let plugins = temp.path().join("plugins");
    write_exe(&primary)?;
    fs::create_dir_all(alternate.parent().unwrap())?;
    fs::copy(std::env::current_exe()?, &alternate)?;
    fs::create_dir(&plugins)?;
    fs::write(plugins.join("other-plugin.dll"), b"unrelated plugin")?;
    fs::write(
        alternate.parent().unwrap().join(core::INI),
        b"other deployment",
    )?;
    let mut game = scanner::Game {
        exe: primary.display().to_string(),
        deployment_dir: plugins.display().to_string(),
        targets: vec![
            primary.display().to_string(),
            alternate.display().to_string(),
        ],
        ..Default::default()
    };
    assert_eq!(game.deployment_executables(), [game.exe.clone()]);
    let target = game.deployment_target(&primary).validate(true)?;
    assert_eq!(
        target.guard_exes.as_slice(),
        std::slice::from_ref(&alternate)
    );
    install(&target)?;
    let before = fs::read(plugins.join(core::INI))?;
    let marker = fs::read(plugins.join(core::OWN).join(core::MARKER))?;
    let ready = temp.path().join("child-ready");
    let mut child = OwnedChild(
        Command::new(&alternate)
            .args(["--exact", "owned_guard_sleeper", "--nocapture"])
            .env("RTXFG_V428_GUARD_READY", &ready)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(0x08000000)
            .spawn()?,
    );
    let start = Instant::now();
    while !ready.is_file() && start.elapsed() < Duration::from_secs(10) {
        assert!(
            child.0.try_wait()?.is_none(),
            "owned child exited before ready"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(ready.is_file(), "owned child must actually be running");
    assert!(!win::running_in_directory(alternate.parent().unwrap())?.is_empty());
    for result in [
        core::assert_target_stopped(&target),
        core::preflight_install_at(&target, &["version.dll".into()]),
        install(&target).map(|_| ()),
        core::apply_parameters_at(
            &target,
            &context(),
            &BTreeMap::from([("logging_level".into(), "3".into())]),
        ),
        cleanup::clean_outcome_at(&target).map(|_| ()),
    ] {
        assert!(result.unwrap_err().to_string().contains("请先完全退出游戏"));
    }
    assert_eq!(fs::read(plugins.join(core::INI))?, before);
    assert_eq!(
        fs::read(plugins.join(core::OWN).join(core::MARKER))?,
        marker
    );
    assert_eq!(fs::read(plugins.join("version.dll"))?, pe(true));

    // A rescan may move an older executable to cleanup_only. Its process still
    // protects the shared folder, and normal per-directory deployment also guards it.
    game.targets = vec![game.exe.clone()];
    game.cleanup_only = vec![alternate.display().to_string()];
    assert!(core::assert_target_stopped(&game.deployment_target(&primary)).is_err());
    game.deployment_dir.clear();
    assert!(core::assert_target_stopped(&game.deployment_target(&primary)).is_err());
    drop(child);

    core::apply_parameters_at(
        &target,
        &context(),
        &BTreeMap::from([("logging_level".into(), "3".into())]),
    )?;
    assert_ne!(fs::read(plugins.join(core::INI))?, before);
    let record = core::record(&plugins)?.unwrap();
    assert_eq!(
        core::key(Path::new(record.game_exe.as_deref().unwrap())),
        core::key(&primary)
    );
    assert!(cleanup::clean_outcome_at(&target)?.complete);
    assert_eq!(
        fs::read(plugins.join("other-plugin.dll"))?,
        b"unrelated plugin"
    );
    assert_eq!(
        fs::read(alternate.parent().unwrap().join(core::INI))?,
        b"other deployment"
    );
    assert!(cleanup::clean_outcome_at(&target)?.complete);
    Ok(())
}

#[test]
fn missing_grouped_executable_remains_guard_without_blocking_valid_deployment() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let primary = temp.path().join("game/Primary.exe");
    let old = temp.path().join("removed/Old.exe");
    let plugins = temp.path().join("plugins");
    write_exe(&primary)?;
    fs::create_dir(&plugins)?;
    let game = scanner::Game {
        exe: primary.display().to_string(),
        deployment_dir: plugins.display().to_string(),
        targets: vec![primary.display().to_string(), old.display().to_string()],
        cleanup_only: vec![old.display().to_string()],
        ..Default::default()
    };
    let target = game.deployment_target(&primary).validate(true)?;
    assert_eq!(target.guard_exes, [old]);
    install(&target)?;
    assert!(cleanup::clean_outcome_at(&target)?.complete);
    assert!(!temp.path().join("removed").exists());
    Ok(())
}

#[test]
fn scanner_normalizes_build_roles_without_excluding_game_title_fragments() -> Result<()> {
    let temp = tempfile::tempdir()?;
    fs::create_dir(temp.path().join("Engine"))?;
    let client = temp
        .path()
        .join("Client/Binaries/Win64/Game-Win64-Shipping.exe");
    write_exe(&client)?;
    for (folder, name) in [
        ("Server", "GameServer-Win64-Shipping.exe"),
        ("ServerLower/server", "gameserver-Win64-Development.exe"),
        ("ServerUpper/server", "GAMESERVER-WIN64-DEBUGGAME.EXE"),
        ("Dedicated", "GameDedicatedServer-Win64-Test.exe"),
        ("Editor", "GameEditor-Win32-Shipping.exe"),
        ("Config", "Game-Config-Win64-Debug.exe"),
        ("Benchmark", "GameBenchmark-Win64-Shipping.exe"),
        ("Crash", "CrashReportClient-Win64-DebugGame.exe"),
        ("Launcher", "GameLauncher-Win64-Test.exe"),
        ("GPU", "vulkaninfo-Win64-Shipping.exe"),
    ] {
        write_exe(&temp.path().join(folder).join("Binaries/Win64").join(name))?;
    }
    let names = [
        "Observer",
        "ObserverGame",
        "ServerQuest",
        "CrashBandicoot",
        "InstallerTycoon",
        "BenchmarkAdventure",
        "VulkanStory",
    ];
    for name in names {
        write_exe(
            &temp
                .path()
                .join(name)
                .join("Binaries/Win64")
                .join(format!("{name}-Win64-Shipping.exe")),
        )?;
    }
    let report = scanner::scan(&[temp.path().into()], &AtomicBool::new(false), |_, _, _| {})?;
    let mut actual = report
        .rows
        .iter()
        .flat_map(|g| &g.targets)
        .map(|p| {
            Path::new(p)
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned()
        })
        .collect::<Vec<_>>();
    actual.sort();
    let mut expected = names
        .iter()
        .map(|n| format!("{n}-Win64-Shipping.exe"))
        .chain(["Game-Win64-Shipping.exe".into()])
        .collect::<Vec<_>>();
    expected.sort();
    assert_eq!(actual, expected);
    Ok(())
}
