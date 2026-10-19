use rtx_fg_manager::{encore, jsonc::Document, neural_runtime, presets::Values};
use serde_json::json;

#[test]
fn pacing_reads_new_location_and_updates_without_stale_legacy_override() -> anyhow::Result<()> {
    let source = br#"{
      // Keep this comment and custom setting.
      "configVersion":4,
      "frameGeneration":{"flipMeteringBypass":true},
      "compatibility":{"flipMeteringBypass":false,"custom":17},
      "vendor":{"flipMeteringBypass":false},
      "menuState":{"position":[12,34]}
    }"#;
    assert_eq!(encore::read(source)?["flipMeteringBypass"], "1");
    assert_eq!(encore::configure(source, &Values::new())?, source);
    let changed = encore::configure(
        source,
        &Values::from([("flipMeteringBypass".into(), "0".into())]),
    )?;
    let doc = Document::parse(&changed)?;
    assert_eq!(doc.value["frameGeneration"]["flipMeteringBypass"], false);
    assert!(
        doc.value["compatibility"]
            .get("flipMeteringBypass")
            .is_none()
    );
    assert_eq!(doc.value["compatibility"]["custom"], 17);
    assert_eq!(doc.value["vendor"]["flipMeteringBypass"], false);
    assert_eq!(doc.value["menuState"]["position"], json!([12, 34]));
    assert!(String::from_utf8(changed)?.contains("// Keep this comment"));
    Ok(())
}

#[test]
fn pacing_old_game_edits_keep_legacy_location_until_upgrade() -> anyhow::Result<()> {
    let source = br#"{"configVersion":4,"compatibility":{"flipMeteringBypass":false}}"#;
    let updated = encore::configure(
        source,
        &Values::from([("flipMeteringBypass".into(), "1".into())]),
    )?;
    let doc = Document::parse(&updated)?;
    assert_eq!(doc.value["compatibility"]["flipMeteringBypass"], true);
    assert!(doc.value.get("frameGeneration").is_none());
    let migrated = encore::migrate_frame_pacing(&updated)?;
    let doc = Document::parse(&migrated)?;
    assert_eq!(doc.value["frameGeneration"]["flipMeteringBypass"], true);
    assert!(
        doc.value["compatibility"]
            .get("flipMeteringBypass")
            .is_none()
    );
    assert_eq!(encore::migrate_frame_pacing(&migrated)?, migrated);
    Ok(())
}

#[test]
fn pacing_upgrade_preserves_false_and_defaults_missing_to_false() -> anyhow::Result<()> {
    encore::validate_template(encore::DEFAULT_CONFIG)?;
    encore::validate_template(&encore::migrate_frame_pacing(encore::DEFAULT_CONFIG)?)?;
    for source in [
        br#"{"compatibility":{"flipMeteringBypass":false}}"#.as_slice(),
        b"{}",
    ] {
        let migrated = encore::migrate_frame_pacing(source)?;
        assert_eq!(
            Document::parse(&migrated)?.value["frameGeneration"]["flipMeteringBypass"],
            false
        );
    }
    assert!(
        encore::migrate_frame_pacing(br#"{"frameGeneration":{"flipMeteringBypass":"wrong"}}"#)
            .is_err()
    );
    assert!(encore::migrate_frame_pacing(br#"{"frameGeneration":17}"#).is_err());
    Ok(())
}

#[test]
fn recommended_nr_already_has_a_qualified_download_and_minimum_mod() -> anyhow::Result<()> {
    let entries: serde_json::Value =
        serde_json::from_str(include_str!("../rust/assets/neural-runtimes.json"))?;
    let latest = entries
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["version"] == neural_runtime::RECOMMENDED)
        .unwrap();
    assert_eq!(latest["version"], "310.9.16");
    assert_eq!(latest["minMod"], "1.1.0");
    assert!(!latest["urls"].as_array().unwrap().is_empty());
    assert_eq!(encore::defaults()["dlssNrPreset"], "b");
    Ok(())
}
