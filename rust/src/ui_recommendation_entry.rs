//! Paint-only decoration around the existing semantic recommendation button.
use gpui::component::{ActiveTheme, button::Button};
use gpui::{
    App, BorderStyle, Bounds, ContentMask, Hsla, IntoElement, ParentElement, PathBuilder, Pixels,
    RenderOnce, SharedString, Styled, Window, canvas, div, linear_color_stop, linear_gradient,
    point, px, quad, rems, size, transparent_black,
};
use std::f32::consts::{FRAC_PI_2, TAU};

/// Distance along a closed rounded rectangle, independent of its aspect ratio.
/// Like an SVG offset-path, equal distances take equal time on sides and corners.
struct BorderTrack {
    width: f32,
    height: f32,
    radius: f32,
}

impl BorderTrack {
    fn length(&self) -> f32 {
        2. * (self.width + self.height - 4. * self.radius) + TAU * self.radius
    }

    #[cfg(test)]
    fn point_at(&self, distance: f32) -> (f32, f32) {
        let r = self.radius;
        let horizontal = self.width - 2. * r;
        let vertical = self.height - 2. * r;
        let arc = FRAC_PI_2 * r;
        let mut d = distance.rem_euclid(self.length());
        for (side, straight) in [horizontal, vertical, horizontal, vertical]
            .into_iter()
            .enumerate()
        {
            if d < straight {
                return match side {
                    0 => (r + d, 0.),
                    1 => (self.width, r + d),
                    2 => (self.width - r - d, self.height),
                    _ => (0., self.height - r - d),
                };
            }
            d -= straight;
            if d < arc {
                let (x, y) = match side {
                    0 => (self.width - r, r),
                    1 => (self.width - r, self.height - r),
                    2 => (r, self.height - r),
                    _ => (r, r),
                };
                let angle = (side as f32 - 1.) * FRAC_PI_2 + d / r;
                return (x + r * angle.cos(), y + r * angle.sin());
            }
            d -= arc;
        }
        (r, 0.)
    }

    fn distance_at(&self, x: f32, y: f32) -> f32 {
        let (w, h, r) = (self.width, self.height, self.radius);
        let (x, y) = (x.clamp(0., w), y.clamp(0., h));
        let a = FRAC_PI_2 * r;
        let horizontal = w - 2. * r;
        let vertical = h - 2. * r;
        let distance = if x >= w - r && y <= r {
            horizontal + ((y - r).atan2(x - (w - r)) + FRAC_PI_2) * r
        } else if x >= w - r && y >= h - r {
            horizontal + a + vertical + (y - (h - r)).atan2(x - (w - r)) * r
        } else if x <= r && y >= h - r {
            2. * horizontal + 2. * a + vertical + ((y - (h - r)).atan2(x - r) - FRAC_PI_2) * r
        } else if x <= r && y <= r {
            2. * horizontal
                + 3. * a
                + 2. * vertical
                + ((y - r).atan2(x - r).rem_euclid(TAU) - std::f32::consts::PI) * r
        } else if y <= h - y && y <= x && y <= w - x {
            x - r
        } else if w - x <= x && w - x <= h - y {
            horizontal + a + y - r
        } else if h - y <= x {
            horizontal + a + vertical + a + w - r - x
        } else {
            2. * horizontal + 3. * a + vertical + h - r - y
        };
        distance.rem_euclid(self.length())
    }
}

// The lit half has a soft leading edge and a longer tail. Both ends reach zero
// before the phase wraps, so the loop has no visible join.
fn beam_strength(trail: f32) -> f32 {
    let smooth = |t: f32| {
        let t = t.clamp(0., 1.);
        t * t * (3. - 2. * t)
    };
    smooth(trail / 0.05) * smooth((0.5 - trail) / 0.23)
}

