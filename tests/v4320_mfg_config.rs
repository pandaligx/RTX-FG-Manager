use anyhow::Result;
use rtx_fg_manager::{presets, rtxmfg};
use serde_json::{Value, json};

// MFG 1.7's parser accepts only these strings; an absent field selects its
// host-aware default. One unrecognised string rejects the entire config.
fn upstream_accepts_ui(value: &Value) -> bool {
    value
        .get("optiScalerUi")
        .is_none_or(|mode| matches!(mode.as_str(), Some("shared" | "rtxmfg" | "optiscaler")))
}

#[test]
fn mfg_default_and_saved_auto_use_upstream_implicit_default() -> Result<()> {
    let defaults = presets::defaults(rtxmfg::PROFILE, &Default::default());
    let output: Value = serde_json::from_slice(&rtxmfg::configure(b"{}", &defaults)?)?;
    assert!(upstream_accepts_ui(&output));
    assert!(output.get("optiScalerUi").is_none());

    let saved = presets::Values::from([
        ("rtx_mode".into(), "3".into()),
        ("rtx_opti_ui".into(), "auto".into()),
    ]);
    let output: Value = serde_json::from_slice(&rtxmfg::configure(b"{}", &saved)?)?;
    assert!(upstream_accepts_ui(&output));
    assert_eq!(output["followGame"], false);
    assert_eq!(output["mode"], "fixed");
    assert_eq!(output["multiplier"], 3);
    Ok(())
}

#[test]
fn mfg_legacy_auto_is_repaired_even_without_other_edits() -> Result<()> {
    let source = br#"{"version":13,"optiScalerUi":"auto","followGame":false,"mode":"fixed","multiplier":3,"future":{"untouched":true}}"#;
    for values in [
        presets::Values::new(),
        presets::Values::from([("rtx_mode".into(), "3".into())]),
        presets::Values::from([("rtx_gpu_override".into(), "0".into())]),
    ] {
        let output = rtxmfg::configure(source, &values)?;
        let result: Value = serde_json::from_slice(&output)?;
        assert!(upstream_accepts_ui(&result));
        assert_eq!(result["multiplier"], 3);
        assert_eq!(result["future"], json!({"untouched":true}));
        assert_eq!(rtxmfg::configure(&output, &presets::Values::new())?, output);
    }
    Ok(())
}

#[test]
fn mfg_explicit_menu_modes_remain_valid_and_unedited_configs_keep_bytes() -> Result<()> {
    for mode in ["shared", "optiscaler", "rtxmfg"] {
        let values = presets::Values::from([("rtx_opti_ui".into(), mode.into())]);
        let output = rtxmfg::configure(br#"{"optiScalerUi":"auto"}"#, &values)?;
        let result: Value = serde_json::from_slice(&output)?;
        assert_eq!(result["optiScalerUi"], mode);
        assert!(upstream_accepts_ui(&result));
        assert_eq!(rtxmfg::configure(&output, &presets::Values::new())?, output);
    }
    for source in [
        b"\xef\xbb\xbf{\r\n  \"version\": 13, \"future\": 1\r\n}".as_slice(),
        br#"{"optiScalerUi":"new-upstream-mode","future":2}"#,
    ] {
        assert_eq!(rtxmfg::configure(source, &presets::Values::new())?, source);
    }
    Ok(())
}

#[test]
fn mfg_161_does_not_receive_170_menu_fields() -> Result<()> {
    let values = presets::Values::from([("rtx_mode".into(), "3".into())]);
    rtxmfg::validate_version(Some("1.6.1"), &values)?;
    let output: Value = serde_json::from_slice(&rtxmfg::configure(b"{}", &values)?)?;
    assert!(output.get("optiScalerUi").is_none());
    assert!(
        rtxmfg::validate_version(
            Some("1.6.1"),
            &presets::Values::from([("rtx_opti_ui".into(), "auto".into())])
        )
        .is_err()
    );
    Ok(())
}
