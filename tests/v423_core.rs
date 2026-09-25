use anyhow::{Result, bail};
use rtx_fg_manager::{core, preferences, presets, scanner, win};
use serde_json::{Value, json};
use std::{
    fs,
    path::Path,
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};

fn exe(path: &Path) -> Result<()> {
    let mut bytes = vec![0; 512];
    bytes[..2].copy_from_slice(b"MZ");
    bytes[60..64].copy_from_slice(&128u32.to_le_bytes());
    bytes[128..132].copy_from_slice(b"PE\0\0");
    bytes[132..134].copy_from_slice(&0x8664u16.to_le_bytes());
    bytes[148..150].copy_from_slice(&240u16.to_le_bytes());
    bytes[150..152].copy_from_slice(&2u16.to_le_bytes());
    bytes[152..154].copy_from_slice(&0x20bu16.to_le_bytes());
    fs::write(path, bytes)?;
    Ok(())
}

#[test]
fn scan_preserves_game_names_containing_tool_words_and_distinct_launch_modes() -> Result<()> {
    let dir = tempfile::tempdir()?;
    for name in [
        "Game-DX11.exe",
        "Game-DX12.exe",
        "CrashReportClient.exe",
        "unins000.exe",
        "Launcher.exe",
        "UE4PrereqSetup_x64.exe",
        "UnityCrashHandler64.exe",
        "UE4Editor-Cmd.exe",
    ] {
        exe(&dir.path().join(name))?;
    }
    fs::write(dir.path().join("nvngx_dlssg.dll"), b"component evidence")?;
    for name in ["CrashBandicoot", "InstallerTycoon"] {
        let folder = dir.path().join(name);
        fs::create_dir_all(&folder)?;
        exe(&folder.join(format!("{name}.exe")))?;
        fs::write(folder.join("nvngx_dlssg.dll"), b"component evidence")?;
    }
    let report = scanner::scan(
        &[dir.path().into(), dir.path().into()],
        &AtomicBool::new(false),
        |_, _, _| {},
    )?;
    let mut names = report
        .rows
        .iter()
        .map(|g| {
            Path::new(&g.exe)
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned()
        })
        .collect::<Vec<_>>();
    names.sort();
    assert_eq!(
        names,
        ["CrashBandicoot.exe", "Game-DX12.exe", "InstallerTycoon.exe"]
    );
    assert_eq!(report.candidates, 3);
    assert_eq!(
        report
            .rows
            .iter()
            .find(|g| g.exe.ends_with("Game-DX12.exe"))
            .unwrap()
            .targets
            .len(),
        1
    );
    Ok(())
}