/// Light from each edge fades into the face of the button. Opposing edges
/// contribute independently, so the glow can cross the center continuously.
fn paint_inner_glow(
    bounds: Bounds<Pixels>,
    track: &BorderTrack,
    phase: f32,
    palette: &[Hsla; 7],
    window: &mut Window,
) {
    let scale = window.scale_factor();
    let step = (3. * scale).round().max(1.) as i32;
    let length = track.length();
    for (horizontal, far, angle) in [
        (true, false, 180.),
        (true, true, 0.),
        (false, false, 90.),
        (false, true, 270.),
    ] {
        let (origin, along, across) = if horizontal {
            (f32::from(bounds.left()), track.width, track.height)
        } else {
            (f32::from(bounds.top()), track.height, track.width)
        };
        let first = (origin * scale).floor() as i32;
        let end = ((origin + along) * scale).ceil() as i32;
        for start in (first..end).step_by(step as usize) {
            let stop = (start + step).min(end);
            let mid = (start + stop) as f32 / (2. * scale) - origin;
            let edge = if far { across } else { 0. };
            let (x, y) = if horizontal { (mid, edge) } else { (edge, mid) };
            let trail = (phase - track.distance_at(x, y) / length).rem_euclid(1.);
            let strength = beam_strength(trail);
            if strength <= 0. {
                continue;
            }
            let tint = crate::ui_themes::flowing_accent(palette, phase - 1.75 * trail);
            let depth = (track.height * (0.52 + 0.43 * strength) / across).min(1.);
            let fog = linear_gradient(
                angle,
                linear_color_stop(tint.opacity(0.46 * strength), 0.),
                linear_color_stop(tint.opacity(0.), depth),
            );
            let from = px(start as f32 / scale);
            let span = px((stop - start) as f32 / scale);
            let clip = if horizontal {
                Bounds::new(point(from, bounds.top()), size(span, bounds.size.height))
            } else {
                Bounds::new(point(bounds.left(), from), size(bounds.size.width, span))
            };
            window.with_content_mask(Some(ContentMask { bounds: clip }), |window| {
                window.paint_quad(quad(
                    bounds,
                    px(track.radius),
                    fog,
                    px(0.),
                    transparent_black(),
                    BorderStyle::Solid,
                ));
            });
        }
    }
}

/// Paint both people inside one fixed slot. Animating the vector geometry keeps
/// neighboring atlas pixels out of the icon and leaves the label stationary.
fn paint_players(bounds: Bounds<Pixels>, phase: f32, color: Hsla, window: &mut Window) {
    let unit =
        bounds.size.width.min(bounds.size.height) * ((1. + 0.10 * (phase * TAU).sin()) / 24.);
    let center = bounds.center();
    let at = |x: f32, y: f32| point(center.x + unit * (x - 12.), center.y + unit * (y - 12.));
    let mut people = PathBuilder::fill();
    for (x, y, r) in [(9., 7.8, 3.5), (17.7, 9.1, 2.7)] {
        people.move_to(at(x - r, y));
        for end in [x + r, x - r] {
            people.arc_to(point(unit * r, unit * r), px(0.), false, true, at(end, y));
        }
        people.close();
    }
    for (x, y, w, h) in [(9., 19.5, 6.7, 6.7), (18.8, 18.5, 2.9, 5.3)] {
        people.move_to(at(x - w, y));
        people.cubic_bezier_to(
            at(x, y - h),
            at(x - w, y - h * 0.64),
            at(x - w * 0.52, y - h),
        );
        people.cubic_bezier_to(
            at(x + w, y),
            at(x + w * 0.52, y - h),
            at(x + w, y - h * 0.64),
        );
        people.close();
    }
    if let Ok(path) = people.build() {
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            window.paint_path(path, color)
        });
    }
}

#[derive(IntoElement)]
pub(super) struct RecommendationEntry {
    button: Button,
    label: SharedString,
    phase: f32,
}

impl RecommendationEntry {
    pub(super) fn new(button: Button, label: SharedString) -> Self {
        Self {
            button,
            label,
            phase: 0.125,
        }
    }

    pub(super) fn phase(mut self, phase: f32) -> Self {
        self.phase = phase;
        self
    }
}

