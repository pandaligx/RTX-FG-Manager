use rtx_fg_manager::{cleanup, cleanup_pending, core, delta, scanner, xbox};
use std::{fs, path::Path, sync::atomic::AtomicBool};

#[test]
#[ignore = "read-only local Xbox installation probe; requires RTXFG_XBOX_GAME"]
fn xbox_protected_installation_readonly_probe() {
    let exe = std::path::PathBuf::from(
        std::env::var_os("RTXFG_XBOX_GAME").expect("explicit local EXE required"),
    );
    let game = xbox::declared(&exe).expect("declared Xbox PC executable");
    let config = game.root.join("MicrosoftGame.config");
    let before = core::digest(&config).unwrap();
    let error = fs::File::open(&exe).expect_err("probe expects the reported protected EXE");
    assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
    core::library_location(&exe).unwrap();
    core::pe64(&exe, false).unwrap();
    // DLL validation must never inherit the EXE-only metadata fallback.
    assert!(core::pe64(&exe, true).is_err());
    let report = scanner::scan(
        std::slice::from_ref(&game.root),
        &AtomicBool::new(false),
        |_, _, _| {},
    )
    .unwrap();
    assert!(
        report
            .rows
            .iter()
            .any(|g| core::key(Path::new(&g.exe)) == core::key(&exe))
    );
    assert_eq!(before, core::digest(&config).unwrap());
    println!("Read-only Xbox probe: {} / {}", game.title, exe.display());
}

fn pe(path: &Path) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut bytes = vec![0u8; 512];
    bytes[..2].copy_from_slice(b"MZ");
    bytes[60..64].copy_from_slice(&128u32.to_le_bytes());
    bytes[128..132].copy_from_slice(b"PE\0\0");
    bytes[132..134].copy_from_slice(&0x8664u16.to_le_bytes());
    fs::write(path, bytes).unwrap();
}

fn pending_fixture(dir: &Path, delta_cache: bool) -> (core::DeploymentTarget, std::path::PathBuf) {
    let exe = dir.join("Game/Binaries/Win64").join(delta::GAME);
    pe(&exe);
    let target = core::DeploymentTarget::for_game(&exe);
    let root = dir.join("cache");
    let id = delta::cache_id(&exe);
    let record = serde_json::json!({"schema":3, "backend":"upstream_sm86", "proxy":"version.dll", "proxies":["version.dll"],
        "hashes":{"version.dll":core::hash(b"fixture"),"dlssg_sm86.ini":core::hash(b"config")},
        "delta_cache_ids": if delta_cache { vec![id.clone()] } else { vec![] }, "cache_pending":true});
    core::atomic_json(
        &target.directory.join(core::OWN).join(core::MARKER),
        &record,
    )
    .unwrap();
    core::record(&target.directory).unwrap();
    if delta_cache {
        delta::register_at(&root, &exe, &id).unwrap();
        fs::write(
            root.join("games").join(&id).join("leftover.log"),
            b"leftover",
        )
        .unwrap();
    }
    (target, root)
}

#[test]
fn confirmed_cleanup_removes_only_previewed_game_cache_and_stage_then_clears_warning() {
    let dir = tempfile::tempdir().unwrap();
    let (target, cache) = pending_fixture(dir.path(), true);
    let own = cache.join("games").join(delta::cache_id(&target.game_exe));
    let foreign = cache.join("games").join(delta::cache_id(
        &dir.path().join("Other/Binaries/Win64").join(delta::GAME),
    ));
    fs::create_dir_all(&foreign).unwrap();
    fs::write(foreign.join("keep.log"), b"other game").unwrap();
    let stage = target
        .directory
        .join(core::OWN)
        .join(core::stage_name("version.dll"));
    fs::write(&stage, b"unverifiable partial download").unwrap();
    let backup = target.directory.join(core::OWN).join("original.dll.backup");
    fs::write(&backup, b"original").unwrap();
    let plan = cleanup_pending::prepare_at(&target, Some(&cache)).unwrap();
    assert!(plan.paths().any(|p| p == own.join("leftover.log")));
    assert!(plan.paths().any(|p| p == stage));
    assert!(!plan.paths().any(|p| p == backup || p.starts_with(&foreign)));
    cleanup_pending::delete_confirmed(&plan).unwrap();
    assert!(!own.exists() && !stage.exists());
    assert_eq!(fs::read(&backup).unwrap(), b"original");
    assert_eq!(fs::read(foreign.join("keep.log")).unwrap(), b"other game");
    // Normal cleanup re-inspects the record; no pending flag is cleared by fiat.
    let outcome = cleanup::clean_outcome_at(&target).unwrap();
    assert!(outcome.complete);
    assert!(core::record(&target.directory).unwrap().is_none());
    assert!(cleanup_pending::delete_confirmed(&plan).is_err());
}

