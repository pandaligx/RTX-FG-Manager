//! Local artwork belonging to the game's verified Steam installation.
use image::{ImageReader, RgbaImage, imageops::FilterType};
use std::{
    fs,
    io::{Cursor, Read},
    path::{Path, PathBuf},
};

pub(crate) fn original_pixels(image: image::DynamicImage) -> RgbaImage {
    if image.width().max(image.height()) > 512 {
        image.resize(512, 512, FilterType::Lanczos3).into_rgba8()
    } else {
        image.into_rgba8()
    }
}

fn best_logo(current: Option<RgbaImage>, candidate: RgbaImage) -> RgbaImage {
    match current {
        Some(icon)
            if icon.width().min(icon.height()) >= candidate.width().min(candidate.height()) =>
        {
            icon
        }
        _ => candidate,
    }
}

fn tile(path: &Path, require_square: bool) -> Option<RgbaImage> {
    const LIMIT: u64 = 4 * 1024 * 1024;
    let path = rtx_fg_manager::core::no_links(path).ok()?;
    if path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("ico"))
    {
        let image = crate::native_icons::native_icon(&path).ok()?;
        return (!require_square || image.width() == image.height()).then_some(image);
    }
    let file = fs::File::open(&path).ok()?;
    if file.metadata().ok()?.len() > LIMIT {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(LIMIT + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 > LIMIT {
        return None;
    }
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(40 * 1024 * 1024);
    reader.limits(limits);
    let img = reader.decode().ok()?;
    if require_square && img.width() != img.height() {
        return None;
    }
    let icon = original_pixels(img);
    icon.pixels().any(|p| p[3] > 0).then_some(icon)
}

fn launcher_logo(root: &Path, id: &str) -> Option<RgbaImage> {
    let cache = root.join("appcache/librarycache");
    let dir = cache.join(id);
    let mut files = vec![
        dir.join("icon.png"),
        dir.join("icon.jpg"),
        dir.join("logo.png"),
        cache.join(format!("{id}_icon.jpg")),
    ];
    if let Ok(entries) = fs::read_dir(&dir) {
        let mut hashed: Vec<_> = entries
            .take(128)
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.file_stem()
                    .and_then(|s| s.to_str())
                    .is_some_and(|s| s.len() == 40 && s.bytes().all(|b| b.is_ascii_hexdigit()))
            })
            .collect();
        hashed.sort();
        for path in hashed {
            if path.is_dir() {
                files.extend(["icon.png", "icon.jpg", "logo.png"].map(|name| path.join(name)));
            } else if path.extension().and_then(|s| s.to_str()).is_some_and(|s| {
                ["jpg", "png", "ico", "jpeg"]
                    .iter()
                    .any(|ext| ext.eq_ignore_ascii_case(s))
            }) {
                files.push(path);
            }
        }
    }
    files
        .iter()
        .filter_map(|path| {
            let named_logo = path
                .file_name()
                .is_some_and(|n| n.eq_ignore_ascii_case("logo.png"));
            tile(path, !named_logo)
        })
        .max_by_key(|i| i.width().min(i.height()))
}

fn cover_art(root: &Path, id: &str) -> Option<RgbaImage> {
    let cache = root.join("appcache/librarycache");
    let dir = cache.join(id);
    [
        dir.join("library_600x900.jpg"),
        cache.join(format!("{id}_library_600x900.jpg")),
        dir.join("header.jpg"),
        cache.join(format!("{id}_header.jpg")),
    ]
    .iter()
    .find_map(|p| tile(p, false))
}

