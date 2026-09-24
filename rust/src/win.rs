use anyhow::{Context, Result, ensure};
use std::{
    collections::BTreeSet,
    ffi::OsStr,
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
};
use windows::{
    Win32::{
        Foundation::*,
        Security::{Cryptography::*, WinTrust::*},
        Storage::FileSystem::*,
        System::{Diagnostics::ToolHelp::*, Threading::*},
        UI::{Shell::*, WindowsAndMessaging::*},
    },
    core::{PCWSTR, PWSTR, w},
};
pub fn wide(s: impl AsRef<OsStr>) -> Vec<u16> {
    s.as_ref().encode_wide().chain(Some(0)).collect()
}
/// Match GPUI's Windows DisplayId to the monitor under the launch pointer.
pub fn launch_monitor() -> Option<u64> {
    use windows::Win32::Graphics::Gdi::{MONITOR_DEFAULTTONEAREST, MonitorFromPoint};
    let mut point = POINT::default();
    // SAFETY: point is writable; MonitorFromPoint returns a borrowed monitor handle.
    unsafe {
        GetCursorPos(&mut point).ok()?;
        let monitor = MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST);
        (!monitor.0.is_null()).then_some(monitor.0 as u64)
    }
}
pub fn from_wide(s: &[u16]) -> String {
    String::from_utf16_lossy(&s[..s.iter().position(|n| *n == 0).unwrap_or(s.len())])
}
pub struct Handle(pub HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: This wrapper owns its valid handle and closes it exactly once.
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}
pub struct GameLock(Handle);
impl Drop for GameLock {
    fn drop(&mut self) {
        // SAFETY: Successful construction acquired this mutex on the same thread.
        unsafe {
            let _ = ReleaseMutex(self.0.0);
        }
    }
}
pub fn game_lock(dir: &Path) -> Result<GameLock> {
    named_lock(&format!(
        "Local\\RTXFG-v3-{}",
        crate::core::hash(crate::core::key(dir).as_bytes())
    ))
}
fn named_lock(name: &str) -> Result<GameLock> {
    named_lock_timeout(name, 0)
}
/// Resource extraction happens on workers; concurrent startup checks may share it.
pub fn resource_lock(dir: &Path) -> Result<GameLock> {
    named_lock_timeout(
        &format!(
            "Local\\RTXFG-v3-{}",
            crate::core::hash(crate::core::key(dir).as_bytes())
        ),
        5_000,
    )
}
fn named_lock_timeout(name: &str, timeout_ms: u32) -> Result<GameLock> {
    let name = wide(name);
    // SAFETY: Name is NUL-terminated; acquired handle remains owned by GameLock.
    unsafe {
        let h = Handle(CreateMutexW(None, false, PCWSTR(name.as_ptr()))?);
        let code = WaitForSingleObject(h.0, timeout_ms);
        ensure!(
            code == WAIT_OBJECT_0 || code == WAIT_ABANDONED,
            "另一个管理器正在操作该游戏"
        );
        Ok(GameLock(h))
    }
}
pub fn instance_lock(data: &Path) -> Result<GameLock> {
    named_lock(&format!(
        "Local\\RTXFG-UI-{}{}",
        crate::core::hash(crate::core::key(data).as_bytes()),
        if is_admin() { "True" } else { "False" }
    ))
}
pub fn rename_no_replace(src: &Path, dst: &Path) -> Result<()> {
    let a = wide(src);
    let b = wide(dst);
    // SAFETY: Both paths have stable NUL-terminated buffers. Flags prohibit overwrite.
    unsafe {
        MoveFileExW(
            PCWSTR(a.as_ptr()),
            PCWSTR(b.as_ptr()),
            MOVEFILE_WRITE_THROUGH,
        )?;
    }
    Ok(())
}
pub fn replace_existing(src: &Path, dst: &Path) -> Result<()> {
    let a = wide(src);
    let b = wide(dst);
    // SAFETY: Both paths have stable NUL-terminated buffers. The source is a
    // fully written staging file on the same volume as the destination.
    unsafe {
        MoveFileExW(
            PCWSTR(a.as_ptr()),
            PCWSTR(b.as_ptr()),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )?;
    }
    Ok(())
}
pub fn running_in_directory(dir: &Path) -> Result<Vec<String>> {
    let names: BTreeSet<_> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .is_some_and(|x| x.eq_ignore_ascii_case("exe"))
        })
        .map(|e| e.file_name().to_string_lossy().to_lowercase())
        .collect();
    let mut result = BTreeSet::new();
    // SAFETY: All Win32 structures are initialized to their ABI sizes, buffers stay
    // live for each call, and every snapshot/process handle is released by RAII.
    unsafe {
        let snapshot = Handle(CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0)?);
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let mut more = Process32FirstW(snapshot.0, &mut entry).is_ok();
        while more {
            if ![0, 4, std::process::id()].contains(&entry.th32ProcessID) {
                let name = from_wide(&entry.szExeFile);
                let mut resolved = false;
                if let Ok(h) = OpenProcess(
                    PROCESS_QUERY_LIMITED_INFORMATION,
                    false,
                    entry.th32ProcessID,
                ) {
                    let h = Handle(h);
                    let mut buf = vec![0; 32768];
                    let mut size = buf.len() as u32;
                    if QueryFullProcessImageNameW(
                        h.0,
                        PROCESS_NAME_WIN32,
                        PWSTR(buf.as_mut_ptr()),
                        &mut size,
                    )
                    .is_ok()
                    {
                        resolved = true;
                        let p = PathBuf::from(from_wide(&buf[..size as usize]));
                        if p.parent()
                            .is_some_and(|p| crate::core::key(p) == crate::core::key(dir))
                        {
                            result.insert(name.clone());
                        }
                    }
                }
                if !resolved && names.contains(&name.to_lowercase()) {
                    result.insert(name);
                }
            }
            more = Process32NextW(snapshot.0, &mut entry).is_ok();
        }
    }
    Ok(result.into_iter().collect())
}
pub fn is_admin() -> bool {
    // SAFETY: IsUserAnAdmin takes no pointers and only queries the current token.
    unsafe { IsUserAnAdmin().as_bool() }
}
pub fn open(target: &str) -> Result<()> {
    let s = wide(target);
    // SAFETY: Fixed verb and owned NUL-terminated target; this does not invoke a shell command string.
    unsafe {
        ensure!(
            ShellExecuteW(
                None,
                w!("open"),
                PCWSTR(s.as_ptr()),
                None,
                None,
                SW_SHOWNORMAL
            )
            .0 as isize
                > 32,
            "无法打开目标"
        );
    }
    Ok(())
}
pub fn elevate(data: &Path) -> Result<()> {
    let exe = wide(std::env::current_exe()?);
    let s = data.to_string_lossy();
    ensure!(!s.contains('"'), "无效路径");
    let params = wide(format!("--data-dir \"{}\"", s.trim_end_matches('\\')));
    // SAFETY: runas is the Windows UAC verb. No arbitrary command concatenation is used.
    unsafe {
        ensure!(
            ShellExecuteW(
                None,
                w!("runas"),
                PCWSTR(exe.as_ptr()),
                PCWSTR(params.as_ptr()),
                None,
                SW_SHOWNORMAL
            )
            .0 as isize
                > 32,
            "未获得管理员权限，原窗口保留"
        );
    }
    Ok(())
}

