//! File ownership, not executable names, selects external cleanup blockers.
use super::{BlockingProcess, Handle, from_wide, process_created, wide};
use anyhow::{Context, Result, bail, ensure};
use std::{collections::BTreeMap, fs::OpenOptions, os::windows::fs::OpenOptionsExt, path::PathBuf};
use windows::{
    Win32::{
        Foundation::{ERROR_MORE_DATA, ERROR_SUCCESS},
        System::{
            RestartManager::{
                CCH_RM_SESSION_KEY, RM_PROCESS_INFO, RmCritical, RmEndSession, RmExplorer,
                RmGetList, RmRegisterResources, RmService, RmStartSession,
            },
            Threading::{
                OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
                QueryFullProcessImageNameW,
            },
        },
    },
    core::{PCWSTR, PWSTR},
};

struct Session(u32);
fn protected_process(pid: u32, info: &RM_PROCESS_INFO) -> bool {
    [0, 4, std::process::id()].contains(&pid)
        || [RmService, RmCritical, RmExplorer].contains(&info.ApplicationType)
        || info.strServiceShortName[0] != 0
}
impl Drop for Session {
    fn drop(&mut self) {
        // SAFETY: This wrapper owns the successfully opened RM session.
        unsafe {
            let _ = RmEndSession(self.0);
        }
    }
}

/// Discover processes holding these exact files, including inherited handles in
/// helpers outside the game folder. Does not close handles or stop processes.
/// Services, Explorer and critical/unidentified processes are never terminable.
/// Errors retain the caller's recovery records instead of assuming no blockers.
pub fn processes_holding_files(paths: &[PathBuf]) -> Result<Vec<BlockingProcess>> {
    if paths.is_empty() {
        return Ok(Vec::new());
    }
    ensure!(paths.len() <= 4096, "待检查文件过多，请先关闭游戏后重试");
    let paths = paths
        .iter()
        .map(|p| crate::core::no_links(p))
        .collect::<Result<Vec<_>>>()?;
    ensure!(
        paths.iter().all(|p| p.is_absolute() && !p.is_dir()),
        "占用检查需要完整文件路径"
    );
    let mut files = Vec::new();
    for path in paths {
        // Request DELETE access without setting any delete disposition. Readers
        // sharing deletion need not be stopped, and normal uninstalls avoid RM's
        // expensive registration entirely. OPEN_REPARSE_POINT does not follow links.
        match OpenOptions::new()
            .access_mode(0x10080)
            .share_mode(7)
            .custom_flags(0x00200000)
            .open(&path)
        {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) if matches!(e.raw_os_error(), Some(5 | 32 | 33)) => files.push(path),
            Err(e) => return Err(e).with_context(|| format!("无法检查占用：{}", path.display())),
        }
    }
    if files.is_empty() {
        return Ok(Vec::new());
    }
    let names = files.iter().map(wide).collect::<Vec<_>>();
    let names = names.iter().map(|p| PCWSTR(p.as_ptr())).collect::<Vec<_>>();
    let mut handle = 0;
    let mut key = [0u16; CCH_RM_SESSION_KEY as usize + 1];
    // SAFETY: Correctly sized outputs and NUL-terminated owned strings remain
    // alive throughout registration. RM only queries; never calls RmShutdown.
    unsafe {
        RmStartSession(&mut handle, None, PWSTR(key.as_mut_ptr()))
            .ok()
            .context("无法检查文件占用，请关闭游戏及其后台进程后重试")?;
    }
    let session = Session(handle);
    // SAFETY: The session is live and all name buffers outlive this call.
    unsafe {
        RmRegisterResources(session.0, Some(&names), None, None).ok()?;
    }
    let mut entries = Vec::<RM_PROCESS_INFO>::new();
    for _ in 0..4 {
        let (mut needed, mut count, mut reboot) = (0, entries.len() as u32, 0);
        // SAFETY: count is the initialized allocation length; RM may write at
        // most that many entries. For the initial sizing call the pointer is NULL.
        let result = unsafe {
            RmGetList(
                session.0,
                &mut needed,
                &mut count,
                (!entries.is_empty()).then_some(entries.as_mut_ptr()),
                &mut reboot,
            )
        };
        if result == ERROR_MORE_DATA {
            ensure!(needed <= 4096, "占用进程过多，请关闭游戏后重试");
            entries.resize(needed as usize, RM_PROCESS_INFO::default());
            continue;
        }
        result.ok().context("无法读取文件占用进程")?;
        ensure!(
            result == ERROR_SUCCESS && count as usize <= entries.len(),
            "占用检查结果无效"
        );
        entries.truncate(count as usize);
        let mut processes = BTreeMap::new();
        for info in entries {
            let pid = info.Process.dwProcessId;
            let created = (u64::from(info.Process.ProcessStartTime.dwHighDateTime) << 32)
                | u64::from(info.Process.ProcessStartTime.dwLowDateTime);
            let protected = protected_process(pid, &info);
            let mut process = BlockingProcess {
                pid,
                name: from_wide(&info.strAppName),
                path: None,
                created: None,
                locked_files: files.clone(),
                protected,
            };
            // SAFETY: Read-only process handle is RAII-owned; query buffer is
            // initialized, bounded, and used only on API success. RM creation
            // time is compared to avoid substituting a reused PID.
            unsafe {
                if let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
                    let h = Handle(h);
                    let mut buf = vec![0; 32768];
                    let mut size = buf.len() as u32;
                    if process_created(h.0).ok() == Some(created)
                        && QueryFullProcessImageNameW(
                            h.0,
                            PROCESS_NAME_WIN32,
                            PWSTR(buf.as_mut_ptr()),
                            &mut size,
                        )
                        .is_ok()
                    {
                        let path = PathBuf::from(from_wide(&buf[..size as usize]));
                        process.name = path
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or(process.name);
                        process.path = Some(path);
                        process.created = Some(created);
                    }
                }
            }
            processes.insert(pid, process);
        }
        return Ok(processes.into_values().collect());
    }
    bail!("占用进程正在变化，请稍后重试")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn services_shell_critical_and_self_are_never_terminable() {
        let mut info = RM_PROCESS_INFO::default();
        assert!(!protected_process(12345, &info));
        for pid in [0, 4, std::process::id()] {
            assert!(protected_process(pid, &info));
        }
        for kind in [RmService, RmCritical, RmExplorer] {
            info.ApplicationType = kind;
            assert!(protected_process(12345, &info));
        }
        info = RM_PROCESS_INFO::default();
        info.strServiceShortName[0] = b'x' as u16;
        assert!(protected_process(12345, &info));
    }
}
