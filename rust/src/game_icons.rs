//! Resource-only icon extraction on a background worker; never executes the EXE.
use anyhow::{Result, ensure};
use std::{
    io::{Cursor, Read},
    path::Path,
};
use windows::{
    Win32::{
        Graphics::Gdi::*,
        UI::{
            Shell::{ExtractIconExW, SHDefExtractIconW},
            WindowsAndMessaging::*,
        },
    },
    core::PCWSTR,
};
const ICON_PIXELS: u32 = 128;

pub fn extract(path: &Path) -> Result<image::RgbaImage> {
    let path = rtx_fg_manager::core::no_links(path)?;
    ensure!(path.is_file(), "Icon file missing");
    // Protected Xbox executables may not expose PE resources even elevated.
    // Use the declared per-executable/package visual, not a generic EXE icon.
    if let Some(game) = rtx_fg_manager::xbox::identified(&path) {
        for logo in game.logos {
            if let Ok(icon) = package_icon(&logo) {
                return Ok(icon);
            }
        }
    }
    let wide = rtx_fg_manager::win::wide(path);
    // SAFETY: Shell extraction reads resources, without loading executable code.
    // Each icon/DC/bitmap is released, including failure paths. The DIB buffer
    // is ICON_PIXELS squared * 4 bytes, top-down.
    unsafe {
        let mut icon = HICON::default();
        let extracted = SHDefExtractIconW(
            PCWSTR(wide.as_ptr()),
            0,
            0,
            Some(&mut icon),
            None,
            ICON_PIXELS,
        );
        if extracted.is_err() || icon.is_invalid() {
            if !icon.is_invalid() {
                let _ = DestroyIcon(icon);
            }
            icon = HICON::default();
            let count = ExtractIconExW(PCWSTR(wide.as_ptr()), 0, Some(&mut icon), None, 1);
            if count != 1 || icon.is_invalid() {
                if !icon.is_invalid() {
                    let _ = DestroyIcon(icon);
                }
                anyhow::bail!("No EXE icon");
            }
        }
        let dc = CreateCompatibleDC(None);
        if dc.is_invalid() {
            let _ = DestroyIcon(icon);
            anyhow::bail!("Icon DC unavailable");
        }
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: ICON_PIXELS as i32,
                biHeight: -(ICON_PIXELS as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        let result = (|| {
            let bitmap = CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0)?;
            let old = SelectObject(dc, HGDIOBJ(bitmap.0));
            let result = (|| {
                ensure!(!bits.is_null(), "No icon pixels");
                let pixels = std::slice::from_raw_parts_mut(
                    bits.cast::<u8>(),
                    (ICON_PIXELS * ICON_PIXELS * 4) as usize,
                );
                pixels.fill(0);
                DrawIconEx(
                    dc,
                    0,
                    0,
                    icon,
                    ICON_PIXELS as i32,
                    ICON_PIXELS as i32,
                    0,
                    None,
                    DI_NORMAL,
                )?;
                let _ = GdiFlush();
                let black = pixels.to_vec();
                pixels.fill(255);
                DrawIconEx(
                    dc,
                    0,
                    0,
                    icon,
                    ICON_PIXELS as i32,
                    ICON_PIXELS as i32,
                    0,
                    None,
                    DI_NORMAL,
                )?;
                let _ = GdiFlush();
                let mut rgba = Vec::with_capacity(black.len());
                for (b, w) in black
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .zip(pixels.as_chunks::<4>().0)
                {
                    let alpha = 255 - w[0].saturating_sub(b[0]);
                    for i in [2, 1, 0] {
                        rgba.push(if alpha == 0 {
                            0
                        } else {
                            (b[i] as u32 * 255 / alpha as u32).min(255) as u8
                        });
                    }
                    rgba.push(alpha);
                }
                Ok(image::RgbaImage::from_raw(ICON_PIXELS, ICON_PIXELS, rgba)
                    .expect("bounded icon pixels"))
            })();
            SelectObject(dc, old);
            let _ = DeleteObject(HGDIOBJ(bitmap.0));
            result
        })();
        let _ = DeleteDC(dc);
        let _ = DestroyIcon(icon);
        result
    }
}

