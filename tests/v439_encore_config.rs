//! Configuration-only regression tests; no DLL is loaded and no GPU is used.
use rtx_fg_manager::{encore, jsonc::Document, presets::Values};
use serde_json::json;

fn values(pairs: &[(&str, &str)]) -> Values {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

#[test]
fn open_migration_is_one_time_preserves_user_content_and_requires_later_enable() {
    let original = b"\xef\xbb\xbf{\r\n// keep comment\r\n\"configVersion\":4,\"nrEnabled\":true,\"neuralRendering\":{\"core\":{\"nrEngine\":\"opendlss\"},\"openExperimental\":{\"nrOpenF16Weights\":true,\"userNote\":42}},\"menuState\":{\"menuKey\":36,\"layout\":7},\"vendor\":{\"nrEngine\":\"opendlss\"}}\r\n";
    assert_eq!(
        encore::configure_for_version(original, &Values::new(), Some("1.2.2")).unwrap(),
        original
    );
    assert_eq!(
        encore::read_for_version(original, Some("1.2.2")).unwrap()["nrEnabled"],
        "1"
    );
    assert_eq!(
        encore::read_for_version(original, Some("1.3.0")).unwrap()["nrEnabled"],
        "0"
    );
    // A stale enabled draft must not silently switch to NVIDIA's engine.
    let migrated =
        encore::configure_for_version(original, &values(&[("nrEnabled", "1")]), Some("1.3.0"))
            .unwrap();
    let doc = Document::parse(&migrated).unwrap();
    assert_eq!(doc.value["neuralRendering"]["core"]["nrEnabled"], false);
    assert!(
        doc.value["neuralRendering"]["core"]
            .get("nrEngine")
            .is_none()
    );
    assert!(doc.value.get("nrEnabled").is_none());
    assert_eq!(
        doc.value["neuralRendering"]["openExperimental"],
        json!({"userNote":42})
    );
    assert_eq!(doc.value["menuState"], json!({"menuKey":36,"layout":7}));
    assert_eq!(doc.value["vendor"]["nrEngine"], "opendlss");
    assert!(migrated.starts_with(b"\xef\xbb\xbf"));
    assert!(String::from_utf8_lossy(&migrated).contains("// keep comment\r\n"));
    assert_eq!(encore::migrate_130(&migrated).unwrap(), migrated);
    let enabled =
        encore::configure_for_version(&migrated, &values(&[("nrEnabled", "1")]), Some("1.3.0"))
            .unwrap();
    assert_eq!(
        encore::read_for_version(&enabled, Some("1.3.0")).unwrap()["nrEnabled"],
        "1"
    );
}

#[test]
fn nvidia_migration_keeps_activation_and_does_not_delete_unknown_objects() {
    let original = br#"{"configVersion":4,"neuralRendering":{"core":{"nrEngine":"nvidia","nrEnabled":true},"openExperimental":{"nrOpenSeed":5,"customPlugin":true}},"vendor":{"nrOpenSeed":17}}"#;
    let out = encore::migrate_130(original).unwrap();
    assert_eq!(encore::read(&out).unwrap()["nrEnabled"], "1");
    let doc = Document::parse(&out).unwrap();
    assert_eq!(
        doc.value["neuralRendering"]["openExperimental"],
        json!({"customPlugin":true})
    );
    assert_eq!(doc.value["vendor"]["nrOpenSeed"], 17);
    let plain = b"{ /* preserve bytes */ \"configVersion\":4,\"menuState\":{\"menuKey\":36}}";
    assert_eq!(encore::migrate_130(plain).unwrap(), plain);
}

#[test]
fn version_gates_prevent_stale_open_writes_and_new_settings_in_old_dlls() {
    for f in encore::fields()
        .iter()
        .filter(|f| f.key == "nrEngine" || f.key.starts_with("nrOpen"))
    {
        assert!(encore::field_available(f, Some("1.2.2")));
        assert!(!encore::field_available(f, Some("1.3.0")));
        assert!(
            encore::configure_for_version(b"{}", &values(&[(&f.key, &f.default)]), Some("1.3.0"))
                .is_err()
        );
    }
    for f in encore::fields()
        .iter()
        .filter(|f| f.since.as_deref() == Some("1.3.0"))
    {
        assert!(encore::field_available(f, Some("1.3.0")));
        for version in [None, Some("1.2.2"), Some("1.3.0-beta.1")] {
            assert!(!encore::field_available(f, version));
            assert!(
                encore::configure_for_version(b"{}", &values(&[(&f.key, &f.default)]), version)
                    .is_err()
            );
        }
    }
}

#[test]
fn foveation_ranges_presets_visibility_and_turing_restart_are_correct() {
    let mut v = encore::defaults();
    assert_eq!(v["nrFoveation"], "0");
    assert!(!encore::field_enabled("nrFoveation", &v, None));
    v.extend(values(&[
        ("nrEnabled", "1"),
        ("nrFoveation", "1"),
        ("nrFoveaPreset", "custom"),
        ("nrFoveaWidth", "72"),
        ("smoothMotionEnabled", "1"),
        ("smoothMotionTuringVariant", "v2"),
    ]));
    assert!(encore::field_enabled("nrFoveaWidth", &v, None));
    assert!(encore::field_visible(
        encore::field("nrFoveaWidth").unwrap(),
        &v
    ));
    v.insert("nrFoveaPreset".into(), "balanced".into());
    assert!(!encore::field_visible(
        encore::field("nrFoveaWidth").unwrap(),
        &v
    ));
    let explicit: Values = v
        .into_iter()
        .filter(|(k, _)| encore::field_available(encore::field(k).unwrap(), Some("1.3.0")))
        .collect();
    let out =
        encore::configure_for_version(encore::DEFAULT_CONFIG, &explicit, Some("1.3.0")).unwrap();
    let doc = Document::parse(&out).unwrap();
    assert_eq!(
        doc.value["neuralRendering"]["experimental"]["nrFoveaWidth"],
        72
    );
    assert_eq!(doc.value["smoothMotion"]["smoothMotionTuringVariant"], "v2");
    assert!(encore::requires_restart(
        encore::field("smoothMotionTuringVariant").unwrap(),
        Some("1.3.0")
    ));
    assert!(!encore::requires_restart(
        encore::field("nrFoveation").unwrap(),
        Some("1.3.0")
    ));
    for (key, min, max) in [
        ("nrFoveaWidth", 1, 100),
        ("nrFoveaHeight", 1, 100),
        ("nrFoveaFalloff", 0, 100),
        ("nrFoveaEdge", 25, 100),
    ] {
        for n in [min, max] {
            assert!(encore::valid_value(key, &n.to_string()));
        }
        for n in [min - 1, max + 1] {
            assert!(!encore::valid_value(key, &n.to_string()));
        }
        for n in ["NaN", "1.5", "-1"] {
            assert!(!encore::valid_value(key, n));
        }
    }
    assert!(encore::validate_cloud_config(&out).is_err()); // Activation is opt-in, never a package default.
    encore::validate_cloud_config(encore::DEFAULT_CONFIG).unwrap();
}
