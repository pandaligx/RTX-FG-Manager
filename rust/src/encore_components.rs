//! Explicit per-install component selection. Native Streamline is opt-in.
use crate::{cloud, core, encore, presets};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Components {
    pub patch: bool,
    pub sr: bool,
    pub rr: bool,
    pub fg: bool,
    pub nr: bool,
    pub intel: bool,
    pub streamline: bool,
    pub added_fg: bool,
}
impl Default for Components {
    fn default() -> Self {
        Self {
            patch: true,
            sr: true,
            rr: true,
            fg: true,
            nr: true,
            intel: true,
            streamline: false,
            added_fg: true,
        }
    }
}
impl Components {
    pub fn selected(self, kind: &str) -> bool {
        match kind {
            "patch" => self.patch,
            "sr" => self.sr,
            "rr" => self.rr,
            "fg" => self.fg,
            "nr" => self.nr,
            "intel" => self.intel,
            "streamline" => self.streamline,
            "encorefg" => self.added_fg,
            _ => false,
        }
    }
    pub fn set(&mut self, kind: &str, checked: bool) {
        match kind {
            "patch" => self.patch = checked,
            "sr" => self.sr = checked,
            "rr" => self.rr = checked,
            "fg" => self.fg = checked,
            "nr" => self.nr = checked,
            "intel" => self.intel = checked,
            "streamline" => self.streamline = checked,
            "encorefg" => self.added_fg = checked,
            _ => {}
        }
    }
}