#[test]
fn steam_scan_uses_install_identity_and_excludes_protected_launchers() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let steam = dir.path().join("Steam");
    let apps = steam.join("steamapps");
    let apex = apps.join("common/Apex Legends");
    let storm = apps.join("common/Stormgate");
    fs::create_dir_all(&apex)?;
    fs::create_dir_all(apex.join("Binaries/Win64"))?;
    fs::create_dir_all(apex.join("Support"))?;
    fs::create_dir_all(storm.join("Engine"))?;
    fs::create_dir_all(storm.join("Stormgate/Binaries/Win64"))?;
    for (id, title, folder) in [
        ("1172470", "Apex Legends", "Apex Legends"),
        ("2012510", "Stormgate", "Stormgate"),
    ] {
        fs::write(
            apps.join(format!("appmanifest_{id}.acf")),
            format!(
                "\"AppState\"\n{{\n\"appid\" \"{id}\"\n\"name\" \"{title}\"\n\"installdir\" \"{folder}\"\n}}"
            ),
        )?;
    }
    exe(&apex.join("r5apex_dx12.exe"))?;
    exe(&apex.join("Binaries/Win64/r5apex_vulkan.exe"))?;
    exe(&apex.join("start_protected_game.exe"))?;
    exe(&apex.join("Support/VideoEncoderDX12.exe"))?;
    exe(&storm.join("Stormgate.exe"))?;
    exe(&storm.join("start_protected_game.exe"))?;
    exe(&storm.join("Stormgate/Binaries/Win64/Stormgate-Win64-Shipping.exe"))?;
    exe(&storm.join("Stormgate/Binaries/Win64/StormgateLauncher-Win64-Shipping.exe"))?;
    exe(&storm.join("Stormgate/Binaries/Win64/StormgateServer-Win64-Shipping.exe"))?;
    exe(&storm.join("Stormgate/Binaries/Win64/EasyAntiCheat-Win64-Shipping.exe"))?;
    exe(&storm.join("Stormgate/Binaries/Win64/StartProtectedGame-Win64-Shipping.exe"))?;
    exe(&steam.join("steam.exe"))?;
    fs::create_dir_all(steam.join("bin"))?;
    exe(&steam.join("bin/steamxboxutil64.exe"))?;
    fs::write(steam.join("nvngx_dlss.dll"), b"client component")?;
    let scan = scanner::scan(&[steam], &AtomicBool::new(false), |_, _, _| {})?;
    assert_eq!(scan.candidates, 2);
    let mut titles = scan
        .rows
        .iter()
        .map(|g| g.title.as_str())
        .collect::<Vec<_>>();
    titles.sort();
    assert_eq!(titles, ["Apex Legends", "Stormgate"]);
    let apex_game = scan
        .rows
        .iter()
        .find(|g| g.title == "Apex Legends")
        .unwrap();
    assert_eq!(apex_game.targets.len(), 2);
    assert!(
        apex_game
            .targets
            .iter()
            .any(|p| p.ends_with("r5apex_dx12.exe"))
    );
    assert!(
        apex_game
            .targets
            .iter()
            .any(|p| p.ends_with("r5apex_vulkan.exe"))
    );
    assert_eq!(
        scan.rows
            .iter()
            .find(|g| g.title == "Stormgate")
            .unwrap()
            .targets
            .len(),
        1
    );
    assert_eq!(
        core::key(Path::new(
            &scan
                .rows
                .iter()
                .find(|g| g.title == "Stormgate")
                .unwrap()
                .root
        )),
        core::key(&storm)
    );
    assert!(
        scan.rows
            .iter()
            .any(|g| g.exe.ends_with("Stormgate-Win64-Shipping.exe"))
    );
    assert!(scan.rows.iter().all(|g| !g.exe.contains("protected_game")));
    assert!(scan.rows.iter().all(|g| {
        g.targets.iter().all(|target| {
            !target.contains("VideoEncoderDX12")
                && !target.contains("EasyAntiCheat")
                && !target.contains("StartProtectedGame")
        })
    }));
    Ok(())
}

#[test]
fn separate_unreal_projects_do_not_merge_under_shared_engine_or_steam_manifest() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let standalone = dir.path().join("SharedEngine");
    let steam_apps = dir.path().join("Steam/steamapps");
    let steam_bundle = steam_apps.join("common/Bundle");
    fs::create_dir_all(standalone.join("Engine"))?;
    fs::create_dir_all(steam_bundle.join("Engine"))?;
    fs::write(
        steam_apps.join("appmanifest_123.acf"),
        "\"AppState\"\n{\n\"name\" \"Bundle\"\n\"installdir\" \"Bundle\"\n}",
    )?;
    for root in [&standalone, &steam_bundle] {
        for project in ["Alpha", "Beta"] {
            let binary = root.join(project).join("Binaries/Win64");
            fs::create_dir_all(&binary)?;
            exe(&binary.join(format!("{project}-Win64-Shipping.exe")))?;
        }
    }
    let report = scanner::scan(&[dir.path().into()], &AtomicBool::new(false), |_, _, _| {})?;
    assert_eq!(report.candidates, 4);
    for root in [&standalone, &steam_bundle] {
        for project in ["Alpha", "Beta"] {
            let game = report
                .rows
                .iter()
                .find(|g| core::key(Path::new(&g.root)) == core::key(&root.join(project)))
                .unwrap();
            assert_eq!(game.targets.len(), 1);
            assert!(game.exe.ends_with(&format!("{project}-Win64-Shipping.exe")));
            if root == &steam_bundle {
                assert_eq!(game.title, format!("Bundle · {project}"));
            }
        }
    }
    Ok(())
}

