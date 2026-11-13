//! Real Windows inherited file handles, synthetic game files. No NVIDIA binary,
//! driver, real game process, or user file is terminated or modified.
use anyhow::{Context, Result, ensure};
use rtx_fg_manager::{cleanup, core, encore, presets, win};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    os::windows::{
        fs::OpenOptionsExt,
        io::{AsRawHandle, FromRawHandle},
        process::CommandExt,
    },
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

#[test]
fn inherited_lock_child() -> Result<()> {
    let Some(control) = std::env::var_os("RTXFG_TEST_INHERITED_HANDLE") else {
        return Ok(());
    };
    let control = PathBuf::from(control);
    // SAFETY: This test-only child receives this handle exclusively as its
    // inherited stdin. It never accesses stdin again; File closes it once.
    let mut file = Some(unsafe { File::from_raw_handle(std::io::stdin().as_raw_handle()) });
    fs::write(control.with_extension("ready"), b"inherited")?;
    let deadline = Instant::now() + Duration::from_secs(90);
    while Instant::now() < deadline {
        if control.with_extension("release").exists() {
            drop(file.take());
            fs::write(control.with_extension("released"), b"released")?;
        }
        if control.with_extension("quit").exists() {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(20));
    }
    Ok(())
}

struct Holder {
    child: Child,
    control: PathBuf,
}
impl Drop for Holder {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
fn wait_file(path: &Path) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !path.exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
    }
    ensure!(
        path.exists(),
        "child readiness timed out: {}",
        path.display()
    );
    Ok(())
}
fn hold(path: &Path, control: PathBuf) -> Result<Holder> {
    hold_with_sharing(path, control, 3)
}
fn hold_with_sharing(path: &Path, control: PathBuf, sharing: u32) -> Result<Holder> {
    // Sharing reads/writes but not deletion recreates the Explorer screenshot.
    // Only the parent opens this path; the child inherits the handle via stdin.
    let file = OpenOptions::new()
        .read(true)
        .share_mode(sharing)
        .open(path)?;
    let child = Command::new(std::env::current_exe()?)
        .args(["--exact", "inherited_lock_child", "--nocapture"])
        .env("RTXFG_TEST_INHERITED_HANDLE", &control)
        .stdin(Stdio::from(file))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(0x08000000)
        .spawn()?;
    let holder = Holder { child, control };
    wait_file(&holder.control.with_extension("ready"))?;
    Ok(holder)
}
fn pe(dll: bool) -> Vec<u8> {
    let mut b = vec![0u8; 512];
    b[..2].copy_from_slice(b"MZ");
    b[60..64].copy_from_slice(&128u32.to_le_bytes());
    b[128..132].copy_from_slice(b"PE\0\0");
    b[132..134].copy_from_slice(&0x8664u16.to_le_bytes());
    b[148..150].copy_from_slice(&240u16.to_le_bytes());
    b[150..152].copy_from_slice(&(if dll { 0x2000u16 } else { 2 }).to_le_bytes());
    b[152..154].copy_from_slice(&0x20bu16.to_le_bytes());
    b
}
fn install(target: &core::DeploymentTarget) -> Result<()> {
    let mut files = BTreeMap::from([
        ("version.dll".into(), pe(true)),
        (
            encore::CONFIG.into(),
            br#"{"configVersion":4,"frameGeneration":{"mode":"game","multiplier":4}}"#.to_vec(),
        ),
        (encore::NOTICES.into(), b"license fixture".to_vec()),
    ]);
    for name in encore::INTEL_FILES {
        files.insert(
            name.into(),
            if name.ends_with(".dll") {
                pe(true)
            } else {
                b"Intel license".to_vec()
            },
        );
    }
    core::deploy_prepared_context_at_with_overrides(
        target,
        encore::BACKEND,
        &["version.dll".into()],
        None,
        files,
        Some("2.0.1"),
        Some(&presets::Context {
            scheme: encore::SCHEME.into(),
            profile: encore::PROFILE.into(),
            delta: false,
            delta_capable: false,
            delta_six: false,
        }),
        None,
        Some("2.0.1"),
    )?;
    Ok(())
}

