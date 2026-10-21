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
use gpui::{Path, PrimitiveBatch, ScaledPixels, Scene};
use windows::Win32::Graphics::{
    Direct3D::{D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST, D3D_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP},
    Direct3D11::{
        ID3D11Buffer, ID3D11Device, ID3D11DeviceContext, ID3D11RenderTargetView,
        ID3D11SamplerState, ID3D11ShaderResourceView, ID3D11Texture2D,
    },
};

use crate::directx_renderer::{
    DirectXGlobalElements, DirectXRenderPipelines, DirectXResources, PathRasterizationSprite,
    PathSprite, PipelineState, RENDER_TARGET_FORMAT,
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
    path_vertices: Vec<PathRasterizationSprite>,
    path_sprites: Vec<PathSprite>,
    /// The scroll layer tiles the frames composite.
    pub(crate) layers: TileCache,
    pub(crate) rounded_masks: crate::fast::rounded_clip::Masks,
}

/// The Direct3D objects a scene is drawn with.
pub(crate) struct Target<'a> {
    pub(crate) device: &'a ID3D11Device,
    pub(crate) device_context: &'a ID3D11DeviceContext,
    pub(crate) atlas: &'a DirectXAtlas,
    pub(crate) globals: &'a DirectXGlobalElements,
    /// What the frame's paths are drawn through; `None` for targets that
    /// never draw paths.
    pub(crate) paths: Option<PathTargets<'a>>,
}

/// The render target a frame's paths are drawn into and upstream's path
/// intermediate textures they are rasterized through: the window's, from its
/// [`DirectXResources`], or ones the layer pixel tests make for their target.
#[derive(Clone, Copy)]
pub(crate) struct PathTargets<'a> {
    pub(crate) render_target_view: &'a Option<ID3D11RenderTargetView>,
    pub(crate) intermediate_texture: &'a ID3D11Texture2D,
    pub(crate) intermediate_srv: &'a Option<ID3D11ShaderResourceView>,
    pub(crate) msaa_texture: &'a ID3D11Texture2D,
    pub(crate) msaa_view: &'a Option<ID3D11RenderTargetView>,
}

