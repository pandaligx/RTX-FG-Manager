//! Opt-in network integration checks; never installs or executes the downloaded file.
use anyhow::Result;
use rtx_fg_manager::{core, transfer};
use std::{fs, sync::atomic::AtomicBool, time::Instant};

#[test]
#[ignore = "requires network; uses a disposable directory and real aria2"]
fn live_domestic_metadata_uses_aria2_and_does_not_touch_fallback() -> Result<()> {
    let mut seen = Vec::new();
    let bytes = transfer::fetch_metadata(
        vec![
            transfer::Source {
                name: "gitee".into(),
                url: "https://gitee.com/api/v5/repos/pandaligx/RTX-FG-Manager/releases/latest"
                    .into(),
            },
            transfer::Source {
                name: "unused-github".into(),
                url: "https://api.github.com/repos/pandaligx/RTX-FG-Manager/releases/latest".into(),
            },
        ],
        transfer::UrlPolicy::Official,
        &AtomicBool::new(false),
        |p| seen.push(p),
    )?;
    let value: serde_json::Value = serde_json::from_slice(&bytes)?;
    assert!(
        value["tag_name"]
            .as_str()
            .is_some_and(|s| s.starts_with('v'))
    );
    assert!(
        seen.iter().all(|p| p.source == "gitee"),
        "domestic metadata did not succeed on its own"
    );
    Ok(())
}

#[test]
#[ignore = "requires published payloads mirror; downloads a 10.6 MB DLL ZIP but never extracts or executes it"]
fn live_gitee_file_then_verified_offline_cache() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let request = transfer::Request {
        sources: vec![transfer::Source { name:"gitee".into(), url:"https://gitee.com/pandaligx/RTX-FG-Manager-payloads/releases/download/payloads/rtxfg-0.3.5-mfg-v421-dbghelp.zip".into() }],
        bytes:Some(10_559_657),sha256:Some("bc5b42a20d11bcac0e3bfc0eef4276db49d1416b66317d646a92e8a2f881e2dd".into()),
        limit:12_000_000,metadata:false,policy:transfer::UrlPolicy::Official,
    };
    let mut events = Vec::new();
    let start = Instant::now();
    let file = transfer::download(
        &request,
        &temp.path().join("verified-dll-package.zip"),
        &AtomicBool::new(false),
        |p| events.push(p),
    )?;
    assert_eq!(fs::metadata(&file)?.len(), 10_559_657);
    assert_eq!(core::digest(&file)?, request.sha256.clone().unwrap());
    if let Some(path) = std::env::var_os("RTXFG_NETWORK_REPORT") {
        core::atomic_json(
            std::path::Path::new(&path),
            &serde_json::json!({"seconds":start.elapsed().as_secs_f64(),"events":events,"no_vpn_claim":false}),
        )?;
    }
    let mut offline = request;
    offline.sources[0].url =
        "https://gitee.com/pandaligx/RTX-FG-Manager/nonexistent-offline-cache-test".into();
    assert_eq!(
        transfer::download(&offline, &file, &AtomicBool::new(false), |_| {})?,
        file
    );
    Ok(())
}
