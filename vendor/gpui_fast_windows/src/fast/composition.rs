// Modified by Longbridge for gpui-fast.
//! Window composition on DirectComposition: native views between GPUI's base
//! content and its overlays (zed-industries/zed#62379).
//!
//! Upstream's `DirectComposition` puts one visual, holding the window's swap
//! chain, at the root of the window's target. Once GPUI orders the window's
//! surfaces, the target's root becomes a visual of ours holding, in order,
//! that base visual, a visual per further GPUI surface (each with a swap
//! chain of its own, drawn and presented with the base one), and the visuals
//! of native surfaces such as a WebView2 controller's. A native surface is a
//! visual with a rectangle clip, placed at its bounds and hidden through its
//! opacity.
//!
//! Recovering from a lost device recreates upstream's `DirectComposition`;
//! [`recreate`] then rebuilds the tree on it: new swap chains for the GPUI
//! surfaces, new visuals for the native surfaces (whose owners are told
//! through their `compositor_recreated` callbacks), and the surfaces' order.

use std::{
    any::Any,
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
};

use anyhow::{Context as _, Result};
use collections::{FxHashMap, FxHashSet};
use gpui::{
    Bounds, ComposedScene, CompositionSurfaceId, DevicePixels, PlatformCompositionSurface,
    PlatformCompositionSurfaceContent, PlatformSurfaceAttachment, Point,
    WindowBackgroundAppearance,
};
use gpui_util::ResultExt;
use windows::{
    Win32::{
        Foundation::HWND,
        Graphics::{
            Direct3D11::{ID3D11RenderTargetView, ID3D11Texture2D},
            DirectComposition::{
                IDCompositionDevice, IDCompositionRectangleClip, IDCompositionVisual,
                IDCompositionVisual3,
            },
            Dxgi::{DXGI_PRESENT, DXGI_SWAP_CHAIN_FLAG, IDXGISwapChain1},
        },
        UI::WindowsAndMessaging::GetClassNameW,
    },
    core::{IUnknown, Interface},
};

use crate::directx_renderer::{
    BUFFER_COUNT, DirectComposition, DirectXRendererDevices, RENDER_TARGET_FORMAT,
    create_render_target_and_its_view, create_swap_chain_for_composition,
};
use crate::{DirectXRenderer, WindowsWindow};

/// The renderer's window composition state.
#[derive(Default)]
pub(crate) struct DirectXComposition {
    /// The swap chains of the GPUI surfaces above the base one.
    overlays: FxHashMap<CompositionSurfaceId, OverlayResources>,
    /// The GPUI surface drawn into the window's own swap chain, once GPUI has
    /// ordered the window's surfaces.
    base: Option<CompositionSurfaceId>,
    /// The surfaces in the order last set.
    surfaces: Vec<PlatformCompositionSurface>,
    /// The native surfaces created for the window that may still be alive.
    native_surfaces: Vec<Weak<RefCell<PortalState>>>,
    /// The visuals on the current DirectComposition device.
    tree: Option<VisualTree>,
}

struct OverlayResources {
    swap_chain: IDXGISwapChain1,
    render_target: Option<ID3D11Texture2D>,
    render_target_view: Option<ID3D11RenderTargetView>,
}

struct VisualTree {
    /// The target's root once the surfaces are ordered.
    root: IDCompositionVisual,
    gpui_visuals: FxHashMap<CompositionSurfaceId, IDCompositionVisual>,
}

struct Portal {
    state: Rc<RefCell<PortalState>>,
}

struct PortalState {
    comp_device: IDCompositionDevice,
    visual: IDCompositionVisual,
    clip: IDCompositionRectangleClip,
    bounds: Cell<Bounds<DevicePixels>>,
    parent_origin: Cell<Point<DevicePixels>>,
    visible: Cell<bool>,
    compositor_recreated_callback: RefCell<Option<Rc<dyn Fn(Box<dyn Any>) -> Result<()>>>>,
}

