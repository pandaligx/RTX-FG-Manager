//! Versioned, content-only help. Downloaded pages receive no native bindings.
use crate::{assets, core, transfer};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

const FILES: [(&str, &[u8]); 4] = [
    ("index.html", include_bytes!("../../help/index.html")),
    ("style.css", include_bytes!("../../help/style.css")),
    ("content.js", include_bytes!("../../help/content.js")),
    ("app.js", include_bytes!("../../help/app.js")),
];
const LIMIT: u64 = 8 * 1024 * 1024;
static UPDATING: AtomicBool = AtomicBool::new(false);
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Manifest {
    pub schema: u32,
    pub version: String,
    pub min_client: String,
    pub max_client: Option<String>,
    pub languages: Vec<String>,
    pub urls: Vec<String>,
    pub bytes: u64,
    pub sha256: String,
    pub files: BTreeMap<String, String>,
}
impl Manifest {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.schema == 1
                && self.bytes > 0
                && self.bytes <= LIMIT
                && core::valid_hash(&self.sha256),
            "帮助清单无效"
        );
        core_version(&self.version)?;
        ensure!(
            crate::updater::version(&self.min_client)? <= crate::updater::version(crate::VERSION)?,
            "帮助需要更新客户端"
        );
        if let Some(max) = &self.max_client {
            ensure!(
                crate::updater::version(crate::VERSION)? <= crate::updater::version(max)?,
                "帮助不适用于当前客户端"
            );
        }
        ensure!(
            !self.languages.is_empty()
                && self.languages.len() <= 5
                && self
                    .languages
                    .iter()
                    .all(|l| ["zh", "en", "ru", "ja", "ko"].contains(&l.as_str())),
            "帮助语言无效"
        );
        ensure!(
            !self.urls.is_empty() && self.urls.len() <= 3,
            "帮助下载地址无效"
        );
        for url in &self.urls {
            transfer::validate_url(url, transfer::UrlPolicy::Official)?;
        }
        ensure!(
            self.files.len() == FILES.len()
                && FILES
                    .iter()
                    .all(|(name, _)| self.files.get(*name).is_some_and(|h| core::valid_hash(h))),
            "帮助文件清单无效"
        );
        Ok(())
    }
}
fn core_version(value: &str) -> Result<Vec<u32>> {
    ensure!(
        value.len() <= 40 && (3..=4).contains(&value.split('.').count()),
        "帮助版本无效"
    );
    ensure!(
        value
            .split('.')
            .all(|p| !p.is_empty() && p.bytes().all(|c| c.is_ascii_digit())),
        "帮助版本无效"
    );
    let mut parts: Vec<u32> = value
        .split('.')
        .map(|part| part.parse().context("帮助版本无效"))
        .collect::<Result<_>>()?;
    parts.resize(4, 0);
    Ok(parts)
}
fn root() -> Result<PathBuf> {
    Ok(assets::cache_root()?.join("help"))
}
fn validate_directory(directory: &Path, manifest: &Manifest) -> Result<()> {
    for (name, hash) in &manifest.files {
        let path = core::no_links(&directory.join(name))?;
        ensure!(
            path.is_file() && path.metadata()?.len() <= LIMIT && core::digest(&path)? == *hash,
            "帮助缓存损坏"
        );
    }
    Ok(())
}
pub fn bundled_at(root: &Path) -> Result<PathBuf> {
    core::no_links(root)?;
    fs::create_dir_all(root)?;
    let identity = core::hash(
        &FILES
            .iter()
            .flat_map(|(_, b)| b.iter().copied())
            .collect::<Vec<_>>(),
    );
    let directory = root.join(format!("bundled-{}", &identity[..20]));
    core::no_links(&directory)?;
    fs::create_dir_all(&directory)?;
    for (name, bytes) in FILES {
        let path = core::no_links(&directory.join(name))?;
        if path.is_file() && core::digest(&path)? == core::hash(bytes) {
            continue;
        }
        use std::io::Write;
        let mut temp = tempfile::NamedTempFile::new_in(&directory)?;
        temp.write_all(bytes)?;
        temp.as_file().sync_all()?;
        temp.persist(&path)?;
    }
    Ok(directory.join("index.html"))
}
pub fn current() -> Result<PathBuf> {
    current_at(&root()?)
}
fn current_at(root: &Path) -> Result<PathBuf> {
    if let Ok(value) = core::read_json(&root.join("current.json"), 32768)
        && let Ok(manifest) = serde_json::from_value::<Manifest>(value)
        && manifest.validate().is_ok()
    {
        let directory = root.join(&manifest.sha256);
        if validate_directory(&directory, &manifest).is_ok() {
            return Ok(directory.join("index.html"));
        }
    }
    bundled_at(root)
}
fn commit_pointer(root: &Path, manifest: &Manifest) -> Result<()> {
    if let Ok(value) = core::read_json(&root.join("current.json"), 32768)
        && let Ok(old) = serde_json::from_value::<Manifest>(value)
    {
        ensure!(
            core_version(&manifest.version)? >= core_version(&old.version)?,
            "不回退帮助内容"
        );
        ensure!(
            old.version != manifest.version || old.sha256 == manifest.sha256,
            "同版本帮助内容发生变化"
        );
    }
    core::atomic_json(&root.join("current.json"), manifest)
}
pub fn install_at(root: &Path, manifest: &Manifest, archive: &[u8]) -> Result<()> {
    manifest.validate()?;
    ensure!(
        archive.len() as u64 == manifest.bytes && core::hash(archive) == manifest.sha256,
        "帮助包校验失败"
    );
    core::no_links(root)?;
    fs::create_dir_all(root)?;
    let _lock = crate::win::resource_lock(root)?;
    if let Ok(value) = core::read_json(&root.join("current.json"), 32768)
        && let Ok(old) = serde_json::from_value::<Manifest>(value)
    {
        ensure!(
            core_version(&manifest.version)? >= core_version(&old.version)?,
            "不回退帮助内容"
        );
        ensure!(
            old.version != manifest.version || old.sha256 == manifest.sha256,
            "同版本帮助内容发生变化"
        );
    }
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(archive))?;
    ensure!(zip.len() == FILES.len(), "帮助包文件范围无效");
    let stage = tempfile::Builder::new()
        .prefix("help-stage-")
        .tempdir_in(root)?;
    let mut seen = std::collections::BTreeSet::new();
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?;
        let name = entry.name().to_owned();
        let hash = manifest.files.get(&name).context("帮助包含未知文件")?;
        ensure!(
            seen.insert(name.clone())
                && !entry.is_dir()
                && entry.size() <= LIMIT
                && entry.unix_mode().is_none_or(|m| m & 0o170000 != 0o120000),
            "帮助包路径无效"
        );
        let mut bytes = Vec::new();
        (&mut entry).take(LIMIT + 1).read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() as u64 <= LIMIT && core::hash(&bytes) == *hash,
            "帮助文件校验失败"
        );
        core::write_new(&stage.path().join(name), &bytes)?;
    }
    let destination = core::no_links(&root.join(&manifest.sha256))?;
    if destination.exists() {
        validate_directory(&destination, manifest)?;
    } else {
        fs::rename(stage.path(), &destination)?;
    }
    // Switching the pointer is the sole commit point. The previously displayed
    // directory remains immutable, including while a help window is open.
    commit_pointer(root, manifest)?;
    Ok(())
}
pub fn check_update() -> Result<()> {
    let cancel = AtomicBool::new(false);
    let raw = transfer::fetch_metadata(vec![
        transfer::Source { name:"gitcode".into(),url:"https://api.gitcode.com/api/v5/repos/gcw_qWIjmve7/RTX-FG-Manager/raw/cloud/help.json?ref=main".into() },
        transfer::Source { name:"github".into(),url:"https://raw.githubusercontent.com/pandaligx/RTX-FG-Manager/main/cloud/help.json".into() },
    ],transfer::UrlPolicy::Official,&cancel, |_|{})?;
    let manifest: Manifest = serde_json::from_slice(&raw)?;
    manifest.validate()?;
    let root = root()?;
    core::no_links(&root)?;
    fs::create_dir_all(&root)?;
    if validate_directory(&root.join(&manifest.sha256), &manifest).is_ok() {
        let _lock = crate::win::resource_lock(&root)?;
        return commit_pointer(&root, &manifest);
    }
    let stage = tempfile::tempdir_in(&root)?;
    let request = transfer::Request {
        sources: manifest
            .urls
            .iter()
            .enumerate()
            .map(|(i, url)| transfer::Source {
                name: format!("help-{}", i + 1),
                url: url.clone(),
            })
            .collect(),
        bytes: Some(manifest.bytes),
        sha256: Some(manifest.sha256.clone()),
        limit: LIMIT,
        metadata: false,
        policy: transfer::UrlPolicy::Official,
    };
    transfer::download(&request, &stage.path().join("help.zip"), &cancel, |_| {})?;
    install_at(&root, &manifest, &fs::read(stage.path().join("help.zip"))?)
}
fn content_language(language: &str) -> &'static str {
    match language {
        "zh" | "zh-CN" => "zh",
        "ru" => "ru",
        "ja" => "ja",
        "ko" => "ko",
        _ => "en",
    }
}
pub fn open(language: &str, dark: bool, topic: &str) -> Result<()> {
    let language = content_language(language).to_owned();
    let topic = if [
        "start",
        "patches",
        "menus",
        "community",
        "maintenance",
        "faq",
    ]
    .contains(&topic)
    {
        topic
    } else {
        "start"
    }
    .to_owned();
    std::thread::Builder::new()
        .name("help-window".into())
        .spawn(move || {
            if let Ok(path) = current()
                && let Ok(mut url) = reqwest::Url::from_file_path(&path)
            {
                url.query_pairs_mut()
                    .append_pair("lang", &language)
                    .append_pair("theme", if dark { "dark" } else { "light" })
                    .append_pair("version", crate::VERSION)
                    .append_pair("topic", &topic);
                let text = url.to_string();
                if crate::help_window::run(&text).is_err() {
                    let _ = crate::win::open(&text);
                }
            }
        })?;
    if UPDATING
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
    {
        std::thread::spawn(|| {
            let _ = check_update();
            UPDATING.store(false, Ordering::Release);
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn manager_language_codes_select_matching_help() {
        assert_eq!(content_language("zh-CN"), "zh");
        for lang in ["en", "ru", "ja", "ko"] {
            assert_eq!(content_language(lang), lang);
        }
        assert_eq!(content_language("unknown"), "en");
    }
    use std::io::{Cursor, Write};
    fn bundle(version: &str, unexpected: bool) -> (Manifest, Vec<u8>) {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let mut files = BTreeMap::new();
        for (name, content) in FILES {
            writer
                .start_file(name, zip::write::FileOptions::default())
                .unwrap();
            writer.write_all(content).unwrap();
            files.insert(name.into(), core::hash(content));
        }
        if unexpected {
            writer
                .start_file("../escape.js", zip::write::FileOptions::default())
                .unwrap();
            writer.write_all(b"bad").unwrap();
        }
        let bytes = writer.finish().unwrap().into_inner();
        let manifest = Manifest {
            schema: 1,
            version: version.into(),
            min_client: "4.2.0".into(),
            max_client: None,
            languages: vec!["zh".into()],
            urls: vec![
                "https://github.com/pandaligx/RTX-FG-Manager/releases/download/help/help.zip"
                    .into(),
            ],
            bytes: bytes.len() as u64,
            sha256: core::hash(&bytes),
            files,
        };
        (manifest, bytes)
    }
    #[test]
    fn update_is_atomic_and_corrupt_download_keeps_previous_help() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        assert!(current_at(root).unwrap().is_file());
        let (first, bytes) = bundle("1.0.0", false);
        install_at(root, &first, &bytes).unwrap();
        let previous = current_at(root).unwrap();
        let pointer = fs::read(root.join("current.json")).unwrap();
        let (next, mut broken) = bundle("1.0.1", false);
        broken[0] ^= 1;
        assert!(install_at(root, &next, &broken).is_err());
        assert_eq!(fs::read(root.join("current.json")).unwrap(), pointer);
        assert_eq!(current_at(root).unwrap(), previous);
        let (escape, bytes) = bundle("1.0.1", true);
        assert!(install_at(root, &escape, &bytes).is_err());
        assert!(!root.join("escape.js").exists());
        let (older, bytes) = bundle("0.9.0", false);
        assert!(install_at(root, &older, &bytes).is_err());
        assert_eq!(fs::read(root.join("current.json")).unwrap(), pointer);
        fs::write(previous.parent().unwrap().join("app.js"), b"changed").unwrap();
        assert!(
            current_at(root)
                .unwrap()
                .to_string_lossy()
                .contains("bundled-")
        );
    }
    #[test]
    fn help_manifest_rejects_incompatible_client_and_foreign_hosts() {
        let (mut manifest, _) = bundle("1.0.0", false);
        manifest.min_client = "99.0.0".into();
        assert!(manifest.validate().is_err());
        manifest.min_client = "4.2.0".into();
        manifest.urls[0] = "https://untrusted.example/help.zip".into();
        assert!(manifest.validate().is_err());
        assert_eq!(
            core_version("1.0.0").unwrap(),
            core_version("1.0.0.0").unwrap()
        );
        assert!(core_version("1.+0.0").is_err());
    }
}
