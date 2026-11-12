//! Linux backend of the `win` facade: the same public API as `win.rs`, but it
//! operates on a Wine prefix instead of the live Windows API.
//!
//! On Linux this manager does not run inside Wine itself; it manipulates a
//! prefix's files directly. `drive_c` is a normal directory tree, so file
//! replacement, locking and atomic renames keep working unchanged. The Windows
//! registry is stored as plain text (`<prefix>/system.reg` = HKLM,
//! `<prefix>/user.reg` = HKCU) and is edited through `crate::wine_reg`.
//!
//! Features with no honest Linux equivalent (UAC elevation, Authenticode
//! signature trust, Windows Error Reporting event log, `ms-settings:` deep
//! links, named kernel objects) degrade into an explicit error or a documented
//! approximation. They must never silently claim success.

use anyhow::{Context, Result, ensure};
use std::{
    collections::BTreeMap,
    ffi::OsStr,
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

// ---------------------------------------------------------------------------
// Wine prefix discovery
// ---------------------------------------------------------------------------

/// Explicit prefix chosen through the UI or `RTXFG_PREFIX`. Kept process-wide
/// so every caller resolves the same prefix for the lifetime of the process.
static PREFIX: OnceLock<Mutex<Option<PathBuf>>> = OnceLock::new();

fn override_slot() -> &'static Mutex<Option<PathBuf>> {
    PREFIX.get_or_init(|| Mutex::new(None))
}

/// Remembers the prefix for this process (and validates that it is a prefix).
pub fn set_prefix(path: &Path) -> Result<()> {
    ensure_prefix(path)?;
    let mut slot = override_slot().lock().expect("prefix lock");
    *slot = Some(path.to_path_buf());
    Ok(())
}

fn ensure_prefix(path: &Path) -> Result<()> {
    ensure!(path.is_dir(), "Wine 前缀不存在：{}", path.display());
    ensure!(
        path.join("system.reg").is_file() || path.join("drive_c").is_dir(),
        "该目录不是 Wine 前缀（缺少 system.reg 与 drive_c）：{}",
        path.display()
    );
    Ok(())
}

/// The prefix this process should manage, or an explicit error explaining how
/// to set one. Never guesses a random directory.
pub fn prefix() -> Result<PathBuf> {
    if let Some(path) = override_slot().lock().expect("prefix lock").clone() {
        return Ok(path);
    }
    let candidates = [
        std::env::var_os("RTXFG_PREFIX").map(PathBuf::from),
        std::env::var_os("WINEPREFIX").map(PathBuf::from),
    ];
    for candidate in candidates.into_iter().flatten() {
        if candidate.is_dir() {
            return Ok(candidate);
        }
    }
    let home = std::env::var_os("HOME").map(PathBuf::from);
    if let Some(home) = home {
        let fallback = home.join(".wine");
        if fallback.is_dir() {
            return Ok(fallback);
        }
    }
    anyhow::bail!(
        "未找到 Wine 前缀，请设置 WINEPREFIX 或使用 --wine-prefix 指定（例如 /home/user/Games/game）"
    )
}

/// Like [`prefix`] but returns the conventional `~/.wine` when nothing is set,
/// for read-only display paths that must not fail the whole UI.
pub fn prefix_or_default() -> PathBuf {
    prefix().unwrap_or_else(|_| {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join(".wine")
    })
}

/// `drive_c` plus any other `drive_*` letter mapped by the prefix, so the
/// scanner sees the same set of files a Windows install would.
pub fn drives() -> Vec<PathBuf> {
    let prefix = prefix_or_default();
    let mut result = Vec::new();
    if let Ok(rows) = fs::read_dir(&prefix) {
        for row in rows.flatten() {
            let name = row.file_name().to_string_lossy().into_owned();
            if name.starts_with("drive_") && row.path().is_dir() {
                result.push(row.path());
            }
        }
    }
    result.sort();
    // Additional host roots (native Linux games) may be configured explicitly.
    if let Some(extra) = std::env::var_os("RTXFG_ROOTS") {
        for part in std::env::split_paths(&extra) {
            if part.is_dir() {
                result.push(part);
            }
        }
    }
    result
}