/// Forwarded to by `WindowsWindow::draw_composed`.
pub(crate) fn draw_composed(window: &WindowsWindow, scene: ComposedScene<'_>) {
    let background_appearance = window.state.background_appearance.get();
    let presented = {
        let mut renderer = window.state.renderer.borrow_mut();
        let drawable = !renderer.skip_draws;
        draw(&mut renderer, scene, background_appearance).log_err().is_some() && drawable
    };
    crate::fast::startup::complete_frame(window, presented);
}

/// Forwarded to by `WindowsWindow::enable_window_composition`.
pub(crate) fn enable_window_composition(window: &WindowsWindow) -> Result<()> {
    ensure_enabled(&window.state.renderer.borrow())
}

/// Forwarded to by `WindowsWindow::create_native_surface`.
pub(crate) fn create_native_surface(
    window: &WindowsWindow,
) -> Result<Rc<dyn PlatformSurfaceAttachment>> {
    let mut renderer = window.state.renderer.borrow_mut();
    ensure_enabled(&renderer)?;
    let composition = renderer
        .direct_composition
        .as_ref()
        .context("DirectComposition is disabled")?;
    let state = Rc::new(RefCell::new(PortalState::new(&composition.comp_device)?));
    renderer
        .fast_composition
        .native_surfaces
        .push(Rc::downgrade(&state));
    Ok(Rc::new(Portal { state }))
}

/// Forwarded to by `WindowsWindow::set_composition_order`.
pub(crate) fn set_composition_order(
    window: &WindowsWindow,
    surfaces: &[PlatformCompositionSurface],
) -> Result<()> {
    set_order(&mut window.state.renderer.borrow_mut(), surfaces)
}

/// Forwarded to by `translate_accelerator`: whether `hwnd` is one of GPUI's
/// windows, whose key downs GPUI handles before they are translated. Key
/// downs of other windows, such as a WebView2 controller's child windows,
/// are left to their own window procedures.
pub(crate) fn is_gpui_window(hwnd: HWND) -> Option<()> {
    let class_name = unsafe { crate::window::WINDOW_CLASS_NAME.as_wide() };
    let mut buffer = [0u16; 64];
    let length = unsafe { GetClassNameW(hwnd, &mut buffer) };
    let length = usize::try_from(length).ok()?;
    (buffer.get(..length)? == class_name).then_some(())
}

/// Forwarded to by `DirectXRenderer::handle_device_lost_impl` before it
/// releases the lost device: drops what was created on it.
pub(crate) fn release(renderer: &mut DirectXRenderer) {
    let state = &mut renderer.fast_composition;
    state.overlays.clear();
    state.tree = None;
}

/// Forwarded to by `DirectXRenderer::handle_device_lost_impl` once it has
/// recreated the devices and DirectComposition: rebuilds the composition on
/// them.
pub(crate) fn recreate(renderer: &mut DirectXRenderer) -> Result<()> {
    let DirectXRenderer {
        devices,
        direct_composition,
        fast_composition: state,
        width,
        height,
        ..
    } = renderer;
    let overlay_ids = state
        .surfaces
        .iter()
        .filter(|surface| {
            matches!(surface.content, PlatformCompositionSurfaceContent::Gpui)
                && Some(surface.id) != state.base
        })
        .map(|surface| surface.id)
        .collect::<Vec<_>>();
    state
        .native_surfaces
        .retain(|surface| surface.strong_count() > 0);
    let Some(composition) = direct_composition.as_ref() else {
        anyhow::ensure!(
            overlay_ids.is_empty(),
            "DirectComposition missing for GPUI surface"
        );
        return Ok(());
    };
    let devices = devices.as_ref().context("devices missing")?;

    let mut tree = VisualTree::new(composition)?;
    let mut overlays = FxHashMap::default();
    for surface in overlay_ids {
        let resources = OverlayResources::new(devices, *width, *height)?;
        tree.set_gpui_swap_chain(composition, surface, &resources.swap_chain)?;
        overlays.insert(surface, resources);
    }
    for surface in &state.native_surfaces {
        let Some(live_surface) = surface.upgrade() else {
            continue;
        };
        PortalState::rebind(&mut live_surface.borrow_mut(), &composition.comp_device)
            .context("Recreating DirectComposition native surface")?;
    }
    state.overlays = overlays;
    if let Some(base_surface) = state.base
        && !state.surfaces.is_empty()
    {
        let platform_context = composition.comp_device.cast::<IUnknown>()?;
        for surface in &state.surfaces {
            match &surface.content {
                PlatformCompositionSurfaceContent::Native(platform_surface)
                | PlatformCompositionSurfaceContent::ExternalGpu(platform_surface) => {
                    platform_surface
                        .compositor_recreated(&platform_context)
                        .context("Rebinding platform composition surface")?;
                }
                PlatformCompositionSurfaceContent::Gpui => {}
            }
        }
        tree.set_surface_order(composition, &state.surfaces, base_surface)
            .context("Restoring DirectComposition surface order")?;
    }
    state.tree = Some(tree);
    Ok(())
}

