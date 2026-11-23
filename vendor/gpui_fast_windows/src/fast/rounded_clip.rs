// Modified by Longbridge for gpui-fast.
//! Rounded scopes evaluate an immutable geometry texture of padding edges
//! directly in the colour shader. Each child appends its geometry and restores
//! the enclosing stack afterward; no mask render targets or passes are needed.

#[cfg(debug_assertions)]
use std::path::PathBuf;
use std::slice;

use anyhow::{Context as _, Result};
use gpui::{AtlasTextureId, LayerFrame, Scene};
use windows::{
    Win32::Graphics::Direct3D11::{
        D3D11_BIND_CONSTANT_BUFFER, D3D11_BIND_SHADER_RESOURCE, D3D11_BUFFER_DESC,
        D3D11_SUBRESOURCE_DATA, D3D11_TEXTURE2D_DESC, D3D11_USAGE_IMMUTABLE, ID3D11Buffer,
        ID3D11Device, ID3D11ShaderResourceView,
    },
    Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_R32G32B32A32_FLOAT, DXGI_SAMPLE_DESC},
};

use crate::directx_renderer::DirectXRenderPipelines;
use crate::fast::frame::{FrameState, Target, draw_scene_masked, upload};

#[cfg(debug_assertions)]
pub(crate) fn shader_path(original: PathBuf, out_dir: &str) -> PathBuf {
    if original
        .file_name()
        .is_some_and(|name| name == "shaders.hlsl")
    {
        PathBuf::from(out_dir).join("rounded_clip_shaders.hlsl")
    } else {
        original
    }
}

pub(crate) fn group(scene: &Scene, texture: AtlasTextureId) -> Option<&LayerFrame> {
    if texture.index < 0xF100_0000 {
        return None;
    }
    let key = texture.index - 0xF000_0000;
    scene
        .layers
        .frames
        .iter()
        .find(|frame| frame.key.0 == key && frame.tile_size == 0)
}

#[derive(Default)]
pub(crate) struct Masks {
    device: Option<ID3D11Device>,
    flags: Option<ID3D11Buffer>,
    clips: Vec<Clip>,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Clip {
    bounds: [f32; 4],
    horizontal: [f32; 4],
    vertical: [f32; 4],
}

impl Masks {
    fn for_device(&mut self, device: &ID3D11Device) {
        if self.device.as_ref() != Some(device) {
            *self = Self {
                device: Some(device.clone()),
                ..Default::default()
            };
        }
    }

