// Modified by Longbridge for gpui-fast.
//! Everything gpui-fast adds to the Windows platform lives under this module.
//!
//! The rest of this crate is kept as close to upstream GPUI (Zed's
//! `crates/gpui_windows`) as possible, so that pulling in upstream changes
//! stays a matter of merging rather than of rewriting. Upstream files only
//! *call into* this module: a field holding this module's state, a line
//! forwarding a call to it. The logic itself is written here, one file per
//! topic.

pub(crate) mod composition;
pub(crate) mod draw_state;
pub(crate) mod frame;
pub(crate) mod globals;
pub(crate) mod layers;

pub(crate) mod compat;
pub(crate) mod color_glyphs;
pub(crate) mod font_loader;
pub(crate) mod startup;