#[test]
fn scanner_does_not_promote_unrelated_software_beside_dlss_files() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let tool = dir.path().join("CaptureTools");
    fs::create_dir_all(&tool)?;
    exe(&tool.join("VideoEncoder.exe"))?;
    fs::write(tool.join("nvngx_dlssg.dll"), b"component evidence")?;
    let scan = scanner::scan(&[tool], &AtomicBool::new(false), |_, _, _| {})?;
    assert!(scan.rows.is_empty());
    Ok(())
}

#[test]
fn scan_skip_details_are_bounded_and_do_not_imply_successful_coverage() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let roots = (0..50)
        .map(|i| dir.path().join(format!("missing-{i}")))
        .collect::<Vec<_>>();
    let report = scanner::scan(&roots, &AtomicBool::new(false), |_, _, _| {})?;
    assert_eq!(report.skipped, 50);
    assert_eq!(report.skipped_details.len(), scanner::MAX_SKIP_DETAILS);
    assert!(
        report
            .skipped_details
            .iter()
            .all(|s| !s.reason.is_empty() && s.path.chars().count() <= 512)
    );
    assert_eq!(report.directories, 0);
    assert!(report.rows.is_empty());
    let cancelled = scanner::scan(&roots, &AtomicBool::new(true), |_, _, _| {})?;
    assert!(cancelled.cancelled);
    assert_eq!(cancelled.skipped, 0);
    Ok(())
}

#[test]
fn mfg_request_cap_uses_generated_frame_count_and_keeps_inactive_target() -> Result<()> {
    let mut values = presets::defaults(presets::MFG_VULKAN, &presets::Values::new());
    values.insert("max_interpolated_frames".into(), "1".into());
    values.insert("force_multiplier".into(), "6".into());
    values.insert("dynamic_target_fps".into(), "240".into());
    assert!(presets::validate(presets::MFG_VULKAN, &values).is_err());
    assert!(presets::normalize(presets::MFG_VULKAN, &mut values));
    assert_eq!(values["force_multiplier"], "2");
    assert!(!presets::normalize(presets::MFG_VULKAN, &mut values));
    assert!(!presets::parameter_enabled(
        presets::MFG_VULKAN,
        "dynamic_target_fps",
        &values
    ));
    assert_eq!(values["dynamic_target_fps"], "240");
    values.insert("dynamic_mfg".into(), "1".into());
    assert!(presets::parameter_enabled(
        presets::MFG_VULKAN,
        "dynamic_target_fps",
        &values
    ));
    let output = presets::configure(
        b"[FrameGeneration]\n; preserve\nPrivateValue=9\n",
        presets::MFG_VULKAN,
        &values,
    )?;
    let text = String::from_utf8(output)?;
    assert!(text.contains("PrivateValue=9") && text.contains("; preserve"));
    assert_eq!(
        presets::read_values(text.as_bytes(), presets::MFG_VULKAN)?,
        values
    );
    values.insert("force_multiplier".into(), "0".into());
    assert!(!presets::normalize(presets::MFG_VULKAN, &mut values));
    assert_eq!(values["force_multiplier"], "0");
    Ok(())
}