// ---------------------------------------------------------------------------
// UTF-16 helpers (kept for API parity with the Windows backend)
// ---------------------------------------------------------------------------

pub fn wide(s: impl AsRef<OsStr>) -> Vec<u16> {
    s.as_ref()
        .to_string_lossy()
        .encode_utf16()
        .chain(Some(0))
        .collect()
}

pub fn from_wide(s: &[u16]) -> String {
    String::from_utf16_lossy(&s[..s.iter().position(|c| *c == 0).unwrap_or(s.len())])
}

pub fn reduced_motion() -> bool {
    false
}

#[derive(Debug, Clone)]
pub struct BlockingProcess {
    pub pid: u32,
    pub name: String,
    pub path: Option<PathBuf>,
    pub created: Option<u64>,
    locked: bool,
}
impl BlockingProcess {
    pub fn holds_patch_files(&self) -> bool {
        self.locked
    }
    pub fn can_terminate(&self) -> bool {
        false
    }
}
pub fn processes_in_directory(
    dir: &Path,
    expected_exe: Option<&Path>,
) -> Result<Vec<BlockingProcess>> {
    scan_processes(dir, expected_exe)
}
pub fn processes_holding_files(paths: &[PathBuf]) -> Result<Vec<BlockingProcess>> {
    let targets = paths
        .iter()
        .map(|p| fs::canonicalize(p))
        .collect::<std::io::Result<std::collections::BTreeSet<_>>>()?;
    if targets.is_empty() {
        return Ok(Vec::new());
    }
    let mut found = Vec::new();
    for (index, entry) in fs::read_dir("/proc")?.enumerate() {
        ensure!(index < PROC_SCAN_LIMIT, "进程过多，无法完成占用检查");
        let entry = entry?;
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else {
            continue;
        };
        let Ok(fds) = fs::read_dir(entry.path().join("fd")) else {
            continue;
        };
        if fds
            .take(65536)
            .flatten()
            .any(|fd| fs::read_link(fd.path()).is_ok_and(|p| targets.contains(&p)))
        {
            let path = fs::read_link(entry.path().join("exe")).ok();
            let name = path
                .as_ref()
                .and_then(|p| p.file_name())
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|| format!("进程 {pid}"));
            found.push(BlockingProcess {
                pid,
                name,
                path,
                created: None,
                locked: true,
            });
        }
    }
    Ok(found)
}
pub fn terminate_confirmed(_process: &BlockingProcess) -> Result<()> {
    anyhow::bail!("Linux 请手动退出游戏；管理器不终止无法确认身份的进程")
}

// ---------------------------------------------------------------------------
// Advisory file locks (replacing named kernel mutexes)
// ---------------------------------------------------------------------------

/// Directory that holds the advisory lock files.
///
/// `$XDG_RUNTIME_DIR` is the right home for per-session, per-user locks, but it
/// is not always usable: a read-only `/run/user/<uid>` (some sandboxes and
/// container runtimes), a missing variable, or a stale directory all make
/// `create_dir_all` fail. Advisory locks only need a shared, writable
/// rendezvous point, so fall back rather than failing every locked write path —
/// a lock that cannot be created must never make the product unusable.
fn lock_root() -> Result<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR") {
        candidates.push(PathBuf::from(runtime).join("rtxfg-locks"));
    }
    if let Some(data) = std::env::var_os("XDG_DATA_HOME") {
        candidates.push(PathBuf::from(data).join("rtx-fg-manager/locks"));
    } else if let Some(home) = std::env::var_os("HOME") {
        candidates.push(PathBuf::from(home).join(".local/share/rtx-fg-manager/locks"));
    }
    candidates.push(std::env::temp_dir().join("rtxfg-locks"));
    let mut last: Option<anyhow::Error> = None;
    for root in candidates {
        match usable_lock_dir(&root) {
            Ok(()) => return Ok(root),
            Err(e) => last = Some(e),
        }
    }
    Err(last.unwrap_or_else(|| anyhow::anyhow!("无法创建锁目录")))
        .context("无法创建锁目录：请设置可写的 XDG_RUNTIME_DIR 或 XDG_DATA_HOME")
}

