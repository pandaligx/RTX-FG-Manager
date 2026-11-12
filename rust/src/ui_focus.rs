//! Pointer focus remains functional, but only Tab navigation paints its hint.
use gpui::component::{
    Disableable, FocusableExt, IconName, Sizable, StyledExt,
    button::Button,
    menu::{DropdownMenu, PopupMenu},
};
use gpui::{
    base::{Root, RootPlugin},
    *,
};

/// Kit's built-in split caret has no focus-style override, so compose its two
/// ordinary buttons here. Their menu, keyboard and activation behavior stay in Kit.
type MenuBuilder = dyn Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu;

#[derive(IntoElement)]
pub struct SplitButton {
    id: ElementId,
    button: Button,
    disabled: bool,
    menu: Box<MenuBuilder>,
}

impl SplitButton {
    pub fn new(id: impl Into<ElementId>, button: Button) -> Self {
        Self {
            id: id.into(),
            button,
            disabled: false,
            menu: Box::new(|menu, _, _| menu),
        }
    }
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
    pub fn dropdown_menu(
        mut self,
        menu: impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static,
    ) -> Self {
        self.menu = Box::new(menu);
        self
    }
}

impl RenderOnce for SplitButton {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let menu = self.menu;
        div()
            .id(self.id)
            .h_flex()
            .child(
                self.button
                    .focus_ring(visible(cx))
                    .disabled(self.disabled)
                    .rounded_r_none(),
            )
            .child(
                Button::new("popup")
                    .small()
                    .focus_ring(visible(cx))
                    .disabled(self.disabled)
                    .dropdown_caret(true)
                    .rounded_l_none()
                    .border_l_0()
                    .dropdown_menu_with_anchor(Anchor::TopRight, move |popup, window, cx| {
                        menu(popup, window, cx)
                    }),
            )
    }
}

/// Keep dialog close buttons on the same input-modality policy as their body.
pub fn dialog_title(title: SharedString, cx: &App) -> impl IntoElement {
    use gpui::component::button::ButtonVariants;
    let keyboard = visible(cx);
    div()
        .h_flex()
        .items_center()
        .justify_between()
        .gap_2()
        .child(div().flex_1().min_w_0().child(title))
        .child(div().flex_none().size_6().child(
            gpui::component::dialog::DialogClose::new().trigger(move |button| {
                button
                    .small()
                    .ghost()
                    .focus_ring(keyboard)
                    .icon(IconName::Close)
            }),
        ))
}

#[derive(Default)]
struct KeyboardFocus(bool);
impl Global for KeyboardFocus {}

pub fn visible(cx: &App) -> bool {
    cx.try_global::<KeyboardFocus>()
        .is_some_and(|state| state.0)
}

pub fn observe<T: 'static>(cx: &mut Context<T>) {
    // Retained editor views need an explicit invalidation when presentation
    // switches between pointer and keyboard; refreshing the root is not enough.
    cx.observe_global::<KeyboardFocus>(|_, cx| cx.notify())
        .detach();
}

fn set_visible(value: bool, cx: &mut App) {
    if visible(cx) != value {
        cx.set_global(KeyboardFocus(value));
        cx.refresh_windows();
    }
}

pub fn init(cx: &mut App) {
    cx.set_global(KeyboardFocus::default());
    // Tab is usually consumed by Kit's navigation action before key-down
    // listeners run. Observe the delivered keystroke without intercepting it.
    cx.observe_keystrokes(|event, _, cx| {
        if event.keystroke.key == "tab"
            && !event.keystroke.modifiers.control
            && !event.keystroke.modifiers.alt
            && !event.keystroke.modifiers.platform
        {
            set_visible(true, cx);
        }
    })
    .detach();
    Root::register_plugin(cx, |_, _| FocusInputLayer);
}

// The manager uses one native window, including its dialogs and popovers.
// A root decorator observes input before descendants can consume it, without
// stealing focus or cancelling their pointer / keyboard actions.
struct FocusInputLayer;
impl Render for FocusInputLayer {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}
impl RootPlugin for FocusInputLayer {
    fn decorate(
        &self,
        surface: AnyElement,
        _: &Root,
        _: &mut Window,
        _: &mut App,
    ) -> impl IntoElement {
        div()
            .size_full()
            .child(
                // Dialogs occlude ancestor hitboxes. Observe at window capture
                // level before painting the surface, without a hitbox or an
                // event stop, so popovers and modal inputs use the same policy.
                canvas(
                    |_, _, _| (),
                    |_, _, window, _| {
                        window.on_mouse_event(|_: &MouseDownEvent, phase, _, cx| {
                            if phase == DispatchPhase::Capture {
                                set_visible(false, cx);
                            }
                        });
                    },
                )
                .absolute()
                .size_full(),
            )
            .child(surface)
    }
}
