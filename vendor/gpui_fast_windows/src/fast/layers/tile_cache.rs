// Modified by Longbridge for gpui-fast.
//! The textures scroll layer tiles are rasterized into, kept from frame to
//! frame so that a frame that only scrolled draws them again without
//! rasterizing anything.
//!
//! A tile's texture holds the layer's content of one generation. The core
//! bumps a layer's generation whenever it repaints the content and lists the
//! tiles whose pixels changed (`LayerFrame::dirty_tiles`); every other tile
//! of the previous generation stays as it is. A tile the cache has missed a
//! generation of could have changed in it, so it is rasterized again.

use anyhow::{Context as _, Result};
use collections::{FxHashMap, FxHashSet, hash_map::Entry};
use gpui::{LayerKey, SceneLayers, TileCoord};
use windows::Win32::Graphics::{
    Direct3D11::{
        D3D11_BIND_CONSTANT_BUFFER, D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE,
        D3D11_BUFFER_DESC, D3D11_COMPARISON_ALWAYS, D3D11_FILTER_MIN_MAG_MIP_POINT,
        D3D11_FLOAT32_MAX, D3D11_SAMPLER_DESC, D3D11_SUBRESOURCE_DATA, D3D11_TEXTURE_ADDRESS_CLAMP,
        D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, D3D11_USAGE_IMMUTABLE, ID3D11Buffer,
        ID3D11Device, ID3D11RenderTargetView, ID3D11SamplerState, ID3D11ShaderResourceView,
        ID3D11Texture2D,
    },
    Dxgi::Common::DXGI_SAMPLE_DESC,
};

use crate::DirectXRenderer;
use crate::directx_renderer::{GlobalParams, RENDER_TARGET_FORMAT};

/// How much tile texture memory a window keeps, by default (spec §5.6).
pub(crate) const DEFAULT_BUDGET_BYTES: u64 = 64 * 1024 * 1024;

/// Frames a layer that no frame composites keeps its tiles for; the core
/// drops such a layer after as many.
const LAYER_KEEP_FRAMES: u64 = 120;

/// Textures of released tiles kept for new tiles to reuse.
const POOL_LIMIT: usize = 8;

/// A window's scroll layer tiles, and what rasterizing them takes.
pub(crate) struct TileCache {
    tiles: FxHashMap<(LayerKey, TileCoord), Tile>,
    layers: FxHashMap<LayerKey, SeenLayer>,
    pool: Vec<TileTexture>,
    frame: u64,
    budget_bytes: u64,
    /// Per tile size: the global parameters tiles are drawn with.
    globals: FxHashMap<u32, Option<ID3D11Buffer>>,
    /// The sampler tiles are composited with. It takes the nearest texel, so
    /// a fragment copies its tile pixel exactly even when the interpolated
    /// texture coordinate lands a hair off the texel center, where a linear
    /// filter would blend in a neighbour.
    sampler: Option<ID3D11SamplerState>,
}

impl Default for TileCache {
    fn default() -> Self {
        Self {
            tiles: FxHashMap::default(),
            layers: FxHashMap::default(),
            pool: Vec::new(),
            frame: 0,
            budget_bytes: DEFAULT_BUDGET_BYTES,
            globals: FxHashMap::default(),
            sampler: None,
        }
    }
}

struct Tile {
    /// The generation of the content the texture holds, or will hold once
    /// this frame's rasterization runs.
    generation: u64,
    tile_size: u32,
    valid: bool,
    /// The last frame that composited or rasterized the tile.
    last_used: u64,
    texture: Option<TileTexture>,
}

struct SeenLayer {
    generation: u64,
    last_frame: u64,
}

pub(crate) struct TileTexture {
    size: u32,
    /// Read back by the pixel tests; the views keep it alive otherwise.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) texture: ID3D11Texture2D,
    pub(crate) render_target: Option<ID3D11RenderTargetView>,
    pub(crate) view: Option<ID3D11ShaderResourceView>,
}

