//! Local signed-package file-system regressions. Does not execute a game or DLL.
use anyhow::Result;
use presets::Values;
use rtx_fg_manager::{anr, cleanup, cloud, core, presets, recommendation_presets, transfer};
use std::{fs, path::PathBuf, sync::atomic::AtomicBool};

fn prepare(catalog: &cloud::Catalog, scheme: &str, proxy: &str) -> Result<cloud::Prepared> {
    let mut downloaded = false;
    let payload = cloud::prepare_with_progress(
        catalog,
        scheme,
        1,
        &[proxy.into()],
        &AtomicBool::new(false),
        |p| downloaded |= p.transfer.phase == transfer::Phase::Downloading,
    )?;
    anyhow::ensure!(
        !downloaded,
        "Only preloaded local signed fixtures may be used"
    );
    Ok(payload)
}

#[test]
#[ignore = "requires signed 0.4.2 and ANR packages in the isolated maintainer cache"]
fn signed_anr_full_install_parameters_cleanup_and_defaults() -> Result<()> {
    let root = PathBuf::from(std::env::var("RTXFG_4334_INPUT")?);
    anyhow::ensure!(std::env::var("LOCALAPPDATA")?.contains("development"));
    let catalog = cloud::bundled();
    for proxy in anr::PROXIES {
        let payload = prepare(&catalog, anr::SCHEME, proxy)?;
        assert_eq!(payload.backend, anr::BACKEND);
        let tmp = tempfile::tempdir_in(root.join("development/v4334-dlss042-anr"))?;
        let exe = tmp.path().join("ANR-test.exe");
        fs::copy(std::env::current_exe()?, &exe)?;
        let target = core::DeploymentTarget::for_game(&exe);
        let context = presets::Context::new(anr::SCHEME, &payload.policy, &exe);
        let install = || {
            core::deploy_prepared_context_at_with_overrides(
                &target,
                &payload.backend,
                &[proxy.into()],
                None,
                payload.files.clone(),
                Some(&payload.version),
                Some(&context),
                Some(&Values::new()),
                payload.upstream_version.as_deref(),
            )
        };
        // Existing OptiScaler is never adopted, even if its INI matches the package.
        fs::write(tmp.path().join("OptiScaler.ini"), b"[user]\ncustom=1\n")?;
        assert!(install().is_err());
        assert!(!tmp.path().join(core::OWN).exists());
        assert_eq!(
            fs::read(tmp.path().join("OptiScaler.ini"))?,
            b"[user]\ncustom=1\n"
        );
        fs::remove_file(tmp.path().join("OptiScaler.ini"))?;
        install()?;
        assert!(core::status_at(&target).starts_with("已部署"));
        for (name, bytes) in &payload.files {
            assert_eq!(fs::read(tmp.path().join(name))?, *bytes);
        }
        let changes = Values::from([
            ("max_generated".into(), "2".into()),
            ("scene_cut".into(), "0".into()),
        ]);
        core::apply_parameters_at(&target, &context, &changes)?;
        let snap = recommendation_presets::capture(&target, &catalog, anr::SCHEME, "0.0.15-B15")?;
        assert_eq!(snap.values["max_generated"], "2");
        snap.validate(anr::SCHEME, "0.0.15-B15")?;
        // Menu changes in the bundled host config remain valid and survive reapply.
        let host = tmp.path().join("OptiScaler.ini");
        let mut bytes = fs::read(&host)?;
        bytes.extend_from_slice(b"\n; menu change\n");
        fs::write(&host, &bytes)?;
        assert!(core::status_at(&target).starts_with("已部署"));
        install()?;
        assert_eq!(fs::read(&host)?, bytes);
        assert_eq!(
            anr::read(&fs::read(tmp.path().join(anr::CONFIG))?)?["max_generated"],
            "2"
        );
        fs::write(tmp.path().join("OptiScaler.log"), b"host log")?;
        // An interrupted nested-config write must not leave the ownership directory.
        fs::write(
            tmp.path()
                .join(core::OWN)
                .join(format!("{}.config-stage", anr::CONFIG.replace('/', "__"))),
            b"max_generated = 2\n",
        )?;
        fs::write(
            tmp.path().join(anr::PROVIDER_DIR).join("nr_fg-session.log"),
            b"provider log",
        )?;
        let unknown = tmp.path().join(anr::PROVIDER_DIR).join("capture.raw");
        fs::write(&unknown, b"keep")?;
        let other = tmp.path().join("OptiScaler/custom/UserPlugin");
        fs::create_dir_all(&other)?;
        fs::write(other.join("custom.dll"), b"keep")?;
        assert!(cleanup::clean_outcome_at(&target)?.complete);
        assert!(!tmp.path().join(core::OWN).exists());
        assert!(!tmp.path().join(anr::CONFIG).exists());
        assert!(!host.exists() && !tmp.path().join("OptiScaler.log").exists());
        assert!(
            !tmp.path()
                .join(anr::PROVIDER_DIR)
                .join("nr_fg-session.log")
                .exists()
        );
        assert_eq!(fs::read(&unknown)?, b"keep");
        assert_eq!(fs::read(other.join("custom.dll"))?, b"keep");
        // The intentionally retained unknown plugin blocks a full reinstall.
        assert!(install().is_err());
        fs::remove_file(unknown)?;
        fs::remove_file(other.join("custom.dll"))?;
        fs::remove_dir(other)?;
        let mut dirs = Vec::new();
        fn empty_tree(path: &std::path::Path, dirs: &mut Vec<PathBuf>) -> Result<()> {
            for e in fs::read_dir(path)? {
                let p = e?.path();
                if p.is_dir() {
                    empty_tree(&p, dirs)?;
                    dirs.push(p);
                }
            }
            Ok(())
        }
        empty_tree(&tmp.path().join("OptiScaler"), &mut dirs)?;
        for p in dirs {
            fs::remove_dir(p)?;
        }
        fs::remove_dir(tmp.path().join("OptiScaler"))?;
        install()?;
        assert_eq!(
            fs::read(tmp.path().join(anr::CONFIG))?,
            payload.files[anr::CONFIG]
        );
        let model = tmp.path().join(anr::COMPONENTS[6]);
        fs::write(&model, b"user replacement model")?;
        assert!(!cleanup::clean_outcome_at(&target)?.complete);
        assert_eq!(fs::read(&model)?, b"user replacement model");
        // Simulate the user approving/removing the changed test-only file.
        fs::remove_file(model)?;
        assert!(cleanup::retry_pending_at(&target)?.complete);
        assert!(!tmp.path().join("OptiScaler").exists());
    }
    Ok(())
}

