//! Manager startup: draw the final client size before exposing the HWND.
use anyhow::Result;
use gpui::{PlatformWindow, Scene};
use gpui_util::ResultExt;
use windows::Win32::{
    Foundation::{HWND, LPARAM, WPARAM},
    UI::WindowsAndMessaging::{PostMessageW, SW_HIDE, SetWindowPlacement, WINDOWPLACEMENT},
};

use crate::{WM_GPUI_FORCE_UPDATE_WINDOW, WindowsWindow};

pub(crate) fn prepare_hidden(hwnd: HWND, placement: &WINDOWPLACEMENT) -> Result<()> {
    let mut hidden_placement = *placement;
    hidden_placement.showCmd = SW_HIDE.0 as u32;
    // Configure the final swap-chain size without flashing an unpainted window.
    unsafe { SetWindowPlacement(hwnd, &hidden_placement)? };
    Ok(())
}

pub(crate) fn request_first_frame(window: &WindowsWindow) {
    if window.state.show_after_first_present.get() {
        // Hidden windows do not reliably receive WM_PAINT. This is posted only
        // after GPUI has installed the root view and its request-frame callback.
        unsafe {
            PostMessageW(
                Some(window.get_raw_handle()),
                WM_GPUI_FORCE_UPDATE_WINDOW,
                WPARAM(0),
                LPARAM(0),
            )
        }
        .log_err();
    }
}

pub(crate) fn draw(window: &WindowsWindow, scene: &Scene) {
    let presented = {
        let mut renderer = window.state.renderer.borrow_mut();
        let drawable = !renderer.skip_draws;
        renderer
            .draw(scene, window.state.background_appearance.get())
            .log_err()
            .is_some()
            && drawable
    };
    complete_frame(window, presented);
}

pub(crate) fn complete_frame(window: &WindowsWindow, presented: bool) {
    if presented && window.state.show_after_first_present.replace(false) {
        let inner = window.0.clone();
        // SetWindowPlacement synchronously dispatches size/focus messages.
        // Wait until the renderer borrow and GPUI's draw coordinator unwind.
        window
            .executor
            .spawn(async move {
                inner.set_window_placement().log_err();
            })
            .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::prepare_hidden;
    use windows::{
        Win32::{
            Foundation::{HWND, RECT},
            UI::WindowsAndMessaging::{
                CreateWindowExW, DestroyWindow, GetWindowRect, IsWindowVisible,
                SW_SHOWNORMAL, WINDOW_EX_STYLE, WINDOWPLACEMENT, WS_OVERLAPPEDWINDOW,
            },
        },
        core::w,
    };

    #[test]
    fn hidden_window_placement_applies_size_without_showing_a_frame() {
        struct TestWindow(HWND);
        impl Drop for TestWindow {
            fn drop(&mut self) {
                unsafe { DestroyWindow(self.0) }.unwrap();
            }
        }
        unsafe {
            let window = TestWindow(
                CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    w!("STATIC"),
                    w!("Startup placement test"),
                    WS_OVERLAPPEDWINDOW,
                    0, 0, 320, 240,
                    None, None, None, None,
                )
                .unwrap(),
            );
            let placement = WINDOWPLACEMENT {
                length: size_of::<WINDOWPLACEMENT>() as u32,
                showCmd: SW_SHOWNORMAL.0 as u32,
                rcNormalPosition: RECT { left: 100, top: 100, right: 740, bottom: 580 },
                ..Default::default()
            };
            prepare_hidden(window.0, &placement).unwrap();
            assert!(!IsWindowVisible(window.0).as_bool());
            let mut rect = RECT::default();
            GetWindowRect(window.0, &mut rect).unwrap();
            assert_eq!(rect.right - rect.left, 640);
            assert_eq!(rect.bottom - rect.top, 480);
        }
    }
}
