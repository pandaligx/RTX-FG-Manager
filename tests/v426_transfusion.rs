//! Configuration/routing regressions only; no DLL is loaded and no GPU is used.
use anyhow::Result;
use rtx_fg_manager::{cloud, core, presets, transfusion};
use std::collections::BTreeMap;

#[test]
fn malformed_known_transfusion_values_are_not_silently_reported_as_defaults() {
    for text in [
        r#"{"frameGeneration":{"mode":true}}"#,
        r#"{"frameGeneration":{"mode":"fixed","multiplier":"6"}}"#,
        r#"{"frameGeneration":{"mode":"fixed","multiplier":null}}"#,
        r#"{"frameGeneration":{"mode":"fixed","multiplier":1.5}}"#,
        r#"{"frameGeneration":false}"#,
        r#"{"overlay":"invalid"}"#,
    ] {
        assert!(transfusion::read(text.as_bytes()).is_err(), "{text}");
    }
    assert_eq!(transfusion::read(b"{}").unwrap()["tf_mode"], "game");
    assert_eq!(
        transfusion::read(br#"{"mode":"fixed"}"#).unwrap()["tf_mode"],
        "4"
    );
}

#[test]
fn dynamic_and_fixed_switches_preserve_inactive_choices_and_unknown_jsonc() -> Result<()> {
    let source = b"\xef\xbb\xbf{\r\n /* keep comment */ \"configVersion\":3,\"frameGeneration\":{\"mode\":\"fixed\",\"multiplier\":3,\"dynamicTargetFrameRate\":237,\"dynamicExperimental56\":true},\"vendor\":{\"path\":\"https://x/a/*b*/\",\"array\":[1,{\"quote\":\"a\\\"//b\"}]}}";
    let dynamic = presets::configure(
        source,
        transfusion::PROFILE,
        &BTreeMap::from([("tf_mode".into(), "dynamic".into())]),
    )?;
    assert!(dynamic.starts_with(&[239, 187, 191]));
    let text = std::str::from_utf8(&dynamic[3..])?;
    assert!(text.contains("/* keep comment */"));
    assert!(text.contains("\"multiplier\":3"));
    assert!(text.contains(
        "\"vendor\":{\"path\":\"https://x/a/*b*/\",\"array\":[1,{\"quote\":\"a\\\"//b\"}]}"
    ));
    let mut values = presets::read_values(&dynamic, transfusion::PROFILE)?;
    assert_eq!(values["tf_target"], "237");
    assert_eq!(values["tf_dynamic56"], "1");
    assert!(presets::parameter_enabled(
        transfusion::PROFILE,
        "tf_target",
        &values
    ));
    values.insert("tf_mode".into(), "2".into());
    assert!(!presets::parameter_enabled(
        transfusion::PROFILE,
        "tf_target",
        &values
    ));
    assert!(!presets::parameter_enabled(
        transfusion::PROFILE,
        "tf_dynamic56",
        &values
    ));
    let fixed = presets::configure(&dynamic, transfusion::PROFILE, &values)?;
    let fixed_values = transfusion::read(&fixed)?;
    assert_eq!(fixed_values["tf_mode"], "2");
    assert_eq!(fixed_values["tf_target"], "237");
    assert_eq!(fixed_values["tf_dynamic56"], "1");
    Ok(())
}

#[test]
fn exact_proxy_names_and_config_protocols_cannot_be_mixed() -> Result<()> {
    for name in transfusion::PROXIES {
        assert_eq!(
            core::deployment_names(transfusion::BACKEND, &[name.into()])?,
            vec![name.to_owned(), transfusion::CONFIG.into()]
        );
    }
    for names in [
        vec!["version.dll".into(), "dxgi.dll".into()],
        vec!["d3d12.dll".into()],
        vec!["dbghelp.dll".into()],
    ] {
        assert!(core::deployment_names(transfusion::BACKEND, &names).is_err());
    }
    assert!(
        presets::validate(
            transfusion::PROFILE,
            &BTreeMap::from([("rtx_mode".into(), "follow".into())])
        )
        .is_err()
    );
    let catalog = cloud::bundled();
    let packages: Vec<_> = catalog
        .packages
        .iter()
        .filter(|p| p.scheme_id == "dlssg-transfusion-1.4.5.3")
        .collect();
    assert_eq!(packages.len(), 4);
    for p in packages {
        assert_eq!(p.backends, [transfusion::BACKEND]);
        assert!(p.files.iter().any(|f| f.name == p.proxy));
        assert!(p.files.iter().any(|f| f.name == transfusion::CONFIG));
        assert!(!p.files.iter().any(|f| f.name == core::INI));
    }
    Ok(())
}

fn interrupted_fixture() -> Result<(tempfile::TempDir, std::path::PathBuf, Vec<u8>)> {
    let temp = tempfile::tempdir()?;
    let exe = temp.path().join("RTXFG-Transfusion-Stage-Recovery.exe");
    let mut pe = vec![0; 512];
    pe[..2].copy_from_slice(b"MZ");
    pe[60..64].copy_from_slice(&128u32.to_le_bytes());
    pe[128..132].copy_from_slice(b"PE\0\0");
    pe[132..134].copy_from_slice(&0x8664u16.to_le_bytes());
    pe[148..150].copy_from_slice(&240u16.to_le_bytes());
    pe[150..152].copy_from_slice(&2u16.to_le_bytes());
    pe[152..154].copy_from_slice(&0x20bu16.to_le_bytes());
    std::fs::write(&exe, &pe)?;
    pe[150..152].copy_from_slice(&0x2000u16.to_le_bytes());
    core::deploy_prepared(
        &exe,
        transfusion::BACKEND,
        &["version.dll".into()],
        None,
        BTreeMap::from([
            ("version.dll".into(), pe.clone()),
            (
                transfusion::CONFIG.into(),
                br#"{"configVersion":3,"frameGeneration":{"mode":"game","multiplier":4}}"#.to_vec(),
            ),
        ]),
        Some("1.4.5"),
    )?;
    Ok((temp, exe, pe))
}

#[test]
fn unverified_cloud_stage_preserves_recovery_but_does_not_block_owned_patch_removal() -> Result<()>
{
    let (temp, exe, dll) = interrupted_fixture()?;
    let root = temp.path().join(core::OWN);
    let stage = root.join("version.dll.stage");
    std::fs::write(&stage, &dll[..100])?;
    std::fs::write(temp.path().join("other-plugin.dll"), b"unknown mod")?;
    std::fs::write(temp.path().join("DLSSG-Transfusion.log"), b"owned log")?;
    let result = rtx_fg_manager::cleanup::clean(&exe)?;
    assert!(result.contains("临时文件待核验"));
    assert!(!temp.path().join("version.dll").exists());
    assert!(!temp.path().join(transfusion::CONFIG).exists());
    assert!(!temp.path().join("DLSSG-Transfusion.log").exists());
    assert_eq!(std::fs::read(&stage)?, &dll[..100]);
    assert_eq!(
        std::fs::read(temp.path().join("other-plugin.dll"))?,
        b"unknown mod"
    );
    assert!(core::record(temp.path())?.unwrap().cache_pending);
    assert!(rtx_fg_manager::cleanup::clean(&exe)?.contains("临时文件待核验"));
    // A user-confirmed removal of the unknown fragment allows the retained
    // recovery record to be finalized on the next uninstall attempt.
    std::fs::remove_file(stage)?;
    rtx_fg_manager::cleanup::clean(&exe)?;
    assert!(core::record(temp.path())?.is_none());
    assert!(exe.is_file());
    Ok(())
}

#[test]
fn completed_stage_matching_record_is_removed_without_embedded_cloud_source() -> Result<()> {
    let (temp, exe, dll) = interrupted_fixture()?;
    let stage = temp.path().join(core::OWN).join("version.dll.stage");
    std::fs::write(&stage, dll)?;
    let result = rtx_fg_manager::cleanup::clean(&exe)?;
    assert!(!result.contains("待核验"));
    assert!(!stage.exists());
    assert!(!temp.path().join("version.dll").exists());
    assert!(!temp.path().join(transfusion::CONFIG).exists());
    assert!(core::record(temp.path())?.is_none());
    Ok(())
}
