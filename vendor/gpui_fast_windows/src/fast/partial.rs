// Modified by Longbridge for gpui-fast.
//! Partial redraw on software adapters: a frame that differs little from
//! the last one is made from it, not drawn whole.
//!
//! GPUI draws every frame whole. On a GPU that is cheap; on WARP (a machine
//! without a GPU) the cost of a frame is the pixels it shades, and a
//! scrolled frame shades the whole window although almost all of it only
//! moved. Measured on a 1440x900 window on two cores, after overdraw
//! culling: ~100 ms a scrolled frame drawn whole, ~19 ms made from the
//! last one.
//!
//! The frames are drawn into two canvases, in turn, and the canvas is
//! copied to the swap chain's buffer. Each frame's primitives are reduced
//! to *signatures* ([`Sig`]: what a primitive looks like apart from where it
//! is, its bounds, its mask, what it may touch) and compared with the last
//! frame's ([`plan`]):
//!
//! - Nothing changed: the last canvas is shown again.
//! - Otherwise the last canvas is copied, and the scene is drawn again only
//!   inside the *damage*: the rectangles of the primitives that came, went
//!   or changed, scissored.
//! - A scroll: when the changed primitives are the same ones moved by one
//!   whole number of pixels vertically inside one clip, that clip's pixels
//!   are copied moved, and the damage is the strip the scroll exposed, what
//!   did not move with it, and what else changed.
//!
//! What keeps this exact:
//!
//! - A pixel outside the damage is covered by the same primitives as its
//!   source pixel in the last frame: moved pairs (same look, same clip where
//!   the copy lands), or *plain* quads that did not move (a solid, square,
//!   borderless quad is the same colour everywhere; only the bands at its
//!   top and bottom that the copy moved are damaged). Everything else over
//!   the scrolled clip is damaged.
//! - The primitives a kept pixel is made of are painted in the same order
//!   as before: the draw orders of kept primitives must rise together in
//!   both frames, else the frame is drawn whole.
//! - A tile uploaded since the last frame (an image replaced in its atlas
//!   slot, a glyph evicted and rasterized again) makes its sprites changed,
//!   whatever their bytes say ([`Uploads`]); a dynamic texture's sprites
//!   always are.
//! - A sprite drawn through a transformation (a rotating icon) reaches
//!   wherever its matrix takes its bounds, and is never a moved pair.
//! - A change of the clear colour (the window's background appearance)
//!   draws the frame whole.
//! - A rounded clip (a sprite standing for a scene drawn through its
//!   rounded bounds) looks like that scene seen from the sprite: its
//!   signature is the scene's, relative to it, so it stays or moves as one.
//! - Nothing is copied for a fractional scroll offset or a clip off the
//!   pixel grid: what moved is damage, or the frame is drawn whole. Frames
//!   with scroll layers or surfaces, or with damage over [`MAX_DAMAGE`] of
//!   the window, are drawn whole.
//!
//! One difference remains: an image drawn scaled samples its texture at
//! positions computed in floating point from where it is, so the same image
//! drawn 48 pixels higher can differ by 1/255 in a few pixels. A moved pixel
//! was drawn once, wherever it was first drawn, and copied since; a copy
//! adds nothing, so the difference never grows. Images drawn at their own
//! size, glyphs and quads move exactly.
//!
//! On software adapters only ([`wanted`]): on a GPU a whole frame costs
//! less than the comparison saves.

use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};

use anyhow::{Context as _, Result};
use gpui::{
    AtlasTextureId, AtlasTextureKind, AtlasTile, Background, Bounds, ContentMask, Corners, Edges,
    Hsla, IsZero, ScaledPixels, Scene, TransformationMatrix,
};
use windows::Win32::{
    Foundation::RECT,
    Graphics::{
        Direct3D11::{
            D3D11_BIND_RENDER_TARGET, D3D11_BOX, D3D11_CULL_BACK, D3D11_FILL_SOLID,
            D3D11_RASTERIZER_DESC, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, ID3D11Device,
            ID3D11DeviceContext, ID3D11DeviceContext1, ID3D11RasterizerState,
            ID3D11RenderTargetView, ID3D11Texture2D, ID3D11View,
        },
        Dxgi::{Common::DXGI_SAMPLE_DESC, DXGI_ADAPTER_FLAG_SOFTWARE, IDXGIAdapter1},
    },
};
use windows_core::Interface;

use crate::DirectXAtlas;
use crate::directx_renderer::RENDER_TARGET_FORMAT;

/// More damage than this share of the window: the frame is drawn whole.
const MAX_DAMAGE: f32 = 0.6;
/// More damage rectangles than this are merged into their box.
const MAX_RECTS: usize = 8;
/// More rectangles than this are first gathered into bands of the window.
const MAX_MERGED: usize = 64;
/// A scroll needs this many primitives moved alike.
const MIN_MOVED: usize = 4;
/// How far two coordinates computed apart may differ and be the same.
const SAME: f32 = 1.0 / 1024.0;

/// Whether frames on `adapter` are made from the last one: on a software
/// adapter (WARP, the Microsoft Basic Render Driver, which Windows also
/// lists without the software flag). `GPUI_PARTIAL_REDRAW=1` turns it on
/// everywhere, `=0` off.
pub(crate) fn wanted(adapter: &IDXGIAdapter1) -> bool {
    match std::env::var("GPUI_PARTIAL_REDRAW").as_deref() {
        Ok("1") => return true,
        Ok("0") => return false,
        _ => {}
    }
    // SAFETY: a query on a live adapter.
    unsafe { adapter.GetDesc1() }.is_ok_and(|desc| {
        desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0
            || (desc.VendorId, desc.DeviceId) == (0x1414, 0x8c)
    })
}

// --- Tiles uploaded since the last frame -------------------------------------

/// The atlas tiles uploaded since the last frame was drawn: their sprites
/// show something new even when their primitives are byte for byte the
/// last frame's (the atlas gives a freed tile's place and id out again).
#[derive(Default)]
pub(crate) struct Uploads {
    tiles: HashSet<(AtlasTextureId, u32)>,
    /// More uploads than are worth keeping (nothing takes them: the window
    /// is drawn another way): the next frame is drawn whole.
    lost: bool,
}

impl Uploads {
    /// The most tiles kept between two frames.
    const MOST: usize = 4096;

    pub(crate) fn record(&mut self, tile: &AtlasTile) {
        if self.tiles.len() >= Self::MOST {
            self.tiles.clear();
            self.lost = true;
        }
        self.tiles.insert((tile.texture_id, tile.tile_id.0));
    }

