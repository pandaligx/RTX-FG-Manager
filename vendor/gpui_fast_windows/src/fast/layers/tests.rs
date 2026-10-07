// Modified by Longbridge for gpui-fast.
//! Pixel tests of scroll layers on Direct3D 11: the same content drawn
//! directly and through layer tiles must come out byte for byte the same.
//!
//! [`Harness`] draws scenes without a window: it owns the objects the
//! renderer would, built by the renderer's own constructors, and draws
//! through `fast::frame` and `fast::layers::raster` as the renderer does,
//! into a texture it reads back. It runs on a hardware device, or on WARP
//! where there is none; where neither can be created the tests skip.

use std::borrow::Cow;
use std::rc::Rc;
use std::slice;

use anyhow::{Context as _, Result};
use gpui::{
    AtlasKey, AtlasTile, Bounds, ContentMask, Corners, DevicePixels, Edges, FontId, GlyphId, Hsla,
    ImageId, LayerFrame, LayerKey, MonochromeSprite, PlatformAtlas, PolychromeSprite, Quad,
    RenderGlyphParams, RenderImageParams, Rgba, ScaledPixels, Scene, SceneLayers, Shadow, Size,
    SubpixelSprite, TileCoord, TransformationMatrix, Underline, layer_tile_id,
    layer_tile_texture_id, linear_color_stop, linear_gradient, point, px, rgba, size,
};
use windows::Win32::{
    Foundation::HMODULE,
    Graphics::{
        Direct3D::{
            D3D_DRIVER_TYPE, D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP,
            D3D_FEATURE_LEVEL_10_1, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_11_1,
        },
        Direct3D11::{
            D3D11_BIND_RENDER_TARGET, D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            D3D11_CULL_NONE, D3D11_FILL_SOLID, D3D11_MAP_READ, D3D11_MAPPED_SUBRESOURCE,
            D3D11_RASTERIZER_DESC, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT,
            D3D11_USAGE_STAGING, D3D11_VIEWPORT, D3D11CreateDevice, ID3D11Device,
            ID3D11DeviceContext, ID3D11Texture2D,
        },
        Dxgi::Common::DXGI_SAMPLE_DESC,
    },
};

use crate::DirectXAtlas;
use crate::directx_renderer::{
    DirectXGlobalElements, DirectXRenderPipelines, RENDER_TARGET_FORMAT,
};
use crate::fast::frame::{FrameState, Target, draw_scene, upload};
use crate::fast::layers::TileCache;
use crate::fast::layers::raster::{clear_color, rasterize};
use crate::fast::layers::tile_cache::create_globals;

const TILE: u32 = 512;

#[test]
fn harness_renders_a_quad_at_the_right_pixels() {
    let Some(mut harness) = Harness::new() else {
        eprintln!("skipped: no Direct3D 11 device");
        return;
    };
    let mut scene = Scene::default();
    scene.insert_primitive(quad(sp(4., 4., 8., 8.), Hsla::from(rgba(0xff0000ff))));
    scene.finish();
    let pixels = harness.render(&scene, device_size(32, 32), rgba(0xffffffff));
    assert_eq!(pixel(&pixels, 32, 8, 8), [255, 0, 0, 255]);
    assert_eq!(pixel(&pixels, 32, 1, 1), [255, 255, 255, 255]);
}

#[test]
fn a_tile_is_cleared_to_the_bytes_an_opaque_quad_of_its_background_writes() {
    let Some(mut harness) = Harness::new() else {
        eprintln!("skipped: no Direct3D 11 device");
        return;
    };
    let mut scene = Scene::default();
    scene.insert_primitive(quad(sp(0., 0., 16., 16.), Hsla::from(background())));
    scene.finish();
    let direct = harness.render(&scene, device_size(16, 16), rgba(0x000000ff));
    let tiles = [(0, 0), (1, 0), (0, 1), (1, 1)];
    let assembled = rasterize_and_assemble(&mut harness, Scene::default(), &tiles);
    assert_eq!(
        pixel(&assembled, 2 * TILE as usize, 3, 3),
        pixel(&direct, 16, 3, 3)
    );
    assert_eq!(clear_color(background())[3], 1.);
}

