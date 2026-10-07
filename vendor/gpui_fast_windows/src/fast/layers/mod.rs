// Modified by Longbridge for gpui-fast.
//! Scroll layer tiles on Direct3D 11: the textures a layer's content is
//! rasterized into, and the draws that composite them where the scene puts
//! them. The core side is `gpui::fast::layers`; see
//! docs/superpowers/specs/2026-09-30-scroll-layers-design.md, §5, and
//! `gpui_wgpu::fast::layers`, which this follows.

pub(crate) mod composite;
pub(crate) mod raster;
pub(crate) mod tile_cache;

pub(crate) use tile_cache::TileCache;

use crate::DirectXRenderer;

/// Forwarded to when the window's size or scale factor, or its Direct3D
/// device, changes: releases every tile, whose pixels or device no longer
/// fit it.
pub(crate) fn release_tiles(renderer: &mut DirectXRenderer) {
    renderer.fast_frame.layers.clear();
}

#[cfg(test)]
mod tests;
