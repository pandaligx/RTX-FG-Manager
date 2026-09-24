use anyhow::Result;
use rtx_fg_manager::{cache, cloud};
#[cfg(feature = "fixture-tests")]
use rtx_fg_manager::{cleanup, core, presets};
#[cfg(feature = "fixture-tests")]
use std::collections::BTreeMap;
use std::{fs, path::Path};
#[test]
fn compact_catalog_expands_three_schemes_and_checks_index() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/catalog420");
    let bytes = fs::read(root.join("catalog.json"))?;
    let index = fs::read(root.join("index.json"))?;
    let c = serde_json::from_slice::<cloud::CompactCatalog>(&bytes)?.expand(&index)?;
    assert_eq!(c.schemes().len(), 3);
    assert_eq!(c.packages.len(), 13);
    assert_eq!(c.default_scheme, "upstream-0.3.5-310-9");
    assert_eq!(c.proxies("initial"), vec!["version.dll"]);
    for backend in ["rtx20", "rtx30"] {
        assert_eq!(
            c.packages
                .iter()
                .filter(|p| p.scheme_id == "initial" && p.backends.contains(&backend.into()))
                .count(),
            1
        );
    }
    let mut bad = index.clone();
    bad[0] ^= 1;
    assert!(
        serde_json::from_slice::<cloud::CompactCatalog>(&bytes)?
            .expand(&bad)
            .is_err()
    );
    let mut edit: serde_json::Value = serde_json::from_slice(&bytes)?;
    edit["schemes"][1]["defaults"]["optimized"] = "3".into();
    assert!(
        serde_json::from_value::<cloud::CompactCatalog>(edit)?
            .expand(&index)
            .is_err()
    );
    Ok(())
}
#[test]
#[cfg(feature = "fixture-tests")]
fn every_profile_uses_correct_keys_and_preserves_unmanaged_content() -> Result<()> {
    // Immutable 4.2.0 protocols; new protocols have their own regression cases.
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/catalog420");
    let c = serde_json::from_slice::<cloud::CompactCatalog>(&fs::read(root.join("catalog.json"))?)?
        .expand(&fs::read(root.join("index.json"))?)?;
    for p in &c.packages {
        let policy = &c.scheme_policies[&p.scheme_id];
        let files = cloud::unpack(
            p,
            &fs::read(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/fixtures/runtime/packages")
                    .join(&p.archive),
            )?,
        )?;
        for backend in &p.backends {
            let mut routed = files.clone();
            core::configure_package(backend, &mut routed)?;
            let values = presets::defaults(&policy.parameter_profile, &policy.defaults);
            let desired =
                presets::configure(&routed[core::INI], &policy.parameter_profile, &values)?;
            let mut custom = String::from_utf8(desired.clone())?;
            custom.push_str("\r\n; user comment\r\n[Custom]\r\nKeep=hello\r\n");
            let mut changed = values.clone();
            changed.insert("logging_level".into(), "3".into());
            let updated = presets::configure(&desired, &policy.parameter_profile, &changed)?;
            let merged = String::from_utf8(presets::merge(custom.as_bytes(), &updated, backend)?)?;
            assert!(merged.contains("; user comment") && merged.contains("Keep=hello"));
            let ini = cleanup::parse_ini(&merged);
            assert_eq!(ini["Logging"]["Level"], "3");
            if backend.starts_with("native") {
                assert!(!ini["FrameGeneration"].contains_key("Optimized"));
                assert_eq!(ini["Compatibility"]["HardwareBilinear"], "0");
                assert_eq!(
                    ini["Compatibility"]["Router"],
                    if backend.ends_with("20") {
                        "SM75"
                    } else {
                        "SM86"
                    }
                );
            }
            if backend == "rtx20" {
                assert_eq!(ini["Compatibility"]["KernelImage"], "Cubin");
            }
        }
    }
    assert!(
        presets::validate(
            "native026",
            &BTreeMap::from([("max_generated_frames".into(), "5".into())])
        )
        .is_err()
    );
    for n in 0..=3 {
        presets::validate(
            "upstream035",
            &BTreeMap::from([("optimized".into(), n.to_string())]),
        )?;
    }
    assert!(
        presets::validate(
            "upstream035",
            &BTreeMap::from([("Directory".into(), "C:\\game".into())])
        )
        .is_err()
    );
    Ok(())
}
#[test]
fn clear_cache_keeps_library_ini_and_cleanup_identities() -> Result<()> {
    let root = tempfile::tempdir()?;
    let data = root.path().join("data");
    let runtime = data.join("runtime-rust");
    fs::create_dir_all(runtime.join("cloud"))?;
    fs::create_dir_all(data.join("updates/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"))?;
    for p in [
        runtime.join("cloud/bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb.zip"),
        runtime.join("cloud/cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc.domestic.download.zip.aria2"),
        data.join("updates/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/RTXManager-v4.1.2-x64.exe"),
    ] {
        fs::write(p, b"cache")?;
    }
    for p in [
        data.join("games.json"),
        data.join("gpu-alias.json"),
        runtime.join("cloud-identities.json"),
        runtime.join("cloud-catalog.json"),
    ] {
        fs::write(p, b"keep")?;
    }
    let r = cache::clean_scoped(&runtime, &data)?;
    assert_eq!(r.files, 3);
    assert_eq!(r.bytes, 15);
    assert_eq!(r.skipped, 0);
    assert_eq!(fs::read(data.join("games.json"))?, b"keep");
    assert_eq!(fs::read(runtime.join("cloud-identities.json"))?, b"keep");
    assert_eq!(fs::read(data.join("gpu-alias.json"))?, b"keep");
    assert_eq!(fs::read(runtime.join("cloud-catalog.json"))?, b"keep");
    fs::create_dir_all(runtime.join("cloud"))?;
    fs::create_dir_all(data.join("updates/personal"))?;
    fs::write(runtime.join("cloud/notes.txt"), b"not cache")?;
    fs::write(data.join("updates/personal/game.exe"), b"game")?;
    let r = cache::clean_scoped(&runtime, &data)?;
    assert_eq!(r.files, 0);
    assert_eq!(fs::read(runtime.join("cloud/notes.txt"))?, b"not cache");
    assert_eq!(fs::read(data.join("updates/personal/game.exe"))?, b"game");
    Ok(())
}