#[test]
fn a_rasterized_tile_equals_the_same_content_drawn_directly() {
    let Some(mut harness) = Harness::new() else {
        eprintln!("skipped: no Direct3D 11 device");
        return;
    };
    let direct = harness.render(
        &content(&harness, (0., 0.), no_mask().bounds),
        device_size(1024, 1024),
        background(),
    );
    let tiles = [(0, 0), (1, 0), (0, 1), (1, 1)];
    let content = content(&harness, (0., 0.), no_mask().bounds);
    let assembled = rasterize_and_assemble(&mut harness, content, &tiles);
    assert_same_pixels(&assembled, &direct, 1024);
}

#[test]
fn tiles_of_negative_coordinates_rasterize_like_positive_ones() {
    let Some(mut harness) = Harness::new() else {
        eprintln!("skipped: no Direct3D 11 device");
        return;
    };
    let direct = harness.render(
        &content(&harness, (0., 0.), no_mask().bounds),
        device_size(1024, 1024),
        background(),
    );
    let tiles = [(-1, -1), (0, -1), (-1, 0), (0, 0)];
    let shifted = content(&harness, (-(TILE as f32), -(TILE as f32)), no_mask().bounds);
    let assembled = rasterize_and_assemble(&mut harness, shifted, &tiles);
    assert_same_pixels(&assembled, &direct, 1024);
}

#[test]
fn a_composited_layer_equals_direct_drawing() {
    let Some(mut harness) = Harness::new() else {
        eprintln!("skipped: no Direct3D 11 device");
        return;
    };
    composite_and_compare(&mut harness, (37., -91.));
}

#[test]
fn a_composited_layer_equals_direct_drawing_at_even_translations() {
    let Some(mut harness) = Harness::new() else {
        eprintln!("skipped: no Direct3D 11 device");
        return;
    };
    composite_and_compare(&mut harness, (-38., 92.));
}

/// Draws a window with a viewport over a scroll layer at `translation`, once
/// from the content directly and once from the layer's tiles, twice (the
/// second frame from the cached tiles), and compares the pixels.
fn composite_and_compare(harness: &mut Harness, translation: (f32, f32)) {
    let window = device_size(700, 500);
    let viewport = sp(40., 30., 600., 400.);

    let mut direct = Scene::default();
    direct.insert_primitive(panel());
    add_content(&mut direct, harness, translation, viewport);
    direct.insert_primitive(scrollbar());
    direct.finish();
    let expected = harness.render(&direct, window, rgba(0x000000ff));

    let key = LayerKey(5);
    let tiles = [coord(0, 0), coord(1, 0), coord(0, 1), coord(1, 1)];
    let layer = layer_frame(key, 1, content(harness, (0., 0.), no_mask().bounds), &tiles);
    let composited = composited_scene(layer, &tiles, translation, viewport);
    let actual = harness.render(&composited, window, rgba(0x000000ff));
    assert_same_pixels(&actual, &expected, 700);

    // A frame that only scrolled draws the cached tiles again.
    let actual = harness.render(&composited, window, rgba(0x000000ff));
    assert_same_pixels(&actual, &expected, 700);
}

/// A composited tile the cache does not hold, which the frame's rasterization
/// normally prevents, draws nothing rather than failing.
#[test]
fn a_composited_tile_the_cache_lacks_draws_nothing() {
    let Some(mut harness) = Harness::new() else {
        eprintln!("skipped: no Direct3D 11 device");
        return;
    };
    let window = device_size(700, 500);
    let viewport = sp(40., 30., 600., 400.);
    let mut without = Scene::default();
    without.insert_primitive(panel());
    without.insert_primitive(scrollbar());
    without.finish();
    let expected = harness.render(&without, window, rgba(0x000000ff));

    let tiles = [coord(0, 0), coord(1, 0)];
    let layer = layer_frame(LayerKey(8), 1, Scene::default(), &tiles);
    let composited = composited_scene(layer, &tiles, (0., 0.), viewport);
    let actual = harness.render_without_rasterizing(&composited, window, rgba(0x000000ff));
    assert_same_pixels(&actual, &expected, 700);
}