    pub(crate) fn clear(&mut self) {
        self.tiles.clear();
        self.lost = true;
    }
}

impl DirectXAtlas {
    /// The tiles uploaded since this was last called; `None` when they are
    /// not all known.
    pub(crate) fn take_uploads(&self) -> Option<HashSet<(AtlasTextureId, u32)>> {
        let mut state = self.0.lock();
        let uploads = &mut state.backend.fast_partial_uploads;
        let tiles = std::mem::take(&mut uploads.tiles);
        (!std::mem::take(&mut uploads.lost)).then_some(tiles)
    }
}

// --- Signatures --------------------------------------------------------------

/// A rectangle in device pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Area {
    pub(crate) x0: f32,
    pub(crate) y0: f32,
    pub(crate) x1: f32,
    pub(crate) y1: f32,
}

impl Area {
    fn of(b: &Bounds<ScaledPixels>) -> Area {
        Area {
            x0: b.origin.x.0,
            y0: b.origin.y.0,
            x1: b.origin.x.0 + b.size.width.0,
            y1: b.origin.y.0 + b.size.height.0,
        }
    }

    fn and(&self, o: &Area) -> Area {
        Area {
            x0: self.x0.max(o.x0),
            y0: self.y0.max(o.y0),
            x1: self.x1.min(o.x1),
            y1: self.y1.min(o.y1),
        }
    }

    fn union(&self, o: &Area) -> Area {
        Area {
            x0: self.x0.min(o.x0),
            y0: self.y0.min(o.y0),
            x1: self.x1.max(o.x1),
            y1: self.y1.max(o.y1),
        }
    }

    fn is_empty(&self) -> bool {
        !(self.x1 > self.x0 && self.y1 > self.y0)
    }

    fn area(&self) -> f32 {
        if self.is_empty() {
            0.0
        } else {
            (self.x1 - self.x0) * (self.y1 - self.y0)
        }
    }

    fn moved(&self, dy: f32) -> Area {
        Area {
            y0: self.y0 + dy,
            y1: self.y1 + dy,
            ..*self
        }
    }

    fn grown(&self, by: f32) -> Area {
        Area {
            x0: self.x0 - by,
            y0: self.y0 - by,
            x1: self.x1 + by,
            y1: self.y1 + by,
        }
    }

    fn touches(&self, o: &Area) -> bool {
        self.x0 <= o.x1 && o.x0 <= self.x1 && self.y0 <= o.y1 && o.y0 <= self.y1
    }

    fn within(&self, o: &Area) -> bool {
        self.is_empty()
            || (self.x0 >= o.x0 - SAME
                && self.y0 >= o.y0 - SAME
                && self.x1 <= o.x1 + SAME
                && self.y1 <= o.y1 + SAME)
    }

    fn same(&self, o: &Area) -> bool {
        (self.is_empty() && o.is_empty())
            || ((self.x0 - o.x0).abs() < SAME
                && (self.y0 - o.y0).abs() < SAME
                && (self.x1 - o.x1).abs() < SAME
                && (self.y1 - o.y1).abs() < SAME)
    }

    /// The pixels it touches.
    fn pixels(&self) -> Area {
        Area {
            x0: self.x0.floor(),
            y0: self.y0.floor(),
            x1: self.x1.ceil(),
            y1: self.y1.ceil(),
        }
    }

    fn on_grid(&self) -> bool {
        let on = |v: f32| (v - v.round()).abs() < SAME;
        on(self.x0) && on(self.y0) && on(self.x1) && on(self.y1)
    }

    fn bits(&self) -> [u32; 4] {
        [
            self.x0.to_bits(),
            self.y0.to_bits(),
            self.x1.to_bits(),
            self.y1.to_bits(),
        ]
    }

    fn rect(&self) -> RECT {
        RECT {
            left: self.x0 as i32,
            top: self.y0 as i32,
            right: self.x1 as i32,
            bottom: self.y1 as i32,
        }
    }
}

/// A primitive as the comparison sees it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Sig {
    /// What it looks like apart from where it is, in what order and through
    /// what mask.
    key: u64,
    order: u32,
    /// Its bounds, its mask, and what it may touch: its bounds (a shadow's
    /// tail, a pixel of antialiasing included) within its mask.
    bounds: Area,
    mask: Area,
    reach: Area,
    /// The same colour everywhere, through any mask.
    plain: bool,
    /// Shows something new whatever its bytes say.
    fresh: bool,
}

struct Key(std::collections::hash_map::DefaultHasher);

impl Key {
    fn new(kind: u8) -> Key {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        kind.hash(&mut hasher);
        Key(hasher)
    }

    fn f(mut self, v: f32) -> Key {
        v.to_bits().hash(&mut self.0);
        self
    }

    fn u(mut self, v: u64) -> Key {
        v.hash(&mut self.0);
        self
    }

    fn color(self, c: &Hsla) -> Key {
        self.f(c.h).f(c.s).f(c.l).f(c.a)
    }

    fn corners(self, c: &Corners<ScaledPixels>) -> Key {
        self.f(c.top_left.0)
            .f(c.top_right.0)
            .f(c.bottom_right.0)
            .f(c.bottom_left.0)
    }

    fn edges(self, e: &Edges<ScaledPixels>) -> Key {
        self.f(e.top.0).f(e.right.0).f(e.bottom.0).f(e.left.0)
    }

    fn size(self, b: &Bounds<ScaledPixels>) -> Key {
        self.f(b.size.width.0).f(b.size.height.0)
    }

    fn tile(self, t: &AtlasTile) -> Key {
        self.u(t.texture_id.index as u64)
            .u(t.texture_id.kind as u64)
            .u(t.tile_id.0 as u64)
            .u(t.bounds.origin.x.0 as u64)
            .u(t.bounds.origin.y.0 as u64)
            .u(t.bounds.size.width.0 as u64)
            .u(t.bounds.size.height.0 as u64)
    }

    /// A background: its colour, or for a gradient (rare) its description.
    fn background(mut self, background: &Background) -> Key {
        match background.as_solid() {
            Some(color) => self.u(0).color(&color),
            None => {
                format!("{background:?}").hash(&mut self.0);
                self
            }
        }
    }

    /// A sprite's transformation. A transformed sprite is drawn where its
    /// matrix takes its bounds, so it is keyed with where they are: moved,
    /// it is another sprite.
    fn transformation(self, t: &TransformationMatrix, bounds: &Bounds<ScaledPixels>) -> Key {
        let key = self
            .f(t.rotation_scale[0][0])
            .f(t.rotation_scale[0][1])
            .f(t.rotation_scale[1][0])
            .f(t.rotation_scale[1][1])
            .f(t.translation[0])
            .f(t.translation[1]);
        if is_unit(t) {
            key
        } else {
            key.f(bounds.origin.x.0).f(bounds.origin.y.0)
        }
    }