/// Confirms a directory can actually hold a lock file.
///
/// `create_dir_all` is not enough: it returns `Ok` when the directory already
/// exists, including on a read-only mount (a read-only `$XDG_RUNTIME_DIR` is
/// common in sandboxes and containers). The directory would then look usable and
/// every later `File::create` would fail. Probing with a real temporary file
/// makes the fallback chain actually fall through.
fn usable_lock_dir(root: &Path) -> Result<()> {
    fs::create_dir_all(root)?;
    let probe = root.join(format!(".probe-{}", std::process::id()));
    match fs::File::create(&probe) {
        Ok(_) => {
            let _ = fs::remove_file(&probe);
            Ok(())
        }
        Err(e) => Err(e).with_context(|| format!("锁目录不可写：{}", root.display())),
    }
}

/// One lock file per directory, mirroring the Windows backend's single named
/// mutex `Local\RTXFG-v3-{hash}`. The different entry points are NOT distinct
/// locks: they differ only in how long they are willing to wait, exactly as
/// `named_lock_timeout` does. Two managers, or a game lock racing a resource
/// lock on the same path, must therefore contend on the same file.
fn lock_path(dir: &Path) -> Result<PathBuf> {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(crate::core::key(dir).as_bytes());
    let digest = hasher.finalize();
    let mut name = String::new();
    for byte in &digest[..16] {
        name.push_str(&format!("{byte:02x}"));
    }
    Ok(lock_root()?.join(format!("{name}.lock")))
}

/// Advisory lock holder. Dropping the value releases the lock; the lock file
/// itself stays behind so concurrent holders keep a stable rendezvous point.
pub struct GameLock {
    file: File,
    label: String,
}

impl Drop for GameLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

fn try_acquire(path: &Path) -> Result<File> {
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(path)
        .with_context(|| format!("无法创建锁文件：{}", path.display()))?;
    match file.try_lock() {
        Ok(()) => Ok(file),
        // `TryLockError` distinguishes "held elsewhere" from a real I/O error.
        Err(std::fs::TryLockError::WouldBlock) => {
            anyhow::bail!(LOCK_HELD)
        }
        Err(std::fs::TryLockError::Error(e)) => {
            Err(e).with_context(|| format!("无法锁定：{}", path.display()))
        }
    }
}

/// Acquires `dir`'s lock, waiting up to `timeout`. `None` waits indefinitely.
/// A zero timeout is the Windows fail-fast `named_lock` (WaitForSingleObject 0).
fn acquire(dir: &Path, timeout: Option<Duration>) -> Result<GameLock> {
    let path = lock_path(dir)?;
    let start = Instant::now();
    loop {
        match try_acquire(&path) {
            Ok(file) => {
                return Ok(GameLock {
                    file,
                    label: path.display().to_string(),
                });
            }
            Err(e) if e.to_string().contains(LOCK_HELD) => match timeout {
                Some(limit) if start.elapsed() >= limit => {
                    anyhow::bail!("另一个管理器正在操作该游戏")
                }
                _ => std::thread::sleep(Duration::from_millis(25)),
            },
            Err(e) => return Err(e),
        }
    }
}

const LOCK_HELD: &str = "lock-held-elsewhere";