fn panel() -> Quad {
    quad(sp(0., 0., 700., 500.), Hsla::from(background()))
}

fn scrollbar() -> Quad {
    quad(sp(620., 40., 12., 120.), Hsla::from(rgba(0x888888ff)))
}

/// A window scene: a panel, `layer`'s `tiles` composited at `translation`
/// inside `viewport`, and a scrollbar over them.
fn composited_scene(
    layer: LayerFrame,
    tiles: &[TileCoord],
    translation: (f32, f32),
    viewport: Bounds<ScaledPixels>,
) -> Scene {
    let key = layer.key;
    let mut composited = Scene::default();
    composited.insert_primitive(panel());
    composited.push_layer(viewport);
    for &tile in tiles {
        let bounds = layer.tile_bounds(tile);
        composited.insert_primitive(PolychromeSprite {
            order: 0,
            pad: 0,
            grayscale: false.into(),
            opacity: 1.,
            bounds: sp(
                bounds.origin.x.0 + translation.0,
                bounds.origin.y.0 + translation.1,
                TILE as f32,
                TILE as f32,
            ),
            content_mask: ContentMask { bounds: viewport },
            corner_radii: Corners::default(),
            tile: AtlasTile {
                texture_id: layer_tile_texture_id(key),
                tile_id: layer_tile_id(tile),
                padding: 0,
                bounds: Bounds {
                    origin: point(DevicePixels(0), DevicePixels(0)),
                    size: device_size(TILE as i32, TILE as i32),
                },
            },
        });
    }
    composited.pop_layer();
    composited.insert_primitive(scrollbar());
    composited.layers.frames.push(layer);
    composited.finish();
    composited
}

#[test]
fn dirty_tiles_are_rasterized_once_they_are_shown() {
    let mut cache = TileCache::default();
    let key = LayerKey(3);
    let all = [coord(0, 0), coord(1, 0), coord(0, 1), coord(1, 1)];
    let mut rasterized = 0;
    let mut frame =
        |cache: &mut TileCache, generation, dirty: &[TileCoord], shown: &[TileCoord]| {
            let layers = scene_layers(key, generation, dirty);
            let planned = cache.begin_frame(&layers, shown.iter().map(|tile| (key, *tile)));
            rasterized += planned.len();
            planned
                .into_iter()
                .map(|(_, tile)| tile)
                .collect::<Vec<_>>()
        };

    let [a, b, c, _] = all;
    // A new layer: only the dirty tiles shown. The others wait until they are.
    let mut shown = vec![a, b];
    shown.sort();
    assert_eq!(frame(&mut cache, 1, &all, &[a, b]), shown);
    // The same generation again: nothing.
    assert_eq!(frame(&mut cache, 1, &all, &[a, b]), vec![]);
    // The next generation dirties a held tile that is not shown: it waits.
    assert_eq!(frame(&mut cache, 2, &[b, c], &[a]), vec![]);
    // Shown again, it is rasterized, and only it.
    assert_eq!(frame(&mut cache, 2, &[b, c], &[a, b]), vec![b]);
    // A dirty tile never shown before is rasterized once it is.
    assert_eq!(frame(&mut cache, 3, &[], &[a, b, c]), vec![c]);
    // A skipped generation may have dirtied any tile: shown tiles again.
    assert_eq!(frame(&mut cache, 5, &[], &[a]), vec![a]);
    // A shown tile the cache never had.
    assert_eq!(frame(&mut cache, 5, &[], &[coord(2, 0)]), vec![coord(2, 0)]);
    assert_eq!(rasterized, 6);
}

