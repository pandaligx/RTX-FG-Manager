use rtx_fg_manager::{encore, jsonc::Document, presets::Values, recommendation_presets::Snapshot};

#[test]
fn rr2_saved_by_the_game_remains_readable_and_is_version_gated() {
    let original =
        br#"{"configVersion":4,"dlssPresets":{"dlssRrPreset":"f"},"menuState":{"open":true}}"#;
    assert_eq!(
        encore::read_for_version(original, Some("1.5.3")).unwrap()["dlssRrPreset"],
        "f"
    );
    let values = Values::from([("dlssRrPreset".into(), "f".into())]);
    assert!(encore::configure_for_version(b"{}", &values, Some("1.5.2")).is_err());
    let out = encore::configure_for_version(original, &values, Some("1.5.3")).unwrap();
    assert_eq!(
        Document::parse(&out).unwrap().value["menuState"]["open"],
        true
    );
}

#[test]
fn vsync_migration_retains_intent_comments_and_unrelated_fields() {
    for (old, expected) in [(true, "off"), (false, "game")] {
        let source = format!(
            "{{\r\n// player comment\r\n\"nativeMenuFeatures\":{{\"vsyncOff\":{old}}},\"vendor\":{{\"vsyncOff\":true}},\"menuState\":{{\"menuKey\":36}}}}"
        );
        assert_eq!(
            encore::read_for_version(source.as_bytes(), Some("1.5.3")).unwrap()["vsync"],
            expected
        );
        let out = encore::configure_for_version(source.as_bytes(), &Values::new(), Some("1.5.3"))
            .unwrap();
        let doc = Document::parse(&out).unwrap();
        assert_eq!(doc.value["nativeMenuFeatures"]["vsync"], expected);
        assert!(doc.value["nativeMenuFeatures"].get("vsyncOff").is_none());
        assert_eq!(doc.value["vendor"]["vsyncOff"], true);
        assert_eq!(doc.value["menuState"]["menuKey"], 36);
        assert!(String::from_utf8_lossy(&out).contains("// player comment\r\n"));
        assert_eq!(
            encore::configure_for_version(&out, &Values::new(), Some("1.5.3")).unwrap(),
            out
        );
    }
}

#[test]
fn modern_vsync_wins_and_retired_turing_cannot_block_new_config() {
    let source = br#"{"nativeMenuFeatures":{"vsync":"on","vsyncOff":true},"smoothMotion":{"smoothMotionTuringVariant":"v1"},"vendor":{"smoothMotionTuringVariant":"v1"}}"#;
    let out = encore::configure_for_version(source, &Values::new(), Some("1.5.3")).unwrap();
    let doc = Document::parse(&out).unwrap();
    assert_eq!(doc.value["nativeMenuFeatures"]["vsync"], "on");
    assert!(
        doc.value["smoothMotion"]
            .get("smoothMotionTuringVariant")
            .is_none()
    );
    assert_eq!(doc.value["vendor"]["smoothMotionTuringVariant"], "v1");
    assert!(!encore::field_available(
        encore::field("smoothMotionTuringVariant").unwrap(),
        Some("1.5.1")
    ));
    assert!(encore::field_available(
        encore::field("smoothMotionTuringVariant").unwrap(),
        Some("1.5.0")
    ));
    assert!(!encore::field_available(
        encore::field("vsyncOff").unwrap(),
        Some("1.5.2")
    ));
    assert!(encore::field_available(
        encore::field("vsyncOff").unwrap(),
        Some("1.5.1")
    ));
}

#[test]
fn old_shares_convert_both_values_and_comparison_defaults() {
    let old = Snapshot {
        schema: 1,
        profile: encore::PROFILE.into(),
        values: Values::from([
            ("vsyncOff".into(), "1".into()),
            ("smoothMotionTuringVariant".into(), "v1".into()),
            ("tf_mode".into(), "3".into()),
        ]),
        defaults: Values::from([
            ("vsyncOff".into(), "0".into()),
            ("smoothMotionTuringVariant".into(), "v1".into()),
            ("tf_mode".into(), "game".into()),
        ]),
    };
    let new = old.for_version(encore::SCHEME, "1.5.0", "1.5.3").unwrap();
    assert_eq!(new.values["vsync"], "off");
    assert_eq!(new.defaults["vsync"], "game");
    assert!(!new.values.contains_key("vsyncOff"));
    assert!(!new.values.contains_key("smoothMotionTuringVariant"));
    assert_eq!(new.values["tf_mode"], "3");
    assert_eq!(old.values["vsyncOff"], "1");
    assert!(new.for_version(encore::SCHEME, "1.5.3", "1.5.0").is_err());
}

#[test]
fn current_template_can_still_configure_old_installs_without_new_keys() {
    for version in ["1.0.0", "1.4.5", "1.5.0", "1.5.1"] {
        let out = encore::configure_for_version(
            encore::DEFAULT_CONFIG,
            &Values::from([("vsyncOff".into(), "1".into())]),
            Some(version),
        )
        .unwrap();
        let doc = Document::parse(&out).unwrap();
        assert_eq!(doc.value["nativeMenuFeatures"]["vsyncOff"], true);
        assert!(doc.value["nativeMenuFeatures"].get("vsync").is_none());
        encore::validate_template(&out).unwrap();
        assert_eq!(
            encore::read_for_version(&out, Some(version)).unwrap()["vsyncOff"],
            "1"
        );
    }
    let out = encore::configure(
        encore::DEFAULT_CONFIG,
        &Values::from([("vsyncOff".into(), "1".into())]),
    )
    .unwrap();
    assert_eq!(encore::read(&out).unwrap()["vsync"], "off");
}

#[test]
fn malformed_active_values_fail_but_retired_aliases_cannot_override_the_new_field() {
    for bytes in [
        br#"{"nativeMenuFeatures":{"vsync":"unknown"}}"#.as_slice(),
        br#"{"nativeMenuFeatures":{"vsyncOff":1}}"#,
    ] {
        assert!(encore::configure_for_version(bytes, &Values::new(), Some("1.5.3")).is_err());
    }
    let old = br#"{"vsyncOff":true,"nativeMenuFeatures":{"vsync":"on","vsyncOff":"obsolete"},"smoothMotionTuringVariant":"obsolete","vendor":{"vsync":"untouched"}}"#;
    let migrated = encore::configure_for_version(old, &Values::new(), Some("1.5.3")).unwrap();
    let doc = Document::parse(&migrated).unwrap();
    assert_eq!(doc.value["nativeMenuFeatures"]["vsync"], "on");
    assert_eq!(doc.value["vendor"]["vsync"], "untouched");
    assert!(doc.value.get("vsyncOff").is_none());
    assert!(doc.value.get("smoothMotionTuringVariant").is_none());
    assert!(encore::configure_for_version(&migrated, &Values::new(), Some("1.5.0")).is_err());
}
