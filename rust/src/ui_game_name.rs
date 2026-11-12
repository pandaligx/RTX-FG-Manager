//! A short local-only rename flow. Catalog matches require an explicit choice.
use super::*;
use rtx_fg_manager::{game_names, scanner::Game};
use std::sync::Arc;

pub(super) struct NameEditor {
    owner: WeakEntity<Manager>,
    exe: String,
    original: String,
    identity: String,
    name: Entity<InputState>,
    query: Entity<InputState>,
    catalog: Arc<game_names::Catalog>,
    candidates: Vec<game_names::Entry>,
    error: Option<String>,
    tr: i18n::Translator,
    _subscriptions: Vec<Subscription>,
}
impl NameEditor {
    fn new(
        owner: WeakEntity<Manager>,
        game: Game,
        c: &Controller,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        crate::ui_focus::observe(cx);
        let original = game_names::original(&game);
        let name = cx.new(|cx| {
            let mut s = InputState::new(window, cx);
            s.set_value(c.game_name(&game), window, cx);
            s
        });
        let query = cx.new(|cx| {
            let mut s = InputState::new(window, cx).placeholder(c.text("输入中文或完整英文名查找"));
            s.set_value(original.clone(), window, cx);
            s
        });
        let name_sub = cx.subscribe_in(&name, window, |this, _, event, window, cx| match event {
            InputEvent::PressEnter { .. } => this.commit(false, window, cx),
            InputEvent::Change => {
                this.error = None;
                cx.notify();
            }
            _ => {}
        });
        let query_sub = cx.subscribe(&query, |this, _, event, cx| {
            if matches!(event, InputEvent::Change) {
                this.candidates = this.catalog.search(&this.query.read(cx).value());
                cx.notify();
            }
        });
        Self {
            owner,
            exe: game.exe.clone(),
            original: original.clone(),
            identity: game_names::keys(&game).join(" · "),
            name,
            query,
            catalog: c.game_names.clone(),
            candidates: c.game_names.search(&original),
            error: None,
            tr: i18n::Translator::new(&c.tr.language),
            _subscriptions: vec![name_sub, query_sub],
        }
    }
    fn commit(&mut self, automatic: bool, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.name.read(cx).value().to_string();
        let result = self.owner.update(cx, |owner, cx| {
            let result = owner
                .c
                .set_game_name(&self.exe, if automatic { None } else { Some(&name) });
            cx.notify();
            result
        });
        match result {
            Ok(Ok(())) => window.close_dialog(cx),
            Ok(Err(e)) => self.error = Some(self.tr.t(&e.to_string())),
            Err(_) => self.error = Some(self.tr.t("游戏已从库中移除")),
        }
        cx.notify();
    }
}
impl Render for NameEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ring = crate::ui_focus::visible(cx);
        let mut body = div()
            .id("game-name-fields")
            .v_flex()
            .gap_3()
            .min_h_0()
            .max_h(
                (window.viewport_size().height - window.rem_size() * 16.)
                    .max(window.rem_size() * 8.),
            )
            .overflow_y_scroll()
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(self.original.clone()),
            )
            .when(!self.identity.is_empty(), |d| {
                d.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(self.identity.clone()),
                )
            })
            .child(
                div()
                    .v_flex()
                    .gap_2()
                    .child(self.tr.t("显示名称"))
                    .child(Input::new(&self.name).focus_ring(ring)),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(
                        self.tr
                            .t("仅修改游戏库显示，不改文件或路径。手动名称不会被扫描覆盖。"),
                    ),
            )
            .child(
                div()
                    .v_flex()
                    .gap_2()
                    .child(self.tr.t("查找中文名称"))
                    .child(Input::new(&self.query).focus_ring(ring)),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(self.tr.t("请核对本体、续作与版本；选择候选后点击保存。")),
            );
        if self.candidates.is_empty() {
            body = body.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(self.tr.t("未找到候选，可直接输入名称。")),
            );
        }
        for entry in &self.candidates {
            let name = entry.chinese().to_owned();
            body = body.child(
                Button::new(SharedString::from(format!("name-candidate-{}", entry.id())))
                    .focus_ring(ring)
                    .ghost()
                    .w_full()
                    .justify_start()
                    .label(format!("{} · {}", entry.chinese(), entry.original()))
                    .tooltip(format!("Wikidata · {}", entry.id()))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.name
                            .update(cx, |s, cx| s.set_value(name.clone(), window, cx));
                        cx.notify();
                    })),
            );
        }
        div()
            .v_flex()
            .gap_4()
            .min_w_0()
            .child(body)
            .when_some(self.error.clone(), |d, error| {
                d.child(div().text_sm().text_color(cx.theme().danger).child(error))
            })
            .child(
                div()
                    .h_flex()
                    .justify_between()
                    .gap_2()
                    .flex_wrap()
                    .child(
                        Button::new("name-auto")
                            .focus_ring(ring)
                            .label(self.tr.t("恢复自动名称"))
                            .on_click(
                                cx.listener(|this, _, window, cx| this.commit(true, window, cx)),
                            ),
                    )
                    .child(
                        div()
                            .h_flex()
                            .gap_2()
                            .child(
                                Button::new("name-cancel")
                                    .focus_ring(ring)
                                    .label(self.tr.t("取消"))
                                    .on_click(|_, window, cx| window.close_dialog(cx)),
                            )
                            .child(
                                Button::new("name-save")
                                    .focus_ring(ring)
                                    .label(self.tr.t("保存"))
                                    .primary()
                                    .disabled(!game_names::valid_title(&self.name.read(cx).value()))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.commit(false, window, cx)
                                    })),
                            ),
                    ),
            )
    }
}
impl Manager {
    pub(super) fn rename_game(&mut self, exe: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.dialog_active || self.c.read_only {
            return;
        }
        let Some(game) = self.c.games.iter().find(|g| g.exe == exe).cloned() else {
            return;
        };
        self.dialog_active = true;
        let owner = cx.entity().downgrade();
        let editor = cx.new(|cx| NameEditor::new(owner.clone(), game, &self.c, window, cx));
        let title = self.t("修改游戏名称");
        let width =
            (window.viewport_size().width - window.rem_size() * 4.).min(window.rem_size() * 38.);
        window.open_dialog(cx, move |dialog, _, cx| {
            let owner = owner.clone();
            dialog
                .title(crate::ui_focus::dialog_title(title.clone(), cx))
                .close_button(false)
                .width(width)
                .child(editor.clone())
                .on_close(move |_, _, cx| {
                    let _ = owner.update(cx, |this, cx| {
                        this.dialog_active = false;
                        cx.notify();
                    });
                })
        });
    }
}
