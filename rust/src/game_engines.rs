//! Read-only renderer evidence, independent of DLSS. File names alone never
//! identify a renderer. Reads are bounded; no DLL is loaded or EXE executed.
use crate::{core, game_installations as installs, game_names};
use std::{
    collections::HashSet,
    fs::{self, File},
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

pub(crate) struct Entry {
    pub(crate) root: PathBuf,
    pub(crate) title: String,
    pub(crate) engine: &'static str,
}
fn entry(root: &Path, title: &str, engine: &'static str) -> Entry {
    Entry {
        root: root.into(),
        title: if game_names::valid_title(title) {
            title.into()
        } else {
            String::new()
        },
        engine,
    }
}
fn file(path: &Path) -> bool {
    path.is_file() && core::no_links(path).is_ok()
}
fn name(path: &Path, value: &str) -> bool {
    path.file_name()
        .is_some_and(|n| n.eq_ignore_ascii_case(value))
}
fn bytes<const N: usize>(f: &mut File, offset: u64) -> Option<[u8; N]> {
    if offset.checked_add(N as u64)? > f.metadata().ok()?.len() {
        return None;
    }
    f.seek(SeekFrom::Start(offset)).ok()?;
    let mut out = [0; N];
    f.read_exact(&mut out).ok()?;
    Some(out)
}
fn pck_header(f: &mut File, offset: u64) -> bool {
    let Some(h) = bytes::<88>(f, offset) else {
        return false;
    };
    let version = u32::from_le_bytes(h[4..8].try_into().unwrap());
    let major = u32::from_le_bytes(h[8..12].try_into().unwrap());
    &h[..4] == b"GDPC" && (1..=4).contains(&version) && (2..=4).contains(&major)
}
fn external_pck(path: &Path) -> bool {
    if !file(path) {
        return false;
    }
    File::open(path).is_ok_and(|mut f| pck_header(&mut f, 0))
}
fn footer_pck(f: &mut File, end: u64) -> bool {
    let Some(tail) = end.checked_sub(12).and_then(|n| bytes::<12>(f, n)) else {
        return false;
    };
    if &tail[8..] != b"GDPC" {
        return false;
    }
    let size = u64::from_le_bytes(tail[..8].try_into().unwrap());
    end.checked_sub(12)
        .and_then(|n| n.checked_sub(size))
        .is_some_and(|offset| offset >= 64 && pck_header(f, offset))
}
fn embedded_pck(path: &Path) -> bool {
    // Godot exports use a PE "pck" section or a length/magic footer.
    // Examine section metadata, never search a large executable for strings.
    let Some(mut f) = File::open(path).ok() else {
        return false;
    };
    let Some(dos) = bytes::<64>(&mut f, 0) else {
        return false;
    };
    if &dos[..2] != b"MZ" {
        return false;
    }
    let pe = u32::from_le_bytes(dos[60..64].try_into().unwrap()) as u64;
    let Some(coff) = bytes::<24>(&mut f, pe) else {
        return false;
    };
    if &coff[..4] != b"PE\0\0" {
        return false;
    }
    let sections = u16::from_le_bytes(coff[6..8].try_into().unwrap()) as u64;
    let optional = u16::from_le_bytes(coff[20..22].try_into().unwrap()) as u64;
    if sections > 96 || optional > 4096 {
        return false;
    }
    for ix in 0..sections {
        let Some(s) = bytes::<40>(&mut f, pe + 24 + optional + ix * 40) else {
            return false;
        };
        if &s[..4] == b"pck\0" || &s[..5] == b".pck\0" {
            let offset = u32::from_le_bytes(s[20..24].try_into().unwrap()) as u64;
            let size = u32::from_le_bytes(s[16..20].try_into().unwrap()) as u64;
            if size >= 95 && (0..8).any(|delta| pck_header(&mut f, offset + delta)) {
                return true;
            }
        }
    }
    let size = f.metadata().map(|m| m.len()).unwrap_or(0);
    if footer_pck(&mut f, size) {
        return true;
    }
    // Authenticode may follow a footer export; its directory stores a file offset.
    let Some(magic) = bytes::<2>(&mut f, pe + 24) else {
        return false;
    };
    let security = match u16::from_le_bytes(magic) {
        0x20b if optional >= 152 => pe + 24 + 144,
        0x10b if optional >= 136 => pe + 24 + 128,
        _ => return false,
    };
    let Some(cert) = bytes::<8>(&mut f, security) else {
        return false;
    };
    let offset = u32::from_le_bytes(cert[..4].try_into().unwrap()) as u64;
    let length = u32::from_le_bytes(cert[4..].try_into().unwrap()) as u64;
    offset > pe
        && length >= 8
        && offset.checked_add(length) == Some(size)
        && (0..8).any(|padding| {
            offset
                .checked_sub(padding)
                .is_some_and(|n| footer_pck(&mut f, n))
        })
}
fn gameinfo(path: &Path) -> Option<String> {
    let text = String::from_utf8(installs::read(path, 128 * 1024)?).ok()?;
    let lower = text.to_ascii_lowercase();
    if !lower.contains("gameinfo") || !lower.contains("filesystem") {
        return None;
    }
    let title = text
        .lines()
        .find_map(|line| {
            let line = line.trim();
            let (key, value) = line.split_once(char::is_whitespace)?;
            (key.trim_matches('"').eq_ignore_ascii_case("game"))
                .then(|| value.trim().trim_matches('"').to_string())
        })
        .unwrap_or_default();
    Some(if title.starts_with('#') {
        String::new()
    } else {
        title
    })
}
fn source(exe: &Path, names: &HashSet<String>) -> Option<Entry> {
    let dir = exe.parent()?;
    let stem = exe.file_stem()?.to_string_lossy().to_ascii_lowercase();
    if names.contains("engine2.dll")
        && names.contains("tier0.dll")
        && name(dir, "win64")
        && name(dir.parent()?, "bin")
    {
        let root = dir.parent()?.parent()?;
        let content = match stem.as_str() {
            "cs2" => "csgo",
            "dota2" => "dota",
            "deadlock" => "citadel",
            _ => &stem,
        };
        let title = gameinfo(&root.join(content).join("gameinfo.gi"))?;
        return Some(entry(
            if name(root, "game") {
                root.parent()?
            } else {
                root
            },
            &title,
            "Source 2",
        ));
    }
    // Source 1 uses a shared engine beside the game-specific GameInfo.
    let bin = if file(&dir.join("bin/engine.dll")) {
        dir.join("bin")
    } else if file(&dir.join("bin/x64/engine.dll")) {
        dir.join("bin/x64")
    } else {
        return None;
    };
    if !file(&bin.join("tier0.dll")) && !file(&dir.join("tier0.dll")) {
        return None;
    }
    let content = match stem.as_str() {
        "hl2" | "hl2_win64" => "hl2",
        "tf_win64" => "tf",
        _ => &stem,
    };
    let title = gameinfo(&dir.join(content).join("gameinfo.txt"))?;
    Some(entry(dir, &title, "Source"))
}
pub(crate) fn cryengine(exe: &Path) -> Option<Entry> {
    let dir = exe.parent()?;
    let root = if name(dir, "win_x64_release") && name(dir.parent()?, "bin") {
        dir.parent()?.parent()?
    } else if name(dir, "bin64") {
        dir.parent()?
    } else {
        return None;
    };
    let data = installs::read(&root.join("Game.cryproject"), 128 * 1024)?;
    let project: serde_json::Value = serde_json::from_slice(&data).ok()?;
    let title = project["info"]["name"].as_str()?;
    let assets = project["content"]["assets"]
        .as_array()?
        .first()?
        .as_str()?
        .replace('\\', "/");
    if !crate::game_name_rules::relative(&assets)
        || core::no_links(&root.join(&assets)).is_err()
        || !root.join(assets).is_dir()
    {
        return None;
    }
    let stem = exe.file_stem()?.to_string_lossy();
    if !stem.eq_ignore_ascii_case("GameLauncher")
        && normalized(&stem) != normalized(title)
        && normalized(&stem) != normalized(&root.file_name()?.to_string_lossy())
    {
        return None;
    }
    let engine = root.join("Engine");
    let packed = core::no_links(&engine).is_ok()
        && fs::read_dir(&engine).is_ok_and(|entries| {
            entries.take(256).flatten().any(|e| {
                e.path()
                    .extension()
                    .is_some_and(|s| s.eq_ignore_ascii_case("pak"))
                    && file(&e.path())
            })
        });
    if !packed && !(file(&dir.join("CrySystem.dll")) && file(&dir.join("CryAction.dll"))) {
        return None;
    }
    Some(entry(root, title, "CRYENGINE"))
}
pub(crate) fn normalized(value: &str) -> String {
    value
        .chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}
/// A matching product title alone also appears on tools shipped with a game.
/// Require a graphics import as independent renderer evidence. Dynamically
/// loaded renderers still use explicit platform or engine declarations.
pub(crate) fn graphics_import(exe: &Path) -> bool {
    fn imports(exe: &Path) -> Option<bool> {
        let mut f = File::open(exe).ok()?;
        let dos = bytes::<64>(&mut f, 0)?;
        if &dos[..2] != b"MZ" {
            return None;
        }
        let pe = u32::from_le_bytes(dos[60..64].try_into().ok()?) as u64;
        let coff = bytes::<24>(&mut f, pe)?;
        if &coff[..4] != b"PE\0\0" {
            return None;
        }
        let sections = u16::from_le_bytes(coff[6..8].try_into().ok()?) as u64;
        let optional = u16::from_le_bytes(coff[20..22].try_into().ok()?) as u64;
        if sections > 96 || !(128..=4096).contains(&optional) {
            return None;
        }
        let magic = u16::from_le_bytes(bytes::<2>(&mut f, pe + 24)?);
        let data_dir = match magic {
            0x20b => 112,
            0x10b => 96,
            _ => return None,
        };
        if optional < data_dir + 16 {
            return None;
        }
        let import = bytes::<8>(&mut f, pe + 24 + data_dir + 8)?;
        let rva = u32::from_le_bytes(import[..4].try_into().ok()?) as u64;
        let size = u32::from_le_bytes(import[4..].try_into().ok()?) as u64;
        if rva == 0 || size < 20 {
            return None;
        }
        let mut map = Vec::new();
        for ix in 0..sections {
            let s = bytes::<40>(&mut f, pe + 24 + optional + ix * 40)?;
            map.push((
                u32::from_le_bytes(s[12..16].try_into().ok()?) as u64,
                u32::from_le_bytes(s[16..20].try_into().ok()?) as u64,
                u32::from_le_bytes(s[20..24].try_into().ok()?) as u64,
            ));
        }
        let offset = |rva: u64, length: u64| {
            map.iter().find_map(|&(base, size, raw)| {
                let delta = rva.checked_sub(base)?;
                (delta.checked_add(length)? <= size).then(|| raw + delta)
            })
        };
        for ix in 0..(size / 20).min(256) {
            let h = bytes::<20>(&mut f, offset(rva + ix * 20, 20)?)?;
            if h == [0; 20] {
                break;
            }
            let name = u32::from_le_bytes(h[12..16].try_into().ok()?) as u64;
            let (raw, available) = map.iter().find_map(|&(base, size, raw)| {
                let delta = name.checked_sub(base)?;
                (delta < size).then(|| (raw + delta, (size - delta).min(128)))
            })?;
            f.seek(SeekFrom::Start(raw)).ok()?;
            let mut data = vec![0; available as usize];
            f.read_exact(&mut data).ok()?;
            let end = data.iter().position(|&b| b == 0)?;
            let dll = data[..end]
                .iter()
                .map(u8::to_ascii_lowercase)
                .collect::<Vec<_>>();
            if [
                b"d3d9.dll".as_slice(),
                b"d3d11.dll",
                b"d3d12.dll",
                b"dxgi.dll",
                b"opengl32.dll",
                b"vulkan-1.dll",
            ]
            .contains(&dll.as_slice())
            {
                return Some(true);
            }
        }
        Some(false)
    }
    imports(exe).unwrap_or(false)
}
pub(crate) fn probe(exe: &Path, names: &HashSet<String>) -> Option<Entry> {
    if external_pck(&exe.with_extension("pck")) || embedded_pck(exe) {
        return Some(entry(exe.parent()?, "", "Godot"));
    }
    source(exe, names).or_else(|| cryengine(exe))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn graphics_imports_are_bounded_and_not_arbitrary_embedded_strings() {
        let t = tempfile::tempdir().unwrap();
        let exe = t.path().join("render.exe");
        let mut data = vec![0; 1024];
        data[..2].copy_from_slice(b"MZ");
        data[60..64].copy_from_slice(&128u32.to_le_bytes());
        data[128..132].copy_from_slice(b"PE\0\0");
        data[134..136].copy_from_slice(&1u16.to_le_bytes());
        data[148..150].copy_from_slice(&240u16.to_le_bytes());
        data[152..154].copy_from_slice(&0x20bu16.to_le_bytes());
        // One .idata section, RVA 0x1000 at file offset 512.
        data[404..408].copy_from_slice(&0x1000u32.to_le_bytes());
        data[408..412].copy_from_slice(&512u32.to_le_bytes());
        data[412..416].copy_from_slice(&512u32.to_le_bytes());
        data[272..276].copy_from_slice(&0x1000u32.to_le_bytes());
        data[276..280].copy_from_slice(&40u32.to_le_bytes());
        data[524..528].copy_from_slice(&0x1040u32.to_le_bytes());
        data[576..585].copy_from_slice(b"D3D12.dll");
        fs::write(&exe, &data).unwrap();
        assert!(graphics_import(&exe));
        data[524..528].copy_from_slice(&u32::MAX.to_le_bytes());
        fs::write(&exe, &data).unwrap();
        assert!(!graphics_import(&exe));
        data[524..528].copy_from_slice(&0x1040u32.to_le_bytes());
        data[576..585].copy_from_slice(b"other.dll");
        fs::write(&exe, &data).unwrap();
        assert!(!graphics_import(&exe));
        data[412..416].copy_from_slice(&u32::MAX.to_le_bytes());
        fs::write(&exe, &data).unwrap();
        assert!(!graphics_import(&exe));
        data.truncate(150);
        fs::write(&exe, &data).unwrap();
        assert!(!graphics_import(&exe));
    }
}