/// Forwarded to by `DirectXRenderer::resize` once the window's swap chain
/// has its new size: resizes the other GPUI surfaces' swap chains.
pub(crate) fn resize(renderer: &mut DirectXRenderer) -> Result<()> {
    let devices = renderer.devices.as_ref().context("devices missing")?;
    for resources in renderer.fast_composition.overlays.values_mut() {
        resources.resize(devices, renderer.width, renderer.height)?;
    }
    Ok(())
}

fn ensure_enabled(renderer: &DirectXRenderer) -> Result<()> {
    anyhow::ensure!(
        renderer.direct_composition.is_some(),
        "DirectComposition is disabled"
    );
    Ok(())
}

/// Draws each GPUI surface's part of `scene` into its swap chain and
/// presents them all. Until GPUI orders the surfaces, the whole scene is
/// drawn as `DirectXRenderer::draw` draws it.
fn draw(
    renderer: &mut DirectXRenderer,
    scene: ComposedScene<'_>,
    background_appearance: WindowBackgroundAppearance,
) -> Result<()> {
    let Some(base_surface) = renderer.fast_composition.base else {
        return renderer.draw(scene.scene(), background_appearance);
    };
    if renderer.skip_draws {
        return Ok(());
    }

    for layer in scene.layers() {
        let surface_scene = scene.layer_scene(layer)?;
        if layer.surface == base_surface {
            renderer.render(&surface_scene, background_appearance)?;
            continue;
        }
        // `render` draws into the window's render target view, which paths
        // also bind again after their intermediate pass; lend it the
        // surface's view for the draw.
        swap_render_target(renderer, layer.surface)?;
        let drawn = renderer.render(&surface_scene, WindowBackgroundAppearance::Transparent);
        swap_render_target(renderer, layer.surface)?;
        drawn?;
    }

    renderer.present().context("presenting base swap chain")?;
    for resources in renderer.fast_composition.overlays.values() {
        unsafe { resources.swap_chain.Present(0, DXGI_PRESENT(0)) }
            .ok()
            .context("presenting GPUI composition swap chain")?;
    }
    Ok(())
}

/// Exchanges the window's render target view with `surface`'s.
fn swap_render_target(renderer: &mut DirectXRenderer, surface: CompositionSurfaceId) -> Result<()> {
    let overlay = renderer
        .fast_composition
        .overlays
        .get_mut(&surface)
        .context("GPUI composition resources missing")?;
    let resources = renderer.resources.as_mut().context("resources missing")?;
    std::mem::swap(
        &mut resources.render_target_view,
        &mut overlay.render_target_view,
    );
    Ok(())
}

