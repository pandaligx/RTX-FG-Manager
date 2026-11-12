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
        ID3D11Device, ID3D11RenderTargetView, ID3D11ShaderResourceView, ID3D11Texture2D,
    },
    Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_R32G32B32A32_FLOAT, DXGI_SAMPLE_DESC},
    core::Interface as _,
};

use crate::directx_renderer::{
    DirectXRenderPipelines, create_path_intermediate_msaa_texture_and_view,
    create_path_intermediate_texture,
};
use crate::fast::frame::{FrameState, PathTargets, Target, draw_scene_masked, upload};

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
    paths: Vec<PathSlot>,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Clip {
    bounds: [f32; 4],
    horizontal: [f32; 4],
    vertical: [f32; 4],
}

struct PathSlot {
    size: (u32, u32),
    used: bool,
    image: PathImage,
}

#[derive(Clone)]
struct PathImage {
    intermediate: ID3D11Texture2D,
    view: Option<ID3D11ShaderResourceView>,
    msaa: ID3D11Texture2D,
    msaa_target: Option<ID3D11RenderTargetView>,
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

    fn paths(&mut self, device: &ID3D11Device, size: (u32, u32)) -> Result<PathImage> {
        if let Some(slot) = self.paths.iter_mut().find(|slot| slot.size == size) {
            slot.used = true;
            return Ok(slot.image.clone());
        }
        let (intermediate, view) = create_path_intermediate_texture(device, size.0, size.1)?;
        let (msaa, msaa_target) =
            create_path_intermediate_msaa_texture_and_view(device, size.0, size.1)?;
        let image = PathImage {
            intermediate,
            view,
            msaa,
            msaa_target,
        };
        self.paths.push(PathSlot {
            size,
            used: true,
            image: image.clone(),
        });
        Ok(image)
    }
}

pub(crate) fn begin_frame(masks: &mut Masks) {
    masks.clips.clear();
    masks.paths.retain(|slot| slot.used);
    for slot in &mut masks.paths {
        slot.used = false;
    }
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
        let mut parent = [None];
        unsafe {
            context.OMGetRenderTargets(Some(&mut parent), None);
        }
        let render_target = parent[0].clone().context("rounded clip render target")?;
        let resource = unsafe { render_target.GetResource()? };
        let texture: ID3D11Texture2D = resource.cast()?;
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        unsafe {
            texture.GetDesc(&mut desc);
        }
        let size = (desc.Width, desc.Height);
        let fallback = if target.paths.is_none() {
            Some(frame.rounded_masks.paths(target.device, size)?)
        } else {
            None
        };
        let parent_view = Some(render_target);
        let paths = target.paths.unwrap_or_else(|| {
            let image = fallback.as_ref().unwrap();
            PathTargets {
                render_target_view: &parent_view,
                intermediate_texture: &image.intermediate,
                intermediate_srv: &image.view,
                msaa_texture: &image.msaa,
                msaa_view: &image.msaa_target,
            }
        });
        let content_target = Target {
            device: target.device,
            device_context: context,
            atlas: target.atlas,
            globals: target.globals,
            paths: Some(PathTargets {
                render_target_view: &parent_view,
                ..paths
            }),
        };
        upload(&content_target, pipelines, child)?;
        draw_scene_masked(&content_target, pipelines, frame, child, view.as_ref())
    })();
    frame.rounded_masks.clips.pop();
    bind(target, &mut frame.rounded_masks, parent_mask)?;
    frame.state.forget();
    result
}
