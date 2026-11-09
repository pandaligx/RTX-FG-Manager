//! One non-modal download surface in the active application window.
//! The controller owns transfers; closing a utility never cancels them.
use super::*;
use gpui_component::{notification::Notification, progress::Progress};

#[derive(Clone, Copy, Debug)]
pub(super) enum FeedbackKind {
    Info,
    Cancelled,
    Success,
    Error,
}

/// Kit owns the surface and dismissal animation; brief results expire after two seconds.
pub(super) fn notify(kind: FeedbackKind, message: String, window: &mut Window, cx: &mut App) {
    let brief = matches!(kind, FeedbackKind::Success | FeedbackKind::Cancelled);
    let previous = brief.then(|| window.notifications(cx));
    window.push_notification(
        Notification::new()
            .id1::<FeedbackKind>(SharedString::from(format!("{kind:?}:{message}")))
            .placement(Anchor::TopCenter)
            .autohide(matches!(kind, FeedbackKind::Info))
            .p_0()
            .content(move |_, _, _| {
                let alert = match kind {
                    FeedbackKind::Info | FeedbackKind::Cancelled => {
                        Alert::info("app-feedback", message.clone())
                    }
                    FeedbackKind::Success => Alert::success("app-feedback", message.clone()),
                    FeedbackKind::Error => Alert::error("app-feedback", message.clone()),
                };
                div()
                    .id("feedback-content")
                    .max_h(rems(18.))
                    .overflow_y_scroll()
                    .child(alert.small().pr_6())
                    .into_any_element()
            }),
        cx,
    );
    if let Some(previous) = previous
        && let Some(note) = window
            .notifications(cx)
            .iter()
            .find(|note| !previous.contains(note))
    {
        // Bind to this instance, so an older timer cannot dismiss a replacement
        // with the same message. A closed window or manually dismissed note is a no-op.
        let note = note.downgrade();
        window
            .spawn(cx, async move |cx| {
                cx.background_executor().timer(Duration::from_secs(2)).await;
                let _ = cx.update(|window, cx| {
                    if let Some(note) = note.upgrade() {
                        note.update(cx, |note, cx| note.dismiss(window, cx));
                    }
                });
            })
            .detach();
    }
}

