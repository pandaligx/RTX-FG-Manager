//! Exercise the ordinary manual-install and card-install paths with signed payloads.
#![cfg(windows)]
use anyhow::{Context, Result, ensure};
use rtx_fg_manager::{
    cleanup, cloud, core, encore, encore_components, presets, recommendation_presets as cards,
    recommendation_runtimes, recommendations, runtime_files, scanner,
};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs, path::Path, sync::atomic::AtomicBool};

fn inventory(root: &Path, dir: &Path, files: &mut BTreeMap<String, String>) -> Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            inventory(root, &path, files)?;
        } else {
            let mut hash = Sha256::new();
            std::io::copy(&mut fs::File::open(&path)?, &mut hash)?;
            files.insert(
                path.strip_prefix(root)?.to_string_lossy().into(),
                format!("{:x}", hash.finalize()),
            );
        }
    }
    Ok(())
}

#[test]
#[ignore = "requires clean installed game, an isolated profile and signed Encore 2.5.0 runtime ZIPs"]
fn manual_author_to_clean_recipient_and_restore() -> Result<()> {
    let root = std::path::PathBuf::from(std::env::var("RTXFG_TEST_GAME_COPY")?);
    let qa_root = std::path::PathBuf::from(std::env::var("LOCALAPPDATA")?).canonicalize()?;
    ensure!(
        qa_root.file_name().is_some_and(|n| n == "profile")
            && root.canonicalize()?.parent() == qa_root.parent(),
        "use a disposable sibling of the isolated profile"
    );
    let target = core::DeploymentTarget::for_game(&root.join("TheSpellBrigade.exe"));
    let evidence = scanner::inspect_for_installation(&target.game_exe);
    ensure!(!evidence.limited && !evidence.neural_input() && !evidence.native_frame_generation());
    let mut before = BTreeMap::new();
    inventory(&root, &root, &mut before)?;
    let catalog = cloud::bundled();
    let cancel = AtomicBool::new(false);
    let entries = vec!["version.dll".to_owned()];
    let payload =
        cloud::prepare_with_progress(&catalog, encore::SCHEME, 1, &entries, &cancel, |_| {})?;
    let version = payload
        .upstream_version
        .as_deref()
        .context("missing Encore version")?;
    let ctx = presets::Context::new(encore::SCHEME, &payload.policy, &target.game_exe);
    let selection = encore_components::Components::default();
    encore_components::deploy(
        &target,
        &payload,
        &entries,
        payload.files.clone(),
        &ctx,
        &presets::Values::new(),
        selection,
    )?;
    runtime_files::complete_encore_selected(
        std::slice::from_ref(&target),
        selection,
        false,
        &cancel,
        &mut |_| {},
    )?;
    core::apply_parameters_at(
        &target,
        &ctx,
        &presets::Values::from([
            ("dlssWithoutGameDlss".into(), "1".into()),
            ("frameGenerationWithoutGameFg".into(), "1".into()),
            ("nrWithoutGameDlss".into(), "1".into()),
        ]),
    )?;
    let runtimes = recommendation_runtimes::capture(&target)?;
    let parameters = cards::capture(&target, &catalog, encore::SCHEME, version)?;
    println!("author {}", serde_json::to_string(&runtimes)?);
    let pairs: Vec<_> = runtimes
        .components
        .iter()
        .map(|c| (c.kind.as_str(), c.source.as_str()))
        .collect();
    for pair in [
        ("sr", "game"),
        ("rr", "game"),
        ("nr", "game"),
        ("encorefg", "managed"),
        ("xess", "patch"),
        ("xell", "patch"),
        ("xess_fg", "patch"),
    ] {
        ensure!(pairs.contains(&pair), "missing {pair:?}");
    }
    let game = scanner::Game {
        exe: target.game_exe.to_string_lossy().into(),
        ..Default::default()
    };
    let capture_card = || -> Result<recommendations::Recipe> {
        let identity =
            recommendations::GameIdentity::new("steam:2904000".into(), "The Spell Brigade".into())?;
        let setup = recommendations::installed_recipe(
            &game,
            identity,
            &catalog,
            String::new(),
            String::new(),
        )?;
        Ok(serde_json::from_slice(&serde_json::to_vec(&setup)?)?)
    };
    let recipe = capture_card()?;
    ensure!(
        recipe.runtimes.as_ref() == Some(&runtimes) && recipe.presets.as_ref() == Some(&parameters)
    );
    let restore = || -> Result<()> {
        cleanup::clean_at(&target)?;
        let mut after = BTreeMap::new();
        inventory(&root, &root, &mut after)?;
        ensure!(
            after == before,
            "file differences: {:?}",
            after
                .keys()
                .chain(before.keys())
                .filter(|k| after.get(*k) != before.get(*k))
                .collect::<Vec<_>>()
        );
        Ok(())
    };
    restore()?;
    recipe.check_runtime_selection()?;
    cards::execute(
        &game,
        &catalog,
        &recipe,
        cards::Action::InstallAndApply,
        1,
        &cancel,
        |_| {},
    )?;
    runtimes.verify_installed(std::slice::from_ref(&target))?;
    ensure!(parameters == cards::capture(&target, &catalog, encore::SCHEME, version)?);
    restore()?;
    cards::execute(
        &game,
        &catalog,
        &recipe,
        cards::Action::InstallOnly,
        1,
        &cancel,
        |_| {},
    )?;
    ensure!(
        recommendation_runtimes::capture(&target)?
            .components
            .is_empty(),
        "patch-only installed an optional runtime"
    );
    restore()?;
    let collision = root.join("sl.common.dll");
    fs::write(&collision, b"unrelated file")?;
    let result = cards::execute(
        &game,
        &catalog,
        &recipe,
        cards::Action::InstallAndApply,
        1,
        &cancel,
        |_| {},
    );
    ensure!(
        result.is_err()
            && fs::read(&collision)? == b"unrelated file"
            && core::record(&root)?.is_none()
    );
    fs::remove_file(collision)?;
    // A second author shared only SR. A recipient must not silently get NR,
    // Intel, RR or a private Streamline set with their new patch.
    let minimal = encore_components::Components {
        patch: true,
        sr: true,
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
        &entries,
        payload.files.clone(),
        &ctx,
        &presets::Values::new(),
        minimal,
    )?;
    runtime_files::complete_encore_selected(
        std::slice::from_ref(&target),
        minimal,
        false,
        &cancel,
        &mut |_| {},
    )?;
    let partial = recommendation_runtimes::capture(&target)?;
    ensure!(partial.components.len() == 1 && partial.components[0].kind == "sr");
    let partial_recipe = capture_card()?;
    restore()?;
    cards::execute(
        &game,
        &catalog,
        &partial_recipe,
        cards::Action::InstallAndApply,
        1,
        &cancel,
        |_| {},
    )?;
    ensure!(
        partial == recommendation_runtimes::capture(&target)?,
        "recipient gained unshared runtimes"
    );
    restore()?;
    println!(
        "round trip and conflict protection verified, {} original files",
        before.len()
    );
    Ok(())
}