#[test]
fn tiles_are_evicted_least_recently_composited_first_within_the_budget() {
    const SIDE: u32 = 16;
    let tile_bytes = (SIDE * SIDE * 4) as u64;
    let key = LayerKey(4);
    let mut cache = TileCache::default();
    let layers = SceneLayers {
        frames: vec![LayerFrame {
            tile_size: SIDE,
            ..layer_frame(key, 1, Scene::default(), &[])
        }],
    };
    let frame = |cache: &mut TileCache, shown: &[TileCoord], budget: u64| {
        cache.begin_frame(&layers, shown.iter().map(|tile| (key, *tile)));
        cache.evict_to_budget(budget);
    };
    let (a, b, c, d, e) = (
        coord(0, 0),
        coord(1, 0),
        coord(2, 0),
        coord(3, 0),
        coord(4, 0),
    );

    frame(&mut cache, &[a, b, c], 3 * tile_bytes);
    frame(&mut cache, &[b, c, d], 3 * tile_bytes);
    assert!(
        !cache.holds(key, a),
        "the least recently composited tile goes"
    );
    assert!(cache.holds(key, b) && cache.holds(key, c) && cache.holds(key, d));

    frame(&mut cache, &[c], 3 * tile_bytes);
    frame(&mut cache, &[d, e], tile_bytes);
    assert!(!cache.holds(key, b) && !cache.holds(key, c));
    assert!(
        cache.holds(key, d) && cache.holds(key, e),
        "tiles composited this frame stay, over budget or not"
    );
}

#[test]
fn releasing_tiles_empties_the_cache_and_the_next_frame_rasterizes_again() {
    let Some(mut harness) = Harness::new() else {
        eprintln!("skipped: no Direct3D 11 device");
        return;
    };
    let key = LayerKey(1);
    let tiles = [(0, 0), (1, 0), (0, 1), (1, 1)];
    let content = content(&harness, (0., 0.), no_mask().bounds);
    rasterize_and_assemble(&mut harness, content, &tiles);
    assert!(harness.frame.layers.holds(key, coord(0, 0)));

    // What `fast::layers::release_tiles` does on resize, scale change and
    // device loss.
    harness.frame.layers.clear();
    assert!(harness.frame.layers.is_empty());

    let content = self::content(&harness, (0., 0.), no_mask().bounds);
    let assembled = rasterize_and_assemble(&mut harness, content, &tiles);
    let direct = harness.render(
        &self::content(&harness, (0., 0.), no_mask().bounds),
        device_size(1024, 1024),
        background(),
    );
    assert_same_pixels(&assembled, &direct, 1024);
}

// --- layer helpers --- //

fn coord(x: i32, y: i32) -> TileCoord {
    TileCoord { x, y }
}

fn background() -> Rgba {
    rgba(0x336699ff)
}

fn layer_frame(key: LayerKey, generation: u64, content: Scene, dirty: &[TileCoord]) -> LayerFrame {
    LayerFrame {
        key,
        generation,
        background: background(),
        tile_size: TILE,
        content: Rc::new(content).into(),
        dirty_tiles: dirty.to_vec(),
    }
}

fn scene_layers(key: LayerKey, generation: u64, dirty: &[TileCoord]) -> SceneLayers {
    SceneLayers {
        frames: vec![layer_frame(key, generation, Scene::default(), dirty)],
    }
}

/// Draws a frame that rasterizes `tiles` of `content` and puts the tiles
/// together into one image, `tiles[0]` at its top left.
fn rasterize_and_assemble(harness: &mut Harness, content: Scene, tiles: &[(i32, i32)]) -> Vec<u8> {
    let key = LayerKey(1);
    let coords: Vec<TileCoord> = tiles.iter().map(|&(x, y)| coord(x, y)).collect();
    // The frame composites the tiles, where they were painted: tiles are
    // rasterized once a frame shows them.
    let layer = layer_frame(key, 1, content, &coords);
    let mut frame = Scene::default();
    for &tile in &coords {
        let bounds = layer.tile_bounds(tile);
        frame.insert_primitive(PolychromeSprite {
            order: 0,
            pad: 0,
            grayscale: false.into(),
            opacity: 1.,
            bounds,
            content_mask: ContentMask { bounds },
            corner_radii: Corners::default(),
            tile: AtlasTile {
                texture_id: layer_tile_texture_id(key),
                tile_id: layer_tile_id(tile),
                padding: 0,
                bounds: Bounds {
                    origin: point(DevicePixels(0), DevicePixels(0)),
                    size: device_size(TILE as i32, TILE as i32),
                },
            },
        });
    }
    frame.layers.frames.push(layer);
    frame.finish();
    harness.render(&frame, device_size(16, 16), background());

    let (min_x, min_y) = (tiles[0].0, tiles[0].1);
    let side = TILE as usize;
    let width = 2 * side;
    let mut image = vec![0; width * width * 4];
    for tile in coords {
        let texture = harness
            .frame
            .layers
            .texture(key, tile)
            .expect("tile rasterized")
            .texture
            .clone();
        let pixels = read_back(&harness.device, &harness.context, &texture).expect("read back");
        let x0 = (tile.x - min_x) as usize * side;
        let y0 = (tile.y - min_y) as usize * side;
        for y in 0..side {
            let from = y * side * 4;
            let to = ((y0 + y) * width + x0) * 4;
            image[to..to + side * 4].copy_from_slice(&pixels[from..from + side * 4]);
        }
    }
    image
}