    fn done(self) -> u64 {
        self.0.finish()
    }
}

fn is_unit(t: &TransformationMatrix) -> bool {
    t.rotation_scale == [[1.0, 0.0], [0.0, 1.0]] && t.translation == [0.0, 0.0]
}

/// The box of where `t` takes `bounds`. The matrix is read both ways round
/// (rows or columns first), so that the box holds whichever the shader does.
fn transformed(bounds: &Bounds<ScaledPixels>, t: &TransformationMatrix) -> Area {
    let area = Area::of(bounds);
    if is_unit(t) {
        return area;
    }
    let m = t.rotation_scale;
    let mut out: Option<Area> = None;
    for (x, y) in [
        (area.x0, area.y0),
        (area.x1, area.y0),
        (area.x0, area.y1),
        (area.x1, area.y1),
    ] {
        for (tx, ty) in [
            (m[0][0] * x + m[0][1] * y, m[1][0] * x + m[1][1] * y),
            (m[0][0] * x + m[1][0] * y, m[0][1] * x + m[1][1] * y),
        ] {
            let (tx, ty) = (tx + t.translation[0], ty + t.translation[1]);
            let point = Area {
                x0: tx,
                y0: ty,
                x1: tx,
                y1: ty,
            };
            out = Some(out.map_or(point, |o| o.union(&point)));
        }
    }
    out.unwrap_or(area)
}

fn sig(
    key: u64,
    order: u32,
    bounds: &Bounds<ScaledPixels>,
    mask: &ContentMask<ScaledPixels>,
    grow: f32,
) -> Sig {
    let bounds = Area::of(bounds);
    let mask = Area::of(&mask.bounds);
    Sig {
        key,
        order,
        bounds,
        mask,
        reach: bounds.grown(grow).and(&mask),
        plain: false,
        fresh: false,
    }
}

/// The scene's primitives as signatures, or `None` for a scene that cannot
/// be compared (scroll layers, whose tiles change under unchanged sprites;
/// surfaces). `uploaded`: the tiles uploaded since the last frame.
pub(crate) fn signatures(
    scene: &Scene,
    uploaded: &HashSet<(AtlasTextureId, u32)>,
    dynamic: &dyn Fn(AtlasTextureId) -> bool,
) -> Option<Vec<Sig>> {
    // A scroll layer's tiles change under sprites that stay the same.
    if scene.layers.frames.iter().any(|frame| frame.tile_size != 0) || !scene.surfaces.is_empty() {
        return None;
    }
    let fresh = |tile: &AtlasTile| {
        uploaded.contains(&(tile.texture_id, tile.tile_id.0)) || dynamic(tile.texture_id)
    };
    let mut out = Vec::with_capacity(scene.len());
    for q in &scene.quads {
        let solid = q.background.as_solid();
        let key = Key::new(1)
            .size(&q.bounds)
            .u(q.border_style as u64)
            .background(&q.background)
            .color(&q.border_color)
            .corners(&q.corner_radii)
            .edges(&q.border_widths)
            .done();
        let c = &q.corner_radii;
        let w = &q.border_widths;
        let mut s = sig(key, q.order, &q.bounds, &q.content_mask, 0.0);
        s.plain = solid.is_some()
            && c.top_left.is_zero()
            && c.top_right.is_zero()
            && c.bottom_left.is_zero()
            && c.bottom_right.is_zero()
            && w.top.is_zero()
            && w.right.is_zero()
            && w.bottom.is_zero()
            && w.left.is_zero();
        out.push(s);
    }
    for s in &scene.shadows {
        let key = Key::new(2)
            .size(&s.bounds)
            .f(s.blur_radius.0)
            .corners(&s.corner_radii)
            .color(&s.color)
            .f(s.shape_bounds.origin.x.0 - s.bounds.origin.x.0)
            .f(s.shape_bounds.origin.y.0 - s.bounds.origin.y.0)
            .size(&s.shape_bounds)
            .corners(&s.shape_corner_radii)
            .u((s.inset == true.into()) as u64)
            .done();
        let grow = 3.0 * s.blur_radius.0.max(0.0) + 1.0;
        out.push(sig(key, s.order, &s.bounds, &s.content_mask, grow));
    }
    for u in &scene.underlines {
        let key = Key::new(3)
            .size(&u.bounds)
            .color(&u.color)
            .f(u.thickness.0)
            .u((u.wavy == true.into()) as u64)
            .done();
        out.push(sig(key, u.order, &u.bounds, &u.content_mask, 1.0));
    }
    for g in &scene.monochrome_sprites {
        let t = &g.transformation;
        let key = Key::new(4)
            .size(&g.bounds)
            .tile(&g.tile)
            .color(&g.color)
            .transformation(t, &g.bounds)
            .done();
        let mut s = sig(key, g.order, &g.bounds, &g.content_mask, 1.0);
        s.reach = transformed(&g.bounds, t).grown(1.0).and(&s.mask);
        s.fresh = fresh(&g.tile);
        out.push(s);
    }
    for g in &scene.subpixel_sprites {
        let t = &g.transformation;
        let key = Key::new(5)
            .size(&g.bounds)
            .tile(&g.tile)
            .color(&g.color)
            .transformation(t, &g.bounds)
            .done();
        let mut s = sig(key, g.order, &g.bounds, &g.content_mask, 1.0);
        s.reach = transformed(&g.bounds, t).grown(1.0).and(&s.mask);
        s.fresh = fresh(&g.tile);
        out.push(s);
    }
    for p in &scene.polychrome_sprites {
        // A rounded clip: a sprite standing for a scene drawn through its
        // rounded bounds. It looks like that scene, seen from the sprite.
        if let Some(group) = crate::fast::rounded_clip::group(scene, p.tile.texture_id) {
            let child = group.content.scene()?;
            let inner = signatures(child, uploaded, dynamic)?;
            let (x, y) = (p.bounds.origin.x.0, p.bounds.origin.y.0);
            let mut key = Key::new(8)
                .size(&p.bounds)
                .corners(&p.corner_radii)
                .f(p.opacity);
            // The clip's vertical radii travel in a frame of the child's.
            for frame in &child.layers.frames {
                if frame.key.0 == u32::MAX {
                    let radii = frame.background;
                    key = key.f(radii.r).f(radii.g).f(radii.b).f(radii.a);
                }
            }
            let mut fresh_inside = false;
            for s in &inner {
                fresh_inside |= s.fresh;
                key = key.u(s.key).u(s.order as u64);
                for area in [&s.bounds, &s.mask] {
                    key = key
                        .f(area.x0 - x)
                        .f(area.y0 - y)
                        .f(area.x1 - x)
                        .f(area.y1 - y);
                }
            }
            let mut s = sig(key.done(), p.order, &p.bounds, &p.content_mask, 1.0);
            s.fresh = fresh_inside;
            out.push(s);
            continue;
        }
        let key = Key::new(6)
            .size(&p.bounds)
            .tile(&p.tile)
            .u((p.grayscale == true.into()) as u64)
            .f(p.opacity)
            .corners(&p.corner_radii)
            .done();
        let mut s = sig(key, p.order, &p.bounds, &p.content_mask, 1.0);
        s.fresh = fresh(&p.tile);
        out.push(s);
    }
    for p in &scene.paths {
        // A path is its tiles where they are: one that moved is new.
        let (order, key) = gpui::painted_path_key(p);
        let key = Key::new(7).u(key).done();
        out.push(sig(key, order, &p.painted_bounds(), p.content_mask(), 1.0));
    }
    Some(out)
}

