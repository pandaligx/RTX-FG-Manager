//! Explicit, previewed removal of per-game leftovers. Never accepts UI paths.
use crate::{core, delta, win};
use anyhow::{Context, Result, ensure};
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    os::windows::{fs::OpenOptionsExt, io::AsRawHandle},
    path::{Path, PathBuf},
};
use windows::Win32::{
    Foundation::HANDLE,
    Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, FILE_DISPOSITION_INFO, FileDispositionInfo,
        GetFileInformationByHandle, SetFileInformationByHandle,
    },
};

#[derive(Clone, Debug, PartialEq, Eq)]
struct Entry {
    path: PathBuf,
    directory: bool,
    identity: (u32, u32, u32),
    content: (u32, u32, u32, u32),
}
#[derive(Clone, Debug)]
pub struct Plan {
    target: core::DeploymentTarget,
    cache_root: Option<PathBuf>,
    record_hash: String,
    roots: Vec<PathBuf>,
    entries: Vec<Entry>,
}
impl Plan {
    pub fn target(&self) -> &core::DeploymentTarget {
        &self.target
    }
    /// Show every file, not just a broad directory deletion promise.
    pub fn paths(&self) -> impl Iterator<Item = &Path> {
        self.entries.iter().map(|entry| entry.path.as_path())
    }
}
fn marker(target: &core::DeploymentTarget) -> PathBuf {
    target.directory.join(core::OWN).join(core::MARKER)
}
fn roots(target: &core::DeploymentTarget, cache_root: Option<&Path>) -> Result<Vec<PathBuf>> {
    let record =
        core::record(&target.directory)?.context("清理记录已不存在，请重新清理更新状态")?;
    target.validate_record(&record)?;
    ensure!(
        record.cache_pending,
        "没有待处理的卸载记录，请先正常卸载补丁"
    );
    let mut paths = Vec::new();
    let id = delta::cache_id(&target.game_exe);
    if delta::is_game(&target.game_exe) && record.delta_cache_ids.contains(&id) {
        let path = core::no_links(
            &cache_root
                .context("无法定位组件缓存目录")?
                .join("games")
                .join(id),
        )?;
        if path.try_exists()? {
            ensure!(path.is_dir(), "组件缓存不是普通目录");
            paths.push(path);
        }
    }
    // Only recorded installation fragments. Backups, originals, configuration,
    // legacy shared caches and another game's ID are deliberately ineligible.
    for name in record.hashes.keys() {
        let path = core::no_links(
            &target
                .directory
                .join(core::OWN)
                .join(core::stage_name(name)),
        )?;
        if path.try_exists()? {
            ensure!(path.is_file(), "安装临时文件不是普通文件");
            paths.push(path);
        }
    }
    for &name in crate::cleanup::private_components(&record) {
        if record
            .hashes
            .get(name)
            .is_some_and(|hash| *hash != core::hash(b""))
        {
            let path = core::no_links(&target.directory.join(name))?;
            if path.try_exists()? {
                ensure!(path.is_file(), "配套文件不是普通文件");
                paths.push(path);
            }
        }
    }
    paths.sort();
    paths.dedup();
    Ok(paths)
}
fn open(path: &Path, deleting: bool) -> Result<File> {
    core::no_links(path)?;
    let directory = path.is_dir();
    let file = OpenOptions::new()
        // FILE_READ_ATTRIBUTES, plus DELETE for the already-confirmed snapshot.
        .access_mode(if deleting { 0x10080 } else { 0x80 })
        .share_mode(if deleting {
            if directory { 3 } else { 1 }
        } else {
            7
        })
        // Inspect the reparse point itself even if one appears after no_links.
        .custom_flags(0x0220_0000)
        .open(path)
        .with_context(|| format!("文件被占用或无法访问：{}", path.display()))?;
    let _ = identity(path, &file)?;
    Ok(file)
}
fn identity(path: &Path, file: &File) -> Result<Entry> {
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: File owns a live handle; info is a correctly sized writable struct.
    unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info)? };
    ensure!(
        info.dwFileAttributes & 0x400 == 0,
        "不删除链接或重解析点：{}",
        path.display()
    );
    let directory = info.dwFileAttributes & 0x10 != 0;
    Ok(Entry {
        path: path.into(),
        directory,
        identity: (
            info.dwVolumeSerialNumber,
            info.nFileIndexHigh,
            info.nFileIndexLow,
        ),
        content: if directory {
            (0, 0, 0, 0)
        } else {
            (
                info.nFileSizeHigh,
                info.nFileSizeLow,
                info.ftLastWriteTime.dwHighDateTime,
                info.ftLastWriteTime.dwLowDateTime,
            )
        },
    })
}
fn collect(
    path: &Path,
    deleting: bool,
    depth: usize,
    opened: &mut Vec<(Entry, File)>,
) -> Result<()> {
    ensure!(
        depth <= 16 && opened.len() < 4096,
        "待清理内容过多，请打开位置核对"
    );
    let file = open(path, deleting)?;
    let entry = identity(path, &file)?;
    let directory = entry.directory;
    opened.push((entry, file));
    if directory {
        let mut children = fs::read_dir(path)?
            .map(|v| v.map(|e| e.path()))
            .take(4097)
            .collect::<std::io::Result<Vec<_>>>()?;
        ensure!(children.len() <= 4096, "待清理内容过多，请打开位置核对");
        children.sort();
        for child in children {
            collect(&child, deleting, depth + 1, opened)?;
        }
    }
    Ok(())
}
pub fn prepare(target: &core::DeploymentTarget) -> Result<Plan> {
    prepare_at(target, delta::root().ok().as_deref())
}
/// Explicit root is also used by isolated filesystem regression tests.
pub fn prepare_at(target: &core::DeploymentTarget, cache_root: Option<&Path>) -> Result<Plan> {
    let target = target.validate(false)?;
    let hash = core::digest(&marker(&target))?;
    let roots = roots(&target, cache_root)?;
    let mut opened = Vec::new();
    for root in &roots {
        collect(root, false, 0, &mut opened)?;
    }
    ensure!(
        core::digest(&marker(&target))? == hash,
        "清理记录已变化，请重新查看文件"
    );
    Ok(Plan {
        target,
        cache_root: cache_root.map(Path::to_path_buf),
        record_hash: hash,
        roots,
        entries: opened.into_iter().map(|(entry, _)| entry).collect(),
    })
}
pub fn delete_confirmed(plan: &Plan) -> Result<()> {
    let target = plan.target.validate(false)?;
    let _game_lock = win::game_lock(&target.directory)?;
    core::assert_target_stopped(&target)?;
    let marker_path = marker(&target);
    // Pin every ancestor against rename/replacement. Leaf handles below own
    // deletion; no path-based recursive remove can cross a replaced junction.
    let mut ancestors = BTreeSet::new();
    for path in plan.roots.iter().chain(std::iter::once(&marker_path)) {
        ancestors.extend(path.ancestors().skip(1).map(Path::to_path_buf));
    }
    let mut pins = Vec::new();
    for path in ancestors {
        let file = OpenOptions::new()
            .access_mode(0x80)
            .share_mode(3)
            .custom_flags(0x0220_0000)
            .open(&path)?;
        ensure!(identity(&path, &file)?.directory, "清理目录已变化");
        pins.push(file);
    }
    let _record_pin = OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(&marker_path)?;
    ensure!(
        core::digest(&marker_path)? == plan.record_hash,
        "清理记录已变化，请重新查看文件"
    );
    ensure!(
        roots(&target, plan.cache_root.as_deref())? == plan.roots,
        "待清理位置已变化，请重新查看文件"
    );
    let mut opened = Vec::new();
    for root in &plan.roots {
        collect(root, true, 0, &mut opened)?;
    }
    ensure!(
        opened
            .iter()
            .map(|(entry, _)| entry)
            .eq(plan.entries.iter()),
        "待清理文件已变化，请重新查看文件后确认"
    );
    // Preflight all identities/access before the first deletion, then children
    // before parents. New files are not enumerated again and never get deleted.
    while let Some((entry, file)) = opened.pop() {
        let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
        // SAFETY: File owns the validated live handle and disposition matches
        // FileDispositionInfo. Deletion follows this handle, not a mutable path.
        unsafe {
            SetFileInformationByHandle(
                HANDLE(file.as_raw_handle()),
                FileDispositionInfo,
                &disposition as *const _ as *const _,
                std::mem::size_of_val(&disposition) as u32,
            )
        }
        .with_context(|| format!("删除未完成，请退出占用程序后重试：{}", entry.path.display()))?;
        drop(file);
    }
    Ok(())
}