fn assert_same_pixels(actual: &[u8], expected: &[u8], width: usize) {
    assert_eq!(actual.len(), expected.len());
    let differing: Vec<(usize, usize)> = (0..actual.len() / 4)
        .filter(|i| actual[i * 4..i * 4 + 4] != expected[i * 4..i * 4 + 4])
        .map(|i| (i % width, i / width))
        .collect();
    if differing.is_empty() {
        return;
    }
    let (min_x, max_x) = differing
        .iter()
        .fold((usize::MAX, 0), |(lo, hi), (x, _)| (lo.min(*x), hi.max(*x)));
    let (min_y, max_y) = differing
        .iter()
        .fold((usize::MAX, 0), |(lo, hi), (_, y)| (lo.min(*y), hi.max(*y)));
    panic!(
        "{} pixels differ, within x {min_x}..={max_x}, y {min_y}..={max_y}; first at {:?}: {:?} instead of {:?}",
        differing.len(),
        differing[0],
        pixel(actual, width, differing[0].0, differing[0].1),
        pixel(expected, width, differing[0].0, differing[0].1),
    );
}

/// Content of every primitive kind a tile holds, spread over the four tiles
/// of (0, 0)..(1024, 1024) and across their edges, moved by `offset` and
/// clipped to `clip`.
fn content(harness: &Harness, offset: (f32, f32), clip: Bounds<ScaledPixels>) -> Scene {
    let mut scene = Scene::default();
    add_content(&mut scene, harness, offset, clip);
    scene.finish();
    scene
}

