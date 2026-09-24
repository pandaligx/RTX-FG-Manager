use anyhow::Result;
use rtx_fg_manager::{cloud, core, transfer};
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, sync::atomic::AtomicBool};

fn request(bytes: &[u8]) -> transfer::Request {
    transfer::Request {
        sources: vec![
            transfer::Source {
                name: "gitee".into(),
                url: "https://gitee.com/pandaligx/RTX-FG-Manager/releases/download/payloads/test.zip".into(),
            },
            transfer::Source {
                name: "github".into(),
                url: "https://github.com/pandaligx/RTX-FG-Manager/releases/download/payloads/test.zip".into(),
            },
        ],
        bytes: Some(bytes.len() as u64),
        sha256: Some(core::hash(bytes)),
        limit: 1024,
        metadata: false,
        policy: transfer::UrlPolicy::Official,
    }
}

#[test]
fn transfer_keeps_https_and_official_host_boundaries() {
    for url in [
        "http://gitee.com/file",
        "https://user:password@gitee.com/file",
        "https://gitee.com:444/file",
        "https://gitee.com.evil.invalid/file",
        "https://github.com@evil.invalid/file",
        "file:///C:/Windows/test",
        "https://gitee.com/file#fragment",
        "https://gitee.com/\nfile",
    ] {
        assert!(
            transfer::validate_url(url, transfer::UrlPolicy::Official).is_err(),
            "{url}"
        );
    }
    assert!(transfer::validate_url(cloud::DOMESTIC, transfer::UrlPolicy::Official).is_ok());
    assert!(transfer::validate_url(cloud::GITHUB, transfer::UrlPolicy::Official).is_ok());
    assert!(
        transfer::validate_url(
            "https://raw.giteeusercontent.com/pandaligx/RTX-FG-Manager/raw/main/cloud/catalog.json",
            transfer::UrlPolicy::Official
        )
        .is_ok()
    );
    assert!(
        transfer::validate_url(
            "https://raw.giteeusercontent.com.evil.invalid/catalog.json",
            transfer::UrlPolicy::Official
        )
        .is_err()
    );
    assert!(
        transfer::validate_url(
            "https://foruda.gitee.com/attach_file/test",
            transfer::UrlPolicy::Official
        )
        .is_ok()
    );
    assert!(
        transfer::validate_url(
            "https://panda.ligxng.cn/d/LT/rtxfg/test.zip",
            transfer::UrlPolicy::Cloud
        )
        .is_ok()
    );
}

#[test]
fn verified_cache_returns_without_starting_downloader_or_fallback() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("cache.zip");
    let bytes = b"previously verified transfer";
    fs::write(&path, bytes)?;
    let mut events = Vec::new();
    let result = transfer::download(&request(bytes), &path, &AtomicBool::new(false), |p| {
        events.push(p)
    })?;
    assert_eq!(result, path);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].source, "cache");
    assert_eq!(events[0].phase, transfer::Phase::Complete);
    assert_eq!(events[0].completed, bytes.len() as u64);
    assert_eq!(fs::read_dir(temp.path())?.count(), 1);
    Ok(())
}

#[test]
fn cancellation_precedes_cache_and_creates_no_partial_or_process() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("absent").join("download.zip");
    let mut events = 0;
    assert!(
        transfer::download(&request(b"payload"), &path, &AtomicBool::new(true), |_| {
            events += 1
        })
        .is_err()
    );
    assert_eq!(events, 0);
    assert!(!path.parent().unwrap().exists());
    Ok(())
}

#[test]
fn cache_is_not_trusted_by_length_alone_and_limits_are_enforced() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("cache.zip");
    let expected = request(b"correct");
    fs::write(&path, b"changed")?;
    assert!(transfer::verify_file(&path, &expected).is_err());
    let mut too_large = expected.clone();
    too_large.limit = 2;
    assert!(too_large.validate().is_err());
    assert!(transfer::verify_file(&path, &too_large).is_err());
    let mut missing_hash = expected;
    missing_hash.sha256 = None;
    assert!(missing_hash.validate().is_err());
    Ok(())
}