// --- The plan ----------------------------------------------------------------

/// How a frame is made from the last one.
#[derive(Debug, PartialEq)]
pub(crate) struct Plan {
    /// The last frame's `.0` goes to `.1` (same size, whole pixels).
    pub(crate) shift: Option<(Area, Area)>,
    /// Where the scene is drawn again (whole pixels, within the window).
    pub(crate) damage: Vec<Area>,
}

fn placed(s: &Sig) -> (u64, [u32; 4], [u32; 4]) {
    (s.key, s.bounds.bits(), s.mask.bits())
}

/// A coordinate to 1/64 of a pixel: what finds candidates for a pair. Two
/// places computed apart (a scrolled place by layout, and the last one plus
/// the scroll) can differ in their last bits; candidates are then compared
/// within [`SAME`].
fn coarse(v: f32) -> i64 {
    (v * 64.0).round() as i64
}

/// A primitive apart from its vertical place.
fn shape(s: &Sig) -> (u64, i64, i64, i64) {
    (
        s.key,
        coarse(s.bounds.x0),
        coarse(s.bounds.x1 - s.bounds.x0),
        coarse(s.bounds.y1 - s.bounds.y0),
    )
}

/// Whether draw orders that rose in the last frame still rise: `pairs` are
/// the (last, new) orders of the primitives kept.
fn orders_agree(pairs: &mut [(u32, u32)]) -> bool {
    pairs.sort_unstable();
    let mut below = 0u32; // the highest new order among lower old orders
    let mut level = 0u32; // the highest new order so far
    let mut last_old = None;
    for &(old, new) in pairs.iter() {
        if last_old != Some(old) {
            below = level;
            last_old = Some(old);
        }
        if new < below {
            return false;
        }
        level = level.max(new);
    }
    true
}

fn tidy(rects: Vec<Area>, view: &Area) -> Vec<Area> {
    let mut rects: Vec<Area> = rects
        .into_iter()
        .map(|r| r.pixels().and(view))
        .filter(|r| !r.is_empty())
        .collect();
    if rects.len() > MAX_MERGED {
        // Many (a page changed): their boxes by band of the window, so that
        // what follows is quick. More damage than needed is still right.
        let band = ((view.y1 - view.y0) / 16.0).max(1.0);
        let mut bands: HashMap<i64, Area> = HashMap::new();
        for r in &rects {
            bands
                .entry((r.y0 / band).floor() as i64)
                .and_modify(|b| *b = b.union(r))
                .or_insert(*r);
        }
        rects = bands.into_values().collect();
    }
    loop {
        let mut merged = false;
        let mut i = 0;
        while i < rects.len() {
            let mut j = i + 1;
            while j < rects.len() {
                if rects[i].touches(&rects[j]) {
                    rects[i] = rects[i].union(&rects[j]);
                    rects.swap_remove(j);
                    merged = true;
                } else {
                    j += 1;
                }
            }
            i += 1;
        }
        if !merged {
            break;
        }
    }
    if rects.len() > MAX_RECTS {
        let all = rects.iter().skip(1).fold(rects[0], |a, r| a.union(r));
        return vec![all];
    }
    rects
}

