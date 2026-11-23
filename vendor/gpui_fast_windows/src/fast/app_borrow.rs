// Modified by Longbridge for gpui-fast.
//! Defers GPUI event handling while the app's `AppCell` is borrowed.
//!
//! Some synchronous OS calls pump the thread's message queue while GPUI
//! still holds a borrow of the app: WebView2 creation
//! (`webview2_com::wait_with_pump`), `Command::spawn`. A `WM_GPUI_*`
//! message whose handler calls into the app (a foreground task's
//! `AsyncApp::update`, a platform callback) would panic there on the second
//! `AppCell::borrow_mut`.
//!
//! Such a message is consumed and queued here instead. Re-posting it from the
//! handler would not work: a nested pump that keeps dispatching delivers the
//! copy while the app is still borrowed, and spins. The queue is drained,
//! oldest first, by calling the handlers directly as soon as a message is
//! handled with the app no longer borrowed:
//!
//! - after each message the platform's own loop dispatches;
//! - before any later `WM_GPUI_*` message, so a newer event never overtakes
//!   a deferred one;
//! - on a timer on the platform window, armed while the queue is not empty,
//!   so OS modal loops (window move and resize, menus, `MessageBox`), which
//!   dispatch messages without returning to the platform's loop, drain it as
//!   well.
//!
//! `WM_GPUI_CLOSE_ONE_WINDOW` and `WM_GPUI_GPU_DEVICE_LOST` only touch
//! platform state, never the app, so they are handled at once even while the
//! app is borrowed.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;

use gpui_util::ResultExt;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{KillTimer, SetTimer};

use crate::{
    WM_GPUI_DISPLAYS_CHANGED, WM_GPUI_DOCK_MENU_ACTION, WM_GPUI_KEYBOARD_LAYOUT_CHANGED,
    WM_GPUI_TASK_DISPATCHED_ON_MAIN_THREAD,
};

/// `WM_TIMER` id, on the platform window, of the timer that drains the queue.
pub(crate) const DEFERRED_EVENTS_TIMER_ID: usize = 1;

/// How often the timer retries while the app stays borrowed, in milliseconds.
const RETRY_MS: u32 = 10;

thread_local! {
    // Every `WM_GPUI_*` message is handled on the platform's thread.
    static DEFERRED: RefCell<VecDeque<(u32, LPARAM)>> = const { RefCell::new(VecDeque::new()) };
    static TIMER_ARMED: Cell<bool> = const { Cell::new(false) };
}

/// Whether handling `message` calls into the app.
fn calls_into_app(message: u32) -> bool {
    matches!(
        message,
        WM_GPUI_TASK_DISPATCHED_ON_MAIN_THREAD
            | WM_GPUI_DOCK_MENU_ACTION
            | WM_GPUI_KEYBOARD_LAYOUT_CHANGED
            | WM_GPUI_DISPLAYS_CHANGED
    )
}

/// Queues `message` when its handler calls into the app and the app is
/// borrowed. Returns whether it did; the caller then returns `Some(0)`
/// without handling the message.
pub(crate) fn defer_while_app_borrowed(
    platform_window: HWND,
    message: u32,
    _wparam: WPARAM,
    lparam: LPARAM,
) -> bool {
    if !calls_into_app(message) || !gpui::app_is_borrowed() {
        return false;
    }
    DEFERRED.with_borrow_mut(|queue| queue.push_back((message, lparam)));
    if !TIMER_ARMED.replace(true) {
        // SAFETY: `platform_window` is the platform's live window, owned by
        // this thread.
        if unsafe {
            SetTimer(
                Some(platform_window),
                DEFERRED_EVENTS_TIMER_ID,
                RETRY_MS,
                None,
            )
        } == 0
        {
            TIMER_ARMED.set(false);
            log::error!(
                "unable to arm the deferred GPUI events timer: {}",
                std::io::Error::last_os_error()
            );
        }
    }
    true
}

/// The oldest deferred message, once the app can be updated again. Stops the
/// timer when the queue is empty.
pub(crate) fn next_deferred(platform_window: HWND) -> Option<(u32, LPARAM)> {
    if DEFERRED.with_borrow(VecDeque::is_empty) {
        if TIMER_ARMED.replace(false) {
            // SAFETY: as in `defer_while_app_borrowed`.
            unsafe { KillTimer(Some(platform_window), DEFERRED_EVENTS_TIMER_ID) }.log_err();
        }
        return None;
    }
    if gpui::app_is_borrowed() {
        return None;
    }
    DEFERRED.with_borrow_mut(VecDeque::pop_front)
}