fn set_order(
    renderer: &mut DirectXRenderer,
    surfaces: &[PlatformCompositionSurface],
) -> Result<()> {
    let base = surfaces
        .iter()
        .find_map(|surface| match surface.content {
            PlatformCompositionSurfaceContent::Gpui => Some(surface.id),
            PlatformCompositionSurfaceContent::Native(_)
            | PlatformCompositionSurfaceContent::ExternalGpu(_) => None,
        })
        .context("composition has no GPUI base surface")?;
    let DirectXRenderer {
        devices,
        direct_composition,
        fast_composition: state,
        width,
        height,
        ..
    } = renderer;
    let devices = devices.as_ref().context("devices missing")?;
    let composition = direct_composition
        .as_ref()
        .context("DirectComposition is disabled")?;
    let DirectXComposition {
        overlays,
        base: previous_base,
        surfaces: previous_surfaces,
        tree,
        ..
    } = state;
    let tree = match tree {
        Some(tree) => tree,
        None => tree.insert(VisualTree::new(composition)?),
    };

    for surface in surfaces {
        if !matches!(surface.content, PlatformCompositionSurfaceContent::Gpui) {
            continue;
        }
        if surface.id == base || overlays.contains_key(&surface.id) {
            continue;
        }
        let resources = OverlayResources::new(devices, *width, *height)?;
        tree.set_gpui_swap_chain(composition, surface.id, &resources.swap_chain)?;
        overlays.insert(surface.id, resources);
    }
    let active_gpui_surfaces = surfaces
        .iter()
        .filter_map(|surface| match surface.content {
            PlatformCompositionSurfaceContent::Gpui if surface.id != base => Some(surface.id),
            _ => None,
        })
        .collect::<FxHashSet<_>>();
    if let Err(error) = tree.set_surface_order(composition, surfaces, base) {
        if let Some(previous_base) = *previous_base
            && !previous_surfaces.is_empty()
            && let Err(rollback_error) =
                tree.set_surface_order(composition, previous_surfaces, previous_base)
        {
            return Err(error).context(format!(
                "restoring the previous DirectComposition tree also failed: {rollback_error:#}"
            ));
        }
        return Err(error);
    }
    overlays.retain(|id, _| active_gpui_surfaces.contains(id));
    tree.gpui_visuals
        .retain(|id, _| active_gpui_surfaces.contains(id));
    *previous_base = Some(base);
    *previous_surfaces = surfaces.to_vec();
    Ok(())
}

impl VisualTree {
    fn new(composition: &DirectComposition) -> Result<Self> {
        Ok(Self {
            root: unsafe { composition.comp_device.CreateVisual() }?,
            gpui_visuals: FxHashMap::default(),
        })
    }

    fn set_gpui_swap_chain(
        &mut self,
        composition: &DirectComposition,
        surface: CompositionSurfaceId,
        swap_chain: &IDXGISwapChain1,
    ) -> Result<()> {
        let visual = unsafe { composition.comp_device.CreateVisual()? };
        unsafe {
            visual.SetContent(swap_chain)?;
            composition.comp_device.Commit()?;
        }
        self.gpui_visuals.insert(surface, visual);
        Ok(())
    }

