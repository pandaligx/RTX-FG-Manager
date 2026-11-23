// Modified by Longbridge for gpui-fast.
//! Create a window on the display it opens on, at the size it opens with.
//!
//! Upstream creates every window with `CW_USEDEFAULT`, which puts it on the
//! primary display, then moves it into place with `SetWindowPlacement`, its
//! physical rect computed with the target display's scale factor. When the
//! target display's scale factor differs from the primary's, the move crosses
//! a DPI boundary: Windows sends `WM_DPICHANGED` with a suggested rect scaled
//! once more by the ratio of the two DPIs, and the window applies it. A window
//! asked for 800×600 on a 150% display beside a 100% primary opens at
//! 1200×900, and a window restoring bounds it saved grows by that ratio every
//! time it opens.
//!
//! Creating the window at its final physical rect places it on the target
//! display from the start, so it gets that display's DPI when it is created
//! and the placement that follows never crosses a DPI boundary. The rect is
//! the client area; `SetWindowPlacement` still adds the frame afterwards, on
//! the same display.

use gpui::{Bounds, Pixels, PlatformDisplay};

use crate::display::WindowsDisplay;

/// The physical rect, `(x, y, width, height)`, to pass to `CreateWindowExW`
/// for a window placed at `bounds` on `display`. Bounds that are not on
/// `display` fall back to its default bounds, as `retrieve_window_placement`
/// does when it places the window afterwards.
pub(crate) fn create_rect(display: WindowsDisplay, bounds: Bounds<Pixels>) -> (i32, i32, i32, i32) {
    let bounds = if display.check_given_bounds(bounds) {
        bounds
    } else {
        display.default_bounds()
    };
    let bounds = bounds.to_device_pixels(display.scale_factor());
    (
        bounds.origin.x.0,
        bounds.origin.y.0,
        bounds.size.width.0,
        bounds.size.height.0,
    )
}

#[cfg(test)]
mod tests {
    use std::{
        rc::Rc,
        sync::{Arc, Mutex},
        time::Duration,
    };

    use gpui::{
        AppContext, Application, Bounds, Context, IntoElement, ParentElement, QuitMode, Render,
        Styled, Window, WindowBounds, WindowOptions, div, rgb, size,
    };

    struct Blank;

    impl Render for Blank {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .size_full()
                .bg(rgb(0x243654))
                .child("initial placement")
        }
    }

    /// Opens an 800×600 window centered on every display and checks it opened
    /// on that display at that size. Only a desktop with displays at different
    /// scale factors exercises the fix: on the others it passes either way.
    #[test]
    #[ignore = "interactive desktop; needs displays at different scale factors to be meaningful"]
    fn window_opens_at_its_size_on_every_display() {
        let failures = Arc::new(Mutex::new(Vec::new()));
        let done = Arc::new(Mutex::new(false));
        let (failures_in_app, done_in_app) = (failures.clone(), done.clone());
        Application::with_platform(Rc::new(crate::WindowsPlatform::new(false).unwrap())).run(
            move |cx| {
                cx.set_quit_mode(QuitMode::Explicit);
                let requested = size(gpui::px(800.), gpui::px(600.));
                let windows = cx
                    .displays()
                    .into_iter()
                    .map(|display| {
                        let id = display.id();
                        let bounds = Bounds::centered(Some(id), requested, cx);
                        let window = cx
                            .open_window(
                                WindowOptions {
                                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                                    display_id: Some(id),
                                    ..Default::default()
                                },
                                |_, cx| cx.new(|_| Blank),
                            )
                            .unwrap();
                        (id, window)
                    })
                    .collect::<Vec<_>>();
                cx.spawn(async move |cx| {
                    cx.background_executor()
                        .timer(Duration::from_millis(500))
                        .await;
                    for (id, window) in windows {
                        let (display, opened) = window
                            .update(cx, |_, window, cx| {
                                (
                                    window.display(cx).map(|display| display.id()),
                                    window.window_bounds().get_bounds().size,
                                )
                            })
                            .unwrap();
                        let off = (f32::from(opened.width) - f32::from(requested.width)).abs()
                            > 1.
                            || (f32::from(opened.height) - f32::from(requested.height)).abs() > 1.;
                        if display != Some(id) || off {
                            failures_in_app.lock().unwrap().push(format!(
                                "display {id:?}: opened on {display:?} at {opened:?}, asked for {requested:?}"
                            ));
                        }
                        window
                            .update(cx, |_, window, _| window.remove_window())
                            .unwrap();
                    }
                    *done_in_app.lock().unwrap() = true;
                    cx.update(|cx| cx.quit());
                })
                .detach();
            },
        );
        assert!(*done.lock().unwrap(), "the check did not complete");
        let failures = failures.lock().unwrap();
        assert!(failures.is_empty(), "{failures:#?}");
    }
}
