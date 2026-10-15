//! Local DXGI metadata only. No hardware information is sent while browsing.
use crate::{community::Hardware, core, win};
use anyhow::Result;
use std::collections::BTreeMap;
use windows::{
    Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIDevice, IDXGIFactory1},
    core::Interface,
};

pub fn detect() -> Result<Vec<Hardware>> {
    let mut rows = Vec::new();
    // SAFETY: Query-only system DXGI COM interfaces; all returned descriptors
    // are copied while their adapters/outputs remain alive. No game DLL loads.
    unsafe {
        let factory: IDXGIFactory1 = CreateDXGIFactory1()?;
        for index in 0..32 {
            let Ok(adapter) = factory.EnumAdapters1(index) else {
                break;
            };
            let desc = adapter.GetDesc1()?;
            if desc.Flags & 2 != 0 {
                continue;
            }
            let model = win::from_wide(&desc.Description);
            let mut h = Hardware {
                gpu_model: Some(model),
                ..Default::default()
            };
            if let Ok(version) = adapter.CheckInterfaceSupport(&IDXGIDevice::IID) {
                let n = version as u64;
                h.driver = Some(format!(
                    "{}.{}.{}.{}",
                    n >> 48,
                    (n >> 32) & 65535,
                    (n >> 16) & 65535,
                    n & 65535
                ));
            }
            rows.push(h);
        }
    }
    Ok(rows)
}

/// File version resources, not a claim that the running game loaded this copy.
pub fn runtime_versions(target: &core::DeploymentTarget) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for (key, candidates) in [
        (
            "fg",
            vec!["dlssg_sm86/runtime/nvngx_dlssg.dll", "nvngx_dlssg.dll"],
        ),
        ("sr", vec!["dlssg_sm86/sr/nvngx_dlss.dll", "nvngx_dlss.dll"]),
        ("rr", vec!["nvngx_dlssd.dll"]),
        ("streamline", vec!["sl.common.dll"]),
    ] {
        for name in candidates {
            if let Ok(path) = core::no_links(&target.directory.join(name))
                && path.is_file()
                && let Ok((version, _)) = win::file_version(&path)
            {
                out.insert(key.into(), version);
                break;
            }
        }
    }
    out
}
