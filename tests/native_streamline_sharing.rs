//! End-to-end fixture with the shipped archives. Only writes under the test root.
use anyhow::{Context, Result};
use rtx_fg_manager::{
    cleanup, cloud, core, presets, recommendation_runtimes, runtime_files, upstream040,
};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};

#[test]
#[ignore = "requires an isolated manager cache containing the published Pipo and native SDK ZIPs"]
fn pipo_two_entries_and_shared_sdk_restore_byte_for_byte() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let cancel = AtomicBool::new(false);
    let catalog = cloud::bundled();
    let scheme = upstream040::PIPO_SCHEME;
    let entries = vec!["version.dll".into(), "dxgi.dll".into()];
    let prepare = |folder: &Path, update_plugin: bool| -> Result<core::DeploymentTarget> {
        fs::create_dir_all(folder)?;
        let exe = folder.join("Game.exe");
        let mut pe = vec![0u8; 512];
        pe[..2].copy_from_slice(b"MZ");
        pe[60..64].copy_from_slice(&128u32.to_le_bytes());
        pe[128..132].copy_from_slice(b"PE\0\0");
        pe[132..134].copy_from_slice(&0x8664u16.to_le_bytes());
        pe[148..150].copy_from_slice(&240u16.to_le_bytes());
        pe[150..152].copy_from_slice(&2u16.to_le_bytes());
        pe[152..154].copy_from_slice(&0x20bu16.to_le_bytes());
        fs::write(&exe, pe)?;
        let target = core::DeploymentTarget::for_game(&exe);
        let sdk = PathBuf::from(
            std::env::var_os("RTXFG_TEST_SL_COMMON")
                .context("set RTXFG_TEST_SL_COMMON to the official test plugin")?,
        );
        let mut original = fs::read(sdk)?;
        original.extend_from_slice(b"fixture original");
        fs::write(folder.join("sl.common.dll"), original)?;
        fs::write(
            folder.join("sl.interposer.dll"),
            b"game interposer: preserve",
        )?;
        let prepared = cloud::prepare(&catalog, scheme, 1, &entries, &cancel, |_| {})?;
        let ctx = presets::Context::new(scheme, &prepared.policy, &exe);
        core::deploy_prepared_context_at_with_overrides(
            &target,
            &prepared.backend,
            &entries,
            None,
            prepared.files,
            Some(&prepared.version),
            Some(&ctx),
            None,
            prepared.upstream_version.as_deref(),
        )?;
        assert!(folder.join("version.dll").is_file() && folder.join("dxgi.dll").is_file());
        assert!(runtime_files::inspect(&target, "streamline").available);
        if update_plugin {
            runtime_files::install_version(
                std::slice::from_ref(&target),
                "streamline",
                "2.14.16",
                false,
                &cancel,
                &mut |_| {},
            )?;
        }
        Ok(target)
    };
    let author = prepare(&dir.path().join("Author"), true)?;
    let recipient = prepare(&dir.path().join("Recipient"), false)?;
    let mut captured = recommendation_runtimes::capture(&author)?;
    captured.components.retain(|c| c.kind == "streamline");
    assert_eq!(captured.components.len(), 1);
    assert_eq!(
        captured.components[0].package_version.as_deref(),
        Some("2.14.16")
    );
    captured.compatible_with(scheme, &entries)?;
    // A second renderer without Streamline must not acquire SDK files.
    let other_dir = dir.path().join("RendererWithoutSL");
    fs::create_dir(&other_dir)?;
    let other_exe = other_dir.join("Other.exe");
    fs::copy(&recipient.game_exe, &other_exe)?;
    let other = core::DeploymentTarget::for_game(&other_exe);
    let targets = [recipient.clone(), other];
    captured.install(&targets, false, &cancel, &mut |_| {})?;
    captured.verify_installed(&targets)?;
    assert!(!other_dir.join("sl.common.dll").exists());
    assert!(!other_dir.join(".rtx-fg-runtimes").exists());
    runtime_files::remove_many(std::slice::from_ref(&author), "streamline")?;
    for target in [author, recipient] {
        cleanup::clean_at(&target)?;
        assert!(fs::read(target.directory.join("sl.common.dll"))?.ends_with(b"fixture original"));
        assert_eq!(
            fs::read(target.directory.join("sl.interposer.dll"))?,
            b"game interposer: preserve"
        );
        for file in [
            "version.dll",
            "dxgi.dll",
            "RTXFG-Streamline-LICENSE.txt",
            ".rtx-fg-runtimes",
        ] {
            assert!(!target.directory.join(file).exists(), "{file}");
        }
    }
    Ok(())
}
