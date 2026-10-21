// Modified by Longbridge for gpui-fast.
//! The names gpui-fast registers its window classes under.
//!
//! A window class is registered once per process and module, so two copies of
//! GPUI's Windows platform in one executable — gpui-fast next to an upstream
//! GPUI such as `gpui-pre` — would otherwise share upstream's classes: the
//! second copy's windows would be created with the first copy's window
//! procedure, which reads their creation context as its own type. Since
//! upstream's platform window became a hidden top-level window, that window
//! procedure handles enough messages during creation to crash. Names of our
//! own keep each copy's windows with its own procedure.

use windows::core::{PCWSTR, w};

/// The class of the hidden window that receives the platform's messages,
/// upstream's `Zed::PlatformWindow`.
pub(crate) const PLATFORM_WINDOW: PCWSTR = w!("GpuiFast::PlatformWindow");

/// The class of GPUI's windows, upstream's `Zed::Window`.
pub(crate) const WINDOW: PCWSTR = w!("GpuiFast::Window");
