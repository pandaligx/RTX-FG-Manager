//! Select executable paths without asking the shell to open their contents.
//! Xbox/GDK may protect an EXE while its game directory remains accessible.
//! Callers must still validate every returned path with core::library_location.
use anyhow::{Context, Result};
use std::{ffi::OsString, os::windows::ffi::OsStringExt, path::PathBuf};
use windows::{
    Win32::{
        Foundation::ERROR_CANCELLED,
        System::Com::*,
        UI::Shell::{Common::COMDLG_FILTERSPEC, *},
    },
    core::{HRESULT, PCWSTR, PWSTR, w},
};

struct Apartment;
impl Apartment {
    fn new() -> Result<Self> {
        // SAFETY: Called on a background worker; all COM objects stay on this
        // thread and are released before the matching CoUninitialize.
        unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()? };
        Ok(Self)
    }
}
impl Drop for Apartment {
    fn drop(&mut self) {
        // SAFETY: Exactly one successful initialization is owned by this guard.
        unsafe { CoUninitialize() };
    }
}

struct ShellPath(PWSTR);
impl Drop for ShellPath {
    fn drop(&mut self) {
        // SAFETY: GetDisplayName transfers ownership of this COM allocation.
        unsafe { CoTaskMemFree(Some(self.0.0.cast())) };
    }
}

fn dialog(title: &str, multiple: bool) -> Result<IFileOpenDialog> {
    let title = crate::win::wide(title);
    // SAFETY: The caller owns a live STA apartment. Strings remain valid for
    // these synchronous setters; no raw pointer escapes into application state.
    unsafe {
        let dialog: IFileOpenDialog =
            CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)?;
        let mut options = dialog.GetOptions()?
            | FOS_NOVALIDATE
            | FOS_FORCEFILESYSTEM
            | FOS_PATHMUSTEXIST
            | FOS_FILEMUSTEXIST
            | FOS_NOCHANGEDIR
            | FOS_DONTADDTORECENT
            | FOS_NODEREFERENCELINKS;
        if multiple {
            options |= FOS_ALLOWMULTISELECT;
        }
        // FOS_NOVALIDATE skips the shell's access/sharing check, not the
        // manager's PE, manifest, architecture or deployment ownership checks.
        dialog.SetOptions(options)?;
        dialog.SetFileTypes(&[COMDLG_FILTERSPEC {
            pszName: w!("Windows EXE"),
            pszSpec: w!("*.exe"),
        }])?;
        dialog.SetTitle(PCWSTR(title.as_ptr()))?;
        Ok(dialog)
    }
}

/// Run on a background worker. Cancellation is distinct from a dialog error.
pub fn pick(title: &str, multiple: bool) -> Result<Option<Vec<PathBuf>>> {
    let _apartment = Apartment::new().context("无法初始化游戏选择窗口")?;
    let dialog = dialog(title, multiple).context("无法打开游戏选择窗口")?;
    // SAFETY: The apartment and dialog remain live on this thread. Shell paths
    // are copied before freeing their allocations; no file contents are opened.
    unsafe {
        if let Err(error) = dialog.Show(None) {
            if error.code() == HRESULT::from_win32(ERROR_CANCELLED.0) {
                return Ok(None);
            }
            return Err(error).context("无法完成游戏文件选择");
        }
        let items = dialog.GetResults()?;
        let mut paths = Vec::new();
        for index in 0..items.GetCount()? {
            let item = items.GetItemAt(index)?;
            let path = ShellPath(item.GetDisplayName(SIGDN_FILESYSPATH)?);
            paths.push(PathBuf::from(OsString::from_wide(path.0.as_wide())));
        }
        Ok(Some(paths))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn game_picker_accepts_path_only_without_weakening_file_selection() {
        std::thread::spawn(|| {
            let _apartment = Apartment::new().unwrap();
            for multiple in [false, true] {
                let dialog = dialog("Game", multiple).unwrap();
                // SAFETY: A live dialog is queried on its creating STA thread.
                let options = unsafe { dialog.GetOptions().unwrap() };
                for required in [
                    FOS_NOVALIDATE,
                    FOS_FORCEFILESYSTEM,
                    FOS_PATHMUSTEXIST,
                    FOS_FILEMUSTEXIST,
                    FOS_NOCHANGEDIR,
                    FOS_NODEREFERENCELINKS,
                ] {
                    assert_eq!(options & required, required);
                }
                assert_eq!((options & FOS_ALLOWMULTISELECT).0 != 0, multiple);
                assert_eq!((options & FOS_PICKFOLDERS).0, 0);
            }
        })
        .join()
        .unwrap();
    }
}
