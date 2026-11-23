// Modified by Longbridge for gpui-fast.
//! Synthetic italic for faces without an italic style (longbridge/nsh#2529).
//!
//! Upstream opens fonts through `IDWriteFontSet::GetMatchingFonts`, whose
//! references carry no simulations: asking for italic in a family without an
//! italic face (most CJK fonts, Microsoft YaHei among them) gives the upright
//! face, and italic text renders upright. DirectWrite 1's
//! `GetFirstMatchingFont`, browsers and CoreText slant such a face instead.
//! This asks DirectWrite for the same face with its oblique simulation, so it
//! rasterizes, measures and bounds the slanted outline itself; advances are
//! unchanged, so carets and selections are too. The simulated face is a
//! separate `IDWriteFontFace3`, so it gets its own `FontId` and atlas entries.

use gpui::FontStyle;
use gpui_util::ResultExt;
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FONT_SIMULATIONS_OBLIQUE, DWRITE_FONT_STYLE, DWRITE_FONT_STYLE_NORMAL,
    DWRITE_FONT_STYLE_OBLIQUE, IDWriteFontFace3, IDWriteFontFaceReference,
};

/// The face to draw a request for `style` with: `face` itself, or when italic
/// is wanted and `face` is upright, the same face with oblique simulation.
pub(crate) fn face(
    reference: &IDWriteFontFaceReference,
    face: IDWriteFontFace3,
    style: FontStyle,
) -> IDWriteFontFace3 {
    if !matches!(style, FontStyle::Italic | FontStyle::Oblique) {
        return face;
    }
    unsafe {
        if face.GetStyle() != DWRITE_FONT_STYLE_NORMAL
            || (face.GetSimulations() & DWRITE_FONT_SIMULATIONS_OBLIQUE).0 != 0
        {
            return face;
        }
        let simulations = face.GetSimulations() | DWRITE_FONT_SIMULATIONS_OBLIQUE;
        reference
            .CreateFontFaceWithSimulations(simulations)
            .log_err()
            .unwrap_or(face)
    }
}

/// The style `face` draws with, counting an oblique simulation (which a
/// fallback face from DirectWrite's layout may carry) as oblique, so the
/// fallback is matched to a slanted `FontId` rather than the upright one.
pub(crate) fn style_of(face: &IDWriteFontFace3) -> DWRITE_FONT_STYLE {
    unsafe {
        let style = face.GetStyle();
        if style == DWRITE_FONT_STYLE_NORMAL
            && (face.GetSimulations() & DWRITE_FONT_SIMULATIONS_OBLIQUE).0 != 0
        {
            DWRITE_FONT_STYLE_OBLIQUE
        } else {
            style
        }
    }
}