fn tile_bytes(tile_size: u32) -> u64 {
    tile_size as u64 * tile_size as u64 * 4
}

impl TileCache {
    /// The texture of `tile` of `layer`, if the cache holds its content.
    pub(crate) fn texture(&self, layer: LayerKey, tile: TileCoord) -> Option<&TileTexture> {
        self.tiles
            .get(&(layer, tile))
            .filter(|tile| tile.valid)
            .and_then(|tile| tile.texture.as_ref())
    }

    /// Whether the cache holds anything of a layer, which frames that
    /// composite no layer still age.
    pub(crate) fn has_layers(&self) -> bool {
        !self.layers.is_empty() || !self.tiles.is_empty()
    }

    #[cfg(test)]
    pub(crate) fn holds(&self, layer: LayerKey, tile: TileCoord) -> bool {
        self.tiles
            .get(&(layer, tile))
            .is_some_and(|tile| tile.valid)
    }

    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.tiles.is_empty()
            && self.layers.is_empty()
            && self.pool.is_empty()
            && self.globals.is_empty()
    }

    /// Starts a frame that composites the tiles `composited` of the layers in
    /// `layers`. Returns the tiles to rasterize before they are drawn, each
    /// with the index of its layer in `layers.frames`: every composited tile
    /// the cache does not hold, or holds from an older generation. A dirty
    /// tile the frame does not composite is only marked stale, and waits
    /// until a frame shows it: a scrolling list dirties the tiles at the edge
    /// of its overscan on most frames. They count as held from here on.
    pub(crate) fn begin_frame(
        &mut self,
        layers: &SceneLayers,
        composited: impl Iterator<Item = (LayerKey, TileCoord)>,
    ) -> Vec<(usize, TileCoord)> {
        self.frame += 1;
        let frame = self.frame;
        let mut planned = FxHashSet::default();

        for layer in &layers.frames {
            let seen = self.layers.entry(layer.key).or_insert(SeenLayer {
                generation: layer.generation.wrapping_sub(2),
                last_frame: frame,
            });
            seen.last_frame = frame;
            if seen.generation == layer.generation {
                continue;
            }
            let previous = seen.generation;
            let consecutive = previous.wrapping_add(1) == layer.generation;
            seen.generation = layer.generation;
            let dirty: FxHashSet<TileCoord> = layer.dirty_tiles.iter().copied().collect();
            for ((key, coord), tile) in &mut self.tiles {
                if *key != layer.key {
                    continue;
                }
                if consecutive
                    && tile.valid
                    && tile.generation == previous
                    && tile.tile_size == layer.tile_size
                    && !dirty.contains(coord)
                {
                    tile.generation = layer.generation;
                } else {
                    tile.valid = false;
                }
            }
        }

        let mut indices = FxHashMap::default();
        for (index, layer) in layers.frames.iter().enumerate() {
            indices.insert(layer.key, index);
        }
        for (key, coord) in composited {
            let Some(&index) = indices.get(&key) else {
                debug_assert!(false, "a composited layer tile has no layer frame");
                continue;
            };
            let layer = &layers.frames[index];
            match self.tiles.get_mut(&(key, coord)) {
                Some(tile)
                    if tile.valid
                        && tile.generation == layer.generation
                        && tile.tile_size == layer.tile_size =>
                {
                    tile.last_used = frame;
                }
                _ => {
                    planned.insert((index, coord));
                }
            }
        }

        let mut planned: Vec<(usize, TileCoord)> = planned.into_iter().collect();
        planned.sort();
        for &(index, coord) in &planned {
            let layer = &layers.frames[index];
            let tile = self.tiles.entry((layer.key, coord)).or_insert(Tile {
                generation: layer.generation,
                tile_size: layer.tile_size,
                valid: false,
                last_used: frame,
                texture: None,
            });
            tile.generation = layer.generation;
            tile.tile_size = layer.tile_size;
            tile.valid = true;
            tile.last_used = frame;
        }

        self.drop_unseen_layers();
        planned
    }

    /// Releases the tiles of layers no frame has composited for a while.
    fn drop_unseen_layers(&mut self) {
        let frame = self.frame;
        let gone: Vec<LayerKey> = self
            .layers
            .iter()
            .filter(|(_, seen)| seen.last_frame + LAYER_KEEP_FRAMES < frame)
            .map(|(key, _)| *key)
            .collect();
        if gone.is_empty() {
            return;
        }
        for key in &gone {
            self.layers.remove(key);
        }
        let released: Vec<(LayerKey, TileCoord)> = self
            .tiles
            .keys()
            .filter(|(key, _)| gone.contains(key))
            .copied()
            .collect();
        for key in released {
            if let Some(texture) = self.tiles.remove(&key).and_then(|tile| tile.texture)
                && self.pool.len() < POOL_LIMIT
            {
                self.pool.push(texture);
            }
        }
    }

    /// Releases tiles, least recently used first, until the tiles and the
    /// pool fit in `budget_bytes`. Tiles this frame composites or
    /// rasterizes are kept even when they alone exceed the budget.
    pub(crate) fn evict_to_budget(&mut self, budget_bytes: u64) {
        let mut bytes: u64 = self
            .tiles
            .values()
            .filter(|tile| tile.valid || tile.texture.is_some())
            .map(|tile| tile_bytes(tile.tile_size))
            .sum::<u64>()
            + self
                .pool
                .iter()
                .map(|texture| tile_bytes(texture.size))
                .sum::<u64>();
        while bytes > budget_bytes
            && let Some(texture) = self.pool.pop()
        {
            bytes -= tile_bytes(texture.size);
        }
        if bytes <= budget_bytes {
            return;
        }
        let mut candidates: Vec<(u64, LayerKey, TileCoord)> = self
            .tiles
            .iter()
            .filter(|(_, tile)| tile.last_used != self.frame)
            .map(|((key, coord), tile)| (tile.last_used, *key, *coord))
            .collect();
        candidates.sort_by_key(|&(last_used, key, coord)| (last_used, key.0, coord));
        for (_, key, coord) in candidates {
            if bytes <= budget_bytes {
                break;
            }
            if let Some(tile) = self.tiles.remove(&(key, coord))
                && (tile.valid || tile.texture.is_some())
            {
                bytes -= tile_bytes(tile.tile_size);
            }
        }
    }

    /// Releases every tile and texture, as when the window's size or scale,
    /// or its Direct3D device, changes.
    pub(crate) fn clear(&mut self) {
        self.tiles.clear();
        self.layers.clear();
        self.pool.clear();
        self.globals.clear();
        self.sampler = None;
    }

    /// Gives every tile in `planned` a texture, fits the cache in its
    /// budget, and makes the globals of the frame's tile sizes.
    pub(crate) fn prepare(
        &mut self,
        device: &ID3D11Device,
        layers: &SceneLayers,
        planned: &[(usize, TileCoord)],
    ) -> Result<()> {
        if self.sampler.is_none() {
            self.sampler = create_point_sampler(device)?;
        }
        self.evict_to_budget(self.budget_bytes);
        for &(index, coord) in planned {
            let layer = &layers.frames[index];
            let Some(tile) = self.tiles.get_mut(&(layer.key, coord)) else {
                continue;
            };
            if tile
                .texture
                .as_ref()
                .is_some_and(|texture| texture.size == tile.tile_size)
            {
                continue;
            }
            let size = tile.tile_size;
            let texture = match self.pool.iter().position(|texture| texture.size == size) {
                Some(position) => self.pool.swap_remove(position),
                None => create_tile_texture(device, size)?,
            };
            tile.texture = Some(texture);
        }
        for &(index, _) in planned {
            let tile_size = layers.frames[index].tile_size;
            if let Entry::Vacant(entry) = self.globals.entry(tile_size) {
                entry.insert(create_globals(device, tile_size, tile_size)?);
            }
        }
        Ok(())
    }

    /// The global parameters buffer tiles of `tile_size` are drawn with.
    pub(crate) fn globals(&self, tile_size: u32) -> Option<&Option<ID3D11Buffer>> {
        self.globals.get(&tile_size)
    }
}

