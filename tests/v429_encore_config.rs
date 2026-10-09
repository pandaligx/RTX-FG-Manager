use rtx_fg_manager::{encore, presets::Values};

fn values(pairs: &[(&str, &str)]) -> Values {
    pairs
        .iter()
        .map(|(k, v)| ((*k).into(), (*v).into()))
        .collect()
}
#[test]
fn upstream_template_covers_every_editable_field_and_conservative_cloud_defaults() {
    encore::validate_cloud_config(encore::DEFAULT_CONFIG).unwrap();
    let read = encore::read(encore::DEFAULT_CONFIG).unwrap();
    assert_eq!(read, encore::defaults());
    assert_eq!(encore::fields().len(), 88);
    assert_eq!(read["menuKey"], "45");
    assert_eq!(read["nrOpenFastProjection"], "1");
    assert_eq!(read["nrFastProcessing"], "1");
    assert_eq!(read["nrOpenUltraFastGhostTolerance"], "0.03");
    let mut unique = std::collections::BTreeSet::new();
    for f in encore::fields() {
        assert!(unique.insert(&f.key));
        assert!(encore::valid_value(&f.key, &f.default));
    }
}

#[test]
fn open_engine_controls_keep_values_but_disable_unsupported_rtx20() {
    let mut settings = encore::defaults();
    settings.insert("nrEnabled".into(), "1".into());
    settings.insert("nrEngine".into(), "opendlss".into());
    assert!(!encore::field_enabled("nrOpenBackend", &settings, Some(0)));
    assert!(encore::field_enabled("nrOpenBackend", &settings, Some(1)));
    assert!(encore::field_enabled("nrOpenBackend", &settings, Some(2)));
    assert_eq!(settings["nrEngine"], "opendlss");
}