impl<'a> PathTargets<'a> {
    pub(crate) fn of(resources: &'a DirectXResources) -> Self {
        PathTargets {
            render_target_view: &resources.render_target_view,
            intermediate_texture: &resources.path_intermediate_texture,
            intermediate_srv: &resources.path_intermediate_srv,
            msaa_texture: &resources.path_intermediate_msaa_texture,
            msaa_view: &resources.path_intermediate_msaa_view,
        }
    }
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
    let resources = renderer.resources.as_ref().context("resources missing")?;
    let target = Target {
        device: &devices.device,
        device_context: &devices.device_context,
        atlas: &renderer.atlas,
        globals: &renderer.globals,
        paths: Some(PathTargets::of(resources)),
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

    for batch in scene.batches() {
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
            restore_upload = false;
        }
        crate::fast::rounded_clip::bind(target, &mut frame.rounded_masks, mask)?;
        let FrameState {
            state,
            path_vertices,
            path_sprites,
            layers,
            ..
        } = frame;
        let mut draw = Draw {
            device_context,
            state,
            batch_params,
            sampler: &target.globals.sampler,
        };
        match batch {
            PrimitiveBatch::Shadows(range) => {
                draw.instances(&pipelines.shadow_pipeline, range.start, range.len(), None)
            }
            PrimitiveBatch::Quads(range) => {
                draw.instances(&pipelines.quad_pipeline, range.start, range.len(), None)
            }
            PrimitiveBatch::Paths(range) => {
                let paths = &scene.paths[range];
                if paths.is_empty() {
                    continue;
                }
                let Some(targets) = &target.paths else {
                    // Layer tiles never hold paths (spec §5.6).
                    debug_assert!(false, "paths drawn into a target without path textures");
                    continue;
                };
                rasterize_paths(
                    &mut draw,
                    target.device,
                    targets,
                    &mut pipelines.path_rasterization_pipeline,
                    path_vertices,
                    paths,
                )
                .and_then(|()| {
                    composite_paths(
                        &mut draw,
                        target.device,
                        targets,
                        &mut pipelines.path_sprite_pipeline,
                        path_sprites,
                        paths,
                    )
                })
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
                draw.instances(pipeline, range.start, range.len(), Some(&view))
            }
            PrimitiveBatch::SubpixelSprites { texture_id, range } => {
                // The atlas has released the texture; upstream skips the batch.
                let Some([view]) = target.atlas.get_texture_view(texture_id) else {
                    continue;
                };
                let pipeline = &pipelines.subpixel_sprites;
                draw.instances(pipeline, range.start, range.len(), Some(&view))
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
                draw.instances(pipeline, range.start, range.len(), Some(&view))
            }
            // Upstream draws nothing for surfaces on Windows either.
            PrimitiveBatch::Surfaces(_) => Ok(()),
        }
        .with_context(|| scene_too_large(scene))?;
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
            first_instance + instance_count <= pipeline.buffer_size,
            "DirectX instance range exceeds the {} buffer",
            pipeline.label
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

/// Upstream's `DirectXRenderer::draw_paths_to_intermediate`.
fn rasterize_paths(
    draw: &mut Draw<'_>,
    device: &ID3D11Device,
    targets: &PathTargets<'_>,
    pipeline: &mut PipelineState<PathRasterizationSprite>,
    vertices: &mut Vec<PathRasterizationSprite>,
    paths: &[Path<ScaledPixels>],
) -> Result<()> {
    let context = draw.device_context;
    unsafe {
        context.ClearRenderTargetView(targets.msaa_view.as_ref().unwrap(), &[0.0; 4]);
        context.OMSetRenderTargets(Some(slice::from_ref(targets.msaa_view)), None);
    }
    draw.state.forget_views();

    vertices.clear();
    for path in paths {
        let bounds = path.clipped_bounds();
        vertices.extend(path.vertices.iter().map(|v| PathRasterizationSprite {
            xy_position: v.xy_position,
            st_position: v.st_position,
            color: path.color,
            bounds,
        }));
    }
    pipeline.update_buffer(device, context, vertices)?;
    draw.state
        .set_pipeline(context, pipeline, D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
    unsafe {
        context.DrawInstanced(vertices.len() as u32, 1, 0, 0);
        context.ResolveSubresource(
            targets.intermediate_texture,
            0,
            targets.msaa_texture,
            0,
            RENDER_TARGET_FORMAT,
        );
        context.OMSetRenderTargets(Some(slice::from_ref(targets.render_target_view)), None);
    }
    draw.state.forget_views();
    Ok(())
}

/// Upstream's `DirectXRenderer::draw_paths_from_intermediate`.
fn composite_paths(
    draw: &mut Draw<'_>,
    device: &ID3D11Device,
    targets: &PathTargets<'_>,
    pipeline: &mut PipelineState<PathSprite>,
    sprites: &mut Vec<PathSprite>,
    paths: &[Path<ScaledPixels>],
) -> Result<()> {
    let (Some(first_path), Some(last_path)) = (paths.first(), paths.last()) else {
        return Ok(());
    };

    // Each pixel must be copied only once, in case of transparent paths.
    // Paths of one draw order are disjoint and copied one by one; a batch
    // mixing draw orders is copied as one rect spanning all of them.
    sprites.clear();
    if last_path.order == first_path.order {
        sprites.extend(paths.iter().map(|path| PathSprite {
            bounds: path.clipped_bounds(),
        }));
    } else {
        let mut bounds = first_path.clipped_bounds();
        for path in &paths[1..] {
            bounds = bounds.union(&path.clipped_bounds());
        }
        sprites.push(PathSprite { bounds });
    }

    let context = draw.device_context;
    pipeline.update_buffer(device, context, sprites)?;
    draw.state
        .set_pipeline(context, pipeline, D3D_PRIMITIVE_TOPOLOGY_TRIANGLESTRIP);
    draw.state
        .set_texture(context, targets.intermediate_srv, draw.sampler);
    unsafe { context.DrawInstanced(4, sprites.len() as u32, 0, 0) };
    Ok(())
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
