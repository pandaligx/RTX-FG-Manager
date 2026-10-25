//! Only manager-owned disposable directories. Cleanup identities and settings survive.
use crate::{assets, core, win};
use anyhow::Result;
use std::{fs, path::Path};
#[derive(Default)]
pub struct Report {
    pub bytes: u64,
    pub files: u64,
    pub skipped: u64,
}
fn tree(path: &Path, depth: u8, r: &mut Report) {
    if depth > 12 || r.files + r.skipped > 100_000 {
        r.skipped += 1;
        return;
    }
    let Ok(path) = core::no_links(path) else {
        r.skipped += 1;
        return;
    };
    let Ok(meta) = fs::symlink_metadata(&path) else {
        return;
    };
    if meta.is_dir() {
        if let Ok(entries) = fs::read_dir(&path) {
            for entry in entries.flatten() {
                tree(&entry.path(), depth + 1, r);
            }
        }
        let _ = fs::remove_dir(&path);
    } else if meta.is_file() {
        if fs::remove_file(&path).is_ok() {
            r.files += 1;
            r.bytes += meta.len();
        } else {
            r.skipped += 1;
        }
    }
}
pub fn clean_scoped(runtime: &Path, data: &Path) -> Result<Report> {
    core::no_links(runtime)?;
    core::no_links(data)?;
    let mut report = Report::default();
    // Never delete runtime/cloud-identities.json or the last valid catalog.
    // Match our filenames as well as the parent: custom --data-dir locations
    // may already contain an unrelated folder named "updates".
    let archive =
        regex::Regex::new(r"^[a-f0-9]{64}(\.(domestic|github)\.download)?\.zip(\.aria2)?$")?;
    let executable = regex::Regex::new(r"^RTXManager-v[0-9]+\.[0-9]+\.[0-9]+-x64\.exe(\.aria2)?$")?;
    for (path, updates) in [(runtime.join("cloud"), false), (data.join("updates"), true)] {
        core::no_links(&path)?;
        let exe = std::env::current_exe()?;
        if exe.starts_with(&path) {
            report.skipped += 1;
            continue;
        }
        if let Ok(entries) = fs::read_dir(&path) {
            for entry in entries.flatten().take(100_000) {
                let p = entry.path();
                let name = entry.file_name().to_string_lossy().into_owned();
                if !updates && archive.is_match(&name) && p.is_file() {
                    tree(&p, 0, &mut report);
                } else if updates
                    && name.len() == 32
                    && name.bytes().all(|b| b.is_ascii_hexdigit())
                    && core::no_links(&p).is_ok()
                {
                    if let Ok(files) = fs::read_dir(&p) {
                        for file in files.flatten().take(1000) {
                            let n = file.file_name().to_string_lossy().into_owned();
                            if (executable.is_match(&n)
                                || [
                                    "download.log",
                                    "install.json",
                                    "installed.json",
                                    "ready.json",
                                ]
                                .contains(&n.as_str()))
                                && file.path().is_file()
                            {
                                tree(&file.path(), 0, &mut report);
                            } else {
                                report.skipped += 1;
                            }
                        }
                    }
                    let _ = fs::remove_dir(&p);
                } else {
                    report.skipped += 1;
                }
            }
        }
        let _ = fs::remove_dir(&path);
    }
    // NR caches are disposable; game-side .rtx-fg-nr original backups are not.
    let nr = core::no_links(&runtime.join("neural-rendering"))?;
    let nr_dll = regex::Regex::new(r"^310\.(8\.0|9\.16)-[a-f0-9]{64}\.dll$")?;
    if let Ok(entries) = fs::read_dir(&nr) {
        for entry in entries.flatten().take(100_000) {
            let name = entry.file_name().to_string_lossy().into_owned();
            if (archive.is_match(&name) || nr_dll.is_match(&name)) && entry.path().is_file() {
                tree(&entry.path(), 0, &mut report);
            } else {
                report.skipped += 1;
            }
        }
    }
    let _ = fs::remove_dir(nr);
    // Optional runtime downloads only; original game backups live elsewhere.
    let optional = core::no_links(&runtime.join("optional-runtimes"))?;
    if let Ok(entries) = fs::read_dir(&optional) {
        for entry in entries.flatten().take(100_000) {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.len() != 64 || !name.bytes().all(|b| b.is_ascii_hexdigit()) {
                report.skipped += 1;
                continue;
            }
            let p = core::no_links(&entry.path())?;
            if let Ok(files) = fs::read_dir(&p) {
                for file in files.flatten().take(100) {
                    let name = file.file_name().to_string_lossy().into_owned();
                    if [
                        "runtime.zip",
                        "runtime.zip.aria2",
                        "nvngx_dlss.dll",
                        "nvngx_dlssd.dll",
                        "nvngx_dlssg.dll",
                        "libxess_fg.dll",
                        "libxell.dll",
                        "LICENSE.txt",
                        "NVIDIA-LICENSE.txt",
                    ]
                    .contains(&name.as_str())
                        && file.path().is_file()
                    {
                        tree(&file.path(), 0, &mut report);
                    } else {
                        report.skipped += 1;
                    }
                }
            }
            let _ = fs::remove_dir(p);
        }
    }
    let _ = fs::remove_dir(optional);
    Ok(report)
}
pub fn operation_lock() -> Result<win::GameLock> {
    let root = assets::cache_root()?;
    core::no_links(&root)?;
    fs::create_dir_all(&root)?;
    win::game_lock(&root.join("cache-operations"))
}

pub fn wait_for_operation(
    cancel: &std::sync::atomic::AtomicBool,
    waiting: impl FnMut(),
) -> Result<win::GameLock> {
    let root = assets::cache_root()?;
    core::no_links(&root)?;
    fs::create_dir_all(&root)?;
    win::cache_lock_wait(&root.join("cache-operations"), cancel, waiting)
}
