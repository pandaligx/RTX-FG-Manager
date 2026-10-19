//! Read-only Windows installation evidence. Never resolve links, execute command
//! lines, request accounts, or traverse a user's documents.
use crate::{core, game_installations::Install, game_names, scanner::Game};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};
use windows::{
    Win32::{
        Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW},
        System::Com::*,
        UI::Shell::*,
    },
    core::{Interface, PCWSTR},
};
use winreg::{RegKey, enums::*};

fn string(key: &RegKey, name: &str) -> String {
    key.get_value::<String, _>(name)
        .ok()
        .filter(|v| v.len() <= 4096)
        .unwrap_or_default()
}
fn path(value: &str) -> Option<PathBuf> {
    let value = value.trim().trim_matches('"').replace('/', "\\");
    let p = PathBuf::from(value);
    (p.is_absolute() && !p.to_string_lossy().starts_with("\\\\") && core::no_links(&p).is_ok())
        .then_some(p)
}
pub(crate) fn registry(cancel: &AtomicBool) -> Vec<Install> {
    let mut out = Vec::new();
    for hive in [HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER] {
        for view in [KEY_WOW64_64KEY, KEY_WOW64_32KEY] {
            let base = RegKey::predef(hive);
            if let Ok(key) = base.open_subkey_with_flags(
                r"Software\Microsoft\Windows\CurrentVersion\Uninstall",
                KEY_READ | view,
            ) {
                for name in key.enum_keys().take(4096).flatten() {
                    if cancel.load(Ordering::Relaxed) {
                        return out;
                    }
                    let Ok(k) = key.open_subkey_with_flags(&name, KEY_READ | view) else {
                        continue;
                    };
                    let title = string(&k, "DisplayName");
                    let root = path(&string(&k, "InstallLocation"));
                    let icon = icon_path(&string(&k, "DisplayIcon"));
                    if !game_names::valid_title(&title) {
                        continue;
                    }
                    // An uninstaller icon never proves a game executable's identity.
                    let target = icon.filter(|p| {
                        !p.file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .to_lowercase()
                            .contains("unins")
                    });
                    if let Some(root) = root.or_else(|| {
                        target
                            .as_deref()
                            .and_then(Path::parent)
                            .map(Path::to_path_buf)
                    }) {
                        let mut i = Install::new(root, "Windows", &title).with_priority(2);
                        if let Some(id) = name.strip_prefix("Steam App ") {
                            i.add_key(format!("steam:{id}"));
                        }
                        if let Some(id) = name.strip_prefix("Uplay Install ") {
                            i.add_key(format!("ubisoft:{id}"));
                        }
                        i.executable = target;
                        out.push(i);
                    }
                }
            }
            for (key_path, platform, dir_field, title_field) in [
                (
                    r"Software\Ubisoft\Launcher\Installs",
                    "ubisoft",
                    "InstallDir",
                    "DisplayName",
                ),
                (r"Software\GOG.com\Games", "gog", "path", "gameName"),
                (
                    r"Software\Tencent\WeGame\Games",
                    "wegame",
                    "InstallPath",
                    "DisplayName",
                ),
                (
                    r"Software\Tencent\Rail\Games",
                    "wegame",
                    "InstallPath",
                    "GameName",
                ),
            ] {
                let Ok(key) = base.open_subkey_with_flags(key_path, KEY_READ | view) else {
                    continue;
                };
                for id in key.enum_keys().take(2048).flatten() {
                    if cancel.load(Ordering::Relaxed) {
                        return out;
                    }
                    let Ok(k) = key.open_subkey_with_flags(&id, KEY_READ | view) else {
                        continue;
                    };
                    if let Some(root) = path(&string(&k, dir_field)) {
                        let mut i = Install::new(root, platform, &string(&k, title_field));
                        i.add_key(format!("{platform}:{}", id.to_lowercase()));
                        out.push(i);
                    }
                }
            }
            // EA's legacy install records remain useful for current EA App games.
            for branch in [r"Software\EA Games", r"Software\Electronic Arts\EA Games"] {
                let Ok(key) = base.open_subkey_with_flags(branch, KEY_READ | view) else {
                    continue;
                };
                for id in key.enum_keys().take(1024).flatten() {
                    if cancel.load(Ordering::Relaxed) {
                        return out;
                    }
                    let Ok(k) = key.open_subkey_with_flags(&id, KEY_READ | view) else {
                        continue;
                    };
                    if let Some(root) = path(&string(&k, "Install Dir")) {
                        out.push(Install::new(root, "EA", &string(&k, "DisplayName")));
                    }
                }
            }
        }
    }
    out
}
fn icon_path(value: &str) -> Option<PathBuf> {
    let value = value.trim();
    let value = if let Some(s) = value.strip_prefix('"') {
        s.split_once('"')?.0
    } else {
        value
            .rsplit_once(',')
            .filter(|(_, n)| n.trim().parse::<i32>().is_ok())
            .map_or(value, |(p, _)| p)
    };
    let p = path(value)?;
    p.extension()
        .is_some_and(|s| s.eq_ignore_ascii_case("exe"))
        .then_some(p)
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}
pub(crate) fn products(exe: &Path) -> Vec<String> {
    if core::no_links(exe).is_err() {
        return vec![];
    }
    let path = wide(&exe.to_string_lossy());
    // SAFETY: buffers live through every query; Windows-returned ranges are
    // checked before reading. No executable code or DLL is loaded.
    unsafe {
        let size = GetFileVersionInfoSizeW(PCWSTR(path.as_ptr()), None);
        if size == 0 || size > 1024 * 1024 {
            return vec![];
        }
        let mut block = vec![0u8; size as usize];
        if GetFileVersionInfoW(PCWSTR(path.as_ptr()), None, size, block.as_mut_ptr().cast())
            .is_err()
        {
            return vec![];
        }
        let query = |key: &str, text: bool| -> Option<Vec<u8>> {
            let key = wide(key);
            let (mut ptr, mut len) = (std::ptr::null_mut(), 0);
            if !VerQueryValueW(
                block.as_ptr().cast(),
                PCWSTR(key.as_ptr()),
                &mut ptr,
                &mut len,
            )
            .as_bool()
            {
                return None;
            }
            let count = (len as usize).checked_mul(if text { 2 } else { 1 })?;
            if ptr.is_null()
                || (ptr as usize) < block.as_ptr() as usize
                || (ptr as usize).checked_add(count)? > block.as_ptr() as usize + block.len()
            {
                return None;
            }
            Some(std::slice::from_raw_parts(ptr.cast::<u8>(), count).to_vec())
        };
        let mut out = Vec::new();
        for t in query(r"\VarFileInfo\Translation", false)
            .unwrap_or_default()
            .as_chunks::<4>()
            .0
            .iter()
            .take(32)
        {
            let locale = u16::from_le_bytes([t[0], t[1]]);
            let cp = u16::from_le_bytes([t[2], t[3]]);
            for field in ["ProductName", "FileDescription"] {
                if let Some(data) = query(
                    &format!("\\StringFileInfo\\{locale:04x}{cp:04x}\\{field}"),
                    true,
                ) {
                    let units: Vec<_> = data
                        .as_chunks::<2>()
                        .0
                        .iter()
                        .map(|b| u16::from_le_bytes([b[0], b[1]]))
                        .take_while(|n| *n != 0)
                        .collect();
                    if let Ok(s) = String::from_utf16(&units)
                        && game_names::valid_title(&s)
                        && !out.contains(&s)
                    {
                        out.push(s);
                    }
                }
            }
        }
        out
    }
}