#[test]
fn legacy_mfg_conflicts_normalize_without_leaking_into_other_protocols() -> Result<()> {
    let legacy = presets::Values::from([
        ("max_interpolated_frames".into(), "2".into()),
        ("force_multiplier".into(), "6".into()),
        ("dynamic_target_fps".into(), "165".into()),
    ]);
    let normalized = presets::defaults(presets::MFG_VULKAN, &legacy);
    assert_eq!(normalized["force_multiplier"], "3");
    assert_eq!(normalized["dynamic_target_fps"], "165");
    let read = presets::read_values(
        b"[FrameGeneration]\nMaxInterpolatedFrames=2\nForceMultiplier=6\nDynamicTargetFPS=165\n",
        presets::MFG_VULKAN,
    )?;
    assert_eq!(read["force_multiplier"], "3");
    let mut other = legacy.clone();
    assert!(!presets::normalize("upstream035", &mut other));
    assert_eq!(legacy, other);
    let invalid = presets::Values::from([("force_multiplier".into(), "200".into())]);
    assert!(presets::configure(b"", presets::MFG_VULKAN, &invalid).is_err());
    assert_eq!(
        presets::defaults(presets::MFG_VULKAN, &invalid)["force_multiplier"],
        "0"
    );
    Ok(())
}

fn settings(n: u64) -> Value {
    json!({"schema":3,"games":[{"exe":"D:\\游戏\\Game.exe","future":{"value":n}}],"roots":[],"language":"ja","extra":n})
}
fn wait_result(store: &preferences::Store, revision: u64) -> Result<preferences::SaveResult> {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(5) {
        if let Some(result) = store.try_result()
            && result.revision == revision
        {
            return Ok(result);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    bail!("settings worker did not report revision {revision}")
}

#[test]
fn settings_async_result_reports_real_failure_and_allows_retry() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("games.json");
    fs::create_dir(&path)?;
    let store = preferences::Store::new(dir.path().into());
    let first = store.save_tracked(settings(1))?;
    assert!(wait_result(&store, first)?.result.is_err());
    fs::remove_dir(&path)?;
    let second = store.save_tracked(settings(2))?;
    assert!(second > first);
    let result = wait_result(&store, second)?;
    assert!(result.result.is_ok() && result.backup_warning.is_none());
    assert_eq!(preferences::load(dir.path())?, settings(2));
    store.finish()?;
    assert_eq!(
        core::read_json(&dir.path().join("games.last-good.json"), 4 * 1024 * 1024)?,
        settings(2)
    );
    Ok(())
}

#[test]
fn settings_latest_revision_survives_coalescing_and_close() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let store = preferences::Store::new(dir.path().into());
    let mut revision = 0;
    for n in 0..100 {
        revision = store.save_tracked(settings(n))?;
    }
    assert!(wait_result(&store, revision)?.result.is_ok());
    store.finish()?;
    assert_eq!(preferences::load(dir.path())?, settings(99));
    assert_eq!(
        core::read_json(&dir.path().join("games.last-good.json"), 4 * 1024 * 1024)?,
        settings(99)
    );
    Ok(())
}

#[test]
fn corrupted_settings_recover_latest_good_and_preserve_exact_damaged_bytes() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let store = preferences::Store::new(dir.path().into());
    store.save(settings(7))?;
    store.finish()?;
    let path = dir.path().join("games.json");
    let corrupt = b"{\"schema\":3,\"games\":[";
    fs::write(&path, corrupt)?;
    assert!(preferences::load(dir.path()).is_err());
    assert_eq!(fs::read(&path)?, corrupt);
    let result = preferences::load_with_recovery(dir.path())?;
    assert!(result.recovered && result.warning.is_some());
    assert_eq!(result.value, settings(7));
    assert_eq!(fs::read(dir.path().join("games.corrupt.json"))?, corrupt);
    assert_eq!(preferences::load(dir.path())?, settings(7));
    assert!(!preferences::load_with_recovery(dir.path())?.recovered);
    Ok(())
}

