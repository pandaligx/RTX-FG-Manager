// Modified by Longbridge for gpui-fast.
//! Selection of the current finished scene for one composition surface.

use gpui::{ComposedSceneLayer, Scene};

/// `present` supplies the current frame's scene after `Frame::finish`.
/// Only the layer covering its full stream may borrow it; other layers must
/// have empty range lists and still replay and draw to clear their surfaces.
pub(super) fn whole_scene_layer<'a>(
    scene: &'a Scene,
    layers: &[ComposedSceneLayer],
) -> Option<(usize, &'a Scene)> {
    let mut target = None;
    for (index, layer) in layers.iter().enumerate() {
        if !layer.ranges.is_empty() {
            let [range] = layer.ranges.as_slice() else {
                return None;
            };
            if target.is_some() || range.start != 0 || range.end != scene.len() {
                return None;
            }
            target = Some((index, scene));
        }
    }
    target
}

#[cfg(test)]
mod tests {
    use super::whole_scene_layer;
    use gpui::{
        Bounds, ComposedSceneLayer, CompositionSurfaceId, ContentMask, Quad, ScaledPixels, Scene,
        point, size,
    };
    #[test]
    fn partial_or_split_layers_require_replay_even_when_primitives_match() {
        let mut scene = Scene::default();
        let bounds = Bounds::new(
            point(ScaledPixels(0.), ScaledPixels(0.)),
            size(ScaledPixels(64.), ScaledPixels(64.)),
        );
        scene.push_layer(bounds);
        scene.insert_primitive(Quad {
            bounds,
            content_mask: ContentMask { bounds },
            ..Default::default()
        });
        scene.pop_layer();
        scene.finish();
        let len = scene.len();
        let layer = |ranges| ComposedSceneLayer {
            surface: CompositionSurfaceId::default(),
            ranges,
        };
        // The inner range has the same sole primitive, but must restore its layer.
        for layers in [
            vec![layer(vec![1..len - 1])],
            vec![layer(vec![0..len - 1])],
            vec![layer(vec![1..len])],
            vec![layer(vec![0..1, 1..len])],
            vec![layer(vec![0..len]), layer(vec![0..len])],
            vec![layer(vec![0..len]), layer(vec![1..len - 1])],
            vec![layer(vec![0..len]), layer(vec![0..0])],
            vec![layer(vec![])],
            vec![layer(vec![]), layer(vec![])],
            vec![layer(vec![0..len + 1])],
            vec![],
        ] {
            assert!(whole_scene_layer(&scene, &layers).is_none(), "{layers:?}");
        }

        let mut empty = Scene::default();
        empty.finish();
        assert!(whole_scene_layer(&empty, &[layer(vec![])]).is_none());
        for (layers, expected_target) in [
            (vec![layer(vec![0..0])], 0),
            (vec![layer(vec![]), layer(vec![0..0]), layer(vec![])], 1),
        ] {
            let (target_index, borrowed) = whole_scene_layer(&empty, &layers).unwrap();
            assert_eq!(target_index, expected_target);
            assert!(std::ptr::eq(borrowed, &empty));
        }
        assert!(whole_scene_layer(&empty, &[layer(vec![0..0]), layer(vec![0..0])]).is_none());
    }
}