/// Inserts [`content`]'s primitives into `scene`.
fn add_content(
    scene: &mut Scene,
    harness: &Harness,
    offset: (f32, f32),
    clip: Bounds<ScaledPixels>,
) {
    let (ox, oy) = offset;
    let at = |x: f32, y: f32, w: f32, h: f32| sp(x + ox, y + oy, w, h);
    let mask = |bounds: Bounds<ScaledPixels>| ContentMask {
        bounds: bounds.intersect(&clip),
    };

    scene.insert_primitive(Quad {
        content_mask: mask(clip),
        ..quad(at(100., 100., 200., 150.), Hsla::from(rgba(0xcc3322ff)))
    });
    // Rounded corners and a border, across the edge of tiles (0, 0) and (1, 0).
    scene.insert_primitive(Quad {
        bounds: at(450.5, 60., 150., 120.),
        content_mask: mask(at(0., 0., 1024., 1024.)),
        background: Hsla::from(rgba(0xeeeeeeff)).into(),
        border_color: Hsla::from(rgba(0x222222ff)),
        corner_radii: Corners::all(ScaledPixels(12.)),
        border_widths: Edges::all(ScaledPixels(3.)),
        ..Default::default()
    });
    // The shadow's tail crosses the tile edge its bounds stop short of.
    scene.insert_primitive(Shadow {
        order: 0,
        blur_radius: ScaledPixels(8.),
        bounds: at(380., 420., 120., 70.),
        corner_radii: Corners::all(ScaledPixels(6.)),
        content_mask: mask(clip),
        color: Hsla::from(rgba(0x00000080)),
        element_bounds: at(380., 420., 120., 70.),
        element_corner_radii: Corners::all(ScaledPixels(6.)),
        inset: 0,
        pad: 0,
    });
    scene.insert_primitive(Quad {
        bounds: at(600., 600., 300., 200.),
        content_mask: mask(clip),
        background: linear_gradient(
            45.,
            linear_color_stop(rgba(0xff0000ff), 0.),
            linear_color_stop(rgba(0x0000ffff), 1.),
        ),
        corner_radii: Corners::all(ScaledPixels(20.)),
        ..Default::default()
    });
    scene.insert_primitive(Underline {
        order: 0,
        pad: 0,
        bounds: at(50., 505., 900., 8.),
        content_mask: mask(clip),
        color: Hsla::from(rgba(0xffcc00ff)),
        thickness: ScaledPixels(2.),
        wavy: true.into(),
    });
    scene.insert_primitive(Underline {
        order: 0,
        pad: 0,
        bounds: at(200., 530., 400., 2.),
        content_mask: mask(clip),
        color: Hsla::from(rgba(0x11aa88ff)),
        thickness: ScaledPixels(1.),
        wavy: false.into(),
    });
    // A clipped quad whose mask crosses a tile edge.
    scene.insert_primitive(Quad {
        bounds: at(700., 200., 200., 200.),
        content_mask: mask(at(490., 250., 300., 100.)),
        background: Hsla::from(rgba(0x44aa44ff)).into(),
        ..Default::default()
    });

    // Glyphs across the vertical and the horizontal tile edge.
    let mono = glyph_tile(harness, 1, false);
    scene.insert_primitive(MonochromeSprite {
        order: 0,
        pad: 0,
        bounds: at(505., 700., 16., 16.),
        content_mask: mask(clip),
        color: Hsla::from(rgba(0xffffffff)),
        tile: mono,
        transformation: TransformationMatrix::unit(),
    });
    let subpixel = glyph_tile(harness, 2, true);
    scene.insert_primitive(SubpixelSprite {
        order: 0,
        pad: 0,
        bounds: at(520., 505., 16., 16.),
        content_mask: mask(clip),
        color: Hsla::from(rgba(0x000000ff)),
        tile: subpixel,
        transformation: TransformationMatrix::unit(),
    });
    scene.insert_primitive(PolychromeSprite {
        order: 0,
        pad: 0,
        grayscale: false.into(),
        opacity: 1.,
        bounds: at(300., 500., 20., 20.),
        content_mask: mask(clip),
        corner_radii: Corners::all(ScaledPixels(4.)),
        tile: image_tile(harness),
    });
}

/// A 16×16 glyph of a made-up font, uploaded to the harness's atlas: a
/// monochrome one, or a subpixel one.
fn glyph_tile(harness: &Harness, glyph: u32, subpixel: bool) -> AtlasTile {
    let key = AtlasKey::Glyph(RenderGlyphParams {
        font_id: FontId(9_999),
        glyph_id: GlyphId(glyph),
        font_size: px(12.),
        subpixel_variant: point(0, 0),
        scale_factor: 1.,
        is_emoji: false,
        subpixel_rendering: subpixel,
        dilation: 0,
    });
    let bytes_per_pixel = if subpixel { 4 } else { 1 };
    let bytes: Vec<u8> = (0..16 * 16 * bytes_per_pixel)
        .map(|i| ((i * 37) % 256) as u8)
        .collect();
    harness
        .atlas
        .get_or_insert_with(key, &mut || {
            Ok(Some((device_size(16, 16), Cow::Owned(bytes.clone()))))
        })
        .expect("glyph uploaded")
        .expect("glyph tile")
}

/// A 20×20 image uploaded to the harness's atlas.
fn image_tile(harness: &Harness) -> AtlasTile {
    let key = AtlasKey::Image(RenderImageParams {
        image_id: ImageId(9_999),
        frame_index: 0,
    });
    let bytes: Vec<u8> = (0..20 * 20)
        .flat_map(|i: u32| {
            [
                (i * 7 % 256) as u8,
                (i * 13 % 256) as u8,
                (i * 3 % 256) as u8,
                255,
            ]
        })
        .collect();
    harness
        .atlas
        .get_or_insert_with(key, &mut || {
            Ok(Some((device_size(20, 20), Cow::Owned(bytes.clone()))))
        })
        .expect("image uploaded")
        .expect("image tile")
}

