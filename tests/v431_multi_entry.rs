//! Inert PE/filesystem tests only: no patch DLL is executed.
use anyhow::Result;
use rtx_fg_manager::{
    cleanup, cloud, core, encore, mfg_plugin, presets, rtxmfg, rtxmfg_upgrade, upstream040,
};
use std::{collections::BTreeMap, fs, os::windows::fs::OpenOptionsExt};

fn pe(dll: bool, identity: u8) -> Vec<u8> {
    let mut b = vec![0; 512];
    b[..2].copy_from_slice(b"MZ");
    b[60..64].copy_from_slice(&128u32.to_le_bytes());
    b[128..132].copy_from_slice(b"PE\0\0");
    b[132..134].copy_from_slice(&0x8664u16.to_le_bytes());
    b[148..150].copy_from_slice(&240u16.to_le_bytes());
    b[150..152].copy_from_slice(&(if dll { 0x2000u16 } else { 2 }).to_le_bytes());
    b[152..154].copy_from_slice(&0x20bu16.to_le_bytes());
    b[511] = identity;
    b
}
fn context(backend: &str) -> presets::Context {
    let (scheme, profile) = match backend {
        encore::BACKEND => (encore::SCHEME, encore::PROFILE),
        rtxmfg::BACKEND => (rtxmfg_upgrade::SCHEME, rtxmfg::PROFILE),
        _ => (upstream040::SCHEME, upstream040::PROFILE),
    };
    presets::Context {
        scheme: scheme.into(),
        profile: profile.into(),
        delta: false,
        delta_capable: false,
    }
}
fn files(backend: &str, entries: &[String], secondary: u8) -> Result<BTreeMap<String, Vec<u8>>> {
    Ok(core::deployment_names(backend, entries)?
        .into_iter()
        .map(|n| {
            let bytes = if n.ends_with(".dll") {
                pe(true, if n == "version.dll" { 1 } else { secondary })
            } else if n == core::INI {
                b"[DLSSG]\nOptimized=2\n".to_vec()
            } else if n == encore::CONFIG {
                encore::DEFAULT_CONFIG.to_vec()
            } else if n == rtxmfg::CONFIG {
                b"{\"followGame\":true,\"mode\":\"follow\",\"multiplier\":2}".to_vec()
            } else {
                b"Fixture license notice\n".to_vec()
            };
            (n, bytes)
        })
        .collect())
}
fn install(
    target: &core::DeploymentTarget,
    backend: &str,
    entries: &[String],
    version: &str,
    secondary: u8,
) -> Result<String> {
    core::deploy_prepared_context_at_with_overrides(
        target,
        backend,
        entries,
        None,
        files(backend, entries, secondary)?,
        Some(version),
        Some(&context(backend)),
        None,
        Some(version),
    )
}
#[test]
fn all_advertised_dlls_install_and_clean_as_one_owned_transaction() -> Result<()> {
    for backend in [upstream040::BACKEND, rtxmfg::BACKEND, encore::BACKEND] {
        let temp = tempfile::tempdir()?;
        let exe = temp.path().join("Multi-Test.exe");
        fs::write(&exe, pe(false, 0))?;
        let target = core::DeploymentTarget::for_game(&exe);
        let entries: Vec<String> = if backend == upstream040::BACKEND {
            upstream040::PROXIES.as_slice()
        } else {
            encore::PROXIES.as_slice()
        }
        .iter()
        .map(|n| (*n).into())
        .collect();
        for name in ["binkw64Hooked.dll", "bink2w64Hooked.dll", "unrelated.dll"] {
            fs::write(temp.path().join(name), pe(true, 99))?;
        }
        install(&target, backend, &entries, "1.0.0", 2)?;
        assert_eq!(
            core::record(temp.path())?.unwrap().selected(),
            core::normalize_proxies(&entries)?
        );
        for name in core::deployment_names(backend, &entries)? {
            assert!(temp.path().join(&name).is_file(), "{backend}: {name}");
        }
        cleanup::clean_outcome_at(&target)?;
        for name in core::deployment_names(backend, &entries)? {
            assert!(!temp.path().join(&name).exists(), "{backend}: {name}");
        }
        for name in ["binkw64Hooked.dll", "bink2w64Hooked.dll", "unrelated.dll"] {
            assert_eq!(fs::read(temp.path().join(name))?, pe(true, 99));
        }
    }
    Ok(())
}
#[test]
fn secondary_dll_upgrade_preserves_config_and_rolls_back_when_locked() -> Result<()> {
    for backend in [upstream040::BACKEND, rtxmfg::BACKEND, encore::BACKEND] {
        let temp = tempfile::tempdir()?;
        let exe = temp.path().join("Multi-Upgrade.exe");
        fs::write(&exe, pe(false, 0))?;
        let target = core::DeploymentTarget::for_game(&exe);
        let entries = vec!["version.dll".into(), "dxgi.dll".into()];
        install(&target, backend, &entries, "1.0.0", 2)?;
        let config = core::config_name(backend);
        let original = fs::read(temp.path().join(config))?;
        let marker = fs::read(temp.path().join(core::OWN).join("install.json"))?;
        {
            let _guard = fs::OpenOptions::new()
                .read(true)
                .share_mode(1)
                .open(temp.path().join("dxgi.dll"))?;
            assert!(install(&target, backend, &entries, "1.0.1", 3).is_err());
            assert_eq!(fs::read(temp.path().join("version.dll"))?, pe(true, 1));
            assert_eq!(fs::read(temp.path().join("dxgi.dll"))?, pe(true, 2));
            assert_eq!(fs::read(temp.path().join(config))?, original);
            assert_eq!(
                fs::read(temp.path().join(core::OWN).join("install.json"))?,
                marker
            );
        }
        install(&target, backend, &entries, "1.0.1", 3)?;
        assert_eq!(fs::read(temp.path().join("dxgi.dll"))?, pe(true, 3));
        assert_eq!(fs::read(temp.path().join(config))?, original);
        assert_eq!(
            core::record(temp.path())?
                .unwrap()
                .payload_version
                .as_deref(),
            Some("1.0.1")
        );
        cleanup::clean_outcome_at(&target)?;
    }
    Ok(())
}
#[test]
fn plugin_mode_stays_exclusive_and_unadvertised_dlls_are_rejected() {
    for (backend, plugin) in [
        (encore::BACKEND, "rtx-encore.asi"),
        (rtxmfg::BACKEND, mfg_plugin::ENTRY),
    ] {
        assert!(core::deployment_names(backend, &[plugin.into(), "version.dll".into()]).is_err());
        assert!(core::deployment_names(backend, &[plugin.into()]).is_ok());
    }
    assert!(
        core::deployment_names(
            upstream040::BACKEND,
            &["version.dll".into(), "winhttp.dll".into()]
        )
        .is_err()
    );
}
#[test]
fn current_client_accepts_multiple_entries_without_changing_legacy_catalog() -> Result<()> {
    let c: cloud::Catalog = serde_json::from_str(include_str!("fixtures/cloud-v3-catalog.json"))?;
    c.validate()?;
    for id in [upstream040::SCHEME, encore::SCHEME, rtxmfg_upgrade::SCHEME] {
        assert_eq!(c.scheme_policies[id].max_selected_proxies, 1);
        assert_eq!(c.selection_limit(id), c.proxies(id).len());
        assert!(c.selection_limit(id) > 1);
    }
    Ok(())
}
