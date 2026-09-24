//! Ownership of the opt-in Delta Force runtime, never of original game plugins.
use crate::{core, win};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    os::windows::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    sync::OnceLock,
};

pub const CAPABILITY: &str = "delta_force_mfg_v1";
pub const GAME: &str = "DeltaForceClient-Win64-Shipping.exe";
pub const REVISION: &str = "Streamline-2.7.32-c39569d4a2a70434";
pub const ID_KEY: &str = "DeltaForceRuntimeId";
const OWNER: &str = "owner.json";

#[derive(Deserialize)]
struct Manifest {
    files: BTreeMap<String, String>,
    proxy_images: Vec<String>,
}
fn manifest() -> &'static Manifest {
    static VALUE: OnceLock<Manifest> = OnceLock::new();
    VALUE.get_or_init(|| {
        serde_json::from_str(include_str!("../delta-runtime.json"))
            .expect("bundled runtime ownership manifest")
    })
}
pub fn known_image(image: &str) -> bool {
    manifest().proxy_images.iter().any(|v| v == image)
}
pub fn is_game(exe: &Path) -> bool {
    exe.file_name()
        .is_some_and(|n| n.eq_ignore_ascii_case(GAME))
        && exe
            .parent()
            .and_then(Path::file_name)
            .is_some_and(|n| n.eq_ignore_ascii_case("Win64"))
        && exe
            .parent()
            .and_then(Path::parent)
            .and_then(Path::file_name)
            .is_some_and(|n| n.eq_ignore_ascii_case("Binaries"))
}
pub fn valid_id(id: &str) -> bool {
    id.len() == 32
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
pub fn cache_id(exe: &Path) -> String {
    core::hash(core::key(exe).as_bytes())[..32].into()
}
pub fn root() -> Result<PathBuf> {
    let local = std::env::var_os("LOCALAPPDATA").context("无法定位当前用户的组件缓存目录")?;
    core::no_links(&PathBuf::from(local).join("RTXFG-Delta4X"))
}
#[derive(Serialize, Deserialize)]
struct Owner {
    schema: u8,
    exe: PathBuf,
    id: String,
}
pub fn register_at(root: &Path, exe: &Path, id: &str) -> Result<()> {
    ensure!(
        is_game(exe) && valid_id(id) && cache_id(exe) == id,
        "组件缓存归属无效"
    );
    let folder = core::no_links(&root.join("games").join(id))?;
    let owner = folder.join(OWNER);
    if owner.exists() {
        read_owner(&folder)?;
    }
    core::atomic_json(
        &owner,
        &Owner {
            schema: 1,
            exe: exe.into(),
            id: id.into(),
        },
    )
}
fn read_owner(folder: &Path) -> Result<Owner> {
    let owner: Owner = serde_json::from_value(core::read_json(&folder.join(OWNER), 8192)?)?;
    ensure!(
        owner.schema == 1
            && valid_id(&owner.id)
            && owner.exe.is_absolute()
            && is_game(&owner.exe)
            && owner.id == cache_id(&owner.exe)
            && folder.file_name().is_some_and(|n| n == owner.id.as_str()),
        "组件缓存归属无效"
    );
    Ok(owner)
}
#[derive(Default, Debug)]
pub struct Report {
    pub files: u64,
    pub bytes: u64,
    pub pending: Vec<String>,
}
impl Report {
    fn merge(&mut self, other: Self) {
        self.files += other.files;
        self.bytes += other.bytes;
        self.pending.extend(other.pending);
    }
    fn defer(&mut self, path: &Path, error: impl std::fmt::Display) {
        self.pending.push(format!("{}：{error}", path.display()));
    }
}
fn recognized_component(path: &Path) -> Result<bool> {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let base = if let Some((base, suffix)) = name.split_once(".tmp-") {
        // Only complete, byte-identical runtime writes. Partial/unknown files
        // stay visible for manual review; a filename alone proves no ownership.
        let fields: Vec<_> = suffix.split('-').collect();
        if fields.len() != 3
            || fields
                .iter()
                .any(|s| s.is_empty() || s.len() > 20 || !s.bytes().all(|b| b.is_ascii_digit()))
        {
            return Ok(false);
        }
        base
    } else {
        name.as_ref()
    };
    Ok(path.metadata()?.len() <= 8 * 1024 * 1024
        && manifest()
            .files
            .get(base)
            .is_some_and(|expected| core::digest(path).is_ok_and(|actual| &actual == expected)))
}
/// Acquire DELETE access before deleting any member. A running proxy holds
/// FILE_SHARE_READ only, so its complete runtime survives a concurrent cleanup.
fn clean_revision(folder: &Path) -> Report {
    let mut report = Report::default();
    let result = (|| -> Result<()> {
        core::no_links(folder)?;
        if !folder.exists() {
            return Ok(());
        }
        ensure!(folder.is_dir(), "组件缓存不是普通目录");
        let mut opened: Vec<(PathBuf, File)> = Vec::new();
        for item in fs::read_dir(folder)?.take(128) {
            let path = core::no_links(&item?.path())?;
            if !path.is_file() || !recognized_component(&path)? {
                report.defer(&path, "保留未知文件");
                continue;
            }
            // GENERIC_READ | DELETE; no write sharing. Files cannot change
            // between identity verification and removal while these handles live.
            let file = OpenOptions::new()
                .access_mode(0x8001_0000)
                .share_mode(0x5)
                .open(&path)
                .with_context(|| format!("组件被占用或不可删除：{}", path.display()))?;
            ensure!(recognized_component(&path)?, "组件内容已变化");
            opened.push((path, file));
        }
        for (path, file) in opened {
            let size = file.metadata()?.len();
            fs::remove_file(&path)?;
            drop(file);
            report.files += 1;
            report.bytes += size;
        }
        if fs::read_dir(folder)?.next().is_none() {
            fs::remove_dir(folder)?;
        }
        Ok(())
    })();
    if let Err(e) = result {
        report.defer(folder, e);
    }
    report
}
pub fn clean_at(root: &Path, exe: &Path, ids: &[String], legacy: bool) -> Report {
    let mut report = Report::default();
    for id in ids {
        let result = (|| -> Result<()> {
            ensure!(
                is_game(exe) && valid_id(id) && id == &cache_id(exe),
                "组件缓存归属无效"
            );
            let folder = core::no_links(&root.join("games").join(id))?;
            if !folder.exists() {
                return Ok(());
            }
            if folder.join(OWNER).exists() {
                read_owner(&folder)?;
            }
            // The deployment record also proves ownership if owner.json is lost.
            let _lock = win::game_lock(&folder)?;
            let result = clean_revision(&folder.join(REVISION));
            let complete = result.pending.is_empty();
            report.merge(result);
            if complete {
                let other = fs::read_dir(&folder)?
                    .filter_map(std::result::Result::ok)
                    .any(|e| e.file_name() != OWNER);
                ensure!(!other, "保留未知缓存目录内容");
                if folder.join(OWNER).is_file() {
                    fs::remove_file(folder.join(OWNER))?;
                }
                fs::remove_dir(folder)?;
            }
            Ok(())
        })();
        if let Err(e) = result {
            report.defer(&root.join("games").join(id), e);
        }
    }
    if legacy {
        report.merge(clean_revision(&root.join(REVISION)));
    }
    report
}
pub fn clean_game(exe: &Path, ids: &[String], legacy: bool) -> Result<Report> {
    if ids.is_empty() && !legacy {
        return Ok(Report::default());
    }
    Ok(clean_at(&root()?, exe, ids, legacy))
}
pub fn clear_available() -> Result<Report> {
    let root = root()?;
    let games = core::no_links(&root.join("games"))?;
    let mut report = Report::default();
    if games.is_dir() {
        for item in fs::read_dir(games)?.take(10_000) {
            let path = item?.path();
            let result = (|| -> Result<()> {
                core::no_links(&path)?;
                let owner = read_owner(&path)?;
                if owner.exe.parent().is_some_and(Path::is_dir) {
                    core::assert_stopped(&owner.exe)?;
                }
                // Clear-cache keeps the owner so subsequent lazy extraction
                // remains discoverable. Uninstall removes it with its record.
                let _lock = win::game_lock(&path)?;
                report.merge(clean_revision(&path.join(REVISION)));
                Ok(())
            })();
            if let Err(e) = result {
                report.defer(&path, e);
            }
        }
    }
    // Only this exact legacy runtime revision; hashed old installer backups are
    // outside this operation. Held runtime handles prevent deletion while in use.
    report.merge(clean_revision(&root.join(REVISION)));
    Ok(report)
}