// --- helpers --- //

fn sp(x: f32, y: f32, w: f32, h: f32) -> Bounds<ScaledPixels> {
    Bounds {
        origin: point(ScaledPixels(x), ScaledPixels(y)),
        size: size(ScaledPixels(w), ScaledPixels(h)),
    }
}

/// A content mask that clips nothing the tests draw.
fn no_mask() -> ContentMask<ScaledPixels> {
    ContentMask {
        bounds: sp(-10_000., -10_000., 20_000., 20_000.),
    }
}

fn quad(bounds: Bounds<ScaledPixels>, color: Hsla) -> Quad {
    Quad {
        bounds,
        content_mask: no_mask(),
        background: color.into(),
        ..Default::default()
    }
}

fn device_size(width: i32, height: i32) -> Size<DevicePixels> {
    size(DevicePixels(width), DevicePixels(height))
}

/// The RGBA bytes of the pixel at (`x`, `y`) of a `width`-wide readback.
fn pixel(pixels: &[u8], width: usize, x: usize, y: usize) -> [u8; 4] {
    let at = (y * width + x) * 4;
    [pixels[at], pixels[at + 1], pixels[at + 2], pixels[at + 3]]
}

/// Draws scenes into textures without a window, through the renderer's
/// pipelines and `fast` drawing.
struct Harness {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    atlas: DirectXAtlas,
    globals: DirectXGlobalElements,
    pipelines: DirectXRenderPipelines,
    frame: FrameState,
}

impl Harness {
    /// A harness on a hardware device, or on WARP where there is none, or
    /// `None` when neither can be created (the tests then skip).
    fn new() -> Option<Harness> {
        let (device, context) = [D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP]
            .into_iter()
            .find_map(create_device)?;
        let pipelines = DirectXRenderPipelines::new(&device)
            .inspect_err(|error| eprintln!("no pipelines: {error:#}"))
            .ok()?;
        let globals = DirectXGlobalElements::new(&device).ok()?;
        set_rasterizer_state(&device, &context).ok()?;
        let atlas = DirectXAtlas::new(&device, &context);
        Some(Harness {
            device,
            context,
            atlas,
            globals,
            pipelines,
            frame: FrameState::default(),
        })
    }

    /// Draws `scene` into a `size` texture cleared to `clear`, as the
    /// renderer draws a frame (rasterizing the layer tiles it needs first),
    /// and returns its pixels as RGBA bytes, row by row.
    fn render(&mut self, scene: &Scene, size: Size<DevicePixels>, clear: Rgba) -> Vec<u8> {
        self.draw(scene, size, clear, true).expect("frame drawn")
    }

    /// [`Self::render`] without rasterizing any layer tile.
    fn render_without_rasterizing(
        &mut self,
        scene: &Scene,
        size: Size<DevicePixels>,
        clear: Rgba,
    ) -> Vec<u8> {
        self.draw(scene, size, clear, false).expect("frame drawn")
    }

    fn draw(
        &mut self,
        scene: &Scene,
        size: Size<DevicePixels>,
        clear: Rgba,
        rasterize_tiles: bool,
    ) -> Result<Vec<u8>> {
        let target = Target {
            device: &self.device,
            device_context: &self.context,
            atlas: &self.atlas,
            globals: &self.globals,
            resources: None,
        };
        if rasterize_tiles {
            rasterize(&target, &mut self.pipelines, &mut self.frame, scene)?;
        }

        let (width, height) = (size.width.0 as u32, size.height.0 as u32);
        let texture = create_target_texture(&self.device, width, height)?;
        let mut render_target = None;
        unsafe {
            self.device
                .CreateRenderTargetView(&texture, None, Some(&mut render_target))?
        };
        let globals = create_globals(&self.device, width, height)?;
        // What `DirectXRenderer::pre_draw` binds.
        let context = &self.context;
        unsafe {
            context.ClearRenderTargetView(
                render_target.as_ref().context("render target view")?,
                &[clear.r, clear.g, clear.b, clear.a],
            );
            context.OMSetRenderTargets(Some(slice::from_ref(&render_target)), None);
            context.RSSetViewports(Some(&[D3D11_VIEWPORT {
                TopLeftX: 0.,
                TopLeftY: 0.,
                Width: width as f32,
                Height: height as f32,
                MinDepth: 0.,
                MaxDepth: 1.,
            }]));
            context.VSSetConstantBuffers(0, Some(slice::from_ref(&globals)));
            context
                .VSSetConstantBuffers(1, Some(slice::from_ref(&self.globals.batch_params_buffer)));
            context.PSSetConstantBuffers(0, Some(slice::from_ref(&globals)));
        }
        upload(&target, &mut self.pipelines, scene)?;
        draw_scene(&target, &mut self.pipelines, &mut self.frame, scene)?;
        unsafe { context.OMSetRenderTargets(None, None) };
        read_back(&self.device, context, &texture)
    }
}

