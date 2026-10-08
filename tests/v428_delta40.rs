use anyhow::Result;
use rtx_fg_manager::{cloud, core, delta, diagnostics, presets};
use serde_json::json;
use std::{collections::BTreeMap, path::Path};

#[test]
fn bundled_and_existing_online_catalogs_enable_only_delta_sm89() -> Result<()> {
    let c = cloud::bundled();
    c.validate()?;
    assert!(c.supports_series(cloud::DELTA_SCHEME, 2));
    assert_eq!(c.proxies(cloud::DELTA_SCHEME).len(), 6);
    assert!(!c.supports_series(&c.default_scheme, 2));
    let index = serde_json::to_vec(&json!({"schema":1,"packages":c.packages}))?;
    let schemes = c.schemes().iter().map(|p| {
        let policy = &c.scheme_policies[&p.scheme_id];
        json!({"id":p.scheme_id,"name":p.label,"profile":policy.parameter_profile,
            "defaults":policy.defaults,"capabilities":policy.capabilities,
            "archives":c.packages.iter().filter(|q| q.scheme_id == p.scheme_id).map(|q| &q.archive).collect::<Vec<_>>()})
    }).collect::<Vec<_>>();
    let compact = json!({"schema":2,"revision":c.revision,"default_scheme":c.default_scheme,
        "sources":c.sources,"schemes":schemes,"index":{
        "url":"https://gitee.com/pandaligx/RTX-FG-Manager/raw/main/index.json",
        "fallback_url":"https://raw.githubusercontent.com/pandaligx/RTX-FG-Manager/main/index.json",
        "bytes":index.len(),"sha256":core::hash(&index)}});
    let expanded =
        serde_json::from_value::<cloud::CompactCatalog>(compact.clone())?.expand(&index)?;
    assert!(expanded.supports_series(cloud::DELTA_SCHEME, 2));
    assert!(!expanded.supports_series(&expanded.default_scheme, 2));
    let mut no_capability = compact;
    for scheme in no_capability["schemes"].as_array_mut().unwrap() {
        if scheme["id"] == cloud::DELTA_SCHEME {
            scheme["capabilities"] = json!([]);
        }
    }
    let expanded =
        serde_json::from_value::<cloud::CompactCatalog>(no_capability)?.expand(&index)?;
    assert!(!expanded.supports_series(cloud::DELTA_SCHEME, 2));
    let mut invalid = c.clone();
    invalid
        .scheme_policies
        .get_mut(&c.default_scheme)
        .unwrap()
        .gpu_paths
        .push("SM89".into());
    assert!(invalid.validate().is_err());
    let mut invalid = c;
    invalid
        .scheme_policies
        .get_mut(cloud::DELTA_SCHEME)
        .unwrap()
        .capabilities
        .clear();
    assert!(invalid.validate().is_err());
    Ok(())
}

#[test]
fn rtx40_delta_settings_keep_auto_hardware_and_game_scoping() -> Result<()> {
    let c = cloud::bundled();
    let p = &c.scheme_policies[cloud::DELTA_SCHEME];
    let game = Path::new("C:/Games/DeltaForce/Binaries/Win64").join(delta::GAME);
    let ctx = presets::Context::new(cloud::DELTA_SCHEME, p, &game);
    for n in 0..=3 {
        let values = BTreeMap::from([("max_generated_frames".into(), n.to_string())]);
        let bytes = ctx.configure(b"[Runtime]\nMode=Bundled\nCacheDirectory=\n", &values)?;
        let mut files = BTreeMap::from([(core::INI.into(), bytes.clone())]);
        core::configure_package("upstream_sm86", &mut files)?;
        assert_eq!(files[core::INI], bytes);
        let text = String::from_utf8(bytes)?;
        assert_eq!(
            diagnostics::ini_value(&text, "Compatibility", "DeltaForceGeneratedFrames")?,
            Some(n.to_string())
        );
        assert_eq!(
            diagnostics::ini_value(&text, "Compatibility", "DeltaForcePrivateStreamline")?,
            Some(u8::from(n >= 2).to_string())
        );
        for key in ["ForceSM86Route", "SimulateAmpere", "Router"] {
            assert!(!text.contains(key));
        }
    }
    let other = presets::Context::new(cloud::DELTA_SCHEME, p, Path::new("C:/Other.exe"));
    assert!(!other.delta);
    let configured = String::from_utf8(other.configure(b"", &p.defaults)?)?;
    assert_ne!(
        diagnostics::ini_value(&configured, "Compatibility", "DeltaForcePrivateStreamline")?
            .as_deref(),
        Some("1")
    );
    Ok(())
}
