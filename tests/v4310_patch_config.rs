use rtx_fg_manager::{encore, jsonc::Document, neural_runtime::Status, presets::Values, rtxmfg};
use serde_json::json;
fn values(pairs: &[(&str, &str)]) -> Values {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}
#[test]
fn encore_140_preserves_comments_menu_and_inactive_passes() {
    let before=b"{\r\n// custom\r\n\"configVersion\":4,\"neuralRendering\":{\"passModels\":{\"nrPass4Preset\":\"a\",\"vendor\":7}},\"menuState\":{\"menuKey\":36,\"menuLanguage\":\"fr\",\"window\":5}}";
    let edits = values(&[
        ("nrPlacement", "before-sr"),
        ("nrPass1Preset", "b"),
        ("menuLanguage", "zh-CN"),
    ]);
    assert!(encore::configure_for_version(before, &edits, Some("1.3.0")).is_err());
    let after = encore::configure_for_version(before, &edits, Some("1.4.1")).unwrap();
    assert_eq!(
        after,
        encore::configure_for_version(before, &edits, Some("1.4.2")).unwrap()
    );
    assert!(String::from_utf8_lossy(&after).contains("// custom\r\n"));
    let d = Document::parse(&after).unwrap();
    assert_eq!(
        d.value["neuralRendering"]["core"]["nrPlacement"],
        "before-sr"
    );
    assert_eq!(
        d.value["neuralRendering"]["passModels"]["nrPass4Preset"],
        "a"
    );
    assert_eq!(d.value["neuralRendering"]["passModels"]["vendor"], 7);
    assert_eq!(
        d.value["menuState"],
        json!({"menuKey":36,"menuLanguage":"zh-CN","window":5})
    );
    let read = encore::read(&after).unwrap();
    assert_eq!(read["nrPass1Preset"], "b");
    assert!(!encore::field_enabled("nrFoveation", &read, None));
    assert!(!encore::field_enabled("nrResolution", &read, None));
}
#[test]
fn mfg_170_uses_upstream_json_types_and_keeps_unexposed_options() {
    let before=br#"{"followGame":true,"dlss5HighlightGuard":true,"dlss5OverallIntensity":0.35,"dlss5PassCount":4,"xemfgFrames":5,"custom":42}"#;
    let edits = values(&[
        ("rtx_nr", "1"),
        ("rtx_nr_scale_auto", "1"),
        ("rtx_nr_intensity", "0.75"),
        ("rtx_xemfg", "1"),
        ("rtx_xemfg_dynamic", "1"),
        ("rtx_opti_ui", "optiscaler"),
    ]);
    assert!(rtxmfg::validate_version(Some("1.6.1"), &edits).is_err());
    rtxmfg::validate_version(Some("1.7.0"), &edits).unwrap();
    let output = rtxmfg::configure(before, &edits).unwrap();
    let d: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(d["dlss5"], true);
    assert_eq!(d["dlss5ResolutionScaleAuto"], true);
    assert_eq!(d["xemfgStandalone"], 1);
    assert_eq!(d["xemfgDynamic"], 1);
    assert_eq!(d["dlss5OverallIntensity"], 0.75);
    assert_eq!(d["dlss5DefaultsVersion"], 3);
    assert_eq!(d["xemfgFrames"], 5);
    assert_eq!(d["dlss5HighlightGuard"], true);
    assert_eq!(d["custom"], 42);
    let read = rtxmfg::read(&output).unwrap();
    for (k, v) in edits {
        assert_eq!(read[&k], v);
    }
    for v in ["NaN", "inf", "-1", "1.01"] {
        assert!(!rtxmfg::valid_value("rtx_nr_intensity", v));
    }
}
#[test]
fn optional_nr_requires_a_supported_installed_patch() {
    let mut state = Status::default();
    assert!(!state.supports("310.9.16"));
    state.mfg_version = Some("1.6.1".into());
    assert!(!state.supports("310.9.16"));
    state.mfg_version = Some("1.7.0".into());
    assert!(state.supports("310.9.16"));
    assert!(!state.supports("310.8.0"));
}

#[test]
fn mfg_explicit_edit_keeps_old_advanced_values_and_blocks_default_remigration() {
    let before=br#"{"dlss5DefaultsVersion":1,"dlss5AdaptiveExposure":true,"dlss5PassCount":8,"xemfgFrames":6}"#;
    let after = rtxmfg::configure(before, &values(&[("rtx_nr_scale_auto", "1")])).unwrap();
    let d: serde_json::Value = serde_json::from_slice(&after).unwrap();
    assert_eq!(d["dlss5DefaultsVersion"], 3);
    assert_eq!(d["dlss5AdaptiveExposure"], true);
    assert_eq!(rtxmfg::read(&after).unwrap()["rtx_nr_passes"], "8");
    assert_eq!(rtxmfg::read(&after).unwrap()["rtx_xemfg_frames"], "6");
    assert_eq!(rtxmfg::configure(before, &Values::new()).unwrap(), before);
}
