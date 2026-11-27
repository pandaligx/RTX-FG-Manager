//! Regression coverage for the game-settings lifetime across uninstall/reinstall.
use super::*;
use rtx_fg_manager::{presets, recommendation_presets as shared, recommendations};

fn controller() -> (tempfile::TempDir, Controller) {
    let dir = tempfile::tempdir().unwrap();
    let c = Controller::new(
        dir.path().into(),
        json!({"language":"zh-CN", "games":[], "auto_update":false}),
        None,
        Some(dir.path().join("test.json")),
    );
    (dir, c)
}

fn changed_parameter(profile: &str, defaults: &presets::Values) -> (String, String) {
    presets::parameters(profile)
        .into_iter()
        .filter(|p| shared::shareable(profile, p.key))
        .find_map(|p| {
            p.choices.iter().find_map(|(value, _)| {
                defaults
                    .get(p.key)
                    .filter(|old| old != value)
                    .map(|_| (p.key.to_owned(), (*value).to_owned()))
            })
        })
        .expect("each shareable protocol has an editable value")
}

fn snapshot(status: &str, installed: usize) -> Box<rtx_fg_manager::deployment::Snapshot> {
    let mut s = rtx_fg_manager::deployment::Snapshot::default();
    s.status = status.into();
    s.total = 2;
    s.installed = installed;
    Box::new(s)
}

#[test]
fn incomplete_or_stale_uninstall_never_resets_any_scheme() {
    let (_dir, mut c) = controller();
    c.games = vec![Game {
        exe: "A.exe".into(),
        ..Default::default()
    }];
    let schemes = c
        .catalog
        .schemes()
        .into_iter()
        .map(|p| p.scheme_id.clone())
        .collect::<BTreeSet<_>>();
    for scheme in schemes {
        let policy = &c.catalog.scheme_policies[&scheme];
        let mut values = presets::defaults(&policy.parameter_profile, &policy.defaults);
        let (key, value) = changed_parameter(&policy.parameter_profile, &values);
        values.insert(key, value);
        c.channel
            .send(Event::PresetApplied("A.exe".into(), scheme, values));
    }
    c.events();
    let original = c.games[0].extra.clone();
    c.status_epoch += 1;
    c.channel.send(Event::Deployments(
        c.status_epoch - 1,
        "A.exe".into(),
        snapshot("未部署", 0),
    ));
    for (status, installed) in [
        ("需检查", 0),
        ("补丁已移除，缓存待清理", 0),
        ("部分已部署", 1),
        ("已部署", 2),
    ] {
        c.channel.send(Event::Deployments(
            c.status_epoch,
            "A.exe".into(),
            snapshot(status, installed),
        ));
    }
    c.events();
    assert_eq!(c.games[0].extra, original);
    c.close();
    c.tick_close();
}

#[test]
fn every_scheme_preserves_new_manual_edits_but_discard_and_restart_return_to_defaults() {
    let (_dir, mut c) = controller();
    let schemes = c
        .catalog
        .schemes()
        .into_iter()
        .map(|p| p.scheme_id.clone())
        .collect::<BTreeSet<_>>();
    for scheme in schemes {
        c.games = vec![Game {
            exe: "A.exe".into(),
            ..Default::default()
        }];
        c.focus = Some("A.exe".into());
        c.set_selection(scheme.clone(), 1, vec!["version.dll".into()]);
        c.channel.send(Event::Deployments(
            c.status_epoch,
            "A.exe".into(),
            snapshot("未部署", 0),
        ));
        c.events();
        let defaults = c.preset_values("A.exe");
        let (key, value) = changed_parameter(&c.parameter_profile(), &defaults);
        c.set_game_option("A.exe", &key, &value);
        assert_eq!(c.explicit_preset_values("A.exe", &scheme)[&key], value);
        c.channel.send(Event::Deployments(
            c.status_epoch,
            "A.exe".into(),
            snapshot("未部署", 0),
        ));
        c.events();
        assert_eq!(
            c.preset_values("A.exe")[&key],
            value,
            "new edit lost: {scheme}"
        );
        assert!(c.preset_dirty("A.exe"));
        c.discard_presets(Some(("A.exe", &scheme)));
        assert_eq!(
            c.preset_values("A.exe"),
            defaults,
            "discard restored old installation: {scheme}"
        );
    }
    let persisted = c.state.clone();
    c.close();
    c.tick_close();
    let restart = tempfile::tempdir().unwrap();
    let mut c = Controller::new(
        restart.path().into(),
        persisted,
        None,
        Some(restart.path().join("test.json")),
    );
    c.focus = Some("A.exe".into());
    assert!(c.first_pending_preset().is_none());
    c.close();
    c.tick_close();
}

