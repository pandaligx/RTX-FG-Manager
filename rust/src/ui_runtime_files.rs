//! A small opt-in inspector; retained status comes from the deployment worker.
use super::*;
use rtx_fg_manager::runtime_files;

struct RuntimeView {
    owner: WeakEntity<Manager>,
}
impl Render for RuntimeView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.owner
            .update(cx, |owner, cx| {
                // Render the fixed target without changing the library selection.
                let selected = owner.c.focus.clone();
                owner.c.focus = owner.runtime_game.clone();
                let body = owner.runtime_body(window, cx);
                owner.c.focus = selected;
                body
            })
            .unwrap_or_else(|_| div().into_any_element())
    }
}
impl Manager {
    pub(super) fn runtime_path(&self, path: &std::path::Path, cx: &mut Context<Self>) -> Div {
        let folder = path.parent().unwrap_or(path).display().to_string();
        let opened = self
            .folder_feedback
            .as_ref()
            .is_some_and(|(p, _)| *p == folder);
        div()
            .h_flex()
            .items_start()
            .gap_2()
            .min_w_0()
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .child(
                Button::new(SharedString::from(format!(
                    "runtime-path-{}",
                    path.display()
                )))
                .custom(
                    ButtonCustomVariant::new(cx)
                        .hover(*cx.theme().tokens.secondary_hover)
                        .active(*cx.theme().tokens.secondary_active),
                )
                .focus_ring(crate::ui_focus::visible(cx))
                .size(rems(1.25))
                .icon(
                    Icon::new(if opened {
                        IconName::FolderOpen
                    } else {
                        IconName::Folder
                    })
                    .size(rems(0.875)),
                )
                .accessibility_label(self.t("打开目录"))
                .tooltip(self.t("打开目录"))
                .flex_shrink_0()
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.folder_feedback = Some((folder.clone(), Instant::now()));
                    this.act(Command::Open(folder.clone()), window, cx);
                })),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .whitespace_normal()
                    .child(path.display().to_string()),
            )
    }

    pub(super) fn clear_runtime_window(&mut self, cx: &mut Context<Self>) {
        self.runtime_window = None;
        self.runtime_feedback = None;
        self.runtime_game = None;
        cx.notify();
    }
    pub(super) fn reconcile_runtime_window(&mut self, cx: &mut Context<Self>) {
        if let Some(handle) = self.runtime_window
            && !cx
                .windows()
                .iter()
                .any(|w| w.window_id() == handle.window_id())
        {
            // A removed utility must never trap main-window commands behind
            // an activation of a dead handle, even if its close callback was
            // skipped. The worker has its own cancellation/generation state.
            self.clear_runtime_window(cx);
        }
    }
    pub(super) fn runtime_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.reconcile_runtime_window(cx);
        if let Some(handle) = self.runtime_window {
            cx.defer(move |cx| {
                let _ = handle.update(cx, |_, window, _| window.activate_window());
            });
            return;
        }
        if self.dialog_active || self.c.focus.is_none() || self.c.busy {
            return;
        }
        self.runtime_game = self.c.focus.clone();
        self.runtime_versions.clear();
        self.runtime_feedback = None;
        let owner = cx.entity();
        let title = self.t("DLSS / XeSS 运行库");
        let work = window
            .display(cx)
            .map(|d| d.visible_bounds())
            .unwrap_or(window.bounds());
        let bounds = Bounds::centered_at(
            work.center(),
            size(
                (window.rem_size() * 52.).min(work.size.width),
                (window.rem_size() * 47.).min(work.size.height),
            ),
        );
        let options = WindowOptions {
            titlebar: Some(TitlebarOptions {
                title: Some(title),
                appears_transparent: true,
                ..Default::default()
            }),
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            // Native window minima are platform coordinates, not content spacing.
            window_min_size: Some(size(
                px(540.).min(work.size.width),
                px(440.).min(work.size.height),
            )),
            ..Default::default()
        };
        // Native window creation may draw synchronously on Windows. Defer until
        // Manager's event-handler lease is released before rendering its child.
        self.dialog_active = true;
        cx.defer(move |cx| {
            let result_owner = owner.clone();
            let result = cx.open_window(options, move |window, cx| {
                let close = owner.downgrade();
                window.on_window_should_close(cx, move |window, cx| {
                    close
                        .update(cx, |this, cx| {
                            if window.has_active_dialog(cx) {
                                return false;
                            }
                            this.clear_runtime_window(cx);
                            true
                        })
                        .unwrap_or(true)
                });
                let editor = cx.new(|cx| {
                    cx.observe(&owner, |_, _, cx| cx.notify()).detach();
                    cx.observe_global::<Theme>(|_, cx| cx.notify()).detach();
                    crate::ui_focus::observe(cx);
                    cx.spawn_in(window, async move |this, cx| {
                        loop {
                            smol::Timer::after(Duration::from_millis(80)).await;
                            if cx
                                .update(|window, cx| {
                                    this.update(cx, |this: &mut RuntimeView, cx| {
                                        let _ = this.owner.update(cx, |owner, cx| {
                                            if owner.c.closing || !owner.accepts_feedback(window) {
                                                return;
                                            }
                                            if let Some(message) = owner.c.warning.take() {
                                                owner.runtime_feedback = Some((true, message));
                                                cx.notify();
                                            } else if let Some(message) = owner.c.notice.take() {
                                                owner.runtime_feedback = Some((false, message));
                                                cx.notify();
                                            }
                                            if owner.c.confirm.is_some() {
                                                owner.dialogs(window, cx);
                                            }
                                        });
                                    })
                                })
                                .is_err()
                            {
                                break;
                            }
                        }
                    })
                    .detach();
                    RuntimeView {
                        owner: owner.downgrade(),
                    }
                });
                cx.new(|cx| Root::new(editor, window, cx))
            });
            result_owner.update(cx, |owner, cx| {
                owner.dialog_active = false;
                match result {
                    Ok(handle) => owner.runtime_window = Some(handle.into()),
                    Err(error) => owner.c.warning = Some(format!("{error:#}")),
                }
                cx.notify();
            });
        });
    }
    fn runtime_body(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let snapshot = self
            .c
            .focus
            .as_ref()
            .and_then(|e| self.c.deployments.get(e));
        let mut rows = div()
            .id("runtime-file-scroll")
            .v_flex()
            .gap_2()
            .min_w_0()
            .min_h_0()
            .flex_1()
            .px_3()
            .py_2()
            .overflow_y_scroll();
        for kind in runtime_files::KINDS {
            if kind == "xemfg" {
                rows = rows.child(
                    div()
                        .pb_2()
                        .border_b_1()
                        .border_color(cx.theme().border)
                        .child(self.neural_panel(window, cx)),
                );
            }
            let statuses: Vec<_> = snapshot
                .map(|s| s.runtimes.iter().filter(|s| s.kind == kind).collect())
                .unwrap_or_default();
            let paths = statuses
                .iter()
                .flat_map(|s| &s.paths)
                .collect::<std::collections::BTreeSet<_>>();
            let versions = statuses
                .iter()
                .flat_map(|s| &s.versions)
                .cloned()
                .collect::<std::collections::BTreeSet<_>>();
            let current = if versions.is_empty() {
                self.c.text("未检测到")
            } else {
                versions.into_iter().collect::<Vec<_>>().join(" / ")
            };
            let available = !statuses.is_empty() && statuses.iter().all(|s| s.available);
            let managed = statuses.iter().any(|s| s.managed);
            let recommended = runtime_files::recommended(kind);
            let choices = runtime_files::versions(kind);
            let single_version = choices.len() < 2;
            let selected = self
                .runtime_versions
                .get(kind)
                .filter(|v| choices.contains(v))
                .cloned()
                .unwrap_or_else(|| recommended.clone());
            let view = cx.entity().downgrade();
            let selected_for_menu = selected.clone();
            let recommended_label = self.t("推荐");
            let version_label = if selected == recommended {
                format!("{selected} ({recommended_label})")
            } else {
                selected.clone()
            };
            let picker = Button::new(SharedString::from(format!("runtime-{kind}-version")))
                .small()
                .label(version_label)
                .icon(IconName::ChevronDown)
                .focus_ring(crate::ui_focus::visible(cx))
                .disabled(self.c.busy || single_version)
                .dropdown_menu(move |mut menu, _, _| {
                    for value in &choices {
                        let value = value.clone();
                        let view = view.clone();
                        menu = menu.item(
                            PopupMenuItem::new(SharedString::from(if value == recommended {
                                format!("{value} ({recommended_label})")
                            } else {
                                value.clone()
                            }))
                            .checked(value == selected_for_menu)
                            .on_click(move |_, window, cx| {
                                let _ = view.update(cx, |this, cx| {
                                    this.act(
                                        Command::RuntimeVersion(kind, value.clone()),
                                        window,
                                        cx,
                                    )
                                });
                            }),
                        );
                    }
                    menu
                });
            let mut card = div()
                .v_flex()
                .gap_2()
                .pb_2()
                .border_b_1()
                .border_color(cx.theme().border)
                .min_w_0()
                .child(
                    div()
                        .h_flex()
                        .flex_wrap()
                        .gap_2()
                        .justify_between()
                        .child(
                            div()
                                .font_weight(FontWeight::MEDIUM)
                                .child(self.t(runtime_files::label(kind))),
                        )
                        .child(runtime_version_indicator(current, &selected, cx)),
                );
            for path in paths {
                card = card.child(self.runtime_path(path, cx));
            }
            if let Some(warning) = statuses.iter().find_map(|s| s.warning.as_ref()) {
                card = card.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(self.t(warning)),
                );
            }
            if single_version && self.c.runtime_catalog_load.failed() {
                card = card.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(self.t(self.c.runtime_catalog_load.single_version_hint(kind))),
                );
            }
            card = card.child(
                div()
                    .h_flex()
                    .flex_wrap()
                    .gap_2()
                    .justify_end()
                    .child(picker)
                    .child(
                        self.button(
                            match kind {
                                "sr" => "runtime-sr-install",
                                "rr" => "runtime-rr-install",
                                "fg" => "runtime-fg-install",
                                _ => "runtime-xemfg-install",
                            },
                            if kind == "xemfg" {
                                "安装"
                            } else {
                                "备份并更新"
                            },
                            Command::RuntimeOperation(kind, false),
                            cx,
                        )
                        .small()
                        .disabled(self.c.busy || self.c.read_only || !available),
                    )
                    .child(
                        self.button(
                            match kind {
                                "sr" => "runtime-sr-restore",
                                "rr" => "runtime-rr-restore",
                                "fg" => "runtime-fg-restore",
                                _ => "runtime-xemfg-restore",
                            },
                            if kind == "xemfg" {
                                "卸载"
                            } else {
                                "恢复原件"
                            },
                            Command::RuntimeOperation(kind, true),
                            cx,
                        )
                        .small()
                        .disabled(self.c.busy || self.c.read_only || !managed),
                    ),
            );
            rows = rows.child(card);
        }
        let game = self
            .c
            .focus
            .as_ref()
            .and_then(|e| self.c.games.iter().find(|g| &g.exe == e))
            .map(|g| self.c.game_name(g))
            .unwrap_or_default();
        let mut footer = div()
            .v_flex()
            .gap_2()
            .p_3()
            .border_t_1()
            .border_color(cx.theme().border)
            .flex_shrink_0()
            .text_xs();
        if self.c.busy {
            let progress = self
                .c
                .payload_progress
                .as_ref()
                .map(|p| {
                    format!(
                        "{}  {:.0}%  {:.1} / {:.1} MB",
                        self.c.text(p.transfer.label()),
                        p.transfer.percent(),
                        p.transfer.completed as f64 / 1_000_000.,
                        p.transfer.total as f64 / 1_000_000.
                    )
                })
                .unwrap_or_else(|| self.c.text("正在处理…"));
            footer = footer.child(
                div()
                    .h_flex()
                    .gap_2()
                    .justify_between()
                    .child(progress)
                    .child(
                        self.button("runtime-cancel", "取消", Command::Cancel, cx)
                            .small(),
                    ),
            );
        } else if let Some((error, message)) = &self.runtime_feedback {
            footer = footer.child(
                div()
                    .whitespace_normal()
                    .text_color(if *error {
                        cx.theme().danger
                    } else {
                        cx.theme().foreground
                    })
                    .child(self.t(if *error && message.starts_with("下载失败") {
                        "下载失败，已保留现有文件。请检查网络后重试。"
                    } else {
                        message
                    })),
            );
        }
        div()
            .v_flex()
            .size_full()
            .min_w_0()
            .min_h_0()
            .text_sm()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(TitleBar::new().child(div().pl_3().child(self.t("DLSS / XeSS 运行库"))))
            .child(
                div()
                    .v_flex()
                    .gap_1()
                    .px_3()
                    .py_2()
                    .flex_shrink_0()
                    .child(
                        div()
                            .h_flex()
                            .gap_2()
                            .justify_between()
                            .child(div().font_weight(FontWeight::SEMIBOLD).child(game))
                            .child(
                                self.button(
                                    "runtime-refresh",
                                    if self.c.runtime_catalog_load.loading() {
                                        "正在加载…"
                                    } else {
                                        "刷新"
                                    },
                                    Command::RefreshRuntimeFiles,
                                    cx,
                                )
                                .small()
                                .disabled(self.c.busy || self.c.runtime_catalog_load.loading()),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .whitespace_normal()
                            .text_color(if self.c.runtime_catalog_load.failed() {
                                cx.theme().warning
                            } else {
                                cx.theme().muted_foreground
                            })
                            .child(self.t(self.c.runtime_catalog_load.message())),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(self.t(
                                "DLSS 更新保留原件，可随时恢复；游戏共用文件可能影响其他方案。",
                            )),
                    ),
            )
            .child(rows)
            .when(self.c.busy || self.runtime_feedback.is_some(), |view| {
                view.child(footer)
            })
            .into_any_element()
    }
}
