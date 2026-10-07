//! UAC bootstrap for the GUI and update installer. Keep the PE manifest
//! asInvoker so an older, non-elevated manager can still launch a new updater.
use anyhow::{Context, Result, ensure};
use std::{ffi::OsString, os::windows::ffi::OsStrExt};
use windows::{
    Win32::{
        Foundation::CloseHandle,
        UI::{
            Shell::{
                SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW,
            },
            WindowsAndMessaging::SW_SHOWNORMAL,
        },
    },
    core::{PCWSTR, w},
};

const RELAUNCHED: &str = "--rtxfg-admin-relaunch";

// Quote one argv element using the Windows CRT rules. In particular, double
// trailing backslashes before the closing quote (e.g. --data-dir "D:\\").
fn quote_argument(arg: &[u16]) -> Result<Vec<u16>> {
    ensure!(!arg.contains(&0), "启动参数包含无效字符");
    let mut result = vec![b'"' as u16];
    let mut slashes = 0;
    for &c in arg {
        if c == b'\\' as u16 {
            slashes += 1;
        } else {
            let count = if c == b'"' as u16 {
                slashes * 2 + 1
            } else {
                slashes
            };
            result.extend(std::iter::repeat_n(b'\\' as u16, count));
            result.push(c);
            slashes = 0;
        }
    }
    result.extend(std::iter::repeat_n(b'\\' as u16, slashes * 2));
    result.push(b'"' as u16);
    Ok(result)
}

fn parameters(args: &[OsString]) -> Result<Vec<u16>> {
    let mut result = Vec::new();
    for arg in args.iter().skip(1).chain([&OsString::from(RELAUNCHED)]) {
        if !result.is_empty() {
            result.push(b' ' as u16);
        }
        result.extend(quote_argument(&arg.encode_wide().collect::<Vec<_>>())?);
    }
    ensure!(result.len() < 32767, "启动参数过长");
    result.push(0);
    Ok(result)
}

/// `true`: this process is elevated and can continue. `false`: Windows has
/// accepted the elevated relaunch, so this bootstrap must exit without a UI.
/// Cancellation is an error, never a successful non-elevated GUI startup.
pub fn ensure_admin(args: &[OsString]) -> Result<bool> {
    if crate::win::is_admin() {
        return Ok(true);
    }
    ensure!(
        !args.iter().any(|arg| arg == RELAUNCHED),
        "未获得管理员权限，程序未启动"
    );
    let executable = std::env::current_exe()?;
    let directory = std::env::current_dir()?;
    let executable = executable
        .as_os_str()
        .encode_wide()
        .chain([0])
        .collect::<Vec<_>>();
    let directory = directory
        .as_os_str()
        .encode_wide()
        .chain([0])
        .collect::<Vec<_>>();
    let args = parameters(args)?;
    // SAFETY: Run only the current executable through the standard Windows UAC
    // verb. All strings are owned UTF-16 buffers valid through this synchronous
    // call; argv elements are quoted separately, with no shell command involved.
    unsafe {
        let mut info = SHELLEXECUTEINFOW {
            cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
            fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC,
            lpVerb: w!("runas"),
            lpFile: PCWSTR(executable.as_ptr()),
            lpParameters: PCWSTR(args.as_ptr()),
            lpDirectory: PCWSTR(directory.as_ptr()),
            nShow: SW_SHOWNORMAL.0,
            ..Default::default()
        };
        ShellExecuteExW(&mut info).context("未获得管理员权限，程序未启动")?;
        ensure!(!info.hProcess.is_invalid(), "无法确认管理员程序已启动");
        CloseHandle(info.hProcess)?;
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elevation_quoting_preserves_unicode_spaces_quotes_and_drive_roots() -> Result<()> {
        for (input, expected) in [
            ("", "\"\""),
            ("--data-dir", "\"--data-dir\""),
            ("D:\\", "\"D:\\\\\""),
            ("C:\\游戏 目录\\", "\"C:\\游戏 目录\\\\\""),
            ("value\"quoted", "\"value\\\"quoted\""),
            ("\\\"", "\"\\\\\\\"\""),
        ] {
            assert_eq!(
                String::from_utf16(&quote_argument(&input.encode_utf16().collect::<Vec<_>>())?)?,
                expected
            );
        }
        assert!(quote_argument(&[0]).is_err());
        Ok(())
    }

    #[test]
    fn updater_and_gui_arguments_keep_order_and_add_one_recursion_guard() -> Result<()> {
        let args = [
            "manager.exe",
            "--apply-update",
            "C:\\更新 目录\\install.json",
        ]
        .map(OsString::from);
        let quoted = parameters(&args)?;
        assert_eq!(quoted.last(), Some(&0));
        assert_eq!(
            String::from_utf16(&quoted[..quoted.len() - 1])?,
            "\"--apply-update\" \"C:\\更新 目录\\install.json\" \"--rtxfg-admin-relaunch\""
        );
        Ok(())
    }
}