#[test]
fn completed_uninstall_discards_old_drafts_and_rejects_stale_completion() {
    let (_dir, mut c) = controller();
    c.games = vec![Game {
        exe: "A.exe".into(),
        ..Default::default()
    }];
    c.focus = Some("A.exe".into());
    let schemes = c
        .catalog
        .schemes()
        .into_iter()
        .map(|p| p.scheme_id.clone())
        .collect::<Vec<_>>();
    for scheme in schemes {
        c.set_selection(scheme.clone(), 1, vec!["version.dll".into()]);
        let defaults = c.preset_values("A.exe");
        let (key, value) = changed_parameter(&c.parameter_profile(), &defaults);
        c.set_game_option("A.exe", &key, &value);
        c.status_epoch += 1;
        c.channel
            .send(Event::Uninstalled(c.status_epoch - 1, "A.exe".into()));
        c.events();
        assert_eq!(c.preset_values("A.exe")[&key], value);
        c.channel
            .send(Event::Uninstalled(c.status_epoch, "A.exe".into()));
        c.events();
        assert_eq!(
            c.preset_values("A.exe"),
            defaults,
            "old draft survived: {scheme}"
        );
        assert!(!c.preset_dirty("A.exe"));
        c.discard_presets(None);
        assert_eq!(c.preset_values("A.exe"), defaults);
    }
    c.close();
    c.tick_close();
}