#[test]
#[ignore = "requires verified signed local ZIP and isolated profile"]
fn optiscaler_signed_install_edit_uninstall() -> Result<()> {
    use rtx_fg_manager::{neural_runtime, optiscaler_nr as opti};
    let qa = std::path::PathBuf::from(std::env::var("LOCALAPPDATA")?);
    ensure!(qa.file_name().is_some_and(|s| s == "profile"));
    let scratch = tempfile::tempdir_in(qa.parent().context("profile parent")?)?;
    let exe = scratch.path().join("Game.exe");
    fs::copy(std::env::current_exe()?, &exe)?;
    let target = core::DeploymentTarget::for_game(&exe);
    let catalog = cloud::bundled();
    ensure!(catalog.proxies(opti::SCHEME) == opti::ENTRIES);
    let stop = AtomicBool::new(false);
    let originals = tempfile::tempdir_in(qa.parent().context("profile parent")?)?;
    let native = scratch.path().join("nvngx_dlssnr.dll");
    let hash = "2bc19bc09a7b89dd44f73e2ac0108fcc586e2154bdd0830f242f4f040e28789f";
    for (index, entry) in opti::ENTRIES.into_iter().enumerate() {
        use rtx_fg_manager::runtime_files::opti_streamline;
        let with_nr = index % 4 < 2;
        let entries = vec![entry.to_owned()];
        let p = cloud::prepare_with_progress(&catalog, opti::SCHEME, 1, &entries, &stop, |_| {})?;
        let queue = cloud::download_files(&catalog, opti::SCHEME, 1, &entries)?;
        ensure!(
            p.files.len() == 39
                && queue.len() == 2
                && queue[1] == opti_streamline::download_file()?
        );
        ensure!(
            presets::read_values(&p.files[opti::CONFIG], opti::PROFILE)?
                == presets::defaults(opti::PROFILE, &presets::Values::new())
        );
        let context = presets::Context::new(opti::SCHEME, &p.policy, &exe);
        let install = || {
            core::deploy_prepared_context_at_with_overrides(
                &target,
                opti::BACKEND,
                &entries,
                None,
                p.files.clone(),
                Some("0.9.97"),
                Some(&context),
                None,
                Some("0.9.97"),
            )
        };
        if entry == "dxgi.dll" {
            fs::write(scratch.path().join("OptiScaler.ini"), b"foreign config")?;
            ensure!(install().is_err() && !scratch.path().join(entry).exists());
            fs::remove_file(scratch.path().join("OptiScaler.ini"))?;
        }
        if entry == "dxgi.dll" {
            fs::write(&native, b"foreign NR")?;
            ensure!(neural_runtime::OptiScalerCompanion::needs_file(&target).is_err());
            ensure!(fs::read(&native)? == b"foreign NR");
            fs::remove_file(&native)?;
        } else if entry == "winmm.dll" {
            fs::copy(originals.path().join("NVIDIA.dll"), &native)?;
        }
        let mut last_source = String::new();
        let companion = with_nr
            .then(|| {
                neural_runtime::OptiScalerCompanion::prepare(
                    std::slice::from_ref(&target),
                    false,
                    &stop,
                    &mut |p| {
                        if !p.source.is_empty() {
                            last_source = p.source;
                        }
                    },
                )
            })
            .transpose()?;
        install()?;
        if let Some(nr) = companion {
            nr.install(&target, &stop)?;
        }
        ensure!(opti_streamline::installed(&target)?);
        ensure!(scratch.path().join("OptiScaler/streamline").is_dir());
        ensure!(!scratch.path().join("sl.interposer.dll").exists());
        ensure!(native.exists() == with_nr);
        if with_nr {
            ensure!(core::digest(&native)? == hash);
        }
        ensure!(neural_runtime::inspect(&target).managed == (with_nr && entry != "winmm.dll"));
        if entry == "dxgi.dll" {
            fs::copy(&native, originals.path().join("NVIDIA.dll"))?;
            println!("NR source: {last_source}");
        }
        if entry == "winmm.dll" {
            ensure!(last_source.is_empty());
        }
        ensure!(core::digest(&scratch.path().join(entry))? == core::hash(&p.files[entry]));
        core::apply_parameters_at(
            &target,
            &context,
            &presets::Values::from([("nr_passes".into(), "2".into())]),
        )?;
        let (_, values) = presets::inspect_at(&target, &catalog)?.context("preset vanished")?;
        ensure!(values["nr_passes"] == "2");
        // SimpleIni writes mixed-case engine names and six decimal places on Save Settings.
        let config_path = target.directory.join(opti::CONFIG);
        let mut saved = fs::read(&config_path)?;
        for (section, key, value) in [
            ("FrameGen", "Enabled", "true"),
            ("FrameGen", "FGInput", "Upscaler"),
            ("FrameGen", "FGOutput", "DLSSG"),
            ("DlssNr", "WorkingScale", "0.500000"),
        ] {
            saved = rtx_fg_manager::diagnostics::edit_ini(&saved, section, key, value)?;
        }
        fs::write(&config_path, &saved)?;
        install()?;
        ensure!(
            fs::read(&config_path)? == saved,
            "reapply overwrote menu settings"
        );
        let (_, readback) =
            presets::inspect_at(&target, &catalog)?.context("menu preset missing")?;
        ensure!(
            readback["fg_input"] == "upscaler"
                && readback["fg_output"] == "dlssg"
                && readback["fg_enabled"] == "true"
                && readback["nr_scale"] == "0.5"
        );
        if entry == "dxgi.dll" {
            ensure!(neural_runtime::inspect(&target).managed);
            let game = scanner::Game {
                exe: exe.to_string_lossy().into(),
                ..Default::default()
            };
            let identity =
                recommendations::GameIdentity::new("steam:2904000".into(), "Game".into())?;
            let card = recommendations::installed_recipe(
                &game,
                identity,
                &catalog,
                String::new(),
                String::new(),
            )?;
            card.check_runtime_selection()?;
            cleanup::clean_at(&target)?;
            cards::execute(
                &game,
                &catalog,
                &card,
                cards::Action::InstallOnly,
                1,
                &stop,
                |_| {},
            )?;
            ensure!(opti_streamline::installed(&target)? && !native.exists());
            cleanup::clean_at(&target)?;
            let private_sl = opti_streamline::download_file()?;
            let mut queued = Vec::new();
            cards::execute(
                &game,
                &catalog,
                &card,
                cards::Action::InstallAndApply,
                1,
                &stop,
                |p| queued.extend(p.transfer.queue),
            )?;
            ensure!(
                queued.iter().any(|f| f.id == private_sl.id),
                "the shared card did not announce its private Streamline files"
            );
            ensure!(neural_runtime::inspect(&target).managed);
            ensure!(opti_streamline::installed(&target)?);
            ensure!(
                presets::inspect_at(&target, &catalog)?
                    .context("missing shared settings")?
                    .1["nr_passes"]
                    == "2"
            );
        }
        fs::write(
            scratch.path().join("OptiScaler.log"),
            b"DLSS Unlocked - OptiScaler v0.9.97 loaded",
        )?;
        let internal = scratch.path().join("OptiScaler/dlssg_sm86/dlssg_sm86.ini");
        let contents =
            fs::read_to_string(&internal)?.replace("MaxGeneratedFrames=3", "MaxGeneratedFrames=2");
        fs::write(&internal, contents)?;
        ensure!(core::status_at(&target).starts_with("已部署"));
        let logs = scratch.path().join("dlssg_sm86/logs");
        fs::create_dir_all(&logs)?;
        fs::write(logs.join("frames_123.jsonl"), b"test")?;
        fs::write(
            scratch.path().join("OptiScaler_123456.log"),
            b"DLSS Unlocked - OptiScaler v0.9.97 loaded",
        )?;
        fs::write(scratch.path().join("keep.txt"), b"user")?;
        cleanup::clean_at(&target)?;
        if entry == "winmm.dll" {
            ensure!(
                core::digest(&native)? == hash,
                "game-provided NR was modified"
            );
            fs::remove_file(&native)?;
        } else {
            ensure!(!native.exists());
        }
        let mut remaining: Vec<_> = fs::read_dir(scratch.path())?
            .map(|p| p.map(|e| e.file_name()))
            .collect::<std::io::Result<_>>()?;
        remaining.sort();
        ensure!(
            remaining
                == [
                    std::ffi::OsString::from("Game.exe"),
                    std::ffi::OsString::from("keep.txt")
                ],
            "{entry}: {remaining:?}"
        );
        println!("installed and restored {entry}");
    }
    Ok(())
}
