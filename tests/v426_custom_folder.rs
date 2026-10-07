//! Isolated plugin-folder regressions; synthetic PE files are never executed.
use anyhow::Result;
use rtx_fg_manager::{cleanup, cloud, core, deployment, diagnostics, presets, scanner};
use std::{collections::BTreeMap, fs, path::Path, sync::atomic::AtomicBool};

fn pe(dll: bool) -> Vec<u8> {
    let mut b = vec![0; 512];
    b[..2].copy_from_slice(b"MZ");
    b[60..64].copy_from_slice(&128u32.to_le_bytes());
    b[128..132].copy_from_slice(b"PE\0\0");
    b[132..134].copy_from_slice(&0x8664u16.to_le_bytes());
    b[148..150].copy_from_slice(&240u16.to_le_bytes());
    b[150..152].copy_from_slice(&(if dll { 0x2000u16 } else { 2 }).to_le_bytes());
    b[152..154].copy_from_slice(&0x20bu16.to_le_bytes());
    b
}
fn context() -> presets::Context {
    presets::Context {
        scheme: "upstream-0.3.5-310-9".into(),
        profile: "upstream035".into(),
        delta: false,
        delta_capable: false,
    }
}
fn files() -> BTreeMap<String, Vec<u8>> {
    BTreeMap::from([
        ("version.dll".into(), pe(true)),
        (
            core::INI.into(),
            b"; keep user comment\r\n[FrameGeneration]\r\nMaxGeneratedFrames=3\r\n[Other]\r\nKeep=1\r\n".to_vec(),
        ),
    ])
}
fn install(target: &core::DeploymentTarget) -> Result<String> {
    core::deploy_prepared_context_at(
        target,
        "upstream_sm86",
        &["version.dll".into()],
        None,
        files(),
        Some("0.3.5"),
        Some(&context()),
    )
}

#[test]
fn custom_folder_without_exe_uses_bound_game_and_preserves_other_plugins() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let exe = temp.path().join("RTXFG-CustomFolder-Game.exe");
    fs::write(&exe, pe(false))?;
    let folder = temp.path().join("中文目录/OptiScaler/plugins");
    fs::create_dir_all(&folder)?;
    fs::write(
        folder.join("OptiScaler.ini"),
        b"keep existing plugin config",
    )?;
    fs::write(folder.join("other-plugin.dll"), b"keep unrelated library")?;
    let target = core::DeploymentTarget::custom(&exe, &folder);
    core::preflight_install_at(&target, &["version.dll".into()])?;
    install(&target)?;
    assert!(!folder.join(exe.file_name().unwrap()).exists());
    assert!(!temp.path().join("version.dll").exists());
    assert!(core::status_at(&target).starts_with("已部署"));
    assert_eq!(core::status(&exe), "未部署");
    let record = core::record(&folder)?.unwrap();
    assert_eq!(record.game_exe.as_deref(), Some(exe.to_str().unwrap()));

    let game = scanner::Game {
        exe: exe.display().to_string(),
        deployment_dir: folder.display().to_string(),
        targets: vec![exe.display().to_string(), "ignored-other-target.exe".into()],
        ..Default::default()
    };
    let snapshot = deployment::inspect(&game, &cloud::bundled());
    assert_eq!((snapshot.total, snapshot.installed), (1, 1));
    assert!(snapshot.details[0].contains("plugins"));
    core::apply_parameters_at(
        &target,
        &context(),
        &BTreeMap::from([("max_generated_frames".into(), "1".into())]),
    )?;
    let ini = fs::read_to_string(folder.join(core::INI))?;
    assert!(ini.contains("; keep user comment\r\n"));
    assert!(ini.contains("Keep=1\r\n"));
    assert_eq!(
        diagnostics::ini_value(&ini, "FrameGeneration", "MaxGeneratedFrames")?,
        Some("1".into())
    );
    let (scheme, values) = presets::inspect_at(&target, &cloud::bundled())?.unwrap();
    assert_eq!(scheme, "upstream-0.3.5-310-9");
    assert_eq!(values["max_generated_frames"], "1");
    assert!(presets::inspect(&exe, &cloud::bundled())?.is_none());

    cleanup::clean_at(&target)?;
    assert!(!folder.join("version.dll").exists());
    assert!(!folder.join(core::INI).exists());
    assert_eq!(
        fs::read(folder.join("OptiScaler.ini"))?,
        b"keep existing plugin config"
    );
    assert_eq!(
        fs::read(folder.join("other-plugin.dll"))?,
        b"keep unrelated library"
    );
    assert_eq!(fs::read(exe)?, pe(false));
    cleanup::clean_at(&target)?;
    Ok(())
}