#[test]
fn captured_upstream_template_has_no_unrepresented_functional_leaf() {
    fn leaves(
        value: &serde_json::Value,
        path: &mut Vec<String>,
        out: &mut std::collections::BTreeSet<Vec<String>>,
    ) {
        if let Some(object) = value.as_object() {
            for (key, child) in object {
                path.push(key.clone());
                leaves(child, path, out);
                path.pop();
            }
        } else {
            out.insert(path.clone());
        }
    }
    // This is the verbatim JSONC emitted by the signed beta.2 DLL in an isolated
    // no-window host, not a configuration synthesized from our field metadata.
    let captured = rtx_fg_manager::jsonc::Document::parse(encore::DEFAULT_CONFIG).unwrap();
    let mut actual = std::collections::BTreeSet::new();
    leaves(&captured.value, &mut Vec::new(), &mut actual);
    assert_eq!(actual.len(), 89);
    let represented: std::collections::BTreeSet<_> =
        encore::fields().iter().map(|f| f.path.clone()).collect();
    let absent: Vec<_> = actual.difference(&represented).cloned().collect();
    assert_eq!(
        absent,
        vec![
            vec!["configVersion".to_string()],
            vec!["frameGeneration".to_string(), "multiplier".to_string()]
        ]
    );
    // The version is protocol metadata. The physical multiplier is deliberately
    // represented by the existing requested-mode control, including every 2–6X.
    for n in 2..=6 {
        let changed = encore::configure(
            encore::DEFAULT_CONFIG,
            &values(&[("tf_mode", &n.to_string())]),
        )
        .unwrap();
        let doc = rtx_fg_manager::jsonc::Document::parse(&changed).unwrap();
        assert_eq!(doc.value["frameGeneration"]["mode"], "fixed");
        assert_eq!(doc.value["frameGeneration"]["multiplier"], n);
    }
}
#[test]
fn advanced_edits_roundtrip_all_fields_without_reformatting_unknown_data() {
    let mut bytes = vec![239, 187, 191];
    bytes.extend(encore::DEFAULT_CONFIG);
    let text = String::from_utf8(bytes[3..].to_vec()).unwrap();
    bytes = vec![239, 187, 191];
    bytes.extend(text.replacen("{","{\r\n// 测试 https://example.test/a//b\r\n\"vendor\":{\"nrEnabled\":\"preserve\",\"text\":\"a\\\"//b\"},",1).as_bytes());
    let changed = values(&[
        ("tf_mode", "6"),
        ("tf_target", "237"),
        ("nrEnabled", "1"),
        ("nrEngine", "opendlss"),
        ("nrPasses", "3"),
        ("nrPass3Style", "cinematic"),
        ("nrIntensity", "0.75"),
        ("nrOpenBackend", "rtx30"),
        ("nrOpenUltraFastGhostTolerance", "0.08"),
        ("overlayShowGpu", "1"),
        ("menuKey", "112"),
        ("hotkeyFixed4", "Ctrl+Shift+F4, Alt+Num4"),
    ]);
    let out = encore::configure(&bytes, &changed).unwrap();
    assert!(out.starts_with(&[239, 187, 191]));
    let s = std::str::from_utf8(&out[3..]).unwrap();
    assert!(s.contains("// 测试 https://example.test/a//b\r\n"));
    assert!(s.contains("\"vendor\":{\"nrEnabled\":\"preserve\",\"text\":\"a\\\"//b\"}"));
    assert!(s.contains("\"core\""));
    let read = encore::read(&out).unwrap();
    for (key, value) in changed {
        assert_eq!(read[&key], value, "{key}");
    }
    let dynamic = encore::configure(&out, &values(&[("tf_mode", "dynamic")])).unwrap();
    assert!(
        std::str::from_utf8(&dynamic[3..])
            .unwrap()
            .contains("\"multiplier\": 6")
    );
    assert!(encore::validate_cloud_config(&out).is_err());
}
#[test]
fn migration_keeps_old_user_values_comments_unknown_and_internal_menu_state() {
    let src=br#"{
      "configVersion":3,
      "frameGeneration":{"mode":"fixed","multiplier":3,"dynamicTargetFrameRate":165},
      "neuralRendering":{"nrEnabled":true,"nrEngine":"opendlss","nrOpenF16Weights":true,"nrOpenBackend":"sm86","nrIntensity":0.7},
      "imageQuality":{"blackwellTransfusion":false,"qualityPolicy":"transfusion"},
      "compatibility":{"smoothMotionSm86":true,"smoothMotionSm86Api":"vulkan","gpuArchitecture":"ampere"},
      "menuState":{"menuKey":36,"menuFirstLaunchShown":true,"menuWindow":"1,2,3,4"},
      "vendor":{"nrEnabled":"untouched"},
      // retain comment
      "unknown":"https://example/a//b"
    }"#;
    let out = encore::migrate(src, &values(&[("tf_overlay", "1")])).unwrap();
    let v = encore::read(&out).unwrap();
    for (key, want) in [
        ("tf_mode", "3"),
        ("tf_target", "165"),
        ("nrEnabled", "1"),
        ("nrEngine", "opendlss"),
        ("nrOpenVramForSpeed", "1"),
        ("nrOpenBackend", "rtx30"),
        ("nrIntensity", "0.7"),
        ("highMultiplierQuality", "0"),
        ("protectionTuning", "classic"),
        ("smoothMotionEnabled", "1"),
        ("smoothMotionApi", "vulkan"),
        ("gpuSeries", "rtx30"),
        ("menuKey", "36"),
        ("tf_overlay", "1"),
    ] {
        assert_eq!(v[key], want, "{key}");
    }
    let s = std::str::from_utf8(&out).unwrap();
    assert!(s.contains("// retain comment"));
    assert!(s.contains("\"menuFirstLaunchShown\":true,\"menuWindow\":\"1,2,3,4\""));
    assert!(s.contains("\"vendor\":{\"nrEnabled\":\"untouched\"}"));
    assert!(!s.contains("nrOpenF16Weights"));
    // Migration adds no unrelated default controls to an existing user's file.
    assert!(!s.contains("nrOpenUltraFast"));
    assert_eq!(encore::migrate(&out, &Values::new()).unwrap(), out);
}
#[test]
fn migration_preserves_comments_around_removed_syntax_and_canonical_wins() {
    let src=b"{\"configVersion\":3,\"mode\" /* between */ : /* value */ \"fixed\",\"multiplier\":2,\"frameGeneration\":{\"mode\":\"dynamic\",\"multiplier\":4},\"unknown\":9 // tail\n}";
    let out = encore::migrate(src, &Values::new()).unwrap();
    let s = std::str::from_utf8(&out).unwrap();
    assert!(s.contains("/* between */"));
    assert!(s.contains("/* value */"));
    assert!(s.contains("// tail"));
    assert_eq!(encore::read(&out).unwrap()["tf_mode"], "dynamic");
    let out = encore::configure(
        b"{\"configVersion\":4,\"unknown\":1 // tail\n}",
        &values(&[("nrEnabled", "1")]),
    )
    .unwrap();
    assert!(std::str::from_utf8(&out).unwrap().contains("// tail"));
    assert_eq!(encore::read(&out).unwrap()["nrEnabled"], "1");
}
#[test]
fn invalid_future_ambiguous_duplicate_types_and_ranges_fail_closed() {
    for src in [
        r#"{"configVersion":5}"#,
        r#"{"configVersion":0}"#,
        r#"{"configVersion":4,"frameGeneration":{"mode":"fixed","multiplier":7}}"#,
        r#"{"configVersion":4,"neuralRendering":{"core":{"nrEnabled":1}}}"#,
        r#"{"configVersion":4,"overlay":{"metrics":false}}"#,
        r#"{"unknown":[{"x":1,"x":2}]}"#,
        r#"{"nrEnabled":true,"neuralRendering":{"nrEnabled":false}}"#,
        r#"{"configVersion":4,"frameGeneration":{"mode":"game","mode":"fixed"}}"#,
        "{/* unfinished",
    ] {
        assert!(encore::read(src.as_bytes()).is_err(), "{src}");
    }
    for (key, value) in [
        ("nrPasses", "5"),
        ("nrIntensity", "NaN"),
        ("nrIntensity", "inf"),
        ("nrIntensity", "2.1"),
        ("nrOpenUltraFastGhostTolerance", "0.31"),
        ("overlayFontSize", "9"),
        ("logFilesKept", "0"),
        ("menuKey", "256"),
        ("nrEngine", "auto"),
        ("tf_target", "1001"),
        ("nrEnabled", "true"),
        ("nrOpenSeed", "4294967296"),
        ("unknown", "1"),
    ] {
        assert!(!encore::valid_value(key, value), "{key}={value}");
    }
}
#[test]
fn hotkey_grammar_and_dependencies_are_shared_with_the_ui() {
    for v in [
        "",
        "Ctrl+Alt+4, Ctrl+Alt+Num4",
        "Shift+F24",
        "Win+Home",
        "control+alt+NumAdd",
    ] {
        assert!(encore::valid_hotkey(v), "{v}");
    }
    for v in [
        "Ctrl",
        "Ctrl+",
        "Ctrl++4",
        "Ctrl+Control+4",
        "Alt+Left+Right",
        "Ctrl+F25",
        ",Ctrl+4",
        "Ctrl+4,",
        "Ctrl+Mouse1",
    ] {
        assert!(!encore::valid_hotkey(v), "{v}");
    }
    let mut v = encore::defaults();
    assert!(!encore::field_enabled("tf_target", &v, None));
    assert!(!encore::field_enabled("nrEngine", &v, None));
    v.extend(values(&[
        ("tf_mode", "dynamic"),
        ("nrEnabled", "1"),
        ("nrEngine", "opendlss"),
        ("nrOpenBackend", "rtx30"),
        ("nrPasses", "2"),
    ]));
    assert!(encore::field_enabled("tf_target", &v, None));
    assert!(encore::field_enabled("nrOpenFast", &v, None));
    assert!(!encore::field_enabled("nrPrecision", &v, None));
    assert!(!encore::field_enabled("nrPass3Style", &v, None));
    assert!(encore::field_enabled("smoothMotionEnabled", &v, Some(1)));
    assert!(!encore::field_enabled("smoothMotionEnabled", &v, Some(0)));
}