/// Exclusive lock for one game directory. Fail-fast, matching `named_lock`:
/// a second holder is reported as an error instead of silently queueing, so a
/// concurrent mutation is refused promptly rather than waiting behind a long
/// extraction.
pub fn game_lock(dir: &Path) -> Result<GameLock> {
    acquire(dir, Some(Duration::ZERO))
}

const CACHE_LOCK_DEADLINE: Duration = Duration::from_secs(15 * 60);

/// Waits for the cache operation lock, reporting progress and honouring
/// cancellation. Returns an error when cancelled or after the same 15 minute
/// deadline the Windows named mutex used.
pub fn cache_lock_wait(
    dir: &Path,
    cancel: &AtomicBool,
    mut waiting: impl FnMut(),
) -> Result<GameLock> {
    let path = lock_path(dir)?;
    let start = Instant::now();
    let mut announced = false;
    loop {
        ensure!(!cancel.load(Ordering::Relaxed), "操作已取消");
        match try_acquire(&path) {
            Ok(file) => {
                return Ok(GameLock {
                    file,
                    label: path.display().to_string(),
                });
            }
            Err(e) if e.to_string().contains(LOCK_HELD) => {
                if !announced {
                    waiting();
                    announced = true;
                }
                ensure!(
                    start.elapsed() < CACHE_LOCK_DEADLINE,
                    "等待其他缓存操作超时，已停止"
                );
                std::thread::sleep(Duration::from_millis(40));
            }
            Err(e) => return Err(e),
        }
    }
}

/// Lock around writing the extracted download tool into the cache. Bounded
/// 5 second wait, matching `resource_lock`'s `named_lock_timeout(..., 5_000)`.
pub fn resource_lock(dir: &Path) -> Result<GameLock> {
    acquire(dir, Some(Duration::from_secs(5)))
}

