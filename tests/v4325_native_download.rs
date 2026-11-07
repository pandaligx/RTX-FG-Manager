//! Explicit single-package network comparison; never extracts or installs game files.
use anyhow::{Context, Result, ensure};
use rtx_fg_manager::{core, transfer};
use std::{
    fs,
    io::Read,
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};

#[test]
#[ignore = "explicit network test: downloads one published package twice to compare concurrency"]
fn one_gitcode_package_parallel_and_single_stream() -> Result<()> {
    let catalog = rtx_fg_manager::cloud::bundled();
    let package = catalog
        .packages
        .iter()
        .find(|p| p.archive == "dlssg-0.4.1-complete-9f397efeb502.zip")
        .context("test package")?;
    let url = package.urls.get("domestic").context("url")?.to_owned();
    let bytes = package.bytes;
    let hash = package.sha256.clone();
    let output =
        std::env::var_os("RTXFG_NATIVE_REPORT").context("RTXFG_NATIVE_REPORT is required")?;
    let temp = tempfile::tempdir()?;
    let request = transfer::Request {
        sources: vec![transfer::Source {
            name: "gitcode".into(),
            url: url.clone(),
        }],
        bytes: Some(bytes),
        sha256: Some(hash.clone()),
        limit: bytes,
        metadata: false,
        policy: transfer::UrlPolicy::Official,
    };
    let mut events = Vec::new();
    let started = Instant::now();
    transfer::download(
        &request,
        &temp.path().join("parallel.zip"),
        &AtomicBool::new(false),
        |p| {
            events.push(
                serde_json::json!({"seconds": started.elapsed().as_secs_f64(), "progress": p}),
            );
        },
    )?;
    let parallel_seconds = started.elapsed().as_secs_f64();
    let started = Instant::now();
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(180))
        .redirect(reqwest::redirect::Policy::custom(|a| {
            if a.previous().len() >= 8
                || transfer::validate_url(a.url().as_str(), transfer::UrlPolicy::Official).is_err()
            {
                a.error("Unsafe redirect")
            } else {
                a.follow()
            }
        }))
        .build()?;
    let response = client
        .get(transfer::validate_url(&url, transfer::UrlPolicy::Official)?)
        .header(reqwest::header::ACCEPT_ENCODING, "identity")
        .send()?
        .error_for_status()?;
    let mut file = fs::File::create(temp.path().join("single.zip"))?;
    let copied = std::io::copy(&mut response.take(bytes + 1), &mut file)?;
    ensure!(
        copied == bytes && core::digest(&temp.path().join("single.zip"))? == hash,
        "single stream bytes differ"
    );
    let single_seconds = started.elapsed().as_secs_f64();
    let report = serde_json::json!({
        "package":package.archive,"bytes":bytes,"sha256":hash,
        "parallel_seconds":parallel_seconds,"single_seconds":single_seconds,
        "parallel_mib_s":bytes as f64 / parallel_seconds / 1048576.,
        "single_mib_s":bytes as f64 / single_seconds / 1048576.,
        "events":events,"installed":false,"vpn_state_not_changed":true,
        "limitations":"One sequential comparison on this connection; CDN cache/order can affect results. Not an aria2 benchmark."
    });
    core::atomic_json(std::path::Path::new(&output), &report)?;
    eprintln!(
        "parallel {parallel_seconds:.2}s; single {single_seconds:.2}s; {bytes} bytes verified"
    );
    Ok(())
}