/// How to make the frame of `new` from the frame of `old` in a window of
/// `view`, or `None` to draw it whole.
pub(crate) fn plan(old: &[Sig], new: &[Sig], view: Area) -> Option<Plan> {
    // The primitives that stayed as they were, as a multiset.
    let mut left: HashMap<_, Vec<usize>> = HashMap::new();
    for (i, s) in old.iter().enumerate().rev() {
        // Popped first to last: primitives alike at one place pair in the
        // sequence they are painted in.
        left.entry(placed(s)).or_default().push(i);
    }
    let mut stayed: Vec<Option<usize>> = vec![None; new.len()];
    let mut old_kept = vec![false; old.len()];
    for (i, s) in new.iter().enumerate() {
        if s.fresh {
            continue;
        }
        if let Some(j) = left.get_mut(&placed(s)).and_then(Vec::pop) {
            stayed[i] = Some(j);
            old_kept[j] = true;
        }
    }
    let changed_new: Vec<usize> = (0..new.len()).filter(|i| stayed[*i].is_none()).collect();
    let changed_old: Vec<usize> = (0..old.len()).filter(|j| !old_kept[*j]).collect();
    let mut orders: Vec<(u32, u32)> = stayed
        .iter()
        .enumerate()
        .filter_map(|(i, j)| j.map(|j| (old[j].order, new[i].order)))
        .collect();

    // A scroll: changed primitives of the same shape vote on how far they moved.
    let mut by_shape: HashMap<_, Vec<usize>> = HashMap::new();
    for &j in &changed_old {
        by_shape.entry(shape(&old[j])).or_default().push(j);
    }
    let mut votes: HashMap<i32, usize> = HashMap::new();
    let mut fractional = false;
    for &i in &changed_new {
        if new[i].fresh {
            continue;
        }
        for &j in by_shape.get(&shape(&new[i])).map_or(&[][..], Vec::as_slice) {
            let d = new[i].bounds.y0 - old[j].bounds.y0;
            if d.round() == 0.0 {
                continue;
            }
            if (d - d.round()).abs() < SAME {
                *votes.entry(d.round() as i32).or_default() += 1;
            } else {
                fractional = true;
            }
        }
    }
    let scroll = votes
        .iter()
        .max_by_key(|(dy, n)| (**n, -dy.abs()))
        .filter(|(_, n)| **n >= MIN_MOVED)
        .map(|(dy, _)| *dy as f32);
    let Some(dy) = scroll else {
        if fractional && !votes.is_empty() {
            return None;
        }
        // No scroll: what changed is drawn again where it was and is.
        if !orders_agree(&mut orders) {
            return None;
        }
        let damage = tidy(
            changed_old
                .iter()
                .map(|&j| old[j].reach)
                .chain(changed_new.iter().map(|&i| new[i].reach))
                .collect(),
            &view,
        );
        let total: f32 = damage.iter().map(Area::area).sum();
        return (total <= MAX_DAMAGE * view.area()).then_some(Plan {
            shift: None,
            damage,
        });
    };

    // The scrolled clip: the masks of what moved by `dy`.
    let mut region: Option<Area> = None;
    for &i in &changed_new {
        let moved = by_shape.get(&shape(&new[i])).is_some_and(|js| {
            js.iter()
                .any(|&j| (new[i].bounds.y0 - old[j].bounds.y0 - dy).abs() < SAME)
        });
        if moved && !new[i].fresh {
            region = Some(region.map_or(new[i].mask, |r| r.union(&new[i].mask)));
        }
    }
    let region = region?.and(&view);
    if region.is_empty() || !region.on_grid() || dy.abs() >= region.y1 - region.y0 {
        return None;
    }
    let region = region.pixels();
    // Where the copy lands, and where it comes from.
    let to = Area {
        y0: region.y0 + dy.max(0.0),
        y1: region.y1 + dy.min(0.0),
        ..region
    };
    let from = to.moved(-dy);

    // What moved with the scroll: the same shape, `dy` further, inside the
    // clip, showing the same part of itself where the copy lands.
    let mut pool: HashMap<_, Vec<usize>> = HashMap::new();
    for &j in &changed_old {
        pool.entry((shape(&old[j]), coarse(old[j].bounds.y0 + dy)))
            .or_default()
            .push(j);
    }
    let mut moved_new = vec![false; new.len()];
    let mut moved_old = vec![false; old.len()];
    for &i in &changed_new {
        let s = &new[i];
        if s.fresh || !s.reach.within(&region) {
            continue;
        }
        let Some(candidates) = pool.get_mut(&(shape(s), coarse(s.bounds.y0))) else {
            continue;
        };
        let fits = |j: &usize| {
            let o = &old[*j];
            o.bounds.moved(dy).same(&s.bounds)
                && o.reach.within(&region)
                && o.reach.moved(dy).and(&to).same(&s.reach.and(&to))
        };
        if let Some(k) = candidates.iter().position(fits) {
            let j = candidates.remove(k);
            moved_new[i] = true;
            moved_old[j] = true;
            orders.push((old[j].order, s.order));
        }
    }
    if !orders_agree(&mut orders) {
        return None;
    }

    let mut damage = Vec::new();
    // The strip the scroll exposed.
    damage.push(if dy < 0.0 {
        Area {
            y0: region.y1 + dy,
            ..region
        }
    } else {
        Area {
            y1: region.y0 + dy,
            ..region
        }
    });
    let copied = |r: &Area| r.and(&from).moved(dy);
    // What came, went or changed: where it was, where the copy took that,
    // and where it is.
    for &j in &changed_old {
        if !moved_old[j] {
            damage.push(old[j].reach);
            damage.push(copied(&old[j].reach));
        }
    }
    for &i in &changed_new {
        if !moved_new[i] {
            damage.push(new[i].reach);
        }
    }
    // What stayed over the clip while its pixels moved under it.
    for (i, s) in new.iter().enumerate() {
        if stayed[i].is_none() || !s.reach.touches(&region) {
            continue;
        }
        let over = s.reach.and(&region);
        if s.plain {
            // One colour: only the bands the copy moved its edges through,
            // and only its edges inside the clip (past the clip's edge the
            // copy brings nothing in from outside it).
            let reach = dy.abs();
            if s.reach.y0 > region.y0 + SAME {
                let top = Area {
                    y1: (over.y0 + reach).min(over.y1),
                    ..over
                };
                damage.extend([top, copied(&top)]);
            }
            if s.reach.y1 < region.y1 - SAME {
                let bottom = Area {
                    y0: (over.y1 - reach).max(over.y0),
                    ..over
                };
                damage.extend([bottom, copied(&bottom)]);
            }
        } else {
            damage.push(over);
            damage.push(copied(&over));
        }
    }
    let damage = tidy(damage, &view);
    let total: f32 = damage.iter().map(Area::area).sum();
    (total <= MAX_DAMAGE * view.area()).then_some(Plan {
        shift: Some((from, to)),
        damage,
    })
}

// --- Drawing -----------------------------------------------------------------

/// What draws a scene for [`Partial::frame`].
pub(crate) trait Drawer {
    /// The colour a frame is cleared to.
    fn clear(&self) -> [f32; 4];
    /// What a frame needs before anything is drawn (the tile cache's and
    /// the rounded clips' frame), drawn or not.
    fn prepare(&mut self) -> Result<()>;
    /// Makes `target` the render target, cleared, with the frame's state bound.
    fn begin(&mut self, target: &Option<ID3D11RenderTargetView>) -> Result<()>;
    /// Uploads the scene's primitives.
    fn upload(&mut self) -> Result<()>;
    /// Draws the uploaded scene into the target; `false` when it could not.
    fn draw(&mut self) -> Result<bool>;
}

/// The scissor test turned on in the rasterizer state that is bound, until
/// dropped: whatever happens meanwhile, the state that was bound is back.
struct Scissor<'a> {
    context: &'a ID3D11DeviceContext,
    bound: Option<ID3D11RasterizerState>,
}

impl<'a> Scissor<'a> {
    fn on(device: &ID3D11Device, context: &'a ID3D11DeviceContext) -> Result<Scissor<'a>> {
        // SAFETY: queries on the device's thread; out-pointers to locals.
        unsafe {
            let bound = context.RSGetState().ok();
            // Direct3D's state when none is bound.
            let mut desc = D3D11_RASTERIZER_DESC {
                FillMode: D3D11_FILL_SOLID,
                CullMode: D3D11_CULL_BACK,
                DepthClipEnable: true.into(),
                ..Default::default()
            };
            if let Some(bound) = &bound {
                bound.GetDesc(&mut desc);
            }
            desc.ScissorEnable = true.into();
            let mut scissored = None;
            device.CreateRasterizerState(&desc, Some(&mut scissored))?;
            context.RSSetState(scissored.as_ref());
            Ok(Scissor { context, bound })
        }
    }
}

impl Drop for Scissor<'_> {
    fn drop(&mut self) {
        // SAFETY: the state that was bound, on the device's thread.
        unsafe { self.context.RSSetState(self.bound.as_ref()) };
    }
}