/// The frame's global parameters for a target of `width` × `height`: the
/// gamma and contrast `DirectXRenderer::pre_draw` writes, with that viewport.
pub(crate) fn global_params(width: u32, height: u32) -> GlobalParams {
    let font_info = DirectXRenderer::get_font_info();
    GlobalParams {
        gamma_ratios: font_info.gamma_ratios,
        viewport_size: [width as f32, height as f32],
        grayscale_enhanced_contrast: font_info.grayscale_enhanced_contrast,
        subpixel_enhanced_contrast: font_info.subpixel_enhanced_contrast,
        is_bgr: font_info.is_bgr as u32,
        _pad: [0; 3],
    }
}

/// An immutable constant buffer holding [`global_params`] for a target of
/// `width` × `height`.
pub(crate) fn create_globals(
    device: &ID3D11Device,
    width: u32,
    height: u32,
) -> Result<Option<ID3D11Buffer>> {
    let params = global_params(width, height);
    let desc = D3D11_BUFFER_DESC {
        ByteWidth: size_of::<GlobalParams>() as u32,
        Usage: D3D11_USAGE_IMMUTABLE,
        BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32,
        CPUAccessFlags: 0,
        MiscFlags: 0,
        StructureByteStride: 0,
    };
    let data = D3D11_SUBRESOURCE_DATA {
        pSysMem: &params as *const GlobalParams as *const _,
        SysMemPitch: 0,
        SysMemSlicePitch: 0,
    };
    let mut buffer = None;
    unsafe { device.CreateBuffer(&desc, Some(&data), Some(&mut buffer))? };
    Ok(buffer)
}

