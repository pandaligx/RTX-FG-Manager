use rtx_fg_manager::{encore, jsonc, presets::Values, recommendation_presets::Snapshot};

#[test]
fn new_controls_are_independent_versioned_and_preserve_jsonc() {
    let source = b"{\r\n// player comment\r\n\"configVersion\":4,\"menuState\":{\"menuKey\":36},\"vendor\":{\"xessTensorCores\":true}}";
    for version in ["1.4.5", "1.4.6", "1.4.7"] {
        assert!(!encore::field_available(
            encore::field("xessTensorCores").unwrap(),
            Some(version)
        ));
        assert!(
            encore::configure_for_version(
                source,
                &Values::from([("xessTensorCores".into(), "1".into())]),
                Some(version)
            )
            .is_err()
        );
    }
    let settings = Values::from([
        ("xessTensorCores".into(), "1".into()),
        ("xefgTensorCores".into(), "0".into()),
        ("reflexMode".into(), "on-boost".into()),
    ]);
    let out = encore::configure_for_version(source, &settings, Some("1.5.0")).unwrap();
    let doc = jsonc::Document::parse(&out).unwrap();
    assert_eq!(doc.value["renderingEngines"]["xessTensorCores"], true);
    assert_eq!(doc.value["renderingEngines"]["xefgTensorCores"], false);
    assert_eq!(doc.value["menuState"]["menuKey"], 36);
    assert_eq!(doc.value["vendor"]["xessTensorCores"], true);
    assert!(String::from_utf8_lossy(&out).contains("// player comment\r\n"));
    assert_eq!(encore::read(&out).unwrap()["frameGenerationEngine"], "game");
    assert_eq!(encore::defaults()["xessTensorCores"], "0");
    assert_eq!(encore::defaults()["xefgTensorCores"], "0");
    assert_eq!(
        encore::configure_for_version(&out, &Values::new(), Some("1.5.0")).unwrap(),
        out
    );
}

#[test]
fn dlss_fg_choice_and_shared_snapshot_require_146() {
    let settings = Values::from([("frameGenerationEngine".into(), "dlss-fg".into())]);
    let snapshot = Snapshot {
        schema: 1,
        profile: encore::PROFILE.into(),
        values: settings.clone(),
        defaults: Values::from([("frameGenerationEngine".into(), "game".into())]),
    };
    for version in ["1.2.0", "1.4.5"] {
        assert!(!encore::value_available(
            "frameGenerationEngine",
            "dlss-fg",
            Some(version)
        ));
        assert!(snapshot.validate(encore::SCHEME, version).is_err());
    }
    for version in ["1.4.6", "1.4.7", "1.5.0"] {
        snapshot.validate(encore::SCHEME, version).unwrap();
        encore::configure_for_version(b"{}", &settings, Some(version)).unwrap();
    }
}
