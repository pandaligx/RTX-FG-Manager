//! Filesystem/protocol regression fixtures; synthetic DLLs are never executed.
use anyhow::Result;
use rtx_fg_manager::{cloud, core, deployment, presets, rtxmfg, scanner};
use serde_json::json;
use std::{collections::BTreeMap, fs, path::Path};
fn pe(dll: bool) -> Vec<u8> {
    let mut b = vec![0; 512];
    b[..2].copy_from_slice(b"MZ");
    b[60..64].copy_from_slice(&128u32.to_le_bytes());
    b[128..132].copy_from_slice(b"PE\0\0");
    b[132..134].copy_from_slice(&0x8664u16.to_le_bytes());
    b[148..150].copy_from_slice(&240u16.to_le_bytes());
    b[150..152].copy_from_slice(&(if dll { 0x2000u16 } else { 2 }).to_le_bytes());
    b[152..154].copy_from_slice(&0x20bu16.to_le_bytes());
    b
}
fn install(dir: &Path, scheme: &str, backend: &str, proxy: &str) -> Result<()> {
    fs::create_dir_all(dir)?;
    let exe = dir.join("Game.exe");
    fs::write(&exe, pe(false))?;
    let c = cloud::bundled();
    let context = presets::Context::new(scheme, &c.scheme_policies[scheme], &exe);
    let config = if backend == rtxmfg::BACKEND {
        b"{}".to_vec()
    } else {
        b"[FrameGeneration]\nMaxGeneratedFrames=3\n".to_vec()
    };
    let values = presets::defaults(&context.profile, &BTreeMap::new());
    core::deploy_prepared_context(
        &exe,
        backend,
        &[proxy.into()],
        None,
        BTreeMap::from([
            (proxy.into(), pe(true)),
            (
                core::config_name(backend).into(),
                context.configure(&config, &values)?,
            ),
        ]),
        Some("1.3.3"),
        Some(&context),
    )?;
    Ok(())
}
#[test]
fn rtx40_json_protocol_is_separate_and_preserves_menu_settings() -> Result<()> {
    for mode in ["follow", "1", "2", "3", "4", "5", "6", "dynamic"] {
        let values = BTreeMap::from([
            ("rtx_mode".into(), mode.into()),
            ("rtx_target".into(), "144".into()),
        ]);
        let after = presets::configure(
            br#"{"menuHotkey":119,"custom":{"keep":true}}"#,
            rtxmfg::PROFILE,
            &values,
        )?;
        let j: serde_json::Value = serde_json::from_slice(&after)?;
        assert_eq!(j["menuHotkey"], 119);
        assert_eq!(j["custom"]["keep"], true);
        assert_eq!(j["followGame"], mode == "follow");
        assert_eq!(
            presets::read_values(&after, rtxmfg::PROFILE)?["rtx_mode"],
            mode
        );
        assert!(!String::from_utf8(after)?.contains("FrameGeneration"));
    }
    assert!(
        presets::validate(
            rtxmfg::PROFILE,
            &BTreeMap::from([("max_generated_frames".into(), "3".into())])
        )
        .is_err()
    );
    assert!(presets::configure(b"[]", rtxmfg::PROFILE, &BTreeMap::new()).is_err());
    let custom =
        br#"{"mode":"dynamic","multiplier":2,"dynamicTargetFrameRate":200,"menuHotkey":119}"#;
    let values = presets::read_values(custom, rtxmfg::PROFILE)?;
    assert_eq!(values["rtx_target"], "200");
    let merged: serde_json::Value =
        serde_json::from_slice(&presets::configure(custom, rtxmfg::PROFILE, &values)?)?;
    assert_eq!(merged["dynamicTargetFrameRate"], 200);
    assert_eq!(merged["menuHotkey"], 119);
    let c = cloud::bundled();
    c.validate()?;
    assert_eq!(c.proxies("rtx40mfg-1.3.3-hf2").len(), 19);
    assert_eq!(
        c.scheme_policies["rtx40mfg-1.3.3-hf2"].gpu_paths,
        vec!["SM89"]
    );
    assert!(core::deployment_names("upstream_sm86", &["d3d11.dll".into()]).is_err());
    assert!(
        core::deployment_names(rtxmfg::BACKEND, &["version.dll".into(), "dxgi.dll".into()])
            .is_err()
    );
    Ok(())
}
#[test]
fn grouped_status_retains_actual_scheme_proxy_and_partial_or_mixed_state() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let a = temp.path().join("Win64r");
    let b = temp.path().join("Win64rh");
    install(&a, "upstream-0.3.5-310-9", "upstream_sm86", "d3d12.dll")?;
    install(&b, "upstream-0.3.5-310-9", "upstream_sm86", "d3d12.dll")?;
    let game = scanner::Game {
        exe: a.join("Game.exe").display().to_string(),
        targets: vec![
            a.join("Game.exe").display().to_string(),
            b.join("Game.exe").display().to_string(),
        ],
        ..Default::default()
    };
    let c = cloud::bundled();
    let d = deployment::inspect(&game, &c);
    assert_eq!((d.installed, d.total), (2, 2));
    assert!(d.can_apply("upstream-0.3.5-310-9"));
    assert!(
        d.details
            .iter()
            .all(|d| d.contains("Github-sdli1995") && d.contains("d3d12.dll") && !d.contains("R2"))
    );
    let mut r = core::record(&b)?.unwrap();
    r.scheme_id = Some("rtxfg-0.3.5-dx12-vulkan".into());
    core::atomic_json(&b.join(core::OWN).join(core::MARKER), &r)?;
    let d = deployment::inspect(&game, &c);
    assert!(d.common.is_none());
    assert_eq!(d.schemes.len(), 2);
    fs::remove_file(b.join("d3d12.dll"))?;
    assert_eq!(deployment::inspect(&game, &c).installed, 1);
    Ok(())
}
#[test]
fn rtx40_install_apply_edit_and_cleanup_leave_other_mod_ini_untouched() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let dir = temp.path();
    fs::write(dir.join(core::INI), b"other mod settings")?;
    install(dir, "rtx40mfg-1.3.3-hf2", rtxmfg::BACKEND, "xinput1_4.dll")?;
    let exe = dir.join("Game.exe");
    let mut config: serde_json::Value =
        serde_json::from_slice(&fs::read(dir.join(rtxmfg::CONFIG))?)?;
    config["menuHotkey"] = json!(121);
    fs::write(dir.join(rtxmfg::CONFIG), serde_json::to_vec(&config)?)?;
    assert!(core::status(&exe).starts_with("已部署"));
    let c = cloud::bundled();
    let ctx = presets::Context::new(
        "rtx40mfg-1.3.3-hf2",
        &c.scheme_policies["rtx40mfg-1.3.3-hf2"],
        &exe,
    );
    core::apply_parameters(
        &exe,
        &ctx,
        &BTreeMap::from([("rtx_mode".into(), "4".into())]),
    )?;
    let after: serde_json::Value = serde_json::from_slice(&fs::read(dir.join(rtxmfg::CONFIG))?)?;
    assert_eq!(after["menuHotkey"], 121);
    assert_eq!(after["multiplier"], 4);
    fs::write(dir.join("original-game.dll"), b"original")?;
    rtx_fg_manager::cleanup::clean(&exe)?;
    assert!(!dir.join("xinput1_4.dll").exists());
    assert!(!dir.join(rtxmfg::CONFIG).exists());
    assert_eq!(fs::read(dir.join(core::INI))?, b"other mod settings");
    assert_eq!(fs::read(dir.join("original-game.dll"))?, b"original");
    rtx_fg_manager::cleanup::clean(&exe)?;
    Ok(())
}