    fn clip_view(&self, device: &ID3D11Device) -> Result<Option<ID3D11ShaderResourceView>> {
        // ps_4_1 does not support StructuredBuffer. Three full-precision
        // texels per edge preserve the same geometry on the existing profile.
        let desc = D3D11_TEXTURE2D_DESC {
            Width: 3,
            Height: u32::try_from(self.clips.len())?,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_R32G32B32A32_FLOAT,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_IMMUTABLE,
            BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
            ..Default::default()
        };
        let data = D3D11_SUBRESOURCE_DATA {
            pSysMem: self.clips.as_ptr().cast(),
            SysMemPitch: std::mem::size_of::<Clip>() as u32,
            ..Default::default()
        };
        let mut texture = None;
        let mut view = None;
        unsafe {
            device.CreateTexture2D(&desc, Some(&data), Some(&mut texture))?;
            device.CreateShaderResourceView(
                texture.as_ref().context("clip geometry texture")?,
                None,
                Some(&mut view),
            )?;
        }
        Ok(view)
    }
}

pub(crate) fn begin_frame(masks: &mut Masks) {
    masks.clips.clear();
}

pub(crate) fn bind(
    target: &Target<'_>,
    masks: &mut Masks,
    view: Option<&ID3D11ShaderResourceView>,
) -> Result<()> {
    masks.for_device(target.device);
    if view.is_some() && masks.flags.is_none() {
        let values = [1u32, 0, 0, 0];
        let desc = D3D11_BUFFER_DESC {
            ByteWidth: 16,
            Usage: D3D11_USAGE_IMMUTABLE,
            BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32,
            ..Default::default()
        };
        let data = D3D11_SUBRESOURCE_DATA {
            pSysMem: values.as_ptr().cast(),
            ..Default::default()
        };
        unsafe {
            target
                .device
                .CreateBuffer(&desc, Some(&data), Some(&mut masks.flags))?;
        }
    }
    let flags = if view.is_some() {
        masks.flags.clone()
    } else {
        None
    };
    unsafe {
        target
            .device_context
            .PSSetConstantBuffers(2, Some(slice::from_ref(&flags)));
        target
            .device_context
            .PSSetShaderResources(15, Some(slice::from_ref(&view.cloned())));
    }
    Ok(())
}

pub(crate) fn draw_group(
    target: &Target<'_>,
    pipelines: &mut DirectXRenderPipelines,
    frame: &mut FrameState,
    group: &LayerFrame,
    sprite: &gpui::PolychromeSprite,
    parent_mask: Option<&ID3D11ShaderResourceView>,
) -> Result<()> {
    let child = group.content.scene().context("rounded clip content")?;
    let radii = child
        .layers
        .frames
        .iter()
        .find(|frame| frame.key.0 == u32::MAX)
        .context("rounded clip geometry")?
        .background;
    frame.rounded_masks.for_device(target.device);
    let b = sprite.bounds;
    let r = sprite.corner_radii;
    frame.rounded_masks.clips.push(Clip {
        bounds: [b.origin.x.0, b.origin.y.0, b.right().0, b.bottom().0],
        horizontal: [
            r.top_left.0,
            r.top_right.0,
            r.bottom_right.0,
            r.bottom_left.0,
        ],
        vertical: [radii.r, radii.g, radii.b, radii.a],
    });
    let result = (|| {
        let view = frame.rounded_masks.clip_view(target.device)?;
        let context = target.device_context;
        let content_target = Target {
            device: target.device,
            device_context: context,
            atlas: target.atlas,
            globals: target.globals,
            // The content is drawn at the group's depth, no pre-pass.
            depth_scale: None,
        };
        // Under the parent's depth buffer, the content is drawn at the
        // group's depth with its quad pass drawing opaque rectangles (no
        // opaque pre-pass: it could not be masked); its globals say so.
        let depth = frame.group_depth;
        let viewport = group_globals(target, frame, depth, true)?;
        upload(&content_target, pipelines, child)?;
        let drawn = draw_scene_masked(&content_target, pipelines, frame, child, view.as_ref());
        if let Some(viewport) = viewport {
            restore_globals(target, frame, &viewport)?;
        }
        drawn
    })();
    frame.rounded_masks.clips.pop();
    bind(target, &mut frame.rounded_masks, parent_mask)?;
    frame.state.forget();
    result
}

/// Writes the frame globals for a group's content drawn under a depth buffer
/// at `depth`: its quad pass draws opaque rectangles. Returns the parent's
/// viewport to restore; `None` (nothing written) without a depth buffer,
/// where the globals already say so (layer tiles).
fn group_globals(
    target: &Target<'_>,
    frame: &crate::fast::frame::FrameState,
    depth: Option<f32>,
    draws_opaque: bool,
) -> Result<Option<windows::Win32::Graphics::Direct3D11::D3D11_VIEWPORT>> {
    let Some(depth) = depth else {
        return Ok(None);
    };
    let context = target.device_context;
    let mut count = 1;
    let mut viewport = [windows::Win32::Graphics::Direct3D11::D3D11_VIEWPORT::default()];
    unsafe { context.RSGetViewports(&mut count, Some(viewport.as_mut_ptr())) };
    let viewport = viewport[0];
    let buffer = target
        .globals
        .global_params_buffer
        .as_ref()
        .context("global params buffer missing")?;
    crate::fast::globals::write_globals(
        frame,
        context,
        buffer,
        &[crate::fast::frame::scene_globals(&viewport, draws_opaque)],
    )?;
    let at_depth = windows::Win32::Graphics::Direct3D11::D3D11_VIEWPORT {
        MinDepth: depth,
        MaxDepth: depth,
        ..viewport
    };
    unsafe { context.RSSetViewports(Some(slice::from_ref(&at_depth))) };
    Ok(Some(viewport))
}

/// Restores the frame globals and viewport [`group_globals`] replaced.
fn restore_globals(
    target: &Target<'_>,
    frame: &crate::fast::frame::FrameState,
    viewport: &windows::Win32::Graphics::Direct3D11::D3D11_VIEWPORT,
) -> Result<()> {
    let buffer = target
        .globals
        .global_params_buffer
        .as_ref()
        .context("global params buffer missing")?;
    let mut params = crate::fast::frame::scene_globals(viewport, false);
    params.depth_scale = target.depth_scale.unwrap_or(0.0);
    crate::fast::globals::write_globals(frame, target.device_context, buffer, &[params])?;
    unsafe {
        target
            .device_context
            .RSSetViewports(Some(slice::from_ref(viewport)))
    };
    Ok(())
}
