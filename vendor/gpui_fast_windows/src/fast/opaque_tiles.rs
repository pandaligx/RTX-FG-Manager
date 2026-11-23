// Modified by Longbridge for gpui-fast.
//! Which polychrome atlas tiles are opaque, for overdraw culling
//! ([`crate::fast::occlusion`]): an image whose every pixel is opaque, drawn
//! at full opacity with square corners, hides what is under it.
//!
//! The atlas records each polychrome tile as it uploads it (its alpha
//! scanned once), forgets it when the tile is removed, and records it anew
//! when its space is reused. Where culling is off (on a GPU adapter) nothing
//! is scanned: the renderer turns tracking off ([`DirectXAtlas::track_opaque_tiles`]).

use std::collections::HashSet;

use gpui::{AtlasTextureId, AtlasTextureKind, AtlasTile};

use crate::DirectXAtlas;

/// The atlas's opaque polychrome tiles.
#[derive(Default)]
pub(crate) struct OpaqueTiles {
    off: bool,
    tiles: HashSet<(AtlasTextureId, u32)>,
}

impl OpaqueTiles {
    /// Records whether `tile`, just uploaded with `bytes` (BGRA, alpha
    /// last), is opaque.
    pub(crate) fn record(&mut self, kind: AtlasTextureKind, tile: &AtlasTile, bytes: &[u8]) {
        if self.off || kind != AtlasTextureKind::Polychrome {
            return;
        }
        let key = (tile.texture_id, tile.tile_id.0);
        if bytes
            .iter()
            .skip(3)
            .step_by(4)
            .all(|&alpha| alpha == u8::MAX)
        {
            self.tiles.insert(key);
        } else {
            self.tiles.remove(&key);
        }
    }

    pub(crate) fn forget(&mut self, tile: &AtlasTile) {
        self.tiles.remove(&(tile.texture_id, tile.tile_id.0));
    }

    pub(crate) fn clear(&mut self) {
        self.tiles.clear();
    }
}

impl DirectXAtlas {
    /// Whether every pixel of `tile`, a polychrome tile, is opaque.
    pub(crate) fn is_opaque(&self, tile: &AtlasTile) -> bool {
        let state = self.0.lock();
        let opaque = &state.backend.fast_opaque_tiles;
        !opaque.off && opaque.tiles.contains(&(tile.texture_id, tile.tile_id.0))
    }

    /// Turns the recording of opaque tiles on or off (off forgets them).
    pub(crate) fn track_opaque_tiles(&self, on: bool) {
        let mut state = self.0.lock();
        let opaque = &mut state.backend.fast_opaque_tiles;
        opaque.off = !on;
        if !on {
            opaque.tiles.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use gpui::{AtlasKey, DevicePixels, ImageId, PlatformAtlas, RenderImageParams, Size};
    use windows::Win32::{
        Foundation::HMODULE,
        Graphics::{
            Direct3D::D3D_DRIVER_TYPE_WARP,
            Direct3D11::{D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION, D3D11CreateDevice},
        },
    };

    use crate::DirectXAtlas;

    fn atlas() -> Option<DirectXAtlas> {
        let mut device = None;
        let mut context = None;
        unsafe {
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_WARP,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )
        }
        .ok()?;
        Some(DirectXAtlas::new(&device?, &context?))
    }

    fn key(id: usize) -> AtlasKey {
        AtlasKey::Image(RenderImageParams {
            image_id: ImageId(id),
            frame_index: 0,
        })
    }

    fn upload(atlas: &DirectXAtlas, key: AtlasKey, alpha: u8) -> gpui::AtlasTile {
        let size = Size {
            width: DevicePixels(32),
            height: DevicePixels(32),
        };
        atlas
            .get_or_insert_with(key, &mut || {
                let pixels = (0..32 * 32).flat_map(|_| [10, 20, 30, alpha]).collect();
                Ok(Some((size, Cow::Owned(pixels))))
            })
            .expect("allocation should succeed")
            .expect("callback returns Some")
    }

    #[test]
    fn opaque_images_are_recorded_and_a_reused_tile_is_recorded_anew() {
        let Some(atlas) = atlas() else {
            return;
        };
        let opaque = upload(&atlas, key(11), 255);
        assert!(atlas.is_opaque(&opaque));
        let translucent = upload(&atlas, key(12), 128);
        assert!(!atlas.is_opaque(&translucent));
        // A removed tile is forgotten; its space, reused, is recorded anew.
        atlas.remove(&key(11));
        assert!(!atlas.is_opaque(&opaque));
        let reused = upload(&atlas, key(13), 0);
        assert!(!atlas.is_opaque(&reused));
    }

    #[test]
    fn nothing_is_recorded_while_tracking_is_off() {
        let Some(atlas) = atlas() else {
            return;
        };
        atlas.track_opaque_tiles(false);
        let tile = upload(&atlas, key(21), 255);
        assert!(!atlas.is_opaque(&tile));
        atlas.track_opaque_tiles(true);
        let tile = upload(&atlas, key(22), 255);
        assert!(atlas.is_opaque(&tile));
    }
}
