//! Explicit local signed-payload regression. Never starts a game or loads a DLL.
use anyhow::{Result, ensure};
use rtx_fg_manager::{
    cleanup, cloud, core, encore,
    encore_components::{self, Components},
    presets, runtime_files, transfer,
};
use std::{fs, path::PathBuf, sync::atomic::AtomicBool};

#[test]
#[ignore = "uses the existing isolated signed Encore cache; no game or DLL execution"]
fn signed_encore_optional_components_preserve_native_files_and_restore_cleanly() -> Result<()> {
    let cache = std::env::var("LOCALAPPDATA")?;
    ensure!(
        cache.contains("development")
            && (cache.contains("v4327-presets") || cache.contains("v4328")),
        "isolated cache required"
    );
    let output = PathBuf::from(std::env::var("RTXFG_COMPONENT_QA")?);
    let tmp = tempfile::tempdir_in(&output)?;
    let catalog = cloud::bundled();
    let cancel = AtomicBool::new(false);
    let mut network = false;
    let mut payload = cloud::prepare_with_progress(
        &catalog,
        encore::SCHEME,
        1,
        &["version.dll".into()],
        &cancel,
        |p| {
            network |= p.transfer.phase == transfer::Phase::Downloading;
        },
    )?;
    ensure!(!network, "expected verified local cache");
    let mut localized = presets::Values::new();
    encore::localize_defaults(&mut localized, "zh-CN");
    payload.files.insert(
        encore::CONFIG.into(),
        encore::configure_for_version(
            &payload.files[encore::CONFIG],
            &localized,
            payload.upstream_version.as_deref(),
        )?,
    );
    let mut reports = Vec::new();
    for name in ["DX12", "SecondExecutable"] {
        let dir = tmp.path().join(name);
        fs::create_dir(&dir)?;
        let exe = dir.join("ComponentTest.exe");
        fs::copy(std::env::current_exe()?, &exe)?;
        let target = core::DeploymentTarget::for_game(&exe);
        let context = presets::Context::new(encore::SCHEME, &payload.policy, &exe);
        for file in [
            "sl.common.dll",
            "sl.interposer.dll",
            "sl.dlss_g.dll",
            "nvngx_dlssg.dll",
        ] {
            fs::write(dir.join(file), format!("native original {file}"))?;
        }
        let patch_only = Components {
            patch: true,
            sr: false,
            rr: false,
            fg: false,
            nr: false,
            intel: false,
            streamline: false,
            added_fg: false,
        };
        encore_components::deploy(
            &target,
            &payload,
            &["version.dll".into()],
            payload.files.clone(),
            &context,
            &presets::Values::new(),
            patch_only,
        )?;
        let original_config = fs::read(dir.join(encore::CONFIG))?;
        ensure!(encore::read(&original_config)?["menuLanguage"] == "zh-CN");
        runtime_files::complete_encore_selected(
            std::slice::from_ref(&target),
            patch_only,
            false,
            &cancel,
            &mut |_| {},
        )?;
        ensure!(!dir.join(encore::INTEL_FILES[0]).exists());
        ensure!(!dir.join("nvngx_dlss.dll").exists() && !dir.join("nvngx_dlssnr.dll").exists());
        let add_intel = Components {
            patch: false,
            intel: true,
            ..patch_only
        };
        encore_components::deploy(
            &target,
            &payload,
            &["dxgi.dll".into()],
            payload.files.clone(),
            &context,
            &presets::Values::new(),
            add_intel,
        )?;
        ensure!(!dir.join("dxgi.dll").exists());
        ensure!(dir.join(encore::INTEL_FILES[0]).is_file());
        ensure!(fs::read(dir.join(encore::CONFIG))? == original_config);
        for file in [
            "sl.common.dll",
            "sl.interposer.dll",
            "sl.dlss_g.dll",
            "nvngx_dlssg.dll",
        ] {
            ensure!(fs::read(dir.join(file))? == format!("native original {file}").as_bytes());
        }
        ensure!(cleanup::clean_outcome_at(&target)?.complete);
        for file in [
            "sl.common.dll",
            "sl.interposer.dll",
            "sl.dlss_g.dll",
            "nvngx_dlssg.dll",
        ] {
            ensure!(fs::read(dir.join(file))? == format!("native original {file}").as_bytes());
        }
        ensure!(
            !dir.join(encore::CONFIG).exists()
                && !dir.join(core::OWN).exists()
                && !dir.join(encore::INTEL_FILES[0]).exists()
        );
        // Fresh manual installation after uninstall must start with the upstream defaults.
        encore_components::deploy(
            &target,
            &payload,
            &["version.dll".into()],
            payload.files.clone(),
            &context,
            &presets::Values::new(),
            patch_only,
        )?;
        ensure!(fs::read(dir.join(encore::CONFIG))? == original_config);
        ensure!(cleanup::clean_outcome_at(&target)?.complete);
        reports.push(serde_json::json!({"directory":name,"native_files_preserved":true,"intel_only_preserved_config_and_entry":true,"uninstall_and_reinstall":true}));
    }
    core::atomic_json(
        &output.join("signed-components.json"),
        &serde_json::json!({"patch_version":payload.version,"network_download":network,"results":reports,"dll_execution":false}),
    )?;
    Ok(())
}
