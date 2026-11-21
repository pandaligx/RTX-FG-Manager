//! A small opt-in inspector; retained status comes from the deployment worker.
use super::*;
use gpui_component::scroll::{Scrollbar, ScrollbarMode};
use rtx_fg_manager::runtime_files;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

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
    fn runtime_path(
        &self,
        kind: &str,
        path: &Path,
        directory: &Path,
        cx: &mut Context<Self>,
    ) -> Div {
        let folder = directory.display().to_string();
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
                    "runtime-path-{kind}-{}",
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
        self.runtime_game = None;
        self.runtime_expanded.clear();
        self.runtime_scroll = ScrollHandle::new();
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
            if !self.c.busy && self.runtime_game != self.c.focus {
                self.runtime_game = self.c.focus.clone();
                self.runtime_versions.clear();
                self.runtime_expanded.clear();
                self.runtime_scroll = ScrollHandle::new();
                cx.notify();
            }
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
                (window.rem_size() * 38.).min(work.size.height),
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
        self.utility_opening = true;
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
                owner.utility_opening = false;
                match result {
                    Ok(handle) => owner.runtime_window = Some(handle.into()),
                    Err(error) => owner.c.warning = Some(format!("{error:#}")),
                }
                cx.notify();
            });
        });
    }
    pub(super) fn runtime_locations(
        &self,
        kind: &'static str,
        paths: BTreeSet<&Path>,
        groups: BTreeMap<String, &runtime_files::StreamlineStatus>,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if groups.is_empty() && paths.len() <= 1 {
            return paths.into_iter().next().map(|path| {
                self.runtime_path(kind, path, path.parent().unwrap_or(path), cx)
                    .into_any_element()
            });
        }
        let open = self.runtime_expanded.contains(kind);
        let mut details = div().v_flex().min_w_0().gap_1().pl_2();
        if open {
            for path in paths {
                details =
                    details.child(self.runtime_path(kind, path, path.parent().unwrap_or(path), cx));
            }
            for group in groups.into_values() {
                let mut plugins = div()
                    .v_flex()
                    .min_w_0()
                    .gap_1()
                    .py_1()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(self.t("Streamline 原生配套插件")),
                    )
                    .child(self.runtime_path(kind, &group.directory, &group.directory, cx));
                let version_row = |name: &str, version: String| {
                    div()
                        .h_flex()
                        .flex_wrap()
                        .justify_between()
                        .gap_x_2()
                        .text_xs()
                        .child(name.to_owned())
                        .child(div().text_color(cx.theme().muted_foreground).child(version))
                };
                for (name, version) in &group.plugins {
                    plugins = plugins.child(version_row(name, self.c.text(version)));
                }
                details = details.child(plugins.child(version_row(
                    "sl.interposer.dll",
                    format!(
                        "{} ({})",
                        self.t(&group.interposer),
                        self.t(if group.interposer_managed {
                            "原件已备份"
                        } else {
                            "保留原件"
                        })
                    ),
                )));
            }
        }
        let trigger = Button::new(SharedString::from(format!("runtime-files-{kind}")))
            .small()
            .ghost()
            .focus_ring(crate::ui_focus::visible(cx))
            .icon(if open {
                IconName::ChevronDown
            } else {
                IconName::ChevronRight
            })
            .label(self.t("查看文件"))
            .on_click(cx.listener(move |this, _, _, cx| {
                if !this.runtime_expanded.remove(kind) {
                    this.runtime_expanded.insert(kind);
                }
                cx.notify();
            }));
        Some(
            gpui_component::collapsible::Collapsible::new()
                .open(open)
                .child(div().h_flex().child(trigger))
                .content(details)
                .into_any_element(),
        )
    }
    fn runtime_body(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let snapshot = self
            .c
            .focus
            .as_ref()
            .and_then(|e| self.c.deployments.get(e));
        let encore_complete = snapshot.is_some_and(|s| {
            let entries = s
                .runtimes
                .iter()
                .filter(|s| s.kind == "encorefg")
                .collect::<Vec<_>>();
            !entries.is_empty() && entries.iter().all(|s| s.applicable)
        });
        let mut rows = div()
            .id("runtime-file-scroll")
            .v_flex()
            .gap_1()
            .min_w_0()
            .min_h_0()
            .h_full()
            .pl_3()
            .pr_4()
            .py_1()
            .track_scroll(&self.runtime_scroll)
            .overflow_y_scroll();
        for kind in runtime_files::KINDS {
            if kind == "xemfg" {
                rows = rows.child(
                    div()
                        .flex_shrink_0()
                        .pb_2()
                        .border_b_1()
                        .border_color(cx.theme().border)
                        .child(self.neural_panel(window, cx)),
                );
            }
            let statuses: Vec<_> = snapshot
                .map(|s| s.runtimes.iter().filter(|s| s.kind == kind).collect())
                .unwrap_or_default();
            let access = runtime_files::access(kind, &statuses);
            if access == runtime_files::Access::Hidden {
                continue;
            }
            let restore_only = access == runtime_files::Access::RestoreOnly;
            let paths = statuses
                .iter()
                .flat_map(|s| &s.paths)
                .filter(|path| {
                    kind != "fg"
                        || path
                            .file_name()
                            .is_some_and(|n| n.eq_ignore_ascii_case("nvngx_dlssg.dll"))
                })
                .map(|p| p.as_path())
                .collect::<BTreeSet<_>>();
            let versions = statuses
                .iter()
                .flat_map(|s| &s.versions)
                .map(|version| self.c.text(version))
                .collect::<std::collections::BTreeSet<_>>();
            let current = if versions.is_empty() {
                self.c.text("未检测到")
            } else {
                versions.into_iter().collect::<Vec<_>>().join(" / ")
            };
            let available = if kind == "streamline" {
                statuses.iter().any(|s| s.available)
                    && statuses.iter().all(|s| !s.applicable || s.available)
            } else {
                !statuses.is_empty() && statuses.iter().all(|s| s.available)
            };
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
                .label(version_label.clone())
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
            let target = if single_version {
                div()
                    .text_xs()
                    .text_color(cx.theme().foreground)
                    .child(version_label)
                    .into_any_element()
            } else {
                picker.into_any_element()
            };
            let controls = div()
                .h_flex()
                .flex_shrink_0()
                .gap_1()
                .when(!restore_only, |row| {
                    row.child(
                        self.button(
                            match kind {
                                "sr" => "runtime-sr-install",
                                "rr" => "runtime-rr-install",
                                "fg" => "runtime-fg-install",
                                "streamline" => "runtime-streamline-install",
                                "encorefg" => "runtime-encorefg-install",
                                _ => "runtime-xemfg-install",
                            },
                            if matches!(kind, "xemfg" | "encorefg") {
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
                })
                .child(
                    self.button(
                        match kind {
                            "sr" => "runtime-sr-restore",
                            "rr" => "runtime-rr-restore",
                            "fg" => "runtime-fg-restore",
                            "streamline" => "runtime-streamline-restore",
                            "encorefg" => "runtime-encorefg-restore",
                            _ => "runtime-xemfg-restore",
                        },
                        if matches!(kind, "xemfg" | "encorefg") {
                            "卸载"
                        } else {
                            "恢复原件"
                        },
                        Command::RuntimeOperation(kind, true),
                        cx,
                    )
                    .small()
                    .disabled(self.c.busy || self.c.read_only || !managed),
                );
            let mut card = div()
                .v_flex()
                .flex_shrink_0()
                .gap_1()
                .py_2()
                .border_b_1()
                .border_color(cx.theme().border)
                .min_w_0()
                .child(
                    div()
                        .h_flex()
                        .flex_wrap()
                        .items_center()
                        .gap_x_2()
                        .gap_y_1()
                        .child(
                            div()
                                .flex_grow(1.)
                                .min_w(rems(8.0))
                                .font_weight(FontWeight::MEDIUM)
                                .child(self.t(runtime_files::label(kind))),
                        )
                        .child(if restore_only {
                            div().text_xs().child(current).into_any_element()
                        } else {
                            runtime_version_indicator(current, target, cx)
                        })
                        .child(controls),
                );
            let groups = if kind == "streamline" {
                statuses
                    .iter()
                    .flat_map(|s| &s.streamline)
                    .map(|s| (core::key(&s.directory), s))
                    .collect()
            } else {
                BTreeMap::new()
            };
            let paths = if kind == "streamline" && !groups.is_empty() {
                BTreeSet::new()
            } else {
                paths
            };
            let locations = self.runtime_locations(kind, paths, groups, cx);
            if let Some(locations) = locations {
                card = card.child(locations);
            }
            if kind == "encorefg" && !available && statuses.iter().any(|s| s.applicable) {
                card = card.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(
                            self.t("游戏已有原生帧生成时，可在「Streamline 原生插件」中检查更新。"),
                        ),
                );
            }
            if let Some(warning) = statuses.iter().find_map(|s| s.warning.as_ref()) {
                card = card.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(self.t(warning)),
                );
            }
            if restore_only {
                card = card.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(self.t("当前安装方案不一致或不支持此组件，仅保留恢复 / 卸载。")),
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
            rows = rows.child(card);
        }
        let game = self
            .c
            .focus
            .as_ref()
            .and_then(|e| self.c.games.iter().find(|g| &g.exe == e));
        let title = game.map(|g| self.c.game_name(g)).unwrap_or_default();
        let game_icon = game
            .and_then(|g| self.c.icons.get(&g.exe))
            .and_then(|icon| icon.clone());
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
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        div()
                            .h_flex()
                            .items_center()
                            .flex_wrap()
                            .gap_2()
                            .justify_between()
                            .child(
                                div()
                                    .h_flex()
                                    .items_center()
                                    .flex_1()
                                    .min_w_0()
                                    .gap_2()
                                    .child(if let Some(icon) = game_icon {
                                        img(icon)
                                            .size_4()
                                            .flex_shrink_0()
                                            .object_fit(ObjectFit::Contain)
                                            .into_any_element()
                                    } else {
                                        Icon::new(IconName::LayoutDashboard)
                                            .size_4()
                                            .into_any_element()
                                    })
                                    .child(
                                        div()
                                            .min_w_0()
                                            .whitespace_normal()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(title),
                                    ),
                            )
                            .when(encore_complete, |row| {
                                row.child(
                                    self.button(
                                        "runtime-encore-complete",
                                        "选择 Encore 安装组件",
                                        Command::RuntimeOperation("encore-complete", false),
                                        cx,
                                    )
                                    .small()
                                    .disabled(self.c.busy || self.c.read_only),
                                )
                            })
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
                                .icon(IconName::RefreshCw)
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
                    ),
            )
            .child(
                div().relative().flex_1().min_h_0().child(rows).child(
                    div().absolute().inset_0().child(
                        Scrollbar::vertical(&self.runtime_scroll).mode(ScrollbarMode::Always),
                    ),
                ),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .px_3()
                    .py_1p5()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(self.t("DLSS 更新保留原件，可随时恢复；游戏共用文件可能影响其他方案。")),
            )
            .relative()
            .child(self.download_overlay(window, cx))
            .into_any_element()
    }
}
