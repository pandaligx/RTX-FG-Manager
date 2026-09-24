//! Native GPUI frontend. Disk, registry, scanning and networking stay in workers.
use crate::controller::{Controller, Event};
use anyhow::Result;
use gpui::{prelude::*, *};
use gpui_component::{
    accordion::Accordion,
    alert::Alert,
    button::{Button, ButtonVariant, ButtonVariants, DropdownButton},
    checkbox::Checkbox,
    dialog::{CancelDialog, ConfirmDialog, DialogButtonProps, DialogFooter},
    input::{Input, InputEvent, InputState},
    menu::{ContextMenuExt, PopupMenuItem},
    radio::{Radio, RadioGroup},
    select::{Select, SelectEvent, SelectItem, SelectState},
    sidebar::{Sidebar, SidebarItem, SidebarMenuItem},
    switch::Switch,
    tag::{Tag, TagVariant},
    theme::ThemeMode,
    tooltip::Tooltip,
    *,
};
use rtx_fg_manager::{VERSION, core, gpu_alias, hags, i18n, preferences, win};
use serde_json::{Value, json};
use std::{
    cell::RefCell,
    collections::HashMap,
    path::PathBuf,
    sync::atomic::Ordering,
    time::{Duration, Instant},
};

#[derive(Clone, PartialEq)]
enum Command {
    Page(u8),
    Help,
    ConfigHelp,
    ClearCache,
    Scan,
    ScanRoots(Vec<PathBuf>),
    Pick(bool),
    Refresh,
    RefreshGpu,
    All(bool),
    Game(String, bool),
    GameOption(String, String),
    ApplyPreset,
    RetrySave,
    Focus(String),
    Proxy(String, bool),
    Scheme(usize),
    Series(usize),
    Language(String),
    Theme(String),
    Source(String),
    CloudSource(String),
    Boolean(String, bool),
    CheckUpdate,
    Download,
    Cancel,
    CancelUpdate,
    Graphics,
    Device(usize),
    Alias(usize),
    Confirm(String),
    Commit(String),
    Patch(bool),
    Open(String),
    CopyLogs,
}
#[derive(Clone)]
struct ChoiceItem {
    label: SharedString,
    value: usize,
}
impl SelectItem for ChoiceItem {
    type Value = usize;
    fn title(&self) -> SharedString {
        self.label.clone()
    }
    fn value(&self) -> &usize {
        &self.value
    }
}
struct ChoiceBinding {
    state: Entity<SelectState<Vec<ChoiceItem>>>,
    items: Vec<(SharedString, Command, bool)>,
    _subscription: Subscription,
}
#[derive(Clone)]
struct NavigationItem {
    label: SharedString,
    item: SidebarMenuItem,
}
impl Collapsible for NavigationItem {
    fn is_collapsed(&self) -> bool {
        self.item.is_collapsed()
    }
    fn collapsed(mut self, collapsed: bool) -> Self {
        self.item = self.item.collapsed(collapsed);
        self
    }
}
impl SidebarItem for NavigationItem {
    fn render(
        self,
        id: impl Into<ElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> impl IntoElement {
        let id = id.into();
        div()
            .id(id.clone())
            .w_full()
            .tooltip(move |window, cx| Tooltip::new(self.label.clone()).build(window, cx))
            .child(self.item.render(id, window, cx))
    }
}
pub struct Manager {
    choices: RefCell<HashMap<&'static str, ChoiceBinding>>,
    c: Controller,
    search: Entity<InputState>,
    _subscription: Subscription,
    scroll: UniformListScrollHandle,
    log_scroll: ScrollHandle,
    filtered: Vec<usize>,
    theme_choice: String,
    language: String,
    system_poll: Instant,
    started: Instant,
    last_tick: Instant,
    max_gap_ms: f64,
    ticks: u64,
    frames: u64,
    first_render_ms: Option<f64>,
    smoke_step: u8,
    update_ready_sent: bool,
    dialog_active: bool,
}
impl Manager {
    fn new(
        data: PathBuf,
        state: Value,
        error: Option<String>,
        smoke: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let c = Controller::new(data, state, error, smoke);
        let search = cx.new(|cx| InputState::new(window, cx).placeholder(c.text("搜索游戏")));
        let subscription = cx.subscribe_in(&search, window, |this, state, event, _, cx| {
            if matches!(event, InputEvent::Change) {
                this.c.search = state.read(cx).value().to_string();
                this.filter();
                cx.notify();
            }
        });
        let theme_choice = c.choice("theme", "跟随系统");
        let language = c.tr.language.clone();
        let mut view = Self {
            choices: RefCell::new(HashMap::new()),
            c,
            search,
            _subscription: subscription,
            scroll: UniformListScrollHandle::new(),
            log_scroll: ScrollHandle::new(),
            filtered: Vec::new(),
            theme_choice,
            language,
            system_poll: Instant::now(),
            started: Instant::now(),
            last_tick: Instant::now(),
            max_gap_ms: 0.,
            ticks: 0,
            frames: 0,
            first_render_ms: None,
            smoke_step: 0,
            update_ready_sent: false,
            dialog_active: false,
        };
        view.filter();
        view.theme(window, cx);
        cx.spawn_in(window, async move |this, cx| {
            loop {
                smol::Timer::after(Duration::from_millis(20)).await;
                if this
                    .update_in(cx, |this, window, cx| this.tick(window, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        view
    }
    fn t(&self, s: &str) -> SharedString {
        self.c.text(s).into()
    }
    fn filter(&mut self) {
        let query = self.c.search.to_lowercase();
        self.filtered = self
            .c
            .games
            .iter()
            .enumerate()
            .filter(|(_, g)| g.exe.to_lowercase().contains(&query))
            .map(|(i, _)| i)
            .collect();
    }
    fn theme(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mode = match self.c.choice("theme", "跟随系统").as_str() {
            "深色" => ThemeMode::Dark,
            "浅色" => ThemeMode::Light,
            _ => {
                if win::dark() {
                    ThemeMode::Dark
                } else {
                    ThemeMode::Light
                }
            }
        };
        if cx.theme().mode != mode {
            Theme::change(mode, Some(window), cx);
        }
        brand_theme(cx);
        gpui_component::set_locale(match self.c.tr.language.as_str() {
            "zh-CN" => "zh-CN",
            "ja" => "ja",
            "ko" => "ko",
            "ru" => "ru",
            _ => "en",
        });
        cx.notify();
    }
    fn act(&mut self, command: Command, window: &mut Window, cx: &mut Context<Self>) {
        if self.c.closing {
            return;
        }
        match command {
            Command::Page(p) => self.c.page = p,
            Command::Help => self.help(window, cx),
            Command::ConfigHelp => self.config_help(window, cx),
            Command::ClearCache => self.c.clear_cache(),
            Command::Refresh => {
                if let Some(exe) = self.c.focus.clone() {
                    self.c.evidence.remove(&exe);
                    self.c.inspect(exe);
                }
                self.c.refresh();
            }
            Command::RefreshGpu => self.c.channel.job(|c| {
                c.send(Event::Devices(gpu_alias::enumerate()?));
                Ok(())
            }),
            Command::Scan => {
                let roots = self.c.state["roots"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(Value::as_str)
                            .map(PathBuf::from)
                            .collect()
                    })
                    .filter(|r: &Vec<PathBuf>| !r.is_empty())
                    .unwrap_or_else(win::drives);
                self.c.scan(roots)
            }
            Command::ScanRoots(r) => self.c.scan(r),
            Command::Pick(folder) => self.c.pick(folder),
            Command::All(yes) => {
                self.c.selected = if yes {
                    self.c.games.iter().map(|g| g.exe.clone()).collect()
                } else {
                    Default::default()
                }
            }
            Command::Game(exe, yes) => {
                if yes {
                    self.c.selected.insert(exe.clone());
                } else {
                    self.c.selected.remove(&exe);
                }
            }
            Command::GameOption(key, value) => {
                if let Some(exe) = self.c.focus.clone() {
                    self.c.set_game_option(&exe, &key, &value);
                }
            }
            Command::Focus(exe) => {
                self.c.focus = Some(exe.clone());
                self.c.inspect(exe);
            }
            Command::Proxy(name, yes) => {
                if !self.c.busy {
                    let mut p = self.c.proxies();
                    if yes && !p.contains(&name) {
                        if self.c.catalog.scheme_policies[&self.c.cloud_scheme()]
                            .max_selected_proxies
                            == 1
                        {
                            p.clear();
                        }
                        p.push(name)
                    } else if !yes {
                        p.retain(|p| *p != name)
                    }
                    if p.is_empty() {
                        p.push("version.dll".into())
                    }
                    if let Ok(p) = core::normalize_proxies(&p) {
                        self.c.state["proxies"] = json!(p);
                        self.c.save();
                    }
                }
            }
            Command::Scheme(i) => {
                let (_, series) = self.profile();
                self.profile_set(i, series)
            }
            Command::Series(i) => {
                let (scheme, _) = self.profile();
                self.profile_set(scheme, i)
            }
            Command::Language(s) => {
                self.c.language(&s);
                self.language = self.c.tr.language.clone();
                self.search.update(cx, |state, cx| {
                    state.set_placeholder(self.c.text("搜索游戏"), window, cx)
                });
                self.theme(window, cx)
            }
            Command::Theme(s) => {
                self.c.state["theme"] = json!(s);
                self.c.state["theme_schema"] = json!(1);
                self.c.save();
                self.theme(window, cx)
            }
            Command::ApplyPreset => self.c.apply_focused_parameters(),
            Command::RetrySave => self.c.save(),
            Command::Source(s) | Command::CloudSource(s) => {
                self.c.state["download_source"] = json!(s);
                self.c.state["cloud_source"] = json!(s);
                self.c.state["update_source"] =
                    json!(if s == "github" { "github" } else { "gitee" });
                self.c.save();
            }
            Command::Boolean(k, v) => {
                self.c.state[&k] = json!(v);
                self.c.save()
            }
            Command::CheckUpdate => self.c.check_update(false),
            Command::Download => self.c.download_update(),
            Command::Cancel => self.c.cancel.store(true, Ordering::Relaxed),
            Command::CancelUpdate => self.c.cancel_update(),
            Command::Graphics => self.c.channel.job(|_| hags::settings()),
            Command::Device(i) => self.c.device_index = i,
            Command::Alias(i) => self.c.alias_index = i,
            Command::Confirm(action) => {
                if action == "remove" && self.c.can_forget_focused_without_confirmation() {
                    self.commit(&action);
                } else {
                    self.c.confirm = Some(action);
                }
            }
            Command::Commit(action) => self.commit(&action),
            Command::Patch(clean) => self.c.request_patch(clean),
            Command::Open(s) => self.c.channel.job(move |_| win::open(&s)),
            Command::CopyLogs => cx.write_to_clipboard(ClipboardItem::new_string(
                self.c.logs.iter().cloned().collect::<Vec<_>>().join("\n"),
            )),
        }
        cx.notify();
    }
    fn profile(&self) -> (usize, usize) {
        (
            self.c
                .catalog
                .schemes()
                .iter()
                .position(|p| p.scheme_id == self.c.cloud_scheme())
                .unwrap_or(0),
            self.c.cloud_series(),
        )
    }
    fn profile_set(&mut self, scheme: usize, series: usize) {
        if self.c.busy {
            return;
        }
        let schemes = self.c.catalog.schemes();
        let Some(p) = schemes.get(scheme) else {
            return;
        };
        let policy = &self.c.catalog.scheme_policies[&p.scheme_id];
        let suffix = if series == 0 { "20" } else { "30" };
        let backend = self
            .c
            .catalog
            .packages
            .iter()
            .filter(|entry| entry.scheme_id == p.scheme_id)
            .flat_map(|entry| entry.backends.iter())
            .find(|b| b.ends_with(suffix))
            .unwrap_or(&p.backends[0])
            .clone();
        let id = p.scheme_id.clone();
        let single = policy.max_selected_proxies == 1;
        self.c.state["cloud_scheme"] = json!(id);
        self.c.state["backend"] = json!(backend);
        self.c.state["cloud_series"] = json!(series);
        self.c.state["backend_schema"] = json!(1);
        if single
            || self
                .c
                .proxies()
                .iter()
                .any(|n| !self.c.catalog.proxies(&id).contains(n))
        {
            self.c.state["proxies"] = json!(["version.dll"])
        }
        self.c.save();
    }
    fn commit(&mut self, action: &str) {
        match action {
            "deploy_batch" | "clean_batch" => self.c.confirm_patch(),
            "hags" => self.c.channel.job(|_| hags::settings()),
            "remove" | "remove_checked" => {
                let paths = if action == "remove_checked" {
                    self.c.selected.clone()
                } else {
                    self.c.focus.iter().cloned().collect()
                };
                self.c.forget(&paths);
                self.filter();
            }
            "update" => self.c.install_update(),
            "alias" | "restore" => {
                if self.c.busy {
                    return;
                }
                if let Some(target) = self.c.devices.get(self.c.device_index).cloned() {
                    let name = (action == "alias").then_some(gpu_alias::NAMES[self.c.alias_index]);
                    self.c.busy = true;
                    self.c.critical = true;
                    self.c.channel.operation(move |c| {
                        match gpu_alias::apply_with_elevation(target, name) {
                            Ok(s) => {
                                c.send(Event::Log(s));
                                c.send(Event::Notice("修改和还原后请重启电脑。".into()));
                                if let Ok(d) = gpu_alias::enumerate() {
                                    c.send(Event::Devices(d));
                                }
                            }
                            Err(e) => c.send(Event::Warning(e.to_string())),
                        }
                        Ok(())
                    });
                }
            }
            _ => {}
        }
    }
    fn button(
        &self,
        id: impl Into<ElementId>,
        label: &str,
        action: Command,
        cx: &Context<Self>,
    ) -> Button {
        Button::new(id)
            .label(self.t(label))
            .on_click(cx.listener(move |this, _, window, cx| this.act(action.clone(), window, cx)))
    }
    fn icon(name: IconName) -> Icon {
        Icon::new(name).size(px(20.))
    }
    // Keep Select entities and their subscriptions alive across renders. Only refresh
    // their data when translations, device enumeration or saved choices change.
    fn menu(
        &self,
        id: &'static str,
        label: SharedString,
        items: Vec<(SharedString, Command, bool)>,
        disabled: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let options = || {
            items
                .iter()
                .enumerate()
                .map(|(value, (label, _, _))| ChoiceItem {
                    label: label.clone(),
                    value,
                })
                .collect::<Vec<_>>()
        };
        let selected = items
            .iter()
            .position(|(_, _, checked)| *checked)
            .map(IndexPath::new);
        let mut choices = self.choices.borrow_mut();
        let binding = choices.entry(id).or_insert_with(|| {
            let state = cx.new(|cx| SelectState::new(options(), selected, window, cx));
            let subscription =
                cx.subscribe_in(&state, window, move |this, _, event, window, cx| {
                    let SelectEvent::Confirm(value) = event;
                    let action = value.and_then(|index| {
                        this.choices
                            .borrow()
                            .get(id)
                            .and_then(|b| b.items.get(index))
                            .map(|(_, action, _)| action.clone())
                    });
                    if let Some(action) = action {
                        this.act(action, window, cx);
                    }
                });
            ChoiceBinding {
                state,
                items: items.clone(),
                _subscription: subscription,
            }
        });
        if binding.items != items {
            binding.state.update(cx, |state, cx| {
                state.set_items(options(), window, cx);
                state.set_selected_index(selected, window, cx);
            });
            binding.items = items;
        }
        Select::new(&binding.state)
            .placeholder(label)
            .disabled(disabled)
            .w_full()
            .menu_max_h(px(300.))
            .into_any_element()
    }
    fn nav(&self, window: &Window, cx: &Context<Self>) -> AnyElement {
        let compact = window.viewport_size().width < px(880.);
        let mut sidebar = Sidebar::new("main-sidebar").collapsed(true);
        for (page, icon, label) in [
            (0, IconName::LayoutDashboard, "游戏"),
            (1, IconName::Cpu, "显卡"),
            (2, IconName::Settings, "设置"),
            (3, IconName::PanelRight, "补丁设置"),
        ] {
            if page == 3 && !compact {
                continue;
            }
            let label = self.t(label);
            sidebar = sidebar.child(NavigationItem {
                label: label.clone(),
                item: SidebarMenuItem::new(label)
                    .icon(Self::icon(icon).size(px(26.)))
                    .active(self.c.page == page)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.act(Command::Page(page), window, cx)
                    })),
            });
        }
        sidebar.h_full().flex_shrink_0().into_any_element()
    }
    fn header(&self, window: &Window, cx: &Context<Self>) -> AnyElement {
        let compact = window.viewport_size().width < px(1000.);
        div()
            .flex()
            .w_full()
            .gap_2()
            .pb_2()
            .when(compact, |d| d.flex_col())
            .when(!compact, |d| d.items_center())
            .child(
                div()
                    .h_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_2()
                    .child(Self::icon(IconName::Cpu).size(px(22.)))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(self.c.gpu.clone()),
                    ),
            )
            .child(
                div()
                    .h_flex()
                    .flex_wrap()
                    .gap_2()
                    .justify_end()
                    .items_center()
                    .when(compact, |d| d.w_full())
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(self.t("硬件加速 GPU 计划")),
                    )
                    .child(
                        self.button("graphics-home", "图形设置", Command::Graphics, cx)
                            .icon(IconName::ExternalLink),
                    )
                    .child(
                        self.button("help", "使用说明", Command::Help, cx)
                            .icon(Self::icon(IconName::BookOpen)),
                    ),
            )
            .into_any_element()
    }
    fn scan_button(&self, cx: &Context<Self>) -> AnyElement {
        let view = cx.entity().downgrade();
        let folder = self.t("选择目录扫描");
        let all = self.t("全盘扫描");
        DropdownButton::new("scan-split")
            .button(
                self.button("scan", "扫描", Command::Scan, cx)
                    .icon(Self::icon(IconName::FolderOpen))
                    .w(px(102.)),
            )
            .disabled(self.c.busy)
            .dropdown_menu(move |mut menu, _, _| {
                for (label, action) in std::iter::once((folder.clone(), Command::Pick(true)))
                    .chain(
                        win::drives()
                            .into_iter()
                            .map(|p| (p.display().to_string().into(), Command::ScanRoots(vec![p]))),
                    )
                    .chain(std::iter::once((
                        all.clone(),
                        Command::ScanRoots(win::drives()),
                    )))
                {
                    let view = view.clone();
                    menu = menu.item(PopupMenuItem::new(label).on_click(move |_, window, cx| {
                        let _ = view.update(cx, |this, cx| this.act(action.clone(), window, cx));
                    }));
                }
                menu
            })
            .into_any_element()
    }
    fn library(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut card = div()
            .v_flex()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .gap_3()
            .p_4()
            .rounded_lg()
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .child(
                div()
                    .h_flex()
                    .justify_between()
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(self.t("游戏库")),
                    )
                    .child(
                        div()
                            .h_flex()
                            .gap_2()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(self.c.games.len().to_string())
                            .child(
                                self.button(
                                    "remove-checked",
                                    "移除勾选",
                                    Command::Confirm("remove_checked".into()),
                                    cx,
                                )
                                .small()
                                .disabled(self.c.busy || self.c.selected.is_empty()),
                            ),
                    ),
            )
            .child(
                div()
                    .h_flex()
                    .gap_2()
                    .flex_wrap()
                    .child(self.scan_button(cx))
                    .child(
                        self.button("add", "添加游戏", Command::Pick(false), cx)
                            .icon(Self::icon(IconName::Plus))
                            .w(px(124.))
                            .disabled(self.c.busy),
                    )
                    .child(
                        self.button("refresh", "刷新", Command::Refresh, cx)
                            .icon(Icon::default().path("app/refresh.svg").size(px(20.)))
                            .w(px(124.))
                            .disabled(self.c.busy),
                    ),
            )
            .child(
                div()
                    .h_flex()
                    .gap_3()
                    .child(
                        Checkbox::new("all")
                            .label(self.t("全选"))
                            .checked(
                                !self.c.games.is_empty()
                                    && self.c.selected.len() == self.c.games.len(),
                            )
                            .on_click(cx.listener(|this, v, window, cx| {
                                this.act(Command::All(*v), window, cx)
                            })),
                    )
                    .child(Input::new(&self.search).cleanable(true).flex_1()),
            );
        if self.c.busy {
            let activity = if let Some(p) = &self.c.payload_progress {
                let phase = match p.transfer.phase {
                    rtx_fg_manager::updater::DownloadPhase::Complete => "下载完成，正在准备安装…",
                    rtx_fg_manager::updater::DownloadPhase::Verifying => "正在校验补丁…",
                    rtx_fg_manager::updater::DownloadPhase::Downloading => "正在下载补丁…",
                    _ => p.transfer.label(),
                };
                div()
                    .h_flex()
                    .gap_2()
                    .flex_1()
                    .min_w_0()
                    .child(self.progress_ring(22., p.transfer.percent() / 100., cx))
                    .child(
                        div()
                            .v_flex()
                            .gap_1()
                            .flex_1()
                            .min_w_0()
                            .text_xs()
                            .child(format!(
                                "{} {}/{} · {} · {}",
                                self.c.text(phase),
                                p.package_index,
                                p.package_count,
                                p.proxy,
                                p.transfer.source
                            ))
                            .child(format!(
                                "{:.1} / {:.1} MB · {:.2} MB/s",
                                p.transfer.completed as f64 / 1_000_000.,
                                p.transfer.total as f64 / 1_000_000.,
                                p.transfer.bytes_per_second as f64 / 1_000_000.
                            )),
                    )
                    .into_any_element()
            } else {
                div()
                    .flex_1()
                    .min_w_0()
                    .text_sm()
                    .child(self.c.progress.clone())
                    .into_any_element()
            };
            card = card.child(
                div()
                    .h_flex()
                    .justify_between()
                    .gap_2()
                    .child(activity)
                    .child(self.button("cancel", "取消", Command::Cancel, cx).small()),
            );
        }
        if self.filtered.is_empty() {
            card = card.child(
                div()
                    .flex_1()
                    .v_flex()
                    .items_center()
                    .justify_center()
                    .gap_3()
                    .text_color(cx.theme().muted_foreground)
                    .child(Self::icon(IconName::LayoutDashboard).size(px(40.)))
                    .child(self.t("添加游戏本体 EXE，或扫描游戏目录")),
            );
        } else {
            card = card.child(
                uniform_list(
                    "game-list",
                    self.filtered.len(),
                    cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                        range
                            .map(|i| this.game_row(this.filtered[i], cx))
                            .collect::<Vec<_>>()
                    }),
                )
                .track_scroll(&self.scroll)
                .flex_1()
                .min_h_0(),
            );
        }
        card.into_any_element()
    }
    fn deployment_tag(&self, exe: &str) -> AnyElement {
        let status = self
            .c
            .statuses
            .get(exe)
            .map(String::as_str)
            .unwrap_or("正在检测…");
        let (label, variant) = deployment_label(status);
        Tag::new()
            .with_variant(variant)
            .outline()
            .small()
            .max_w(px(245.))
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .line_height(px(20.))
                    .child(self.t(&label)),
            )
            .into_any_element()
    }
    fn game_row(&self, index: usize, cx: &Context<Self>) -> AnyElement {
        let game = &self.c.games[index];
        let exe = game.exe.clone();
        let focus = exe.clone();
        let p = PathBuf::from(&exe);
        let selected = self.c.selected.contains(&exe);
        let menu_exe = exe.clone();
        let open_folder = p.parent().unwrap().display().to_string();
        let double_folder = open_folder.clone();
        let open_label = self.t("打开目录");
        let detail_tip = exe.clone();
        let view = cx.entity().downgrade();
        let remove_label = self.t("从游戏库移除");
        let menu_disabled = self.c.busy;
        let game_icon = self
            .c
            .icons
            .get(&exe)
            .and_then(|i| i.as_ref())
            .map(|i| img(i.clone()).size(px(36.)).into_any_element())
            .unwrap_or_else(|| {
                Self::icon(IconName::LayoutDashboard)
                    .size(px(32.))
                    .text_color(cx.theme().muted_foreground)
                    .into_any_element()
            });
        div()
            .id(("game-row", index))
            .h(px(72.))
            .h_flex()
            .w_full()
            .gap_4()
            .px_2()
            .rounded_md()
            .cursor_pointer()
            .on_click(cx.listener(move |this, ev: &ClickEvent, window, cx| {
                this.act(Command::Focus(focus.clone()), window, cx);
                if ev.click_count() == 2 {
                    this.act(Command::Open(double_folder.clone()), window, cx);
                }
            }))
            .when(self.c.focus.as_ref() == Some(&exe), |d| {
                d.bg(cx.theme().secondary)
            })
            .child(
                Checkbox::new(("game-check", index))
                    .checked(selected)
                    .on_click(cx.listener(move |this, yes, window, cx| {
                        cx.stop_propagation();
                        this.act(Command::Game(exe.clone(), *yes), window, cx)
                    })),
            )
            .child(game_icon)
            .child(
                div()
                    .id(("game-label", index))
                    .v_flex()
                    .min_w_0()
                    .flex_1()
                    .gap_1()
                    .cursor_pointer()
                    .child(
                        div()
                            .h_flex()
                            .gap_2()
                            .min_w_0()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .truncate()
                                    .child(
                                        p.file_name()
                                            .unwrap_or_default()
                                            .to_string_lossy()
                                            .to_string(),
                                    ),
                            )
                            .child(self.deployment_tag(&game.exe)),
                    )
                    .child(
                        div()
                            .id(("game-path", index))
                            .tooltip(move |window, cx| {
                                Tooltip::new(detail_tip.clone()).build(window, cx)
                            })
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .truncate()
                            .child(game.exe.clone()),
                    ),
            )
            .context_menu(move |menu, _, _| {
                let exe = menu_exe.clone();
                let open_view = view.clone();
                let folder = open_folder.clone();
                let view = view.clone();
                menu.item(
                    PopupMenuItem::new(open_label.clone()).on_click(move |_, window, cx| {
                        let _ = open_view.update(cx, |this, cx| {
                            this.act(Command::Open(folder.clone()), window, cx)
                        });
                    }),
                )
                .item(
                    PopupMenuItem::new(remove_label.clone())
                        .disabled(menu_disabled)
                        .on_click(move |_, window, cx| {
                            let _ = view.update(cx, |this, cx| {
                                this.c.focus = Some(exe.clone());
                                this.act(Command::Confirm("remove".into()), window, cx);
                            });
                        }),
                )
            })
            .into_any_element()
    }
    fn logs(&self, height: Pixels, cx: &Context<Self>) -> AnyElement {
        div()
            .v_flex()
            .h((height * 0.23).clamp(px(100.), px(182.)))
            .min_h(px(110.))
            .flex_shrink_0()
            .p_4()
            .rounded_lg()
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .gap_2()
            .child(
                div()
                    .h_flex()
                    .justify_between()
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(self.t("操作日志")),
                    )
                    .child(
                        self.button("copy-log", "复制", Command::CopyLogs, cx)
                            .ghost()
                            .small(),
                    ),
            )
            .child(
                div()
                    .id("logs")
                    .overflow_y_scroll()
                    .track_scroll(&self.log_scroll)
                    .flex_1()
                    .min_h_0()
                    .text_xs()
                    .children(self.c.logs.iter().map(|s| div().py_1().child(s.clone()))),
            )
            .into_any_element()
    }
    fn preset_panel(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let context = self.c.preset_context(self.c.focus.as_deref().unwrap_or(""));
        let options = self.c.focus.as_deref().map(|exe| self.c.preset_values(exe));
        let mut fields = div().v_flex().gap_3().w_full().min_w_0();
        let mut common = div().v_flex().gap_3().w_full().min_w_0();
        if let Some(options) = options {
            if context.profile == rtx_fg_manager::presets::MFG_VULKAN {
                fields = fields.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(self.t(
                        "独立MFG协议：默认跟随游戏；RTX20待实测。点 ? 查看倍率、优化和快捷键说明。",
                    )),
                );
            }
            if context.delta {
                common = common.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().primary)
                        .child(self.t("三角洲专项：默认4X；可选跟随游戏、2X、3X、4X。")),
                );
            }
            fields = fields.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(self.t("仅修改当前选中游戏的此方案参数。")),
            );
            for param in context.parameters() {
                let value = &options[param.key];
                let selected = param
                    .choices
                    .iter()
                    .find(|(v, _)| *v == value)
                    .map(|(_, l)| *l)
                    .unwrap_or(param.default);
                let disabled = self.c.busy
                    || !context.parameter_enabled(param.key, &options)
                    || self.c.focus.as_ref().is_some_and(|e| {
                        self.c.running_games.contains(e) || self.c.preset_reads.contains(e)
                    })
                    || (param.key == "hardware_bilinear" && self.c.cloud_series() == 0);
                let choices = param
                    .choices
                    .iter()
                    .filter(|(v, _)| {
                        param.key != "force_multiplier"
                            || *v == "0"
                            || v.parse::<u32>().unwrap_or(0)
                                <= options
                                    .get("max_interpolated_frames")
                                    .and_then(|s| s.parse::<u32>().ok())
                                    .unwrap_or(5)
                                    + 1
                    })
                    .map(|(v, l)| {
                        (
                            self.t(l),
                            Command::GameOption(param.key.into(), (*v).into()),
                            *v == value,
                        )
                    })
                    .collect();
                let field = div()
                    .v_flex()
                    .gap_1()
                    .child(div().text_xs().child(self.t(param.label)))
                    .child(self.menu(param.key, self.t(selected), choices, disabled, window, cx));
                if matches!(
                    param.key,
                    "max_generated_frames" | "max_interpolated_frames" | "force_multiplier"
                ) {
                    common = common.child(field);
                } else {
                    fields = fields.child(field);
                }
            }
            fields = fields.child(
                self.button(
                    "preset-reset",
                    "恢复默认参数",
                    Command::GameOption("reset".into(), "".into()),
                    cx,
                )
                .small()
                .ghost()
                .disabled(
                    self.c.busy
                        || self.c.focus.as_ref().is_some_and(|e| {
                            self.c.running_games.contains(e) || self.c.preset_reads.contains(e)
                        }),
                ),
            );
        } else {
            common = common.child(
                div()
                    .text_sm()
                    .child(self.t("选择一款游戏后，可为它单独设置参数。")),
            );
        }
        let title = div()
            .h_flex()
            .gap_2()
            .items_center()
            .child(self.t("预设参数"))
            .child(
                Button::new("preset-help")
                    .label("?")
                    .ghost()
                    .small()
                    .tooltip(self.t("参数说明"))
                    .on_click(cx.listener(|this, _, window, cx| {
                        cx.stop_propagation();
                        this.act(Command::ConfigHelp, window, cx);
                    })),
            );
        div()
            .v_flex()
            .gap_3()
            .w_full()
            .flex_shrink_0()
            .child(title)
            .when(
                self.c
                    .focus
                    .as_deref()
                    .is_some_and(|exe| self.c.preset_dirty(exe)),
                |d| {
                    d.child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().info)
                            .child(self.t("有修改，待应用")),
                    )
                },
            )
            .child(common)
            .child(
                Accordion::new("preset-accordion")
                    .small()
                    .item(|item| {
                        item.title(self.t("高级参数"))
                            .open(self.c.boolean("preset_expanded", false))
                            .child(fields)
                    })
                    .on_toggle_click(cx.listener(|this, indices: &[usize], _, cx| {
                        this.c.state["preset_expanded"] = json!(!indices.is_empty());
                        this.c.save();
                        cx.notify();
                    })),
            )
            .when(self.c.can_apply_parameters(), |d| {
                d.child(
                    self.button(
                        "apply-preset",
                        "应用参数到当前游戏",
                        Command::ApplyPreset,
                        cx,
                    )
                    .small()
                    .tooltip(
                        self.t("只修改当前游戏的参数，不重新下载 DLL；批量安装仍使用底部按钮。"),
                    )
                    .disabled(
                        self.c.busy
                            || self.c.read_only
                            || self.c.focus.as_ref().is_some_and(|e| {
                                self.c.running_games.contains(e) || self.c.preset_reads.contains(e)
                            }),
                    ),
                )
            })
            .into_any_element()
    }
    fn details(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let (scheme, series) = self.profile();
        let mut panel = div()
            .id("details-scroll")
            .v_flex()
            .flex_1()
            .w_full()
            .min_h_0()
            .overflow_y_scroll()
            .gap_4()
            .p_4();
        panel = panel
            .child(
                div()
                    .v_flex()
                    .gap_2()
                    .child(div().text_sm().child(self.t("适配方案")))
                    .child(
                        self.menu(
                            "scheme",
                            self.c.cloud_label(self.c.catalog.schemes()[scheme]).into(),
                            self.c
                                .catalog
                                .schemes()
                                .iter()
                                .enumerate()
                                .map(|(i, s)| {
                                    (
                                        self.c.cloud_label(s).into(),
                                        Command::Scheme(i),
                                        i == scheme,
                                    )
                                })
                                .collect(),
                            self.c.busy,
                            window,
                            cx,
                        ),
                    ),
            )
            .child(
                div()
                    .v_flex()
                    .gap_2()
                    .child(div().text_sm().child(self.t("显卡系列")))
                    .child(
                        RadioGroup::horizontal("gpu-series")
                            .selected_index(Some(series))
                            .disabled(self.c.busy)
                            .child(
                                Radio::new("rtx20").label("RTX 20").disabled(
                                    !self.c.catalog.scheme_policies[&self.c.cloud_scheme()]
                                        .gpu_paths
                                        .contains(&"SM75".into()),
                                ),
                            )
                            .child(
                                Radio::new("rtx30").label("RTX 30").disabled(
                                    !self.c.catalog.scheme_policies[&self.c.cloud_scheme()]
                                        .gpu_paths
                                        .contains(&"SM86".into()),
                                ),
                            )
                            .on_click(cx.listener(|this, index, window, cx| {
                                this.act(Command::Series(*index), window, cx)
                            })),
                    ),
            );
        let cloud_source = self.c.choice("download_source", "domestic");
        panel = panel
            .child(
                div()
                    .v_flex()
                    .gap_2()
                    .child(div().text_sm().child(self.t("下载线路")))
                    .child(self.menu(
                        "cloud-source",
                        self.t(if cloud_source == "github" {
                            "GitHub 优先"
                        } else {
                            "国内优先"
                        }),
                        vec![
                            (
                                self.t("国内优先"),
                                Command::CloudSource("domestic".into()),
                                cloud_source != "github",
                            ),
                            (
                                self.t("GitHub 优先"),
                                Command::CloudSource("github".into()),
                                cloud_source == "github",
                            ),
                        ],
                        self.c.busy,
                        window,
                        cx,
                    )),
            )
            .child(self.preset_panel(window, cx));
        let proxies = self.c.proxies();
        let mut checks = div()
            .v_flex()
            .gap_3()
            .child(div().text_sm().child(self.t("DLL 加载入口")));
        for n in core::PROXIES {
            if !self
                .c
                .catalog
                .proxies(&self.c.cloud_scheme())
                .iter()
                .any(|p| p == n)
            {
                continue;
            }
            checks = checks.child(
                Checkbox::new(n)
                    // The component's built-in label forces line-height: 1 inside
                    // an overflow-hidden wrapper, clipping descenders with our font.
                    // A child keeps the standard checkbox interaction but supplies
                    // its own full line box, including the bottom of "g".
                    .child(
                        div()
                            .line_height(px(24.))
                            .min_h(px(24.))
                            .flex_shrink_0()
                            .child(n),
                    )
                    .tooltip(n)
                    .checked(proxies.iter().any(|s| s == n))
                    .disabled(self.c.busy)
                    .items_center()
                    .min_h(px(24.))
                    .on_click(cx.listener(move |this, yes, window, cx| {
                        this.act(Command::Proxy(n.into(), *yes), window, cx)
                    })),
            );
        }
        let proxy_hint = if self.c.parameter_profile() == rtx_fg_manager::presets::MFG_VULKAN {
            "此上游版本仅提供version.dll，请勿改名或混装其他方案的DLL。"
        } else if self.c.cloud_scheme().starts_with("upstream") {
            "上游支持多选入口；游戏先加载的 DLL 生效，其余仅转发。优先 version/winmm/dinput8/dbghelp；dxgi/d3d12 按需使用。"
        } else {
            "优先单选 version.dll；不兼容时卸载后换入口。多选可能冲突。"
        };
        panel = panel
            .child(
                Accordion::new("proxy-accordion")
                    .small()
                    .item(|item| {
                        item.title(format!(
                            "{} · {}",
                            self.c.text("DLL 加载入口"),
                            proxies.join(", ")
                        ))
                        .open(self.c.boolean("proxies_expanded", false))
                        .child(checks)
                    })
                    .on_toggle_click(cx.listener(|this, indices: &[usize], _, cx| {
                        this.c.state["proxies_expanded"] = json!(!indices.is_empty());
                        this.c.save();
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(self.t(proxy_hint)),
            );
        panel =
            panel.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(self.t(
                        "DLL 按需从云端下载，国内优先，失败后尝试 GitHub；已缓存文件可离线使用。",
                    )),
            );
        if self.c.cloud_scheme().starts_with("upstream") {
            panel = panel
                .child(Alert::info("upstream-scope", self.t("sdli1995原版使用D3D12；Vulkan可选0.2.6或独立Dlssg-MFG-Vulkan方案，兼容性需按游戏实测。")).small());
        }
        if self.c.parameter_profile() == rtx_fg_manager::presets::MFG_VULKAN {
            panel = panel.child(
                Alert::info(
                    "mfg-vulkan-scope",
                    self.t("新上游方案：作者已宣布Vulkan支持，RTX20尚待实测；不包含三角洲专项。"),
                )
                .small(),
            );
        }
        if self.c.cloud_scheme().contains("experimental")
            || self.c.cloud_scheme().starts_with("upstream")
        {
            panel = panel.child(
                Alert::info(
                    "experimental-limit",
                    self.t("这是倍率上限，实际倍率由游戏请求；不会自动给游戏添加 5X/6X 菜单。"),
                )
                .small(),
            );
        }
        let target_hint = if self.c.selected.is_empty() {
            self.c.text("处理当前游戏；勾选复选框可批量处理。")
        } else {
            self.c.text(&format!(
                "将处理勾选的 {0} 款游戏。",
                self.c.targets().len()
            ))
        };
        if let Some(exe) = &self.c.focus {
            let game = self.c.games.iter().find(|g| &g.exe == exe);
            let probe = self.c.evidence.get(exe).and_then(Option::as_ref);
            let (dlss, anti) = match probe {
                Some(p) => (
                    if p.frame_generation
                        || game.is_some_and(|g| {
                            g.reasons.iter().any(|s| s.contains("DLSS 帧生成组件"))
                        })
                    {
                        "发现 DLSS 帧生成组件；是否可用由游戏决定。"
                    } else if p.super_resolution {
                        "发现 DLSS 超分组件，不代表支持帧生成。"
                    } else {
                        "有限检测未发现 DLSS 帧生成组件，不代表游戏不支持。"
                    },
                    if p.anti_cheat || game.is_some_and(|g| g.anti) {
                        "发现反作弊线索，请先确认游戏是否允许此类补丁。"
                    } else {
                        "未发现反作弊线索，不代表没有反作弊。"
                    },
                ),
                None => ("正在检测…", "游戏本身须支持 DLSS 帧生成。"),
            };
            panel = panel
                .child(Alert::info("game-evidence", self.t(dlss)).small())
                .child(Alert::new("anti-evidence", self.t(anti)).small());
            if let Some(g) = game {
                for reason in &g.reasons {
                    panel = panel.child(div().text_xs().child(self.t(reason)));
                }
            }
        }
        div()
            .v_flex()
            .size_full()
            .min_h_0()
            .rounded_lg()
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .overflow_hidden()
            .child(panel)
            .child(
                div()
                    .v_flex()
                    .flex_shrink_0()
                    .p_4()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .gap_2()
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(target_hint),
                    )
                    .child(
                        self.button("deploy", "安装并应用", Command::Patch(false), cx)
                            .primary()
                            .w_full()
                            .disabled(
                                self.c.busy || self.c.read_only || self.c.targets().is_empty(),
                            ),
                    )
                    .child(
                        self.button("clean", "卸载补丁", Command::Patch(true), cx)
                            .w_full()
                            .disabled(self.c.busy || self.c.targets().is_empty()),
                    ),
            )
            .into_any_element()
    }
    fn home(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        div()
            .h_flex()
            .items_stretch()
            .gap_4()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .child(
                div()
                    .v_flex()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .gap_4()
                    .child(self.library(cx))
                    .child(self.logs(window.viewport_size().height, cx)),
            )
            .when(window.viewport_size().width >= px(880.), |d| {
                d.child(
                    div()
                        .w(px(324.))
                        .flex_shrink_0()
                        .min_h_0()
                        .child(self.details(window, cx)),
                )
            })
            .into_any_element()
    }
    fn update_panel(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let source = self.c.choice("download_source", "domestic");
        let mut panel = div()
            .v_flex()
            .w_full()
            .min_w_0()
            .flex_shrink_0()
            .gap_3()
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(self.t("软件更新")),
            )
            .child(format!("v{VERSION}"))
            .child(
                div().w_full().child(
                    self.menu(
                        "source",
                        self.t(match source.as_str() {
                            "github" => "GitHub 优先",
                            _ => "国内优先",
                        }),
                        [("domestic", "国内优先"), ("github", "GitHub 优先")]
                            .iter()
                            .map(|(s, t)| (self.t(t), Command::Source((*s).into()), *s == source))
                            .collect(),
                        false,
                        window,
                        cx,
                    ),
                ),
            );
        for (key, label, default) in [
            ("auto_update", "自动检查更新", true),
            ("auto_download", "自动下载更新", false),
        ] {
            panel = panel.child(
                Switch::new(key)
                    .label(self.t(label))
                    .checked(self.c.boolean(key, default))
                    .on_click(cx.listener(move |this, yes, window, cx| {
                        this.act(Command::Boolean(key.into(), *yes), window, cx)
                    })),
            );
        }
        panel = panel.child(
            div().h_flex().child(
                self.button("check-update", "检查更新", Command::CheckUpdate, cx)
                    .disabled(self.c.update_busy),
            ),
        );
        if !self.c.update_status.is_empty() {
            panel = panel.child(div().text_sm().child(self.t(&self.c.update_status)));
        }
        if let Some(m) = &self.c.update {
            panel = panel.child(format!(
                "{} · {} · {:.1} MB",
                m.version,
                m.source,
                m.bytes as f64 / 1_000_000.
            ));
            let mut actions = div()
                .id("download-actions")
                .h_flex()
                .w_full()
                .flex_shrink_0()
                .flex_wrap()
                .gap_2()
                .child(if self.c.download.is_some() {
                    self.button(
                        "install-update",
                        "安装更新",
                        Command::Commit("update".into()),
                        cx,
                    )
                    .primary()
                    .disabled(self.c.busy || self.c.update_busy)
                } else {
                    self.button("download-update", "下载更新", Command::Download, cx)
                        .primary()
                        .disabled(self.c.update_busy)
                });
            if !self.c.checking && self.c.download_progress.total > 0 {
                actions = actions.child(
                    div()
                        .h_flex()
                        .flex_shrink_0()
                        .gap_2()
                        .child(self.download_ring(22., cx))
                        .child(div().text_sm().whitespace_nowrap().child(format!(
                            "{}%",
                            self.c.download_progress.percent().floor() as u32
                        ))),
                );
            }
            if self.c.update_busy && !self.c.checking {
                actions = actions.child(
                    self.button("cancel-update", "取消", Command::CancelUpdate, cx)
                        .small()
                        .ghost()
                        .disabled(self.c.update_installing),
                );
            }
            panel = panel.child(actions);
            panel = panel.child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(self.t("点击更新后，下载校验完成将自动替换并重启。")),
            );
        }
        if !self.c.checking && self.c.download_progress.total > 0 {
            let p = &self.c.download_progress;
            // Give statistics a definite full-width row; wrap whole fields,
            // never measure a shrink-to-fit column that breaks every character.
            panel = panel.child(
                div()
                    .id("download-statistics")
                    .h_flex()
                    .w_full()
                    .flex_shrink_0()
                    .flex_wrap()
                    .gap_2()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(div().whitespace_nowrap().child(format!(
                        "{:.1} / {:.1} MB",
                        p.completed as f64 / 1_000_000.,
                        p.total as f64 / 1_000_000.
                    )))
                    .child(div().whitespace_nowrap().child(format!(
                        "· {:.2} MB/s",
                        p.bytes_per_second as f64 / 1_000_000.
                    )))
                    .child(div().whitespace_nowrap().child(format!("· {}", p.source)))
                    .when(self.c.update_busy && p.bytes_per_second > 0, |d| {
                        let seconds = p
                            .total
                            .saturating_sub(p.completed)
                            .checked_div(p.bytes_per_second)
                            .unwrap_or(0);
                        let label = self.t(&format!("剩余约 {} 秒", seconds));
                        d.child(div().whitespace_nowrap().child(label))
                    }),
            );
        }
        if self.c.update_busy && self.c.checking {
            panel = panel.child(
                div().h_flex().child(
                    self.button("cancel-update", "取消", Command::CancelUpdate, cx)
                        .small(),
                ),
            );
        }
        panel.into_any_element()
    }
    fn settings(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let language = self.c.choice("language", "system");
        let theme = self.c.choice("theme", "跟随系统");
        let wide = window.viewport_size().width >= px(980.);
        let preferences = div()
            .v_flex()
            .w_full()
            .min_w_0()
            .gap_5()
            .child(
                div()
                    .v_flex()
                    .gap_2()
                    .w_full()
                    .min_w_0()
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(self.t("语言")),
                    )
                    .child(
                        self.menu(
                            "language",
                            i18n::LANGUAGES
                                .iter()
                                .find(|(c, _)| *c == language)
                                .map(|(c, s)| {
                                    if *c == "system" {
                                        self.t("跟随系统")
                                    } else {
                                        (*s).into()
                                    }
                                })
                                .unwrap_or_else(|| "System".into()),
                            i18n::LANGUAGES
                                .iter()
                                .map(|(c, s)| {
                                    (
                                        if *c == "system" {
                                            self.t("跟随系统")
                                        } else {
                                            (*s).into()
                                        },
                                        Command::Language((*c).into()),
                                        *c == language,
                                    )
                                })
                                .collect(),
                            false,
                            window,
                            cx,
                        ),
                    ),
            )
            .child(
                div()
                    .v_flex()
                    .gap_2()
                    .w_full()
                    .min_w_0()
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(self.t("主题")),
                    )
                    .child(
                        self.menu(
                            "theme",
                            self.t(&theme),
                            ["跟随系统", "浅色", "深色"]
                                .iter()
                                .map(|s| (self.t(s), Command::Theme((*s).into()), *s == theme))
                                .collect(),
                            false,
                            window,
                            cx,
                        ),
                    ),
            );
        let preferences=preferences.child(div().v_flex().gap_2().child(div().font_weight(FontWeight::SEMIBOLD).child(self.t("软件缓存"))).child(div().text_xs().text_color(cx.theme().muted_foreground).child(self.t("清理下载、更新暂存及未使用的已识别组件缓存；保留游戏列表、参数和归属记录。需要时重新下载或生成缓存。"))).child(self.button("clear-cache","清理缓存",Command::ClearCache,cx).disabled(self.c.busy||self.c.update_busy)));
        let mut links = div()
            .grid()
            .w_full()
            .grid_cols(if wide { 2 } else { 1 })
            .gap_2();
        for (icon, label, url) in [
            (
                IconName::Github,
                "GitHub · sdli1995",
                "https://github.com/sdli1995/dlssg_for_sm86",
            ),
            (IconName::Globe, "小南瓜 · lgxng.cn", "https://lgxng.cn/"),
            (
                IconName::Github,
                "GitHub · pandaligx",
                "https://github.com/pandaligx",
            ),
            (
                IconName::Play,
                "哔哩哔哩 · 小南瓜",
                "https://b23.tv/5mHCHFn",
            ),
            (
                IconName::Play,
                "哔哩哔哩 · 大大大怪将军阁下（测试支持）",
                "https://space.bilibili.com/608531525",
            ),
            (
                IconName::Play,
                "哔哩哔哩 · 云外逸声",
                "https://space.bilibili.com/256887068",
            ),
        ] {
            links = links.child(
                div().h_flex().child(
                    self.button(url, label, Command::Open(url.into()), cx)
                        .icon(if url.contains("bilibili.com") || url.contains("b23.tv") {
                            Icon::default().path("app/bilibili.svg").size(px(20.))
                        } else {
                            Self::icon(icon)
                        })
                        .ghost(),
                ),
            );
        }
        let panel = div()
            .v_flex()
            .w_full()
            .min_w_0()
            .flex_shrink_0()
            .gap_5()
            .p_5()
            .child(
                div()
                    .text_lg()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(self.t("设置")),
            )
            .child(
                div()
                    .grid()
                    .w_full()
                    .min_w_0()
                    .grid_cols(if wide { 2 } else { 1 })
                    .gap_8()
                    .items_start()
                    .child(self.update_panel(window, cx))
                    .child(preferences),
            )
            .child(
                div()
                    .mt_2()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(self.t("关于与致谢")),
            )
            .child(links);
        div()
            .id("settings-scroll")
            .v_flex()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .overflow_y_scroll()
            .bg(cx.theme().background)
            .child(panel)
            .into_any_element()
    }
    fn download_ring(&self, diameter: f32, cx: &App) -> AnyElement {
        self.progress_ring(diameter, self.c.download_progress.percent() / 100., cx)
    }
    fn progress_ring(&self, diameter: f32, progress: f32, cx: &App) -> AnyElement {
        let foreground = cx.theme().info;
        let track = cx.theme().border;
        canvas(
            |_, _, _| {},
            move |bounds, _, window, _| {
                let center = bounds.center();
                let radius = bounds.size.width / 2. - px(2.);
                // Split into short arcs: also renders a complete ring without an
                // ambiguous same-start/end SVG arc. Geometry scales with DPI.
                for (fraction, color) in [(1., track), (progress, foreground)] {
                    if fraction <= 0. {
                        continue;
                    }
                    let mut path = PathBuilder::stroke(px(2.));
                    path.move_to(point(center.x, center.y - radius));
                    let steps = (fraction * 64.).ceil() as u32;
                    for step in 1..=steps {
                        let angle = std::f32::consts::TAU * fraction * step as f32 / steps as f32
                            - std::f32::consts::FRAC_PI_2;
                        path.arc_to(
                            point(radius, radius),
                            px(0.),
                            false,
                            true,
                            point(
                                center.x + radius * angle.cos(),
                                center.y + radius * angle.sin(),
                            ),
                        );
                    }
                    if let Ok(path) = path.build() {
                        window.paint_path(path, color);
                    }
                }
            },
        )
        .size(px(diameter))
        .flex_shrink_0()
        .into_any_element()
    }
    fn download_bar(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = &self.c.download_progress;
        div()
            .h_flex()
            .w_full()
            .flex_shrink_0()
            .gap_3()
            .p_3()
            .bg(cx.theme().background)
            .child(self.download_ring(22., cx))
            .child(
                div()
                    .v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_1()
                    .text_sm()
                    .child(self.t(&self.c.update_status))
                    .child(format!(
                        "{:.1} / {:.1} MB · {:.2} MB/s",
                        p.completed as f64 / 1_000_000.,
                        p.total as f64 / 1_000_000.,
                        p.bytes_per_second as f64 / 1_000_000.
                    )),
            )
            .child(
                self.button("view-download", "查看下载", Command::Page(2), cx)
                    .small(),
            )
            .into_any_element()
    }
    fn gpu_page(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        div()
            .id("gpu-scroll")
            .v_flex()
            .gap_5()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .p_5()
            .bg(cx.theme().background)
            .rounded_lg()
            .border_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .text_lg()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(self.t("显卡名称")),
            )
            .child(
                self.t(
                    "仅修改 Windows 显示名称，不改变显卡性能或硬件能力。修改和还原后请重启电脑。",
                ),
            )
            .child(
                div().max_w(px(640.)).child(
                    self.menu(
                        "gpu-device",
                        self.c
                            .devices
                            .get(self.c.device_index)
                            .map(|d| d.name.clone().into())
                            .unwrap_or_else(|| "NVIDIA".into()),
                        self.c
                            .devices
                            .iter()
                            .enumerate()
                            .map(|(i, d)| {
                                (
                                    d.name.clone().into(),
                                    Command::Device(i),
                                    i == self.c.device_index,
                                )
                            })
                            .collect(),
                        self.c.busy,
                        window,
                        cx,
                    ),
                ),
            )
            .child(
                div().h_flex().child(
                    self.button("gpu-refresh", "刷新", Command::RefreshGpu, cx)
                        .disabled(self.c.busy),
                ),
            )
            .when_some(self.c.devices.get(self.c.device_index), |d, device| {
                d.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(format!("{} · {}", device.version, device.instance)),
                )
            })
            .child(
                div().max_w(px(460.)).child(
                    self.menu(
                        "gpu-alias",
                        gpu_alias::NAMES[self.c.alias_index].into(),
                        gpu_alias::NAMES
                            .iter()
                            .enumerate()
                            .map(|(i, s)| ((*s).into(), Command::Alias(i), i == self.c.alias_index))
                            .collect(),
                        self.c.busy,
                        window,
                        cx,
                    ),
                ),
            )
            .child(
                div()
                    .h_flex()
                    .gap_3()
                    .child(
                        self.button(
                            "alias",
                            "修改显卡名称",
                            Command::Confirm("alias".into()),
                            cx,
                        )
                        .primary()
                        .disabled(self.c.busy || self.c.devices.is_empty()),
                    )
                    .child(
                        self.button(
                            "restore",
                            "还原原始名称",
                            Command::Confirm("restore".into()),
                            cx,
                        )
                        .disabled(self.c.busy || self.c.devices.is_empty()),
                    ),
            )
            .into_any_element()
    }
    fn config_help(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.dialog_active {
            return;
        }
        self.dialog_active = true;
        let title = self.t("预设参数说明");
        let ok = self.t("确定");
        let profile = self.c.parameter_profile();
        let context = self.c.preset_context(self.c.focus.as_deref().unwrap_or(""));
        let mut sections = vec![
            (self.t("当前方案"), self.c.cloud_label(self.c.catalog.selected(&self.c.cloud_scheme())).into()),
            (self.t("倍率上限"),self.t(if context.delta {"三角洲可选跟随游戏、2X、3X、4X，默认4X。4X不是保证四倍FPS，开启后的延迟需实测；旧组件已加载时会保留原运行时。"} else {"默认最多 4X；上游方案可设 5X/6X，实际倍率由游戏自身插件请求，不会增加游戏菜单。"})),
            (self.t("日志级别"),self.t("默认 1 只记录错误；2 用于诊断；3 记录详细过程。0.2.6 的 Vulkan 桥接日志暂不受此开关控制。")),
            (self.t("怎样生效"),self.t("仅保存当前游戏、当前方案的参数。完全退出游戏后点击安装补丁以应用；已部署相同 DLL 时只更新这些参数，保留其他 INI 内容。恢复默认只重置这里的选项。")),
        ];
        if profile == rtx_fg_manager::presets::MFG_VULKAN {
            sections.remove(2);
            sections[1] = (self.t("倍率上限与请求倍率"), self.t("作者默认上限6X，请求倍率跟随游戏。上限与请求是两项独立设置；请求倍率请勿超过上限。支持情况取决于游戏和运行库，不会添加游戏菜单，也不保证成倍FPS。"));
            sections.insert(2, (self.t("动态多帧与优化"), self.t("动态多帧默认关闭，仅兼容的DX12路径支持；目标帧率0跟随游戏。四项优化按作者默认开启，可能影响画质或兼容性，出问题可逐项关闭。日志是开关，不是0—3级别。")));
            sections.insert(3, (self.t("保留作者配置"), self.t("RenderScale保持游戏控制；BlackwellScatter/BlackwellBlend默认关闭，作者记录过GPU挂起，不在此提供开关。其余高级设置和快捷键原样保留。Ctrl+F2—F6请求2X—6X，Ctrl+F11跟随游戏，Ctrl+F12恢复启动INI。")));
            sections.insert(1, (self.t("Vulkan 支持范围"), self.t("sm86-7发布说明新增Vulkan（含RTX Remix示例），不等于所有Vulkan游戏可用。作者仅有RTX3070，RTX20支持尚待实测。此方案独立于Smooth Motion和三角洲专项，只提供version.dll。")));
        } else if profile.starts_with("upstream") {
            sections.insert(1,(self.t("内核模式"),self.t("0 为原厂数值；1 为逐位一致优化（默认）；2/3 进一步加速但有画质损失，仅新版 310.9 方案支持。保持默认即可。")));
            sections.insert(2,(self.t("UI 重组预设"),self.t("Auto 由游戏或驱动决定。A 关闭 UI 重组；B 尝试让生成帧里的 HUD 更干净，但多数游戏不会提供所需的 UI 平面。")));
        } else if profile == "native026" {
            sections.insert(1,(self.t("采样方式（仅 SM86）"),self.t("0.2.6 默认精确采样。近似采样仅适用于 SM86，可能改变画面；RTX20 保持精确。不会套用上游 0—3 档。")));
        } else {
            sections.insert(1,(self.t("初始方案"),self.t("RTX20 使用 SM75 R2，RTX30 使用原始 SM86。只提供补丁开关、倍率上限和日志级别，不支持新版内核档位或 UI 重组。")));
        }
        if context.delta {
            sections.insert(1, (self.t("三角洲专项"), self.t("本扩展方案通过同一倍率选项联动三角洲专项：跟随游戏和2X使用原始运行时，3X/4X使用独立缓存中的多帧组件。不会增加游戏菜单或覆盖原游戏组件；更改后退出游戏并点安装应用。")));
        }
        sections.push((self.t("卸载与缓存"), self.t("卸载会清理本游戏已确认归属的补丁、配置、日志和独立组件缓存。被占用或未知文件会保留并提示，稍后可重试卸载或清理缓存。其他游戏和旧测试还原备份不会整目录删除。")));
        let view = cx.entity().downgrade();
        let width = (window.viewport_size().width - px(90.)).min(px(620.));
        let height = (window.viewport_size().height - px(270.)).max(px(160.));
        window.open_dialog(cx, move |dialog, _, _| {
            let view = view.clone();
            dialog
                .title(title.clone())
                .width(width)
                .button_props(DialogButtonProps::default().ok_text(ok.clone()))
                .footer(
                    DialogFooter::new().child(
                        Button::new("config-help-ok")
                            .label(ok.clone())
                            .primary()
                            .on_click(|_, window, cx| {
                                window.dispatch_action(Box::new(ConfirmDialog), cx)
                            }),
                    ),
                )
                .child(
                    div()
                        .id("config-help-scroll")
                        .v_flex()
                        .gap_4()
                        .max_h(height)
                        .overflow_y_scroll()
                        .children(sections.iter().map(|(heading, text)| {
                            div()
                                .v_flex()
                                .flex_shrink_0()
                                .gap_1()
                                .child(
                                    div()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(heading.clone()),
                                )
                                .child(text.clone())
                        })),
                )
                .on_close(move |_, _, cx| {
                    let _ = view.update(cx, |this, cx| {
                        this.dialog_active = false;
                        cx.notify();
                    });
                })
        });
    }
    fn help(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.dialog_active {
            return;
        }
        self.dialog_active = true;
        let sections: Vec<(String, String)> =
            serde_json::from_str(include_str!("../assets/help.json")).unwrap();
        let sections = sections
            .into_iter()
            .map(|(a, b)| (self.t(&a), self.t(&b)))
            .collect::<Vec<_>>();
        let title = self.t("使用说明 · RTX 帧生成管理器");
        let ok = self.t("确定");
        let view = cx.entity().downgrade();
        let height = (window.viewport_size().height - px(270.)).max(px(160.));
        let width = (window.viewport_size().width - px(100.)).min(px(760.));
        window.open_dialog(cx, move |dialog, _, _| {
            let view = view.clone();
            dialog
                .title(title.clone())
                .width(width)
                .button_props(DialogButtonProps::default().ok_text(ok.clone()))
                .footer(
                    DialogFooter::new().child(
                        Button::new("help-ok").label(ok.clone()).primary().on_click(
                            |_, window, cx| window.dispatch_action(Box::new(ConfirmDialog), cx),
                        ),
                    ),
                )
                .child(
                    div()
                        .id("help-scroll")
                        .v_flex()
                        .gap_5()
                        .h(height)
                        .overflow_y_scroll()
                        .children(sections.iter().map(|(a, b)| {
                            div()
                                .v_flex()
                                .gap_2()
                                .child(div().font_weight(FontWeight::SEMIBOLD).child(a.clone()))
                                .child(b.clone())
                        })),
                )
                .on_close(move |_, _, cx| {
                    let _ = view.update(cx, |this, cx| {
                        this.dialog_active = false;
                        if this.c.boolean("welcome_pending", false) {
                            this.c.state["welcome_pending"] = json!(false);
                            this.c.save();
                        }
                        cx.notify()
                    });
                })
        });
    }
    fn update_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.c.update_offer = false;
        let Some(m) = self.c.update.clone() else {
            return;
        };
        self.dialog_active = true;
        let title = self.t("发现新版本");
        let explanation = self.t(
            "下载校验完成后使用新版文件名并自动重启。新版界面启动成功后删除旧程序，失败则还原。",
        );
        let details = format!(
            "v{VERSION} → v{}\n{}\n{:.1} MB · {}",
            m.version,
            m.file,
            m.bytes as f64 / 1_000_000.,
            m.source
        );
        let accept = self.t("下载并更新");
        let later = self.t("稍后");
        let width = (window.viewport_size().width - px(80.)).min(px(520.));
        let top = ((window.viewport_size().height - px(320.)) / 2.).max(px(40.));
        let view = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _, _| {
            let ok_view = view.clone();
            let close_view = view.clone();
            dialog
                .title(title.clone())
                .width(width)
                .margin_top(top)
                .footer(
                    DialogFooter::new()
                        .child(Button::new("update-later").label(later.clone()).on_click(
                            |_, window, cx| {
                                window.dispatch_action(Box::new(CancelDialog), cx);
                            },
                        ))
                        .child(
                            Button::new("update-accept")
                                .label(accept.clone())
                                .primary()
                                .on_click(|_, window, cx| {
                                    window.dispatch_action(Box::new(ConfirmDialog), cx);
                                }),
                        ),
                )
                .overlay_closable(false)
                .child(
                    div()
                        .v_flex()
                        .w_full()
                        .gap_3()
                        .child(
                            div()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(details.clone()),
                        )
                        .child(explanation.clone()),
                )
                .on_ok(move |_, _, cx| {
                    let _ = ok_view.update(cx, |this, cx| {
                        this.dialog_active = false;
                        this.c.download_update();
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
    fn dialogs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.dialog_active {
            return;
        }
        if self.c.warning.is_none()
            && self.c.notice.is_none()
            && self.c.confirm.is_none()
            && self.c.update_offer
        {
            self.update_dialog(window, cx);
            return;
        }
        let (title, text, action, status_icon) = if let Some(s) = self.c.warning.take() {
            (
                self.t("操作未完成"),
                self.t(&s),
                None,
                IconName::TriangleAlert,
            )
        } else if let Some(s) = self.c.notice.take() {
            (self.t("操作完成"), self.t(&s), None, IconName::CircleCheck)
        } else if let Some(a) = self.c.confirm.take() {
            let text = match a.as_str() {
                "deploy_batch" => "确认向以下游戏安装补丁？请先完全退出这些游戏。",
                "clean_batch" => "确认卸载以下游戏的补丁？请先完全退出这些游戏。",
                "remove" | "remove_checked" => "仅从游戏库移除；游戏文件和已部署补丁保持不变。",
                "update" => "更新将关闭管理器并替换程序，成功启动后删除旧版。是否继续？",
                _ => "仅修改 Windows 显示名称，不改变显卡性能或硬件能力。修改和还原后请重启电脑。",
            };
            let mut message = self.c.text(text);
            if a == "deploy_batch" || a == "clean_batch" {
                message.push_str("\n\n");
                message.push_str(&self.c.pending_patch_list());
            } else if a == "remove_checked" {
                message.push_str("\n\n");
                message.push_str(
                    &self
                        .c
                        .selected
                        .iter()
                        .cloned()
                        .collect::<Vec<_>>()
                        .join("\n"),
                );
            } else if a == "remove"
                && let Some(exe) = &self.c.focus
            {
                message.push_str(&format!("\n\n{exe}"));
            }
            (self.t("请确认"), message.into(), Some(a), IconName::Info)
        } else {
            return;
        };
        self.dialog_active = true;
        let view = cx.entity().downgrade();
        // The pinned AlertDialog overwrites its base on_close callback during
        // conversion. Release our guard in both standard button handlers instead.
        let props = DialogButtonProps::default()
            .ok_text(self.t(if action.is_some() { "继续" } else { "确定" }))
            .cancel_text(self.t("取消"))
            .show_cancel(action.is_some())
            .ok_variant(if action.as_deref() == Some("clean_batch") {
                ButtonVariant::Danger
            } else {
                ButtonVariant::Primary
            });
        let width = (window.viewport_size().width - px(90.)).min(px(600.));
        let height = (window.viewport_size().height - px(240.)).max(px(120.));
        window.open_alert_dialog(cx, move |dialog, _, _| {
            let ok_view = view.clone();
            let cancel_view = view.clone();
            let action = action.clone();
            dialog
                .title(title.clone())
                .icon(Self::icon(status_icon.clone()).size(px(28.)))
                .width(width)
                .button_props(props.clone())
                .overlay_closable(false)
                .child(
                    div()
                        .id("message-scroll")
                        .max_h(height)
                        .overflow_y_scroll()
                        .child(text.clone()),
                )
                .on_ok(move |_, window, cx| {
                    let _ = ok_view.update(cx, |this, cx| {
                        this.dialog_active = false;
                        if let Some(action) = &action {
                            this.act(Command::Commit(action.clone()), window, cx)
                        }
                        cx.notify();
                    });
                    true
                })
                .on_cancel(move |_, _, cx| {
                    let _ = cancel_view.update(cx, |this, cx| {
                        this.dialog_active = false;
                        this.c.cancel_patch();
                        cx.notify();
                    });
                    true
                })
        });
    }
    fn tick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let now = Instant::now();
        self.max_gap_ms = self
            .max_gap_ms
            .max(now.duration_since(self.last_tick).as_secs_f64() * 1000.);
        self.last_tick = now;
        self.ticks += 1;
        if !self.update_ready_sent && self.frames > 0 && !self.c.read_only {
            self.update_ready_sent = true;
            let data = self.c.data.clone();
            self.c
                .channel
                .job(move |_| rtx_fg_manager::updater::signal_ready(&data));
        }
        let before = self.c.games.len();
        let logs = self.c.logs.len();
        if self.c.events() {
            if before != self.c.games.len() {
                self.filter()
            }
            if logs != self.c.logs.len() {
                self.log_scroll.scroll_to_bottom();
            }
            cx.notify();
        }
        if self.c.install_pending && !self.c.busy && !self.c.update_busy && !self.c.closing {
            self.c.install_update();
            cx.notify();
        }
        if self.system_poll.elapsed() > Duration::from_secs(2) {
            self.system_poll = Instant::now();
            let choice = self.c.choice("theme", "跟随系统");
            if choice != self.theme_choice || choice == "跟随系统" {
                self.theme_choice = choice;
                self.theme(window, cx)
            }
            if self.c.smoke.is_none()
                && !self.c.closing
                && self.c.boolean("auto_update", true)
                && self.c.last_check.elapsed() > Duration::from_secs(21600)
            {
                self.c.check_update(true)
            }
        }
        if !self.c.closing {
            if self.c.smoke.is_none()
                && self.c.boolean("welcome_pending", false)
                && !self.dialog_active
            {
                self.help(window, cx);
            } else {
                self.dialogs(window, cx);
            }
        }
        if let Some(out) = self.c.smoke.clone() {
            let t = self.started.elapsed().as_secs_f64();
            let next = if self.c.boolean("smoke_update", false) {
                if t > self.c.state["smoke_hold_seconds"]
                    .as_f64()
                    .unwrap_or(4.)
                    .clamp(1., 600.)
                {
                    5
                } else {
                    0
                }
            } else if t > 3.5 {
                5
            } else if t > 2.5 {
                4
            } else if t > 1.8 {
                3
            } else if t > 1.2 {
                2
            } else if t > 0.6 {
                1
            } else {
                0
            };
            if next > self.smoke_step {
                self.smoke_step = next;
                match next {
                    1 => self.help(window, cx),
                    2 => {
                        window.close_dialog(cx);
                        self.c.page = 1;
                    }
                    3 => self.c.page = 2,
                    4 => {
                        self.c.page = if window.viewport_size().width < px(880.) {
                            3
                        } else {
                            0
                        }
                    }
                    5 => {
                        let report = json!({"first_ui_ms":self.first_render_ms,"frames":self.frames,"heartbeats":self.ticks,"max_event_gap_ms":self.max_gap_ms,"language":self.c.tr.language,"dark":cx.theme().mode.is_dark(),"renderer":"GPUI D3D11","python_runtime":false,"scan_exercised":self.c.boolean("smoke_scan",false),"scan_busy_at_close":self.c.busy,"width":f32::from(window.viewport_size().width),"height":f32::from(window.viewport_size().height)});
                        self.c.channel.job(move |c| {
                            core::atomic_json(&out, &report)?;
                            c.send(Event::SmokeWritten);
                            Ok(())
                        });
                    }
                    _ => {}
                }
                cx.notify();
            }
        }
        self.c.tick_close();
        if self.c.saved {
            cx.quit();
        }
    }
}
impl Render for Manager {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.frames += 1;
        if self.first_render_ms.is_none() {
            self.first_render_ms = Some(self.started.elapsed().as_secs_f64() * 1000.)
        }
        let content = match self.c.page {
            1 => self.gpu_page(window, cx),
            2 => self.settings(window, cx),
            3 if window.viewport_size().width < px(880.) => self.details(window, cx),
            _ => self.home(window, cx),
        };
        div()
            .v_flex()
            .size_full()
            .min_h_0()
            .font_family("Microsoft YaHei UI")
            .text_size(px(14.))
            .bg(cx.theme().muted)
            .text_color(cx.theme().foreground)
            .child(
                TitleBar::new().child(
                    div()
                        .h_flex()
                        .gap_2()
                        .child(img("app/manager.png").size(px(16.)))
                        .child(format!(
                            "{} by小南瓜 · v{VERSION}",
                            self.c.text("RTX 帧生成管理器")
                        )),
                ),
            )
            .child(
                div()
                    .h_flex()
                    .items_stretch()
                    .flex_1()
                    .min_h_0()
                    .child(self.nav(window, cx))
                    .child(
                        div()
                            .v_flex()
                            .flex_1()
                            .min_w_0()
                            .min_h_0()
                            .p_4()
                            .child(self.header(window, cx))
                            .when(self.c.save_error.is_some(), |d| {
                                d.child(
                                    div()
                                        .h_flex()
                                        .gap_2()
                                        .py_2()
                                        .child(
                                            div()
                                                .flex_1()
                                                .text_xs()
                                                .text_color(cx.theme().warning)
                                                .child(
                                                    self.t("设置未保存，请检查目录权限后重试。"),
                                                ),
                                        )
                                        .child(
                                            self.button(
                                                "retry-save",
                                                "重试",
                                                Command::RetrySave,
                                                cx,
                                            )
                                            .small(),
                                        ),
                                )
                            })
                            .child(content)
                            .when(
                                self.c.page != 2 && self.c.update_busy && !self.c.checking,
                                |d| d.child(self.download_bar(cx)),
                            )
                            .when(self.c.closing, |d| d.child(self.t("正在安全结束操作…"))),
                    ),
            )
            .children(Root::render_dialog_layer(window, cx))
            .children(Root::render_sheet_layer(window, cx))
            .children(Root::render_notification_layer(window, cx))
    }
}
pub fn run(data: PathBuf, smoke: Option<PathBuf>) -> Result<()> {
    let data = core::no_links(&data)?;
    let _lock = win::instance_lock(&data)?;
    let (state, error) = match preferences::load_with_recovery(&data) {
        Ok(mut report) => {
            if let Some(warning) = report.warning {
                report.value["recovery_notice"] = json!(warning);
            }
            (report.value, None)
        }
        Err(e) => (
            json!({"schema":3,"games":[],"roots":[]}),
            Some(e.to_string()),
        ),
    };
    gpui_platform::application()
        .with_assets(crate::ui_assets::Assets)
        .run(move |cx| {
            gpui_component::init(cx);
            let mode = match state["theme"].as_str() {
                Some("深色") => ThemeMode::Dark,
                Some("浅色") => ThemeMode::Light,
                _ => {
                    if win::dark() {
                        ThemeMode::Dark
                    } else {
                        ThemeMode::Light
                    }
                }
            };
            Theme::change(mode, None, cx);
            brand_theme(cx);
            let pointer_monitor = win::launch_monitor();
            let display = cx
                .displays()
                .into_iter()
                .find(|d| Some(u64::from(d.id())) == pointer_monitor)
                .or_else(|| cx.primary_display());
            let work = display
                .as_ref()
                .map(|d| d.visible_bounds())
                .unwrap_or(Bounds::new(
                    point(px(0.), px(0.)),
                    size(px(1280.), px(900.)),
                ));
            let bounds = fitted_bounds(work);
            let window_size = bounds.size;
            let options = WindowOptions {
                titlebar: Some(TitlebarOptions {
                    title: Some("RTX 帧生成管理器 by小南瓜".into()),
                    appears_transparent: true,
                    ..Default::default()
                }),
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                display_id: display.map(|d| d.id()),
                window_min_size: Some(size(
                    px(620.).min(window_size.width),
                    px(460.).min(window_size.height),
                )),
                ..Default::default()
            };
            match cx.open_window(options, move |window, cx| {
                let view = cx.new(|cx| Manager::new(data, state, error, smoke, window, cx));
                let close = view.downgrade();
                window.on_window_should_close(cx, move |_, cx| {
                    close
                        .update(cx, |this, _| {
                            this.c.close();
                            false
                        })
                        .unwrap_or(true)
                });
                cx.new(|cx| Root::new(view, window, cx))
            }) {
                Ok(_) => {}
                Err(e) => {
                    win::message("RTX Manager", &format!("{e:#}"));
                    cx.quit();
                }
            }
        });
    Ok(())
}

fn brand_theme(cx: &mut App) {
    let t = Theme::global_mut(cx);
    t.font_family = "Microsoft YaHei UI".into();
    t.font_size = px(14.);
}

fn deployment_label(status: &str) -> (String, TagVariant) {
    if let Some(rest) = status.strip_prefix("已部署 ") {
        let mut parts = rest.split(" / ");
        let profile = parts.next().unwrap_or_default();
        let saved_version = profile.split_once(" @").map(|(_, v)| v);
        let version = if profile.starts_with("NATIVE_X6") {
            format!("{} 5X/6X", saved_version.unwrap_or("0.2.4"))
        } else if profile.starts_with("NATIVE") || profile.starts_with("UPSTREAM") {
            saved_version.unwrap_or("0.2.5").to_string()
        } else {
            "R2".into()
        };
        (
            format!(
                "已部署 · {} · {}",
                version,
                parts.next().unwrap_or_default()
            ),
            TagVariant::Success,
        )
    } else if status == "未部署" {
        (status.into(), TagVariant::Secondary)
    } else if status.starts_with("补丁已移除，缓存待清理") {
        ("补丁已移除，缓存待清理".into(), TagVariant::Warning)
    } else if status.starts_with("正在") {
        (status.into(), TagVariant::Info)
    } else {
        ("需检查".into(), TagVariant::Warning)
    }
}
fn fitted_bounds(work: Bounds<Pixels>) -> Bounds<Pixels> {
    let width = px(1200.).min((work.size.width - px(32.)).max(px(1.)));
    let height = px(790.).min((work.size.height - px(32.)).max(px(1.)));
    Bounds::centered_at(work.center(), size(width, height))
}

#[cfg(test)]
mod layout_tests {
    use super::{deployment_label, fitted_bounds};
    use gpui::{Bounds, point, px, size};
    use gpui_component::tag::TagVariant;
    #[test]
    fn launch_fits_and_centers_in_scaled_work_areas() {
        for scale in [1.0, 1.25, 1.5, 2.0, 3.0] {
            for (w, h) in [
                (800., 600.),
                (1366., 768.),
                (1920., 1080.),
                (2560., 1440.),
                (3840., 2160.),
            ] {
                for x in [-2560., 0., 1920.] {
                    let work = Bounds::new(
                        point(px(x), px(-100.)),
                        size(px(w / scale), px((h - 48.) / scale)),
                    );
                    let b = fitted_bounds(work);
                    assert_eq!(b.center(), work.center());
                    assert!(b.size.width <= work.size.width && b.size.height <= work.size.height);
                    assert!(b.origin.x >= work.origin.x && b.origin.y >= work.origin.y);
                }
            }
        }
    }
    #[test]
    fn deployment_tags_identify_scheme_and_never_mark_unknown_success() {
        for (s, label) in [
            (
                "已部署 NATIVE_SM86 / version.dll",
                "已部署 · 0.2.5 · version.dll",
            ),
            (
                "已部署 NATIVE_X6_SM75 / winmm.dll, dxgi.dll",
                "已部署 · 0.2.4 5X/6X · winmm.dll, dxgi.dll",
            ),
            ("已部署 SM75_R2 / version.dll", "已部署 · R2 · version.dll"),
            (
                "已部署 NATIVE30 @0.2.6 / version.dll",
                "已部署 · 0.2.6 · version.dll",
            ),
            (
                "已部署 NATIVE_X6_20 @0.2.6 / winmm.dll",
                "已部署 · 0.2.6 5X/6X · winmm.dll",
            ),
        ] {
            let (text, tag) = deployment_label(s);
            assert_eq!(text, label);
            assert!(matches!(tag, TagVariant::Success));
        }
        assert!(matches!(
            deployment_label("未知 DLL").1,
            TagVariant::Warning
        ));
        assert!(matches!(deployment_label("正在安装…").1, TagVariant::Info));
        assert!(matches!(
            deployment_label("未部署").1,
            TagVariant::Secondary
        ));
    }
}
