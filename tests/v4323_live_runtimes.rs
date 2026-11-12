//! Opt-in test against a user-authorized, already installed Encore game.
//! This changes runtimes; launching and verifying the game is a separate step.
use anyhow::{Context, Result, ensure};
use rtx_fg_manager::{cleanup, cloud, core, encore, neural_runtime, presets, runtime_files};
use std::{path::PathBuf, sync::atomic::AtomicBool};

#[test]
#[ignore = "changes a real game; requires explicit RTXFG_LIVE_GAME and RTXFG_LIVE_ACTION"]
fn authorized_runtime_completion_or_restore() -> Result<()> {
    let exe = PathBuf::from(std::env::var("RTXFG_LIVE_GAME").context("explicit game required")?);
    let action = std::env::var("RTXFG_LIVE_ACTION")?;
    let target = core::DeploymentTarget::for_game(&exe).validate(true)?;
    core::assert_target_stopped(&target)?;
    if action == "install" {
        ensure!(
            core::record(&target.directory)?.is_none(),
            "preserve existing installation"
        );
        let prepared = cloud::prepare_with_progress(
            &cloud::bundled(),
            encore::SCHEME,
            1,
            &["version.dll".into()],
            &AtomicBool::new(false),
            |_| {},
        )?;
        let context = presets::Context::new(&prepared.scheme_id, &prepared.policy, &exe);
        core::deploy_prepared_context_at_with_overrides(
            &target,
            &prepared.backend,
            &["version.dll".into()],
            None,
            prepared.files,
            Some(&prepared.version),
            Some(&context),
            None,
            prepared.upstream_version.as_deref(),
        )?;
    } else {
        ensure!(
            core::record(&target.directory)?.is_some(),
            "install Encore with the manager first"
        );
    }
    match action.as_str() {
        "complete" | "install" => runtime_files::complete_encore_runtimes(
            std::slice::from_ref(&target),
            false,
            &AtomicBool::new(false),
            &mut |_| {},
        )?,
        "restore" => {
            for kind in runtime_files::KINDS {
                runtime_files::remove_many(std::slice::from_ref(&target), kind)?;
            }
            neural_runtime::remove_many(std::slice::from_ref(&target))?;
        }
        "uninstall" => {
            ensure!(
                cleanup::clean_outcome_at(&target)?.complete,
                "cleanup incomplete"
            );
        }
        _ => anyhow::bail!("unknown action"),
    }
    println!("{action} completed for {}", exe.display());
    for s in runtime_files::inspect_all(&target) {
        println!(
            "{} managed={} present={} versions={:?}",
            s.kind, s.managed, s.present, s.versions
        );
    }
    Ok(())
}
