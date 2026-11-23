// Modified by Longbridge for gpui-fast.
//! A frame's batches drawn with state set only when it changes.
//!
//! Upstream's `DirectXRenderer::render` draws each batch through its
//! `PipelineState`, which binds the whole pipeline, the sampler and the
//! texture, and maps the batch constant buffer, for every batch; each path
//! batch also collects its vertices and sprites into new `Vec`s. This draws
//! the same batches in the same order with the same draws, but binds through
//! [`DrawState`], which skips what is already bound, and collects path
//! vertices and sprites into buffers kept from frame to frame.
//!
//! Upstream's loop still runs while a graphics debugger such as PIX captures
//! the frame, so that each batch keeps its labeled event.

use std::slice;

use anyhow::{Context as _, Result};
use gpui::{PaintedPath, PrimitiveBatch, Scene};
use windows::Win32::Graphics::{
    Direct3D::D3D_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP,
    Direct3D11::{
        D3D11_VIEWPORT, ID3D11Buffer, ID3D11Device, ID3D11DeviceContext, ID3D11SamplerState,
        ID3D11ShaderResourceView,
    },
};

use crate::directx_renderer::{
    DirectXGlobalElements, DirectXRenderPipelines, GlobalParams, PipelineState,
};
use crate::fast::draw_state::DrawState;
use crate::fast::globals::UploadedGlobals;
use crate::fast::layers::{TileCache, composite};
use crate::{DirectXAtlas, DirectXRenderer};

/// The renderer's gpui-fast state, kept from frame to frame.
#[derive(Default)]
pub(crate) struct FrameState {
    pub(crate) globals: UploadedGlobals,
    pub(crate) state: DrawState,
    /// The frame's path tiles, curves and paints, kept from frame to frame.
    path_tiles: Vec<gpui::PathTile>,
    path_curves: Vec<gpui::PathCurve>,
    path_paints: Vec<gpui::PathPaint>,
    /// The depth the scene being drawn gives a rounded clip group's content
    /// (`None` where it is drawn without a depth buffer).
    pub(crate) group_depth: Option<f32>,
    /// The scroll layer tiles the frames composite.
    pub(crate) layers: TileCache,
    pub(crate) rounded_masks: crate::fast::rounded_clip::Masks,
    /// Set where overdraw culling is not wanted (`fast::occlusion::wanted`).
    pub(crate) skip_occlusion: bool,
}

impl FrameState {
    /// The state of a renderer on `adapter`: culling on a software adapter
    /// only, and the atlas recording opaque tiles only there too.
    pub(crate) fn for_adapter(
        atlas: &DirectXAtlas,
        adapter: &windows::Win32::Graphics::Dxgi::IDXGIAdapter1,
    ) -> FrameState {
        let mut state = FrameState::default();
        FrameState::adapter_changed(&mut state, atlas, adapter);
        state
    }

    /// Follows a new adapter (after a lost device).
    pub(crate) fn adapter_changed(
        &mut self,
        atlas: &DirectXAtlas,
        adapter: &windows::Win32::Graphics::Dxgi::IDXGIAdapter1,
    ) {
        let cull = crate::fast::occlusion::wanted(adapter);
        self.skip_occlusion = !cull;
        atlas.track_opaque_tiles(cull);
    }
}

/// The Direct3D objects a scene is drawn with.
pub(crate) struct Target<'a> {
    pub(crate) device: &'a ID3D11Device,
    pub(crate) device_context: &'a ID3D11DeviceContext,
    pub(crate) atlas: &'a DirectXAtlas,
    pub(crate) globals: &'a DirectXGlobalElements,
    /// The quad depth scale of a scene drawn with the depth buffer and the
    /// opaque pre-pass, as `DirectXRenderer::render`; `None` for targets
    /// drawn without a depth buffer (layer tiles), whose quad pass draws
    /// opaque rectangles too.
    pub(crate) depth_scale: Option<f32>,
}

