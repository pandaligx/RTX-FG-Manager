// Modified by Longbridge for gpui-fast.
//! Rasterizing scroll layer tiles before the frame draws them: each tile is
//! the layer's content over the tile, translated into the tile's space,
//! drawn into the tile's texture cleared to the layer's baked background,
//! with the frame's pipelines, a viewport the size of the tile, and global
//! parameters whose gamma and contrast are the frame's own.
//!
//! All position-dependent shading is relative to each primitive's own bounds,
//! and the tile is a whole number of device pixels from the content's
//! origin, so a tile holds exactly the pixels the content would have drawn
//! over the same background in the window.
//!
//! Tiles are drawn before `DirectXRenderer::pre_draw`, which then binds the
//! window's render target, viewport and globals again, and before the
//! frame's own instances are uploaded, which the tiles' uploads would
//! otherwise overwrite.

use std::slice;

use anyhow::{Context as _, Result};
use gpui::{Rgba, Scene};
use windows::Win32::Graphics::Direct3D11::{D3D11_VIEWPORT, ID3D11ShaderResourceView};

use crate::DirectXRenderer;
use crate::directx_renderer::DirectXRenderPipelines;
use crate::fast::frame::{FrameState, Target, draw_scene, upload};
use crate::fast::layers::composite;

/// Forwarded to by `DirectXRenderer::render` before it draws the frame:
/// rasterizes the layer tiles `scene` needs that the cache does not hold.
pub(crate) fn rasterize_tiles(renderer: &mut DirectXRenderer, scene: &Scene) -> Result<()> {
    if scene.layers.frames.is_empty() && !renderer.fast_frame.layers.has_layers() {
        return Ok(());
    }
    let devices = renderer.devices.as_ref().context("devices missing")?;
    let target = Target {
        device: &devices.device,
        device_context: &devices.device_context,
        atlas: &renderer.atlas,
        globals: &renderer.globals,
        resources: None,
    };
    rasterize(
        &target,
        &mut renderer.pipelines,
        &mut renderer.fast_frame,
        scene,
    )
}

/// Starts the frame of `frame`'s tile cache for `scene` and rasterizes the
/// tiles it plans, leaving no render target bound.
pub(crate) fn rasterize(
    target: &Target<'_>,
    pipelines: &mut DirectXRenderPipelines,
    frame: &mut FrameState,
    scene: &Scene,
) -> Result<()> {
    let planned = frame
        .layers
        .begin_frame(&scene.layers, composite::composited_tiles(scene));
    if planned.is_empty() {
        return Ok(());
    }
    let result = frame
        .layers
        .prepare(target.device, &scene.layers, &planned)
        .and_then(|()| {
            unbind_textures(target, frame);
            planned.iter().try_for_each(|&(index, tile)| {
                let layer = &scene.layers.frames[index];
                let tile_scene = layer.tile_scene(tile);
                let texture = frame
                    .layers
                    .texture(layer.key, tile)
                    .context("a planned layer tile was not prepared")?;
                let render_target = texture.render_target.clone();
                let globals = frame
                    .layers
                    .globals(layer.tile_size)
                    .context("a planned layer tile has no globals")?
                    .clone();
                let context = target.device_context;
                let side = layer.tile_size as f32;
                unsafe {
                    context.OMSetRenderTargets(Some(slice::from_ref(&render_target)), None);
                    context.RSSetViewports(Some(&[D3D11_VIEWPORT {
                        TopLeftX: 0.,
                        TopLeftY: 0.,
                        Width: side,
                        Height: side,
                        MinDepth: 0.,
                        MaxDepth: 1.,
                    }]));
                    context.ClearRenderTargetView(
                        render_target
                            .as_ref()
                            .context("tile render target missing")?,
                        &clear_color(layer.background),
                    );
                    context.VSSetConstantBuffers(0, Some(slice::from_ref(&globals)));
                    context.PSSetConstantBuffers(0, Some(slice::from_ref(&globals)));
                    context.VSSetConstantBuffers(
                        1,
                        Some(slice::from_ref(&target.globals.batch_params_buffer)),
                    );
                }
                upload(target, pipelines, &tile_scene)?;
                draw_scene(target, pipelines, frame, &tile_scene)
            })
        });
    // The next draw binds tile textures as shader resources.
    unsafe { target.device_context.OMSetRenderTargets(None, None) };
    if result.is_err() {
        // The tiles this frame was to rasterize were not.
        frame.layers.clear();
    }
    result
}

/// Unbinds the textures of both stages, which may be tiles composited by the
/// previous frame and about to be drawn into.
fn unbind_textures(target: &Target<'_>, frame: &mut FrameState) {
    let none: [Option<ID3D11ShaderResourceView>; 1] = [None];
    unsafe {
        target.device_context.VSSetShaderResources(0, Some(&none));
        target.device_context.PSSetShaderResources(0, Some(&none));
    }
    frame.state.forget_views();
}

/// The clear colour a tile over `background` starts from: the bytes an
/// opaque quad of that colour writes to the frame's UNORM render target.
pub(crate) fn clear_color(background: Rgba) -> [f32; 4] {
    [background.r, background.g, background.b, background.a]
}