struct Canvas {
    size: (u32, u32),
    textures: [ID3D11Texture2D; 2],
    views: [Option<ID3D11RenderTargetView>; 2],
    current: usize,
    last: Vec<Sig>,
    /// What the last frame was cleared to.
    clear: [f32; 4],
}

/// A window's partial redraw.
#[derive(Default)]
pub(crate) struct Partial {
    on: bool,
    canvas: Option<Canvas>,
    /// How the last frame was made (tests).
    pub(crate) last_plan: Option<Plan>,
}

impl Partial {
    pub(crate) fn for_adapter(adapter: &IDXGIAdapter1) -> Partial {
        Partial {
            on: wanted(adapter),
            ..Default::default()
        }
    }

    #[cfg(test)]
    pub(crate) fn enabled() -> Partial {
        Partial {
            on: true,
            ..Default::default()
        }
    }

    /// Forgets the last frame (the device is new, or the frame was drawn
    /// another way).
    pub(crate) fn reset(&mut self) {
        self.canvas = None;
        self.last_plan = None;
    }

    /// Draws `scene` with `drawer` into a canvas, from the last frame where
    /// it can, and returns the canvas; `None` when the frame is the
    /// caller's to draw (off, or the scene cannot be compared).
    pub(crate) fn frame(
        &mut self,
        device: &ID3D11Device,
        context: &ID3D11DeviceContext,
        size: (u32, u32),
        scene: &Scene,
        uploaded: &HashSet<(AtlasTextureId, u32)>,
        drawer: &mut dyn Drawer,
    ) -> Result<Option<ID3D11Texture2D>> {
        self.last_plan = None;
        if !self.on {
            return Ok(None);
        }
        let dynamic = |id: AtlasTextureId| {
            id.kind == AtlasTextureKind::Polychrome
                && crate::fast::dynamic_textures::view(id).is_some()
        };
        let Some(sigs) = signatures(scene, uploaded, &dynamic) else {
            self.canvas = None;
            return Ok(None);
        };
        // `ClearView` clears the damage alone.
        let Ok(context1) = context.cast::<ID3D11DeviceContext1>() else {
            self.canvas = None;
            return Ok(None);
        };
        if self.canvas.as_ref().is_none_or(|c| c.size != size) {
            self.canvas = Some(Canvas::new(device, size)?);
        }
        let canvas = self.canvas.as_mut().context("canvas")?;
        let view = Area {
            x0: 0.0,
            y0: 0.0,
            x1: size.0 as f32,
            y1: size.1 as f32,
        };
        let clear = drawer.clear();
        // Another clear colour (the window's background appearance changed)
        // shows wherever nothing is drawn: the frame is drawn whole.
        let plan = if canvas.last.is_empty() || canvas.clear != clear {
            None
        } else {
            plan(&canvas.last, &sigs, view)
        };
        let drawn = match canvas.make(
            device,
            context,
            &context1,
            scene,
            plan.as_ref(),
            clear,
            drawer,
        ) {
            Ok(true) => true,
            Ok(false) => {
                self.canvas = None;
                return Ok(None);
            }
            Err(error) => {
                // The canvas is half drawn.
                self.canvas = None;
                return Err(error);
            }
        };
        debug_assert!(drawn);
        canvas.last = sigs;
        canvas.clear = clear;
        let texture = canvas.textures[canvas.current].clone();
        self.last_plan = plan;
        Ok(Some(texture))
    }
}

impl Canvas {
    /// Makes the next frame by `plan` (`None`: whole); `false` when the
    /// scene could not be drawn.
    #[allow(clippy::too_many_arguments)]
    fn make(
        &mut self,
        device: &ID3D11Device,
        context: &ID3D11DeviceContext,
        context1: &ID3D11DeviceContext1,
        scene: &Scene,
        plan: Option<&Plan>,
        clear: [f32; 4],
        drawer: &mut dyn Drawer,
    ) -> Result<bool> {
        drawer.prepare()?;
        let next = 1 - self.current;
        let Some(plan) = plan else {
            drawer.begin(&self.views[next])?;
            drawer.upload()?;
            self.current = next;
            return drawer.draw();
        };
        // Nothing changed: the last canvas again.
        if plan.shift.is_none() && plan.damage.is_empty() {
            return Ok(true);
        }
        drawer.begin(&self.views[next])?;
        // SAFETY: Direct3D calls on the device's thread with live objects;
        // the boxes lie within the canvases.
        unsafe {
            context.CopyResource(&self.textures[next], &self.textures[self.current]);
            if let Some((from, to)) = &plan.shift {
                let from_box = D3D11_BOX {
                    left: from.x0 as u32,
                    top: from.y0 as u32,
                    front: 0,
                    right: from.x1 as u32,
                    bottom: from.y1 as u32,
                    back: 1,
                };
                context.CopySubresourceRegion(
                    &self.textures[next],
                    0,
                    to.x0 as u32,
                    to.y0 as u32,
                    0,
                    &self.textures[self.current],
                    0,
                    Some(&from_box),
                );
            }
        }
        let rects: Vec<RECT> = plan.damage.iter().map(Area::rect).collect();
        let mut drawn = true;
        if !rects.is_empty() {
            if let Some(target) = self.views[next].as_ref() {
                // SAFETY: as above.
                unsafe { context1.ClearView(&target.cast::<ID3D11View>()?, &clear, Some(&rects)) };
            }
            let _scissor = Scissor::on(device, context)?;
            // Drawing a rounded clip uploads its content over the scene's.
            let upload_each = !scene.layers.frames.is_empty();
            drawer.upload()?;
            for (n, rect) in rects.iter().enumerate() {
                if n > 0 && upload_each {
                    drawer.upload()?;
                }
                // SAFETY: as above.
                unsafe { context.RSSetScissorRects(Some(std::slice::from_ref(rect))) };
                drawn = drawn && drawer.draw()?;
            }
        }
        self.current = next;
        Ok(drawn)
    }