/// Draws `scene`'s batches in place of `DirectXRenderer::render`'s loop, once
/// the scene's instances are uploaded. Returns `false`, having drawn nothing,
/// when a graphics debugger is capturing and upstream's labeled loop should
/// run instead; that loop cannot draw layer tiles, so a scene compositing
/// them is drawn here regardless.
pub(crate) fn draw_batches(renderer: &mut DirectXRenderer, scene: &Scene) -> Result<bool> {
    let devices = renderer.devices.as_ref().context("devices missing")?;
    let capturing = devices
        .annotation
        .as_ref()
        .is_some_and(|annotation| unsafe { annotation.GetStatus().as_bool() });
    if capturing && scene.layers.frames.is_empty() {
        return Ok(false);
    }
    let target = Target {
        device: &devices.device,
        device_context: &devices.device_context,
        atlas: &renderer.atlas,
        globals: &renderer.globals,
        depth_scale: Some(gpui::quad_depth_scale(scene.quads.len())),
    };
    draw_scene(
        &target,
        &mut renderer.pipelines,
        &mut renderer.fast_frame,
        scene,
    )?;
    Ok(true)
}

/// Uploads `scene`'s instances to the pipelines' buffers, as
/// `DirectXRenderer::upload_scene_buffers` does.
pub(crate) fn upload(
    target: &Target<'_>,
    pipelines: &mut DirectXRenderPipelines,
    scene: &Scene,
) -> Result<()> {
    let (device, context) = (target.device, target.device_context);
    if !scene.shadows.is_empty() {
        let pipeline = &mut pipelines.shadow_pipeline;
        pipeline.update_buffer(device, context, &scene.shadows)?;
    }
    if !scene.quads.is_empty() {
        let pipeline = &mut pipelines.quad_pipeline;
        pipeline.update_buffer(device, context, &scene.quads)?;
    }
    if !scene.underlines.is_empty() {
        let pipeline = &mut pipelines.underline_pipeline;
        pipeline.update_buffer(device, context, &scene.underlines)?;
    }
    if !scene.monochrome_sprites.is_empty() {
        let pipeline = &mut pipelines.mono_sprites;
        pipeline.update_buffer(device, context, &scene.monochrome_sprites)?;
    }
    if !scene.subpixel_sprites.is_empty() {
        let pipeline = &mut pipelines.subpixel_sprites;
        pipeline.update_buffer(device, context, &scene.subpixel_sprites)?;
    }
    if !scene.polychrome_sprites.is_empty() {
        let pipeline = &mut pipelines.poly_sprites;
        pipeline.update_buffer(device, context, &scene.polychrome_sprites)?;
    }
    Ok(())
}

/// Uploads `scene`'s path tiles, curves and paints, as
/// `DirectXRenderer::upload_scene_buffers` does, into buffers `frame` keeps.
pub(crate) fn upload_paths(
    target: &Target<'_>,
    pipelines: &mut DirectXRenderPipelines,
    frame: &mut FrameState,
    scene: &Scene,
) -> Result<()> {
    if scene.paths.is_empty() {
        return Ok(());
    }
    let (device, context) = (target.device, target.device_context);
    let (tiles, curves, paints) = (
        &mut frame.path_tiles,
        &mut frame.path_curves,
        &mut frame.path_paints,
    );
    tiles.clear();
    curves.clear();
    paints.clear();
    for path in &scene.paths {
        path.append_to(paints, curves, tiles);
    }
    pipelines
        .path_tile_pipeline
        .update_buffer(device, context, tiles)?;
    pipelines.path_curves.write(device, context, curves)?;
    pipelines.path_paints.write(device, context, paints)?;
    let buffers = [
        pipelines.path_curves.view.clone(),
        pipelines.path_paints.view.clone(),
    ];
    unsafe {
        context.VSSetShaderResources(2, Some(&buffers));
        context.PSSetShaderResources(2, Some(&buffers));
    }
    Ok(())
}

/// Draws `scene`'s batches, whose instances are uploaded, into the render
/// target bound on the device context.
pub(crate) fn draw_scene(
    target: &Target<'_>,
    pipelines: &mut DirectXRenderPipelines,
    frame: &mut FrameState,
    scene: &Scene,
) -> Result<()> {
    let result = draw_scene_masked(target, pipelines, frame, scene, None);
    crate::fast::rounded_clip::bind(target, &mut frame.rounded_masks, None)?;
    result
}

