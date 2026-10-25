//! Independent recommendations for game DLSS files. Private NR/XeSS dependencies
//! stay pinned to patch compatibility; refreshing this catalog never installs DLLs.
use crate::{
    assets, core,
    runtime_files::{Package, bundled_package},
    transfer,
};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    sync::{OnceLock, RwLock, atomic::AtomicBool},
};

const CACHE: &str = "runtime-catalog.json";
static ACTIVE: OnceLock<RwLock<Option<Catalog>>> = OnceLock::new();

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Catalog {
    schema: u32,
    revision: u64,
    min_manager_version: String,
    packages: Vec<Package>,
    #[serde(default)]
    history: Vec<Package>,
}
impl Catalog {
    fn parse(bytes: &[u8]) -> Result<Self> {
        ensure!(bytes.len() <= 64 * 1024, "运行库清单过大");
        let c: Self = serde_json::from_slice(bytes)?;
        ensure!(
            c.schema == 1
                && c.revision > 0
                && c.packages.len() == 3
                && crate::updater::version(&c.min_manager_version)?
                    <= crate::updater::version(crate::VERSION)?,
            "运行库清单不适用于此管理器"
        );
        ensure!(c.history.len() <= 24, "运行库历史版本过多");
        ensure!(
            c.packages
                .iter()
                .map(|p| &p.kind)
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                == 3,
            "运行库类型无效"
        );
        let mut kinds = std::collections::BTreeSet::new();
        for p in c.packages.iter().chain(&c.history) {
            let latest = c.packages.iter().find(|latest| latest.kind == p.kind);
            ensure!(
                ["sr", "rr", "fg"].contains(&p.kind.as_str())
                    && kinds.insert((&p.kind, &p.version))
                    && latest.is_some(),
                "运行库类型无效"
            );
            let base = bundled_package(&p.kind)?;
            let v = semver::Version::parse(&p.version)?;
            let floor = semver::Version::parse(&base.version)?;
            ensure!(
                v.major == floor.major
                    && v.pre.is_empty()
                    && v.build.is_empty()
                    && if c.packages.contains(p) {
                        v >= floor
                    } else {
                        v < semver::Version::parse(&latest.unwrap().version)?
                    },
                "运行库版本超出已支持范围"
            );
            // A same-version alternative is not an upgrade, even if a remote
            // manifest points to a different, otherwise valid vendor build.
            ensure!(v != floor || p == &base, "同版本运行库内容不同");
            ensure!(
                p.bytes > 0
                    && p.bytes <= 512 * 1024 * 1024
                    && core::valid_hash(&p.sha256)
                    && p.files.len() == 2
                    && p.urls.len() == 2,
                "运行库包无效"
            );
            for (file, expected) in p.files.iter().zip(&base.files) {
                ensure!(
                    file.name == expected.name
                        && file.bytes > 0
                        && file.bytes <= 512 * 1024 * 1024
                        && core::valid_hash(&file.sha256),
                    "运行库文件无效"
                );
            }
            let name = format!("runtime-{}-{}-{}.zip", p.kind, p.version, &p.sha256[..12]);
            ensure!(
                p.urls
                    == [
                        format!(
                            "https://gitcode.com/gcw_qWIjmve7/RTX-FG-Manager-payloads/releases/download/payloads/{name}"
                        ),
                        format!(
                            "https://github.com/pandaligx/RTX-FG-Manager/releases/download/payloads/{name}"
                        )
                    ],
                "运行库来源不是管理器资源仓库"
            );
        }
        Ok(c)
    }
    fn merge(&self, old: &Self) -> Result<Self> {
        // Revision is metadata, not the identity of a package. Earlier caches
        // may have the same (or a higher) revision without the history field.
        // Merge immutable versions so those caches cannot pin an empty list.
        for prior in old.packages.iter().chain(&old.history) {
            if let Some(next) = self
                .packages
                .iter()
                .chain(&self.history)
                .find(|p| p.kind == prior.kind && p.version == prior.version)
            {
                ensure!(next == prior, "同版本运行库内容不同");
            }
        }
        let mut merged = self.clone();
        merged.revision = merged.revision.max(old.revision);
        if crate::updater::version(&old.min_manager_version)?
            > crate::updater::version(&merged.min_manager_version)?
        {
            merged
                .min_manager_version
                .clone_from(&old.min_manager_version);
        }
        merged.packages.clear();
        merged.history.clear();
        for kind in ["sr", "rr", "fg"] {
            let mut versions = self
                .packages
                .iter()
                .chain(&self.history)
                .chain(&old.packages)
                .chain(&old.history)
                .filter(|p| p.kind == kind)
                .map(|p| Ok((semver::Version::parse(&p.version)?, p.clone())))
                .collect::<Result<Vec<_>>>()?;
            versions.sort_by(|a, b| b.0.cmp(&a.0));
            versions.dedup_by(|a, b| a.0 == b.0);
            let mut versions = versions.into_iter().map(|(_, package)| package);
            merged.packages.push(
                versions
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("运行库类型无效"))?,
            );
            merged.history.extend(versions.take(8));
        }
        // Keep all validation limits in one place, including the merged size.
        Self::parse(&serde_json::to_vec(&merged)?)
    }
}
pub(crate) fn selected(kind: &str) -> Option<Package> {
    ACTIVE
        .get()?
        .read()
        .ok()?
        .as_ref()?
        .packages
        .iter()
        .find(|p| p.kind == kind)
        .cloned()
}
pub(crate) fn history(kind: &str) -> Vec<Package> {
    ACTIVE
        .get()
        .and_then(|lock| lock.read().ok())
        .and_then(|c| {
            c.as_ref().map(|c| {
                c.history
                    .iter()
                    .filter(|p| p.kind == kind)
                    .cloned()
                    .collect()
            })
        })
        .unwrap_or_default()
}
pub fn cached() -> Option<Catalog> {
    let path = core::no_links(&assets::cache_root().ok()?.join(CACHE)).ok()?;
    if fs::metadata(&path).ok()?.len() > 64 * 1024 {
        return None;
    }
    Catalog::parse(&fs::read(path).ok()?).ok()
}
pub fn activate(catalog: Catalog) -> Result<()> {
    let mut current = ACTIVE
        .get_or_init(|| RwLock::new(None))
        .write()
        .map_err(|_| anyhow::anyhow!("运行库清单状态异常"))?;
    *current = Some(match current.as_ref() {
        Some(old) => catalog.merge(old)?,
        None => catalog,
    });
    Ok(())
}
pub fn refresh(github: bool, cancel: &AtomicBool) -> Result<Catalog> {
    let mut sources = vec![
        transfer::Source { name: "gitcode".into(), url: "https://api.gitcode.com/api/v5/repos/gcw_qWIjmve7/RTX-FG-Manager/raw/cloud/runtime-catalog.json?ref=main".into() },
        transfer::Source { name: "github".into(), url: "https://raw.githubusercontent.com/pandaligx/RTX-FG-Manager/main/cloud/runtime-catalog.json".into() }
    ];
    sources.sort_by_key(|s| s.name != if github { "github" } else { "gitcode" });
    let old = cached();
    let next = fetch_from(sources, old.as_ref(), |source| {
        ensure!(
            !cancel.load(std::sync::atomic::Ordering::Relaxed),
            "运行库清单下载已取消"
        );
        transfer::fetch_metadata(vec![source], transfer::UrlPolicy::Official, cancel, |_| {})
    })?;
    ensure!(
        !cancel.load(std::sync::atomic::Ordering::Relaxed),
        "运行库清单下载已取消"
    );
    core::atomic_json(&assets::cache_root()?.join(CACHE), &next)?;
    Ok(next)
}
fn fetch_from(
    sources: Vec<transfer::Source>,
    old: Option<&Catalog>,
    mut fetch: impl FnMut(transfer::Source) -> Result<Vec<u8>>,
) -> Result<Catalog> {
    let mut errors = Vec::new();
    let mut available = old.cloned();
    let mut fetched = false;
    for source in sources {
        let name = source.name.clone();
        match fetch(source).and_then(|bytes| {
            let catalog = Catalog::parse(&bytes)?;
            let has_history = ["sr", "rr", "fg"]
                .iter()
                .all(|kind| catalog.history.iter().any(|p| p.kind == *kind));
            let merged = match available.as_ref() {
                Some(old) => catalog.merge(old)?,
                None => catalog,
            };
            Ok((merged, has_history))
        }) {
            Ok((catalog, true)) => return Ok(catalog),
            Ok((catalog, false)) => {
                // A mirror can still serve the pre-history schema. Try the
                // other source before reporting a successful empty refresh.
                available = Some(catalog);
                fetched = true;
            }
            Err(e) => errors.push(format!("{name}: {e}")),
        }
    }
    if fetched {
        return Ok(available.unwrap());
    }
    anyhow::bail!("{}", errors.join("; "))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "downloads only the two small public runtime catalogs, not DLLs"]
    fn published_runtime_catalogs_validate() -> Result<()> {
        for (name, url) in [
            (
                "gitcode",
                "https://api.gitcode.com/api/v5/repos/gcw_qWIjmve7/RTX-FG-Manager/raw/cloud/runtime-catalog.json?ref=main",
            ),
            (
                "github",
                "https://raw.githubusercontent.com/pandaligx/RTX-FG-Manager/main/cloud/runtime-catalog.json",
            ),
        ] {
            let bytes = transfer::fetch_metadata(
                vec![transfer::Source {
                    name: name.into(),
                    url: url.into(),
                }],
                transfer::UrlPolicy::Official,
                &AtomicBool::new(false),
                |_| {},
            )?;
            let catalog = Catalog::parse(&bytes)?;
            println!(
                "{name}: revision {}, {} bytes, valid",
                catalog.revision,
                bytes.len()
            );
        }
        Ok(())
    }
    fn fixture() -> serde_json::Value {
        serde_json::json!({"schema":1,"revision":1,"min_manager_version":"4.3.10",
            "packages": [bundled_package("sr").unwrap(), bundled_package("rr").unwrap(), bundled_package("fg").unwrap()]})
    }
    #[test]
    fn history_allows_explicit_rollback_but_not_automatic_downgrade_or_mutation() {
        let mut v = fixture();
        let mut p = v["packages"][0].clone();
        p["version"] = "310.9.1".into();
        for url in p["urls"].as_array_mut().unwrap() {
            *url = url.as_str().unwrap().replace("310.9.16", "310.9.1").into();
        }
        v["history"] = serde_json::json!([p]);
        let old = Catalog::parse(&serde_json::to_vec(&v).unwrap()).unwrap();
        assert_eq!(old.history[0].version, "310.9.1");
        let mut next = old.clone();
        next.revision += 1;
        next.history[0].bytes += 1;
        assert!(next.merge(&old).is_err());
        v["history"] = serde_json::json!([v["history"][0], v["history"][0]]);
        assert!(!valid(&v));
    }
    fn valid(v: &serde_json::Value) -> bool {
        Catalog::parse(&serde_json::to_vec(v).unwrap()).is_ok()
    }
    #[test]
    fn invalid_primary_metadata_uses_the_other_source() {
        let sources = vec![
            transfer::Source {
                name: "gitcode".into(),
                url: String::new(),
            },
            transfer::Source {
                name: "github".into(),
                url: String::new(),
            },
        ];
        let mut calls = 0;
        let result = fetch_from(sources, None, |_| {
            calls += 1;
            Ok(if calls == 1 {
                b"<html>error</html>".to_vec()
            } else {
                serde_json::to_vec(&fixture()).unwrap()
            })
        });
        assert!(result.is_ok());
        assert_eq!(calls, 2);
    }
    #[test]
    fn accepts_complete_baseline_rejects_private_types_and_foreign_urls() {
        let mut v = fixture();
        assert!(valid(&v));
        v["packages"][0]["kind"] = "xemfg".into();
        assert!(!valid(&v));
        let mut v = fixture();
        v["packages"][0]["urls"][0] = "https://gitcode.com/other/file.zip".into();
        assert!(!valid(&v));
    }
    #[test]
    fn rejects_same_version_mutation_new_major_prerelease_and_duplicate() {
        for version in ["309.9.0", "311.0.0", "310.10.0-beta", "310.10.0+dev"] {
            let mut v = fixture();
            v["packages"][0]["version"] = version.into();
            assert!(!valid(&v));
        }
        let mut v = fixture();
        v["packages"][0]["bytes"] = 1.into();
        assert!(!valid(&v));
        let mut v = fixture();
        v["packages"][0] = v["packages"][1].clone();
        assert!(!valid(&v));
    }
    #[test]
    fn keeps_immutable_packages_when_revision_changes() {
        let old = Catalog::parse(&serde_json::to_vec(&fixture()).unwrap()).unwrap();
        let mut next = old.clone();
        assert!(next.merge(&old).unwrap() == old);
        next.revision += 1;
        assert_eq!(next.merge(&old).unwrap().revision, next.revision);
        next.packages[0].sha256 = "a".repeat(64);
        assert!(next.merge(&old).is_err());
    }
    fn published_fixture() -> Catalog {
        Catalog::parse(include_bytes!("../../cloud/runtime-catalog.json")).unwrap()
    }
    fn sources() -> Vec<transfer::Source> {
        ["gitcode", "github"]
            .into_iter()
            .map(|name| transfer::Source {
                name: name.into(),
                url: String::new(),
            })
            .collect()
    }
    #[test]
    fn old_cache_without_history_is_repaired_despite_same_or_higher_revision() {
        let incoming = published_fixture();
        for revision in [incoming.revision, incoming.revision + 99999] {
            let mut old = incoming.clone();
            old.history.clear();
            old.revision = revision;
            let repaired = incoming.merge(&old).unwrap();
            assert!(repaired.history == incoming.history);
            assert!(repaired.packages == old.packages);
            assert_eq!(repaired.revision, revision);
            // The repaired document stays useful after a restart/offline read.
            let reloaded = Catalog::parse(&serde_json::to_vec(&repaired).unwrap()).unwrap();
            assert!(reloaded == repaired);
            assert!(incoming.merge(&reloaded).unwrap() == repaired);
        }
    }
    #[test]
    fn stale_mirror_without_history_checks_secondary_and_retains_known_versions() {
        let full = published_fixture();
        let mut stale = full.clone();
        stale.history.clear();
        let mut calls = 0;
        let fetched = fetch_from(sources(), Some(&stale), |_| {
            calls += 1;
            Ok(serde_json::to_vec(if calls == 1 { &stale } else { &full }).unwrap())
        })
        .unwrap();
        assert_eq!(calls, 2);
        assert!(fetched.history == full.history);
        let mut calls = 0;
        let fetched = fetch_from(sources(), Some(&full), |_| {
            calls += 1;
            if calls == 2 {
                anyhow::bail!("offline")
            }
            Ok(serde_json::to_vec(&stale).unwrap())
        })
        .unwrap();
        assert!(fetched.history == full.history);
        assert!(fetch_from(sources(), Some(&full), |_| anyhow::bail!("offline")).is_err());
    }
    #[test]
    fn conflicting_primary_history_falls_back_even_with_a_newer_revision() {
        let full = published_fixture();
        let mut conflict = full.clone();
        conflict.revision += 1;
        conflict.history[0].bytes += 1;
        let mut calls = 0;
        let fetched = fetch_from(sources(), Some(&full), |_| {
            calls += 1;
            Ok(serde_json::to_vec(if calls == 1 { &conflict } else { &full }).unwrap())
        })
        .unwrap();
        assert_eq!(calls, 2);
        assert!(fetched == full);
    }
    #[test]
    fn merging_stale_mirrors_never_downgrades_and_limits_history_per_component() {
        let old = published_fixture();
        let mut newer = old.clone();
        // Use valid, synthetic future versions; no network or DLL writes.
        for package in &mut newer.packages {
            for patch in 17..30 {
                let mut historical = package.clone();
                historical.version = format!("310.9.{patch}");
                historical.urls = package
                    .urls
                    .iter()
                    .map(|url| url.replace("310.9.16", &historical.version))
                    .collect();
                newer.history.push(historical);
            }
            package.version = "310.9.30".into();
            package.urls = package
                .urls
                .iter()
                .map(|url| url.replace("310.9.16", "310.9.30"))
                .collect();
        }
        let merged = old.merge(&newer).unwrap();
        assert!(merged.packages.iter().all(|p| p.version == "310.9.30"));
        assert_eq!(merged.history.len(), 24);
        for kind in ["sr", "rr", "fg"] {
            assert_eq!(merged.history.iter().filter(|p| p.kind == kind).count(), 8);
        }
    }
}