    /// Makes the root hold `surfaces`' visuals in order, each nested in its
    /// parent's, with `base_surface` drawn by upstream's visual.
    fn set_surface_order(
        &self,
        composition: &DirectComposition,
        surfaces: &[PlatformCompositionSurface],
        base_surface: CompositionSurfaceId,
    ) -> Result<()> {
        let visuals = surfaces
            .iter()
            .map(|surface| match &surface.content {
                PlatformCompositionSurfaceContent::Gpui if surface.id == base_surface => {
                    Ok((surface.id, composition.comp_visual.clone()))
                }
                PlatformCompositionSurfaceContent::Gpui => self
                    .gpui_visuals
                    .get(&surface.id)
                    .cloned()
                    .map(|visual| (surface.id, visual))
                    .context("GPUI composition visual is missing"),
                PlatformCompositionSurfaceContent::Native(platform_surface)
                | PlatformCompositionSurfaceContent::ExternalGpu(platform_surface) => {
                    let handle = platform_surface.platform_handle()?;
                    let unknown = handle.downcast::<IUnknown>().map_err(|_| {
                        anyhow::anyhow!("native surface is not a DirectComposition visual")
                    })?;
                    unknown
                        .cast::<IDCompositionVisual>()
                        .map(|visual| (surface.id, visual))
                        .map_err(Into::into)
                }
            })
            .collect::<Result<Vec<_>>>()?;
        let visuals_by_id = visuals.iter().cloned().collect::<FxHashMap<_, _>>();
        for surface in surfaces {
            match &surface.content {
                PlatformCompositionSurfaceContent::Gpui => {
                    let visual = visuals_by_id
                        .get(&surface.id)
                        .context("GPUI composition visual is missing")?;
                    unsafe {
                        visual.SetOffsetX2(
                            (surface.window_origin.x - surface.parent_origin.x).0 as f32,
                        )?;
                        visual.SetOffsetY2(
                            (surface.window_origin.y - surface.parent_origin.y).0 as f32,
                        )?;
                    }
                }
                PlatformCompositionSurfaceContent::Native(platform_surface)
                | PlatformCompositionSurfaceContent::ExternalGpu(platform_surface) => {
                    platform_surface.set_parent_origin(surface.parent_origin)?;
                }
            }
        }
        unsafe {
            // Upstream's visual is the target's root until the first order.
            composition.comp_target.SetRoot(&self.root)?;
            self.root.RemoveAllVisuals()?;
            for surface in surfaces {
                if matches!(surface.content, PlatformCompositionSurfaceContent::Gpui) {
                    visuals_by_id
                        .get(&surface.id)
                        .context("GPUI composition visual is missing")?
                        .RemoveAllVisuals()?;
                }
            }
            let mut previous_by_parent = FxHashMap::default();
            for surface in surfaces {
                let visual = visuals_by_id
                    .get(&surface.id)
                    .context("composition visual is missing")?;
                let parent = surface.parent.and_then(|parent| visuals_by_id.get(&parent));
                let container = parent.unwrap_or(&self.root);
                let previous = previous_by_parent.get(&surface.parent);
                container.AddVisual(visual, true, previous)?;
                previous_by_parent.insert(surface.parent, visual.clone());
            }
            composition.comp_device.Commit()?;
        }
        Ok(())
    }
}

impl PortalState {
    fn new(comp_device: &IDCompositionDevice) -> Result<Self> {
        let visual = unsafe { comp_device.CreateVisual() }?;
        let clip = unsafe { comp_device.CreateRectangleClip() }?;
        unsafe {
            visual.SetClip(&clip)?;
        }
        Ok(Self {
            comp_device: comp_device.clone(),
            visual,
            clip,
            bounds: Cell::new(Bounds::default()),
            parent_origin: Cell::new(Point::default()),
            visible: Cell::new(true),
            compositor_recreated_callback: RefCell::new(None),
        })
    }

    /// Replaces `state`'s visual with one on `comp_device`, placed, clipped
    /// and shown as the old one was.
    fn rebind(state: &mut Self, comp_device: &IDCompositionDevice) -> Result<()> {
        let bounds = state.bounds.get();
        let parent_origin = state.parent_origin.get();
        let visible = state.visible.get();
        let compositor_recreated_callback = state.compositor_recreated_callback.borrow().clone();
        let replacement = Self::new(comp_device)?;
        let x = (bounds.origin.x - parent_origin.x).0 as f32;
        let y = (bounds.origin.y - parent_origin.y).0 as f32;
        let width = bounds.size.width.0.max(0) as f32;
        let height = bounds.size.height.0.max(0) as f32;
        unsafe {
            replacement.visual.SetOffsetX2(x)?;
            replacement.visual.SetOffsetY2(y)?;
            replacement.clip.SetLeft2(0.0)?;
            replacement.clip.SetTop2(0.0)?;
            replacement.clip.SetRight2(width)?;
            replacement.clip.SetBottom2(height)?;
            replacement
                .visual
                .cast::<IDCompositionVisual3>()?
                .SetOpacity2(if visible { 1.0 } else { 0.0 })?;
        }
        replacement.bounds.set(bounds);
        replacement.parent_origin.set(parent_origin);
        replacement.visible.set(visible);
        *replacement.compositor_recreated_callback.borrow_mut() = compositor_recreated_callback;
        *state = replacement;
        Ok(())
    }
}

