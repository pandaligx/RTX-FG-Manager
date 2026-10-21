use rtx_fg_manager::{
    cleanup, cloud, core,
    deployment::{self, IssueKind},
    scanner::Game,
};
use std::{fs, path::Path};

fn game(path: &Path) -> core::DeploymentTarget {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut bytes = vec![0u8; 512];
    bytes[..2].copy_from_slice(b"MZ");
    bytes[60..64].copy_from_slice(&128u32.to_le_bytes());
    bytes[128..132].copy_from_slice(b"PE\0\0");
    bytes[132..134].copy_from_slice(&0x8664u16.to_le_bytes());
    fs::write(path, bytes).unwrap();
    core::DeploymentTarget::for_game(path)
}
fn record(target: &core::DeploymentTarget, pending: bool) {
    core::atomic_json(
        &target.directory.join(core::OWN).join(core::MARKER),
        &serde_json::json!({"schema":3,"backend":"upstream_sm86","proxy":"version.dll",
            "hashes":{"version.dll":core::hash(b"patch"),"dlssg_sm86.ini":core::hash(b"config")},
            "cache_pending":pending}),
    )
    .unwrap();
}
#[test]
fn explanations_distinguish_foreign_files_incomplete_installs_and_normal_configuration_changes() {
    assert_eq!(
        IssueKind::from_status("已有手动补丁或其他 MOD / 保留其他文件"),
        Some(IssueKind::External)
    );
    assert_eq!(
        IssueKind::from_status("未完成 / 可清理恢复"),
        Some(IssueKind::Interrupted)
    );
    assert_eq!(
        IssueKind::from_status("升级未完成 / 可恢复原安装"),
        Some(IssueKind::Interrupted)
    );
    assert_eq!(
        IssueKind::from_status("DLL 已调整 / 保留其他 MOD"),
        Some(IssueKind::Modified)
    );
    assert_eq!(
        IssueKind::from_status("需检查：无法访问"),
        Some(IssueKind::Unavailable)
    );
    assert_eq!(
        IssueKind::from_status("已部署 ENCORE / 配置已修改，可正常卸载"),
        None
    );
    assert_eq!(IssueKind::from_status("未部署"), None);
}
#[test]
fn manual_config_is_explained_without_offering_residual_deletion() {
    let t = tempfile::tempdir().unwrap();
    let target = game(&t.path().join("Game/game.exe"));
    fs::write(target.directory.join(core::INI), b"user config").unwrap();
    let g = Game {
        exe: target.game_exe.display().to_string(),
        ..Default::default()
    };
    let state = deployment::inspect(&g, &cloud::bundled());
    assert_eq!(state.issue_label(), Some("发现未受管理的文件"));
    assert!(!state.cleanup_pending);
    assert_eq!(
        fs::read(target.directory.join(core::INI)).unwrap(),
        b"user config"
    );
}
#[test]
fn grouped_and_old_folders_keep_residuals_separate_from_working_installation() {
    let t = tempfile::tempdir().unwrap();
    let active = game(&t.path().join("Active/game.exe"));
    let pending = game(&t.path().join("Pending/game.exe"));
    let old = game(&t.path().join("Old/game.exe"));
    record(&active, false);
    fs::write(active.directory.join("version.dll"), b"patch").unwrap();
    fs::write(active.directory.join(core::INI), b"config").unwrap();
    record(&pending, true);
    record(&old, true);
    let g = Game {
        exe: active.game_exe.display().to_string(),
        targets: vec![
            active.game_exe.display().to_string(),
            pending.game_exe.display().to_string(),
        ],
        cleanup_only: vec![old.game_exe.display().to_string()],
        ..Default::default()
    };
    let state = deployment::inspect(&g, &cloud::bundled());
    assert_eq!((state.total, state.installed), (2, 1));
    assert!(state.cleanup_pending);
    assert_eq!(state.issues().len(), 2);
    for issue in state.issues() {
        assert_eq!(issue.kind(), IssueKind::Residuals);
        assert_ne!(
            core::key(&issue.target().directory),
            core::key(&active.directory)
        );
        assert!(cleanup::retry_pending_at(issue.target()).unwrap().complete);
    }
    assert_eq!(
        fs::read(active.directory.join("version.dll")).unwrap(),
        b"patch"
    );
    let refreshed = deployment::inspect(&g, &cloud::bundled());
    assert!(!refreshed.cleanup_pending && refreshed.issues().is_empty());
}
#[test]
fn residual_retry_clears_stages_but_preserves_nr_saves_backups_and_other_mods() {
    let t = tempfile::tempdir().unwrap();
    let target = game(&t.path().join("Game/game.exe"));
    record(&target, true);
    let stage = target
        .directory
        .join(core::OWN)
        .join(core::stage_name("version.dll"));
    fs::write(&stage, b"patch").unwrap();
    for file in ["nvngx_dlssnr.dll", "OtherMod.dll", "save.dat"] {
        fs::write(target.directory.join(file), b"keep").unwrap();
    }
    fs::write(
        target.directory.join(core::OWN).join("original.dll.backup"),
        b"original",
    )
    .unwrap();
    assert!(cleanup::retry_pending_at(&target).unwrap().complete);
    assert!(!stage.exists() && core::record(&target.directory).unwrap().is_none());
    for file in ["nvngx_dlssnr.dll", "OtherMod.dll", "save.dat"] {
        assert_eq!(fs::read(target.directory.join(file)).unwrap(), b"keep");
    }
    assert_eq!(
        fs::read(target.directory.join(core::OWN).join("original.dll.backup")).unwrap(),
        b"original"
    );
}
#[test]
fn stale_residual_action_never_removes_a_reinstalled_patch() {
    let t = tempfile::tempdir().unwrap();
    let target = game(&t.path().join("Game/game.exe"));
    record(&target, false);
    fs::write(target.directory.join("version.dll"), b"patch").unwrap();
    assert!(cleanup::retry_pending_at(&target).is_err());
    // Even if an old pending flag remains, a newly present patch stops cleanup.
    record(&target, true);
    assert!(cleanup::retry_pending_at(&target).is_err());
    assert_eq!(
        fs::read(target.directory.join("version.dll")).unwrap(),
        b"patch"
    );
    assert!(
        core::record(&target.directory)
            .unwrap()
            .unwrap()
            .cache_pending
    );
}
#[test]
fn restored_original_entry_does_not_block_residual_cleanup() {
    let t = tempfile::tempdir().unwrap();
    let target = game(&t.path().join("Game/game.exe"));
    record(&target, true);
    fs::write(target.directory.join("version.dll"), b"restored original").unwrap();
    assert!(cleanup::retry_pending_at(&target).unwrap().complete);
    assert_eq!(
        fs::read(target.directory.join("version.dll")).unwrap(),
        b"restored original"
    );
    assert!(core::record(&target.directory).unwrap().is_none());
}
#[test]
fn legacy_empty_proxy_list_recovers_only_with_a_valid_explicit_entry_and_file_set() {
    let t = tempfile::tempdir().unwrap();
    let target = game(&t.path().join("Game/game.exe"));
    record(&target, true);
    let marker = target.directory.join(core::OWN).join(core::MARKER);
    let mut value: serde_json::Value = serde_json::from_slice(&fs::read(&marker).unwrap()).unwrap();
    value["proxies"] = serde_json::json!([]);
    core::atomic_json(&marker, &value).unwrap();
    let legacy = core::record(&target.directory).unwrap().unwrap();
    assert_eq!(legacy.selected(), ["version.dll"]);
    core::atomic_json(&marker, &legacy).unwrap();
    assert!(
        core::record(&target.directory)
            .unwrap()
            .unwrap()
            .cache_pending
    );
    value.as_object_mut().unwrap().remove("proxy");
    core::atomic_json(&marker, &value).unwrap();
    assert!(core::record(&target.directory).is_err());
    value["proxy"] = serde_json::json!("version.dll");
    value["hashes"]["save.dat"] = serde_json::json!(core::hash(b"keep"));
    core::atomic_json(&marker, &value).unwrap();
    assert!(core::record(&target.directory).is_err());
}
#[test]
fn locked_residual_retains_recovery_record_for_retry() {
    use std::os::windows::fs::OpenOptionsExt;
    let t = tempfile::tempdir().unwrap();
    let target = game(&t.path().join("Game/game.exe"));
    record(&target, true);
    let stage = target
        .directory
        .join(core::OWN)
        .join(core::stage_name("version.dll"));
    fs::write(&stage, b"patch").unwrap();
    let hold = fs::OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(&stage)
        .unwrap();
    assert!(cleanup::retry_pending_at(&target).is_err());
    assert!(
        stage.exists()
            && core::record(&target.directory)
                .unwrap()
                .unwrap()
                .cache_pending
    );
    drop(hold);
    assert!(cleanup::retry_pending_at(&target).unwrap().complete);
}