fn create_device(driver_type: D3D_DRIVER_TYPE) -> Option<(ID3D11Device, ID3D11DeviceContext)> {
    let mut device = None;
    let mut context = None;
    unsafe {
        D3D11CreateDevice(
            None,
            driver_type,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            Some(&[
                D3D_FEATURE_LEVEL_11_1,
                D3D_FEATURE_LEVEL_11_0,
                D3D_FEATURE_LEVEL_10_1,
            ]),
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        )
    }
    .ok()?;
    Some((device?, context?))
}

/// The rasterizer state `DirectXResources::new` sets.
fn set_rasterizer_state(device: &ID3D11Device, context: &ID3D11DeviceContext) -> Result<()> {
    let desc = D3D11_RASTERIZER_DESC {
        FillMode: D3D11_FILL_SOLID,
        CullMode: D3D11_CULL_NONE,
        FrontCounterClockwise: false.into(),
        DepthBias: 0,
        DepthBiasClamp: 0.0,
        SlopeScaledDepthBias: 0.0,
        DepthClipEnable: true.into(),
        ScissorEnable: false.into(),
        MultisampleEnable: true.into(),
        AntialiasedLineEnable: false.into(),
    };
    let mut state = None;
    unsafe {
        device.CreateRasterizerState(&desc, Some(&mut state))?;
        context.RSSetState(state.as_ref());
    }
    Ok(())
}

fn create_target_texture(
    device: &ID3D11Device,
    width: u32,
    height: u32,
) -> Result<ID3D11Texture2D> {
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
    unsafe { device.CreateTexture2D(&desc, None, Some(&mut texture))? };
    texture.context("creating the harness's target")
}

/// The pixels of `texture`, a BGRA texture, as RGBA bytes, row by row: copied
/// into a staging texture and mapped, as `DirectXRenderer::render_to_image`
/// reads the window back.
fn read_back(
    device: &ID3D11Device,
    context: &ID3D11DeviceContext,
    texture: &ID3D11Texture2D,
) -> Result<Vec<u8>> {
    let mut desc = D3D11_TEXTURE2D_DESC::default();
    unsafe { texture.GetDesc(&mut desc) };
    let (width, height) = (desc.Width as usize, desc.Height as usize);
    let staging_desc = D3D11_TEXTURE2D_DESC {
        Usage: D3D11_USAGE_STAGING,
        BindFlags: 0,
        CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
        MiscFlags: 0,
        ..desc
    };
    let mut staging = None;
    unsafe { device.CreateTexture2D(&staging_desc, None, Some(&mut staging))? };
    let staging: ID3D11Texture2D = staging.context("creating a staging texture")?;
    let mut pixels = vec![0u8; width * height * 4];
    unsafe {
        context.CopyResource(&staging, texture);
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        context.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;
        // SAFETY: the mapping holds `RowPitch * height` bytes, `RowPitch` is
        // at least a row's `width * 4`, and `pixels` is a fresh allocation.
        let source = mapped.pData as *const u8;
        for row in 0..height {
            std::ptr::copy_nonoverlapping(
                source.add(row * mapped.RowPitch as usize),
                pixels.as_mut_ptr().add(row * width * 4),
                width * 4,
            );
        }
        context.Unmap(&staging, 0);
    }
    for pixel in pixels.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    Ok(pixels)
}
