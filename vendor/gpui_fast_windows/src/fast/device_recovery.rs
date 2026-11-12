// Modified by Longbridge for gpui-fast.
//! Keeps a window alive when its renderer fails to resize or to recover from a lost device.
//!
//! Upstream treats any failed resize as a lost device, and panics when a window's renderer
//! cannot be rebuilt on the devices the platform recreated. Both crash in the field:
//!
//! - a resize that runs out of memory (`E_OUTOFMEMORY`) on a healthy device invalidates the
//!   devices, and the rebuild then allocates the same size and fails the same way;
//! - after a GPU reset (`DXGI_ERROR_DEVICE_RESET`), the renderer's retries all reuse the one set
//!   of devices the platform recreated, which a driver still recovering may lose again.
//!
//! So a resize is retried at the next frame unless the device is lost, a failed rebuild asks the
//! platform for new devices a few times before giving up, and a renderer left without its
//! devices or render target skips drawing instead of unwrapping them.

use std::sync::atomic::Ordering;

use anyhow::Error;
use gpui::{DevicePixels, Size};

use crate::{DirectXDevices, DirectXRenderer, WindowsWindowInner};

/// How many times in a row a window asks the platform for new devices before it gives up.
const MAX_FAILED_RECOVERIES: u32 = 5;

#[derive(Default)]
pub(crate) struct RecoveryState {
    /// Rebuilds that failed since the window last recovered.
    failed_recoveries: u32,
    /// The size a failed resize left the renderer without buffers for.
    pending_resize: Option<Size<DevicePixels>>,
}

impl RecoveryState {
    fn recovered(&mut self) {
        self.failed_recoveries = 0;
        self.pending_resize = None;
    }

    /// Counts a failed rebuild; `false` once the window has used up its attempts.
    fn failed(&mut self) -> bool {
        self.failed_recoveries += 1;
        self.failed_recoveries < MAX_FAILED_RECOVERIES
    }
}

/// Forwarded to by `WindowsWindowInner::handle_size_change`.
pub(crate) fn resize(window: &WindowsWindowInner, size: Size<DevicePixels>) {
    resize_renderer(window, size, false);
}

/// Forwarded to by `WindowsWindowInner::draw_window`: retries a resize that failed.
pub(crate) fn retry_resize(window: &WindowsWindowInner) {
    let pending = window.state.renderer.borrow().fast_recovery.pending_resize;
    if let Some(size) = pending {
        resize_renderer(window, size, true);
    }
}

fn resize_renderer(window: &WindowsWindowInner, size: Size<DevicePixels>, retry: bool) {
    let mut renderer = window.state.renderer.borrow_mut();
    if renderer.fast_recovery.pending_resize.is_some() {
        // `resize` skips the size it already holds, and it records the size before resizing.
        renderer.width = 0;
    }
    let Err(error) = renderer.resize(size) else {
        renderer.fast_recovery.pending_resize = None;
        return;
    };
    if is_device_lost(&renderer) {
        log::error!("Failed to resize renderer, invalidating devices: {error:#}");
        renderer.fast_recovery.pending_resize = None;
        window
            .state
            .invalidate_devices
            .store(true, Ordering::Release);
    } else {
        log::error!(
            "Failed to resize renderer to {}x{}, retrying at the next frame: {error:#}",
            size.width.0,
            size.height.0
        );
        renderer.fast_recovery.pending_resize = Some(size);
        if retry {
            // Retrying every frame would spin on a size that cannot be allocated yet.
            return;
        }
    }
    drop(renderer);
    crate::fast::frame_schedule::FrameRequest::request(&window.state.fast_frame_request);
}

/// Forwarded to by `WindowsWindowInner::handle_device_lost`: rebuilds the renderer on the
/// platform's new devices, and returns whether it can draw again.
pub(crate) fn recover(window: &WindowsWindowInner, devices: &DirectXDevices) -> bool {
    let mut renderer = window.state.renderer.borrow_mut();
    let Err(error) = renderer.handle_device_lost(devices) else {
        renderer.fast_recovery.recovered();
        return true;
    };
    let device_lost = unsafe { devices.device.GetDeviceRemovedReason() }.err();
    let retry = renderer.fast_recovery.failed();
    log::error!(
        "Failed to recover renderer from lost device (attempt {} of {MAX_FAILED_RECOVERIES}), \
         new device lost: {device_lost:?}: {error:#}",
        renderer.fast_recovery.failed_recoveries,
    );
    if !retry {
        recovery_failed(error);
    }
    drop(renderer);
    window
        .state
        .invalidate_devices
        .store(true, Ordering::Release);
    crate::fast::frame_schedule::FrameRequest::request(&window.state.fast_frame_request);
    false
}

/// Whether the renderer has the devices and the render target a frame draws with: a failed
/// rebuild or resize leaves it without them until it recovers.
pub(crate) fn can_draw(renderer: &DirectXRenderer) -> bool {
    renderer.devices.is_some()
        && renderer
            .resources
            .as_ref()
            .is_some_and(|resources| resources.render_target_view.is_some())
}

/// Panics with the whole chain of `error`, which names the step that failed and its `HRESULT`.
pub(crate) fn recovery_failed(error: Error) -> ! {
    panic!("Device lost: {error:#}");
}

fn is_device_lost(renderer: &DirectXRenderer) -> bool {
    renderer
        .devices
        .as_ref()
        .is_none_or(|devices| unsafe { devices.device.GetDeviceRemovedReason() }.is_err())
}

#[cfg(test)]
mod tests {
    use super::{MAX_FAILED_RECOVERIES, RecoveryState};

    #[test]
    fn a_window_gives_up_after_its_failed_recoveries() {
        let mut state = RecoveryState::default();
        for _ in 1..MAX_FAILED_RECOVERIES {
            assert!(state.failed());
        }
        assert!(!state.failed());
    }

    #[test]
    fn recovering_resets_the_attempts() {
        let mut state = RecoveryState::default();
        for _ in 1..MAX_FAILED_RECOVERIES {
            assert!(state.failed());
        }
        state.recovered();
        assert!(state.failed());
    }
}