struct ComApartment;
impl Drop for ComApartment {
    fn drop(&mut self) {
        // SAFETY: paired with successful CoInitializeEx on this worker thread.
        unsafe { CoUninitialize() }
    }
}
pub(crate) fn shortcuts(games: &[Game], cancel: &AtomicBool) -> BTreeMap<String, Vec<String>> {
    let mut out = BTreeMap::<String, Vec<String>>::new();
    // SAFETY: initialized and released on the same background worker. No COM
    // object escapes this function and no link is resolved, launched or saved.
    let initialized = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    if initialized.is_err() {
        return out;
    }
    let _apartment = ComApartment;
    let mut pending = Vec::new();
    for (env, suffix) in [
        ("USERPROFILE", "Desktop"),
        ("PUBLIC", "Desktop"),
        ("APPDATA", "Microsoft/Windows/Start Menu/Programs"),
        ("ProgramData", "Microsoft/Windows/Start Menu/Programs"),
    ] {
        if let Some(p) = std::env::var_os(env) {
            pending.push((PathBuf::from(p).join(suffix), 0));
        }
    }
    let mut budget = 2048usize;
    while let Some((dir, depth)) = pending.pop() {
        if budget == 0 || cancel.load(Ordering::Relaxed) {
            break;
        }
        if core::no_links(&dir).is_err() {
            continue;
        }
        let Ok(entries) = fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.take(budget).flatten() {
            budget -= 1;
            if cancel.load(Ordering::Relaxed) {
                return out;
            }
            let p = entry.path();
            if core::no_links(&p).is_err() {
                continue;
            }
            if p.is_dir() && depth < 3 {
                pending.push((p, depth + 1));
                continue;
            }
            if !p.extension().is_some_and(|s| s.eq_ignore_ascii_case("lnk"))
                || fs::metadata(&p).map_or(true, |m| m.len() > 128 * 1024)
            {
                continue;
            }
            let title = p
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            if !game_names::is_chinese(&title) || !game_names::valid_title(&title) {
                continue;
            }
            if let Some(target) = shortcut_target(&p) {
                let key = core::key(&target);
                if games.iter().any(|g| core::key(Path::new(&g.exe)) == key) {
                    let names = out.entry(key).or_default();
                    if !names.contains(&title) {
                        names.push(title);
                    }
                }
            }
        }
    }
    out
}
fn shortcut_target(path: &Path) -> Option<PathBuf> {
    // SAFETY: ShellLink is Microsoft's in-process parser. Only Load/GetPath and
    // GetArguments are used, with bounded local inputs and output buffers.
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).ok()?;
        let file: IPersistFile = link.cast().ok()?;
        let file_name = wide(&path.to_string_lossy());
        file.Load(PCWSTR(file_name.as_ptr()), STGM_READ).ok()?;
        let mut args = [0u16; 2048];
        link.GetArguments(&mut args).ok()?;
        if args[0] != 0 {
            return None;
        } // launchers with arguments are not a direct game target
        let mut buf = [0u16; 4096];
        link.GetPath(&mut buf, std::ptr::null_mut(), SLGP_RAWPATH.0 as u32)
            .ok()?;
        let value = String::from_utf16(&buf[..buf.iter().position(|x| *x == 0)?]).ok()?;
        self::path(&value)
    }
}