#[test]
#[ignore = "requires signed 0.4.1 and 0.4.2 archives in isolated cache"]
fn signed_dlss042_upgrade_keeps_edits_fresh_install_uses_upstream_defaults() -> Result<()> {
    let root = PathBuf::from(std::env::var("RTXFG_4334_INPUT")?);
    let old: cloud::Catalog = serde_json::from_slice(&fs::read(
        root.join("development/v4334-dlss042-anr/before-catalog.json"),
    )?)?;
    let catalog = cloud::bundled();
    let tmp = tempfile::tempdir_in(root.join("development/v4334-dlss042-anr"))?;
    let exe = tmp.path().join("DlssUpgrade.exe");
    fs::copy(std::env::current_exe()?, &exe)?;
    let target = core::DeploymentTarget::for_game(&exe);
    for proxy in rtx_fg_manager::upstream040::PROXIES {
        let old = prepare(&old, rtx_fg_manager::upstream040::SCHEME, proxy)?;
        let new = prepare(&catalog, rtx_fg_manager::upstream040::SCHEME, proxy)?;
        let context = presets::Context::new(rtx_fg_manager::upstream040::SCHEME, &new.policy, &exe);
        let install = |p: &cloud::Prepared| {
            core::deploy_prepared_context_at_with_overrides(
                &target,
                &p.backend,
                &[proxy.into()],
                None,
                p.files.clone(),
                Some(&p.version),
                Some(&context),
                Some(&Values::new()),
                p.upstream_version.as_deref(),
            )
        };
        install(&old)?;
        core::apply_parameters_at(
            &target,
            &context,
            &Values::from([
                ("optimized".into(), "2".into()),
                ("max_generated_frames".into(), "2".into()),
            ]),
        )?;
        install(&new)?;
        let values =
            presets::read_values(&fs::read(tmp.path().join(core::INI))?, &context.profile)?;
        assert_eq!(values["optimized"], "2");
        assert_eq!(values["max_generated_frames"], "2");
        assert!(cleanup::clean_outcome_at(&target)?.complete);
        install(&new)?;
        assert_eq!(fs::read(tmp.path().join(core::INI))?, new.files[core::INI]);
        let values = presets::read_values(&new.files[core::INI], &context.profile)?;
        assert_eq!(values["optimized"], "1");
        assert_eq!(values["xess_hudless"], "Auto");
        assert!(cleanup::clean_outcome_at(&target)?.complete);
        assert!(!tmp.path().join("dlssg_sm86").exists());
    }
    Ok(())
}
