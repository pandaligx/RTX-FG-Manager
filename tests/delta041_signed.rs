//! Opt-in filesystem regression. Never loads the signed DLLs or a real game.
use anyhow::Result;
use rtx_fg_manager::{cleanup, cloud, core, delta, presets, upstream040};
use std::{collections::BTreeMap, fs, path::PathBuf, sync::atomic::AtomicBool};

#[test]
#[ignore = "requires the staged signed Delta 0.4.1 bundle and isolated LOCALAPPDATA"]
fn signed_delta041_all_entries_install_apply_cleanup_and_fresh_defaults() -> Result<()> {
    let local = std::env::var("LOCALAPPDATA")?;
    assert!(local.contains("isolated-appdata"));
    let root = std::env::var_os("RTXFG_DELTA_QA_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")));
    let modern = std::env::var_os("RTXFG_DELTA6_QA").is_some();
    let resources = std::env::var_os("RTXFG_DELTA6_QA")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("development/delta041-port/resources"));
    let catalog: cloud::Catalog =
        serde_json::from_slice(&fs::read(resources.join("rust/cloud-catalog.json"))?)?;
    catalog.validate()?;
    let sid = cloud::DELTA_041_SCHEME;
    let package = catalog
        .packages
        .iter()
        .find(|p| p.scheme_id == sid)
        .unwrap();
    let archive = fs::read(resources.join("files").join(&package.archive))?;
    for package in catalog.packages.iter().filter(|p| p.scheme_id == sid) {
        cloud::unpack(package, &archive)?;
    }
    let cache = rtx_fg_manager::assets::cache_root()?.join("cloud");
    fs::create_dir_all(&cache)?;
    fs::write(cache.join(format!("{}.zip", package.sha256)), archive)?;
    for entry in catalog.proxies(sid) {
        let temp = tempfile::tempdir()?;
        let dir = temp.path().join("三角洲/DeltaForce/Binaries/Win64");
        fs::create_dir_all(&dir)?;
        let exe = dir.join(delta::GAME);
        fs::copy(std::env::current_exe()?, &exe)?;
        let target = core::DeploymentTarget::for_game(&exe);
        let entries = vec![entry.clone()];
        let prepared = cloud::prepare_with_progress(
            &catalog,
            sid,
            1,
            &entries,
            &AtomicBool::new(false),
            |progress| {
                assert_ne!(
                    progress.transfer.phase,
                    rtx_fg_manager::transfer::Phase::Downloading
                )
            },
        )?;
        let context = presets::Context::new(sid, &prepared.policy, &exe);
        assert!(context.delta);
        let install = || {
            core::deploy_prepared_context_at_with_overrides(
                &target,
                &prepared.backend,
                &entries,
                None,
                prepared.files.clone(),
                Some(&prepared.version),
                Some(&context),
                None,
                prepared.upstream_version.as_deref(),
            )
        };
        if modern {
            let old: cloud::Catalog = serde_json::from_slice(&fs::read(
                std::env::var_os("RTXFG_DELTA_PREVIOUS_CATALOG")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| {
                        root.join("development/delta041-port/resources/rust/cloud-catalog.json")
                    }),
            )?)?;
            let previous = cloud::prepare_with_progress(
                &old,
                sid,
                1,
                &entries,
                &AtomicBool::new(false),
                |progress| {
                    assert_ne!(
                        progress.transfer.phase,
                        rtx_fg_manager::transfer::Phase::Downloading
                    );
                },
            )?;
            let old_context = presets::Context::new(sid, &previous.policy, &exe);
            core::deploy_prepared_context_at_with_overrides(
                &target,
                &previous.backend,
                &entries,
                None,
                previous.files,
                Some(&previous.version),
                Some(&old_context),
                None,
                previous.upstream_version.as_deref(),
            )?;
        }
        install()?;
        let record = core::record(&dir)?.unwrap();
        assert_eq!(record.scheme_id.as_deref(), Some(sid));
        assert_eq!(record.delta_cache_ids.len(), 1);
        assert_eq!(
            core::digest(&dir.join(&entry))?,
            core::hash(&prepared.files[&entry])
        );
        for file in prepared.files.keys().filter(|f| f.as_str() != core::INI) {
            assert_eq!(
                core::digest(&dir.join(file))?,
                core::hash(&prepared.files[file])
            );
        }
        let config = dir.join(core::INI);
        assert_eq!(
            upstream040::read_041(&fs::read(&config)?)?["optimized"],
            "1"
        );
        core::apply_parameters_at(
            &target,
            &context,
            &BTreeMap::from([
                ("optimized".into(), "2".into()),
                (
                    "max_generated_frames".into(),
                    if modern { "5" } else { "2" }.into(),
                ),
            ]),
        )?;
        let changed = fs::read(&config)?;
        if modern {
            assert_eq!(
                upstream040::read_041(&changed)?["max_generated_frames"],
                "5"
            );
            assert!(String::from_utf8_lossy(&changed).contains("DeltaForceGeneratedFrames=5"));
            assert_eq!(
                record.upstream_version.as_deref(),
                Some(delta::SIX_X_VERSION)
            );
        }
        install()?;
        assert_eq!(
            fs::read(&config)?,
            changed,
            "Reinstall preserves explicit settings"
        );
        let logs = dir.join("dlssg_sm86/logs");
        fs::create_dir_all(&logs)?;
        fs::write(logs.join("dlssg_sm86.log"), b"managed log")?;
        fs::write(dir.join("rtxfg-vulkan-bridge.log"), b"managed bridge log")?;
        fs::write(dir.join("unrelated-plugin.txt"), b"keep")?;
        let outcome = cleanup::clean_outcome_at(&target)?;
        assert!(outcome.complete, "{}: {}", entry, outcome.message);
        for file in prepared.files.keys() {
            assert!(!dir.join(file).exists(), "leftover {file}");
        }
        assert!(!dir.join("rtxfg-vulkan-bridge.log").exists());
        assert!(!logs.join("dlssg_sm86.log").exists());
        assert_eq!(fs::read(dir.join("unrelated-plugin.txt"))?, b"keep");
        assert!(exe.is_file());
        install()?;
        let fresh = upstream040::read_041(&fs::read(config)?)?;
        assert_eq!(fresh["optimized"], "1", "uninstall must discard old preset");
        assert_eq!(fresh["max_generated_frames"], "3");
        assert!(cleanup::clean_outcome_at(&target)?.complete);
    }
    Ok(())
}