impl RenderOnce for RecommendationEntry {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme();
        let palette = crate::ui_themes::recommendation_accents(theme);
        let radius = theme.radius;
        let phase = self.phase;
        let icon_color = theme.foreground;
        let players = canvas(
            |_, _, _| (),
            move |bounds, _, window, _| {
                paint_players(bounds, phase, icon_color, window);
            },
        )
        .size(rems(1.18))
        .flex_shrink_0();
        let light = canvas(
            |_, _, _| (),
            move |bounds, _, window, _| {
                let width = f32::from(bounds.size.width);
                let height = f32::from(bounds.size.height);
                if width < 2. || height < 2. {
                    return;
                }
                let track = BorderTrack {
                    width,
                    height,
                    radius: (f32::from(radius) - 1.).clamp(0.1, width.min(height) / 2.),
                };
                paint_inner_glow(bounds, &track, phase, &palette, window);
                let length = track.length();
                let scale = window.scale_factor();
                // Clip analytically rounded rings on the physical pixel grid. Only
                // visit the perimeter; the canvas has no hitbox or layout padding.
                let left = f32::from(bounds.left());
                let top = f32::from(bounds.top());
                let x0 = (left * scale).floor() as i32;
                let y0 = (top * scale).floor() as i32;
                let x1 = (f32::from(bounds.right()) * scale).ceil() as i32;
                let y1 = (f32::from(bounds.bottom()) * scale).ceil() as i32;
                let tile = (4. * scale).round().max(1.) as i32;
                let band = (track.radius.max(2.) * scale).ceil() as i32;
                for y in (y0..y1).step_by(tile as usize) {
                    let bottom = (y + tile).min(y1);
                    let mut x = x0;
                    while x < x1 {
                        let right = (x + tile).min(x1);
                        if y >= y0 + band
                            && bottom <= y1 - band
                            && x >= x0 + band
                            && right <= x1 - band
                        {
                            x += ((x1 - band - x) / tile).max(1) * tile;
                            continue;
                        }
                        let lx = (x + right) as f32 / (2. * scale) - left;
                        let ly = (y + bottom) as f32 / (2. * scale) - top;
                        let distance = track.distance_at(lx, ly) / length;
                        let trail = (phase - distance).rem_euclid(1.);
                        let alpha = beam_strength(trail);
                        if alpha > 0. {
                            let color =
                                crate::ui_themes::flowing_accent(&palette, phase - 1.75 * trail);
                            let clip = Bounds::new(
                                point(px(x as f32 / scale), px(y as f32 / scale)),
                                size(
                                    px((right - x) as f32 / scale),
                                    px((bottom - y) as f32 / scale),
                                ),
                            );
                            window.with_content_mask(
                                Some(ContentMask { bounds: clip }),
                                |window| {
                                    window.paint_quad(quad(
                                        bounds,
                                        px(track.radius),
                                        transparent_black(),
                                        px(2.),
                                        color.opacity(alpha),
                                        BorderStyle::Solid,
                                    ));
                                },
                            );
                        }
                        x = right;
                    }
                }
            },
        )
        .absolute()
        .inset_0()
        .size_full();
        self.button.p_0().child(
            div()
                .relative()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .gap_1()
                .child(light)
                .child(players)
                .child(
                    div()
                        .min_w_0()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .child(self.label),
                ),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exactly_half_the_perimeter_lights_up_and_the_loop_wraps() {
        for phase in [0., 0.13, 0.51, 0.92] {
            let strength =
                |position: f32, phase: f32| beam_strength((phase - position).rem_euclid(1.));
            let lit = (0..1000)
                .filter(|i| strength(*i as f32 / 1000., phase) > 0.)
                .count();
            assert!((499..=501).contains(&lit));
            for i in 0..100 {
                let position = i as f32 / 100.;
                assert!(
                    (strength(position, phase) - strength(position, phase + 1.)).abs() < 0.00001
                );
            }
        }
    }

    #[test]
    fn border_track_is_closed_and_has_no_jump_at_corners() {
        for (width, height, radius) in [(360., 26., 6.), (220., 38., 9.), (40., 40., 20.)] {
            let track = BorderTrack {
                width,
                height,
                radius,
            };
            let length = track.length();
            let start = track.point_at(0.);
            assert_eq!(start, track.point_at(length));
            assert_eq!(start, track.point_at(-length));
            // Physical travel stays uniform, including all corner and loop joins.
            for i in 0..5000 {
                let d = length * i as f32 / 5000.;
                let (ax, ay) = track.point_at(d);
                let projected = track.distance_at(ax, ay);
                let error = (projected - d).abs();
                assert!(error.min((length - error).abs()) < 0.002);
                let (bx, by) = track.point_at(d + 0.1);
                let travel = ((bx - ax).powi(2) + (by - ay).powi(2)).sqrt();
                assert!((travel - 0.1).abs() < 0.001, "distance {d}: {travel}");
                assert!(
                    ax >= -0.001 && ay >= -0.001 && ax <= width + 0.001 && ay <= height + 0.001
                );
            }
        }
    }
}