#[test]
fn confirmed_cleanup_rejects_changes_and_locks_before_deleting_anything() {
    use std::os::windows::fs::OpenOptionsExt;
    let dir = tempfile::tempdir().unwrap();
    let (target, cache) = pending_fixture(dir.path(), true);
    let own = cache.join("games").join(delta::cache_id(&target.game_exe));
    let plan = cleanup_pending::prepare_at(&target, Some(&cache)).unwrap();
    fs::write(own.join("new.txt"), b"created after preview").unwrap();
    assert!(cleanup_pending::delete_confirmed(&plan).is_err());
    assert!(own.join("owner.json").exists());
    let plan = cleanup_pending::prepare_at(&target, Some(&cache)).unwrap();
    fs::write(own.join("leftover.log"), b"changed after preview").unwrap();
    assert!(cleanup_pending::delete_confirmed(&plan).is_err());
    let plan = cleanup_pending::prepare_at(&target, Some(&cache)).unwrap();
    let held = fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(own.join("leftover.log"))
        .unwrap();
    assert!(cleanup_pending::delete_confirmed(&plan).is_err());
    assert!(own.join("owner.json").exists() && own.join("new.txt").exists());
    drop(held);
    let marker = target.directory.join(core::OWN).join(core::MARKER);
    let mut record = core::record(&target.directory).unwrap().unwrap();
    record.cleanup_pending_reasons.push("changed".into());
    core::atomic_json(&marker, &record).unwrap();
    assert!(cleanup_pending::delete_confirmed(&plan).is_err());
    assert!(own.join("leftover.log").exists());
}

#[test]
fn cleanup_never_offers_foreign_or_shared_caches_and_rejects_links() {
    let dir = tempfile::tempdir().unwrap();
    let (target, cache) = pending_fixture(dir.path(), true);
    let own = cache.join("games").join(delta::cache_id(&target.game_exe));
    let other = dir.path().join("Other/Binaries/Win64").join(delta::GAME);
    let id = delta::cache_id(&other);
    delta::register_at(&cache, &other, &id).unwrap();
    fs::create_dir_all(cache.join(delta::REVISION)).unwrap();
    fs::write(cache.join(delta::REVISION).join("keep.txt"), b"shared").unwrap();
    let marker = target.directory.join(core::OWN).join(core::MARKER);
    let mut record = core::record(&target.directory).unwrap().unwrap();
    record.delta_cache_ids = vec![id];
    record.delta_legacy_cache = true;
    core::atomic_json(&marker, &record).unwrap();
    assert_eq!(
        cleanup_pending::prepare_at(&target, Some(&cache))
            .unwrap()
            .paths()
            .count(),
        0
    );
    record.delta_cache_ids = vec![delta::cache_id(&target.game_exe)];
    core::atomic_json(&marker, &record).unwrap();
    let plan = cleanup_pending::prepare_at(&target, Some(&cache)).unwrap();
    let outside = dir.path().join("unrelated");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("keep.txt"), b"keep").unwrap();
    std::os::windows::fs::symlink_dir(&outside, own.join("linked")).unwrap();
    assert!(cleanup_pending::prepare_at(&target, Some(&cache)).is_err());
    assert!(cleanup_pending::delete_confirmed(&plan).is_err());
    assert_eq!(fs::read(outside.join("keep.txt")).unwrap(), b"keep");
    fs::remove_dir(own.join("linked")).unwrap();
}

#[test]
fn stale_missing_delta_cache_does_not_leave_an_eternal_warning() {
    let dir = tempfile::tempdir().unwrap();
    let exe = dir.path().join("new/Binaries/Win64").join(delta::GAME);
    let old = dir.path().join("old/Binaries/Win64").join(delta::GAME);
    let id = delta::cache_id(&old);
    let root = dir.path().join("cache");
    // Reproduces a copied 4.3.0 deployment record after the old cache was removed.
    assert!(
        delta::clean_at(&root, &exe, std::slice::from_ref(&id), false)
            .pending
            .is_empty()
    );
    let foreign = root.join("games").join(&id);
    fs::create_dir_all(&foreign).unwrap();
    fs::write(foreign.join("keep.txt"), b"another game's data").unwrap();
    assert!(
        !delta::clean_at(&root, &exe, std::slice::from_ref(&id), false)
            .pending
            .is_empty()
    );
    assert!(foreign.join("keep.txt").exists());
    fs::remove_file(foreign.join("keep.txt")).unwrap();
    fs::remove_dir(&foreign).unwrap();
    assert!(
        delta::clean_at(&root, &exe, &[id], false)
            .pending
            .is_empty()
    );
    assert!(
        !delta::clean_at(&root, &exe, &["../outside".into()], false)
            .pending
            .is_empty()
    );
}

