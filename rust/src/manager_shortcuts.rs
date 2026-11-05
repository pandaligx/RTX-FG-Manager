//! Retarget only direct Shell links to this manager; never resolve or launch links.
use crate::core;
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use std::{
    collections::BTreeSet,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};
use windows::{
    Win32::{System::Com::*, UI::Shell::*},
    core::{Interface, PCWSTR},
};

const LIMIT: u64 = 128 * 1024;
struct Apartment;
impl Apartment {
    fn new() -> Result<Self> {
        // SAFETY: This guard stays on its creating thread and pairs every success.
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
        }
        Ok(Self)
    }
}
impl Drop for Apartment {
    fn drop(&mut self) {
        // SAFETY: Paired with the successful initialization on this thread.
        unsafe {
            CoUninitialize();
        }
    }
}
fn wide(path: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}
fn text(buffer: &[u16]) -> Result<String> {
    let end = buffer
        .iter()
        .position(|v| *v == 0)
        .context("快捷方式路径过长")?;
    Ok(String::from_utf16(&buffer[..end])?)
}
fn read(path: &Path) -> Result<Vec<u8>> {
    let mut file = fs::File::open(core::no_links(path)?)?;
    ensure!(
        file.metadata()?.is_file() && file.metadata()?.len() <= LIMIT,
        "快捷方式文件过大"
    );
    let mut bytes = Vec::new();
    (&mut file).take(LIMIT + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= LIMIT, "快捷方式文件过大");
    Ok(bytes)
}

fn same_target(a: &Path, b: &Path) -> bool {
    if core::key(a) == core::key(b) {
        return true;
    }
    if !a
        .file_name()
        .zip(b.file_name())
        .is_some_and(|(x, y)| x.eq_ignore_ascii_case(y))
    {
        return false;
    }
    let drive = |p: &Path| match p.components().next() {
        Some(std::path::Component::Prefix(prefix)) => match prefix.kind() {
            std::path::Prefix::Disk(d) | std::path::Prefix::VerbatimDisk(d) => {
                Some(d.to_ascii_lowercase())
            }
            _ => None,
        },
        _ => None,
    };
    // ShellLink expands existing 8.3 parent names. Normalize only same-drive,
    // same-basename candidates; never probe an unrelated UNC/network target.
    if drive(a).is_none() || drive(a) != drive(b) {
        return false;
    }
    let parent = |p: &Path| {
        p.parent()
            .and_then(|p| core::no_links(p).ok())
            .and_then(|p| fs::canonicalize(p).ok())
            .map(|p| core::key(&p))
    };
    parent(a).zip(parent(b)).is_some_and(|(a, b)| a == b)
}

pub(crate) fn roots() -> Result<Vec<(PathBuf, usize)>> {
    let mut roots = Vec::new();
    for (id, depth) in [
        (FOLDERID_Desktop, 0),
        (FOLDERID_PublicDesktop, 0),
        (FOLDERID_StartMenu, 8),
        (FOLDERID_CommonStartMenu, 8),
        (FOLDERID_QuickLaunch, 3),
    ] {
        // SAFETY: Windows owns the result until CoTaskMemFree; copy before freeing.
        let path = unsafe {
            let raw = SHGetKnownFolderPath(&id, KF_FLAG_DONT_VERIFY, None)?;
            let value = raw.to_string();
            CoTaskMemFree(Some(raw.as_ptr().cast()));
            PathBuf::from(value?)
        };
        if path.exists() {
            roots.push((path, depth));
        }
    }
    Ok(roots)
}

