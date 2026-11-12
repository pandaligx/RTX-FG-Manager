//! Paint-only decoration around the existing semantic recommendation button.
use gpui::component::{ActiveTheme, Icon, button::Button};
use gpui::{
    App, BorderStyle, Bounds, ContentMask, IntoElement, ParentElement, RenderOnce, Styled, Window,
    canvas, div, point, px, quad, size, transparent_black,
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
        } else if y <= r {
            x - r
        } else if x >= w - r {
            horizontal + a + y - r
        } else if y >= h - r {
            horizontal + a + vertical + a + w - r - x
        } else {
            2. * horizontal + 3. * a + vertical + h - r - y
        };
        distance.rem_euclid(self.length())
    }
}

#[derive(IntoElement)]
pub(super) struct RecommendationEntry {
    button: Button,
    phase: f32,
}

impl RecommendationEntry {
    pub(super) fn new(button: Button) -> Self {
        Self {
            button,
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
        let edge = px(2.25);
        let palette = crate::ui_themes::recommendation_accents(theme);
        let phase = self.phase;
        let radius = theme.radius + edge;
        let icon_color = theme.muted_foreground;
        div()
            .relative()
            .w_full()
            .p(edge)
            .child(
                canvas(
                    |_, _, _| (),
                    move |bounds, _, window, _| {
                        let width = f32::from(bounds.size.width - edge);
                        let height = f32::from(bounds.size.height - edge);
                        if width <= 0. || height <= 0. {
                            return;
                        }
                        let track = BorderTrack {
                            width,
                            height,
                            radius: f32::from(radius - edge / 2.)
                                .max(0.1)
                                .min(width.min(height) / 2.),
                        };
                        let length = track.length();
                        // GPUI's rounded-quad shader antialiases both ring edges.
                        // Color tiles only clip that same analytic shape: unlike
                        // separate polygon strokes, corners have no faceted joins.
                        let scale = window.scale_factor().max(1.);
                        let x0 = (f32::from(bounds.left()) * scale).floor() as i32;
                        let y0 = (f32::from(bounds.top()) * scale).floor() as i32;
                        let x1 = (f32::from(bounds.right()) * scale).ceil() as i32;
                        let y1 = (f32::from(bounds.bottom()) * scale).ceil() as i32;
                        let outer = f32::from(radius);
                        for y in (y0..y1).step_by(3) {
                            for x in (x0..x1).step_by(3) {
                                let left = x as f32 / scale;
                                let top = y as f32 / scale;
                                let right = (x + 3).min(x1) as f32 / scale;
                                let bottom = (y + 3).min(y1) as f32 / scale;
                                let lx = left - f32::from(bounds.left());
                                let rx = right - f32::from(bounds.left());
                                let ty = top - f32::from(bounds.top());
                                let by = bottom - f32::from(bounds.top());
                                let bw = f32::from(bounds.size.width);
                                let bh = f32::from(bounds.size.height);
                                let stroke = f32::from(edge);
                                if (lx >= outer
                                    && rx <= bw - outer
                                    && ty >= stroke
                                    && by <= bh - stroke)
                                    || (ty >= outer
                                        && by <= bh - outer
                                        && lx >= stroke
                                        && rx <= bw - stroke)
                                {
                                    continue;
                                }
                                let distance = track
                                    .distance_at((lx + rx - stroke) / 2., (ty + by - stroke) / 2.);
                                let color = crate::ui_themes::flowing_accent(
                                    &palette,
                                    distance / length - phase,
                                );
                                let clip = Bounds::new(
                                    point(px(left), px(top)),
                                    size(px(right - left), px(bottom - top)),
                                );
                                window.with_content_mask(
                                    Some(ContentMask { bounds: clip }),
                                    |window| {
                                        window.paint_quad(quad(
                                            bounds,
                                            radius,
                                            transparent_black(),
                                            edge,
                                            color,
                                            BorderStyle::Solid,
                                        ));
                                    },
                                );
                            }
                        }
                    },
                )
                .absolute()
                .inset_0()
                .size_full(),
            )
            .child(
                // Button variants can use translucent fills. An opaque inner
                // surface keeps the gradient confined to the surrounding edge.
                div()
                    .w_full()
                    .rounded(theme.radius)
                    .bg(theme.background)
                    .child(self.button.icon(
                        Icon::new(gpui::assets::IconName::UsersRound).text_color(icon_color),
                    )),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
