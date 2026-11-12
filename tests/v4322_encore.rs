use rtx_fg_manager::{encore, jsonc::Document, presets::Values, recommendation_presets::Snapshot};

#[test]
fn pacing_migration_accepts_game_written_auto_and_preserves_explicit_choices() {
    for (wire, expected) in [("true", "1"), ("false", "0"), ("\"auto\"", "auto")] {
        let input = format!(
            "{{\r\n// player comment\r\n\"frameGeneration\":{{\"flipMeteringBypass\":{wire},\"flipMeteringLearned\":\"bypass\"}},\"menuState\":{{\"locale\":\"zh-CN\"}},\"vendor\":{{\"keep\":42}}}}"
        );
        let values = encore::read_for_version(input.as_bytes(), Some("2.0.1")).unwrap();
        assert_eq!(values["flipMeteringBypass"], expected);
        let out = encore::configure_for_version(
            input.as_bytes(),
            &Values::from([("dlssRenderScale".into(), "ultra".into())]),
            Some("2.0.1"),
        )
        .unwrap();
        let doc = Document::parse(&out).unwrap();
        assert_eq!(
            doc.value["frameGeneration"]["flipMeteringLearned"],
            "bypass"
        );
        assert_eq!(doc.value["menuState"]["locale"], "zh-CN");
        assert_eq!(doc.value["vendor"]["keep"], 42);
        assert!(String::from_utf8_lossy(&out).contains("// player comment\r\n"));
        assert_eq!(
            encore::configure_for_version(&out, &Values::new(), Some("2.0.1")).unwrap(),
            out
        );
    }
    let legacy = br#"{"compatibility":{"patchFlipMetering":true}}"#;
    let moved = encore::migrate_frame_pacing(legacy).unwrap();
    assert_eq!(
        encore::read_for_version(&moved, Some("2.0.1")).unwrap()["flipMeteringBypass"],
        "1"
    );
}

#[test]
fn new_defaults_and_old_template_use_distinct_pacing_types() {
    assert_eq!(
        encore::read_for_version(b"{}", Some("2.0.1")).unwrap()["flipMeteringBypass"],
        "auto"
    );
    assert_eq!(
        encore::defaults_for_version(Some("1.6.0"))["flipMeteringBypass"],
        "0"
    );
    assert_eq!(
        encore::defaults_for_version(Some("2.0.1"))["flipMeteringBypass"],
        "auto"
    );
    for v in ["1.0.0", "1.5.3", "1.6.0"] {
        let out =
            encore::configure_for_version(encore::DEFAULT_CONFIG, &Values::new(), Some(v)).unwrap();
        assert_eq!(
            encore::read_for_version(&out, Some(v)).unwrap()["flipMeteringBypass"],
            "0"
        );
        assert!(
            encore::configure_for_version(
                &out,
                &Values::from([("flipMeteringBypass".into(), "auto".into())]),
                Some(v)
            )
            .is_err()
        );
    }
    encore::validate_cloud_config(encore::DEFAULT_CONFIG).unwrap();
    let values = encore::read(encore::DEFAULT_CONFIG).unwrap();
    for k in [
        "dlssWithoutGameDlss",
        "nrWithoutGameDlss",
        "frameGenerationWithoutGameFg",
    ] {
        assert_eq!(values[k], "0");
    }
}