fn finish(c: &mut Controller) -> Result<()> {
    let until = Instant::now() + Duration::from_secs(240);
    loop {
        c.events();
        if !c.busy && !c.refresh_busy && !c.refresh_pending && c.preset_reads.is_empty() {
            break;
        }
        anyhow::ensure!(
            Instant::now() < until,
            "operation timed out: {}",
            c.progress
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    anyhow::ensure!(c.warning.is_none(), "{:?}", c.warning);
    Ok(())
}

#[test]
fn native_runtime_choices_require_standard_or_delta_dlssg_042() {
    let (_data, mut c) = controller();
    assert!(c.has_patch_options(upstream040::SCHEME));
    assert!(c.has_patch_options(cloud::DELTA_041_SCHEME));
    for other in [
        upstream040::PIPO_SCHEME,
        cloud::DELTA_SCHEME,
        rtx_fg_manager::anr::SCHEME,
    ] {
        assert!(!c.has_patch_options(other));
    }
    for p in &mut c.catalog.packages {
        if dlssg_native_options(&p.scheme_id) {
            p.upstream_version = Some("0.4.1".into());
        }
    }
    assert!(!c.has_patch_options(upstream040::SCHEME));
    assert!(!c.has_patch_options(cloud::DELTA_041_SCHEME));
    let path = "without-native-plugins.exe".to_owned();
    let paths = BTreeSet::from([path.clone()]);
    c.patch_install_plan = Some(PatchInstallPlan {
        fingerprint: c.patch_options_fingerprint(&paths),
        paths,
        choices: vec![],
        dlssg_options: vec![DlssgInstallOption {
            exe: path.clone(),
            game: "Game".into(),
            patch_label: "Dlssg_for_sm86".into(),
            miles_profile: false,
            yanyun: None,
            unavailable: BTreeMap::from([(
                runtime_files::NativeComponent::Streamline,
                "未检测到游戏自带的对应组件".into(),
            )]),
        }],
    });
    assert!(
        c.confirm_patch_options(
            vec![],
            NativeSelections {
                runtimes: BTreeMap::from([(
                    path,
                    BTreeSet::from([runtime_files::NativeComponent::Streamline]),
                )]),
                ..Default::default()
            }
        )
        .is_err()
    );
    assert!(c.selected_native_components.runtimes.is_empty());
    assert!(c.selected_native_components.yanyun.is_empty());
    assert!(!c.busy);
    c.close();
    c.tick_close();
}

#[test]
#[ignore = "requires the standard 0.4.2 archive and native SDK in an isolated cache"]
fn standard_patch_streamline_opt_in_round_trip() -> Result<()> {
    native_runtime_round_trip(upstream040::SCHEME, false)
}

#[test]
#[ignore = "requires the Delta 0.4.2 archive and native SDK in an isolated cache"]
fn delta_patch_streamline_opt_in_round_trip() -> Result<()> {
    native_runtime_round_trip(cloud::DELTA_041_SCHEME, false)
}

#[test]
#[ignore = "requires the signed Delta patch and official Streamline SDK in an isolated cache"]
fn miles_patch_streamline_opt_in_round_trip() -> Result<()> {
    native_runtime_round_trip(cloud::DELTA_041_SCHEME, true)
}

fn native_runtime_round_trip(scheme: &str, miles: bool) -> Result<()> {
    use runtime_files::NativeComponent;
    let sdk =
        std::fs::read(std::env::var_os("RTXFG_TEST_SL_COMMON").context("missing fixture SDK")?)?;
    let mut original = sdk.clone();
    original.extend_from_slice(b"original game plugin");
    let (_data, mut c) = controller();
    let game_dir = tempfile::tempdir()?;
    let base = if miles {
        let apps = game_dir.path().join("steamapps");
        std::fs::create_dir_all(apps.join("common"))?;
        std::fs::write(
            apps.join("appmanifest_1817190.acf"),
            "\"appid\" \"1817190\"\n\"installdir\" \"Native\"",
        )?;
        apps.join("common")
    } else {
        game_dir.path().to_path_buf()
    };
    let mut targets = Vec::new();
    for renderer in ["Native", "WithoutStreamline"] {
        let folder = base.join(renderer);
        std::fs::create_dir(&folder)?;
        let exe = folder.join(if miles && renderer == "Native" {
            "MilesMorales.exe"
        } else {
            "TestGame.exe"
        });
        std::fs::copy(std::env::current_exe()?, &exe)?;
        targets.push(exe.display().to_string());
    }
    let native = base.join("Native");
    let plugin = native.join("sl.common.dll");
    let interposer = native.join("sl.interposer.dll");
    let native_dlls = [
        (NativeComponent::SuperResolution, "nvngx_dlss.dll"),
        (NativeComponent::RayReconstruction, "nvngx_dlssd.dll"),
        (NativeComponent::FrameGeneration, "nvngx_dlssg.dll"),
    ];
    for (_, name) in native_dlls {
        std::fs::write(native.join(name), &original)?;
    }
    std::fs::write(&plugin, &original)?;
    let original_entry: &[u8] = if miles {
        &original
    } else {
        b"original game interposer"
    };
    std::fs::write(&interposer, original_entry)?;
    if miles {
        std::fs::write(native.join("sl.dlss_g.dll"), &original)?;
    }
    let game = Game {
        exe: targets[0].clone(),
        targets: targets.clone(),
        ..Default::default()
    };
    c.focus = Some(game.exe.clone());
    c.games = vec![game.clone()];
    c.set_selection(scheme.into(), 1, vec!["version.dll".into()]);
    for kinds in [
        BTreeSet::new(),
        BTreeSet::from([NativeComponent::FrameGeneration]),
        BTreeSet::from(NativeComponent::ALL),
    ] {
        let opt_in = kinds.contains(&NativeComponent::Streamline);
        c.request_patch(false);
        finish(&mut c)?;
        let choices = &c
            .patch_install_plan
            .as_ref()
            .context("no installation dialog")?
            .dlssg_options;
        assert!(
            choices
                .iter()
                .any(|o| o.exe == targets[0] && o.unavailable.is_empty())
        );
        assert!(
            choices
                .iter()
                .any(|o| o.exe == targets[1] && o.unavailable.len() == NativeComponent::ALL.len())
        );
        assert!(c.selected_native_components.runtimes.is_empty());
        assert!(c.selected_native_components.yanyun.is_empty());
        // Launcher metadata may arrive between the probe and the click.
        c.games[0]
            .extra
            .insert("name_identity".into(), json!({"title": "Localized game"}));
        let mut checked = NativeSelections::default();
        if !kinds.is_empty() {
            checked.runtimes.insert(targets[0].clone(), kinds.clone());
        }
        c.confirm = None; // The native dialog consumes the pending prompt on opening.
        c.confirm_patch_options(vec![], checked)?;
        finish(&mut c)?;
        let deploy = game.deployment_targets();
        for t in &deploy {
            assert!(t.directory.join("version.dll").is_file());
        }
        assert_eq!(
            core::digest(&plugin)?,
            core::hash(if opt_in { &sdk } else { &original })
        );
        assert_eq!(
            runtime_files::installed_package_version(
                &game.deployment_target(std::path::Path::new(&targets[0])),
                "streamline"
            )?
            .as_deref(),
            opt_in.then_some(runtime_files::NATIVE_STREAMLINE_VERSION)
        );
        assert!(!base.join("WithoutStreamline/sl.common.dll").exists());
        for (kind, name) in native_dlls {
            let updated = kinds.contains(&kind);
            assert_eq!(
                core::digest(&native.join(name))? != core::hash(&original),
                updated
            );
            if updated {
                assert!(
                    win::file_version(&native.join(name))?
                        .0
                        .starts_with(kind.version())
                );
            }
            assert!(!base.join("WithoutStreamline").join(name).exists());
        }
        if miles && opt_in {
            assert_eq!(
                core::digest(&interposer)?,
                "b1d7395aebd7b9722841d64dcf7433680020b6a8e421f851c16d5ba0c2182edf"
            );
        } else {
            assert_eq!(std::fs::read(&interposer)?, original_entry);
        }
        c.request_patch(true);
        finish(&mut c)?;
        assert_eq!(core::digest(&plugin)?, core::hash(&original));
        assert_eq!(std::fs::read(&interposer)?, original_entry);
        for (_, name) in native_dlls {
            assert_eq!(core::digest(&native.join(name))?, core::hash(&original));
        }
        for t in &deploy {
            assert!(!t.directory.join("version.dll").exists());
            assert!(!t.directory.join(".rtx-fg-runtimes").exists());
        }
    }
    c.request_patch(false);
    finish(&mut c)?;
    assert!(c.selected_native_components.runtimes.is_empty());
    assert!(c.selected_native_components.yanyun.is_empty());
    assert!(c.patch_install_plan.is_some());
    c.cancel_patch();
    assert!(c.patch_install_plan.is_none() && !native.join("version.dll").exists());
    c.close();
    c.tick_close();
    Ok(())
}

#[test]
#[ignore = "explicit six-package download and signed-file deployment in isolated LOCALAPPDATA; no DLL execution"]
fn all_current_signed_schemes_share_uninstall_and_manual_reinstall() -> Result<()> {
    let root = PathBuf::from(std::env::var("RTXFG_PRESET_QA")?);
    anyhow::ensure!(
        std::env::var("LOCALAPPDATA")?.contains("v4327-presets"),
        "isolated cache required"
    );
    let catalog = cloud::bundled();
    let schemes = catalog
        .schemes()
        .into_iter()
        .map(|p| p.scheme_id.clone())
        .collect::<BTreeSet<_>>();
    assert_eq!(schemes.len(), 6);
    let mut report = Vec::new();
    for scheme in schemes {
        let (_data, mut c) = controller();
        eprintln!("{scheme}: download and initial installation");
        let game_dir = tempfile::tempdir_in(&root)?;
        let mut paths = Vec::new();
        for name in ["DX12", "Alternative"] {
            let dir = game_dir.path().join(name);
            std::fs::create_dir(&dir)?;
            let exe = dir.join("IsolatedPresetGame.exe");
            std::fs::copy(std::env::current_exe()?, &exe)?;
            std::fs::write(dir.join("unrelated-mod.txt"), b"preserve")?;
            paths.push(exe.display().to_string());
        }
        let identity = recommendations::GameIdentity::new(
            "steam:2012510".into(),
            "Isolated preset QA".into(),
        )?;
        let game = Game {
            exe: paths[0].clone(),
            targets: paths.clone(),
            extra: [("recommendation_game".into(), json!(identity))]
                .into_iter()
                .collect(),
            ..Default::default()
        };
        c.games = vec![game.clone()];
        c.focus = Some(game.exe.clone());
        c.set_selection(scheme.clone(), 1, vec!["version.dll".into()]);
        let targets = game.deployment_targets();
        let policy = &catalog.scheme_policies[&scheme];
        let defaults = presets::defaults(&policy.parameter_profile, &policy.defaults);
        let (key, value) = changed_parameter(&policy.parameter_profile, &defaults);
        let paths: BTreeSet<_> = paths.into_iter().collect();
        // Real download/verified cache preparation, installation and completion events.
        c.perform(false, paths.clone());
        finish(&mut c)?;
        let baseline = targets
            .iter()
            .map(|t| presets::inspect_at(t, &catalog).map(|v| v.unwrap().1))
            .collect::<Result<Vec<_>>>()?;
        assert_ne!(
            baseline[0][&key], value,
            "test must change a value: {scheme}"
        );
        for target in &targets {
            let context = presets::Context::new(&scheme, policy, &target.game_exe);
            core::apply_parameters_at(
                target,
                &context,
                &BTreeMap::from([(key.clone(), value.clone())]),
            )?;
        }
        if scheme == rtx_fg_manager::encore::SCHEME {
            rtx_fg_manager::neural_runtime::install_many(
                &targets,
                "310.9.16",
                false,
                &AtomicBool::new(false),
                &mut |_| {},
            )?;
            for target in &targets {
                core::apply_parameters_at(
                    target,
                    &presets::Context::new(&scheme, policy, &target.game_exe),
                    &BTreeMap::from([("nrEnabled".into(), "1".into())]),
                )?;
            }
        }
        let recipe = recommendations::installed_recipe(
            &game,
            identity,
            &catalog,
            "RTX 4060".into(),
            "581.80".into(),
        )?;
        std::fs::write(
            root.join(format!("{scheme}-recipe.json")),
            serde_json::to_vec_pretty(&recipe)?,
        )?;
        assert_eq!(recipe.presets.as_ref().unwrap().values[&key], value);
        c.perform(true, paths.clone());
        finish(&mut c)?;
        // Apply the same public-card pipeline as the UI, without publishing a fake card.
        eprintln!("{scheme}: shared patch, parameters and runtimes");
        c.use_recommendation(recipe.clone(), None, shared::Action::InstallAndApply)?;
        finish(&mut c)?;
        for target in &targets {
            assert_eq!(
                presets::inspect_at(target, &catalog)?.unwrap().1[&key],
                value,
                "shared values not installed: {scheme}"
            );
        }
        let shared_values = c.preset_values(&game.exe);
        // A direct reinstall/update preserves installed values.
        eprintln!("{scheme}: in-place installation preserves shared values");
        c.perform(false, paths.clone());
        finish(&mut c)?;
        assert_eq!(
            c.preset_values(&game.exe)[&key],
            value,
            "existing install must be preserved: {scheme}"
        );
        eprintln!("{scheme}: complete uninstall");
        c.perform(true, paths.clone());
        finish(&mut c)?;
        for target in &targets {
            assert!(core::record(&target.directory)?.is_none());
            assert!(presets::inspect_at(target, &catalog)?.is_none());
            assert!(!target.directory.join("nvngx_dlssnr.dll").exists());
            assert_eq!(
                std::fs::read(target.directory.join("unrelated-mod.txt"))?,
                b"preserve"
            );
        }
        assert_eq!(
            c.preset_values(&game.exe)[&key],
            defaults[&key],
            "UI retained shared value: {scheme}"
        );
        // Emulate a 4.3.26 cache or an immediate click before the status refresh.
        eprintln!("{scheme}: manual reinstall with stale saved snapshot");
        c.channel.send(Event::PresetApplied(
            game.exe.clone(),
            scheme.clone(),
            shared_values,
        ));
        c.events();
        // Legacy caches can retain a field list even though no edit is pending.
        c.games[0]
            .extra
            .entry("preset_dirty_keys")
            .or_insert_with(|| json!({}))[&scheme] = json!([&key]);
        assert!(c.explicit_preset_values(&game.exe, &scheme).is_empty());
        c.perform(false, paths.clone());
        finish(&mut c)?;
        for (target, expected) in targets.iter().zip(&baseline) {
            let actual = presets::inspect_at(target, &catalog)?.unwrap().1;
            assert_eq!(
                &actual, expected,
                "manual reinstall replayed shared values: {scheme}"
            );
        }
        assert_eq!(c.preset_values(&game.exe)[&key], baseline[0][&key]);
        c.perform(true, paths);
        finish(&mut c)?;
        c.save();
        let persisted = c.state.clone();
        c.close();
        c.tick_close();
        let restart = tempfile::tempdir()?;
        let mut c = Controller::new(
            restart.path().into(),
            persisted,
            None,
            Some(restart.path().join("test.json")),
        );
        c.focus = Some(game.exe.clone());
        assert_eq!(
            c.preset_values(&game.exe)[&key],
            defaults[&key],
            "restart resurrected old values: {scheme}"
        );
        c.close();
        c.tick_close();
        report.push(json!({"scheme":scheme,"directories":2,"shared_key":key,"shared_value":value,"manual_default":baseline[0][&key],"passed":true}));
        std::fs::write(
            root.join("signed-roundtrip.json"),
            serde_json::to_vec_pretty(&report)?,
        )?;
        eprintln!("{scheme}: passed");
    }
    Ok(())
}