pub(crate) fn draw_scene_masked(
    target: &Target<'_>,
    pipelines: &mut DirectXRenderPipelines,
    frame: &mut FrameState,
    scene: &Scene,
    mask: Option<&ID3D11ShaderResourceView>,
) -> Result<()> {
    let batch_params = target
        .globals
        .batch_params_buffer
        .as_ref()
        .context("batch params buffer missing")?;
    let device_context = target.device_context;
    frame.state.forget();
    let mut restore_upload = false;
    // Primitives a later opaque rectangle hides are not drawn (none under
    // a mask, whose antialiased edge would show them).
    let occlusion = if mask.is_none() && !frame.skip_occlusion {
        crate::fast::occlusion::Occlusion::of(scene, |sprite| target.atlas.is_opaque(&sprite.tile))
    } else {
        crate::fast::occlusion::Occlusion::default()
    };
    // `DirectXRenderer::upload_scene_buffers` uploaded the frame's own.
    if target.depth_scale.is_none() {
        upload_paths(target, pipelines, frame, scene)?;
    }
    // As `DirectXRenderer::render`: opaque rectangles first, writing depth,
    // then each batch at the depth of the quads before it.
    let viewport = depth_viewport(target)?;
    if let (Some(depth_scale), Some(viewport)) = (target.depth_scale, viewport)
        && !scene.quads.is_empty()
    {
        set_depth_range(device_context, &viewport, 0.0, 1.0);
        pipelines.opaque_quad_pipeline.draw(
            device_context,
            &pipelines.quad_pipeline,
            batch_params,
            scene.quads.len() as u32,
        )?;
        let _ = depth_scale;
    }
    // The opaque pass wrote the batch start behind the draw state's back.
    frame.state.forget();
    let mut quad_cursor: u32 = 0;
    let mut path_tile_cursor = 0;

    for batch in scene.batches() {
        if let (Some(depth_scale), Some(viewport)) = (target.depth_scale, viewport.as_ref()) {
            if matches!(&batch, PrimitiveBatch::Quads(_)) {
                set_depth_range(device_context, viewport, 0.0, 1.0);
            } else {
                let depth = gpui::quad_depth(quad_cursor, depth_scale);
                set_depth_range(device_context, viewport, depth, depth);
            }
            frame.group_depth = Some(gpui::quad_depth(quad_cursor, depth_scale));
        } else {
            frame.group_depth = None;
        }
        if let PrimitiveBatch::Quads(range) = &batch {
            quad_cursor += range.len() as u32;
        }
        if let PrimitiveBatch::PolychromeSprites {
            texture_id,
            ref range,
        } = batch
            && let Some(group) = crate::fast::rounded_clip::group(scene, texture_id)
        {
            for index in range.clone() {
                crate::fast::rounded_clip::draw_group(
                    target,
                    pipelines,
                    frame,
                    group,
                    &scene.polychrome_sprites[index],
                    mask,
                )?;
            }
            restore_upload = true;
            continue;
        }
        if restore_upload {
            upload(target, pipelines, scene)?;
            upload_paths(target, pipelines, frame, scene)?;
            if let (Some(depth_scale), Some(viewport)) = (target.depth_scale, viewport.as_ref()) {
                let depth = gpui::quad_depth(quad_cursor, depth_scale);
                set_depth_range(device_context, viewport, depth, depth);
            }
            restore_upload = false;
        }
        crate::fast::rounded_clip::bind(target, &mut frame.rounded_masks, mask)?;
        let FrameState { state, layers, .. } = frame;
        let mut draw = Draw {
            device_context,
            state,
            batch_params,
            sampler: &target.globals.sampler,
        };
        match batch {
            PrimitiveBatch::Shadows(range) if !occlusion.is_empty() => {
                crate::fast::occlusion::visible_runs(range, |i| {
                    occlusion.hides_shadow(&scene.shadows[i])
                })
                .into_iter()
                .try_for_each(|run| {
                    draw.instances(&pipelines.shadow_pipeline, run.start, run.len(), None)
                })
            }
            PrimitiveBatch::Shadows(range) => {
                draw.instances(&pipelines.shadow_pipeline, range.start, range.len(), None)
            }
            PrimitiveBatch::Quads(range) if !occlusion.is_empty() => {
                // Hidden quads are skipped; the depth buffer culls what a
                // partly covered one has under later opaque rectangles.
                crate::fast::occlusion::visible_runs(range, |i| {
                    occlusion.hides_quad(&scene.quads[i])
                })
                .into_iter()
                .try_for_each(|run| {
                    draw.instances(&pipelines.quad_pipeline, run.start, run.len(), None)
                })
            }
            PrimitiveBatch::Quads(range) => {
                draw.instances(&pipelines.quad_pipeline, range.start, range.len(), None)
            }
            PrimitiveBatch::Paths(range) => {
                let tiles_start = path_tile_cursor;
                path_tile_cursor += scene.paths[range]
                    .iter()
                    .map(PaintedPath::tile_count)
                    .sum::<usize>();
                draw.instances(
                    &pipelines.path_tile_pipeline,
                    tiles_start,
                    path_tile_cursor - tiles_start,
                    None,
                )
            }
            PrimitiveBatch::Underlines(range) => draw.instances(
                &pipelines.underline_pipeline,
                range.start,
                range.len(),
                None,
            ),
            PrimitiveBatch::MonochromeSprites { texture_id, range } => {
                // The atlas has released the texture; upstream skips the batch.
                let Some([view]) = target.atlas.get_texture_view(texture_id) else {
                    continue;
                };
                let pipeline = &pipelines.mono_sprites;
                if occlusion.is_empty() {
                    draw.instances(pipeline, range.start, range.len(), Some(&view))
                } else {
                    let sprites = &scene.monochrome_sprites;
                    crate::fast::occlusion::visible_runs(range, |i| {
                        let s = &sprites[i];
                        occlusion.hides_glyph(
                            s.order,
                            &s.bounds,
                            &s.content_mask,
                            &s.transformation,
                        )
                    })
                    .into_iter()
                    .try_for_each(|run| draw.instances(pipeline, run.start, run.len(), Some(&view)))
                }
            }
            PrimitiveBatch::SubpixelSprites { texture_id, range } => {
                // The atlas has released the texture; upstream skips the batch.
                let Some([view]) = target.atlas.get_texture_view(texture_id) else {
                    continue;
                };
                let pipeline = &pipelines.subpixel_sprites;
                if occlusion.is_empty() {
                    draw.instances(pipeline, range.start, range.len(), Some(&view))
                } else {
                    let sprites = &scene.subpixel_sprites;
                    crate::fast::occlusion::visible_runs(range, |i| {
                        let s = &sprites[i];
                        occlusion.hides_glyph(
                            s.order,
                            &s.bounds,
                            &s.content_mask,
                            &s.transformation,
                        )
                    })
                    .into_iter()
                    .try_for_each(|run| draw.instances(pipeline, run.start, run.len(), Some(&view)))
                }
            }
            PrimitiveBatch::PolychromeSprites { texture_id, range }
                if composite::is_layer_texture(texture_id) =>
            {
                let sprites = &scene.polychrome_sprites;
                composite::tile_runs(sprites, range).try_for_each(|run| {
                    let Some(view) =
                        composite::texture_for_run(layers, texture_id, &sprites[run.clone()])
                    else {
                        return Ok(());
                    };
                    let pipeline = &pipelines.poly_sprites;
                    let sampler = std::mem::replace(&mut draw.sampler, layers.sampler());
                    let drawn = draw.instances(pipeline, run.start, run.len(), Some(view));
                    draw.sampler = sampler;
                    drawn
                })
            }
            PrimitiveBatch::PolychromeSprites { texture_id, range } => {
                // The atlas has released the texture; upstream skips the batch.
                let Some([view]) = target.atlas.get_texture_view(texture_id) else {
                    continue;
                };
                let pipeline = &pipelines.poly_sprites;
                if occlusion.is_empty() {
                    draw.instances(pipeline, range.start, range.len(), Some(&view))
                } else {
                    crate::fast::occlusion::visible_runs(range, |i| {
                        occlusion.hides_sprite(&scene.polychrome_sprites[i])
                    })
                    .into_iter()
                    .try_for_each(|run| draw.instances(pipeline, run.start, run.len(), Some(&view)))
                }
            }
            // Upstream draws nothing for surfaces on Windows either.
            PrimitiveBatch::Surfaces(_) => Ok(()),
            // Native views are only composited on macOS.
            PrimitiveBatch::NativeViews(_) => Ok(()),
        }
        .with_context(|| scene_too_large(scene))?;
    }
    frame.group_depth = None;
    if let Some(viewport) = viewport {
        set_depth_range(device_context, &viewport, 0.0, 1.0);
    }
    Ok(())
}