impl PlatformSurfaceAttachment for Portal {
    fn set_bounds(&self, bounds: Bounds<DevicePixels>) -> Result<()> {
        let state = self.state.borrow();
        let parent_origin = state.parent_origin.get();
        let x = (bounds.origin.x - parent_origin.x).0 as f32;
        let y = (bounds.origin.y - parent_origin.y).0 as f32;
        let width = bounds.size.width.0.max(0) as f32;
        let height = bounds.size.height.0.max(0) as f32;
        unsafe {
            state.visual.SetOffsetX2(x)?;
            state.visual.SetOffsetY2(y)?;
            state.clip.SetLeft2(0.0)?;
            state.clip.SetTop2(0.0)?;
            state.clip.SetRight2(width)?;
            state.clip.SetBottom2(height)?;
            state.comp_device.Commit()?;
        }
        state.bounds.set(bounds);
        Ok(())
    }

    fn bounds(&self) -> Bounds<DevicePixels> {
        self.state.borrow().bounds.get()
    }

    fn set_parent_origin(&self, origin: Point<DevicePixels>) -> Result<()> {
        let bounds = {
            let state = self.state.borrow();
            state.parent_origin.set(origin);
            state.bounds.get()
        };
        self.set_bounds(bounds)
    }

    fn set_visible(&self, visible: bool) -> Result<()> {
        let state = self.state.borrow();
        if state.visible.get() != visible {
            unsafe {
                state
                    .visual
                    .cast::<IDCompositionVisual3>()?
                    .SetOpacity2(if visible { 1.0 } else { 0.0 })?;
                state.comp_device.Commit()?;
            }
            state.visible.set(visible);
        }
        Ok(())
    }

    fn compositor_recreated(&self, _platform_context: &dyn Any) -> Result<()> {
        let callback = self
            .state
            .borrow()
            .compositor_recreated_callback
            .borrow()
            .clone();
        if let Some(callback) = callback {
            callback(self.platform_handle()?)?;
        }
        Ok(())
    }

    fn set_compositor_recreated_callback(
        &self,
        callback: Rc<dyn Fn(Box<dyn Any>) -> Result<()>>,
    ) -> Result<()> {
        *self
            .state
            .borrow()
            .compositor_recreated_callback
            .borrow_mut() = Some(callback);
        Ok(())
    }

    fn platform_handle(&self) -> Result<Box<dyn Any>> {
        Ok(Box::new(self.state.borrow().visual.cast::<IUnknown>()?))
    }
}

impl OverlayResources {
    fn new(devices: &DirectXRendererDevices, width: u32, height: u32) -> Result<Self> {
        let swap_chain = create_swap_chain_for_composition(
            &devices.dxgi_factory,
            &devices.device,
            width,
            height,
        )?;
        let (render_target, render_target_view) =
            create_render_target_and_its_view(&swap_chain, &devices.device)?;
        Ok(Self {
            swap_chain,
            render_target: Some(render_target),
            render_target_view,
        })
    }

    fn resize(&mut self, devices: &DirectXRendererDevices, width: u32, height: u32) -> Result<()> {
        self.render_target.take();
        self.render_target_view.take();
        unsafe {
            self.swap_chain.ResizeBuffers(
                BUFFER_COUNT as u32,
                width,
                height,
                RENDER_TARGET_FORMAT,
                DXGI_SWAP_CHAIN_FLAG(0),
            )?;
        }
        let (render_target, render_target_view) =
            create_render_target_and_its_view(&self.swap_chain, &devices.device)?;
        self.render_target = Some(render_target);
        self.render_target_view = render_target_view;
        Ok(())
    }
}
