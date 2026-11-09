//! Configuration/ownership tests; these do not load the signed game DLL.
use anyhow::Result;
use rtx_fg_manager::{
    encore,
    encore_install::{Choice, Frames, Reconstruction},
    presets::Values,
    recommendation_presets::Snapshot,
    scanner::Evidence,
    transfer,
};

#[test]
fn nis_has_no_dlss_or_intel_runtime_requirement_and_is_version_gated() {
    let mut plan = Choice::new(
        "fixture.exe".into(),
        "Fixture".into(),
        &Evidence::default(),
        false,
    );
    plan.frames = Some(Frames::None);
    plan.reconstruction = Some(Reconstruction::Nis);
    plan.components.sr = false;
    plan.components.intel = false;
    assert!(plan.values().is_none());
    plan.nis_available = true;
    let values = plan.values().unwrap();
    assert_eq!(values["dlssWithoutGameDlssEngine"], "nis");
    assert!(encore::configure_for_version(b"{}", &values, Some("2.1.0")).is_err());
    assert!(encore::configure_for_version(b"{}", &values, Some("2.2.0")).is_ok());
    let snapshot = Snapshot {
        schema: 1,
        profile: encore::PROFILE.into(),
        defaults: values.clone(),
        values,
    };
    assert!(snapshot.added_runtime_dependencies().is_empty());
}

#[test]
fn encore220_defaults_ranges_and_old_version_guards() -> Result<()> {
    let values = Values::from([
        ("overrideAutoFrameGeneration".into(), "0".into()),
        ("spatialAa".into(), "smaa".into()),
        ("spatialAaQuality".into(), "ultra".into()),
        ("spatialSharpness".into(), "65".into()),
        ("overlayStyle".into(), "dashboard".into()),
        ("overlayOpacity".into(), "80".into()),
    ]);
    for (key, value) in &values {
        assert!(!encore::value_available(key, value, Some("2.1.0")));
        assert!(encore::value_available(key, value, Some("2.2.0")));
    }
    let config = encore::configure_for_version(
        b"{ // retain comment\n\"vendorExtension\":17}",
        &values,
        Some("2.2.0"),
    )?;
    assert!(String::from_utf8_lossy(&config).contains("retain comment"));
    assert!(String::from_utf8_lossy(&config).contains("vendorExtension"));
    let read = encore::read(&config)?;
    for (key, value) in &values {
        assert_eq!(read[key], *value);
    }
    assert_eq!(read["overlayShowFps"], "1");
    assert_eq!(read["overlayShowSourceFps"], "1");
    assert_eq!(read["overlayGraph"], "0");
    assert!(
        encore::configure_for_version(
            b"{}",
            &Values::from([("spatialSharpness".into(), "101".into())]),
            Some("2.2.0")
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn legacy_overlay_gpu_metrics_inherit_without_overriding_explicit_switches() -> Result<()> {
    let old = br#"{ "overlay":{"metrics":{"overlayShowGpu":true},"overlayShowGpuPower":false},"menuState":{"menuLanguage":"ja"},"unknown":42 }"#;
    let upgraded = encore::configure_for_version(old, &Values::new(), Some("2.2.0"))?;
    let settings = encore::read_for_version(&upgraded, Some("2.2.0"))?;
    assert_eq!(settings["overlayShowGpuLoad"], "1");
    assert_eq!(settings["overlayShowGpuTemperature"], "1");
    assert_eq!(settings["overlayShowGpuPower"], "0");
    assert_eq!(settings["menuLanguage"], "ja");
    assert_eq!(
        upgraded,
        encore::configure_for_version(&upgraded, &Values::new(), Some("2.2.0"))?
    );
    Ok(())
}

#[test]
fn chinese_defaults_do_not_force_other_languages_or_override_explicit_choice() {
    for language in ["zh-CN", "en", "ru", "ja", "ko"] {
        let mut values = encore::defaults_for_version(Some("2.2.0"));
        encore::localize_defaults(&mut values, language);
        assert_eq!(
            values["menuLanguage"],
            if language == "zh-CN" { "zh-CN" } else { "en" }
        );
        values.extend(Values::from([("menuLanguage".into(), "fr".into())]));
        assert_eq!(values["menuLanguage"], "fr");
    }
}

#[test]
fn cancellation_survives_error_context_but_does_not_mask_genuine_errors() {
    let cancelled = anyhow::Error::new(transfer::Failure::Cancelled).context("runtime preparation");
    assert!(transfer::is_cancelled(&cancelled));
    assert!(transfer::is_cancelled(
        &anyhow::anyhow!("已取消").context("payload")
    ));
    assert!(!transfer::is_cancelled(
        &anyhow::anyhow!("hash mismatch").context("download")
    ));
    assert!(!transfer::is_cancelled(&anyhow::anyhow!(
        "network failed after 已取消"
    )));
}
