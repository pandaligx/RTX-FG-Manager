//! Standard GPUI Kit settings navigation; existing Manager commands own changes.
use super::*;
use gpui_component::{
    group_box::GroupBoxVariant,
    setting::{SettingField, SettingGroup, SettingItem, SettingPage, Settings},
};

struct SettingsView {
    owner: WeakEntity<Manager>,
}

impl Render for SettingsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.owner
            .update(cx, |owner, cx| {
                div()
                    .v_flex()
                    .size_full()
                    .min_w_0()
                    .min_h_0()
                    .font_family("Microsoft YaHei UI")
                    .text_sm()
                    .bg(cx.theme().background)
                    .text_color(cx.theme().foreground)
                    .child(TitleBar::new().child(div().pl_3().child(owner.t("设置"))))
                    .child(
                        div()
                            .flex_1()
                            .min_h_0()
                            .min_w_0()
                            .child(owner.settings(window, cx)),
                    )
                    .when(owner.c.save_error.is_some(), |d| {
                        d.child(
                            div()
                                .h_flex()
                                .gap_2()
                                .p_3()
                                .child(
                                    div()
                                        .flex_1()
                                        .text_color(cx.theme().warning)
                                        .child(owner.t("设置未保存，请检查目录权限后重试。")),
                                )
                                .child(owner.button(
                                    "settings-retry-save",
                                    "重试",
                                    Command::RetrySave,
                                    cx,
                                )),
                        )
                    })
                    .when(owner.c.closing, |d| {
                        d.child(owner.t(owner.c.close_status()))
                    })
                    .into_any_element()
            })
            .unwrap_or_else(|_| div().into_any_element())
    }
}

impl Manager {
    pub(super) fn clear_settings_window(&mut self, cx: &mut Context<Self>) {
        self.settings_window = None;
        self.theme_preview = None;
        // Select subscriptions are window-bound. Recreate them on reopening,
        // rather than dispatching changes into a closed native window.
        cx.notify();
    }

    pub(super) fn reconcile_settings_window(&mut self, cx: &mut Context<Self>) {
        if let Some(handle) = self.settings_window
            && !cx
                .windows()
                .iter()
                .any(|w| w.window_id() == handle.window_id())
        {
            self.clear_settings_window(cx);
        }
    }

