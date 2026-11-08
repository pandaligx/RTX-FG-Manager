use rtx_fg_manager::{encore, presets::Values, recommendation_presets::Snapshot};

#[test]
fn source_pacing_is_live_version_gated_and_preserves_jsonc() {
    let f = encore::field("xefgSourcePacing").unwrap();
    assert!(!f.restart);
    assert_eq!(f.default, "1");
    assert!(!encore::field_available(f, Some("2.0.1")));
    assert!(encore::field_available(f, Some("2.1.0")));
    let bytes = br#"{ // retain player notes
      "configVersion": 4, "xefg": {"xefgSourcePacing":false}, "unknown":{"keep":42}, "menuState":{"tab":3}
    }"#;
    let changes = Values::from([("xefgSourcePacing".into(), "1".into())]);
    let result = encore::configure_for_version(bytes, &changes, Some("2.1.0")).unwrap();
    let text = String::from_utf8(result.clone()).unwrap();
    assert!(
        text.contains("retain player notes")
            && text.contains("\"keep\":42")
            && text.contains("\"tab\":3")
    );
    assert_eq!(encore::read(&result).unwrap()["xefgSourcePacing"], "1");
    let snapshot = Snapshot {
        schema: 1,
        profile: encore::PROFILE.into(),
        values: changes,
        defaults: Values::from([("xefgSourcePacing".into(), "1".into())]),
    };
    assert!(snapshot.validate(encore::SCHEME, "2.1.0").is_ok());
    assert!(snapshot.validate(encore::SCHEME, "2.0.1").is_err());
    let mut enabled = encore::defaults();
    enabled.insert("frameGenerationEngine".into(), "xefg".into());
    assert!(encore::field_enabled("xefgSourcePacing", &enabled, None));
    enabled.insert("xefgMfg4VariablePacing".into(), "0".into());
    assert!(!encore::field_enabled("xefgSourcePacing", &enabled, None));
}