#[test]
fn custom_folder_rejects_collision_and_wrong_game_binding() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let exe = temp.path().join("RTXFG-CustomFolder-A.exe");
    let other_exe = temp.path().join("RTXFG-CustomFolder-B.exe");
    fs::write(&exe, pe(false))?;
    fs::write(&other_exe, pe(false))?;
    let folder = temp.path().join("plugins");
    fs::create_dir(&folder)?;
    let target = core::DeploymentTarget::custom(&exe, &folder);
    fs::write(folder.join("version.dll"), b"original OptiScaler library")?;
    assert!(core::preflight_install_at(&target, &["version.dll".into()]).is_err());
    assert!(install(&target).is_err());
    assert_eq!(
        fs::read(folder.join("version.dll"))?,
        b"original OptiScaler library"
    );
    fs::remove_file(folder.join("version.dll"))?;
    install(&target)?;
    let wrong = core::DeploymentTarget::custom(&other_exe, &folder);
    assert!(core::preflight_install_at(&wrong, &["version.dll".into()]).is_err());
    assert!(install(&wrong).is_err());
    assert!(core::apply_parameters_at(&wrong, &context(), &BTreeMap::new()).is_err());
    assert!(presets::inspect_at(&wrong, &cloud::bundled()).is_err());
    assert!(cleanup::clean_at(&wrong).is_err());
    assert!(folder.join("version.dll").is_file());
    cleanup::clean_at(&target)?;
    Ok(())
}

#[test]
fn custom_folder_validation_rejects_system_root_and_internal_dirs() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let missing = temp.path().join("missing");
    assert!(core::deployment_directory(&missing).is_err());
    assert!(core::deployment_directory(Path::new(r"C:\")).is_err());
    let windows = std::env::var_os("WINDIR").unwrap_or_else(|| r"C:\Windows".into());
    assert!(core::deployment_directory(Path::new(&windows)).is_err());
    let internal = temp.path().join(core::OWN).join("cache");
    fs::create_dir_all(&internal)?;
    assert!(core::deployment_directory(&internal).is_err());
    let exe = temp.path().join("RTXFG-CustomFolder-Missing.exe");
    let target = core::DeploymentTarget::custom(&exe, temp.path());
    assert!(target.validate(true).is_err());
    assert!(target.validate(false).is_ok());
    Ok(())
}

#[test]
fn custom_folder_can_remove_owned_patch_after_game_directory_is_gone() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let game_dir = temp.path().join("removed-game");
    let plugin_dir = temp.path().join("remaining-plugins");
    fs::create_dir(&game_dir)?;
    fs::create_dir(&plugin_dir)?;
    let exe = game_dir.join("RTXFG-CustomFolder-Orphan-Guard.exe");
    fs::write(&exe, pe(false))?;
    fs::write(plugin_dir.join("OptiScaler.ini"), b"unrelated settings")?;
    let target = core::DeploymentTarget::custom(&exe, &plugin_dir);
    install(&target)?;
    fs::remove_file(&exe)?;
    fs::remove_dir(&game_dir)?;

    assert!(target.validate(false).is_ok());
    assert!(target.validate(true).is_err());
    assert!(core::location(&exe, false).is_err());
    assert!(core::status_at(&target).starts_with("已部署"));
    assert!(install(&target).is_err());
    assert!(core::apply_parameters_at(&target, &context(), &BTreeMap::new()).is_err());
    cleanup::clean_at(&target)?;
    assert_eq!(core::status_at(&target), "未部署");
    assert_eq!(
        fs::read(plugin_dir.join("OptiScaler.ini"))?,
        b"unrelated settings"
    );
    assert!(!plugin_dir.join("version.dll").exists());
    assert!(!plugin_dir.join(core::INI).exists());
    Ok(())
}

#[test]
fn custom_folder_settings_roundtrip_preserves_real_executable() -> Result<()> {
    let old: scanner::Game = serde_json::from_str(r#"{"exe":"C:\\Games\\Game.exe"}"#)?;
    assert!(old.deployment_dir.is_empty());
    let game = scanner::Game {
        exe: r"C:\Games\Game.exe".into(),
        deployment_dir: r"C:\Games\OptiScaler\plugins".into(),
        ..Default::default()
    };
    let restored: scanner::Game = serde_json::from_slice(&serde_json::to_vec(&game)?)?;
    assert_eq!(restored.exe, game.exe);
    assert_eq!(restored.deployment_dir, game.deployment_dir);
    assert_eq!(restored.deployment_executables(), vec![game.exe]);
    Ok(())
}

#[test]
fn scanner_excludes_exact_gpu_utilities_even_beside_rendering_libraries() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let tools = [
        "vulkaninfo-x64",
        "vulkaninfo-x86",
        "vulkaninfo",
        "vulkaninfo64",
        "nvidia-smi",
        "nvidia-debugdump",
    ];
    // Adjacent runtime libraries can be shared by diagnostic tools and games.
    // Use otherwise sufficient game evidence to exercise the filename boundary.
    let games = ["VulkanGame", "NvidiaStory", "Game-with-vulkaninfo"];
    for name in tools.iter().chain(games.iter()) {
        let dir = temp.path().join(name);
        fs::create_dir_all(dir.join(format!("{name}_Data")))?;
        fs::write(dir.join(format!("{name}.exe")), pe(false))?;
        fs::write(dir.join("UnityPlayer.dll"), b"runtime fixture")?;
        fs::write(dir.join("nvngx_dlssg.dll"), b"runtime fixture")?;
    }
    let report = scanner::scan(&[temp.path().into()], &AtomicBool::new(false), |_, _, _| {})?;
    assert_eq!(report.rows.len(), games.len());
    for name in games {
        assert!(report.rows.iter().any(|game| {
            Path::new(&game.exe)
                .file_stem()
                .is_some_and(|stem| stem == name)
        }));
    }
    Ok(())
}