impl Manager {
    pub(super) fn scan_banner(&self, cx: &Context<Self>) -> AnyElement {
        let Some(scan) = self.c.scan_progress.as_ref().filter(|_| self.c.busy) else {
            return div().into_any_element();
        };
        let cancelling = self.c.cancel.load(Ordering::Relaxed);
        let path = scan.path.clone();
        let mut counts = format!("{} {}", scan.directories, self.c.text("个目录"));
        if scan.skipped > 0 {
            counts.push_str(&format!("，{} {}", self.c.text("已跳过"), scan.skipped));
        }
        div()
            .v_flex()
            .gap_2()
            .p_3()
            .min_w_0()
            .rounded_md()
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().muted)
            .child(
                div()
                    .h_flex()
                    .gap_2()
                    .child(div().flex_1().child(self.t(if cancelling {
                        "正在停止扫描…"
                    } else {
                        "正在扫描游戏…"
                    })))
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(counts),
                    )
                    .child(
                        self.button("scan-cancel", "取消", Command::Cancel, cx)
                            .small()
                            .disabled(cancelling),
                    ),
            )
            .child(
                Progress::new("scan-progress")
                    .small()
                    .loading(true)
                    .accessibility_label(self.t("正在扫描游戏…")),
            )
            .child(
                div()
                    .id("scan-current-path")
                    .min_w_0()
                    .text_xs()
                    .truncate()
                    .text_color(cx.theme().muted_foreground)
                    .child(if path.is_empty() {
                        self.c.text("正在读取游戏安装记录…")
                    } else {
                        path.clone()
                    })
                    .tooltip(move |window, cx| Tooltip::new(path.clone()).build(window, cx)),
            )
            .into_any_element()
    }

    pub(super) fn initial_scan_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.c.dismiss_initial_scan();
        self.dialog_active = true;
        let title = self.t("查找已安装的游戏");
        let message = self.t("建议先扫描一次全盘，将游戏添加到游戏库。");
        let accept = self.t("扫描全盘");
        let later = self.t("稍后");
        let view = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, window, cx| {
            let ok_view = view.clone();
            let close_view = view.clone();
            dialog
                .title(crate::ui_focus::dialog_title(title.clone(), cx))
                .width(window.rem_size() * 26.)
                .centered()
                .close_button(false)
                .child(message.clone())
                .footer(
                    DialogFooter::new()
                        .child(
                            Button::new("initial-scan-later")
                                .small()
                                .label(later.clone())
                                .on_click(|_, window, cx| {
                                    window.dispatch_action(Box::new(Cancel), cx)
                                }),
                        )
                        .child(
                            Button::new("initial-scan-start")
                                .small()
                                .primary()
                                .label(accept.clone())
                                .on_click(|_, window, cx| {
                                    window
                                        .dispatch_action(Box::new(Confirm { secondary: false }), cx)
                                }),
                        ),
                )
                .on_ok(move |_, _, cx| {
                    let _ = ok_view.update(cx, |this, cx| {
                        this.dialog_active = false;
                        this.c.scan(win::drives());
                        cx.notify();
                    });
                    true
                })
                .on_close(move |_, _, cx| {
                    let _ = close_view.update(cx, |this, cx| {
                        this.dialog_active = false;
                        cx.notify();
                    });
                })
        });
    }

    /// Route feedback once to a live foreground window, independent of dialogs.
    pub(super) fn present_feedback(&mut self, cx: &mut Context<Self>) {
        let active = cx.active_window().filter(|w| {
            w.window_id() == self.main_window.window_id()
                || self
                    .runtime_window
                    .is_some_and(|h| h.window_id() == w.window_id())
                || self
                    .settings_window
                    .is_some_and(|h| h.window_id() == w.window_id())
                || self
                    .recommendation_windows
                    .values()
                    .any(|h| h.window_id() == w.window_id())
        });
        let live = cx.windows();
        let target = active
            .or(self
                .presentation_window
                .filter(|old| live.iter().any(|w| w.window_id() == old.window_id())))
            .unwrap_or(self.main_window);
        if self.presentation_window != Some(target) {
            self.presentation_window = Some(target);
            cx.notify();
        }
        let warning = self.c.warning.take();
        let notice = self.c.notice.take();
        let information = self.c.information.take();
        for (kind, text) in [
            (FeedbackKind::Error, warning),
            (FeedbackKind::Success, notice),
            (FeedbackKind::Cancelled, information),
        ] {
            if let Some(text) = text {
                let text = self.c.text(&text);
                let fallback = self.main_window;
                cx.defer(move |cx| {
                    let handle = if cx
                        .windows()
                        .iter()
                        .any(|w| w.window_id() == target.window_id())
                    {
                        target
                    } else {
                        fallback
                    };
                    let _ = handle.update(cx, |_, window, cx| notify(kind, text, window, cx));
                });
            }
        }
    }

    pub(super) fn download_overlay(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        if self.c.closing
            || self
                .presentation_window
                .unwrap_or(self.main_window)
                .window_id()
                != window.window_handle().window_id()
        {
            return div().into_any_element();
        }
        let transfers: Vec<_> = self
            .c
            .tasks
            .current()
            .filter(|(_, task)| task.is_downloading())
            .collect();
        if transfers.is_empty() {
            return div().into_any_element();
        }
        let mut card = div()
            .id("download-float")
            .occlude()
            .v_flex()
            .gap_4()
            .p_4()
            .w(rems(30.))
            .max_w_full()
            .max_h_full()
            .overflow_y_scroll()
            .rounded_lg()
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().popover)
            .text_color(cx.theme().popover_foreground)
            .text_sm()
            .shadow_lg();
        for (update, task) in transfers {
            let idle = rtx_fg_manager::transfer::Progress::default();
            let p = task.transfer.as_ref().unwrap_or(&idle);
            let cancelling = if update {
                self.c.update_cancel.load(Ordering::Relaxed)
            } else {
                self.c.cancel.load(Ordering::Relaxed)
            };
            let (done, bytes) = task.bytes().unwrap_or((0, 0));
            let total = if bytes > 0 {
                format!("{:.1}", bytes as f64 / 1_048_576.)
            } else {
                "?".into()
            };
            let mut rows = div()
                .id(if update {
                    "update-download-files"
                } else {
                    "work-download-files"
                })
                .v_flex()
                .min_h_0()
                .max_h(rems(12.))
                .overflow_y_scroll();
            for row in &task.files {
                let complete = row.phase == Some(rtx_fg_manager::transfer::Phase::Complete);
                let active = row.phase.is_some() && !complete;
                let name = self.c.text(&row.file.name);
                let tooltip = name.clone();
                rows = rows.child(
                    div()
                        .id(SharedString::from(row.file.id.clone()))
                        .h_flex()
                        .items_center()
                        .gap_2()
                        .py_1()
                        .min_w_0()
                        .child(
                            Icon::new(if complete {
                                gpui::assets::IconName::Check
                            } else {
                                gpui::assets::IconName::Download
                            })
                            .size_3p5()
                            .flex_shrink_0()
                            .text_color(if complete {
                                cx.theme().success
                            } else {
                                cx.theme().muted_foreground
                            }),
                        )
                        .child(div().flex_1().min_w_0().truncate().child(name))
                        .child(
                            div()
                                .flex_shrink_0()
                                .text_color(if active {
                                    cx.theme().foreground
                                } else {
                                    cx.theme().muted_foreground
                                })
                                .child(self.t(row.label())),
                        )
                        .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx)),
                );
            }
            card = card.child(
                div()
                    .v_flex()
                    .gap_2()
                    .min_w_0()
                    .child(
                        div()
                            .h_flex()
                            .gap_3()
                            .min_w_0()
                            .child(
                                Icon::new(gpui::assets::IconName::Download)
                                    .size_4()
                                    .text_color(cx.theme().muted_foreground),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .font_weight(FontWeight::MEDIUM)
                                    .whitespace_normal()
                                    .child(task.title.clone()),
                            )
                            .child(
                                self.button(
                                    if update {
                                        "download-update-cancel"
                                    } else {
                                        "download-work-cancel"
                                    },
                                    "取消",
                                    if update {
                                        Command::CancelUpdate
                                    } else {
                                        Command::Cancel
                                    },
                                    cx,
                                )
                                .small()
                                .disabled(cancelling),
                            ),
                    )
                    .child(
                        div()
                            .h_flex()
                            .gap_2()
                            .min_w_0()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(div().flex_1().min_w_0().truncate().child(
                                if task.files.is_empty() {
                                    task.component.clone()
                                } else {
                                    format!(
                                        "{} {}/{}",
                                        self.c.text("已完成"),
                                        task.completed_files(),
                                        task.files.len()
                                    )
                                },
                            ))
                            .child(self.t(if cancelling {
                                "正在取消…"
                            } else {
                                &task.stage
                            })),
                    )
                    .child(
                        Progress::new(if update {
                            "download-update-progress"
                        } else {
                            "download-work-progress"
                        })
                        .accessibility_label(self.t(&task.stage))
                        .small()
                        .value(task.fraction().unwrap_or(0.))
                        .loading(task.fraction().is_none()),
                    )
                    .when(!task.files.is_empty(), |view| view.child(rows.text_sm()))
                    .child(
                        div()
                            .h_flex()
                            .flex_wrap()
                            .gap_2()
                            .justify_between()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(format!("{:.1} / {} MiB", done as f64 / 1_048_576., total))
                            .child(format!(
                                "{:.1} MiB/s",
                                p.bytes_per_second as f64 / 1_048_576.
                            ))
                            .child(p.source.clone()),
                    ),
            );
        }
        // Only the card occludes input. No modal/focus-trap state is created.
        deferred(
            div()
                .absolute()
                .inset_0()
                .p_4()
                .flex()
                .items_center()
                .justify_center()
                .child(card),
        )
        .priority(2)
        .into_any_element()
    }
}
