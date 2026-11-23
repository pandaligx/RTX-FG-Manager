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
    ImageId, LayerFrame, LayerKey, MonochromeSprite, PaintedPath, PathBuilder, PlatformAtlas,
    PolychromeSprite, Quad, RenderGlyphParams, RenderImageParams, Rgba, ScaledPixels, Scene,
    SceneLayers, Shadow, Size, SubpixelSprite, TileCoord, TransformationMatrix, Underline,
    layer_tile_id, layer_tile_texture_id, linear_color_stop, linear_gradient, point, px, rgba,
    size,
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

#[cfg(feature = "test-support")]
#[path = "../rounded_clip_fixtures.rs"]
mod rounded_clip_tests;

#[cfg(feature = "test-support")]
#[gpui::test]
fn rounded_overflow_retained_child_follows_ancestor_clip(cx: &mut gpui::TestAppContext) {
    let mut harness = Harness::new().expect("retained clipping regression requires a GPU");
    rounded_clip_tests::retained_frames(cx, |scene, size| {
        harness.render(scene, size, rgba(0xffffffff))
    });
}

#[cfg(feature = "test-support")]
fn render_div_fixture(cx: &mut gpui::TestAppContext, build: fn() -> gpui::Div) -> (Vec<u8>, usize) {
    let mut harness =
        Harness::new().expect("rounded clipping regression requires Direct3D or WARP");
    rounded_clip_tests::render_with(cx, build, |scene, size| {
        harness.render(scene, size, rgba(0xffffffff))
    })
}

#[cfg(feature = "test-support")]
#[gpui::test]
fn rounded_overflow_inside_scroll_tiles_matches_direct_pixels(cx: &mut gpui::TestAppContext) {
    let mut harness =
        Harness::new().expect("rounded clipping regression requires Direct3D or WARP");
    rounded_clip_tests::render_with(cx, rounded_clip_tests::square_fixture, |scene, size| {
        let direct = harness.render(scene, size, rgba(0xffffffff));
        let tiled = rounded_clip_tests::tiled_scene(scene, size);
        let actual = harness.render(&tiled, size, rgba(0xffffffff));
        assert_same_pixels(&actual, &direct, size.width.0 as usize);
        actual
    });
}

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
    composite_and_compare(&mut harness, (37., -91.), false);
}

#[test]
fn a_composited_layer_equals_direct_drawing_at_even_translations() {
    let Some(mut harness) = Harness::new() else {
        eprintln!("skipped: no Direct3D 11 device");
        return;
    };
    composite_and_compare(&mut harness, (-38., 92.), false);
}

/// Paths something draws over stay out of the tiles: the core draws them,
/// and what it draws over them, over the composited tiles
/// (`gpui::fast::layers::overlay`), as without a layer. That holds at odd
/// translations too, as the paths are rasterized where they show.
#[test]
fn covered_paths_drawn_over_the_tiles_equal_direct_drawing() {
    let Some(mut harness) = Harness::new() else {
        eprintln!("skipped: no Direct3D 11 device");
        return;
    };
    for translation in [(37., -91.), (-38., 92.), (3., 5.), (101., -233.)] {
        composite_and_compare(&mut harness, translation, true);
    }
}

