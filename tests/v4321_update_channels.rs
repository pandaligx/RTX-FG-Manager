use anyhow::{Context, Result, bail, ensure};
use rtx_fg_manager::{core, updater, win};
use std::{
    fs,
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};

fn manifest(path: &Path, schema: u32, version: &str) -> Result<updater::Manifest> {
    let file = if schema == 1 {
        format!("RTXManager-v{version}-x64.exe")
    } else {
        "RTXManager.exe".into()
    };
    Ok(updater::Manifest {
        schema,
        version: version.into(),
        url: format!(
            "https://github.com/{}/releases/download/v{version}/{file}",
            updater::REPO
        ),
        file,
        source: "github".into(),
        bytes: fs::metadata(path)?.len(),
        sha256: core::digest(path)?,
        notes: Default::default(),
    })
}

fn launch_signature_probe(exe: &Path, report: &Path) -> Result<()> {
    let mut child = Command::new(exe)
        .args(["--signature".as_ref(), exe.as_os_str(), report.as_os_str()])
        .env_remove("RTXFG_UPDATE_READY")
        .env_remove("RTXFG_RENAME_READY")
        .creation_flags(0x08000000)
        .spawn()?;
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            ensure!(status.success() && report.is_file(), "signed probe failed");
            return Ok(());
        }
        if start.elapsed() > Duration::from_secs(30) {
            let _ = child.kill();
            let _ = child.wait();
            bail!("signed probe timed out");
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[test]
#[ignore = "Requires two local signed historical EXEs and the unsigned candidate; no downloads"]
fn signed_baselines_cover_fixed_name_cache_launch_and_rollback() -> Result<()> {
    let path = |key| -> Result<PathBuf> { Ok(std::env::var_os(key).context(key)?.into()) };
    let old = path("RTXFG_UPDATE_OLD_FIXTURE")?;
    let new = path("RTXFG_UPDATE_NEW_FIXTURE")?;
    let candidate = path("RTXFG_UPDATE_UNSIGNED_CANDIDATE")?;
    let old_version = win::verify_signature(&old)?.version;
    let new_version = win::verify_signature(&new)?.version;
    ensure!(updater::version(&old_version)? < updater::version(&new_version)?);
    let temp = tempfile::tempdir()?;
    let data = temp.path().join("profile");
    let mut cached = Vec::new();
    for (exe, version) in [(&old, &old_version), (&new, &new_version)] {
        for schema in [1, 2] {
            updater::verify(exe, &manifest(exe, schema, version)?)?;
        }
        let m = manifest(exe, 2, version)?;
        let folder = data.join("updates").join(&m.sha256[..32]);
        fs::create_dir_all(&folder)?;
        fs::copy(exe, folder.join(&m.file))?;
        cached.push(updater::download_with_preference(
            &m,
            &data,
            "github",
            &AtomicBool::new(false),
            |_| {},
        )?);
    }
    assert_ne!(cached[0], cached[1]);
    let mut wrong_version = manifest(&new, 2, rtx_fg_manager::VERSION)?;
    assert!(updater::verify(&new, &wrong_version).is_err());
    wrong_version.sha256 = "0".repeat(64);
    assert!(updater::verify(&new, &wrong_version).is_err());
    // Exact candidate bytes still fail the production signature gate. No test
    // switch in the shipped app can bypass this or impersonate a future build.
    assert!(
        updater::verify(
            &candidate,
            &manifest(&candidate, 2, rtx_fg_manager::VERSION)?
        )
        .is_err()
    );

    for name in [
        "RTXManager-v4.3.18-x64.exe",
        "RTXManager.exe",
        "我的管理器.exe",
    ] {
        let target = temp.path().join(name);
        let stage = temp.path().join("stage.new");
        let backup = temp.path().join("backup.old");
        let neighbour = temp.path().join("unrelated.exe");
        fs::write(&neighbour, b"unrelated")?;
        fs::copy(&old, &target)?;
        for fail in [true, false] {
            fs::copy(&new, &stage)?;
            let report = temp.path().join(format!("{name}-{fail}.json"));
            let result =
                updater::replace_and_launch(&target, &stage, &backup, &core::digest(&old)?, || {
                    launch_signature_probe(&target, &report)?;
                    if fail {
                        bail!("simulated missing ready acknowledgement");
                    }
                    Ok(())
                });
            assert_eq!(result.is_err(), fail);
            assert_eq!(
                core::digest(&target)?,
                core::digest(if fail { &old } else { &new })?
            );
            assert!(!backup.exists());
            assert_eq!(fs::read(&neighbour)?, b"unrelated");
        }
    }
    Ok(())
}