pub(crate) fn resolve(
    exe: &Path,
    index: &mut rtx_fg_manager::game_identity::SteamArtworkIndex,
    native: impl FnOnce() -> anyhow::Result<RgbaImage>,
) -> anyhow::Result<RgbaImage> {
    if !exe
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"))
    {
        return native();
    }
    let mut logo = tile(&exe.with_extension("ico"), false);
    let Some((library, id)) = index.lookup(exe) else {
        return match native() {
            Ok(icon) => Ok(best_logo(logo, icon)),
            Err(error) => logo.ok_or(error),
        };
    };
    let mut roots: Vec<PathBuf> = vec![library.to_owned()];
    #[cfg(windows)]
    {
        use winreg::{RegKey, enums::*};
        let uninstall =
            format!(r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\Steam App {id}");
        for hive in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
            for view in [KEY_WOW64_64KEY, KEY_WOW64_32KEY] {
                if let Ok(key) =
                    RegKey::predef(hive).open_subkey_with_flags(&uninstall, KEY_READ | view)
                    && let Ok(value) = key.get_value::<String, _>("DisplayIcon")
                {
                    let raw = value.trim();
                    let raw = raw
                        .rsplit_once(',')
                        .filter(|(_, i)| i.trim().parse::<i32>().is_ok())
                        .map_or(raw, |(s, _)| s);
                    let path = Path::new(raw.trim().trim_matches('"'));
                    if matches!(path.components().next(), Some(std::path::Component::Prefix(p))
                        if matches!(p.kind(), std::path::Prefix::Disk(_) | std::path::Prefix::VerbatimDisk(_)))
                        && path.is_absolute()
                        && path
                            .extension()
                            .is_some_and(|v| v.eq_ignore_ascii_case("ico"))
                        && let Some(icon) = tile(path, false)
                    {
                        logo = Some(best_logo(logo, icon));
                    }
                }
            }
        }
        if let Ok(key) = RegKey::predef(HKEY_CURRENT_USER).open_subkey(r"Software\Valve\Steam")
            && let Ok(root) = key.get_value::<String, _>("SteamPath")
        {
            roots.push(root.into());
        }
        if let Some(dir) = std::env::var_os("ProgramFiles(x86)") {
            roots.push(PathBuf::from(dir).join("Steam"));
        }
    }
    #[cfg(not(windows))]
    if let Some(home) = std::env::var_os("HOME") {
        let home = Path::new(&home);
        roots.extend([
            home.join(".local/share/Steam"),
            home.join(".steam/steam"),
            home.join(".var/app/com.valvesoftware.Steam/data/Steam"),
        ]);
    }
    roots.dedup();
    for icon in roots.iter().filter_map(|root| launcher_logo(root, id)) {
        logo = Some(best_logo(logo, icon));
    }
    // A verified launcher logo is preferable to a generic executable icon.
    // Older Steam caches often have only a 32 px thumbnail; try the actual EXE then.
    if let Some(original) = logo.take_if(|i| i.width().min(i.height()) >= 256) {
        return Ok(original);
    }
    match native() {
        Ok(icon) => Ok(best_logo(logo, icon)),
        Err(error) => logo
            .or_else(|| roots.iter().find_map(|root| cover_art(root, id)))
            .ok_or(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn artwork_uses_the_verified_steam_install_dir() -> anyhow::Result<()> {
        let tmp = tempfile::tempdir()?;
        let apps = tmp.path().join("steamapps");
        let game = apps.join("common/Miles Morales");
        fs::create_dir_all(&game)?;
        fs::write(
            apps.join("appmanifest_987654301.acf"),
            "\"appid\" \"987654301\"\n\"installdir\" \"Miles Morales\"\n",
        )?;
        let exe = game.join("game.exe");
        fs::write(&exe, [])?;
        for (id, color) in [("987654301", 180u8), ("999", 26u8)] {
            let art = tmp.path().join("appcache/librarycache").join(id);
            fs::create_dir_all(&art)?;
            image::RgbImage::from_pixel(32, 32, image::Rgb([color, 0, 0]))
                .save(art.join("abcdabcdabcdabcdabcdabcdabcdabcdabcdabcd.jpg"))?;
        }
        let icon = resolve(&exe, &mut Default::default(), || {
            anyhow::bail!("fixture EXE contains no icon")
        })?;
        assert_eq!(icon.dimensions(), (32, 32));
        assert!(icon.get_pixel(16, 16)[0] > 170);
        let original =
            RgbaImage::from_fn(256, 256, |x, y| image::Rgba([x as u8, y as u8, 12, 255]));
        let logo = resolve(&exe, &mut Default::default(), || Ok(original.clone()))?;
        assert_eq!(logo, original);
        let hash_folder = tmp
            .path()
            .join("appcache/librarycache/987654301/1111111111111111111111111111111111111111");
        fs::create_dir_all(&hash_folder)?;
        let wordmark = RgbaImage::from_fn(240, 96, |x, y| {
            image::Rgba([0, x as u8, y as u8, if x < 8 { 0 } else { 255 }])
        });
        wordmark.save(hash_folder.join("logo.png"))?;
        let found = resolve(&exe, &mut Default::default(), || {
            anyhow::bail!("no embedded original")
        })?;
        assert_eq!(found, wordmark);
        Ok(())
    }
}