/// Draws a window with a viewport over a scroll layer at `translation`, once
/// from the content directly and once from the layer's tiles, twice (the
/// second frame from the cached tiles), and compares the pixels. With
/// `covered`, [`add_covered_paths`] is drawn after the content, over the
/// tiles.
fn composite_and_compare(harness: &mut Harness, translation: (f32, f32), covered: bool) {
    let window = device_size(700, 500);
    let viewport = sp(40., 30., 600., 400.);

    let mut direct = Scene::default();
    direct.insert_primitive(panel());
    add_content(&mut direct, harness, translation, viewport);
    if covered {
        add_covered_paths(&mut direct, harness, translation, viewport);
    }
    direct.insert_primitive(scrollbar());
    direct.finish();
    let expected = harness.render(&direct, window, rgba(0x000000ff));

    let key = LayerKey(5);
    let tiles = [coord(0, 0), coord(1, 0), coord(0, 1), coord(1, 1)];
    let layer = layer_frame(key, 1, content(harness, (0., 0.), no_mask().bounds), &tiles);
    let overlay = |scene: &mut Scene| {
        if covered {
            add_covered_paths(scene, harness, translation, viewport);
        }
    };
    let composited = composited_scene(layer, &tiles, translation, viewport, &overlay);
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
    let composited = composited_scene(layer, &tiles, (0., 0.), viewport, &|_| {});
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
/// inside `viewport`, what `overlay` draws over them, and a scrollbar over
/// them.
fn composited_scene(
    layer: LayerFrame,
    tiles: &[TileCoord],
    translation: (f32, f32),
    viewport: Bounds<ScaledPixels>,
    overlay: &dyn Fn(&mut Scene),
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
    overlay(&mut composited);
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
        shape_bounds: at(380., 420., 120., 70.),
        shape_corner_radii: Corners::all(ScaledPixels(6.)),
        inset: false.into(),
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
        animation: Default::default(),
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

/// What GPUI Kit draws for a table's rounded frame: paths filling corner
/// notches, a bordered quad and a glyph drawn over them, and a path nothing
/// covers, moved by `offset` and clipped to `clip`, in drawing order.
fn add_covered_paths(
    scene: &mut Scene,
    harness: &Harness,
    (ox, oy): (f32, f32),
    clip: Bounds<ScaledPixels>,
) {
    let mask = ContentMask { bounds: clip };
    for (x, y, dx, dy) in [
        (200., 330., 1., 1.),
        (330.5, 330., -1., 1.),
        (200., 410.25, 1., -1.),
    ] {
        let corner = point(px(x + ox), px(y + oy));
        let mut path = PathBuilder::fill();
        path.move_to(corner);
        path.line_to(corner + point(px(9. * dx), px(0.)));
        path.curve_to(corner + point(px(0.), px(9. * dy)), corner);
        path.line_to(corner);
        scene.insert_primitive(PaintedPath::new(
            path.build().expect("path"),
            1.,
            ContentMask {
                bounds: clip.map(|c| px(c.0)),
            },
            Hsla::from(rgba(0x3366ccff)).into(),
        ));
    }
    scene.insert_primitive(Quad {
        bounds: sp(200. + ox, 330. + oy, 130.5, 80.25),
        content_mask: mask,
        background: Hsla::from(rgba(0x00000000)).into(),
        border_color: Hsla::from(rgba(0x222222ff)),
        corner_radii: Corners::all(ScaledPixels(9.)),
        border_widths: Edges::all(ScaledPixels(1.)),
        ..Default::default()
    });
    scene.insert_primitive(MonochromeSprite {
        order: 0,
        animation: Default::default(),
        bounds: sp(204. + ox, 334. + oy, 16., 16.),
        content_mask: mask,
        color: Hsla::from(rgba(0xff8800ff)),
        tile: glyph_tile(harness, 1, false),
        transformation: TransformationMatrix::unit(),
    });
    let mut path = PathBuilder::fill();
    path.move_to(point(px(250. + ox), px(360. + oy)));
    path.line_to(point(px(290.5 + ox), px(372. + oy)));
    path.curve_to(
        point(px(255. + ox), px(400. + oy)),
        point(px(300. + ox), px(395. + oy)),
    );
    path.line_to(point(px(250. + ox), px(360. + oy)));
    scene.insert_primitive(PaintedPath::new(
        path.build().expect("path"),
        1.,
        ContentMask {
            bounds: clip.map(|c| px(c.0)),
        },
        Hsla::from(rgba(0x8800ffcc)).into(),
    ));
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
        if rasterize_tiles {
            let tiles = Target {
                device: &self.device,
                device_context: &self.context,
                atlas: &self.atlas,
                globals: &self.globals,
                depth_scale: None,
            };
            rasterize(&tiles, &mut self.pipelines, &mut self.frame, scene)?;
        }

        let (width, height) = (size.width.0 as u32, size.height.0 as u32);
        let texture = create_target_texture(&self.device, width, height)?;
        let mut render_target = None;
        unsafe {
            self.device
                .CreateRenderTargetView(&texture, None, Some(&mut render_target))?
        };
        let target = Target {
            device: &self.device,
            device_context: &self.context,
            atlas: &self.atlas,
            globals: &self.globals,
            depth_scale: None,
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

// Core test-support comes from the target dev-dependency. This test does not
// enable this crate's feature-gated cross-workspace rounded-clip fixture suite.
#[gpui::test]
fn composition_whole_scene_matches_replayed_surface_pixels(cx: &mut gpui::TestAppContext) {
    use crate::fast::composition_scene::whole_scene_layer;
    struct View;
    impl gpui::Render for View {
        fn render(
            &mut self,
            _: &mut gpui::Window,
            _: &mut gpui::Context<Self>,
        ) -> impl gpui::IntoElement {
            use gpui::{ParentElement as _, Styled as _};
            gpui::div().size(gpui::px(64.)).bg(gpui::white()).child(
                gpui::div()
                    .size(gpui::px(64.))
                    .rounded(gpui::px(24.))
                    .overflow_hidden()
                    .child(gpui::div().size(gpui::px(64.)).bg(gpui::black())),
            )
        }
    }
    use gpui::AppContext as _;
    let window = cx.update(|cx| {
        cx.open_window(Default::default(), |_, cx| cx.new(|_| View))
            .unwrap()
    });
    let mut harness = Harness::new().expect("composition regression requires Direct3D or WARP");
    window
        .update(cx, |_, window, cx| {
            window.draw(cx).clear(cx);
            let scene = window.painted_scene();
            assert!(
                !scene.layers.frames.is_empty(),
                "rounded clips were compiled"
            );
            let full = gpui::ComposedSceneLayer {
                surface: gpui::CompositionSurfaceId::default(),
                ranges: vec![0..scene.len()],
            };
            let empty = gpui::ComposedSceneLayer {
                ranges: vec![],
                ..full
            };
            for (layers, expected_target) in [
                (vec![full.clone(), empty.clone()], 0),
                (vec![empty.clone(), full.clone()], 1),
                (vec![full.clone()], 0),
                (vec![empty.clone(), full, empty], 1),
            ] {
                let (target_index, borrowed) = whole_scene_layer(scene, &layers).unwrap();
                assert_eq!(target_index, expected_target);
                assert!(std::ptr::eq(borrowed, scene));
                // The former per-surface path: replay the target's ranges
                // into a fresh scene and finish, independent of selection.
                let mut replayed = Scene::default();
                for range in &layers[expected_target].ranges {
                    replayed.replay_balanced(range.clone(), scene).unwrap();
                }
                replayed.finish();
                let side = (64. * window.scale_factor()) as i32;
                let extent = size(DevicePixels(side), DevicePixels(side));
                let expected = harness.render(&replayed, extent, rgba(0xffffffff));
                let actual = harness.render(borrowed, extent, rgba(0xffffffff));
                assert_eq!(actual, expected);
                assert_eq!(pixel(&actual, side as usize, 1, 1), [255, 255, 255, 255]);
                assert_eq!(
                    pixel(&actual, side as usize, side as usize / 2, side as usize / 2),
                    [0, 0, 0, 255]
                );
            }
        })
        .unwrap();
}

// --- Overdraw culling (`fast::occlusion`) --- //

/// Draws `scene` with culling off and on and checks that culling changed no
/// byte; returns whether culling hid anything.
fn culling_keeps_pixels(harness: &mut Harness, scene: &Scene) -> bool {
    let size = device_size(1024, 1024);
    harness.frame.skip_occlusion = true;
    let everything = harness.render(scene, size, rgba(0xffffffff));
    harness.frame.skip_occlusion = false;
    let culled = harness.render(scene, size, rgba(0xffffffff));
    assert_same_pixels(&culled, &everything, 1024);
    let occlusion = crate::fast::occlusion::Occlusion::of(scene, |sprite| {
        harness.atlas.is_opaque(&sprite.tile)
    });
    scene.quads.iter().any(|q| occlusion.hides_quad(q))
        || scene.shadows.iter().any(|s| occlusion.hides_shadow(s))
        || scene
            .polychrome_sprites
            .iter()
            .any(|s| occlusion.hides_sprite(s))
}

/// Every kind of primitive (`add_content`) over nested plain backgrounds
/// (a window, a content area under a top bar, a translucent page), then
/// `cover` over them.
fn covered_scene(harness: &Harness, cover: Option<Quad>) -> Scene {
    let mut scene = Scene::default();
    scene.insert_primitive(quad(sp(0., 0., 1024., 1024.), Hsla::from(rgba(0x101010ff))));
    scene.insert_primitive(quad(
        sp(0., 40.5, 1024., 983.5),
        Hsla::from(rgba(0x202830ff)),
    ));
    scene.insert_primitive(quad(
        sp(176., 40.5, 848., 983.5),
        Hsla::from(rgba(0x30405080)),
    ));
    add_content(&mut scene, harness, (0., 0.), sp(0., 0., 1024., 1024.));
    if let Some(cover) = cover {
        scene.insert_primitive(cover);
    }
    scene.finish();
    scene
}

#[test]
fn culling_nested_backgrounds_changes_no_pixel() {
    let Some(mut harness) = Harness::new() else {
        eprintln!("skipped: no Direct3D 11 device");
        return;
    };
    let scene = covered_scene(&harness, None);
    // Partly covered backgrounds are no longer cut into pieces: upstream's
    // opaque depth pre-pass already shades each covered pixel once, so
    // culling only has to leave every pixel unchanged here.
    culling_keeps_pixels(&mut harness, &scene);
}

#[test]
fn culling_under_covers_changes_no_pixel() {
    let Some(mut harness) = Harness::new() else {
        eprintln!("skipped: no Direct3D 11 device");
        return;
    };
    let opaque = Hsla::from(rgba(0x445566ff));
    let cover = |bounds| quad(bounds, opaque);
    let cases: Vec<(&str, Quad)> = vec![
        ("opaque", cover(sp(80., 80., 560., 480.))),
        ("fractional", cover(sp(80.5, 79.25, 560.25, 480.75))),
        (
            "translucent",
            quad(sp(80., 80., 560., 480.), Hsla::from(rgba(0x44556680))),
        ),
        (
            "rounded",
            Quad {
                corner_radii: Corners::all(ScaledPixels(16.)),
                ..cover(sp(80., 80., 560., 480.))
            },
        ),
        (
            "masked",
            Quad {
                content_mask: ContentMask {
                    bounds: sp(80., 80., 300., 200.),
                },
                ..cover(sp(0., 0., 1024., 1024.))
            },
        ),
        (
            "translucent border",
            Quad {
                border_widths: Edges::all(ScaledPixels(4.)),
                border_color: Hsla::from(rgba(0xffffff80)),
                ..cover(sp(80., 80., 560., 480.))
            },
        ),
        (
            "opaque border",
            Quad {
                border_widths: Edges::all(ScaledPixels(4.)),
                border_color: Hsla::from(rgba(0xffffffff)),
                ..cover(sp(80., 80., 560., 480.))
            },
        ),
        // The shadow at (380, 420) 120x70 blurs 8 px: its tail reaches 24 px out.
        (
            "opaque border, fractional",
            Quad {
                border_widths: Edges::all(ScaledPixels(4.)),
                border_color: Hsla::from(rgba(0xffffffff)),
                ..cover(sp(80.5, 79.25, 560.25, 480.75))
            },
        ),
        ("shadow body only", cover(sp(380., 420., 120., 70.))),
        ("shadow and tail", cover(sp(350., 390., 180., 130.))),
        ("everything", cover(sp(0., 0., 1024., 1024.))),
    ];
    for (name, cover) in cases {
        let scene = covered_scene(&harness, Some(cover));
        eprintln!("cover: {name}");
        culling_keeps_pixels(&mut harness, &scene);
    }
    // A full cover hides everything under it.
    let scene = covered_scene(&harness, Some(cover(sp(0., 0., 1024., 1024.))));
    let occlusion = crate::fast::occlusion::Occlusion::of(&scene, |sprite| {
        harness.atlas.is_opaque(&sprite.tile)
    });
    let last = scene.quads.iter().map(|q| q.order).max().unwrap();
    assert!(
        scene
            .quads
            .iter()
            .filter(|q| q.order < last)
            .all(|q| occlusion.hides_quad(q))
    );
    assert!(scene.shadows.iter().all(|s| occlusion.hides_shadow(s)));
    assert!(!scene.monochrome_sprites.is_empty() && !scene.subpixel_sprites.is_empty());
    assert!(
        scene
            .monochrome_sprites
            .iter()
            .all(|s| occlusion.hides_glyph(s.order, &s.bounds, &s.content_mask, &s.transformation))
    );
    assert!(scene.subpixel_sprites.iter().all(|s| occlusion.hides_glyph(
        s.order,
        &s.bounds,
        &s.content_mask,
        &s.transformation
    )));
}

/// A 20x20 image, opaque or with one translucent pixel.
fn image_cover_tile(harness: &Harness, id: usize, opaque: bool) -> AtlasTile {
    let key = AtlasKey::Image(RenderImageParams {
        image_id: ImageId(id),
        frame_index: 0,
    });
    let bytes: Vec<u8> = (0..20 * 20)
        .flat_map(|i: u32| {
            let alpha = if !opaque && i == 210 { 254 } else { 255 };
            [
                (i * 5 % 256) as u8,
                (i * 11 % 256) as u8,
                (i * 17 % 256) as u8,
                alpha,
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

fn image_cover(tile: AtlasTile, bounds: Bounds<ScaledPixels>, opacity: f32) -> PolychromeSprite {
    PolychromeSprite {
        order: 0,
        pad: 0,
        grayscale: false.into(),
        opacity,
        bounds,
        content_mask: no_mask(),
        corner_radii: Corners::default(),
        tile,
    }
}

#[test]
fn culling_under_images_changes_no_pixel() {
    let Some(mut harness) = Harness::new() else {
        eprintln!("skipped: no Direct3D 11 device");
        return;
    };
    assert!(
        harness
            .atlas
            .is_opaque(&image_cover_tile(&harness, 9_001, true))
    );
    assert!(
        !harness
            .atlas
            .is_opaque(&image_cover_tile(&harness, 9_002, false))
    );
    // A video frame over the player's background: magnified, at fractional
    // bounds, so that bilinear sampling reaches the image's edges.
    let bounds = sp(60.5, 70.25, 700.75, 520.5);
    for (name, id, opaque, opacity, hides) in [
        ("opaque image", 9_001, true, 1.0, true),
        ("translucent pixel", 9_002, false, 1.0, false),
        ("drawn translucent", 9_001, true, 0.99, false),
    ] {
        eprintln!("image cover: {name}");
        let tile = image_cover_tile(&harness, id, opaque);
        let mut scene = Scene::default();
        scene.insert_primitive(quad(sp(0., 0., 1024., 1024.), Hsla::from(rgba(0x000000ff))));
        add_content(&mut scene, &harness, (0., 0.), sp(0., 0., 1024., 1024.));
        scene.insert_primitive(image_cover(tile, bounds, opacity));
        scene.finish();
        culling_keeps_pixels(&mut harness, &scene);
        let occlusion = crate::fast::occlusion::Occlusion::of(&scene, |sprite| {
            harness.atlas.is_opaque(&sprite.tile)
        });
        // What is under the image's middle, away from the content's own
        // covers, is hidden only when the image hides.
        let mut under = quad(sp(400., 300., 20., 20.), Hsla::from(rgba(0x000000ff)));
        under.order = scene.quads[0].order;
        assert_eq!(occlusion.hides_quad(&under), hides, "{name}");
    }
}

/// A primitive whose mask's right (bottom) edge falls on a pixel centre
/// exactly where a cover's right (bottom) edge does: the clip keeps that
/// column (a centre on the clip plane is kept), the cover does not (the
/// rasterizer's top-left rule drops a centre on its right edge). The hidden
/// quad is plain (drawn through `Occlusion::pieces`) or bordered (tested by
/// `Occlusion::hides_quad`); neither may be culled at that column.
#[test]
fn culling_at_a_shared_edge_on_a_pixel_centre_changes_no_pixel() {
    let Some(mut harness) = Harness::new() else {
        eprintln!("skipped: no Direct3D 11 device");
        return;
    };
    // The hidden quad's bounds reach past the cover only where its mask
    // cuts them, at the cover's edge.
    for (name, hidden_bounds, hidden_mask, cover_bounds) in [
        (
            "right",
            sp(100., 100., 600., 400.),
            sp(0., 0., 640.5, 1024.),
            sp(80., 80., 560.5, 480.),
        ),
        (
            "bottom",
            sp(100., 100., 400., 600.),
            sp(0., 0., 1024., 560.5),
            sp(80., 80., 560., 480.5),
        ),
    ] {
        for bordered in [false, true] {
            eprintln!("shared edge: {name}, bordered {bordered}");
            let mut hidden = Quad {
                content_mask: ContentMask {
                    bounds: hidden_mask,
                },
                ..quad(hidden_bounds, Hsla::from(rgba(0xcc3322ff)))
            };
            if bordered {
                hidden.border_widths = Edges::all(ScaledPixels(2.));
                hidden.border_color = Hsla::from(rgba(0x22cc33ff));
            }
            let mut scene = Scene::default();
            scene.insert_primitive(quad(sp(0., 0., 1024., 1024.), Hsla::from(rgba(0x101010ff))));
            scene.insert_primitive(hidden);
            scene.insert_primitive(quad(cover_bounds, Hsla::from(rgba(0x445566ff))));
            scene.finish();
            culling_keeps_pixels(&mut harness, &scene);
            let occlusion = crate::fast::occlusion::Occlusion::of(&scene, |_| false);
            assert!(!occlusion.hides_quad(&scene.quads[1]), "{name} {bordered}");
        }
    }
}

// --- Partial redraw (`fast::partial`) --- //

/// Draws a scene for `fast::partial::Partial` as `Harness::draw` does, into
/// the canvas it is given.
struct PartialDrawer<'a> {
    harness: &'a mut Harness,
    scene: &'a Scene,
    size: (u32, u32),
    clear: Rgba,
    target: Option<windows::Win32::Graphics::Direct3D11::ID3D11RenderTargetView>,
    /// Drawing fails (a device error mid-frame).
    fail: bool,
}

impl crate::fast::partial::Drawer for PartialDrawer<'_> {
    fn clear(&self) -> [f32; 4] {
        [self.clear.r, self.clear.g, self.clear.b, self.clear.a]
    }

    fn prepare(&mut self) -> Result<()> {
        let harness = &mut *self.harness;
        let tiles = Target {
            device: &harness.device,
            device_context: &harness.context,
            atlas: &harness.atlas,
            globals: &harness.globals,
            depth_scale: None,
        };
        rasterize(
            &tiles,
            &mut harness.pipelines,
            &mut harness.frame,
            self.scene,
        )
    }

    fn begin(
        &mut self,
        target: &Option<windows::Win32::Graphics::Direct3D11::ID3D11RenderTargetView>,
    ) -> Result<()> {
        let (width, height) = self.size;
        let clear = crate::fast::partial::Drawer::clear(self);
        let globals = create_globals(&self.harness.device, width, height)?;
        let context = &self.harness.context;
        unsafe {
            context.ClearRenderTargetView(target.as_ref().context("canvas view")?, &clear);
            context.OMSetRenderTargets(Some(slice::from_ref(target)), None);
            context.RSSetViewports(Some(&[D3D11_VIEWPORT {
                TopLeftX: 0.,
                TopLeftY: 0.,
                Width: width as f32,
                Height: height as f32,
                MinDepth: 0.,
                MaxDepth: 1.,
            }]));
            context.VSSetConstantBuffers(0, Some(slice::from_ref(&globals)));
            context.VSSetConstantBuffers(
                1,
                Some(slice::from_ref(&self.harness.globals.batch_params_buffer)),
            );
            context.PSSetConstantBuffers(0, Some(slice::from_ref(&globals)));
        }
        self.target = target.clone();
        Ok(())
    }

    fn upload(&mut self) -> Result<()> {
        let harness = &mut *self.harness;
        let target = Target {
            device: &harness.device,
            device_context: &harness.context,
            atlas: &harness.atlas,
            globals: &harness.globals,
            depth_scale: None,
        };
        upload(&target, &mut harness.pipelines, self.scene)
    }

    fn draw(&mut self) -> Result<bool> {
        if self.fail {
            anyhow::bail!("the drawer fails");
        }
        let harness = &mut *self.harness;
        let target = Target {
            device: &harness.device,
            device_context: &harness.context,
            atlas: &harness.atlas,
            globals: &harness.globals,
            depth_scale: None,
        };
        draw_scene(
            &target,
            &mut harness.pipelines,
            &mut harness.frame,
            self.scene,
        )?;
        Ok(true)
    }
}

/// Draws `scene` as the next frame of `partial` and checks it against the
/// scene drawn whole, byte for byte. Returns how the frame was made:
/// whether pixels were copied moved, and how many rectangles were drawn
/// again (`None`: drawn whole).
fn partial_frame(
    harness: &mut Harness,
    partial: &mut crate::fast::partial::Partial,
    scene: &Scene,
    uploaded: &[AtlasTile],
) -> Option<(bool, usize)> {
    partial_frame_cleared(harness, partial, scene, uploaded, rgba(0xffffffff))
}

/// [`partial_frame`] in a window cleared to `clear`.
fn partial_frame_cleared(
    harness: &mut Harness,
    partial: &mut crate::fast::partial::Partial,
    scene: &Scene,
    uploaded: &[AtlasTile],
    clear: Rgba,
) -> Option<(bool, usize)> {
    let size = (1024u32, 1024u32);
    let uploaded = uploaded
        .iter()
        .map(|tile| (tile.texture_id, tile.tile_id.0))
        .collect();
    let (device, context) = (harness.device.clone(), harness.context.clone());
    let canvas = {
        let mut drawer = PartialDrawer {
            harness,
            scene,
            size,
            clear,
            target: None,
            fail: false,
        };
        partial
            .frame(&device, &context, size, scene, &uploaded, &mut drawer)
            .expect("frame drawn")
            .expect("a canvas")
    };
    unsafe { context.OMSetRenderTargets(None, None) };
    let made = read_back(&device, &context, &canvas).expect("canvas read");
    let whole = harness.render(scene, device_size(1024, 1024), clear);
    assert_same_pixels(&made, &whole, 1024);
    partial
        .last_plan
        .as_ref()
        .map(|plan| (plan.shift.is_some(), plan.damage.len()))
}

/// A window: a rounded header and two translucent bars that stay, the bars
/// across the scrolled content; and in a clip below the header, scrolled by
/// `offset`, every kind of primitive (`add_content`) and a column of
/// rounded rows that runs past the clip's bottom. `backdrop`: over a plain
/// background (else the cleared window shows).
fn scrolled_scene(harness: &Harness, offset: f32, extra: Option<Quad>, backdrop: bool) -> Scene {
    let clip = sp(32., 96., 960., 896.);
    let mut scene = Scene::default();
    if backdrop {
        scene.insert_primitive(quad(sp(0., 0., 1024., 1024.), Hsla::from(rgba(0x101418ff))));
    }
    add_content(&mut scene, harness, (0., offset), clip);
    for row in 0..14 {
        scene.insert_primitive(Quad {
            content_mask: ContentMask { bounds: clip },
            corner_radii: Corners::all(ScaledPixels(8.)),
            ..quad(
                sp(820., 110. + 110. * row as f32 + offset, 150., 90.),
                Hsla::from(rgba(0x2266aaff + 0x0c0700 * row)),
            )
        });
    }
    if let Some(extra) = extra {
        scene.insert_primitive(extra);
    }
    scene.insert_primitive(Quad {
        corner_radii: Corners::all(ScaledPixels(10.)),
        ..quad(sp(16., 16., 992., 64.), Hsla::from(rgba(0x334455ff)))
    });
    // Two bars that stay over the scrolled content: a rounded one (drawn
    // again wherever the copy moved pixels under it) and a plain one (the
    // same colour over every pixel: only its edges are).
    scene.insert_primitive(Quad {
        corner_radii: Corners::all(ScaledPixels(6.)),
        ..quad(sp(32., 400., 960., 40.), Hsla::from(rgba(0x00000080)))
    });
    scene.insert_primitive(quad(sp(32., 640., 960., 30.), Hsla::from(rgba(0x80000060))));
    scene.finish();
    scene
}

#[test]
fn partial_redraw_of_a_scroll_equals_whole_drawing() {
    let Some(mut harness) = Harness::new() else {
        eprintln!("skipped: no Direct3D 11 device");
        return;
    };
    for backdrop in [true, false] {
        let mut partial = crate::fast::partial::Partial::enabled();
        let frame = |harness: &mut Harness, partial: &mut _, offset: f32, extra: Option<Quad>| {
            let scene = scrolled_scene(harness, offset, extra, backdrop);
            partial_frame(harness, partial, &scene, &[])
        };
        // The first frame is drawn whole.
        assert_eq!(frame(&mut harness, &mut partial, 0., None), None);
        // Scrolled up, twice: moved pixels and damage.
        for offset in [-48., -96.] {
            let made =
                frame(&mut harness, &mut partial, offset, None).expect("made from the last frame");
            assert!(made.0, "pixels are copied moved at {offset}");
        }
        // Nothing changed: nothing drawn.
        assert_eq!(
            frame(&mut harness, &mut partial, -96., None),
            Some((false, 0))
        );
        // Something new without a scroll: only that is drawn again.
        let extra = Quad {
            content_mask: ContentMask {
                bounds: sp(32., 96., 960., 896.),
            },
            ..quad(sp(600., 500., 120., 80.), Hsla::from(rgba(0xffaa00ff)))
        };
        assert_eq!(
            frame(&mut harness, &mut partial, -96., Some(extra)),
            Some((false, 1))
        );
        // It goes while the content scrolls back down.
        let made = frame(&mut harness, &mut partial, -80., None).expect("made from the last frame");
        assert!(made.0);
        // A fractional offset copies nothing (drawn whole, or what moved drawn
        // again), and the next whole offset is made from it.
        let made = frame(&mut harness, &mut partial, -80.5, None);
        assert!(made.is_none_or(|(moved, _)| !moved));
        frame(&mut harness, &mut partial, -64., None);
        frame(&mut harness, &mut partial, 0., None);
    }
}

#[test]
fn partial_redraw_draws_a_tile_uploaded_again() {
    let Some(mut harness) = Harness::new() else {
        eprintln!("skipped: no Direct3D 11 device");
        return;
    };
    let mut partial = crate::fast::partial::Partial::enabled();
    let scene = scrolled_scene(&harness, 0., None, true);
    assert_eq!(partial_frame(&mut harness, &mut partial, &scene, &[]), None);
    // The same primitives, but the image's tile was uploaded since: its
    // sprite is drawn again.
    let image = image_tile(&harness);
    assert_eq!(
        partial_frame(&mut harness, &mut partial, &scene, &[image]),
        Some((false, 1))
    );
}

/// A glyph drawn large and turned by `angle` about its centre, over a
/// plain background.
fn turned_sprite_scene(harness: &Harness, angle: f32) -> Scene {
    let mut scene = Scene::default();
    scene.insert_primitive(quad(sp(0., 0., 1024., 1024.), Hsla::from(rgba(0x101418ff))));
    let (sin, cos) = angle.sin_cos();
    let (cx, cy) = (400. + 60., 400. + 60.);
    scene.insert_primitive(MonochromeSprite {
        order: 0,
        animation: Default::default(),
        bounds: sp(400., 400., 120., 120.),
        content_mask: ContentMask {
            bounds: sp(0., 0., 1024., 1024.),
        },
        color: Hsla::from(rgba(0xffffffff)),
        tile: glyph_tile(harness, 1, false),
        transformation: TransformationMatrix {
            rotation_scale: [[cos, -sin], [sin, cos]],
            translation: [cx - cos * cx + sin * cy, cy - sin * cx - cos * cy],
        },
    });
    scene.finish();
    scene
}

#[test]
fn partial_redraw_reaches_where_a_transformation_takes_a_sprite() {
    let Some(mut harness) = Harness::new() else {
        eprintln!("skipped: no Direct3D 11 device");
        return;
    };
    let mut partial = crate::fast::partial::Partial::enabled();
    let mut made = Vec::new();
    for angle in [0., 0.4, 0.8, 1.9] {
        let scene = turned_sprite_scene(&harness, angle);
        made.push(partial_frame(&mut harness, &mut partial, &scene, &[]));
    }
    // Each turn is one rectangle of damage, the turned sprite's corners in it.
    assert_eq!(
        made,
        [None, Some((false, 1)), Some((false, 1)), Some((false, 1))]
    );
}

#[test]
fn partial_redraw_draws_whole_when_the_clear_colour_changes() {
    let Some(mut harness) = Harness::new() else {
        eprintln!("skipped: no Direct3D 11 device");
        return;
    };
    let mut partial = crate::fast::partial::Partial::enabled();
    // No background: the clear colour shows between the primitives.
    let scene = scrolled_scene(&harness, 0., None, false);
    let white = rgba(0xffffffff);
    let clear = rgba(0x00000000);
    assert_eq!(
        partial_frame_cleared(&mut harness, &mut partial, &scene, &[], white),
        None
    );
    assert_eq!(
        partial_frame_cleared(&mut harness, &mut partial, &scene, &[], white),
        Some((false, 0))
    );
    assert_eq!(
        partial_frame_cleared(&mut harness, &mut partial, &scene, &[], clear),
        None
    );
    assert_eq!(
        partial_frame_cleared(&mut harness, &mut partial, &scene, &[], clear),
        Some((false, 0))
    );
}

#[test]
fn partial_redraw_puts_the_rasterizer_state_back_when_drawing_fails() {
    let Some(mut harness) = Harness::new() else {
        eprintln!("skipped: no Direct3D 11 device");
        return;
    };
    let mut partial = crate::fast::partial::Partial::enabled();
    let scene = scrolled_scene(&harness, 0., None, true);
    assert_eq!(partial_frame(&mut harness, &mut partial, &scene, &[]), None);
    let (device, context) = (harness.device.clone(), harness.context.clone());
    let bound = || unsafe { context.RSGetState() }.ok();
    let before = bound();
    // A frame of damage whose drawing fails.
    let scrolled = scrolled_scene(&harness, -48., None, true);
    let failed = {
        let mut drawer = PartialDrawer {
            harness: &mut harness,
            scene: &scrolled,
            size: (1024, 1024),
            clear: rgba(0xffffffff),
            target: None,
            fail: true,
        };
        partial.frame(
            &device,
            &context,
            (1024, 1024),
            &scrolled,
            &Default::default(),
            &mut drawer,
        )
    };
    assert!(failed.is_err());
    assert_eq!(bound(), before, "the scissor test is left on");
    // The half-drawn canvas is forgotten: the next frame is drawn whole,
    // and right.
    assert_eq!(
        partial_frame(&mut harness, &mut partial, &scrolled, &[]),
        None
    );
}

/// A glyph drawn through `transformation` in `color` (`None`: no glyph),
/// over a plain background.
fn transformed_sprite_scene(
    harness: &Harness,
    sprite: Option<(TransformationMatrix, u32)>,
) -> Scene {
    let mut scene = Scene::default();
    scene.insert_primitive(quad(sp(0., 0., 1024., 1024.), Hsla::from(rgba(0x101418ff))));
    if let Some((transformation, color)) = sprite {
        scene.insert_primitive(MonochromeSprite {
            order: 0,
            animation: Default::default(),
            bounds: sp(200., 200., 120., 120.),
            content_mask: ContentMask {
                bounds: sp(0., 0., 1024., 1024.),
            },
            color: Hsla::from(rgba(color)),
            tile: glyph_tile(harness, 1, false),
            transformation,
        });
    }
    scene.finish();
    scene
}

/// A turn by `angle` about (`cx`, `cy`).
fn turn_about(angle: f32, cx: f32, cy: f32) -> TransformationMatrix {
    let (sin, cos) = angle.sin_cos();
    TransformationMatrix {
        rotation_scale: [[cos, -sin], [sin, cos]],
        translation: [cx - cos * cx + sin * cy, cy - sin * cx - cos * cy],
    }
}

#[test]
fn partial_redraw_follows_sprites_moved_scaled_turned_and_removed() {
    let Some(mut harness) = Harness::new() else {
        eprintln!("skipped: no Direct3D 11 device");
        return;
    };
    let mut partial = crate::fast::partial::Partial::enabled();
    let moved = TransformationMatrix {
        rotation_scale: [[1., 0.], [0., 1.]],
        translation: [300., 40.],
    };
    let scaled = TransformationMatrix {
        rotation_scale: [[2., 0.], [0., 1.5]],
        translation: [0., 0.],
    };
    let states = [
        None,
        // Comes, drawn away from its bounds.
        Some((moved, 0xffffffff)),
        // Changes colour there.
        Some((moved, 0xff4040ff)),
        // Scaled, turned, and gone from where the turn took it.
        Some((scaled, 0xff4040ff)),
        Some((turn_about(0.7, 260., 260.), 0xffffffff)),
        None,
        // Comes scaled, and goes.
        Some((scaled, 0xffffffff)),
        None,
    ];
    for (n, state) in states.into_iter().enumerate() {
        let scene = transformed_sprite_scene(&harness, state);
        let made = partial_frame(&mut harness, &mut partial, &scene, &[]);
        if n > 0 {
            let (copied, rects) = made.expect("made from the last frame");
            assert!(!copied && rects >= 1, "state {n}: {made:?}");
        }
    }
}

/// `source` (a window with one rounded clip) again, with a glyph turned by
/// `angle` among the clip's content (`None`: without), reaching past the
/// clip's edge. `side`: the window's.
#[cfg(feature = "test-support")]
fn clip_with_turned_sprite(
    harness: &Harness,
    source: &Scene,
    side: f32,
    angle: Option<f32>,
) -> Scene {
    // The scene's primitives again, in the order they are painted in.
    fn again(source: &Scene, scene: &mut Scene) {
        assert!(
            source.shadows.is_empty()
                && source.underlines.is_empty()
                && source.paths.is_empty()
                && source.monochrome_sprites.is_empty()
                && source.subpixel_sprites.is_empty()
        );
        let mut quads = source.quads.iter().peekable();
        let mut sprites = source.polychrome_sprites.iter().peekable();
        loop {
            let quad_first = match (quads.peek(), sprites.peek()) {
                (Some(quad), Some(sprite)) => quad.order <= sprite.order,
                (Some(_), None) => true,
                (None, Some(_)) => false,
                (None, None) => break,
            };
            if quad_first {
                scene.insert_primitive(*quads.next().unwrap());
            } else {
                scene.insert_primitive(*sprites.next().unwrap());
            }
        }
    }
    let group = source
        .polychrome_sprites
        .iter()
        .find_map(|sprite| crate::fast::rounded_clip::group(source, sprite.tile.texture_id))
        .expect("a rounded clip");
    let content = group.content.scene().expect("the clip's content");
    let mut child = Scene::default();
    again(content, &mut child);
    if let Some(angle) = angle {
        // Near the clip's right edge: turned, its corners pass the edge.
        let bounds = sp(side * 0.55, side * 0.3, side * 0.4, side * 0.4);
        child.insert_primitive(MonochromeSprite {
            order: 0,
            animation: Default::default(),
            bounds,
            content_mask: ContentMask {
                bounds: sp(0., 0., 1024., 1024.),
            },
            color: Hsla::from(rgba(0xff4040ff)),
            tile: glyph_tile(harness, 1, false),
            transformation: turn_about(angle, side * 0.75, side * 0.5),
        });
    }
    child.layers.frames = content.layers.frames.clone();
    child.finish();
    let mut scene = Scene::default();
    again(source, &mut scene);
    scene.layers.frames = source
        .layers
        .frames
        .iter()
        .map(|frame| {
            if frame.key == group.key {
                LayerFrame {
                    generation: frame.generation + 1,
                    content: Scene::default().into(),
                    ..frame.clone()
                }
            } else {
                frame.clone()
            }
        })
        .collect();
    let key = group.key;
    if let Some(frame) = scene
        .layers
        .frames
        .iter_mut()
        .find(|frame| frame.key == key)
    {
        frame.content = child.into();
    }
    scene.finish();
    scene
}

#[cfg(feature = "test-support")]
#[gpui::test]
fn partial_redraw_follows_a_transformed_sprite_inside_a_rounded_clip(
    cx: &mut gpui::TestAppContext,
) {
    let mut harness =
        Harness::new().expect("rounded clipping regression requires Direct3D or WARP");
    rounded_clip_tests::render_with(cx, rounded_clip_tests::square_fixture, |source, size| {
        let side = size.width.0 as f32;
        let extent = device_size(1024, 1024);
        let white = rgba(0xffffffff);
        let without = clip_with_turned_sprite(&harness, source, side, None);
        let with = clip_with_turned_sprite(&harness, source, side, Some(0.6));
        // The glyph is drawn, and the clip is.
        let plain = harness.render(&without, extent, white);
        assert_ne!(plain, harness.render(&with, extent, white));
        assert_same_pixels(&plain, &harness.render(source, extent, white), 1024);

        let mut partial = crate::fast::partial::Partial::enabled();
        for (n, angle) in [None, Some(0.6), Some(1.3), None, Some(2.0), None]
            .into_iter()
            .enumerate()
        {
            let scene = clip_with_turned_sprite(&harness, source, side, angle);
            let made = partial_frame(&mut harness, &mut partial, &scene, &[]);
            assert_eq!(made.is_some(), n > 0, "frame {n}: {made:?}");
        }
        plain
    });
}
