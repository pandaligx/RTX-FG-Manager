//! Resolve Windows 10 1607 DPI helpers optionally for 1507/1511.
use std::sync::OnceLock;
use windows::{
    Win32::{
        Foundation::HWND,
        Graphics::Gdi::{
            GetDC, GetDeviceCaps, LOGPIXELSX, MONITOR_DEFAULTTONEAREST, MonitorFromWindow,
            ReleaseDC,
        },
        System::LibraryLoader::{GetModuleHandleW, GetProcAddress},
        UI::{
            HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI},
            WindowsAndMessaging::{GetSystemMetrics, SYSTEM_METRICS_INDEX},
        },
    },
    core::{s, w},
};

pub(crate) unsafe fn get_dpi_for_window(hwnd: HWND) -> u32 {
    type GetDpi = unsafe extern "system" fn(HWND) -> u32;
    static API: OnceLock<Option<GetDpi>> = OnceLock::new();
    let api = API.get_or_init(|| unsafe {
        let module = GetModuleHandleW(w!("user32.dll")).ok()?;
        GetProcAddress(module, s!("GetDpiForWindow"))
            .map(|proc| std::mem::transmute::<unsafe extern "system" fn() -> isize, GetDpi>(proc))
    });
    if let Some(api) = api {
        let dpi = unsafe { api(hwnd) };
        if dpi != 0 {
            return dpi;
        }
    }
    let mut x = 96;
    let mut y = 96;
    let monitor = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
    if unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut x, &mut y) }.is_ok() && x != 0 {
        x
    } else {
        96
    }
}

pub(crate) unsafe fn get_system_metrics_for_dpi(index: SYSTEM_METRICS_INDEX, dpi: u32) -> i32 {
    type GetMetrics = unsafe extern "system" fn(SYSTEM_METRICS_INDEX, u32) -> i32;
    static API: OnceLock<Option<GetMetrics>> = OnceLock::new();
    let api = API.get_or_init(|| unsafe {
        let module = GetModuleHandleW(w!("user32.dll")).ok()?;
        GetProcAddress(module, s!("GetSystemMetricsForDpi")).map(|proc| {
            std::mem::transmute::<unsafe extern "system" fn() -> isize, GetMetrics>(proc)
        })
    });
    if let Some(api) = api {
        return unsafe { api(index, dpi) };
    }
    let dc = unsafe { GetDC(None) };
    let system_dpi = if dc.is_invalid() {
        96
    } else {
        let dpi = unsafe { GetDeviceCaps(Some(dc), LOGPIXELSX) };
        unsafe { ReleaseDC(None, dc) };
        if dpi > 0 { dpi } else { 96 }
    };
    let value = unsafe { GetSystemMetrics(index) };
    ((value as i64 * dpi as i64 + system_dpi as i64 / 2) / system_dpi as i64) as i32
}