#[test]
fn recovery_never_overwrites_future_schema_invalid_structure_or_old_quarantine() -> Result<()> {
    let dir = tempfile::tempdir()?;
    core::atomic_json(&dir.path().join("games.last-good.json"), &settings(1))?;
    let path = dir.path().join("games.json");
    for invalid in [
        json!({"schema":4,"games":[]}),
        json!({"schema":3,"games":{}}),
    ] {
        core::atomic_json(&path, &invalid)?;
        assert!(preferences::load_with_recovery(dir.path()).is_err());
        assert_eq!(core::read_json(&path, 4 * 1024 * 1024)?, invalid);
    }
    fs::write(&path, b"new broken file")?;
    fs::write(dir.path().join("games.corrupt.json"), b"older broken file")?;
    assert!(preferences::load_with_recovery(dir.path()).is_err());
    assert_eq!(fs::read(&path)?, b"new broken file");
    assert_eq!(
        fs::read(dir.path().join("games.corrupt.json"))?,
        b"older broken file"
    );
    Ok(())
}

#[test]
fn missing_settings_restore_backup_but_do_not_hide_unknown_backup() -> Result<()> {
    let dir = tempfile::tempdir()?;
    assert!(!preferences::load_with_recovery(dir.path())?.recovered);
    core::atomic_json(&dir.path().join("games.last-good.json"), &settings(3))?;
    let report = preferences::load_with_recovery(dir.path())?;
    assert!(report.recovered);
    assert_eq!(report.value, settings(3));
    fs::remove_file(dir.path().join("games.json"))?;
    let future = json!({"schema":4,"games":[]});
    core::atomic_json(&dir.path().join("games.last-good.json"), &future)?;
    assert!(preferences::load_with_recovery(dir.path()).is_err());
    assert!(!dir.path().join("games.json").exists());
    Ok(())
}

#[test]
fn successful_settings_write_distinguishes_backup_failure() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let future = json!({"schema":4,"games":[]});
    core::atomic_json(&dir.path().join("games.last-good.json"), &future)?;
    let store = preferences::Store::new(dir.path().into());
    let revision = store.save_tracked(settings(4))?;
    let result = wait_result(&store, revision)?;
    assert!(result.result.is_ok());
    assert!(result.backup_warning.is_some());
    assert_eq!(preferences::load(dir.path())?, settings(4));
    assert_eq!(
        core::read_json(&dir.path().join("games.last-good.json"), 4 * 1024 * 1024)?,
        future
    );
    store.finish()?;
    Ok(())
}

#[test]
fn startup_resource_workers_wait_for_each_other_without_changing_game_lock_behavior() -> Result<()>
{
    use std::sync::mpsc;

    let directory = tempfile::tempdir()?;
    let path = directory.path().join("shared-aria2-resource");
    let first = win::resource_lock(&path)?;
    let (started_tx, started_rx) = mpsc::channel();
    let (acquired_tx, acquired_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    std::thread::scope(|scope| -> Result<()> {
        let resource_path = &path;
        let worker = scope.spawn(move || -> Result<(), String> {
            started_tx.send(()).map_err(|e| e.to_string())?;
            let lock = win::resource_lock(resource_path).map_err(|e| e.to_string());
            acquired_tx
                .send(lock.as_ref().map(|_| ()).map_err(Clone::clone))
                .map_err(|e| e.to_string())?;
            let _lock = lock?;
            release_rx
                .recv_timeout(Duration::from_secs(5))
                .map_err(|e| e.to_string())?;
            Ok(())
        });
        started_rx.recv_timeout(Duration::from_secs(5))?;
        assert_eq!(
            acquired_rx.recv_timeout(Duration::from_millis(100)),
            Err(mpsc::RecvTimeoutError::Timeout)
        );
        drop(first);
        acquired_rx
            .recv_timeout(Duration::from_secs(5))?
            .map_err(anyhow::Error::msg)?;
        // Resource extraction may wait; a concurrent game mutation still fails
        // promptly. Different resource paths must not share a global lock.
        assert!(win::game_lock(&path).is_err());
        let independent = win::resource_lock(&directory.path().join("other-resource"))?;
        drop(independent);
        release_tx.send(())?;
        worker
            .join()
            .map_err(|_| anyhow::anyhow!("resource worker panicked"))?
            .map_err(anyhow::Error::msg)?;
        Ok(())
    })
}