/// Validates and handles a GPUI event after draining any older deferred work.
pub(crate) fn handle_event(
    platform: &crate::platform::WindowsPlatformInner,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> Option<isize> {
    if wparam.0 != platform.validation_number {
        log::error!("Wrong validation number while processing message: {message}");
        return None;
    }
    let platform_window = platform.dispatcher.platform_window_handle.as_raw();
    if defer_while_app_borrowed(platform_window, message, wparam, lparam) {
        return Some(0);
    }
    drain(platform);
    platform.fast_dispatch_gpui_event(message, lparam)
}

/// Handles only the timer owned by the deferred event queue.
pub(crate) fn handle_timer(
    platform: &crate::platform::WindowsPlatformInner,
    wparam: WPARAM,
) -> Option<isize> {
    if wparam.0 != DEFERRED_EVENTS_TIMER_ID {
        return None;
    }
    drain(platform);
    Some(0)
}

/// Handles deferred events oldest first, until the queue empties or the app is borrowed again.
pub(crate) fn drain(platform: &crate::platform::WindowsPlatformInner) {
    let platform_window = platform.dispatcher.platform_window_handle.as_raw();
    while let Some((message, lparam)) = next_deferred(platform_window) {
        platform.fast_dispatch_gpui_event(message, lparam);
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;
    use std::time::{Duration, Instant};

    use gpui::{App, Application, AsyncApp, QuitMode};
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, KillTimer, MSG, PM_REMOVE, PeekMessageW, SetTimer, TranslateMessage,
    };

    thread_local! {
        static APP: RefCell<Option<AsyncApp>> = const { RefCell::new(None) };
        static LAUNCH_TASK_RAN: RefCell<Option<Rc<Cell<bool>>>> = const { RefCell::new(None) };
        static FAILURE: Cell<Option<&'static str>> = const { Cell::new(None) };
    }

    /// Dispatches this thread's messages until `done` or `ms` have passed,
    /// as WebView2 creation or an OS modal loop does.
    fn pump(ms: u64, done: impl Fn() -> bool) {
        let start = Instant::now();
        let mut msg = MSG::default();
        while !done() && start.elapsed() < Duration::from_millis(ms) {
            // SAFETY: dispatches this thread's own queue.
            unsafe {
                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// Spawns a foreground task that updates the app; the cell says whether
    /// it has run.
    fn spawn_update(cx: &mut App) -> Rc<Cell<bool>> {
        let ran = Rc::new(Cell::new(false));
        let flag = ran.clone();
        cx.spawn(async move |cx: &mut AsyncApp| cx.update(|_| flag.set(true)))
            .detach();
        ran
    }

    fn fail(reason: &'static str) {
        if FAILURE.get().is_none() {
            FAILURE.set(Some(reason));
        }
    }

    /// Stands in for an OS modal loop (window move, menu, `MessageBox`): a
    /// callback outside any app borrow and outside the platform's loop. It
    /// defers a task from a nested pump inside a borrow, then pumps without
    /// the borrow; the task must run inside that pump.
    unsafe extern "system" fn modal_loop(_: HWND, _: u32, id: usize, _: u32) {
        // SAFETY: the thread timer set by the test, on this thread.
        unsafe { KillTimer(None, id) }.ok();
        if !LAUNCH_TASK_RAN.with_borrow(|ran| ran.as_ref().is_some_and(|ran| ran.get())) {
            fail("the task deferred during launch did not run from the platform's loop");
        }
        let app = APP.with_borrow(|app| app.clone().unwrap());
        let ran = app.update(|cx| {
            let ran = spawn_update(cx);
            pump(200, || false);
            ran
        });
        if ran.get() {
            fail("a task ran inside the app borrow");
        }
        pump(2_000, || ran.get());
        if !ran.get() {
            fail("a task deferred before a modal loop did not run inside it");
        }
        app.update(|cx| cx.quit());
    }

    #[test]
    #[ignore = "runs the Windows message loop: cargo test -p gpui_windows -- --ignored nested_pump"]
    fn nested_pump_while_app_borrowed() {
        std::thread::spawn(|| {
            std::thread::sleep(Duration::from_secs(60));
            eprintln!("nested_pump_while_app_borrowed: timed out");
            std::process::exit(90);
        });
        Application::with_platform(Rc::new(crate::WindowsPlatform::new(false).unwrap())).run(
            |cx| {
                cx.set_quit_mode(QuitMode::Explicit);
                // The launch callback holds the app borrow, as `Window::draw`
                // does when a web view is built in `prepaint`. The task's
                // `update` used to panic inside this pump.
                let ran = spawn_update(cx);
                pump(200, || false);
                if ran.get() {
                    fail("a task ran inside the launch borrow");
                }
                LAUNCH_TASK_RAN.with_borrow_mut(|slot| *slot = Some(ran));
                APP.with_borrow_mut(|app| *app = Some(cx.to_async()));
                // SAFETY: a thread timer; `modal_loop` matches `TIMERPROC`.
                unsafe { SetTimer(None, 0, 100, Some(modal_loop)) };
            },
        );
        assert_eq!(FAILURE.get(), None);
    }
}
