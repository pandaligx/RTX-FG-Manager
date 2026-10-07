//! Keep modern color formats when available, with a Windows 10 COLR fallback.
use windows::{
    Win32::Graphics::DirectWrite::{
        DWRITE_COLOR_GLYPH_RUN, DWRITE_GLYPH_IMAGE_FORMATS, DWRITE_GLYPH_IMAGE_FORMATS_COLR,
        DWRITE_GLYPH_RUN, DWRITE_MATRIX, DWRITE_MEASURING_MODE_NATURAL,
        IDWriteColorGlyphRunEnumerator, IDWriteColorGlyphRunEnumerator1, IDWriteFactory3,
        IDWriteFactory4,
    },
    core::{Interface, Result},
};
use windows_numerics::Vector2;

pub(crate) enum ColorGlyphEnumerator {
    Modern(IDWriteColorGlyphRunEnumerator1),
    Legacy(IDWriteColorGlyphRunEnumerator),
}

pub(crate) fn translate(
    factory: &IDWriteFactory3,
    origin: Vector2,
    run: &DWRITE_GLYPH_RUN,
    formats: DWRITE_GLYPH_IMAGE_FORMATS,
    transform: Option<&DWRITE_MATRIX>,
) -> Result<ColorGlyphEnumerator> {
    if let Ok(modern) = factory.cast::<IDWriteFactory4>() {
        // QueryInterface is capability detection; errors translating an actual
        // glyph remain errors, rather than silently changing its format.
        return unsafe {
            modern.TranslateColorGlyphRun(
                origin,
                run,
                None,
                formats,
                DWRITE_MEASURING_MODE_NATURAL,
                transform.map(std::ptr::from_ref),
                0,
            )
        }
        .map(ColorGlyphEnumerator::Modern);
    }
    translate_legacy(factory, origin, run, transform)
}

fn translate_legacy(
    factory: &IDWriteFactory3,
    origin: Vector2,
    run: &DWRITE_GLYPH_RUN,
    transform: Option<&DWRITE_MATRIX>,
) -> Result<ColorGlyphEnumerator> {
    // The original color enumerator exposes COLR layers, without run1's format
    // field. Its availability predates the factory4 multi-format overload.
    unsafe {
        factory.TranslateColorGlyphRun(
            origin.X,
            origin.Y,
            run,
            None,
            DWRITE_MEASURING_MODE_NATURAL,
            transform.map(std::ptr::from_ref),
            0,
        )
    }
    .map(ColorGlyphEnumerator::Legacy)
}

impl ColorGlyphEnumerator {
    pub(crate) fn next(
        &mut self,
    ) -> Result<Option<(&DWRITE_COLOR_GLYPH_RUN, DWRITE_GLYPH_IMAGE_FORMATS)>> {
        // Both interfaces start before the first run. MoveNext must succeed
        // before GetCurrentRun; the returned run lives until the next advance.
        unsafe {
            match self {
                Self::Modern(enumerator) => {
                    if !enumerator.MoveNext()?.as_bool() {
                        return Ok(None);
                    }
                    let run = &*enumerator.GetCurrentRun()?;
                    Ok(Some((&run.Base, run.glyphImageFormat)))
                }
                Self::Legacy(enumerator) => {
                    if !enumerator.MoveNext()?.as_bool() {
                        return Ok(None);
                    }
                    let run = &*enumerator.GetCurrentRun()?;
                    Ok(Some((run, DWRITE_GLYPH_IMAGE_FORMATS_COLR)))
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ColorGlyphEnumerator, translate, translate_legacy};
    use std::mem::ManuallyDrop;
    use windows::{
        Win32::Graphics::DirectWrite::{
            DWRITE_FACTORY_TYPE_ISOLATED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_WEIGHT_NORMAL, DWRITE_GLYPH_IMAGE_FORMATS_COLR, DWRITE_GLYPH_RUN,
            DWriteCreateFactory, IDWriteFactory3, IDWriteFactory4,
        },
        core::{BOOL, Interface, Result, w},
    };
    use windows_numerics::Vector2;

    fn enumerate_emoji(force_legacy: bool) -> Result<()> {
        // DirectWrite and the installed Windows emoji font exercise real COM
        // enumeration, including its required initial MoveNext, without a GPU.
        unsafe {
            let factory: IDWriteFactory3 = DWriteCreateFactory(DWRITE_FACTORY_TYPE_ISOLATED)?;
            let mut collection = None;
            factory.GetSystemFontCollection(false, &mut collection, true)?;
            let collection = collection.unwrap();
            let mut family_index = 0;
            let mut exists = BOOL(0);
            collection.FindFamilyName(w!("Segoe UI Emoji"), &mut family_index, &mut exists)?;
            assert!(
                exists.as_bool(),
                "Windows emoji font is required for this regression"
            );
            let face = collection
                .GetFontFamily(family_index)?
                .GetFirstMatchingFont(
                    DWRITE_FONT_WEIGHT_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    DWRITE_FONT_STYLE_NORMAL,
                )?
                .CreateFontFace()?;
            let codepoint = 0x1f600;
            let mut glyph_index = 0;
            face.GetGlyphIndices(&codepoint, 1, &mut glyph_index)?;
            assert_ne!(glyph_index, 0);
            let mut run = DWRITE_GLYPH_RUN {
                fontFace: ManuallyDrop::new(Some(face)),
                fontEmSize: 24.0,
                glyphCount: 1,
                glyphIndices: &glyph_index,
                ..Default::default()
            };
            let result = (|| -> Result<()> {
                let modern_available = factory.cast::<IDWriteFactory4>().is_ok();
                let mut enumerator = if force_legacy {
                    translate_legacy(&factory, Vector2::default(), &run, None)?
                } else {
                    translate(
                        &factory,
                        Vector2::default(),
                        &run,
                        DWRITE_GLYPH_IMAGE_FORMATS_COLR,
                        None,
                    )?
                };
                assert_eq!(
                    matches!(enumerator, ColorGlyphEnumerator::Modern(_)),
                    modern_available && !force_legacy
                );
                let mut layers = 0;
                while let Some((layer, format)) = enumerator.next()? {
                    assert!(layer.glyphRun.glyphCount > 0);
                    if format & DWRITE_GLYPH_IMAGE_FORMATS_COLR != Default::default() {
                        layers += 1;
                    }
                }
                assert!(layers > 0, "a color glyph must expose at least one layer");
                Ok(())
            })();
            ManuallyDrop::drop(&mut run.fontFace);
            result
        }
    }

    #[test]
    fn color_enumerator_uses_available_modern_interface() -> Result<()> {
        enumerate_emoji(false)
    }

    #[test]
    fn legacy_color_enumerator_reads_first_and_remaining_layers() -> Result<()> {
        enumerate_emoji(true)
    }
}