#[derive(Serialize)]
struct Edit {
    path: PathBuf,
    before: Vec<u8>,
    after: Vec<u8>,
}
pub(crate) struct Plan(Vec<Edit>);
impl Plan {
    pub(crate) fn inspect(
        target: &Path,
        destination: &Path,
        roots: &[(PathBuf, usize)],
    ) -> Result<Self> {
        let _apartment = Apartment::new()?;
        let mut edits = Vec::new();
        let mut seen = BTreeSet::new();
        let mut pending = roots.to_vec();
        let mut count = 0;
        while let Some((directory, depth)) = pending.pop() {
            let directory = core::no_links(&directory)?;
            if !seen.insert(core::key(&directory)) {
                continue;
            }
            for entry in fs::read_dir(&directory)? {
                let entry = entry?;
                count += 1;
                ensure!(count <= 10000, "快捷方式目录过大，未改名");
                let path = entry.path();
                let metadata = fs::symlink_metadata(&path)?;
                use std::os::windows::fs::MetadataExt;
                if metadata.file_attributes() & 0x400 != 0 {
                    continue;
                }
                if metadata.is_dir() {
                    // Never descend into desktop documents, games or checkouts.
                    if depth > 0 {
                        pending.push((path, depth - 1));
                    }
                } else if path
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("lnk"))
                    && metadata.len() <= LIMIT
                {
                    if let Some(edit) = inspect_link(&path, target, destination)? {
                        edits.push(edit);
                    }
                    ensure!(edits.len() <= 256, "匹配的快捷方式过多，未改名");
                }
            }
        }
        Ok(Self(edits))
    }

    pub(crate) fn save_backup(&self, folder: &Path) -> Result<()> {
        core::atomic_json(
            &folder.join("shortcut-backup.json"),
            &serde_json::to_value(&self.0)?,
        )
    }

    /// A failed shortcut edit or launch restores every already-written link.
    pub(crate) fn with_changes(&self, launch: impl FnOnce() -> Result<()>) -> Result<()> {
        let mut written = 0;
        let result = (|| {
            for edit in &self.0 {
                replace(&edit.path, &edit.before, &edit.after)?;
                written += 1;
            }
            launch()
        })();
        if let Err(error) = result {
            let mut failures = Vec::new();
            for edit in self.0[..written].iter().rev() {
                if let Err(e) = replace(&edit.path, &edit.after, &edit.before) {
                    failures.push(format!("{}：{e}", edit.path.display()));
                }
            }
            ensure!(
                failures.is_empty(),
                "{error}；部分快捷方式已被占用或修改，原件备份保留：{}",
                failures.join("；")
            );
            return Err(error);
        }
        Ok(())
    }
}

fn inspect_link(path: &Path, target: &Path, destination: &Path) -> Result<Option<Edit>> {
    let before = read(path)?;
    // SAFETY: In-process ShellLink parser, initialized COM thread, bounded owned
    // UTF-16 buffers. Do not call Resolve (no network lookup or target execution).
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
        let file: IPersistFile = link.cast()?;
        if file.Load(PCWSTR(wide(path).as_ptr()), STGM_READ).is_err() {
            return Ok(None);
        }
        let mut buffer = [0u16; 32768];
        if link
            .GetPath(&mut buffer, std::ptr::null_mut(), SLGP_RAWPATH.0 as u32)
            .is_err()
        {
            return Ok(None);
        }
        let original = PathBuf::from(text(&buffer)?);
        if !same_target(&original, target) {
            return Ok(None);
        }
        link.SetPath(PCWSTR(wide(destination).as_ptr()))?;
        buffer.fill(0);
        let mut index = 0;
        link.GetIconLocation(&mut buffer, &mut index)?;
        if same_target(Path::new(&text(&buffer)?), target) {
            link.SetIconLocation(PCWSTR(wide(destination).as_ptr()), index)?;
        }
        // Save a copy preserves arguments, working directory and custom metadata.
        let temp = tempfile::NamedTempFile::new_in(path.parent().context("快捷方式路径无效")?)?;
        file.Save(PCWSTR(wide(temp.path()).as_ptr()), false)?;
        let after = read(temp.path())?;
        ensure!(
            read(path)? == before,
            "快捷方式已变化，未改名：{}",
            path.display()
        );
        Ok(Some(Edit {
            path: path.to_owned(),
            before,
            after,
        }))
    }
}

