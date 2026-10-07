// Modified by Longbridge for gpui-fast.
//! Drawing scroll layer tiles where the scene composites them: polychrome
//! sprites whose texture id lies in the range no atlas allocates stand for
//! tiles, and are drawn with the polychrome sprite pipeline from the tiles'
//! own textures instead of an atlas texture.
//!
//! A tile sprite covers the tile's whole texture at whole device pixels, so
//! every fragment samples a texel center and copies the tile's pixel.

use std::ops::Range;

use gpui::{
    AtlasTextureId, LayerKey, PolychromeSprite, Scene, TileCoord, TileId, decode_layer_tile,
};
use windows::Win32::Graphics::Direct3D11::ID3D11ShaderResourceView;

use crate::fast::layers::TileCache;

/// The layer tiles `scene` composites, in the order it lists them.
pub(crate) fn composited_tiles(scene: &Scene) -> impl Iterator<Item = (LayerKey, TileCoord)> + '_ {
    let has_layers = !scene.layers.frames.is_empty();
    scene
        .polychrome_sprites
        .iter()
        .take_while(move |_| has_layers)
        .filter_map(|sprite| decode_layer_tile(sprite.tile.texture_id, sprite.tile.tile_id))
}

/// Whether sprites of `texture` are layer tiles rather than atlas sprites.
pub(crate) fn is_layer_texture(texture: AtlasTextureId) -> bool {
    decode_layer_tile(texture, TileId(0)).is_some()
}

/// Splits a batch of layer tile sprites into runs of the same tile: the
/// scene batches sprites by texture id, which is one per layer, while each
/// tile has a texture of its own.
pub(crate) fn tile_runs(
    sprites: &[PolychromeSprite],
    range: Range<usize>,
) -> impl Iterator<Item = Range<usize>> + '_ {
    let mut start = range.start;
    std::iter::from_fn(move || {
        if start >= range.end {
            return None;
        }
        let tile = sprites[start].tile.tile_id;
        let end = (start..range.end)
            .find(|&index| sprites[index].tile.tile_id != tile)
            .unwrap_or(range.end);
        let run = start..end;
        start = end;
        Some(run)
    })
}

/// The view a run of sprites of one layer tile is drawn from, or `None`
/// when the cache does not hold the tile, which rasterizing every composited
/// tile it lacks before the frame is drawn prevents. The run then draws
/// nothing.
pub(crate) fn texture_for_run<'a>(
    cache: &'a TileCache,
    texture: AtlasTextureId,
    sprites: &[PolychromeSprite],
) -> Option<&'a Option<ID3D11ShaderResourceView>> {
    let first = sprites.first()?;
    debug_assert!(
        sprites
            .iter()
            .all(|sprite| sprite.tile.tile_id == first.tile.tile_id),
        "a run of layer tile sprites spans tiles"
    );
    let (layer, tile) = decode_layer_tile(texture, first.tile.tile_id)?;
    let view = cache.texture(layer, tile).map(|texture| &texture.view);
    if view.is_none() {
        log::debug!("layer {layer:?} tile {tile:?} is composited but not rasterized");
    }
    view
}