/// A `size` × `size` texture of the frame's format that can be drawn into
/// and sampled.
impl TileCache {
    /// The sampler tiles are composited with. Composited tiles were
    /// rasterized since the cache was last cleared, which made it.
    pub(crate) fn sampler(&self) -> &Option<ID3D11SamplerState> {
        &self.sampler
    }
}

fn create_point_sampler(device: &ID3D11Device) -> Result<Option<ID3D11SamplerState>> {
    let desc = D3D11_SAMPLER_DESC {
        Filter: D3D11_FILTER_MIN_MAG_MIP_POINT,
        AddressU: D3D11_TEXTURE_ADDRESS_CLAMP,
        AddressV: D3D11_TEXTURE_ADDRESS_CLAMP,
        AddressW: D3D11_TEXTURE_ADDRESS_CLAMP,
        MipLODBias: 0.0,
        MaxAnisotropy: 1,
        ComparisonFunc: D3D11_COMPARISON_ALWAYS,
        BorderColor: [0.0; 4],
        MinLOD: 0.0,
        MaxLOD: D3D11_FLOAT32_MAX,
    };
    let mut sampler = None;
    unsafe { device.CreateSamplerState(&desc, Some(&mut sampler))? };
    Ok(sampler)
}

pub(crate) fn create_tile_texture(device: &ID3D11Device, size: u32) -> Result<TileTexture> {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: size,
        Height: size,
        MipLevels: 1,
        ArraySize: 1,
        Format: RENDER_TARGET_FORMAT,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };
    let mut texture = None;
    unsafe { device.CreateTexture2D(&desc, None, Some(&mut texture))? };
    let texture = texture.context("creating a layer tile texture")?;
    let mut render_target = None;
    unsafe { device.CreateRenderTargetView(&texture, None, Some(&mut render_target))? };
    let mut view = None;
    unsafe { device.CreateShaderResourceView(&texture, None, Some(&mut view))? };
    Ok(TileTexture {
        size,
        texture,
        render_target,
        view,
    })
}
