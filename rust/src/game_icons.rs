//! Resource-only icon extraction on a background worker; never executes the EXE.
use anyhow::{Result, ensure};
use std::path::Path;
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

#[cfg(test)]
mod tests {
    use super::*;
    use image::{
        ExtendedColorType,
        codecs::ico::{IcoEncoder, IcoFrame},
    };
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
