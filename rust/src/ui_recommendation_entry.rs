//! Paint-only decoration around the existing semantic recommendation button.
use gpui::component::{ActiveTheme, Icon, button::Button};
use gpui::{
    App, BoxShadow, IntoElement, ParentElement, RenderOnce, Styled, Transformation, Window, div,
    linear_color_stop, linear_gradient, point, px, size,
};

#[derive(IntoElement)]
pub(super) struct RecommendationEntry {
    button: Button,
    phase: f32,
    animated: bool,
}

impl RecommendationEntry {
    pub(super) fn new(button: Button, animated: bool) -> Self {
        Self {
            button,
            phase: 0.125,
            animated,
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
        let edge = px(1.25);
        let breath = if self.animated {
            (1. - (std::f32::consts::TAU * self.phase * 2.).cos()) * 0.5
        } else {
            1.
        };
        let scale = 1. + 0.18 * breath;
        div()
            .w_full()
            .p(edge)
            .rounded(theme.radius + edge)
            .bg(linear_gradient(
                360. * self.phase,
                linear_color_stop(theme.info, 0.),
                linear_color_stop(theme.success, 1.),
            ))
            .shadow(vec![BoxShadow {
                color: theme.info.opacity(0.10 + 0.08 * breath),
                offset: point(px(0.), px(0.)),
                blur_radius: px(3. + 2. * breath),
                spread_radius: px(0.),
                inset: false,
            }])
            .child(
                // Button variants can use translucent fills. An opaque inner
                // surface keeps the gradient confined to the surrounding edge.
                div()
                    .w_full()
                    .rounded(theme.radius)
                    .bg(theme.background)
                    .child(
                        self.button.icon(
                            Icon::new(gpui::assets::IconName::UsersRound)
                                .text_color(theme.info)
                                .opacity(0.55 + 0.45 * breath)
                                .transform(Transformation::scale(size(scale, scale))),
                        ),
                    ),
            )
    }
}