    fn new(device: &ID3D11Device, (width, height): (u32, u32)) -> Result<Canvas> {
        let make = || -> Result<(ID3D11Texture2D, Option<ID3D11RenderTargetView>)> {
            let desc = D3D11_TEXTURE2D_DESC {
                Width: width,
                Height: height,
                MipLevels: 1,
                ArraySize: 1,
                Format: RENDER_TARGET_FORMAT,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: D3D11_BIND_RENDER_TARGET.0 as u32,
                CPUAccessFlags: 0,
                MiscFlags: 0,
            };
            let mut texture = None;
            let mut view = None;
            // SAFETY: a valid description; out-pointers to locals.
            unsafe {
                device.CreateTexture2D(&desc, None, Some(&mut texture))?;
                let texture = texture.context("canvas texture")?;
                device.CreateRenderTargetView(&texture, None, Some(&mut view))?;
                Ok((texture, view))
            }
        };
        let (a, a_view) = make()?;
        let (b, b_view) = make()?;
        Ok(Canvas {
            size: (width, height),
            textures: [a, b],
            views: [a_view, b_view],
            current: 0,
            last: Vec::new(),
            clear: [0.0; 4],
        })
    }
}

// --- The renderer ------------------------------------------------------------

struct RendererDrawer<'a> {
    renderer: &'a mut crate::DirectXRenderer,
    scene: &'a Scene,
    clear: [f32; 4],
    /// The window's own render target view, while a canvas's is in its place.
    window: Option<Option<ID3D11RenderTargetView>>,
}

impl RendererDrawer<'_> {
    fn depth_scale(&self) -> f32 {
        gpui::quad_depth_scale(self.scene.quads.len())
    }
}

impl Drawer for RendererDrawer<'_> {
    fn clear(&self) -> [f32; 4] {
        self.clear
    }

    fn prepare(&mut self) -> Result<()> {
        // As `render` begins: the tile cache's frame (tiles of layers that
        // are gone age out) and the rounded clips'.
        crate::fast::layers::raster::rasterize_tiles(self.renderer, self.scene)
    }

    fn begin(&mut self, target: &Option<ID3D11RenderTargetView>) -> Result<()> {
        // The renderer draws into its resources' view (paths composite
        // through it): the canvas's takes its place for the frame.
        let resources = self
            .renderer
            .resources
            .as_mut()
            .context("resources missing")?;
        let window = std::mem::replace(&mut resources.render_target_view, target.clone());
        self.window.get_or_insert(window);
        self.renderer.pre_draw(&self.clear, self.depth_scale())
    }

    fn upload(&mut self) -> Result<()> {
        self.renderer.upload_scene_buffers(self.scene)
    }

    fn draw(&mut self) -> Result<bool> {
        crate::fast::frame::draw_batches(self.renderer, self.scene)
    }
}

impl Drop for RendererDrawer<'_> {
    fn drop(&mut self) {
        if let (Some(window), Some(resources)) =
            (self.window.take(), self.renderer.resources.as_mut())
        {
            resources.render_target_view = window;
        }
    }
}

/// Draws `scene` into the window's buffer from the last frame, in place of
/// `DirectXRenderer::render`; `false` when the frame is `render`'s to draw.
pub(crate) fn draw(
    renderer: &mut crate::DirectXRenderer,
    scene: &Scene,
    background: gpui::WindowBackgroundAppearance,
) -> Result<bool> {
    // As `render` clears.
    let clear = match background {
        gpui::WindowBackgroundAppearance::Opaque => [1.0f32; 4],
        _ => [0.0f32; 4],
    };
    let mut partial = std::mem::take(&mut renderer.fast_partial);
    let drawn = (|| -> Result<bool> {
        let Some(uploaded) = renderer.atlas.take_uploads() else {
            partial.reset();
            return Ok(false);
        };
        let devices = renderer.devices.as_ref().context("devices missing")?;
        let (device, context) = (devices.device.clone(), devices.device_context.clone());
        let size = (renderer.width, renderer.height);
        let canvas = {
            let mut drawer = RendererDrawer {
                renderer,
                scene,
                clear,
                window: None,
            };
            partial.frame(&device, &context, size, scene, &uploaded, &mut drawer)?
        };
        let Some(canvas) = canvas else {
            return Ok(false);
        };
        let resources = renderer.resources.as_ref().context("resources missing")?;
        let buffer = resources
            .render_target
            .as_ref()
            .context("render target missing")?;
        // SAFETY: two live textures of the window's size and format.
        unsafe { context.CopyResource(buffer, &canvas) };
        Ok(true)
    })();
    if !matches!(drawn, Ok(true)) {
        partial.reset();
    }
    renderer.fast_partial = partial;
    drawn
}

#[cfg(test)]
mod tests {
    use super::{Area, Plan, Sig, orders_agree, plan};

    const VIEW: Area = Area {
        x0: 0.0,
        y0: 0.0,
        x1: 1000.0,
        y1: 800.0,
    };
    /// The scrolled clip of the tests.
    const CLIP: Area = Area {
        x0: 100.0,
        y0: 40.0,
        x1: 1000.0,
        y1: 800.0,
    };

    fn at(key: u64, order: u32, x: f32, y: f32, w: f32, h: f32, mask: Area) -> Sig {
        let bounds = Area {
            x0: x,
            y0: y,
            x1: x + w,
            y1: y + h,
        };
        Sig {
            key,
            order,
            bounds,
            mask,
            reach: bounds.and(&mask),
            plain: false,
            fresh: false,
        }
    }

    /// A window: a plain background, a header, and ten rows in the clip,
    /// scrolled by `offset`.
    fn frame(offset: f32) -> Vec<Sig> {
        let mut out = vec![
            Sig {
                plain: true,
                ..at(1, 1, 0.0, 0.0, 1000.0, 800.0, VIEW)
            },
            at(2, 2, 0.0, 0.0, 1000.0, 40.0, VIEW),
        ];
        for row in 0..10 {
            out.push(at(
                10 + row,
                3,
                120.0,
                60.0 + 100.0 * row as f32 + offset,
                600.0,
                80.0,
                CLIP,
            ));
        }
        out
    }

    fn total(plan: &Plan) -> f32 {
        plan.damage.iter().map(Area::area).sum()
    }

    #[test]
    fn an_unchanged_frame_needs_nothing() {
        let plan = plan(&frame(0.0), &frame(0.0), VIEW).unwrap();
        assert_eq!(
            plan,
            Plan {
                shift: None,
                damage: vec![]
            }
        );
    }

