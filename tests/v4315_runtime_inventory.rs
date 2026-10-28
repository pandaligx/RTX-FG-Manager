//! Optional local fixture checks; no DLL loading, game launch or public sharing.
use anyhow::Result;
use rtx_fg_manager::{recommendation_runtimes as runtimes, win};
use std::{collections::BTreeMap, fs, io::Write, path::PathBuf};

#[test]
#[ignore = "requires private Wuthering Waves paths, recipes and a prefilled isolated cache"]
fn copied_game_tree_installs_and_recaptures_both_recommendations() -> Result<()> {
    use rtx_fg_manager::{
        cloud, core, recommendation_presets as shared, recommendations, scanner::Game,
    };
    use std::sync::atomic::AtomicBool;
    assert!(std::env::var("LOCALAPPDATA")?.contains("v4315-settings-window"));
    let root = PathBuf::from(std::env::var("RTXFG_WUTHERING_ROOT")?);
    let recipes: Vec<recommendations::Recipe> =
        serde_json::from_slice(&fs::read(std::env::var("RTXFG_WUTHERING_RECIPES")?)?)?;
    assert_eq!(recipes.len(), 2);
    let originals = [
        "Client/Binaries/Win64/Client-Win64-Shipping.exe",
        "Engine/Plugins/Runtime/Nvidia/DLSS/Binaries/ThirdParty/Win64/nvngx_dlss.dll",
        "Engine/Plugins/Runtime/Nvidia/DLSS/Binaries/ThirdParty/Win64/nvngx_dlssd.dll",
        "Engine/Plugins/Runtime/Nvidia/StreamlineCore/Binaries/ThirdParty/Win64/nvngx_dlssg.dll",
    ];
    let before = originals
        .iter()
        .map(|p| core::digest(&root.join(p)))
        .collect::<Result<Vec<_>>>()?;
    let catalog = cloud::bundled();
    for recipe in recipes {
        recipe.validate(&catalog)?;
        let fixture = tempfile::tempdir()?;
        let copied_root = fixture.path().join("Wuthering Waves Game");
        for name in originals {
            let destination = copied_root.join(name);
            fs::create_dir_all(destination.parent().unwrap())?;
            fs::copy(root.join(name), destination)?;
        }
        let exe = copied_root.join(originals[0]);
        let game = Game {
            title: "Wuthering Waves".into(),
            exe: exe.to_string_lossy().into(),
            ..Game::default()
        };
        let target = core::DeploymentTarget::for_game(&exe);
        // The split menu's three independent operations must compose to the
        // same result as installing the complete shared setup.
        shared::execute(
            &game,
            &catalog,
            &recipe,
            shared::Action::Selected {
                patch: true,
                parameters: false,
                runtimes: false,
            },
            1,
            &AtomicBool::new(false),
            |_| {},
        )?;
        let mut runtime_recipe = recipe.clone();
        runtime_recipe.presets = None;
        shared::execute(
            &game,
            &catalog,
            &runtime_recipe,
            shared::Action::Selected {
                patch: false,
                parameters: false,
                runtimes: true,
            },
            1,
            &AtomicBool::new(false),
            |_| {},
        )?;
        shared::execute(
            &game,
            &catalog,
            &recipe,
            shared::Action::Selected {
                patch: false,
                parameters: true,
                runtimes: false,
            },
            1,
            &AtomicBool::new(false),
            |_| {},
        )?;
        shared::execute(
            &game,
            &catalog,
            &recipe,
            shared::Action::Selected {
                patch: true,
                parameters: true,
                runtimes: true,
            },
            1,
            &AtomicBool::new(false),
            |_| {},
        )?;
        let captured = recommendations::installed_recipe(
            &game,
            recipe.game.clone(),
            &catalog,
            "RTX 4060".into(),
            "621.01".into(),
        )?;
        captured.validate(&catalog)?;
        let actual = captured.presets.as_ref().unwrap();
        let expected = recipe.presets.as_ref().unwrap().for_version(
            &recipe.scheme,
            &recipe.patch_version,
            &captured.patch_version,
        )?;
        for (name, value) in expected.values {
            assert_eq!(actual.values.get(&name), Some(&value), "{name}");
        }
        let serialized = serde_json::to_vec(&captured)?;
        assert!(
            !String::from_utf8_lossy(&serialized)
                .contains(&copied_root.to_string_lossy().to_string())
        );
        serde_json::from_slice::<recommendations::Recipe>(&serialized)?.validate(&catalog)?;
        if recipe
            .runtimes
            .as_ref()
            .unwrap()
            .components
            .iter()
            .any(|c| c.preserves_game_files())
        {
            for name in &originals[1..] {
                assert_eq!(
                    core::digest(&copied_root.join(name))?,
                    core::digest(&root.join(name))?
                );
            }
        }
        assert!(core::record(&target.directory)?.is_some());
        // Patch uninstall must restore all owned runtime originals itself.
        let cleanup = rtx_fg_manager::cleanup::clean_outcome_at(&target)?;
        assert!(cleanup.complete, "{cleanup:?}");
        for (name, hash) in originals.iter().zip(&before) {
            assert_eq!(core::digest(&copied_root.join(name))?, *hash);
        }
        eprintln!(
            "Install/apply/capture/serialize/cleanup passed: {}",
            recipe.title
        );
    }
    for (name, hash) in originals.iter().zip(&before) {
        assert_eq!(
            core::digest(&root.join(name))?,
            *hash,
            "actual game was modified"
        );
    }
    Ok(())
}

#[test]
#[ignore = "requires RTXFG_INTEL_FIXTURE with the three locally cached Intel DLLs"]
fn prepared_intel_versions_match_the_published_inventory() -> Result<()> {
    let root = PathBuf::from(std::env::var("RTXFG_INTEL_FIXTURE")?);
    let mut components = Vec::new();
    let mut files = BTreeMap::new();
    for (name, kind, expected) in [
        ("libxell.dll", "xell", "1.3.2"),
        ("libxess.dll", "xess", "2.0.2"),
        ("libxess_fg.dll", "xess_fg", "1.3.1"),
    ] {
        let bytes = fs::read(root.join(name))?;
        assert_eq!(win::file_version(&root.join(name))?.0, expected);
        let mut temporary = tempfile::Builder::new().suffix(".dll").tempfile()?;
        temporary.write_all(&bytes)?;
        eprintln!(
            "{name}: open temporary = {:?}",
            win::file_version(temporary.path())
        );
        let closed = temporary.into_temp_path();
        eprintln!(
            "{name}: closed temporary = {:?}",
            win::file_version(&closed)
        );
        components.push(runtimes::Component {
            kind: kind.into(),
            versions: vec![expected.into()],
            source: "patch".into(),
            package_version: None,
        });
        files.insert(format!("rtx-encore-runtimes/intel/{name}"), bytes);
    }
    runtimes::Snapshot {
        schema: 1,
        components,
    }
    .verify_prepared(&files)
}