/// What every batch's draw needs.
struct Draw<'a> {
    device_context: &'a ID3D11DeviceContext,
    state: &'a mut DrawState,
    batch_params: &'a ID3D11Buffer,
    sampler: &'a Option<ID3D11SamplerState>,
}

impl Draw<'_> {
    /// Upstream's `PipelineState::draw_range` (and `draw_range_with_texture`
    /// when `texture` is given).
    fn instances<T>(
        &mut self,
        pipeline: &PipelineState<T>,
        first_instance: usize,
        instance_count: usize,
        texture: Option<&Option<ID3D11ShaderResourceView>>,
    ) -> Result<()> {
        if instance_count == 0 {
            return Ok(());
        }
        anyhow::ensure!(
            first_instance + instance_count <= pipeline.instances.capacity,
            "DirectX instance range exceeds the {} buffer",
            pipeline.instances.label
        );
        let context = self.device_context;
        self.state
            .set_batch_start(context, self.batch_params, first_instance as u32)?;
        self.state
            .set_pipeline(context, pipeline, D3D_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP);
        if let Some(texture) = texture {
            self.state.set_texture(context, texture, self.sampler);
        }
        unsafe { context.DrawInstanced(4, instance_count as u32, 0, 0) };
        Ok(())
    }
}

/// The viewport bound for a scene drawn with the depth buffer, whose depth
/// range each batch narrows; `None` for scenes drawn without one.
fn depth_viewport(target: &Target<'_>) -> Result<Option<D3D11_VIEWPORT>> {
    if target.depth_scale.is_none() {
        return Ok(None);
    }
    let mut count = 1;
    let mut viewport = [D3D11_VIEWPORT::default()];
    unsafe {
        target
            .device_context
            .RSGetViewports(&mut count, Some(viewport.as_mut_ptr()))
    };
    anyhow::ensure!(count == 1, "no viewport bound");
    Ok(Some(viewport[0]))
}