fn package_icon(path: &Path) -> Result<image::RgbaImage> {
    let path = rtx_fg_manager::core::no_links(path)?;
    let file = std::fs::File::open(path)?;
    const MAX_BYTES: u64 = 4 * 1024 * 1024;
    ensure!(
        file.metadata()?.len() <= MAX_BYTES,
        "Package icon too large"
    );
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= MAX_BYTES, "Package icon too large");
    let mut reader = image::ImageReader::with_format(Cursor::new(bytes), image::ImageFormat::Png);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(2048);
    limits.max_image_height = Some(2048);
    limits.max_alloc = Some(32 * 1024 * 1024);
    reader.limits(limits);
    let icon = reader
        .decode()?
        .resize(
            ICON_PIXELS,
            ICON_PIXELS,
            image::imageops::FilterType::Lanczos3,
        )
        .to_rgba8();
    let mut canvas = image::RgbaImage::new(ICON_PIXELS, ICON_PIXELS);
    image::imageops::overlay(
        &mut canvas,
        &icon,
        ((ICON_PIXELS - icon.width()) / 2) as i64,
        ((ICON_PIXELS - icon.height()) / 2) as i64,
    );
    Ok(canvas)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{
        ExtendedColorType,
        codecs::ico::{IcoEncoder, IcoFrame},
    };
    #[test]
    fn xbox_package_visuals_use_overrides_and_fall_back_when_an_asset_is_missing() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let exe = dir.path().join("Protected.exe");
        std::fs::write(&exe, b"no executable icon")?;
        image::RgbaImage::from_pixel(150, 150, image::Rgba([21, 144, 39, 255]))
            .save(dir.path().join("game.png"))?;
        image::RgbaImage::from_pixel(64, 64, image::Rgba([208, 32, 11, 255]))
            .save(dir.path().join("override.png"))?;
        let config = dir.path().join("MicrosoftGame.config");
        std::fs::write(
            &config,
            r#"<Game><ExecutableList><Executable Name="Protected.exe" TargetDeviceFamily="PC" OverrideLogo="override.png"/></ExecutableList><ShellVisuals Square150x150Logo="game.png"/></Game>"#,
        )?;
        assert_eq!(extract(&exe)?.get_pixel(64, 64).0, [208, 32, 11, 255]);
        std::fs::write(dir.path().join("override.png"), b"invalid PNG")?;
        assert_eq!(extract(&exe)?.get_pixel(64, 64).0, [21, 144, 39, 255]);
        std::fs::write(
            &config,
            r#"<Game><ExecutableList><Executable Name="Protected.exe" TargetDeviceFamily="PC" OverrideLogo="../outside.png"/></ExecutableList><ShellVisuals Square150x150Logo="missing.png" Square44x44Logo="game.png"/></Game>"#,
        )?;
        assert_eq!(rtx_fg_manager::xbox::declared(&exe).unwrap().logos.len(), 2);
        assert_eq!(extract(&exe)?.get_pixel(64, 64).0, [21, 144, 39, 255]);
        Ok(())
    }
    #[test]
    #[ignore = "read-only local Xbox icon probe; requires RTXFG_XBOX_GAME"]
    fn xbox_local_icon_readonly_probe() -> Result<()> {
        let exe = std::path::PathBuf::from(
            std::env::var_os("RTXFG_XBOX_GAME").expect("explicit Xbox installation"),
        );
        let icon = extract(&exe)?;
        ensure!(
            icon.dimensions() == (128, 128) && icon.pixels().any(|p| p.0[3] != 0),
            "Missing visible package icon"
        );
        println!("Read-only Xbox icon probe: {} / 128x128", exe.display());
        Ok(())
    }
    #[test]
    fn native_high_resolution_detail_and_alpha_survive_extraction() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let file = dir.path().join("高分辨率.ico");
        let mut pixels = image::RgbaImage::new(128, 128);
        for (x, y, p) in pixels.enumerate_pixels_mut() {
            *p = if x < 8 || y < 8 {
                image::Rgba([0, 0, 0, 0])
            } else if (x + y) % 2 == 0 {
                image::Rgba([255, 255, 255, 255])
            } else {
                image::Rgba([0, 0, 0, 255])
            };
        }
        let frame = IcoFrame::as_png(pixels.as_raw(), 128, 128, ExtendedColorType::Rgba8)?;
        IcoEncoder::new(std::fs::File::create(&file)?).encode_images(&[frame])?;
        let got = extract(&file)?;
        assert_eq!(got.dimensions(), (128, 128));
        assert_eq!(got.get_pixel(2, 2).0[3], 0);
        assert!(got.get_pixel(64, 64).0[0] > 240);
        assert!(got.get_pixel(65, 64).0[0] < 15);
        assert!(extract(&dir.path().join("missing.exe")).is_err());
        Ok(())
    }
}