fn replace(path: &Path, expected: &[u8], next: &[u8]) -> Result<()> {
    ensure!(
        read(path)? == expected,
        "快捷方式已变化：{}",
        path.display()
    );
    let mut temp = tempfile::NamedTempFile::new_in(path.parent().context("快捷方式路径无效")?)?;
    temp.write_all(next)?;
    temp.as_file().sync_all()?;
    ensure!(
        read(path)? == expected,
        "快捷方式已变化：{}",
        path.display()
    );
    temp.persist(path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn create(path: &Path, target: &Path) -> Result<()> {
        // SAFETY: Tests initialize COM; inputs are owned and NUL-terminated.
        unsafe {
            let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
            link.SetPath(PCWSTR(wide(target).as_ptr()))?;
            link.SetIconLocation(PCWSTR(wide(target).as_ptr()), 7)?;
            link.SetArguments(windows::core::w!("--data-dir \"C:\\中文配置\""))?;
            link.SetDescription(windows::core::w!("Keep my shortcut"))?;
            let file: IPersistFile = link.cast()?;
            file.Save(PCWSTR(wide(path).as_ptr()), false)?;
        }
        Ok(())
    }
    #[test]
    fn real_shell_links_retarget_only_exact_exe_and_keep_arguments() -> Result<()> {
        let _apartment = Apartment::new()?;
        let dir = tempfile::tempdir()?;
        let target = dir.path().join("RTXManager-v4.3.18-x64.exe");
        let next = dir.path().join("RTXManager.exe");
        fs::write(&target, b"fixture")?;
        let link_path = dir.path().join("我的管理器.lnk");
        create(&link_path, &target)?;
        let other = dir.path().join("other.lnk");
        create(&other, &dir.path().join("another.exe"))?;
        let unrelated = read(&other)?;
        let plan = Plan::inspect(&target, &next, &[(dir.path().to_owned(), 0)])?;
        assert_eq!(plan.0.len(), 1);
        plan.with_changes(|| Ok(()))?;
        assert_eq!(read(&other)?, unrelated);
        assert!(
            Plan::inspect(&target, &next, &[(dir.path().to_owned(), 0)])?
                .0
                .is_empty()
        );
        assert_eq!(
            Plan::inspect(&next, &target, &[(dir.path().to_owned(), 0)])?
                .0
                .len(),
            1
        );
        // SAFETY: COM is initialized; stack buffers stay valid for all calls.
        unsafe {
            let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
            let file: IPersistFile = link.cast()?;
            file.Load(PCWSTR(wide(&link_path).as_ptr()), STGM_READ)?;
            let mut buffer = [0u16; 4096];
            link.GetArguments(&mut buffer)?;
            assert_eq!(text(&buffer)?, "--data-dir \"C:\\中文配置\"");
            link.GetDescription(&mut buffer)?;
            assert_eq!(text(&buffer)?, "Keep my shortcut");
            let mut index = 0;
            link.GetIconLocation(&mut buffer, &mut index)?;
            assert!(same_target(Path::new(&text(&buffer)?), &next));
            assert_eq!(index, 7);
        }
        Ok(())
    }
    #[test]
    fn failed_launch_restores_original_shortcut_bytes() -> Result<()> {
        let _apartment = Apartment::new()?;
        let dir = tempfile::tempdir()?;
        let target = dir.path().join("old.exe");
        let path = dir.path().join("manager.lnk");
        create(&path, &target)?;
        let before = read(&path)?;
        let plan = Plan::inspect(
            &target,
            &dir.path().join("RTXManager.exe"),
            &[(dir.path().to_owned(), 0)],
        )?;
        assert!(
            plan.with_changes(|| anyhow::bail!("injected launch failure"))
                .is_err()
        );
        assert_eq!(read(&path)?, before);
        Ok(())
    }

    #[test]
    fn desktop_scan_stays_out_of_game_and_document_subdirectories() -> Result<()> {
        let _apartment = Apartment::new()?;
        let dir = tempfile::tempdir()?;
        let sub = dir.path().join("Documents");
        fs::create_dir(&sub)?;
        let target = dir.path().join("old.exe");
        create(&sub.join("private.lnk"), &target)?;
        let next = dir.path().join("RTXManager.exe");
        assert!(
            Plan::inspect(&target, &next, &[(dir.path().to_owned(), 0)])?
                .0
                .is_empty()
        );
        assert_eq!(
            Plan::inspect(&target, &next, &[(dir.path().to_owned(), 1)])?
                .0
                .len(),
            1
        );
        Ok(())
    }

    #[test]
    fn locked_second_shortcut_restores_first_without_overwriting_conflict() -> Result<()> {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = tempfile::tempdir()?;
        let first = dir.path().join("first.lnk");
        let second = dir.path().join("second.lnk");
        fs::write(&first, b"first original")?;
        fs::write(&second, b"second original")?;
        let guard = fs::OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(&second)?;
        let plan = Plan(vec![
            Edit {
                path: first.clone(),
                before: b"first original".to_vec(),
                after: b"changed".to_vec(),
            },
            Edit {
                path: second.clone(),
                before: b"second original".to_vec(),
                after: b"changed".to_vec(),
            },
        ]);
        assert!(
            plan.with_changes(|| panic!("must not launch after failed shortcut write"))
                .is_err()
        );
        assert_eq!(fs::read(first)?, b"first original");
        assert_eq!(fs::read(second)?, b"second original");
        drop(guard);
        Ok(())
    }
}