/// Reuse the core deployment transaction for additions and upgrades. Unchecked
/// owned Intel files retain their exact bytes and removal ownership. Unowned
/// files are never adopted or overwritten by this selection.
pub fn deploy(
    target: &core::DeploymentTarget,
    payload: &cloud::Prepared,
    proxies: &[String],
    mut files: BTreeMap<String, Vec<u8>>,
    context: &presets::Context,
    overrides: &presets::Values,
    selection: Components,
) -> Result<String> {
    ensure!(
        payload.backend == encore::BACKEND,
        "组件选择只适用于 RTX Encore"
    );
    let previous = core::record(&target.directory)?;
    let mut version = payload.version.clone();
    let mut upstream = payload.upstream_version.clone();
    let mut entries = proxies.to_vec();
    let empty = presets::Values::new();
    let mut applied_overrides = overrides;
    if !selection.patch {
        let old = previous.as_ref().context("首次安装需要勾选 RTX Encore")?;
        ensure!(
            old.backend == encore::BACKEND && !old.cache_pending,
            "请先安装 RTX Encore"
        );
        if !selection.intel {
            return Ok("保留现有补丁与设置".into());
        }
        if encore::INTEL_FILES.iter().all(|name| {
            files
                .get(*name)
                .is_some_and(|bytes| old.hashes.get(*name) == Some(&core::hash(bytes)))
                && owned_bytes(target, old, name).is_ok()
        }) {
            return Ok("配套 Intel 组件已安装，保留现有补丁与设置".into());
        }
        version = old.payload_version.clone().context("已安装补丁缺少版本")?;
        upstream = old.upstream_version.clone();
        entries = old.selected();
        files.retain(|name, _| encore::INTEL_FILES.contains(&name.as_str()));
        for name in old
            .hashes
            .keys()
            .filter(|n| !encore::INTEL_FILES.contains(&n.as_str()))
        {
            files.insert(name.clone(), owned_bytes(target, old, name)?);
        }
        applied_overrides = &empty;
    }
    if !selection.intel {
        for name in encore::INTEL_FILES {
            files.remove(name);
            if let Some(old) = &previous
                && old.backend == encore::BACKEND
                && old.hashes.contains_key(name)
            {
                files.insert(name.into(), owned_bytes(target, old, name)?);
            }
        }
    }
    core::deploy_prepared_context_at_with_overrides(
        target,
        encore::BACKEND,
        &entries,
        None,
        files,
        Some(&version),
        Some(context),
        Some(applied_overrides),
        upstream.as_deref(),
    )
}
fn owned_bytes(target: &core::DeploymentTarget, old: &core::Record, name: &str) -> Result<Vec<u8>> {
    let path = core::no_links(&target.directory.join(name))?;
    ensure!(
        path.metadata()?.len() <= 128 * 1024 * 1024,
        "补丁文件大小异常"
    );
    let bytes = fs::read(path)?;
    ensure!(
        name == encore::CONFIG || old.hashes.get(name) == Some(&core::hash(&bytes)),
        "已安装文件被修改，请先检查：{name}"
    );
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn default_and_old_saved_choices_preserve_native_streamline() {
        assert!(!Components::default().streamline);
        let saved: Components = serde_json::from_str(r#"{"sr":false,"intel":false}"#).unwrap();
        assert!(!saved.sr && !saved.intel && !saved.streamline);
        assert!(saved.patch && saved.nr);
    }

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
    fn payload(identity: u8, version: &str) -> cloud::Prepared {
        let mut files = BTreeMap::from([
            ("version.dll".into(), pe(true, identity)),
            (encore::CONFIG.into(), br#"{"configVersion":4}"#.to_vec()),
            (encore::NOTICES.into(), b"notices".to_vec()),
        ]);
        for name in encore::INTEL_FILES {
            files.insert(
                name.into(),
                if name.ends_with(".dll") {
                    pe(true, identity)
                } else {
                    b"license".to_vec()
                },
            );
        }
        cloud::Prepared {
            backend: encore::BACKEND.into(),
            version: version.into(),
            upstream_version: Some(version.into()),
            files,
            scheme_id: encore::SCHEME.into(),
            policy: cloud::bundled().scheme_policies[encore::SCHEME].clone(),
        }
    }
    #[test]
    fn optional_components_keep_proxy_config_and_cleanup_ownership() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let exe = tmp.path().join("ComponentTest.exe");
        fs::write(&exe, pe(false, 0))?;
        let target = core::DeploymentTarget::for_game(&exe);
        let p = payload(1, "2.1.0");
        let context = presets::Context::new(encore::SCHEME, &p.policy, &exe);
        let selection = Components {
            intel: false,
            ..Default::default()
        };
        deploy(
            &target,
            &p,
            &["version.dll".into()],
            p.files.clone(),
            &context,
            &presets::Values::new(),
            selection,
        )?;
        assert!(!target.directory.join(encore::INTEL_FILES[0]).exists());
        let custom=br#"{"configVersion":4,"vendor":{"keep":true},"frameGeneration":{"mode":"fixed","multiplier":3}}"#;
        fs::write(target.directory.join(encore::CONFIG), custom)?;
        let p2 = payload(2, "2.2.0");
        deploy(
            &target,
            &p2,
            &["dxgi.dll".into()],
            p2.files.clone(),
            &context,
            &presets::Values::new(),
            Components {
                patch: false,
                ..Default::default()
            },
        )?;
        let r = core::record(&target.directory)?.unwrap();
        assert_eq!(r.upstream_version.as_deref(), Some("2.1.0"));
        assert_eq!(fs::read(target.directory.join("version.dll"))?, pe(true, 1));
        assert!(!target.directory.join("dxgi.dll").exists());
        assert_eq!(fs::read(target.directory.join(encore::CONFIG))?, custom);
        assert_eq!(
            fs::read(target.directory.join(encore::INTEL_FILES[0]))?,
            pe(true, 2)
        );
        let values = encore::read_for_version(
            &fs::read(target.directory.join(encore::CONFIG))?,
            Some("2.1.0"),
        )?;
        assert_eq!(values["tf_mode"], "3");
        let p3 = payload(3, "2.3.0");
        deploy(
            &target,
            &p3,
            &["version.dll".into()],
            p3.files.clone(),
            &context,
            &presets::Values::new(),
            selection,
        )?;
        assert_eq!(fs::read(target.directory.join("version.dll"))?, pe(true, 3));
        assert_eq!(
            fs::read(target.directory.join(encore::INTEL_FILES[0]))?,
            pe(true, 2)
        );
        assert!(crate::cleanup::clean_outcome_at(&target)?.complete);
        assert!(!target.directory.join(encore::INTEL_FILES[0]).exists());
        assert!(exe.exists());
        Ok(())
    }
}
