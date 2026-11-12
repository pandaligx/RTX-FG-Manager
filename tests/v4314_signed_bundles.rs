//! Explicit maintainer check against the two staged, signed resource bundles.
//! No DLL is loaded, no real game is changed and this is not GPU validation.
use anyhow::Result;
use rtx_fg_manager::{cleanup, cloud, core, encore, presets, upstream040};
use std::{collections::BTreeMap, fs, sync::atomic::AtomicBool};

#[test]
#[ignore = "requires staged bundle cache in an isolated LOCALAPPDATA"]
fn staged_signed_bundles_install_apply_reinstall_and_uninstall() -> Result<()> {
    let cache = std::env::var("LOCALAPPDATA")?;
    assert!(cache.contains("v4314"));
    let catalog = cloud::bundled();
    for scheme in [upstream040::SCHEME, encore::SCHEME] {
        let temp = tempfile::tempdir()?;
        let exe = temp.path().join("FixtureGame.exe");
        fs::copy(std::env::current_exe()?, &exe)?;
        let target = core::DeploymentTarget::for_game(&exe);
        let entries = vec!["version.dll".into()];
        let prepared = cloud::prepare(
            &catalog,
            scheme,
            1,
            &entries,
            &AtomicBool::new(false),
            |_| {},
        )?;
        let context = presets::Context::new(scheme, &prepared.policy, &exe);
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
        install()?;
        assert!(core::status_at(&target).starts_with("已部署"));
        for (name, bytes) in &prepared.files {
            assert_eq!(
                core::digest(&target.directory.join(name))?,
                core::hash(bytes),
                "{name}"
            );
        }
        let config = target.directory.join(core::config_name(&prepared.backend));
        let mut text = fs::read_to_string(&config)?;
        text.push_str(if scheme == upstream040::SCHEME {
            "\n; keep user comment\n[UserCustom]\nKeep=123\n"
        } else {
            "\n// keep user comment\n"
        });
        fs::write(&config, text)?;
        let values = BTreeMap::from([(
            if scheme == upstream040::SCHEME {
                "xess_hudless"
            } else {
                "smoothMotionEnabled"
            }
            .into(),
            "1".into(),
        )]);
        core::apply_parameters_at(&target, &context, &values)?;
        let applied = fs::read(&config)?;
        install()?;
        assert_eq!(
            fs::read(&config)?,
            applied,
            "Reinstall must preserve user configuration"
        );
        fs::write(temp.path().join("other-mod.txt"), b"preserve")?;
        if scheme == upstream040::SCHEME {
            let logs = temp.path().join("dlssg_sm86/logs");
            fs::create_dir_all(&logs)?;
            fs::write(logs.join("dlssg_sm86.log"), b"log")?;
            fs::write(logs.join("frame_timing.txt"), b"timing")?;
        }
        let outcome = cleanup::clean_outcome_at(&target)?;
        assert!(outcome.complete, "{}", outcome.message);
        for name in prepared.files.keys() {
            assert!(!target.directory.join(name).exists(), "{name}");
        }
        assert_eq!(fs::read(temp.path().join("other-mod.txt"))?, b"preserve");
        assert!(exe.is_file());
        assert!(!temp.path().join("dlssg_sm86/logs/dlssg_sm86.log").exists());
    }
    Ok(())
}
