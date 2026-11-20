// Modified by Longbridge for gpui-fast.
//! `WindowBackgroundAppearance::Blurred` as Windows 11's own acrylic.
//!
//! Upstream blurs through the accent policy of `SetWindowCompositionAttribute`,
//! an undocumented Windows 10 API. It blurs what is behind the window but adds
//! none of acrylic's tint, luminosity or noise, so content drawn over it reads
//! only where whatever is behind the window happens to contrast with it: dark
//! text over a dark window behind disappears. Windows 11 22H2 (build 22621)
//! documents acrylic as a system backdrop, `DWMSBT_TRANSIENTWINDOW`, the
//! material its own menus and flyouts use, tinted for the light or dark theme.
//! Builds before it keep upstream's accent policy.
//!
//! A system backdrop stays on the window until something replaces it, so the
//! appearances that draw none (`Opaque`, `Transparent`) put it back to
//! `DWMSBT_AUTO`, the default. That also removes a Mica backdrop the window
//! had before, which upstream leaves in place.

use std::sync::OnceLock;

use windows::Win32::{
    Foundation::HWND,
    Graphics::Dwm::{
        DWM_SYSTEMBACKDROP_TYPE, DWMSBT_AUTO, DWMSBT_TRANSIENTWINDOW, DWMWA_SYSTEMBACKDROP_TYPE,
        DwmSetWindowAttribute,
    },
};

/// The first Windows build with `DWMWA_SYSTEMBACKDROP_TYPE`.
const SYSTEM_BACKDROP_BUILD: u32 = 22621;

/// Blurs what is behind the window: acrylic where the system backdrop exists,
/// upstream's accent policy before it.
pub(crate) fn blur(hwnd: HWND) {
    if has_system_backdrop() {
        // Drop an accent policy an earlier appearance set, so only the backdrop draws.
        crate::window::set_window_composition_attribute(hwnd, None, 0);
        set_system_backdrop(hwnd, DWMSBT_TRANSIENTWINDOW);
    } else {
        crate::window::set_window_composition_attribute(hwnd, Some((0, 0, 0, 0)), 4);
    }
}

/// Removes a system backdrop an earlier appearance set.
pub(crate) fn clear(hwnd: HWND) {
    if has_system_backdrop() {
        set_system_backdrop(hwnd, DWMSBT_AUTO);
    }
}

fn set_system_backdrop(hwnd: HWND, backdrop: DWM_SYSTEMBACKDROP_TYPE) {
    // SAFETY: the attribute's value is a DWM_SYSTEMBACKDROP_TYPE, read for its size.
    let result = unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_SYSTEMBACKDROP_TYPE,
            &backdrop as *const _ as *const _,
            std::mem::size_of_val(&backdrop) as u32,
        )
    };
    if let Err(error) = result {
        log::warn!(
            "Setting the system backdrop to {} failed: {error}",
            backdrop.0
        );
    }
}

/// Whether this Windows has `DWMWA_SYSTEMBACKDROP_TYPE`, asked once per process.
fn has_system_backdrop() -> bool {
    static HAS: OnceLock<bool> = OnceLock::new();
    *HAS.get_or_init(|| {
        let mut version = unsafe { std::mem::zeroed() };
        let status = unsafe { windows::Wdk::System::SystemServices::RtlGetVersion(&mut version) };
        status.is_ok() && version.dwBuildNumber >= SYSTEM_BACKDROP_BUILD
    })
}