/// Upstream's `DirectXRenderer::set_viewport_depth_range`.
fn set_depth_range(
    context: &ID3D11DeviceContext,
    viewport: &D3D11_VIEWPORT,
    minimum_depth: f32,
    maximum_depth: f32,
) {
    let viewport = D3D11_VIEWPORT {
        MinDepth: minimum_depth,
        MaxDepth: maximum_depth,
        ..*viewport
    };
    unsafe { context.RSSetViewports(Some(slice::from_ref(&viewport))) };
}

/// The globals a scene's draws read: the frame's, with `depth_scale`, and
/// whether its quad pass draws opaque rectangles (`global_pad.x`).
pub(crate) fn scene_globals(viewport: &D3D11_VIEWPORT, draws_opaque: bool) -> GlobalParams {
    let font_info = crate::DirectXRenderer::get_font_info();
    GlobalParams {
        gamma_ratios: font_info.gamma_ratios,
        viewport_size: [viewport.Width, viewport.Height],
        grayscale_enhanced_contrast: font_info.grayscale_enhanced_contrast,
        subpixel_enhanced_contrast: font_info.subpixel_enhanced_contrast,
        is_bgr: font_info.is_bgr as u32,
        depth_scale: 0.0,
        _pad: [draws_opaque as u32, 0],
    }
}

fn scene_too_large(scene: &Scene) -> String {
    format!(
        "scene too large:\
        {} paths, {} shadows, {} quads, {} underlines, {} mono, {} subpixel, {} poly, {} surfaces",
        scene.paths.len(),
        scene.shadows.len(),
        scene.quads.len(),
        scene.underlines.len(),
        scene.monochrome_sprites.len(),
        scene.subpixel_sprites.len(),
        scene.polychrome_sprites.len(),
        scene.surfaces.len(),
    )
}