#[test]
fn added_features_are_gated_and_not_confused_with_native_paths() {
    for (key, value) in [
        ("dlssWithoutGameDlss", "1"),
        ("dlssWithoutGameDlssEngine", "xess"),
        ("dlssWithoutGameDlssOutput", "screen"),
        ("presentOpticalFlow", "fast"),
        ("nrWithoutGameDlss", "1"),
        ("frameGenerationWithoutGameFg", "1"),
        ("dlssRenderScale", "ultra"),
    ] {
        let values = Values::from([(key.into(), value.into())]);
        assert!(
            encore::configure_for_version(b"{}", &values, Some("1.6.0")).is_err(),
            "{key}"
        );
        let out = encore::configure_for_version(b"{}", &values, Some("2.0.1")).unwrap();
        assert_eq!(encore::read(&out).unwrap()[key], value);
    }
    let v = Values::from([
        ("nrEnabled".into(), "0".into()),
        ("nrWithoutGameDlss".into(), "1".into()),
    ]);
    assert!(encore::field_enabled("nrIntensity", &v, None));
    let xess = Values::from([
        ("dlssWithoutGameDlss".into(), "1".into()),
        ("dlssWithoutGameDlssEngine".into(), "xess".into()),
    ]);
    assert!(encore::field_enabled("xessTensorCores", &xess, None));
    assert!(encore::requires_restart(
        encore::field("dlssSrPreset").unwrap(),
        Some("2.0.1")
    ));
    assert!(!encore::requires_restart(
        encore::field("dlssSrPreset").unwrap(),
        Some("1.6.0")
    ));
}

#[test]
fn game_written_menu_languages_remain_readable_without_leaking_into_shares() {
    for value in [
        "en", "fr", "zh-CN", "de", "es", "nl", "pl", "pt-BR", "vi", "ru", "ja",
    ] {
        let input = format!("{{\"menuState\":{{\"menuLanguage\":\"{value}\",\"localLayout\":7}}}}");
        assert_eq!(
            encore::read(input.as_bytes()).unwrap()["menuLanguage"],
            value
        );
        let out = encore::configure_for_version(
            input.as_bytes(),
            &Values::from([("tf_mode".into(), "3".into())]),
            Some("2.0.1"),
        )
        .unwrap();
        assert_eq!(
            Document::parse(&out).unwrap().value["menuState"]["localLayout"],
            7
        );
    }
    assert!(!rtx_fg_manager::recommendation_presets::shareable(
        encore::PROFILE,
        "menuLanguage"
    ));
}

#[test]
fn shares_keep_old_pacing_intent_and_never_publish_the_local_learning() {
    let old = Snapshot {
        schema: 1,
        profile: encore::PROFILE.into(),
        values: Values::from([("flipMeteringBypass".into(), "1".into())]),
        defaults: Values::from([("flipMeteringBypass".into(), "0".into())]),
    };
    let new = old.for_version(encore::SCHEME, "1.6.0", "2.0.1").unwrap();
    assert_eq!(new.values, old.values);
    assert_eq!(new.defaults, old.defaults);
    let mut auto = new.clone();
    auto.values
        .insert("flipMeteringBypass".into(), "auto".into());
    auto.validate(encore::SCHEME, "2.0.1").unwrap();
    assert!(auto.validate(encore::SCHEME, "1.6.0").is_err());
    assert!(!rtx_fg_manager::recommendation_presets::shareable(
        encore::PROFILE,
        "flipMeteringLearned"
    ));
    auto.values
        .insert("flipMeteringLearned".into(), "bypass".into());
    auto.defaults
        .insert("flipMeteringLearned".into(), "bypass".into());
    assert!(auto.validate(encore::SCHEME, "2.0.1").is_err());
}

#[test]
fn added_runtime_inventory_requires_one_matched_bundle_and_encore() {
    use rtx_fg_manager::recommendation_runtimes::{Component, Snapshot};
    let mut s = Snapshot {
        schema: 1,
        components: vec![Component {
            kind: "encorefg".into(),
            source: "managed".into(),
            versions: vec!["2.14.16".into(), "310.9.16".into()],
            package_version: Some("2.14.16".into()),
        }],
    };
    s.compatible_with(encore::SCHEME, &["version.dll".into()])
        .unwrap();
    assert!(
        s.compatible_with("rtx40mfg-1.3.3-hf2", &["version.dll".into()])
            .is_err()
    );
    s.components[0].versions.pop();
    assert!(s.validate().is_err());
}