#[test]
fn inherited_external_log_handles_are_found_before_cleanup_and_one_retry_finishes() -> Result<()> {
    for custom in [false, true] {
        let temp = tempfile::tempdir()?;
        let dir = temp.path().join("game");
        fs::create_dir(&dir)?;
        let exe = dir.join("SyntheticGame.exe");
        fs::write(&exe, pe(false))?;
        let target = if custom {
            let plugin = dir.join("plugins");
            fs::create_dir(&plugin)?;
            core::DeploymentTarget::custom(&exe, &plugin)
        } else {
            core::DeploymentTarget::for_game(&exe)
        };
        install(&target)?;
        let logdir = target.directory.join("rtx-encore-logs");
        fs::create_dir(&logdir)?;
        let log = logdir.join("rtx-encore.log");
        fs::write(&log, b"encore session")?;
        let other = temp.path().join("other-game.log");
        fs::write(&other, b"unrelated")?;
        let unknown = logdir.join("my-notes.log");
        fs::write(&unknown, b"user notes")?;
        let mut unrelated = hold(&other, temp.path().join("unrelated"))?;
        let mut unknown_holder = hold(&unknown, temp.path().join("unknown"))?;
        let a = hold(&log, temp.path().join("holder-a"))?;
        let b = hold(&log, temp.path().join("holder-b"))?;
        assert_eq!(fs::remove_file(&log).unwrap_err().raw_os_error(), Some(32));
        assert!(
            core::blocking_processes(&target)?.is_empty(),
            "old directory scan misses the external helpers"
        );
        let found = cleanup::blocking_processes(&target)?;
        assert_eq!(
            found.iter().map(|p| p.pid).collect::<BTreeSet<_>>(),
            BTreeSet::from([a.child.id(), b.child.id()])
        );
        assert!(
            found
                .iter()
                .all(|p| p.can_terminate() && p.holds_patch_files())
        );
        let marker = target.directory.join(core::OWN).join(core::MARKER);
        let before = fs::read(&marker)?;
        assert!(
            cleanup::clean_outcome_at(&target)
                .unwrap_err()
                .to_string()
                .contains("占用")
        );
        assert_eq!(fs::read(&marker)?, before);
        assert!(target.directory.join("version.dll").exists());
        let mut stale = found[0].clone();
        stale.created = stale.created.map(|t| t + 1);
        assert!(
            win::terminate_confirmed(&stale).is_err(),
            "PID/time mismatch must not kill"
        );
        for p in &found {
            win::terminate_confirmed(p)?;
        }
        assert!(unrelated.child.try_wait()?.is_none());
        assert!(unknown_holder.child.try_wait()?.is_none());
        assert!(cleanup::clean_outcome_at(&target)?.complete);
        for name in [
            encore::CONFIG,
            core::OWN,
            "version.dll",
            "rtx-encore-runtimes",
        ] {
            assert!(!target.directory.join(name).exists(), "{name}");
        }
        assert!(!log.exists());
        assert_eq!(fs::read(&unknown)?, b"user notes");
        drop(unknown_holder);
        fs::remove_file(&unknown)?;
        fs::remove_dir(&logdir)?;
        // Also reproduce an old partial uninstall with proxy removed and record retained.
        install(&target)?;
        fs::create_dir(&logdir)?;
        fs::write(&log, b"legacy session")?;
        fs::remove_file(target.directory.join("version.dll"))?;
        let holder = hold(&log, temp.path().join("old"))?;
        let found = cleanup::blocking_processes(&target)?;
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].pid, holder.child.id());
        win::terminate_confirmed(&found[0])?;
        assert!(cleanup::clean_outcome_at(&target)?.complete);
        for name in [
            encore::CONFIG,
            core::OWN,
            "rtx-encore-logs",
            "rtx-encore-runtimes",
        ] {
            assert!(!target.directory.join(name).exists(), "{name}");
        }
        assert!(unrelated.child.try_wait()?.is_none());
    }
    Ok(())
}

#[test]
fn released_file_handle_does_not_authorize_terminating_still_running_helper() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("locked.log");
    fs::write(&path, b"data")?;
    let mut holder = hold(&path, temp.path().join("holder"))?;
    let p = win::processes_holding_files(std::slice::from_ref(&path))?
        .into_iter()
        .find(|p| p.pid == holder.child.id())
        .context("missing holder")?;
    fs::write(holder.control.with_extension("release"), b"release")?;
    wait_file(&holder.control.with_extension("released"))?;
    win::terminate_confirmed(&p)?;
    assert!(
        holder.child.try_wait()?.is_none(),
        "released helper must not be terminated"
    );
    fs::remove_file(path)?;
    Ok(())
}

#[test]
fn readers_allowing_delete_are_not_treated_as_blockers() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("readable.log");
    fs::write(&path, b"data")?;
    let mut holder = hold_with_sharing(&path, temp.path().join("reader"), 7)?;
    assert!(win::processes_holding_files(std::slice::from_ref(&path))?.is_empty());
    fs::remove_file(&path)?;
    assert!(holder.child.try_wait()?.is_none());
    Ok(())
}