    #[test]
    fn a_scroll_copies_the_clip_and_damages_the_strip_it_exposed() {
        let plan = plan(&frame(0.0), &frame(-48.0), VIEW).unwrap();
        let (from, to) = plan.shift.unwrap();
        assert_eq!(to, Area { y1: 752.0, ..CLIP });
        assert_eq!(from, Area { y0: 88.0, ..CLIP });
        // The plain background runs past the clip: no bands of it, only
        // the strip.
        assert_eq!(plan.damage, vec![Area { y0: 752.0, ..CLIP }]);
        // The strip at the bottom, and the bands of the plain background at
        // the clip's top and bottom that the copy moved.
        assert!(plan.damage.iter().any(|r| r.y1 == 800.0 && r.y0 <= 752.0));
        assert!(total(&plan) < 0.25 * VIEW.area());
        // Scrolling back exposes the top.
        let back = super::plan(&frame(-48.0), &frame(0.0), VIEW).unwrap();
        assert_eq!(back.shift.unwrap().1, Area { y0: 88.0, ..CLIP });
    }

    #[test]
    fn a_change_without_a_scroll_damages_only_what_changed() {
        let mut new = frame(0.0);
        new[5].key = 99;
        let plan = plan(&frame(0.0), &new, VIEW).unwrap();
        assert_eq!(plan.shift, None);
        assert_eq!(plan.damage, vec![new[5].reach]);
    }

    #[test]
    fn what_does_not_move_with_the_scroll_is_damaged() {
        // A sticky bar over the clip that stays where it is.
        let bar = at(50, 9, 100.0, 40.0, 900.0, 30.0, CLIP);
        let mut old = frame(0.0);
        old.push(bar);
        let mut new = frame(-48.0);
        new.push(bar);
        let plan = plan(&old, &new, VIEW).unwrap();
        // Where it is, and where the copy took its pixels.
        let covered = |r: Area| plan.damage.iter().any(|d| r.within(d));
        assert!(covered(bar.reach));
        assert!(covered(
            Area {
                y0: 40.0,
                y1: 40.0 + 30.0,
                ..bar.reach
            }
            .moved(0.0)
        ));
    }

    #[test]
    fn a_fractional_scroll_copies_nothing() {
        // No pixel can be copied half a pixel further: what moved is drawn
        // again where it was and where it is.
        let plan = plan(&frame(0.0), &frame(-48.5), VIEW).unwrap();
        assert_eq!(plan.shift, None);
        assert!(!plan.damage.is_empty());
    }

    #[test]
    fn a_fresh_tile_is_damage_whatever_its_bytes() {
        let mut new = frame(0.0);
        new[4].fresh = true;
        let plan = plan(&frame(0.0), &new, VIEW).unwrap();
        assert_eq!(plan.damage, vec![new[4].reach]);
    }

    #[test]
    fn a_change_of_painting_order_is_drawn_whole() {
        // Two overlapping primitives that stay, their orders swapped.
        let a = at(70, 5, 200.0, 200.0, 100.0, 100.0, VIEW);
        let b = at(71, 6, 250.0, 250.0, 100.0, 100.0, VIEW);
        let old = vec![a, b];
        let new = vec![Sig { order: 6, ..a }, Sig { order: 5, ..b }];
        assert_eq!(plan(&old, &new, VIEW), None);
        // Orders renumbered alike are the same painting order.
        let renumbered = vec![Sig { order: 8, ..a }, Sig { order: 9, ..b }];
        assert!(plan(&old, &renumbered, VIEW).is_some());
        assert!(orders_agree(&mut [(1, 1), (1, 2), (2, 2)]));
        assert!(!orders_agree(&mut [(1, 3), (2, 2)]));
    }

    #[test]
    fn primitives_alike_at_one_place_keep_their_sequence() {
        // Two backgrounds, one over the other, alike in everything but order.
        let mut old = frame(0.0);
        old.insert(1, Sig { order: 2, ..old[0] });
        assert_eq!(
            plan(&old, &old, VIEW),
            Some(Plan {
                shift: None,
                damage: vec![]
            })
        );
        // And when the content between them scrolls.
        let mut new = frame(-48.0);
        new.insert(1, Sig { order: 2, ..new[0] });
        assert!(plan(&old, &new, VIEW).unwrap().shift.is_some());
    }

    #[test]
    fn a_scroll_between_pixels_is_a_scroll_whatever_the_last_bits() {
        // Rows a quarter of a pixel off the grid (a 125 % display), their
        // new places a bit of rounding off the old ones plus the scroll.
        let rows = |offset: f32, noise: u32| -> Vec<Sig> {
            let mut out = frame(0.0);
            for (n, s) in out.iter_mut().enumerate().skip(2) {
                let y = f32::from_bits(
                    (s.bounds.y0 + 0.25 + offset).to_bits() + noise * (n as u32 % 2),
                );
                let bounds = Area {
                    y0: y,
                    y1: y + 80.0,
                    ..s.bounds
                };
                *s = Sig {
                    bounds,
                    reach: bounds.and(&s.mask),
                    ..*s
                };
            }
            out
        };
        let plan = plan(&rows(0.0, 0), &rows(-48.0, 1), VIEW).unwrap();
        assert!(plan.shift.is_some());
        assert_eq!(plan.damage, vec![Area { y0: 752.0, ..CLIP }]);
    }

    #[test]
    fn a_page_of_changes_is_planned_quickly() {
        // Thousands of small changes: gathered into bands, not merged pair
        // by pair.
        let cells = |key: u64| -> Vec<Sig> {
            (0..6000u64)
                .map(|n| {
                    at(
                        key + n,
                        1,
                        (n % 100) as f32 * 10.0,
                        (n / 100) as f32 * 13.0,
                        6.0,
                        6.0,
                        VIEW,
                    )
                })
                .collect()
        };
        let started = std::time::Instant::now();
        assert_eq!(plan(&cells(0), &cells(100_000), VIEW), None);
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
    }

    #[test]
    fn a_clip_off_the_pixel_grid_is_drawn_whole() {
        let clip = Area { y0: 40.5, ..CLIP };
        let rows = |offset: f32| -> Vec<Sig> {
            (0..10)
                .map(|row| {
                    at(
                        10 + row,
                        3,
                        120.0,
                        60.0 + 100.0 * row as f32 + offset,
                        600.0,
                        80.0,
                        clip,
                    )
                })
                .collect()
        };
        assert_eq!(plan(&rows(0.0), &rows(-48.0), VIEW), None);
    }

    #[test]
    fn too_much_damage_is_drawn_whole() {
        let old = frame(0.0);
        let new: Vec<Sig> = old
            .iter()
            .map(|s| Sig {
                key: s.key + 1000,
                ..*s
            })
            .collect();
        assert_eq!(plan(&old, &new, VIEW), None);
    }
}