#[test]
fn delta_unknown_files_and_invalid_owner_stay_visible_until_user_handles_them() {
    let dir = tempfile::tempdir().unwrap();
    let exe = dir.path().join("Game/Binaries/Win64").join(delta::GAME);
    let root = dir.path().join("cache");
    let id = delta::cache_id(&exe);
    delta::register_at(&root, &exe, &id).unwrap();
    let folder = root.join("games").join(&id);
    let unknown = folder.join(delta::REVISION).join("my-capture.txt");
    fs::create_dir_all(unknown.parent().unwrap()).unwrap();
    fs::write(&unknown, b"manual capture").unwrap();
    assert!(
        !delta::clean_at(&root, &exe, std::slice::from_ref(&id), false)
            .pending
            .is_empty()
    );
    assert_eq!(fs::read(&unknown).unwrap(), b"manual capture");
    fs::remove_file(&unknown).unwrap();
    assert!(
        delta::clean_at(&root, &exe, std::slice::from_ref(&id), false)
            .pending
            .is_empty()
    );
    assert!(!folder.exists());
    delta::register_at(&root, &exe, &id).unwrap();
    fs::write(folder.join("owner.json"), b"{corrupt}").unwrap();
    assert!(
        !delta::clean_at(&root, &exe, std::slice::from_ref(&id), false)
            .pending
            .is_empty()
    );
    assert!(folder.join("owner.json").exists());
}

#[test]
fn xbox_manifest_scan_finds_arbitrary_renderer_names_without_dlss_sentinels() {
    let dir = tempfile::tempdir().unwrap();
    let content = dir.path().join("XboxGames/A Game/Content");
    let exe = content.join("App/Binaries/WinGDK/opaque.exe");
    pe(&exe);
    pe(&content.join("Tools/tool.exe"));
    fs::write(content.join("MicrosoftGame.config"), r#"<Game configVersion="1"><ShellVisuals DisplayName="测试 &amp; Game"/><ExecutableList><Executable Name="App\Binaries\WinGDK\opaque.exe" TargetDeviceFamily="PC"/><Executable Name="Tools\tool.exe" TargetDeviceFamily="PC" IsDevOnly="true"/></ExecutableList></Game>"#).unwrap();
    let report =
        scanner::scan(&[dir.path().into()], &AtomicBool::new(false), |_, _, _| {}).unwrap();
    assert_eq!(report.rows.len(), 1);
    assert_eq!(core::key(Path::new(&report.rows[0].exe)), core::key(&exe));
    assert_eq!(report.rows[0].title, "测试 & Game");
    assert_eq!(report.rows[0].targets.len(), 1);
    assert_eq!(xbox::declared(&exe).unwrap().machine, 0x8664);
    // A manifest is not permission to accept corrupt readable executables.
    fs::write(&exe, b"not a Windows game").unwrap();
    assert!(core::library_location(&exe).is_err());
    assert!(core::pe64(&exe, false).is_err());
}

#[test]
fn xbox_manifest_rejects_escaping_console_and_developer_entries() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("MicrosoftGame.config");
    fs::write(&path, r#"<Game><ExecutableList><Executable Name="..\outside.exe" TargetDeviceFamily="PC"/><Executable Name="C:\outside.exe" TargetDeviceFamily="PC"/><Executable Name="game.exe:stream.exe" TargetDeviceFamily="PC"/><Executable Name="console.exe" TargetDeviceFamily="Scarlett"/><Executable Name="dev.exe" TargetDeviceFamily="PC" IsDevOnly="true"/><Executable Name="arm.exe" TargetDeviceFamily="PC" Architecture="ARM64"/></ExecutableList></Game>"#).unwrap();
    let games = xbox::manifest(&path).unwrap();
    assert_eq!(games.len(), 1);
    assert_eq!(games[0].machine, 0xaa64);
    fs::write(&path, r#"<!DOCTYPE Game [<!ENTITY bad "oops">]><Game/>"#).unwrap();
    assert!(xbox::manifest(&path).is_err());
    fs::write(&path, "<Game><ExecutableList>").unwrap();
    assert!(xbox::manifest(&path).is_err());
}