#[test]
fn every_editor_field_can_be_written_and_has_four_translations() {
    let translations: serde_json::Value =
        serde_json::from_str(include_str!("../rust/assets/encore-translations.json")).unwrap();
    for f in encore::fields() {
        let candidate = match f.kind {
            encore::FieldKind::Bool => if f.default == "1" { "0" } else { "1" }.into(),
            encore::FieldKind::Choice => f
                .choices
                .iter()
                .find(|c| c.value != f.default)
                .unwrap()
                .value
                .clone(),
            encore::FieldKind::Integer => f.min.unwrap_or(0.0).to_string(),
            encore::FieldKind::Decimal => f.min.unwrap_or(0.125).to_string(),
            encore::FieldKind::Hotkey => "Ctrl+Alt+F12".into(),
        };
        let out = encore::configure(
            encore::DEFAULT_CONFIG,
            &Values::from([(f.key.clone(), candidate.clone())]),
        )
        .unwrap();
        assert_eq!(encore::read(&out).unwrap()[&f.key], candidate, "{}", f.key);
        for text in std::iter::once(&f.label)
            .chain(std::iter::once(&f.help))
            .chain(f.choices.iter().map(|c| &c.label))
        {
            let list = translations[text]
                .as_array()
                .unwrap_or_else(|| panic!("Missing translation: {text}"));
            assert_eq!(list.len(), 4);
            assert!(
                list.iter()
                    .all(|v| v.as_str().is_some_and(|s| !s.is_empty()))
            );
        }
    }
}