pub fn elevated_gpu_action(encoded: &str, action: &str) -> Result<()> {
    ensure!(
        encoded.len() <= 8192
            && encoded
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"+/=".contains(&c)),
        "无效设备参数"
    );
    ensure!(
        ["restore", "4060", "5090"].contains(&action),
        "无效名称操作"
    );
    let exe = wide(std::env::current_exe()?);
    let params = wide(format!("--gpu-name-action {encoded} {action}"));
    // SAFETY: Run only this executable with fixed-mode, base64-only arguments.
    // Worker waits on its own owned helper handle; the GUI remains responsive.
    unsafe {
        let mut info = SHELLEXECUTEINFOW {
            cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
            fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC,
            lpVerb: w!("runas"),
            lpFile: PCWSTR(exe.as_ptr()),
            lpParameters: PCWSTR(params.as_ptr()),
            nShow: SW_HIDE.0,
            ..Default::default()
        };
        ShellExecuteExW(&mut info).context("未获得管理员权限，操作未完成")?;
        ensure!(!info.hProcess.is_invalid(), "无法跟踪显卡名称操作");
        let process = Handle(info.hProcess);
        ensure!(
            WaitForSingleObject(process.0, INFINITE) == WAIT_OBJECT_0,
            "无法等待显卡名称操作完成"
        );
        let mut code = 1;
        GetExitCodeProcess(process.0, &mut code)?;
        ensure!(
            code == 0,
            "显卡名称操作未完成，请检查管理员权限、驱动状态及原名备份"
        );
    }
    Ok(())
}
pub fn language() -> String {
    // SAFETY: Query-only API with no memory parameters.
    let id = unsafe { windows::Win32::Globalization::GetUserDefaultUILanguage() } & 0x3ff;
    match id {
        4 => "zh-CN",
        25 => "ru",
        17 => "ja",
        18 => "ko",
        _ => "en",
    }
    .into()
}
pub fn region() -> String {
    let mut buf = [0; 16]; // SAFETY: The API receives a sized, mutable UTF-16 slice.
    let count = unsafe { windows::Win32::Globalization::GetUserDefaultGeoName(&mut buf) };
    if count > 0 && from_wide(&buf) == "CN" {
        "gitee"
    } else {
        "github"
    }
    .into()
}
pub fn dark() -> bool {
    use winreg::{RegKey, enums::*};
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize")
        .and_then(|k| k.get_value::<u32, _>("AppsUseLightTheme"))
        .unwrap_or(1)
        == 0
}
pub fn build() -> u32 {
    use winreg::{RegKey, enums::*};
    RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey("SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion")
        .and_then(|k| k.get_value::<String, _>("CurrentBuildNumber"))
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}
pub fn drives() -> Vec<PathBuf> {
    // SAFETY: System enumeration and fixed three-character drive-root strings.
    unsafe {
        let mask = GetLogicalDrives();
        (0..26)
            .filter_map(|i| {
                let p = format!("{}:\\", (b'A' + i) as char);
                let w = wide(&p);
                if mask & (1 << i) != 0 && GetDriveTypeW(PCWSTR(w.as_ptr())) == 3 {
                    Some(PathBuf::from(p))
                } else {
                    None
                }
            })
            .collect()
    }
}
pub fn gpu_names() -> Result<Vec<(String, i32)>> {
    let root = PathBuf::from(std::env::var_os("SystemRoot").context("Missing SystemRoot")?);
    let path = root.join("System32/nvcuda.dll");
    // SAFETY: Load only the system CUDA driver; symbols use the documented CUDA
    // driver ABI, sized writable buffers and handles valid for the library lifetime.
    unsafe {
        let dll = libloading::Library::new(path)?;
        let init: libloading::Symbol<unsafe extern "system" fn(u32) -> i32> =
            dll.get(b"cuInit\0")?;
        ensure!(init(0) == 0, "CUDA 初始化失败");
        let count_fn: libloading::Symbol<unsafe extern "system" fn(*mut i32) -> i32> =
            dll.get(b"cuDeviceGetCount\0")?;
        let get: libloading::Symbol<unsafe extern "system" fn(*mut i32, i32) -> i32> =
            dll.get(b"cuDeviceGet\0")?;
        let name_fn: libloading::Symbol<unsafe extern "system" fn(*mut u8, i32, i32) -> i32> =
            dll.get(b"cuDeviceGetName\0")?;
        let cc: libloading::Symbol<unsafe extern "system" fn(*mut i32, *mut i32, i32) -> i32> =
            dll.get(b"cuDeviceComputeCapability\0")?;
        let mut n = 0;
        ensure!(
            count_fn(&mut n) == 0 && (0..32).contains(&n),
            "CUDA 查询失败"
        );
        let mut rows = Vec::new();
        for i in 0..n {
            let (mut dev, mut major, mut minor) = (0, 0, 0);
            let mut name = [0; 256];
            ensure!(
                get(&mut dev, i) == 0
                    && name_fn(name.as_mut_ptr(), 256, dev) == 0
                    && cc(&mut major, &mut minor, dev) == 0,
                "CUDA 查询失败"
            );
            rows.push((
                String::from_utf8_lossy(&name[..name.iter().position(|b| *b == 0).unwrap_or(256)])
                    .into(),
                major * 10 + minor,
            ));
        }
        Ok(rows)
    }
}
#[derive(Debug, serde::Serialize)]
pub struct Signature {
    pub status: String,
    pub thumbprint: String,
    pub version: String,
    pub name: String,
}
pub fn file_version(path: &Path) -> Result<(String, String)> {
    let wpath = wide(path);
    // SAFETY: Windows validates its version resource; all returned pointers are used
    // only while the backing buffer lives and checked against its address range.
    unsafe {
        let size = GetFileVersionInfoSizeW(PCWSTR(wpath.as_ptr()), None);
        ensure!(size > 0 && size < 1024 * 1024, "文件版本信息无效");
        let mut block = vec![0u8; size as usize];
        GetFileVersionInfoW(
            PCWSTR(wpath.as_ptr()),
            None,
            size,
            block.as_mut_ptr().cast(),
        )?;
        let query = |s: &str| -> Result<Vec<u8>> {
            let w = wide(s);
            let (mut ptr, mut len) = (std::ptr::null_mut(), 0);
            ensure!(
                VerQueryValueW(
                    block.as_ptr().cast(),
                    PCWSTR(w.as_ptr()),
                    &mut ptr,
                    &mut len
                )
                .as_bool(),
                "缺少文件版本字段"
            );
            let bytes = if s == "\\" || s == "\\VarFileInfo\\Translation" {
                len as usize
            } else {
                len as usize * 2
            };
            ensure!(
                !ptr.is_null()
                    && (ptr as usize) >= block.as_ptr() as usize
                    && (ptr as usize)
                        .checked_add(bytes)
                        .is_some_and(|end| end <= block.as_ptr() as usize + block.len()),
                "版本字段越界"
            );
            Ok(std::slice::from_raw_parts(ptr.cast::<u8>(), bytes).to_vec())
        };
        let fixed = query("\\")?;
        ensure!(fixed.len() >= 16, "无效固定版本信息");
        let ms = u32::from_le_bytes(fixed[8..12].try_into()?);
        let ls = u32::from_le_bytes(fixed[12..16].try_into()?);
        let version = format!("{}.{}.{}", ms >> 16, ms & 65535, ls >> 16);
        let translations = query("\\VarFileInfo\\Translation")?;
        let mut name = String::new();
        for t in translations.as_chunks::<4>().0 {
            let locale = u16::from_le_bytes([t[0], t[1]]);
            let page = u16::from_le_bytes([t[2], t[3]]);
            if let Ok(v) = query(&format!(
                "\\StringFileInfo\\{locale:04x}{page:04x}\\InternalName"
            )) {
                name = from_wide(
                    &v.as_chunks::<2>()
                        .0
                        .iter()
                        .map(|b| u16::from_le_bytes([b[0], b[1]]))
                        .collect::<Vec<_>>(),
                );
                break;
            }
        }
        Ok((version, name))
    }
}
pub fn verify_signature(path: &Path) -> Result<Signature> {
    let wpath = wide(path);
    // SAFETY: Trust structures use exact ABI sizes, the file-name buffer lives until
    // WTD_STATEACTION_CLOSE, and provider pointers are read only after success.
    unsafe {
        let mut file = WINTRUST_FILE_INFO {
            cbStruct: std::mem::size_of::<WINTRUST_FILE_INFO>() as u32,
            pcwszFilePath: PCWSTR(wpath.as_ptr()),
            ..Default::default()
        };
        let mut data = WINTRUST_DATA {
            cbStruct: std::mem::size_of::<WINTRUST_DATA>() as u32,
            dwUIChoice: WTD_UI_NONE,
            fdwRevocationChecks: WTD_REVOKE_NONE,
            dwUnionChoice: WTD_CHOICE_FILE,
            dwStateAction: WTD_STATEACTION_VERIFY,
            ..Default::default()
        };
        data.Anonymous.pFile = &mut file;
        let mut action = WINTRUST_ACTION_GENERIC_VERIFY_V2;
        let status = WinVerifyTrust(
            HWND(std::ptr::null_mut()),
            &mut action,
            (&mut data as *mut WINTRUST_DATA).cast(),
        );
        let outcome = (|| -> Result<String> {
            ensure!(status == 0, "更新文件的发布者签名无效：{status:#x}");
            let provider = WTHelperProvDataFromStateData(data.hWVTStateData);
            ensure!(!provider.is_null(), "签名提供程序无效");
            let signer = WTHelperGetProvSignerFromChain(provider, 0, false, 0);
            ensure!(
                !signer.is_null() && (*signer).csCertChain > 0 && !(*signer).pasCertChain.is_null(),
                "签名证书链无效"
            );
            let cert = (*(*signer).pasCertChain).pCert;
            ensure!(!cert.is_null(), "签名证书缺失");
            let mut hash = [0u8; 20];
            let mut len = 20;
            CertGetCertificateContextProperty(
                cert,
                CERT_SHA1_HASH_PROP_ID,
                Some(hash.as_mut_ptr().cast()),
                &mut len,
            )?;
            ensure!(len == 20, "证书指纹无效");
            Ok(hash.iter().map(|b| format!("{b:02X}")).collect())
        })();
        data.dwStateAction = WTD_STATEACTION_CLOSE;
        let _ = WinVerifyTrust(
            HWND(std::ptr::null_mut()),
            &mut action,
            (&mut data as *mut WINTRUST_DATA).cast(),
        );
        let thumbprint = outcome?;
        let (version, name) = file_version(path)?;
        ensure!(
            thumbprint == crate::updater::PUBLISHER && name == "RTXManager",
            "更新文件的发布者签名无效"
        );
        Ok(Signature {
            status: "Valid".into(),
            thumbprint,
            version,
            name,
        })
    }
}
pub fn wait_process(pid: u32, timeout: u32) -> Result<()> {
    // SAFETY: This only opens a synchronization handle; it never terminates a process.
    unsafe {
        match OpenProcess(PROCESS_SYNCHRONIZE, false, pid) {
            Ok(h) => {
                let h = Handle(h);
                ensure!(
                    WaitForSingleObject(h.0, timeout) == WAIT_OBJECT_0,
                    "请先退出旧版管理器"
                );
            }
            Err(e) => ensure!(
                e.code() == windows::core::HRESULT::from_win32(87),
                "无法等待旧版管理器退出"
            ),
        }
    }
    Ok(())
}
pub fn message(title: &str, text: &str) {
    let t = wide(title);
    let b = wide(text);
    // SAFETY: Stable NUL-terminated strings and no owner pointer.
    unsafe {
        MessageBoxW(
            None,
            PCWSTR(b.as_ptr()),
            PCWSTR(t.as_ptr()),
            MB_OK | MB_ICONERROR,
        );
    }
}
