//! Cloud payloads: bounded downloads, validated archives and offline cache.
//! Network requests run only in controller workers, never in cleanup or rendering.
use crate::{assets, core};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{Cursor, Read},
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

pub const DOMESTIC: &str = "https://gitee.com/pandaligx/RTX-FG-Manager/raw/main/cloud/catalog.json";
pub const GITHUB: &str =
    "https://raw.githubusercontent.com/pandaligx/RTX-FG-Manager/main/cloud/catalog.json";
const LIMIT: u64 = 128 * 1024 * 1024;
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct File {
    pub name: String,
    pub bytes: u64,
    pub sha256: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Package {
    pub id: String,
    pub scheme_id: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub labels: BTreeMap<String, String>,
    pub version: String,
    pub backends: Vec<String>,
    pub proxy: String,
    pub archive: String,
    pub bytes: u64,
    pub sha256: String,
    pub files: Vec<File>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Policy {
    pub gpu_paths: Vec<String>,
    pub max_selected_proxies: usize,
    pub ini_policy: String,
    #[serde(default)]
    pub parameter_profile: String,
    #[serde(default)]
    pub defaults: crate::presets::Values,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub capabilities: BTreeSet<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Source {
    pub base_url: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Catalog {
    #[serde(skip)]
    pub prefer_github: bool,
    pub schema: u32,
    pub revision: String,
    pub default_scheme: String,
    pub sources: BTreeMap<String, Source>,
    pub packages: Vec<Package>,
    pub scheme_policies: BTreeMap<String, Policy>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub skipped_schemes: BTreeMap<String, String>,
}
pub struct Prepared {
    pub backend: String,
    pub version: String,
    pub files: BTreeMap<String, Vec<u8>>,
    pub scheme_id: String,
    pub policy: Policy,
}

#[derive(Deserialize)]
pub struct CompactCatalog {
    pub schema: u32,
    pub revision: String,
    pub default_scheme: String,
    pub sources: BTreeMap<String, Source>,
    pub index: IndexLocation,
    pub schemes: Vec<CompactScheme>,
}
#[derive(Deserialize)]
pub struct IndexLocation {
    pub url: String,
    pub fallback_url: String,
    pub sha256: String,
    pub bytes: u64,
}
#[derive(Deserialize)]
pub struct CompactScheme {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub names: BTreeMap<String, String>,
    pub profile: String,
    #[serde(default)]
    pub min_manager_version: Option<String>,
    #[serde(default)]
    pub defaults: crate::presets::Values,
    pub archives: Vec<String>,
    #[serde(default)]
    pub capabilities: BTreeSet<String>,
}
impl CompactCatalog {
    pub fn expand(self, index: &[u8]) -> Result<Catalog> {
        ensure!(
            self.schema == 2 && !self.schemes.is_empty() && self.schemes.len() <= 32,
            "Invalid compact catalog"
        );
        ensure!(
            index.len() <= 1024 * 1024
                && index.len() as u64 == self.index.bytes
                && core::valid_hash(&self.index.sha256)
                && core::hash(index) == self.index.sha256,
            "Cloud index integrity mismatch"
        );
        #[derive(Deserialize)]
        struct Index {
            schema: u32,
            packages: Vec<Package>,
        }
        let index: Index = serde_json::from_slice(index)?;
        ensure!(
            index.schema == 1 && index.packages.len() <= 256,
            "Invalid cloud index"
        );
        let mut packages = Vec::new();
        let mut policies = BTreeMap::new();
        let mut skipped_schemes = BTreeMap::new();
        let mut scheme_ids = BTreeSet::new();
        for s in self.schemes {
            ensure!(
                !s.id.is_empty() && s.id.len() <= 150 && scheme_ids.insert(s.id.clone()),
                "Duplicate/invalid scheme"
            );
            let newer = s
                .min_manager_version
                .as_deref()
                .map(crate::updater::version)
                .transpose()?
                .is_some_and(|minimum| {
                    minimum
                        > crate::updater::version(crate::VERSION).expect("valid manager version")
                });
            if newer || crate::presets::validate(&s.profile, &BTreeMap::new()).is_err() {
                skipped_schemes.insert(s.id, "此方案需要更新管理器".into());
                continue;
            }
            crate::presets::validate(&s.profile, &s.defaults)?;
            ensure!(
                !policies.contains_key(&s.id) && !s.archives.is_empty(),
                "Duplicate/empty scheme"
            );
            let (ini_policy, max_selected_proxies) = match s.profile.as_str() {
                "initial" => ("initial", 1),
                "native026" => ("native", 5),
                crate::presets::MFG_VULKAN => ("upstream_proxy", 1),
                _ => ("upstream_proxy", 6),
            };
            for name in &s.archives {
                let matches = index
                    .packages
                    .iter()
                    .filter(|p| &p.archive == name)
                    .collect::<Vec<_>>();
                ensure!(matches.len() == 1, "Missing or ambiguous archive: {name}");
                let mut p = matches[0].clone();
                p.scheme_id = s.id.clone();
                p.label = s.name.clone();
                p.labels = s.names.clone();
                packages.push(p);
            }
            policies.insert(
                s.id,
                Policy {
                    gpu_paths: vec!["SM75".into(), "SM86".into()],
                    max_selected_proxies,
                    ini_policy: ini_policy.into(),
                    parameter_profile: s.profile,
                    defaults: s.defaults,
                    capabilities: s.capabilities,
                },
            );
        }
        ensure!(
            !packages.is_empty(),
            "No compatible schemes; update the manager"
        );
        let default_scheme = if policies.contains_key(&self.default_scheme) {
            self.default_scheme
        } else {
            ensure!(
                skipped_schemes.contains_key(&self.default_scheme),
                "Missing default scheme"
            );
            packages[0].scheme_id.clone()
        };
        let c = Catalog {
            prefer_github: false,
            schema: 2,
            revision: self.revision,
            default_scheme,
            sources: self.sources,
            packages,
            scheme_policies: policies,
            skipped_schemes,
        };
        c.validate()?;
        Ok(c)
    }
}
fn safe_url(value: &str) -> Result<reqwest::Url> {
    crate::transfer::validate_url(value, crate::transfer::UrlPolicy::Cloud)
}
impl Catalog {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.schema == 2 && !self.packages.is_empty() && self.packages.len() <= 256,
            "Unsupported cloud catalog"
        );
        ensure!(
            self.scheme_policies.contains_key(&self.default_scheme),
            "Missing default scheme"
        );
        ensure!(
            self.packages
                .iter()
                .any(|p| p.scheme_id == self.default_scheme),
            "Default scheme has no packages"
        );
        for name in ["domestic", "github"] {
            let url = &self
                .sources
                .get(name)
                .context("Missing cloud source")?
                .base_url;
            safe_url(url)?;
            ensure!(url.ends_with('/'), "Cloud base URL must end in /");
        }
        let mut ids = BTreeSet::new();
        let mut routes = BTreeSet::new();
        for p in &self.packages {
            let policy = self
                .scheme_policies
                .get(&p.scheme_id)
                .context("Missing scheme policy")?;
            crate::presets::validate(&policy.parameter_profile, &policy.defaults)?;
            if policy.parameter_profile == crate::presets::MFG_VULKAN {
                ensure!(
                    p.proxy == "version.dll" && policy.max_selected_proxies == 1,
                    "MFG Vulkan sm86-7 provides only version.dll"
                );
            }
            ensure!(
                policy.capabilities.len() <= 16
                    && policy
                        .capabilities
                        .iter()
                        .all(|c| !c.is_empty() && c.len() <= 64),
                "Invalid scheme capabilities"
            );
            ensure!(
                !policy.capabilities.contains(crate::delta::CAPABILITY)
                    || policy.parameter_profile == "upstream035",
                "Delta capability requires the 0.3.5 protocol"
            );
            ensure!(
                match policy.parameter_profile.as_str() {
                    "initial" => policy.ini_policy == "initial",
                    "native026" => policy.ini_policy == "native",
                    _ => policy.ini_policy == "upstream_proxy",
                },
                "Parameter/backend mismatch"
            );
            ensure!(
                !p.label.is_empty() && p.label.len() <= 200 && ids.insert(&p.id),
                "Duplicate or invalid cloud package"
            );
            crate::updater::version(&p.version)?;
            ensure!(
                core::PROXIES.contains(&p.proxy.as_str()) && p.files.len() == 2,
                "Invalid cloud proxy"
            );
            ensure!(
                !p.archive.is_empty()
                    && p.archive.len() <= 150
                    && p.archive.ends_with(".zip")
                    && p.archive
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b)),
                "Invalid archive name"
            );
            ensure!(
                p.bytes > 0 && p.bytes <= LIMIT && core::valid_hash(&p.sha256),
                "Invalid archive digest/size"
            );
            ensure!(
                p.files
                    .iter()
                    .map(|f| f.name.as_str())
                    .collect::<BTreeSet<_>>()
                    == BTreeSet::from([p.proxy.as_str(), core::INI]),
                "Invalid package entries"
            );
            for f in &p.files {
                ensure!(
                    f.bytes > 0 && f.bytes <= LIMIT && core::valid_hash(&f.sha256),
                    "Invalid entry digest/size"
                );
            }
            ensure!(
                policy.max_selected_proxies >= 1
                    && policy.max_selected_proxies <= 7
                    && matches!(
                        policy.ini_policy.as_str(),
                        "native" | "initial" | "upstream_proxy"
                    ),
                "Unsupported deployment protocol"
            );
            ensure!(!p.backends.is_empty(), "Missing backend");
            for b in &p.backends {
                ensure!(
                    core::BACKENDS.contains(&b.as_str()),
                    "Unsupported cloud backend"
                );
                core::folder(b)?;
                ensure!(
                    routes.insert((&p.scheme_id, b, &p.proxy)),
                    "Ambiguous package route"
                );
                let expected = if b == "upstream_sm86" {
                    "upstream_proxy"
                } else if b.starts_with("native") {
                    "native"
                } else {
                    "initial"
                };
                ensure!(expected == policy.ini_policy, "Backend policy mismatch");
            }
            ensure!(
                policy
                    .gpu_paths
                    .iter()
                    .all(|g| matches!(g.as_str(), "SM75" | "SM86"))
                    && !policy.gpu_paths.is_empty(),
                "Invalid GPU path"
            );
            if policy.ini_policy == "upstream_proxy" {
                ensure!(
                    policy
                        .gpu_paths
                        .iter()
                        .all(|path| matches!(path.as_str(), "SM75" | "SM86"))
                        && policy.max_selected_proxies <= 6,
                    "Unsupported upstream GPU/multi-proxy policy"
                );
            }
        }
        Ok(())
    }
    pub fn schemes(&self) -> Vec<&Package> {
        let mut seen = BTreeSet::new();
        self.packages
            .iter()
            .filter(|p| seen.insert(&p.scheme_id))
            .collect()
    }
    pub fn selected(&self, id: &str) -> &Package {
        self.packages
            .iter()
            .find(|p| p.scheme_id == id)
            .unwrap_or_else(|| {
                self.packages
                    .iter()
                    .find(|p| p.scheme_id == self.default_scheme)
                    .unwrap()
            })
    }
    pub fn proxies(&self, id: &str) -> Vec<String> {
        self.packages
            .iter()
            .filter(|p| p.scheme_id == id)
            .map(|p| p.proxy.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }
}
pub fn bundled() -> Catalog {
    let c: Catalog = serde_json::from_str(include_str!("../cloud-catalog.json"))
        .expect("built-in cloud catalog");
    c.validate().expect("valid built-in cloud catalog");
    c
}
pub fn cached() -> Catalog {
    assets::cache_root()
        .ok()
        .and_then(|r| core::read_json(&r.join("cloud-catalog.json"), 1024 * 1024).ok())
        .and_then(|v| serde_json::from_value::<Catalog>(v).ok())
        .filter(|c| c.validate().is_ok())
        .unwrap_or_else(bundled)
}
pub fn refresh() -> Result<Catalog> {
    refresh_with_options(false, &AtomicBool::new(false), |_| {})
}

pub fn refresh_with_options(
    prefer_github: bool,
    cancel: &AtomicBool,
    mut progress: impl FnMut(crate::transfer::Progress),
) -> Result<Catalog> {
    let sources = if prefer_github {
        [("github", GITHUB), ("gitee", DOMESTIC)]
    } else {
        [("gitee", DOMESTIC), ("github", GITHUB)]
    };
    let mut errors = Vec::new();
    for (name, url) in sources {
        ensure!(!cancel.load(Ordering::Relaxed), "下载已取消");
        let mut stage = "清单下载失败";
        let result = (|| -> Result<Catalog> {
            let bytes = crate::transfer::fetch_metadata(
                vec![crate::transfer::Source {
                    name: name.into(),
                    url: url.into(),
                }],
                crate::transfer::UrlPolicy::Official,
                cancel,
                &mut progress,
            )?;
            stage = "清单格式无效";
            let compact: CompactCatalog =
                serde_json::from_slice(bytes.strip_prefix(&[239, 187, 191]).unwrap_or(&bytes))?;
            ensure!(
                compact.index.bytes > 0
                    && compact.index.bytes <= 1024 * 1024
                    && core::valid_hash(&compact.index.sha256),
                "Invalid index size/digest"
            );
            safe_url(&compact.index.url)?;
            safe_url(&compact.index.fallback_url)?;
            let urls = if prefer_github {
                [
                    (&compact.index.fallback_url, "github"),
                    (&compact.index.url, "gitee"),
                ]
            } else {
                [
                    (&compact.index.url, "gitee"),
                    (&compact.index.fallback_url, "github"),
                ]
            };
            let root = core::no_links(&assets::cache_root()?)?;
            fs::create_dir_all(&root)?;
            let temp = tempfile::Builder::new()
                .prefix("metadata-")
                .tempdir_in(&root)?;
            let request = crate::transfer::Request {
                sources: urls
                    .into_iter()
                    .map(|(url, name)| crate::transfer::Source {
                        name: name.into(),
                        url: url.clone(),
                    })
                    .collect(),
                bytes: Some(compact.index.bytes),
                sha256: Some(compact.index.sha256.clone()),
                limit: 1024 * 1024,
                metadata: true,
                policy: crate::transfer::UrlPolicy::Cloud,
            };
            stage = "索引下载或完整性校验失败";
            let index = crate::transfer::download(
                &request,
                &temp.path().join("index.json"),
                cancel,
                &mut progress,
            )?;
            stage = "索引或方案配置无效";
            let mut c = compact.expand(&fs::read(index)?)?;
            c.prefer_github = prefer_github;
            c.validate()?;
            ensure!(!cancel.load(Ordering::Relaxed), "下载已取消");
            stage = "云端目录缓存写入失败";
            core::atomic_json(&root.join("cloud-catalog.json"), &c)?;
            Ok(c)
        })();
        match result {
            Ok(c) => return Ok(c),
            Err(_) => {
                ensure!(!cancel.load(Ordering::Relaxed), "下载已取消");
                errors.push(format!("{name}: {stage}"));
                progress(crate::transfer::Progress {
                    source: name.into(),
                    phase: crate::transfer::Phase::Switching,
                    detail: stage.into(),
                    ..Default::default()
                });
            }
        }
    }
    ensure!(!cancel.load(Ordering::Relaxed), "下载已取消");
    bail!(
        "Cloud catalog unavailable; cached catalog retained: {}",
        errors.join("；")
    )
}

pub fn unpack(p: &Package, bytes: &[u8]) -> Result<BTreeMap<String, Vec<u8>>> {
    ensure!(
        bytes.len() as u64 == p.bytes && core::hash(bytes) == p.sha256,
        "Cloud archive integrity mismatch"
    );
    let mut z = zip::ZipArchive::new(Cursor::new(bytes))?;
    ensure!(z.len() == p.files.len(), "Unexpected archive entries");
    let mut out = BTreeMap::new();
    for i in 0..z.len() {
        let mut entry = z.by_index(i)?;
        let f = p
            .files
            .iter()
            .find(|f| f.name == entry.name())
            .context("Unexpected archive path")?;
        ensure!(
            entry.size() == f.bytes
                && !entry.is_dir()
                && entry.unix_mode().is_none_or(|m| m & 0o170000 != 0o120000),
            "Invalid archive entry"
        );
        let mut b = Vec::new();
        (&mut entry).take(f.bytes + 1).read_to_end(&mut b)?;
        ensure!(
            b.len() as u64 == f.bytes && core::hash(&b) == f.sha256 && !out.contains_key(&f.name),
            "Cloud entry integrity mismatch"
        );
        if f.name.ends_with(".dll") {
            ensure!(b.len() >= 64 && &b[..2] == b"MZ", "Invalid DLL header");
            let n = u32::from_le_bytes(b[60..64].try_into()?) as usize;
            let h = b
                .get(n..n.saturating_add(24))
                .context("Invalid PE offset")?;
            ensure!(
                &h[..4] == b"PE\0\0"
                    && h[4..6] == [0x64, 0x86]
                    && u16::from_le_bytes([h[22], h[23]]) & 0x2000 != 0,
                "Not an x64 DLL"
            );
        } else {
            std::str::from_utf8(&b).context("INI is not UTF-8")?;
        }
        out.insert(f.name.clone(), b);
    }
    Ok(out)
}
fn remove_partial(path: &Path) -> Result<()> {
    let control = path.with_file_name(format!(
        "{}.aria2",
        path.file_name()
            .context("Invalid cloud cache path")?
            .to_string_lossy()
    ));
    for item in [path, control.as_path()] {
        if item.exists() {
            fs::remove_file(core::no_links(item)?)?;
        }
    }
    Ok(())
}
fn archive(
    c: &Catalog,
    p: &Package,
    cancel: &AtomicBool,
    report: &mut impl FnMut(crate::transfer::Progress),
) -> Result<BTreeMap<String, Vec<u8>>> {
    let root = core::no_links(&assets::cache_root()?.join("cloud"))?;
    fs::create_dir_all(&root)?;
    let path = core::no_links(&root.join(format!("{}.zip", p.sha256)))?;
    ensure!(!cancel.load(Ordering::Relaxed), "下载已取消");
    if path.is_file()
        && path.metadata()?.len() == p.bytes
        && let Ok(files) = unpack(p, &fs::read(&path)?)
    {
        report(crate::transfer::Progress {
            completed: p.bytes,
            total: p.bytes,
            source: "cache".into(),
            phase: crate::transfer::Phase::Complete,
            ..Default::default()
        });
        return Ok(files);
    }
    if path.exists() {
        fs::remove_file(core::no_links(&path)?)?;
    }
    let order = if c.prefer_github {
        ["github", "domestic"]
    } else {
        ["domestic", "github"]
    };
    let request = crate::transfer::Request {
        sources: order
            .into_iter()
            .map(|name| crate::transfer::Source {
                name: if name == "domestic" && c.sources[name].base_url.contains("gitee.com/") {
                    "gitee".into()
                } else {
                    name.into()
                },
                url: c.sources[name].base_url.clone() + &p.archive,
            })
            .collect(),
        bytes: Some(p.bytes),
        sha256: Some(p.sha256.clone()),
        limit: LIMIT,
        metadata: false,
        policy: crate::transfer::UrlPolicy::Cloud,
    };
    // Keep the pre-existing owned cache naming format. The source shown in the
    // UI comes from the transfer, never from this local filename.
    let partial = core::no_links(&root.join(format!("{}.{}.download.zip", p.sha256, order[0])))?;
    crate::transfer::download(&request, &partial, cancel, &mut *report)?;
    let bytes = fs::read(&partial)?;
    let files = unpack(p, &bytes)?;
    ensure!(!cancel.load(Ordering::Relaxed), "下载已取消");
    use std::io::Write;
    let mut tmp = tempfile::NamedTempFile::new_in(&root)?;
    tmp.write_all(&bytes)?;
    tmp.as_file().sync_all()?;
    core::no_links(&path)?;
    tmp.persist(&path)?;
    remove_partial(&partial)?;
    Ok(files)
}

#[derive(Clone, Debug)]
pub struct CloudProgress {
    pub transfer: crate::updater::DownloadProgress,
    pub proxy: String,
    pub package_index: usize,
    pub package_count: usize,
}

pub fn prepare(
    c: &Catalog,
    scheme: &str,
    series: usize,
    proxies: &[String],
    cancel: &AtomicBool,
    mut report: impl FnMut(String),
) -> Result<Prepared> {
    let mut previous = None;
    prepare_with_progress(c, scheme, series, proxies, cancel, |p| {
        let key = (p.package_index, p.transfer.source.clone(), p.transfer.phase);
        if previous.as_ref() != Some(&key) {
            report(format!(
                "DLL · {} · {}/{} · {}",
                p.transfer.source, p.package_index, p.package_count, p.proxy
            ));
            previous = Some(key);
        }
    })
}

pub fn prepare_with_progress(
    c: &Catalog,
    scheme: &str,
    series: usize,
    proxies: &[String],
    cancel: &AtomicBool,
    mut report: impl FnMut(CloudProgress),
) -> Result<Prepared> {
    c.validate()?;
    let _cache_lock = crate::cache::operation_lock()?;
    ensure!(series <= 1, "Invalid GPU selection");
    let policy = c
        .scheme_policies
        .get(scheme)
        .context("Scheme no longer available")?;
    ensure!(
        policy
            .gpu_paths
            .iter()
            .any(|g| g == if series == 0 { "SM75" } else { "SM86" }),
        "This scheme does not support the selected GPU path"
    );
    let selected = core::normalize_proxies(proxies)?;
    ensure!(
        selected.len() <= policy.max_selected_proxies,
        "This scheme requires a single DLL proxy"
    );
    let mut out = BTreeMap::new();
    let mut backend = None;
    let mut version = None;
    let package_count = selected.len();
    for (index, proxy) in selected.into_iter().enumerate() {
        ensure!(!cancel.load(Ordering::Relaxed), "Cancelled");
        let p = c
            .packages
            .iter()
            .find(|p| {
                p.scheme_id == scheme
                    && p.proxy == proxy
                    && p.backends.iter().any(|b| {
                        if policy.ini_policy == "upstream_proxy" {
                            b == "upstream_sm86"
                        } else {
                            b.ends_with(if series == 0 { "20" } else { "30" })
                        }
                    })
            })
            .context("Proxy unavailable in this scheme")?;
        let b = p
            .backends
            .iter()
            .find(|b| {
                if policy.ini_policy == "upstream_proxy" {
                    b.as_str() == "upstream_sm86"
                } else {
                    b.ends_with(if series == 0 { "20" } else { "30" })
                }
            })
            .context("Missing GPU package")?;
        ensure!(
            backend.as_ref().is_none_or(|v| v == b)
                && version.as_ref().is_none_or(|v| v == &p.version),
            "Inconsistent scheme packages"
        );
        backend = Some(b.clone());
        version = Some(p.version.clone());
        let mut package_progress = |transfer| {
            report(CloudProgress {
                transfer,
                proxy: proxy.clone(),
                package_index: index + 1,
                package_count,
            })
        };
        for (name, bytes) in archive(c, p, cancel, &mut package_progress)? {
            ensure!(
                out.get(&name).is_none_or(|old| old == &bytes),
                "Conflicting INI files"
            );
            out.insert(name, bytes);
        }
    }
    let backend = backend.context("No package selected")?;
    if policy.parameter_profile != crate::presets::MFG_VULKAN {
        core::configure_package(&backend, &mut out)?;
    }
    remember_images(&out)?;
    Ok(Prepared {
        backend,
        version: version.context("No version")?,
        files: out,
        scheme_id: scheme.into(),
        policy: policy.clone(),
    })
}
fn remember_images(files: &BTreeMap<String, Vec<u8>>) -> Result<()> {
    let root = assets::cache_root()?;
    std::fs::create_dir_all(&root)?;
    let _lock = crate::win::game_lock(&root)?;
    let path = root.join("cloud-identities.json");
    let mut hashes: BTreeSet<String> = if path.exists() {
        serde_json::from_value(core::read_json(&path, 1024 * 1024)?)?
    } else {
        BTreeSet::new()
    };
    for (n, b) in files {
        if n.ends_with(".dll") {
            hashes.insert(crate::cleanup::image_digest(b).context("Invalid DLL image identity")?);
        }
    }
    ensure!(hashes.len() <= 10000, "Cloud identity cache full");
    core::atomic_json(&path, &hashes)
}
pub fn known_image(image: &str) -> bool {
    static ORIGINALS: std::sync::OnceLock<BTreeSet<String>> = std::sync::OnceLock::new();
    if ORIGINALS
        .get_or_init(|| {
            serde_json::from_str(include_str!("../cloud-identities.json"))
                .expect("cleanup image catalog")
        })
        .contains(image)
    {
        return true;
    }
    assets::cache_root()
        .ok()
        .and_then(|r| core::read_json(&r.join("cloud-identities.json"), 1024 * 1024).ok())
        .and_then(|v| serde_json::from_value::<BTreeSet<String>>(v).ok())
        .is_some_and(|h| h.contains(image))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[cfg(feature = "fixture-tests")]
    fn all_prepared_packages_match_and_keep_route_configuration() {
        let c = bundled();
        for p in &c.packages {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/runtime/packages")
                .join(&p.archive);
            let mut files = unpack(p, &std::fs::read(path).unwrap()).unwrap();
            for backend in &p.backends {
                let original = files[&p.proxy].clone();
                if c.scheme_policies[&p.scheme_id].parameter_profile == crate::presets::MFG_VULKAN {
                    let values =
                        crate::presets::defaults(crate::presets::MFG_VULKAN, &BTreeMap::new());
                    assert_eq!(
                        crate::presets::configure(
                            &files[core::INI],
                            crate::presets::MFG_VULKAN,
                            &values
                        )
                        .unwrap(),
                        files[core::INI]
                    );
                    continue;
                }
                core::configure_package(backend, &mut files).unwrap();
                assert_eq!(original, files[&p.proxy]);
                let ini =
                    crate::cleanup::parse_ini(std::str::from_utf8(&files[core::INI]).unwrap());
                if backend == "upstream_sm86" {
                    assert!(!ini["Compatibility"].contains_key("Router"));
                }
                if backend.starts_with("native") {
                    assert_eq!(
                        ini["Compatibility"]["Router"],
                        if backend.ends_with("20") {
                            "SM75"
                        } else {
                            "SM86"
                        }
                    );
                }
                if backend == "upstream_sm86" {
                    assert_eq!(ini["Logging"]["Directory"], "dlssg_sm86\\logs");
                    assert_eq!(ini["Runtime"]["CacheDirectory"], "");
                } else {
                    assert_eq!(ini["Logging"]["Directory"], format!("{}\\logs", core::OWN));
                }
            }
        }
    }
    #[test]
    fn reject_invalid_catalog_and_urls() {
        let mut c = bundled();
        c.validate().unwrap();
        c.packages[0].archive = "../evil.zip".into();
        assert!(c.validate().is_err());
        for u in [
            "http://example.org/",
            "file:///c:/a",
            "https://user:pw@example.org/",
        ] {
            assert!(safe_url(u).is_err());
        }
        let mut c = bundled();
        c.packages.push(c.packages[0].clone());
        assert!(c.validate().is_err());
    }
    #[test]
    fn reject_zip_paths_even_with_matching_archive_hash() {
        use std::io::Write;
        let mut p = bundled().packages[0].clone();
        let mut z = zip::ZipWriter::new(Cursor::new(Vec::new()));
        z.start_file("../version.dll", zip::write::FileOptions::default())
            .unwrap();
        z.write_all(b"unsafe").unwrap();
        z.start_file(core::INI, zip::write::FileOptions::default())
            .unwrap();
        z.write_all(b"[Logging]\nLevel=1").unwrap();
        let bytes = z.finish().unwrap().into_inner();
        p.bytes = bytes.len() as u64;
        p.sha256 = core::hash(&bytes);
        assert!(unpack(&p, &bytes).is_err());
    }
    #[test]
    fn reject_corrupt_archive_and_cancel_before_download() {
        let c = bundled();
        assert!(unpack(&c.packages[0], b"wrong").is_err());
        assert!(
            prepare(
                &c,
                &c.default_scheme,
                1,
                &["version.dll".into()],
                &AtomicBool::new(true),
                |_| {}
            )
            .is_err()
        );
        assert!(
            prepare(
                &c,
                "initial-rtx30",
                0,
                &["version.dll".into()],
                &AtomicBool::new(false),
                |_| {}
            )
            .is_err()
        );
    }
}