#[test]
fn progress_and_failure_reasons_do_not_invent_completed_bytes() {
    assert_eq!(
        transfer::Progress::parse_aria_line("[#abcdef 12B/100B(12%) CN:2 DL:40B ETA:2s]", 100),
        Some((12, 40, 2))
    );
    assert!(
        transfer::Progress::parse_aria_line("[#abcdef 101/100(100%) CN:2 DL:40]", 100).is_none()
    );
    assert!(transfer::Progress::parse_aria_line("[#abcdef 12/200(6%) CN:2 DL:40]", 100).is_none());
    assert_eq!(
        transfer::Failure::from_aria_log(Some(8), ""),
        transfer::Failure::Range
    );
    assert_eq!(
        transfer::Failure::from_aria_log(Some(22), "status=416"),
        transfer::Failure::Range
    );
    assert_eq!(
        transfer::Failure::from_aria_log(Some(24), ""),
        transfer::Failure::Unauthorized
    );
    assert_eq!(
        transfer::Failure::from_aria_log(Some(32), ""),
        transfer::Failure::Integrity
    );
    assert_eq!(
        transfer::Failure::from_aria_log(Some(1), "status=429"),
        transfer::Failure::RateLimited
    );
    assert_eq!(
        transfer::Failure::from_aria_log(Some(1), "certificate verification failed"),
        transfer::Failure::Certificate
    );
    let progress = transfer::Progress {
        completed: 100,
        total: 100,
        phase: transfer::Phase::Verifying,
        ..Default::default()
    };
    assert!(progress.percent() < 100.);
}

fn compact_catalog() -> Result<(Value, Vec<u8>)> {
    let c = cloud::bundled();
    let package = c
        .packages
        .iter()
        .find(|p| p.scheme_id == c.default_scheme)
        .unwrap();
    let policy = &c.scheme_policies[&package.scheme_id];
    let index = serde_json::to_vec(&json!({"schema":1,"packages":[package]}))?;
    let value = json!({
        "schema":2,"revision":"offline-test","default_scheme":package.scheme_id,
        "sources":c.sources,
        "index":{"url":"https://gitee.com/index.json","fallback_url":"https://github.com/index.json",
            "bytes":index.len(),"sha256":core::hash(&index)},
        "schemes":[{"id":package.scheme_id,"name":package.label,"profile":policy.parameter_profile,
            "defaults":policy.defaults,"archives":[package.archive],"capabilities":policy.capabilities}]
    });
    Ok((value, index))
}

#[test]
fn unknown_parameter_protocol_only_skips_that_scheme() -> Result<()> {
    let (mut value, index) = compact_catalog()?;
    let known = value["default_scheme"].as_str().unwrap().to_owned();
    value["schemes"].as_array_mut().unwrap().push(json!({
        "id":"future","name":"Future runtime","profile":"future-protocol",
        "archives":["not-interpretable-by-old-client.zip"]
    }));
    value["default_scheme"] = json!("future");
    let catalog = serde_json::from_value::<cloud::CompactCatalog>(value)?.expand(&index)?;
    assert_eq!(catalog.default_scheme, known);
    assert!(catalog.skipped_schemes.contains_key("future"));
    assert_eq!(catalog.schemes().len(), 1);
    Ok(())
}

#[test]
fn version_gating_and_bad_known_defaults_are_distinct() -> Result<()> {
    let (mut value, index) = compact_catalog()?;
    let mut future = value["schemes"][0].clone();
    future["id"] = json!("new-manager-only");
    future["min_manager_version"] = json!("9999.0.0");
    value["schemes"].as_array_mut().unwrap().push(future);
    let catalog = serde_json::from_value::<cloud::CompactCatalog>(value)?.expand(&index)?;
    assert!(catalog.skipped_schemes.contains_key("new-manager-only"));
    let (mut bad, index) = compact_catalog()?;
    bad["schemes"][0]["defaults"] = json!(BTreeMap::from([("unknown_key", "1")]));
    assert!(
        serde_json::from_value::<cloud::CompactCatalog>(bad)?
            .expand(&index)
            .is_err()
    );
    Ok(())
}

#[test]
fn invalid_default_or_index_is_not_silently_accepted() -> Result<()> {
    let (mut value, index) = compact_catalog()?;
    value["default_scheme"] = json!("typo-not-a-scheme");
    assert!(
        serde_json::from_value::<cloud::CompactCatalog>(value)?
            .expand(&index)
            .is_err()
    );
    let (value, mut index) = compact_catalog()?;
    index.push(b' ');
    assert!(
        serde_json::from_value::<cloud::CompactCatalog>(value)?
            .expand(&index)
            .is_err()
    );
    Ok(())
}
