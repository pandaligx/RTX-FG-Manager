// Modified by Longbridge for gpui-fast.
//! Device context state set only when it changes.
//!
//! Upstream sets the vertex and pixel shaders, the blend state, the topology,
//! the instance buffer view (for both stages), the sampler and the atlas
//! texture (for both stages) before every batch's draw, and maps the batch
//! constant buffer to write the batch's first instance. The D3D11 runtime
//! doesn't filter repeated calls: each one takes the context's lock, swaps
//! the references it holds and marks the state dirty for the driver. Runs of
//! text are one sprite batch each, all with the same shaders, sampler and
//! atlas texture, so most of those calls bound what was already bound.
//!
//! What is bound is forgotten at the start of every frame, because text
//! rasterization (`direct_write.rs`) sets its own shaders and state on the
//! same context between frames.

use anyhow::Result;
use windows::Win32::Graphics::{
    Direct3D::D3D_PRIMITIVE_TOPOLOGY,
    Direct3D11::{
        ID3D11BlendState, ID3D11Buffer, ID3D11DeviceContext, ID3D11PixelShader, ID3D11SamplerState,
        ID3D11ShaderResourceView, ID3D11VertexShader,
    },
};

use crate::directx_renderer::PipelineState;

/// What the renderer has bound on the device context during this frame.
#[derive(Default)]
pub(crate) struct DrawState {
    vertex: Option<ID3D11VertexShader>,
    fragment: Option<ID3D11PixelShader>,
    blend_state: Option<ID3D11BlendState>,
    topology: Option<D3D_PRIMITIVE_TOPOLOGY>,
    /// Slot 1 of both stages: the pipeline's instance buffer.
    instances: Option<Option<ID3D11ShaderResourceView>>,
    /// Slot 0 of both stages: the sprite atlas texture or path texture.
    texture: Option<Option<ID3D11ShaderResourceView>>,
    sampler: Option<Option<ID3D11SamplerState>>,
    /// The batch constant buffer and the first instance written to it.
    batch_start: Option<(ID3D11Buffer, u32)>,
}

impl DrawState {
    /// Forgets what is bound, for a frame that starts from unknown state.
    pub(crate) fn forget(&mut self) {
        *self = Self::default();
    }

    /// Forgets the shader resource bindings, which the runtime unbinds by itself
    /// when their resource becomes a render target.
    pub(crate) fn forget_views(&mut self) {
        self.texture = None;
        self.instances = None;
    }

    /// Binds `pipeline`'s shaders, blend state and instance buffer, and
    /// `topology`: what upstream's `set_pipeline_state` binds.
    pub(crate) fn set_pipeline<T>(
        &mut self,
        device_context: &ID3D11DeviceContext,
        pipeline: &PipelineState<T>,
        topology: D3D_PRIMITIVE_TOPOLOGY,
    ) {
        unsafe {
            if self.instances.as_ref() != Some(&pipeline.view) {
                let view = std::slice::from_ref(&pipeline.view);
                device_context.VSSetShaderResources(1, Some(view));
                device_context.PSSetShaderResources(1, Some(view));
                self.instances = Some(pipeline.view.clone());
            }
            if self.topology != Some(topology) {
                device_context.IASetPrimitiveTopology(topology);
                self.topology = Some(topology);
            }
            if self.vertex.as_ref() != Some(&pipeline.vertex) {
                device_context.VSSetShader(&pipeline.vertex, None);
                self.vertex = Some(pipeline.vertex.clone());
            }
            if self.fragment.as_ref() != Some(&pipeline.fragment) {
                device_context.PSSetShader(&pipeline.fragment, None);
                self.fragment = Some(pipeline.fragment.clone());
            }
            if self.blend_state.as_ref() != Some(&pipeline.blend_state) {
                device_context.OMSetBlendState(&pipeline.blend_state, None, 0xFFFFFFFF);
                self.blend_state = Some(pipeline.blend_state.clone());
            }
        }
    }

    /// Binds `sampler` to the pixel shader and `texture` to both stages.
    pub(crate) fn set_texture(
        &mut self,
        device_context: &ID3D11DeviceContext,
        texture: &Option<ID3D11ShaderResourceView>,
        sampler: &Option<ID3D11SamplerState>,
    ) {
        unsafe {
            if self.sampler.as_ref() != Some(sampler) {
                device_context.PSSetSamplers(0, Some(std::slice::from_ref(sampler)));
                self.sampler = Some(sampler.clone());
            }
            if self.texture.as_ref() != Some(texture) {
                let texture_slice = std::slice::from_ref(texture);
                device_context.VSSetShaderResources(0, Some(texture_slice));
                device_context.PSSetShaderResources(0, Some(texture_slice));
                self.texture = Some(texture.clone());
            }
        }
    }

    /// Writes `first_instance` to the batch constant buffer unless it already
    /// holds it. A dynamic buffer keeps what was last written to it.
    pub(crate) fn set_batch_start(
        &mut self,
        device_context: &ID3D11DeviceContext,
        buffer: &ID3D11Buffer,
        first_instance: u32,
    ) -> Result<()> {
        if let Some((written_buffer, written)) = &self.batch_start
            && written_buffer == buffer
            && *written == first_instance
        {
            return Ok(());
        }
        self.batch_start = None;
        crate::directx_renderer::update_batch_start(device_context, buffer, first_instance)?;
        self.batch_start = Some((buffer.clone(), first_instance));
        Ok(())
    }
}
