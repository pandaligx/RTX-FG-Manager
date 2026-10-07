// Modified by Longbridge for gpui-fast.
//! The global constant buffer written only when it changes.
//!
//! Upstream maps the global parameters buffer with `D3D11_MAP_WRITE_DISCARD`
//! at the start of every frame, which makes the driver rename the buffer and
//! copy the parameters, yet they only change with the viewport size. A dynamic
//! buffer keeps its contents until it is mapped again, so a frame whose
//! parameters are the ones already in the buffer skips the map.

use std::cell::RefCell;

use anyhow::Result;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_MAP_WRITE_DISCARD, D3D11_MAPPED_SUBRESOURCE, ID3D11Buffer, ID3D11DeviceContext,
};

use crate::directx_renderer::GlobalParams;

const SIZE: usize = size_of::<GlobalParams>();

/// The buffer last written and the bytes written to it. The buffer is part of
/// the key because recovering from a lost device creates a new one.
#[derive(Default)]
pub(crate) struct UploadedGlobals(RefCell<Option<(ID3D11Buffer, [u8; SIZE])>>);

/// Forwarded to by `DirectXRenderer::pre_draw` in place of its globals write.
pub(crate) fn write_globals(
    frame: &crate::fast::frame::FrameState,
    device_context: &ID3D11DeviceContext,
    buffer: &ID3D11Buffer,
    [params]: &[GlobalParams; 1],
) -> Result<()> {
    // SAFETY: `GlobalParams` is `repr(C)` and made of 4-byte scalars with its
    // padding spelled out as fields, so all of its bytes are initialized.
    let bytes: [u8; SIZE] = unsafe { std::mem::transmute_copy::<GlobalParams, [u8; SIZE]>(params) };
    let mut uploaded = frame.globals.0.borrow_mut();
    if let Some((uploaded_buffer, uploaded_bytes)) = uploaded.as_ref()
        && uploaded_buffer == buffer
        && *uploaded_bytes == bytes
    {
        return Ok(());
    }
    *uploaded = None;
    unsafe {
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        device_context.Map(buffer, 0, D3D11_MAP_WRITE_DISCARD, 0, Some(&mut mapped))?;
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), mapped.pData as *mut u8, SIZE);
        device_context.Unmap(buffer, 0);
    }
    *uploaded = Some((buffer.clone(), bytes));
    Ok(())
}
