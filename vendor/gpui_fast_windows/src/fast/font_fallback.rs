// Modified by Longbridge for gpui-fast.
//! A second way to open a font when upstream's fails.
//!
//! Upstream opens fonts through DirectWrite 3's font sets (`IDWriteFontSet` →
//! `IDWriteFontFaceReference`) and also needs the font's features
//! (`IDWriteTypography`) to apply; if any step fails, the font is dropped. On
//! some users' machines that path returns E_NOTIMPL for every font, Arial
//! included, and the app panics in `TextSystem::resolve_font` on startup. This
//! looks the family up by name through DirectWrite 1 instead, falls back to no
//! features when they don't apply, and logs each failing step with the family
//! name, so a machine where this fails too still tells us which step it was.

use anyhow::{Context as _, Result};
use gpui::{Font, SharedString, font_name_with_fallbacks_shared};
use gpui_util::ResultExt;
use windows::{
    Win32::Graphics::DirectWrite::{
        DWRITE_FONT_STRETCH_NORMAL, IDWriteFactory3, IDWriteFontCollection1, IDWriteFontFace3,
        IDWriteTypography,
    },
    core::{BOOL, HSTRING, Interface},
};

use crate::direct_write::{
    DirectWriteComponents, DirectWriteState, FontInfo, apply_font_features, font_style_to_dwrite,
    font_weight_to_dwrite,
};

/// Like upstream, the fonts the app loaded come before the system's.
pub(crate) fn font_info(
    state: &DirectWriteState,
    components: &DirectWriteComponents,
    font: &Font,
) -> Option<FontInfo> {
    let family = font_name_with_fallbacks_shared(&font.family, &components.system_ui_font_name);
    [&state.custom_font_collection, &state.system_font_collection]
        .into_iter()
        .find_map(|collection| {
            family_font_info(
                font,
                family,
                collection,
                &components.factory,
                &state.system_font_collection,
            )
            .with_context(|| format!("legacy DirectWrite lookup of font family {family:?}"))
            .log_err()
            .flatten()
        })
}

/// `Ok(None)` when the collection has no such family, which is not a failure.
fn family_font_info(
    font: &Font,
    family: &SharedString,
    collection: &IDWriteFontCollection1,
    factory: &IDWriteFactory3,
    system_font_collection: &IDWriteFontCollection1,
) -> Result<Option<FontInfo>> {
    let font_family_h = HSTRING::from(family.as_str());
    let mut index = 0;
    let mut exists = BOOL(0);
    unsafe { collection.FindFamilyName(&font_family_h, &mut index, &mut exists) }
        .context("FindFamilyName")?;
    if !exists.as_bool() {
        return Ok(None);
    }
    let font_face = unsafe {
        collection
            .GetFontFamily(index)
            .context("GetFontFamily")?
            .GetFirstMatchingFont(
                font_weight_to_dwrite(font.weight),
                DWRITE_FONT_STRETCH_NORMAL,
                font_style_to_dwrite(font.style),
            )
            .context("GetFirstMatchingFont")?
            .CreateFontFace()
            .context("IDWriteFont::CreateFontFace")?
    }
    .cast::<IDWriteFontFace3>()
    .context("cast to IDWriteFontFace3")?;
    let features = typography(font, family, factory)?;
    let fallbacks = font.fallbacks.as_ref().and_then(|fallbacks| {
        DirectWriteState::generate_font_fallbacks(fallbacks, factory, system_font_collection)
            .log_err()
            .flatten()
    });
    log::warn!("font family {family:?} resolved through the legacy DirectWrite lookup");
    Ok(Some(FontInfo {
        font_family_h,
        font_face,
        features,
        fallbacks,
        font_collection: collection.clone(),
    }))
}

/// Without the requested features when they don't apply: the font still
/// draws, only without tabular numerals and the like.
fn typography(
    font: &Font,
    family: &SharedString,
    factory: &IDWriteFactory3,
) -> Result<IDWriteTypography> {
    let typography = unsafe { factory.CreateTypography() }.context("CreateTypography")?;
    match apply_font_features(&typography, &font.features) {
        Ok(()) => Ok(typography),
        Err(error) => {
            log::warn!("dropping the features of font family {family:?}: {error:#}");
            unsafe { factory.CreateTypography() }.context("CreateTypography")
        }
    }
}