    pub(super) fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.reconcile_settings_window(cx);
        if let Some(handle) = self.settings_window {
            cx.defer(move |cx| {
                let _ = handle.update(cx, |_, window, _| window.activate_window());
            });
            return;
        }
        if self.dialog_active {
            return;
        }
        let work = window
            .display(cx)
            .map(|d| d.visible_bounds())
            .unwrap_or(window.bounds());
        let options = WindowOptions {
            titlebar: Some(TitlebarOptions {
                title: Some(self.t("设置")),
                appears_transparent: true,
                ..Default::default()
            }),
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered_at(
                work.center(),
                size(
                    (window.rem_size() * 64.).min(work.size.width),
                    (window.rem_size() * 46.).min(work.size.height),
                ),
            ))),
            window_min_size: Some(size(
                px(720.).min(work.size.width),
                px(460.).min(work.size.height),
            )),
            ..Default::default()
        };
        let owner = cx.entity();
        self.dialog_active = true;
        cx.defer(move |cx| {
            let result_owner = owner.clone();
            let result = cx.open_window(options, move |window, cx| {
                let close = owner.downgrade();
                window.on_window_should_close(cx, move |window, cx| {
                    close
                        .update(cx, |owner, cx| {
                            if window.has_active_dialog(cx) {
                                return false;
                            }
                            owner.clear_settings_window(cx);
                            true
                        })
                        .unwrap_or(true)
                });
                let event_window = window.window_handle();
                owner.update(cx, |owner, cx| {
                    owner._font_subscription =
                        cx.subscribe(&owner.font_slider, move |_, _, event, cx| {
                            if let SliderEvent::Change(value) = event {
                                let size = value.end().round() as u8;
                                let owner = cx.entity().downgrade();
                                cx.defer(move |cx| {
                                    let _ = event_window.update(cx, |_, window, cx| {
                                        let _ = owner.update(cx, |owner, cx| {
                                            owner.set_font_size(size, window, cx)
                                        });
                                    });
                                });
                            }
                        });
                });
                let view = cx.new(|cx| {
                    cx.observe(&owner, |_, _, cx| cx.notify()).detach();
                    cx.observe_global::<Theme>(|_, cx| cx.notify()).detach();
                    crate::ui_focus::observe(cx);
                    cx.spawn_in(window, async move |this, cx| {
                        loop {
                            smol::Timer::after(Duration::from_millis(80)).await;
                            if cx
                                .update(|window, cx| {
                                    this.update(cx, |this: &mut SettingsView, cx| {
                                        let _ = this.owner.update(cx, |owner, cx| {
                                            if !owner.c.closing {
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
                    SettingsView {
                        owner: owner.downgrade(),
                    }
                });
                cx.new(|cx| Root::new(view, window, cx))
            });
            result_owner.update(cx, |owner, cx| {
                owner.dialog_active = false;
                match result {
                    Ok(handle) => owner.settings_window = Some(handle.into()),
                    Err(error) => owner.c.warning = Some(format!("{error:#}")),
                }
                cx.notify();
            });
        });
    }

    fn about_link(&self, label: &str, url: &'static str, cx: &Context<Self>) -> AnyElement {
        gpui::base::Link::new(url)
            .href(url)
            .accessibility_label(self.t(label))
            .on_activate(cx.listener(move |this, _, window, cx| {
                this.act(Command::Open(url.into()), window, cx);
            }))
            .text_sm()
            .text_color(cx.theme().link)
            .cursor_pointer()
            .hover(|d| d.underline())
            .when(crate::ui_focus::visible(cx), |d| {
                d.focus(|s| s.bg(cx.theme().muted))
            })
            .child(self.t(label))
            .into_any_element()
    }

    fn settings_links(&self, cx: &Context<Self>) -> AnyElement {
        let groups: &[(&str, &[(&str, &str)])] = &[
            ("作者", &[("小南瓜", "https://lgxng.cn/")]),
            (
                "项目主页",
                &[
                    ("GitHub", "https://github.com/pandaligx/RTX-FG-Manager"),
                    ("GitCode", "https://gitcode.com/gcw_qWIjmve7/RTX-FG-Manager"),
                ],
            ),
            (
                "作者主页",
                &[
                    ("GitHub", "https://github.com/pandaligx"),
                    ("哔哩哔哩", "https://b23.tv/5mHCHFn"),
                ],
            ),
            (
                "补丁项目",
                &[
                    (
                        "Dlssg_for_sm86",
                        "https://github.com/sdli1995/dlssg_for_sm86",
                    ),
                    ("MFG-Unlocker", "https://github.com/dashdogy/MFG-Unlocker"),
                    ("RTX-Encore", "https://github.com/SilyNoMeta/rtx-encore"),
                ],
            ),
            (
                "界面组件",
                &[("GPUI Kit", "https://github.com/longbridge/gpui-kit")],
            ),
            (
                "鸣谢",
                &[
                    (
                        "大大大怪将军阁下（测试支持）",
                        "https://space.bilibili.com/608531525",
                    ),
                    ("云外逸声", "https://space.bilibili.com/256887068"),
                ],
            ),
        ];
        div()
            .v_flex()
            .w_full()
            .gap_3()
            .pt_3()
            .border_t_1()
            .border_color(cx.theme().border)
            .children(groups.iter().map(|(label, links)| {
                div()
                    .h_flex()
                    .items_start()
                    .gap_4()
                    .min_w_0()
                    .child(
                        div()
                            .w(rems(7.))
                            .flex_shrink_0()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(self.t(label)),
                    )
                    .child(
                        div()
                            .h_flex()
                            .flex_1()
                            .min_w_0()
                            .flex_wrap()
                            .gap_x_4()
                            .gap_y_2()
                            .children(
                                links
                                    .iter()
                                    .map(|(label, url)| self.about_link(label, url, cx)),
                            ),
                    )
            }))
            .into_any_element()
    }

    // The Settings component retains search/navigation state. Custom fields use
    // weak access to the existing owner, avoiding a second preferences model.
    fn setting_field<F>(&self, label: &str, cx: &Context<Self>, render: F) -> SettingItem
    where
        F: Fn(&Self, &mut Window, &mut Context<Self>) -> AnyElement + 'static,
    {
        let owner = cx.entity().downgrade();
        SettingItem::new(
            self.t(label),
            SettingField::render(move |_, window, cx| {
                owner
                    .update(cx, |this, cx| render(this, window, cx))
                    .unwrap_or_else(|_| div().into_any_element())
            }),
        )
    }

    fn setting_content<F>(&self, keywords: &[&str], cx: &Context<Self>, render: F) -> SettingItem
    where
        F: Fn(&Self, &mut Window, &mut Context<Self>) -> AnyElement + 'static,
    {
        let owner = cx.entity().downgrade();
        SettingItem::render(move |_, window, cx| {
            owner
                .update(cx, |this, cx| render(this, window, cx))
                .unwrap_or_else(|_| div().into_any_element())
        })
        .keywords(keywords.iter().map(|s| self.t(s)))
    }

    fn setting_toggle(
        &self,
        key: &'static str,
        label: &'static str,
        default: bool,
        cx: &Context<Self>,
    ) -> SettingItem {
        self.setting_field(label, cx, move |this, _, cx| {
            Switch::new(key)
                .focus_ring(crate::ui_focus::visible(cx))
                .accessibility_label(this.t(label))
                .checked(this.c.boolean(key, default))
                .on_click(cx.listener(move |this, yes, window, cx| {
                    this.act(Command::Boolean(key.into(), *yes), window, cx)
                }))
                .into_any_element()
        })
    }

    pub(super) fn settings(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let appearance = SettingGroup::new().gap_3().items([
            self.setting_field("语言", cx, |this, window, cx| {
                let selected = this.c.choice("language", "system");
                let choices = i18n::LANGUAGES
                    .iter()
                    .map(|(code, label)| {
                        (
                            if *code == "system" {
                                this.t("跟随系统")
                            } else {
                                (*label).into()
                            },
                            Command::Language((*code).into()),
                            *code == selected,
                        )
                    })
                    .collect::<Vec<_>>();
                let label = choices
                    .iter()
                    .find(|c| c.2)
                    .map(|c| c.0.clone())
                    .unwrap_or_else(|| this.t("跟随系统"));
                div()
                    .w(rems(14.))
                    .child(this.menu("language", label, choices, false, window, cx))
                    .into_any_element()
            }),
            self.setting_field("主题", cx, |this, _, cx| {
                let label = if this.c.choice("theme", "跟随系统") == "跟随系统" {
                    this.t("跟随系统")
                } else {
                    cx.theme().theme_name().to_string().into()
                };
                this.button("settings-theme", "更换主题…", Command::Themes, cx)
                    .label(format!("{label}…"))
                    .icon(IconName::Palette)
                    .w(rems(14.))
                    .into_any_element()
            }),
            self.setting_field("界面动效", cx, |this, window, cx| {
                let selected = this.c.choice("ui_motion", "on");
                let choices = [("system", "跟随系统"), ("on", "开启"), ("off", "关闭")]
                    .into_iter()
                    .map(|(key, label)| (this.t(label), Command::Motion(key), selected == key))
                    .collect::<Vec<_>>();
                let label = choices
                    .iter()
                    .find(|c| c.2)
                    .map(|c| c.0.clone())
                    .unwrap_or_else(|| this.t("开启"));
                div()
                    .w(rems(14.))
                    .child(this.menu("ui-motion", label, choices, false, window, cx))
                    .into_any_element()
            }),
            self.setting_field("字体大小", cx, |this, _, cx| {
                let size = preferred_font_size(&this.c.state) as u8;
                div()
                    .v_flex()
                    .w_full()
                    .gap_2()
                    .child(
                        div()
                            .h_flex()
                            .w_full()
                            .gap_3()
                            .child(
                                this.button(
                                    "font-smaller",
                                    "缩小字体",
                                    Command::FontSize(size.saturating_sub(1)),
                                    cx,
                                )
                                .label("A−")
                                .accessibility_label(this.t("缩小字体"))
                                .tooltip(this.t("缩小字体"))
                                .disabled(size <= 12),
                            )
                            .child(Slider::new(&this.font_slider).flex_1().min_w_0())
                            .child(
                                this.button(
                                    "font-larger",
                                    "放大字体",
                                    Command::FontSize(size + 1),
                                    cx,
                                )
                                .label("A+")
                                .accessibility_label(this.t("放大字体"))
                                .tooltip(this.t("放大字体"))
                                .disabled(size >= 18),
                            )
                            .child(div().w(rems(4.)).text_sm().child(format!("{size} px")))
                            .child(
                                this.button("font-reset", "恢复默认", Command::FontSize(13), cx)
                                    .ghost(),
                            ),
                    )
                    .into_any_element()
            })
            .layout(Axis::Vertical)
            .description(self.t("即时生效，控件与间距同步缩放")),
        ]);
        let library = SettingGroup::new().gap_3().items([
            self.setting_toggle("chinese_game_names", "中文界面优先显示中文游戏名", true, cx)
                .description(self.t("名称优先读取安装记录；无法确认时保留原名，可手动改名。")),
            self.setting_content(&["游戏库", "更新名称库"], cx, |this, _, cx| {
                div()
                    .v_flex()
                    .gap_2()
                    .text_sm()
                    .child(
                        div()
                            .h_flex()
                            .gap_3()
                            .flex_wrap()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(this.c.text(&format!(
                                        "离线名称库：{0} 条 · {1}",
                                        this.c.game_names.len(),
                                        this.c.game_names.revision()
                                    ))),
                            )
                            .child(
                                this.button(
                                    "update-game-names",
                                    "更新名称库",
                                    Command::UpdateGameNames,
                                    cx,
                                )
                                .disabled(this.c.names_busy || this.c.read_only),
                            ),
                    )
                    .when(!this.c.names_status.is_empty(), |d| {
                        d.child(this.t(&this.c.names_status))
                    })
                    .into_any_element()
            }),
        ]);
        let updates = SettingGroup::new().gap_3().items([
            self.setting_field("当前版本", cx, |_, _, _| {
                div()
                    .text_sm()
                    .child(format!("v{VERSION}"))
                    .into_any_element()
            }),
            self.setting_field("下载来源", cx, |this, window, cx| {
                let selected = this.c.choice("download_source", "domestic");
                let label = this.t(if selected == "github" {
                    "GitHub 优先"
                } else {
                    "GitCode 优先"
                });
                let choices = [("domestic", "GitCode 优先"), ("github", "GitHub 优先")]
                    .into_iter()
                    .map(|(key, label)| {
                        (this.t(label), Command::Source(key.into()), key == selected)
                    })
                    .collect();
                div()
                    .w(rems(14.))
                    .child(this.menu("source", label, choices, false, window, cx))
                    .into_any_element()
            }),
            self.setting_toggle("auto_update", "自动检查更新", true, cx),
            self.setting_toggle("auto_download", "自动下载更新", false, cx),
            self.setting_content(
                &["软件更新", "检查更新", "更新日志", "下载更新"],
                cx,
                |this, window, cx| this.update_panel(window, cx),
            ),
        ]);
        let maintenance = SettingGroup::new().gap_3().item(
            self.setting_field("清理缓存", cx, |this, _, cx| {
                this.button("clear-cache", "清理缓存", Command::ClearCache, cx)
                    .disabled(this.c.busy || this.c.update_busy).into_any_element()
            }).description(self.t("清理下载、更新暂存及未使用的已识别组件缓存；保留游戏列表、参数和归属记录。需要时重新下载或生成缓存。"))
        );
        let about = SettingGroup::new().gap_3().items([
            self.setting_content(
                &["关于", "当前版本", "RTX-FG-Manager"],
                cx,
                |this, _, cx| {
                    div()
                        .h_flex()
                        .items_center()
                        .gap_4()
                        .py_2()
                        .child(Self::brand_logo(rems(4.), cx))
                        .child(
                            div()
                                .v_flex()
                                .gap_1()
                                .min_w_0()
                                .child(
                                    div()
                                        .text_base()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child("RTX-FG-Manager"),
                                )
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(this.t("Windows 帧生成补丁管理器")),
                                )
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(format!("v{VERSION}")),
                                ),
                        )
                        .into_any_element()
                },
            ),
            self.setting_content(
                &["关于与致谢", "GitHub", "小南瓜"],
                cx,
                |this, _, cx| this.settings_links(cx),
            ),
        ]);
        let page = |label: &str, icon: IconName, group: SettingGroup| {
            SettingPage::new(self.t(label))
                .icon(icon)
                .resettable(false)
                .group(group)
        };
        div()
            .id("manager-settings")
            .size_full()
            .min_w_0()
            .min_h_0()
            .bg(cx.theme().background)
            .child(
                Settings::new("manager-settings-pages")
                    .small()
                    .sidebar_width(window.rem_size() * 14.)
                    .sidebar_size_range(window.rem_size() * 11. ..window.rem_size() * 22.)
                    .with_group_variant(GroupBoxVariant::Outline)
                    .pages([
                        page("外观", IconName::Palette, appearance)
                            .description(self.t("外观与偏好更改后自动保存")),
                        page("游戏库", IconName::Folder, library),
                        page("软件更新", IconName::RefreshCw, updates),
                        page("缓存维护", IconName::Settings, maintenance),
                        page("关于", IconName::Info, about),
                    ]),
            )
            .into_any_element()
    }
}