/// Single-instance lock for one data directory. Not a game lock: the Windows
/// backend renames it with the elevation state, so it lives in its own file.
pub fn instance_lock(data: &Path) -> Result<GameLock> {
    let path = lock_root()?.join(format!("ui-{}.lock", {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(crate::core::key(data).as_bytes());
        hasher.update(if is_admin() { "True" } else { "False" });
        let digest = hasher.finalize();
        let mut name = String::new();
        for byte in &digest[..16] {
            name.push_str(&format!("{byte:02x}"));
        }
        name
    }));
    let start = Instant::now();
    loop {
        match try_acquire(&path) {
            Ok(file) => {
                return Ok(GameLock {
                    file,
                    label: path.display().to_string(),
                });
            }
            Err(e) if e.to_string().contains(LOCK_HELD) => {
                // A second UI instance is a user-visible condition, not a crash.
                if start.elapsed() > Duration::from_secs(3) {
                    anyhow::bail!("另一个管理器实例正在运行")
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => return Err(e),
        }
    }
}

// ---------------------------------------------------------------------------
// Atomic file replacement
// ---------------------------------------------------------------------------

/// Renames without overwriting: fails when the destination already exists.
/// The kernel performs the existence check and rename as one operation.
/// Ordinary rename(2) overwrites a target and is unsafe for original backups.
pub fn rename_no_replace(src: &Path, dst: &Path) -> Result<()> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    let source = CString::new(src.as_os_str().as_bytes())?;
    let destination = CString::new(dst.as_os_str().as_bytes())?;
    // SAFETY: Both NUL-terminated paths live through this call. No pointer is
    // retained; RENAME_NOREPLACE applies equally to files and directories.
    let result = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            source.as_ptr(),
            libc::AT_FDCWD,
            destination.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if result != 0 {
        return Err(std::io::Error::last_os_error())
            .with_context(|| format!("无法移动 {} -> {}", src.display(), dst.display()));
    }
    sync_parent(dst)
}

/// Replaces the destination atomically, overwriting whatever was there.
pub fn replace_existing(src: &Path, dst: &Path) -> Result<()> {
    fs::rename(src, dst)
        .with_context(|| format!("无法替换 {} -> {}", src.display(), dst.display()))?;
    sync_parent(dst)
}

fn sync_parent(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        // Durability of the rename itself: the directory entry matters, not the
        // file contents, because the staged file was already fsynced.
        if let Ok(dir) = File::open(parent) {
            let _ = dir.sync_all();
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Process inspection: which Wine processes hold these files
// ---------------------------------------------------------------------------

const PROC_SCAN_LIMIT: usize = 8192;

/// Names of running processes whose executable lives inside `dir`.
///
/// This is a best-effort approximation: Linux has no equivalent of the
/// Toolhelp snapshot, and a game started through Wine may appear as a
/// `wine-preloader` process. It inspects both `/proc/<pid>/exe` and the
/// command line, so a prefix-launched game is normally detected, but callers
/// must treat an empty result as "no evidence", never as a guarantee.
pub fn running_in_directory(dir: &Path) -> Result<Vec<String>> {
    running_in_directory_impl(dir, None)
}

/// Like [`running_in_directory`], but also tolerates the expected executable
/// being absent (an update may have moved it).
pub fn running_in_directory_or_missing(dir: &Path, expected_exe: &Path) -> Result<Vec<String>> {
    running_in_directory_impl(dir, Some(expected_exe))
}

fn running_in_directory_impl(dir: &Path, expected_exe: Option<&Path>) -> Result<Vec<String>> {
    Ok(scan_processes(dir, expected_exe)?
        .into_iter()
        .map(|p| p.name)
        .collect())
}
fn scan_processes(dir: &Path, expected_exe: Option<&Path>) -> Result<Vec<BlockingProcess>> {
    let dir = fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    let mut names = Vec::new();
    let rows = match fs::read_dir("/proc") {
        Ok(rows) => rows,
        Err(error) => return Err(error).context("无法检查 /proc 中的运行进程"),
    };
    let mut visited = 0;
    for row in rows.flatten() {
        visited += 1;
        if visited > PROC_SCAN_LIMIT {
            break;
        }
        let pid = row.file_name().to_string_lossy().into_owned();
        if !pid.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        let base = row.path();
        let mut found: Option<(PathBuf, String)> = None;
        if let Ok(exe) = fs::read_link(base.join("exe")) {
            let name = exe
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            found = Some((exe, name));
        }
        if found.as_ref().is_none_or(|(exe, _)| !under(exe, &dir)) {
            // A Wine process can exec the game after start-up, so also consult
            // the command line before concluding the process is unrelated.
            if let Ok(raw) = fs::read(base.join("cmdline")) {
                let text = raw
                    .split(|b| *b == 0)
                    .map(|part| String::from_utf8_lossy(part).into_owned())
                    .collect::<Vec<_>>();
                for arg in text {
                    let candidate = PathBuf::from(&arg);
                    if candidate.is_absolute() && under(&candidate, &dir) {
                        let name = candidate
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_else(|| arg.clone());
                        found = Some((candidate, name));
                        break;
                    }
                }
            }
        }
        if let Some((exe, name)) = found
            && under(&exe, &dir)
        {
            let name = if name.is_empty() {
                format!("进程 {pid}")
            } else {
                name
            };
            names.push(BlockingProcess {
                pid: pid.parse()?,
                name,
                path: Some(exe),
                created: None,
                locked: false,
            });
        }
    }
    if let Some(expected) = expected_exe
        && !expected.exists()
        && names.is_empty()
    {
        // Missing expected executable with no running holder is the normal
        // state during an interrupted update; nothing to add.
    }
    names.sort_by_key(|p| p.pid);
    Ok(names)
}

fn under(path: &Path, dir: &Path) -> bool {
    let path = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    crate::core::within(&path, dir)
}

// ---------------------------------------------------------------------------
// Elevation: Linux has no UAC
// ---------------------------------------------------------------------------

/// Always `true`: a Wine prefix is an ordinary user-owned directory, so this
/// manager never needs elevation. Returning `true` keeps the Windows call
/// sites (which ask before writing driver metadata) from aborting.
pub fn is_admin() -> bool {
    true
}

/// Linux has no Admin approval prompt. The Windows updater relaunches itself
/// this way; an AppImage is read-only, so self-elevation is meaningless.
pub fn elevate(_data: &Path) -> Result<()> {
    anyhow::bail!("Linux 版无需也无法请求管理员权限；请直接运行 AppImage")
}

pub fn elevated_gpu_action(_encoded: &str, _action: &str) -> Result<()> {
    anyhow::bail!("修改显卡显示名称需要 Windows 驱动与 SetupAPI；Linux 版不支持此功能")
}

// ---------------------------------------------------------------------------
// Desktop integration
// ---------------------------------------------------------------------------

/// Opens a URL or file with the desktop's handler.
pub fn open(target: &str) -> Result<()> {
    ensure!(
        !target.starts_with("ms-settings:"),
        "Windows 设置页面无法在 Linux 打开（{target}）"
    );
    // `xdg-open` dispatches to the running desktop session (X11 or Wayland),
    // so no display-server branch is needed here.
    std::process::Command::new("xdg-open")
        .arg(target)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .with_context(|| format!("无法调用 xdg-open 打开 {target}"))?;
    Ok(())
}

/// Fatal-error notification. The Windows backend shows a MessageBox; on Linux
/// that is a portal/GTK dialog, which is unavailable before the GUI starts, so
/// stderr is authoritative and a dialog is attempted only when a display exists.
pub fn message(title: &str, text: &str) {
    let _ = writeln!(std::io::stderr(), "{title}: {text}");
    if std::env::var_os("DISPLAY").is_none() && std::env::var_os("WAYLAND_DISPLAY").is_none() {
        return;
    }
    let _ = rfd::MessageDialog::new()
        .set_title(title)
        .set_description(text)
        .set_level(rfd::MessageLevel::Error)
        .show();
}

// ---------------------------------------------------------------------------
// Locale and theming, read from the environment and the prefix registry
// ---------------------------------------------------------------------------

/// Maps the POSIX locale (`LANGUAGE`/`LC_ALL`/`LC_MESSAGES`/`LANG`) onto the
/// same language tags the Windows UI language check produced.
pub fn language() -> String {
    let raw = ["LANGUAGE", "LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .find_map(|name| std::env::var(name).ok())
        .unwrap_or_default();
    let raw = raw.split(':').next().unwrap_or("").to_lowercase();
    if raw.starts_with("zh") {
        "zh-CN".into()
    } else if raw.starts_with("ru") {
        "ru".into()
    } else if raw.starts_with("ja") {
        "ja".into()
    } else if raw.starts_with("ko") {
        "ko".into()
    } else {
        "en".into()
    }
}

/// Download mirror preference. Linux builds publish through GitHub releases.
pub fn region() -> String {
    "github".into()
}

/// Dark mode preference, read from the prefix's user registry. Wine stores the
/// same `Personalize\AppsUseLightTheme` value Windows does.
pub fn dark() -> bool {
    use crate::wine_reg::{Hive, RegValue};
    let prefix = prefix_or_default();
    matches!(
        crate::wine_reg::get_value(
            &prefix,
            Hive::User,
            "Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize",
            "AppsUseLightTheme"
        )
        .ok()
        .flatten(),
        Some(RegValue::Dword(0))
    )
}

/// Windows build number the prefix emulates, so version-gated features stay
/// consistent with what Wine reports to the games themselves.
pub fn build() -> u32 {
    use crate::wine_reg::{Hive, RegValue};
    let prefix = prefix_or_default();
    let value = crate::wine_reg::get_value(
        &prefix,
        Hive::Machine,
        "Software\\Microsoft\\Windows NT\\CurrentVersion",
        "CurrentBuildNumber",
    )
    .ok()
    .flatten();
    match value {
        Some(RegValue::Sz(text)) | Some(RegValue::ExpandSz(text)) => {
            text.trim().parse().unwrap_or(0)
        }
        Some(RegValue::Dword(v)) => v,
        _ => 0,
    }
}

/// Launches on a monitor nearest the pointer. GTK/GPUI place the window on the
/// focused output, so there is no portable equivalent; `None` means "default".
pub fn launch_monitor() -> Option<u64> {
    None
}

// ---------------------------------------------------------------------------
// CUDA device names (identical approach to the Windows backend, different .so)
// ---------------------------------------------------------------------------

/// CUDA driver entry points, loaded from `libcuda.so.1` exactly as the Windows
/// backend loads `nvcuda.dll`. Returns `(name, compute capability * 10 + minor)`.
pub fn gpu_names() -> Result<Vec<(String, i32)>> {
    #[allow(clippy::type_complexity)]
    type CuInit = unsafe extern "C" fn(u32) -> i32;
    type CuDeviceGetCount = unsafe extern "C" fn(*mut i32) -> i32;
    type CuDeviceGet = unsafe extern "C" fn(*mut i32, i32) -> i32;
    type CuDeviceGetName = unsafe extern "C" fn(*mut i8, i32, i32) -> i32;
    type CuDeviceComputeCapability = unsafe extern "C" fn(*mut i32, *mut i32, i32) -> i32;

    // SAFETY: dlopen of the vendor-provided CUDA driver library. The library
    // handle is kept alive for the whole call, and no symbol is invoked before
    // its own SAFETY comment below.
    let library = unsafe { libloading::Library::new("libcuda.so.1") }
        // SAFETY: same as above; this is only a fallback name for the same library.
        .or_else(|_| unsafe { libloading::Library::new("libcuda.so") })
        .context("未找到 CUDA 驱动库（libcuda.so.1）")?;
    // SAFETY: symbols are looked up by their stable CUDA driver names and are
    // only called while `library` stays alive; every out-pointer below is a
    // live stack variable sized for the call.
    unsafe {
        let init: libloading::Symbol<CuInit> = library.get(b"cuInit\0")?;
        ensure!(init(0) == 0, "CUDA 初始化失败");
        let count: libloading::Symbol<CuDeviceGetCount> = library.get(b"cuDeviceGetCount\0")?;
        let mut total = 0i32;
        ensure!(count(&mut total) == 0, "无法读取 CUDA 设备数量");
        ensure!(total >= 0, "CUDA 设备数量异常");
        let get: libloading::Symbol<CuDeviceGet> = library.get(b"cuDeviceGet\0")?;
        let name: libloading::Symbol<CuDeviceGetName> = library.get(b"cuDeviceGetName\0")?;
        let capability: libloading::Symbol<CuDeviceComputeCapability> =
            library.get(b"cuDeviceComputeCapability\0")?;
        let mut result = Vec::new();
        for index in 0..total.min(64) {
            let mut device = 0i32;
            if get(&mut device, index) != 0 {
                continue;
            }
            let mut buffer = [0i8; 256];
            if name(buffer.as_mut_ptr(), buffer.len() as i32, device) != 0 {
                continue;
            }
            let text = std::ffi::CStr::from_ptr(buffer.as_ptr())
                .to_string_lossy()
                .trim()
                .to_string();
            let (mut major, mut minor) = (0i32, 0i32);
            let score = if capability(&mut major, &mut minor, device) == 0 {
                major * 10 + minor
            } else {
                0
            };
            if !text.is_empty() {
                result.push((text, score));
            }
        }
        Ok(result)
    }
}

// ---------------------------------------------------------------------------
// PE version metadata (Authenticode cannot be verified outside Windows)
// ---------------------------------------------------------------------------

#[derive(Debug, serde::Serialize)]
pub struct Signature {
    pub status: String,
    pub thumbprint: String,
    pub version: String,
    pub name: String,
}

/// Reads `(FileVersion, InternalName)` from a PE version resource in pure Rust.
pub fn file_version(path: &Path) -> Result<(String, String)> {
    let data = fs::read(path).context("无法读取文件版本信息")?;
    let info = crate::pe_version::parse(&data)?;
    Ok((info.file_version, info.internal_name))
}

/// Authenticode verification is a Windows trust service. On Linux the file hash
/// and its embedded version are still checked by the caller, so this reports
/// `linux-not-verifiable` and never claims a publisher identity it could not
/// establish. Callers that require a trusted publisher must not run on Linux.
pub fn verify_signature(path: &Path) -> Result<Signature> {
    let (version, name) = file_version(path).unwrap_or_default();
    Ok(Signature {
        status: "linux-not-verifiable".into(),
        thumbprint: String::new(),
        version,
        name,
    })
}

/// Waits for a previously launched process to exit, bounded by `timeout_ms`.
/// Used by the updater after it hands control to a replacement executable.
pub fn wait_process(pid: u32, timeout: u32) -> Result<()> {
    let start = Instant::now();
    let deadline = Duration::from_millis(u64::from(timeout));
    let proc = PathBuf::from(format!("/proc/{pid}"));
    while proc.exists() {
        ensure!(start.elapsed() < deadline, "等待进程 {pid} 退出超时");
        std::thread::sleep(Duration::from_millis(50));
    }
    Ok(())
}

/// Not used on Linux, but kept so callers can share a platform-neutral helper.
#[allow(dead_code)]
pub(crate) fn lock_label(lock: &GameLock) -> &str {
    &lock.label
}

/// Reading a registry hive directly, for callers that need raw values.
#[allow(dead_code)]
pub(crate) fn hive_values(
    hive: crate::wine_reg::Hive,
    key: &str,
) -> Result<BTreeMap<String, crate::wine_reg::RegValue>> {
    let path = crate::wine_reg::hive_path(&prefix_or_default(), hive);
    if !path.is_file() {
        return Ok(BTreeMap::new());
    }
    let file = crate::wine_reg::RegFile::open(&path)?;
    Ok(file.values(key))
}

#[cfg(test)]
mod platform_regressions {
    use super::*;
    #[test]
    fn no_replace_keeps_existing_file_directory_and_link() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let source = temp.path().join("source");
        let target = temp.path().join("target");
        fs::write(&source, b"new")?;
        fs::write(&target, b"original")?;
        assert!(rename_no_replace(&source, &target).is_err());
        assert_eq!(fs::read(&target)?, b"original");
        fs::remove_file(&target)?;
        std::os::unix::fs::symlink(temp.path().join("missing"), &target)?;
        assert!(rename_no_replace(&source, &target).is_err());
        assert!(fs::symlink_metadata(&target)?.file_type().is_symlink());
        fs::remove_file(&target)?;
        rename_no_replace(&source, &target)?;
        assert_eq!(fs::read(&target)?, b"new");
        fs::create_dir(&source)?;
        let directory = temp.path().join("directory");
        fs::create_dir(&directory)?;
        assert!(rename_no_replace(&source, &directory).is_err());
        assert!(source.is_dir() && directory.is_dir());
        Ok(())
    }
    #[test]
    fn utf16_alias_value_matches_windows_terminator() {
        assert_eq!(wide("GPU"), vec![71, 80, 85, 0]);
        assert_eq!(from_wide(&wide("显卡")), "显卡");
    }
    #[test]
    fn own_open_file_is_reported_but_not_terminable() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("owned.log");
        let _open = File::create(&path)?;
        let holders = processes_holding_files(&[path])?;
        let current = holders
            .iter()
            .find(|p| p.pid == std::process::id())
            .context("missing holder")?;
        assert!(current.holds_patch_files() && !current.can_terminate());
        Ok(())
    }
}
