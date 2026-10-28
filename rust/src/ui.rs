//! Native GPUI frontend. Disk, registry, scanning and networking stay in workers.
use crate::controller::{Controller, Event};
use anyhow::Result;
use gpui::component as gpui_component;
use gpui::{prelude::*, *};
use gpui_component::{
    accordion::Accordion,
    alert::Alert,
    button::{Button, ButtonCustomVariant, ButtonVariant, ButtonVariants},
    checkbox::Checkbox,
    command::{Command as ThemeCommand, CommandItem, CommandState},
    dialog::{Cancel, Confirm, DialogButtonProps, DialogFooter},
    input::{Input, InputEvent, InputState},
    menu::{ContextMenuExt, DropdownMenu, PopupMenuItem},
    resizable::{ResizableState, h_resizable, resizable_panel},
    select::{Select, SelectEvent, SelectItem, SelectState},
    slider::{Slider, SliderEvent, SliderState},
    switch::Switch,
    tag::{Tag, TagVariant},
    theme::ThemeMode,
    tooltip::Tooltip,
    *,
};
use rtx_fg_manager::{VERSION, core, gpu_alias, hags, i18n, preferences, win};
use serde_json::{Value, json};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    path::PathBuf,
    rc::Rc,
    sync::atomic::Ordering,
    time::{Duration, Instant},
};

#[path = "ui_encore.rs"]
mod encore_editor;
#[path = "ui_game_name.rs"]
mod game_name_editor;
#[path = "ui_recommendation_entry.rs"]
mod recommendation_entry;
#[path = "ui_recommendations.rs"]
mod recommendations;
#[path = "ui_runtime_files.rs"]
mod runtime_files;
#[path = "ui_settings.rs"]
mod settings;
#[path = "ui_theme_preview.rs"]
mod theme_preview;

#[derive(Clone, PartialEq)]
enum Command {
    Page(u8),
    ToggleSidebar,
    FontSize(u8),
    ConfigHelp,
    Editor(String),
    ClearCache,
    Scan,
    ScanRoots(Vec<PathBuf>),
    Pick(bool),
    CustomFolder,
    ReleaseNotes,
    Refresh,
    RefreshGpu,
    GpuEditor,
    RenameGame(String),
    Recommendations(String),
    UpdateGameNames,
    NeuralVersion(&'static str),
    InstallNeural(&'static str),
    RemoveNeural,
    RuntimeFiles,
    RefreshRuntimeFiles,
    RuntimeOperation(&'static str, bool),
    RuntimeVersion(&'static str, String),
    RetryCleanup,
    CleanupDetails,
    ReviewGame(String),
    RecoverGame,
    CloseReview,
    RefreshReview,
    DeleteCleanup,
    CleanupLocation(PathBuf),
    RecheckBlocked,
    All(bool),
    Game(String, bool),
    GameOption(String, String),
    ApplyPreset,
    KeepEditing,
    ClosePresetWithoutApplying,
    RetrySave,
    Focus(String),
    Proxy(String, bool),
    Scheme(usize),
    Language(String),
    Themes,
    Motion(&'static str),
    Source(String),
    Boolean(String, bool),
    CheckUpdate,
    Download,
    Cancel,
    CancelUpdate,
    Hags(bool),
    RefreshHags,
    Device(usize),
    Alias(usize),
    Confirm(String),
    Commit(String),
    Patch(bool),
    Open(String),
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
#[derive(Default)]
struct GameFilter {
    revision: Option<u64>,
    query: String,
    indices: Vec<usize>,
}
impl GameFilter {
    fn refresh(&mut self, c: &Controller) {
        if self.revision == Some(c.library_revision) && self.query == c.search {
            return;
        }
        let query = c.search.to_lowercase();
        self.indices = c
            .games
            .iter()
            .enumerate()
            .filter(|(_, g)| {
                g.exe.to_lowercase().contains(&query)
                    || rtx_fg_manager::game_names::matches_query(g, &c.game_names, &query)
                    || g.root.to_lowercase().contains(&query)
            })
            .map(|(i, _)| i)
            .collect();
        self.revision = Some(c.library_revision);
        self.query = c.search.clone();
    }
}
/// Display only current deployment locations, never cleanup-only remnants.
/// Keep scanner order and collapse alternate executables in the same folder.
fn game_directories(game: &rtx_fg_manager::scanner::Game) -> Vec<String> {
    game.deployment_targets()
        .iter()
        .map(|target| target.directory.display().to_string())
        .collect()
}
pub struct Manager {
    drag_gap: Option<(String, bool)>,
    folder_feedback: Option<(String, Instant)>,
    choices: RefCell<HashMap<(WindowId, &'static str), ChoiceBinding>>,
    encore_editor: RefCell<Option<(String, Entity<encore_editor::EncoreEditor>)>>,
    encore_invalid: Rc<Cell<bool>>,
    c: Controller,
    search: Entity<InputState>,
    hags_focus: FocusHandle,
    font_slider: Entity<SliderState>,
    _font_subscription: Subscription,
    _subscription: Subscription,
    split_state: Entity<ResizableState>,
    _split_subscription: Subscription,
    scroll: ListState,
    filtered: GameFilter,
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
    utility_opening: bool,
    runtime_window: Option<AnyWindowHandle>,
    settings_window: Option<AnyWindowHandle>,
    recommendation_windows: HashMap<String, AnyWindowHandle>,
    feedback_window: Option<AnyWindowHandle>,
    main_window: AnyWindowHandle,
    runtime_game: Option<String>,
    runtime_versions: HashMap<&'static str, String>,
    runtime_feedback: Option<(bool, String)>,
    editor_open: bool,
    preset_close: Option<bool>, // true: app exit; false: editor dismissal
    theme_preview: theme_preview::ThemePreview<WindowId>,
}
/// Render after Manager::render has released its entity borrow. Building a
/// live editor inside Root::render_dialog_layer would recursively borrow Manager.
struct GameEditor {
    owner: WeakEntity<Manager>,
}
struct GpuEditor {
    owner: WeakEntity<Manager>,
}
impl Render for GpuEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.owner
            .update(cx, |owner, cx| owner.gpu_editor_body(window, cx))
            .unwrap_or_else(|_| div().into_any_element())
    }
}
struct CleanupDetailsView {
    owner: WeakEntity<Manager>,
    snapshot: rtx_fg_manager::deployment::Snapshot,
    details_open: bool,
}
impl Render for CleanupDetailsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity().downgrade();
        self.owner
            .update(cx, |owner, cx| {
                owner.cleanup_body(&self.snapshot, self.details_open, view, window, cx)
            })
            .unwrap_or_else(|_| div().into_any_element())
    }
}
#[derive(Clone)]
struct GameDrag {
    exe: String,
    title: String,
}
impl Render for GameDrag {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_3()
            .py_2()
            .rounded_md()
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .text_sm()
            .max_w(rems(24.))
            .truncate()
            .child(self.title.clone())
    }
}
impl Render for GameEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.owner
            .update(cx, |owner, cx| owner.game_editor_body(window, cx))
            .unwrap_or_else(|_| div().into_any_element())
    }
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
        crate::ui_focus::observe(cx);
        cx.observe_window_activation(window, |_, _, cx| cx.notify())
            .detach();
        cx.set_reduce_motion(preferred_reduce_motion(&state, win::reduced_motion()));
        let mut c = Controller::new(data, state, error, smoke);
        if !c.boolean("sidebar_compact_default_v3", false) {
            c.state["sidebar_collapsed"] = json!(true);
            c.state["sidebar_compact_default_v3"] = json!(true);
            c.save();
        }
        let font_size = preferred_font_size(&c.state);
        Theme::update(cx, |theme| {
            theme.font_size = px(font_size * 8. / 7.);
            // Keep a thin focus edge without the outward halo in every window.
            theme.focus_ring = false;
        });
        let font_slider = cx.new(|_| {
            SliderState::new()
                .min(12.)
                .max(18.)
                .step(1.)
                .default_value(font_size)
        });
        let font_window = window.window_handle();
        let font_subscription = cx.subscribe(&font_slider, move |_, _, event, cx| {
            if let SliderEvent::Change(value) = event {
                let size = value.end().round() as u8;
                let owner = cx.entity().downgrade();
                cx.defer(move |cx| {
                    let _ = font_window.update(cx, |_, window, cx| {
                        let _ = owner.update(cx, |owner, cx| owner.set_font_size(size, window, cx));
                    });
                });
            }
        });
        let search = cx.new(|cx| InputState::new(window, cx).placeholder(c.text("搜索游戏")));
        let subscription = cx.subscribe(&search, |this, state, event, cx| {
            if matches!(event, InputEvent::Change) {
                this.c.search = state.read(cx).value().to_string();
                this.filter();
                cx.notify();
            }
        });
        let split_state = cx.new(|_| ResizableState::default());
        let split_subscription = cx.observe(&split_state, |this, state, cx| {
            let Some(width) = state.read(cx).sizes().get(1).copied() else {
                return;
            };
            let width = f32::from(width);
            let saved = this.c.state["inspector_width"].as_f64().unwrap_or(384.) as f32;
            if width.is_finite() && width >= 320. && (width - saved).abs() > 0.5 {
                this.c.state["inspector_width"] = json!(width);
                this.c.save();
            }
        });
        let theme_choice = c.choice("theme", "跟随系统");
        let language = c.tr.language.clone();
        let mut view = Self {
            folder_feedback: None,
            choices: RefCell::new(HashMap::new()),
            encore_editor: RefCell::new(None),
            encore_invalid: Rc::new(Cell::new(false)),
            c,
            search,
            hags_focus: cx.focus_handle(),
            font_slider,
            _font_subscription: font_subscription,
            _subscription: subscription,
            split_state,
            _split_subscription: split_subscription,
            scroll: ListState::new(0, ListAlignment::Top, px(200.)),
            filtered: GameFilter::default(),
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
            utility_opening: false,
            runtime_window: None,
            settings_window: None,
            recommendation_windows: HashMap::new(),
            feedback_window: None,
            main_window: window.window_handle(),
            runtime_game: None,
            runtime_versions: HashMap::new(),
            runtime_feedback: None,
            drag_gap: None,
            editor_open: false,
            preset_close: None,
            theme_preview: theme_preview::ThemePreview::new(),
        };
        view.filter();
        view.theme(cx);
        let owner = cx.entity().downgrade();
        cx.on_window_closed(move |cx, closed| {
            let owner = owner.clone();
            // Native close notifications can arrive inside another entity's
            // update. Release that lease before touching the owning view.
            cx.defer(move |cx| {
                let _ = owner.update(cx, |this, cx| {
                    if this.theme_preview.window_closed(closed) {
                        this.dialog_active = false;
                        this.theme(cx);
                    }
                    this.choices.borrow_mut().retain(|(id, _), _| *id != closed);
                    this.recommendation_windows
                        .retain(|_, w| w.window_id() != closed);
                    if this
                        .feedback_window
                        .is_some_and(|w| w.window_id() == closed)
                    {
                        this.feedback_window = None;
                    }
                    if this.runtime_window.is_some_and(|w| w.window_id() == closed) {
                        this.clear_runtime_window(cx);
                    }
                    if this
                        .settings_window
                        .is_some_and(|w| w.window_id() == closed)
                    {
                        this.clear_settings_window(cx);
                    }
                });
            });
        })
        .detach();
        cx.spawn_in(window, async move |this, cx| {
            loop {
                smol::Timer::after(Duration::from_millis(20)).await;
                if cx
                    .update(|window, cx| this.update(cx, |this, cx| this.tick(window, cx)))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        // Exit progress belongs to the application, not a utility/main-window
        // update lease. On Windows a closing or occluded window may stop driving
        // its own frame work while another native window is active.
        cx.spawn(async move |this, cx| {
            loop {
                smol::Timer::after(Duration::from_millis(50)).await;
                let result = this.update(cx, |this, cx| {
                    if this.c.closing {
                        this.c.events();
                        this.c.tick_close();
                        if this.c.saved {
                            cx.quit();
                        } else {
                            cx.notify();
                        }
                    }
                });
                if result.is_err() {
                    break;
                }
            }
        })
        .detach();
        view
    }
    fn scale(&self) -> f32 {
        preferred_font_size(&self.c.state) / 14.
    }
    fn layout(&self, window: &Window) -> LayoutMetrics {
        LayoutMetrics::new(
            f32::from(window.viewport_size().width),
            self.scale(),
            self.c.boolean("sidebar_collapsed", true),
        )
    }
    fn set_font_size(&mut self, size: u8, window: &mut Window, cx: &mut Context<Self>) {
        let size = size.clamp(12, 18);
        if preferred_font_size(&self.c.state) == size as f32 {
            return;
        }
        self.c.state["ui_font_size"] = json!(size);
        self.font_slider
            .update(cx, |slider, cx| slider.set_value(size as f32, window, cx));
        Theme::update(cx, |theme| theme.font_size = px(size as f32 * 8. / 7.));
        self.scroll.remeasure();
        self.c.save();
        cx.notify();
    }
    fn t(&self, s: &str) -> SharedString {
        self.c.text(s).into()
    }
    fn filter(&mut self) {
        if self.filtered.revision == Some(self.c.library_revision)
            && self.filtered.query == self.c.search
        {
            return;
        }
        let query_changed = self.filtered.query != self.c.search;
        let previous = self.scroll.logical_scroll_top();
        self.filtered.refresh(&self.c);
        let count = self.filtered.indices.len();
        if query_changed {
            self.scroll.reset(count);
        } else if self.scroll.item_count() != count {
            self.scroll.splice(0..self.scroll.item_count(), count);
            self.scroll.scroll_to(previous);
        } else {
            // Rescans can add directories without changing the number of games.
            self.scroll.remeasure();
        }
    }
    fn theme(&mut self, cx: &mut Context<Self>) {
        if self.theme_preview.is_active() {
            return;
        }
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
        let mode_changed = crate::ui_themes::apply(
            mode,
            &self.c.choice("ui_theme_light", "Default Light"),
            &self.c.choice("ui_theme_dark", "Default Dark"),
            cx,
        );
        let locale = match self.c.tr.language.as_str() {
            "zh-CN" => "zh-CN",
            "ja" => "ja",
            "ko" => "ko",
            "ru" => "ru",
            _ => "en",
        };
        let locale_changed = &*gpui_component::locale() != locale;
        if locale_changed {
            gpui_component::set_locale(locale);
        }
        if mode_changed || locale_changed {
            cx.notify();
        }
    }
    fn act(&mut self, command: Command, window: &mut Window, cx: &mut Context<Self>) {
        if self.c.closing || self.utility_opening {
            return;
        }
        self.reconcile_runtime_window(cx);
        self.reconcile_settings_window(cx);
        self.reconcile_dialogs(window, cx);
        if self.dialog_active && !window.has_active_dialog(cx) {
            // Only an actual modal owns interaction, never an open utility.
            if let Some(handle) = self.feedback_window {
                cx.defer(move |cx| {
                    let _ = handle.update(cx, |_, w, _| w.activate_window());
                });
            }
            return;
        }
        if !self.c.busy {
            self.feedback_window = Some(window.window_handle());
        }
        // Utility actions are bound to the game shown in that utility. Explicit
        // activation selects that game in the main view too; opening another
        // window or changing the library selection never silently retargets it.
        if self
            .runtime_window
            .is_some_and(|w| w.window_id() == window.window_handle().window_id())
            && matches!(
                command,
                Command::InstallNeural(_)
                    | Command::RuntimeOperation(..)
                    | Command::RefreshRuntimeFiles
                    | Command::Confirm(_)
                    | Command::Commit(_)
            )
            && let Some(exe) = self.runtime_game.clone()
        {
            if !self.c.games.iter().any(|g| g.exe == exe) {
                self.runtime_feedback =
                    Some((true, self.c.text("游戏已移除，请关闭此窗口后重新选择游戏")));
                cx.notify();
                return;
            }
            self.c.focus = Some(exe);
        }
        if self.encore_invalid.get()
            && self.c.parameter_profile() == rtx_fg_manager::encore::PROFILE
            && self
                .encore_editor
                .borrow()
                .as_ref()
                .is_some_and(|(exe, _)| self.c.focus.as_ref() == Some(exe))
            && (matches!(command, Command::ApplyPreset | Command::Patch(false))
                || matches!(&command, Command::Commit(action) if action == "deploy_batch"))
        {
            self.c.warning = Some(
                self.c
                    .text("请先修正高级参数中的无效输入，或恢复该项默认值。"),
            );
            cx.notify();
            return;
        }
        match command {
            Command::KeepEditing => self.preset_close = None,
            Command::ClosePresetWithoutApplying => {
                let exit = self.preset_close.take() == Some(true);
                let target = self.c.focus.clone().map(|exe| (exe, self.c.cloud_scheme()));
                self.c.discard_presets(if exit {
                    None
                } else {
                    target.as_ref().map(|(e, s)| (e.as_str(), s.as_str()))
                });
                self.encore_editor.borrow_mut().take();
                self.encore_invalid.set(false);
                self.editor_open = false;
                self.dialog_active = false;
                window.close_dialog(cx);
                if exit {
                    self.c.close();
                }
            }
            Command::FontSize(size) => self.set_font_size(size, window, cx),
            Command::ToggleSidebar => {
                self.c.state["sidebar_collapsed"] =
                    json!(!self.c.boolean("sidebar_collapsed", true));
                self.c.save();
            }
            Command::Page(2) => self.open_settings(window, cx),
            Command::Page(p) => self.c.page = p,
            Command::GpuEditor => self.gpu_editor(window, cx),
            Command::RenameGame(exe) => self.rename_game(exe, window, cx),
            Command::Recommendations(exe) => self.recommendations(exe, window, cx),
            Command::UpdateGameNames => self.c.update_game_names(),
            Command::ConfigHelp => {
                if self.dialog_active {
                    self.c.state["preset_help_open"] =
                        json!(!self.c.boolean("preset_help_open", false));
                } else {
                    self.config_help(window, cx);
                }
            }
            Command::ClearCache => self.c.clear_cache(),
            Command::Refresh => {
                if let Some(exe) = self.c.focus.clone() {
                    self.c.evidence.remove(&exe);
                    self.c.inspect(exe);
                }
                self.c.refresh();
                self.c.identify_game_names();
            }
            Command::NeuralVersion(version) => {
                if let Some(exe) = self.c.focus.clone() {
                    self.c.neural_versions.insert(exe, version);
                }
            }
            Command::InstallNeural(version) => {
                self.runtime_feedback = None;
                self.c.install_neural_runtime(version);
            }
            Command::RuntimeFiles => self.runtime_dialog(window, cx),
            Command::RefreshRuntimeFiles => {
                if !self.c.busy {
                    self.c.refresh_runtime_catalog();
                    if let Some(exe) = self.c.focus.clone() {
                        self.c.evidence.remove(&exe);
                    }
                    self.c.refresh();
                }
            }
            Command::RuntimeVersion(kind, version) => {
                if !self.c.busy {
                    self.runtime_versions.insert(kind, version);
                }
            }
            Command::RuntimeOperation(kind, remove) => {
                self.runtime_feedback = None;
                let version = self
                    .runtime_versions
                    .get(kind)
                    .filter(|version| {
                        rtx_fg_manager::runtime_files::versions(kind).contains(version)
                    })
                    .cloned()
                    .unwrap_or_else(|| rtx_fg_manager::runtime_files::recommended(kind));
                self.c.runtime_operation(kind, remove, version);
            }
            Command::RecheckBlocked => {
                self.dialog_active = false;
                window.close_dialog(cx);
                self.c.recheck_blocked();
            }
            Command::RemoveNeural => {
                self.runtime_feedback = None;
                self.c.confirm = Some("remove_nr".into());
            }
            Command::RetryCleanup => {
                if self.dialog_active {
                    self.dialog_active = false;
                    window.close_dialog(cx);
                }
                self.c.retry_focused_cleanup();
            }
            Command::CleanupDetails => self.cleanup_dialog(window, cx),
            Command::ReviewGame(exe) => {
                if !self.c.busy {
                    self.c.focus = Some(exe);
                    self.cleanup_dialog(window, cx);
                }
            }
            Command::RecoverGame => {
                self.dialog_active = false;
                window.close_dialog(cx);
                self.c.request_focused_recovery();
            }
            Command::CloseReview => {
                self.dialog_active = false;
                window.close_dialog(cx);
            }
            Command::RefreshReview => {
                self.dialog_active = false;
                window.close_dialog(cx);
                self.c.refresh();
            }
            Command::DeleteCleanup => {
                self.dialog_active = false;
                window.close_dialog(cx);
                self.c.preview_cleanup_deletion();
            }
            Command::CleanupLocation(path) => {
                self.c.channel.job(move |_| {
                    let path = core::no_links(&path)?;
                    let folder = if path.is_dir() {
                        path
                    } else {
                        path.parent().unwrap_or(&path).to_path_buf()
                    };
                    anyhow::ensure!(folder.is_dir(), "目录已不存在，请点击重新清理更新状态");
                    win::open(&folder.to_string_lossy())
                });
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
            Command::CustomFolder => self.c.pick_custom_folder(),
            Command::ReleaseNotes => self.release_notes(window, cx),
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
                    self.c.focus = Some(exe.clone());
                    self.c.inspect(exe.clone());
                } else {
                    self.c.selected.remove(&exe);
                }
            }
            Command::GameOption(key, value) => {
                if let Some(exe) = self.c.focus.clone() {
                    self.c.set_game_option(&exe, &key, &value);
                }
            }
            Command::Editor(exe) => {
                if !self.c.busy {
                    self.c.focus = Some(exe.clone());
                    self.c.inspect(exe);
                    self.game_editor(window, cx);
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
                        // Plugin mode and DLL mode are distinct loading methods.
                        if name.ends_with(".asi") {
                            p.clear();
                        } else {
                            p.retain(|n| !n.ends_with(".asi"));
                        }
                        p.push(name)
                    } else if !yes {
                        p.retain(|p| *p != name)
                    }
                    if p.is_empty() {
                        p.push("version.dll".into())
                    }
                    if let Ok(p) = core::normalize_proxies(&p) {
                        self.c
                            .set_selection(self.c.cloud_scheme(), self.c.cloud_series(), p);
                    }
                }
            }
            Command::Scheme(i) => {
                let (_, series) = self.profile();
                self.profile_set(i, series)
            }

            Command::Language(s) => {
                self.c.language(&s);
                self.language = self.c.tr.language.clone();
                self.search.update(cx, |state, cx| {
                    state.set_placeholder(self.c.text("搜索游戏"), window, cx)
                });
                self.theme(cx)
            }
            Command::Themes => self.open_themes(window, cx),
            Command::Motion(mode) => {
                self.c.state["ui_motion"] = json!(mode);
                cx.set_reduce_motion(preferred_reduce_motion(
                    &self.c.state,
                    win::reduced_motion(),
                ));
                self.c.save();
            }
            Command::ApplyPreset => self.c.apply_focused_parameters(),
            Command::RetrySave => self.c.save(),
            Command::Source(s) => {
                self.c.state["download_source"] = json!(s);
                self.c.state["cloud_source"] = json!(s);
                self.c.state["update_source"] =
                    json!(if s == "github" { "github" } else { "gitee" });
                self.c.save();
            }
            Command::Boolean(k, v) => {
                self.c.state[&k] = json!(v);
                if k == "chinese_game_names" {
                    self.c.library_revision += 1;
                }
                self.c.save()
            }
            Command::CheckUpdate => self.c.check_update(false),
            Command::Download => self.c.download_update(),
            Command::Cancel => self.c.cancel.store(true, Ordering::Relaxed),
            Command::CancelUpdate => self.c.cancel_update(),
            Command::Hags(enabled) => self.c.set_hags(enabled),
            Command::RefreshHags => self.c.refresh_hags(),
            Command::Device(i) => self.c.device_index = i,
            Command::Alias(i) => self.c.alias_index = i,
            Command::Confirm(action) => {
                if matches!(action.as_str(), "alias" | "restore") {
                    // Move to the existing explicit confirmation; opening the
                    // editor and choosing a model never writes the registry.
                    window.close_dialog(cx);
                    self.dialog_active = false;
                }
                if action == "remove" && self.c.can_forget_focused_without_confirmation() {
                    self.commit(&action);
                } else {
                    self.c.confirm = Some(action);
                }
            }
            Command::Commit(action) => self.commit(&action),
            Command::Patch(clean) => self.c.request_patch(clean),
            Command::Open(s) => self.c.channel.job(move |_| win::open(&s)),
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
        let id = p.scheme_id.clone();
        if !self.c.catalog.supports_series(&id, series) {
            self.c.notice = Some(self.c.text("此方案不支持当前显卡系列，请选择兼容方案。"));
            return;
        }
        let allowed = self.c.catalog.proxies(&id);
        let proxies = if id == self.c.cloud_scheme() {
            self.c.proxies()
        } else {
            self.c
                .focus
                .as_ref()
                .and_then(|exe| self.c.games.iter().find(|g| &g.exe == exe))
                .and_then(|g| g.extra.get("proxy_choices"))
                .and_then(|v| v.get(&id))
                .and_then(|v| serde_json::from_value::<Vec<String>>(v.clone()).ok())
                .filter(|p| {
                    !p.is_empty()
                        && p.len() <= self.c.catalog.selection_limit(&id)
                        && p.iter().all(|n| allowed.contains(n))
                })
                .unwrap_or_else(|| vec!["version.dll".into()])
        };
        self.c.set_selection(id, series, proxies);
    }
    fn commit(&mut self, action: &str) {
        match action {
            "deploy_batch" | "clean_batch" | "recover_patch" => self.c.confirm_patch(),
            "remove_nr" => self.c.remove_neural_runtime(),
            "delete_cleanup" => self.c.confirm_cleanup_deletion(),
            "end_processes" => self.c.end_blocked_and_retry(),
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
            .focus_ring(crate::ui_focus::visible(cx))
            .small()
            .label(self.t(label))
            .on_click(cx.listener(move |this, _, window, cx| {
                cx.stop_propagation();
                this.act(action.clone(), window, cx)
            }))
    }
    fn brand_logo(size: Rems, cx: &App) -> Div {
        div()
            .size(size)
            .flex_shrink_0()
            .relative()
            .child(
                svg()
                    .path("app/manager-backplate.svg")
                    .size_full()
                    .text_color(cx.theme().foreground),
            )
            .child(
                div().absolute().inset_0().h_flex().justify_center().child(
                    Icon::new(IconName::Cpu)
                        .size(size * 0.625)
                        .text_color(cx.theme().background),
                ),
            )
    }
    fn icon(name: IconName) -> Icon {
        Icon::new(name).size(px(20.))
    }
    fn recommendation_entry(&self, exe: String, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let animate = !self.dialog_active && !cx.reduce_motion();
        let button = self
            .button(
                "game-recommendations",
                "玩家推荐方案",
                Command::Recommendations(exe),
                cx,
            )
            .custom(
                ButtonCustomVariant::new(cx)
                    .color(theme.background)
                    .foreground(theme.foreground)
                    .hover(theme.secondary_hover)
                    .active(theme.secondary_active),
            )
            .w_full();
        // A single animation clock paints the border and icon together. Visible,
        // unfocused windows keep moving without requiring mouse hover.
        let entry = recommendation_entry::RecommendationEntry::new(button, animate);
        if animate {
            entry
                .with_animation(
                    "recommendation-entry-glow",
                    Animation::new(Duration::from_millis(4800))
                        .repeat_synced()
                        .with_max_fps(30.),
                    |entry, phase| entry.phase(phase),
                )
                .into_any_element()
        } else {
            entry.into_any_element()
        }
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
        let key = (window.window_handle().window_id(), id);
        let binding = choices.entry(key).or_insert_with(|| {
            let state = cx.new(|cx| SelectState::new(options(), selected, window, cx));
            let event_window = window.window_handle();
            let subscription = cx.subscribe(&state, move |this, _, event, cx| {
                let SelectEvent::Confirm(value) = event;
                let action = value.and_then(|index| {
                    this.choices
                        .borrow()
                        .get(&key)
                        .and_then(|binding| binding.items.get(index))
                        .map(|(_, action, _)| action.clone())
                });
                if let Some(action) = action {
                    let owner = cx.entity().downgrade();
                    // The shared Manager can be observed by several windows.
                    // Resolve the exact emitting window, not Manager's current
                    // entity-to-window association in GPUI.
                    cx.defer(move |cx| {
                        let _ = event_window.update(cx, |_, window, cx| {
                            let _ = owner.update(cx, |owner, cx| owner.act(action, window, cx));
                        });
                    });
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
        let state = binding.state.clone();
        div()
            .id(SharedString::from(format!("select-surface-{id}")))
            .w_full()
            .on_mouse_down_out(move |_, window, cx| {
                let state = state.clone();
                // Outside pointer dismissal must not leave the restored trigger focused.
                // Defer until the popup and any newly clicked control have handled the event.
                window.defer(cx, move |window, cx| {
                    if state.focus_handle(cx).is_focused(window) {
                        window.blur(cx);
                    }
                });
            })
            .child(
                Select::new(&binding.state)
                    .focus_ring(crate::ui_focus::visible(cx))
                    .placeholder(label)
                    .disabled(disabled)
                    .w_full()
                    .menu_max_h(px(300.)),
            )
            .into_any_element()
    }
    fn nav(&self, window: &Window, cx: &Context<Self>) -> AnyElement {
        let layout = self.layout(window);
        let collapsed = layout.collapsed;
        let mut sidebar = div()
            .v_flex()
            .h_full()
            .flex_shrink_0()
            .w(px(layout.nav))
            .pt_3()
            .px_2()
            .gap_3()
            .bg(cx.theme().sidebar)
            .text_color(cx.theme().sidebar_foreground)
            .border_r_1()
            .border_color(cx.theme().sidebar_border)
            .child(
                div()
                    .h_flex()
                    .h(rems(2.5))
                    .w_full()
                    .gap_2()
                    .when(collapsed, |d| d.justify_center())
                    .when(!collapsed, |d| d.px_1())
                    .child(Self::brand_logo(rems(2.5), cx))
                    .when(!collapsed, |d| {
                        d.child(div().text_sm().whitespace_nowrap().child("RTX-FG-Manager"))
                    }),
            );
        let mut navigation = div().v_flex().w_full().gap_1();
        for (page, icon, label) in [
            (0, IconName::LayoutDashboard, "游戏"),
            (2, IconName::Settings, "设置"),
        ] {
            let label = self.t(label);
            navigation = navigation.child(
                Button::new(("navigation", page as usize))
                    .focus_ring(crate::ui_focus::visible(cx))
                    .small()
                    .ghost()
                    .h_8()
                    .w_full()
                    .accessibility_label(label.clone())
                    .child(
                        div()
                            .h_flex()
                            .w_full()
                            .gap_2()
                            .when(collapsed, |d| d.justify_center())
                            .child(Self::icon(icon).size_5().flex_shrink_0())
                            .when(!collapsed, |d| {
                                d.child(div().text_sm().child(label.clone()))
                            }),
                    )
                    .selected(if page == 2 {
                        self.settings_window.is_some()
                    } else {
                        self.c.page == page
                    })
                    .when(collapsed, |b| b.tooltip(label.clone()))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.act(Command::Page(page), window, cx)
                    })),
            );
        }
        sidebar = sidebar.child(navigation);
        sidebar.into_any_element()
    }

    fn finish_theme_preview(&mut self, generation: u64, cx: &mut Context<Self>) {
        if !self.theme_preview.finish(generation) {
            return;
        }
        self.dialog_active = false;
        self.theme(cx);
        cx.notify();
    }

    fn open_themes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.dialog_active {
            return;
        }
        let choices = Rc::new(crate::ui_themes::choices(cx));
        let system = self.c.choice("theme", "跟随系统") == "跟随系统";
        let active = cx.theme().theme_name().to_string();
        let items = choices
            .iter()
            .map(|choice| {
                let (name, badge, checked) = match choice {
                    crate::ui_themes::Choice::System => {
                        (self.t("跟随系统"), self.t("系统"), system)
                    }
                    crate::ui_themes::Choice::Named { mode, name } => (
                        name.clone().into(),
                        self.t(if mode.is_dark() { "深色" } else { "浅色" }),
                        !system && name == &active,
                    ),
                };
                CommandItem::new()
                    .label(name.clone())
                    .keywords([badge.clone()])
                    .child(move |_, cx| {
                        div()
                            .h_flex()
                            .w_full()
                            .gap_2()
                            .items_center()
                            .child(div().flex_1().min_w_0().truncate().child(name.clone()))
                            .child(Tag::new().outline().small().child(badge.clone()))
                            .child(div().w(px(16.)).when(checked, |d| {
                                d.child(
                                    Icon::new(IconName::Check)
                                        .size(px(14.))
                                        .text_color(cx.theme().primary),
                                )
                            }))
                    })
            })
            .collect::<Vec<_>>();
        let palette = cx.new(|cx| CommandState::new(window, cx));
        let generation = self.theme_preview.begin(window.window_handle().window_id());
        self.dialog_active = true;
        let owner = cx.weak_entity();
        let focus = Rc::new(Cell::new(true));
        let placeholder = self.t("搜索主题…");
        let hint = self.t("移动预览 · 点击保存 · Esc 取消");
        window.open_dialog(cx, move |dialog, window, _| {
            let width = (window.viewport_size().width - px(40.)).min(px(460.));
            let close_owner = owner.clone();
            let content_owner = owner.clone();
            let choices = choices.clone();
            let items = items.clone();
            let palette = palette.clone();
            let focus = focus.clone();
            let placeholder = placeholder.clone();
            let hint = hint.clone();
            dialog
                .width(width)
                .p_0()
                .close_button(false)
                .overlay(false)
                .overlay_closable(true)
                .on_close(move |_, _, cx| {
                    let _ = close_owner
                        .update(cx, |this, cx| this.finish_theme_preview(generation, cx));
                })
                .content(move |content, window, cx| {
                    let height =
                        (window.viewport_size().height - px(180.)).clamp(px(140.), px(420.));
                    if focus.replace(false) {
                        let palette = palette.clone();
                        window.defer(cx, move |window, cx| {
                            palette.read(cx).focus_handle(cx).focus(window, cx)
                        });
                    }
                    let select_owner = content_owner.clone();
                    let confirm_owner = content_owner.clone();
                    let cancel_owner = content_owner.clone();
                    let outside_owner = content_owner.clone();
                    let selected_choices = choices.clone();
                    let confirmed_choices = choices.clone();
                    content.child(
                        div()
                            // Keep the page undimmed while previewing; a dialog
                            // without a backdrop needs its own outside dismissal.
                            .on_mouse_down_out(move |event, window, cx| {
                                if event.button != MouseButton::Left
                                    || event.position.y < gpui_component::TITLE_BAR_HEIGHT
                                {
                                    return;
                                }
                                let _ = outside_owner.update(cx, |this, cx| {
                                    if !this.theme_preview.matches(generation) {
                                        return;
                                    }
                                    this.finish_theme_preview(generation, cx);
                                    window.close_dialog(cx);
                                    cx.stop_propagation();
                                });
                            })
                            .capture_action(move |_: &Cancel, window, cx| {
                                let _ = cancel_owner.update(cx, |this, cx| {
                                    this.finish_theme_preview(generation, cx)
                                });
                                window.close_dialog(cx);
                                cx.stop_propagation();
                            })
                            .child(
                                ThemeCommand::new(&palette)
                                    .bordered(false)
                                    .placeholder(placeholder.clone())
                                    .max_h(height)
                                    .items(items.clone())
                                    .on_select(move |index, _, cx| {
                                        let _ = select_owner.update(cx, |this, cx| {
                                            if !this.theme_preview.matches(generation) {
                                                return;
                                            }
                                            if let Some(choice) = selected_choices.get(index.row) {
                                                choice.apply(&this.c.state, win::dark(), cx);
                                            }
                                        });
                                    })
                                    .on_confirm(move |index, window, cx| {
                                        let _ = confirm_owner.update(cx, |this, cx| {
                                            if !this.theme_preview.matches(generation) {
                                                return;
                                            }
                                            if let Some(choice) = confirmed_choices.get(index.row) {
                                                choice.persist(&mut this.c.state);
                                                this.c.save();
                                            }
                                            this.finish_theme_preview(generation, cx);
                                            window.close_dialog(cx);
                                        });
                                    })
                                    .footer({
                                        let hint = hint.clone();
                                        move |_, _, cx| {
                                            div()
                                                .px_3()
                                                .py_2()
                                                .text_xs()
                                                .text_color(cx.theme().muted_foreground)
                                                .child(hint.clone())
                                        }
                                    }),
                            ),
                    )
                })
        });
        cx.notify();
    }
    fn scan_button(&self, cx: &Context<Self>) -> AnyElement {
        let view = cx.entity().downgrade();
        let folder = self.t("选择目录扫描");
        let all = self.t("全盘扫描");
        crate::ui_focus::SplitButton::new(
            "scan-split",
            self.button("scan", "扫描", Command::Scan, cx)
                .icon(IconName::FolderOpen)
                .min_w(rems(5.)),
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
    fn add_game_button(&self, cx: &Context<Self>) -> AnyElement {
        let view = cx.entity().downgrade();
        let label = self.t("添加游戏文件夹（Xbox 等）");
        crate::ui_focus::SplitButton::new(
            "add-game-split",
            self.button("add", "添加游戏", Command::Pick(false), cx)
                .icon(IconName::Plus)
                .min_w(rems(5.)),
        )
        .disabled(self.c.busy)
        .dropdown_menu(move |menu, _, _| {
            let view = view.clone();
            menu.item(
                PopupMenuItem::new(label.clone()).on_click(move |_, window, cx| {
                    let _ = view.update(cx, |this, cx| this.act(Command::Pick(true), window, cx));
                }),
            )
        })
        .into_any_element()
    }
    fn cleanup_body(
        &self,
        snapshot: &rtx_fg_manager::deployment::Snapshot,
        details_open: bool,
        view: WeakEntity<CleanupDetailsView>,
        window: &Window,
        cx: &Context<Self>,
    ) -> AnyElement {
        use rtx_fg_manager::deployment::IssueKind;
        let mut summary = div().v_flex().gap_3();
        let mut kinds = Vec::new();
        for issue in snapshot.issues() {
            if !kinds.contains(&issue.kind()) {
                kinds.push(issue.kind());
                summary = summary.child(
                    div()
                        .v_flex()
                        .gap_1()
                        .child(
                            div()
                                .font_weight(FontWeight::MEDIUM)
                                .child(self.t(issue.kind().label())),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(self.t(issue.kind().help())),
                        ),
                );
            }
        }
        let pending = snapshot.cleanup_pending;
        let repair = kinds
            .iter()
            .any(|k| matches!(k, IssueKind::Interrupted | IssueKind::Modified));
        let mut details = div().v_flex().gap_3();
        for issue in snapshot.issues() {
            let path = issue.target().directory.clone();
            let id = core::key(&path);
            details = details.child(
                div()
                    .v_flex()
                    .gap_1()
                    .min_w_0()
                    .child(
                        div()
                            .text_sm()
                            .whitespace_normal()
                            .child(path.display().to_string()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .whitespace_normal()
                            .text_color(cx.theme().muted_foreground)
                            .child(self.t(issue.detail())),
                    )
                    .child(
                        self.button(
                            SharedString::from(format!("review-open:{id}")),
                            "打开位置",
                            Command::CleanupLocation(path),
                            cx,
                        )
                        .small()
                        .ghost(),
                    ),
            );
        }
        if pending {
            details = details.child(div().text_xs().whitespace_normal()
                .child(snapshot.cleanup_details.iter().map(|s| self.c.text(s)).collect::<Vec<_>>().join("\n\n")))
                .child(div().text_xs().child(self.t("常规清理仍有残留时，可查看管理器允许删除的具体文件，确认后再删除。共享目录和其他游戏文件会保留。")))
                .child(self.button("cleanup-preview", "查看可删除文件", Command::DeleteCleanup, cx).small().disabled(self.c.busy || self.c.read_only));
        }
        let name = self
            .c
            .focus
            .as_ref()
            .and_then(|e| self.c.games.iter().find(|g| &g.exe == e))
            .map(|g| self.c.game_name(g))
            .unwrap_or_default();
        div().v_flex().gap_3()
            .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child(name))
            .child(div().id("review-scroll").v_flex().min_h_0().gap_3()
                .max_h(window.viewport_size().height * 0.48).overflow_y_scroll()
                .child(summary)
                .when(pending, |body| body.child(div().text_sm().child(self.t("仅处理标记为待清理的目录。游戏存档、原文件备份、其他 MOD 和正常部署目录保持不变。"))))
                .child(Accordion::new("review-details").small()
                    .item(|item| item.title(self.t("技术详情与文件位置")).open(details_open).child(details))
                    .on_toggle_click(move |indices: &[usize], _, cx| {
                        let _ = view.update(cx, |this, cx| { this.details_open = !indices.is_empty(); cx.notify(); });
                    })))
            .child(div().h_flex().flex_wrap().justify_end().gap_2()
                .child(self.button("review-close", "关闭详情", Command::CloseReview, cx).small().ghost())
                .when(kinds.contains(&IssueKind::Unavailable), |buttons| buttons.child(self.button("review-refresh", "刷新", Command::RefreshReview, cx).small().disabled(self.c.busy)))
                .when(repair, |buttons| buttons.child(self.button("review-recover", "卸载并恢复", Command::RecoverGame, cx).small().disabled(self.c.busy)))
                .when(pending, |buttons| buttons.child(self.button("cleanup-retry-dialog", "清理残留", Command::RetryCleanup, cx).small().primary().disabled(self.c.busy))))
            .into_any_element()
    }
    fn cleanup_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.dialog_active {
            return;
        }
        let Some(snapshot) = self
            .c
            .focus
            .as_ref()
            .and_then(|e| self.c.deployments.get(e))
            .cloned()
        else {
            return;
        };
        self.dialog_active = true;
        let view = cx.entity().downgrade();
        let content = cx.new(|cx| {
            crate::ui_focus::observe(cx);
            CleanupDetailsView {
                owner: view.clone(),
                snapshot,
                details_open: false,
            }
        });
        let title = self.t("处理游戏状态");
        let width = (window.viewport_size().width - px(64.)).min(window.rem_size() * 40.);
        window.open_dialog(cx, move |dialog, _, cx| {
            let view = view.clone();
            dialog
                .title(crate::ui_focus::dialog_title(title.clone(), cx))
                .close_button(false)
                .width(width)
                .child(content.clone())
                .on_close(move |_, _, cx| {
                    let _ = view.update(cx, |this, cx| {
                        this.dialog_active = false;
                        cx.notify();
                    });
                })
        });
    }
    fn library(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut card = div()
            .v_flex()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .gap_3()
            .p_4()
            .bg(cx.theme().background)
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_lg()
                    .child(self.t("游戏库")),
            )
            .child(
                div()
                    .h_flex()
                    .gap_2()
                    .flex_wrap()
                    .child(self.scan_button(cx))
                    .child(self.add_game_button(cx))
                    .child(
                        self.button("add-folder", "部署目录", Command::CustomFolder, cx)
                            .icon(IconName::FolderOpen)
                            .disabled(self.c.busy),
                    )
                    .child(
                        div()
                            .h_flex()
                            .gap_2()
                            .flex_shrink_0()
                            .child(
                                self.button("refresh", "刷新", Command::Refresh, cx)
                                    .icon(Icon::default().path("app/refresh.svg").size(px(14.)))
                                    .min_w(rems(5.))
                                    .disabled(self.c.busy),
                            )
                            .child(
                                self.button(
                                    "remove-checked",
                                    "移除游戏",
                                    Command::Confirm("remove_checked".into()),
                                    cx,
                                )
                                .icon(Icon::default().path("app/trash.svg"))
                                .disabled(self.c.busy || self.c.selected.is_empty()),
                            ),
                    ),
            )
            .child(
                div()
                    .h_flex()
                    .gap_3()
                    .child(
                        Checkbox::new("all")
                            .focus_ring(crate::ui_focus::visible(cx))
                            .label(self.t("全选"))
                            .checked(
                                !self.c.games.is_empty()
                                    && self.c.selected.len() == self.c.games.len(),
                            )
                            .on_click(cx.listener(|this, v, window, cx| {
                                this.act(Command::All(*v), window, cx)
                            })),
                    )
                    .child(
                        div()
                            .id("library-search-surface")
                            .flex_1()
                            .min_w_0()
                            .on_mouse_down_out({
                                let search = self.search.clone();
                                move |_, window, cx| {
                                    let focus = search.focus_handle(cx);
                                    window.defer(cx, move |window, cx| {
                                        if focus.is_focused(window) {
                                            window.blur(cx);
                                        }
                                    });
                                }
                            })
                            .child(
                                Input::new(&self.search)
                                    .focus_ring(crate::ui_focus::visible(cx))
                                    .cleanable(true)
                                    .w_full(),
                            ),
                    ),
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
        if self.filtered.indices.is_empty() {
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
                list(
                    self.scroll.clone(),
                    cx.processor(|this, i, window, cx| {
                        this.game_row(this.filtered.indices[i], window, cx)
                    }),
                )
                .flex_1()
                .min_h_0(),
            );
        }
        card.into_any_element()
    }
    fn deployment_status(&self, exe: &str, cx: &App) -> AnyElement {
        let status = self
            .c
            .statuses
            .get(exe)
            .map(String::as_str)
            .unwrap_or("正在检测…");
        let (mut label, mut variant) = deployment_label(status);
        label = self.c.text(&label);
        let snapshot = self.c.deployments.get(exe);
        if !status.starts_with("正在")
            && let Some(d) = snapshot
            && d.installed > 0
        {
            let names = d
                .installed_labels
                .iter()
                .map(|label| self.c.text(label))
                .collect::<Vec<_>>();
            label = format!(
                "{} · {} · {}",
                self.c.text(if d.installed == d.total {
                    "已安装"
                } else {
                    "部分安装"
                }),
                names.join(" / "),
                d.proxies.iter().cloned().collect::<Vec<_>>().join(", ")
            );
            if d.total > 1 {
                label.push_str(&format!(" · {}/{}", d.installed, d.total));
            }
            variant = if d.installed == d.total && d.common.is_some() {
                TagVariant::Success
            } else {
                TagVariant::Warning
            };
        }
        if !status.starts_with("正在")
            && let Some(d) = snapshot
            && let Some(issue) = d.issue_label()
        {
            if d.installed > 0 {
                label.push_str(&format!(" · {}", self.c.text(issue)));
            } else {
                label = self.c.text(issue);
            }
            variant = if d
                .issues()
                .iter()
                .all(|i| i.kind() == rtx_fg_manager::deployment::IssueKind::External)
            {
                TagVariant::Info
            } else {
                TagVariant::Warning
            };
        }
        if let Some(snapshot) = snapshot {
            let count = snapshot.neural.iter().filter(|n| n.present).count();
            let missing = snapshot.neural.iter().any(|n| n.managed && !n.present);
            let warning = snapshot.neural.iter().any(|n| n.warning.is_some());
            let nr = component_installation_label(
                "DLSSNR",
                count,
                snapshot.neural.len(),
                missing,
                warning,
                &self.c.tr,
            );
            let xess: Vec<_> = snapshot
                .runtimes
                .iter()
                .filter(|s| s.kind == "xemfg")
                .collect();
            let xess_relevant = xess.iter().any(|s| s.managed || !s.versions.is_empty());
            let xe = component_installation_label(
                "XeSS",
                xess.iter().filter(|s| s.present).count(),
                xess.len(),
                xess.iter()
                    .any(|s| (s.managed || !s.versions.is_empty()) && !s.present),
                xess_relevant && xess.iter().any(|s| s.warning.is_some()),
                &self.c.tr,
            );
            for (component, needs_check) in [nr, xe].into_iter().flatten() {
                label.push_str(&format!(" · {component}"));
                if needs_check {
                    variant = TagVariant::Warning;
                }
            }
        }
        div()
            .flex_1()
            .min_w_0()
            .text_xs()
            .text_color(match variant {
                TagVariant::Success => cx.theme().success,
                TagVariant::Warning => cx.theme().warning,
                TagVariant::Info => cx.theme().info,
                _ => cx.theme().muted_foreground,
            })
            .whitespace_normal()
            .child(self.t(&label))
            .into_any_element()
    }
    fn game_row(&self, index: usize, _window: &Window, cx: &Context<Self>) -> AnyElement {
        let game = &self.c.games[index];
        let directories = game_directories(game);
        let preset_exe = game.exe.clone();
        let parameter_button =
            Button::new(SharedString::from(format!("game-presets:{}", game.exe)))
                .focus_ring(crate::ui_focus::visible(cx))
                .small()
                .custom(
                    ButtonCustomVariant::new(cx)
                        .hover(*cx.theme().tokens.secondary_hover)
                        .active(*cx.theme().tokens.secondary_active),
                )
                .selected(self.editor_open && self.c.focus.as_ref() == Some(&game.exe))
                .size(rems(1.5))
                .icon(Icon::new(IconName::Settings).size(rems(1.)))
                .accessibility_label(self.t("参数设置"))
                .tooltip(self.t("参数设置"))
                .flex_shrink_0()
                .disabled(self.c.busy)
                .on_click(cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    this.act(Command::Editor(preset_exe.clone()), window, cx)
                }));
        let drag = GameDrag {
            exe: game.exe.clone(),
            title: self.c.game_name(game),
        };
        let drag_target = game.exe.clone();
        let drop_target = game.exe.clone();
        let drop_after = self
            .drag_gap
            .as_ref()
            .is_some_and(|(exe, after)| exe == &game.exe && *after);
        let drop_color = cx.theme().primary;
        let exe = game.exe.clone();
        let focus = exe.clone();
        let selected = self.c.selected.contains(&exe);
        let menu_exe = exe.clone();
        let open_folder = game.deployment_directory().display().to_string();
        let open_label = self.t("打开目录");
        let view = cx.entity().downgrade();
        let remove_label = self.t("从游戏库移除");
        let rename_label = self.t("修改游戏名称");
        let menu_disabled = self.c.busy;
        let game_icon = self
            .c
            .icons
            .get(&exe)
            .and_then(|i| i.as_ref())
            .map(|i| img(i.clone()).size_full().into_any_element())
            .unwrap_or_else(|| {
                Self::icon(IconName::LayoutDashboard)
                    .size_10()
                    .text_color(cx.theme().muted_foreground)
                    .into_any_element()
            });
        div()
            .id(SharedString::from(format!("game-row:{}", game.exe)))
            .min_h(rems(4.5))
            .py_1p5()
            .h_flex()
            .w_full()
            .gap_4()
            .px_2()
            .border_b_1()
            .border_color(cx.theme().border)
            .hover(|d| d.bg(cx.theme().muted))
            .when(
                !self.c.busy && !self.c.read_only && !self.dialog_active,
                |row| {
                    row.on_drag(drag, |drag, _, _, cx| cx.new(|_| drag.clone()))
                        .on_drag_move(cx.listener(
                            move |this, event: &DragMoveEvent<GameDrag>, _, cx| {
                                let gap = (
                                    drag_target.clone(),
                                    event.event.position.y > event.bounds.center().y,
                                );
                                if event.bounds.contains(&event.event.position)
                                    && this.drag_gap.as_ref() != Some(&gap)
                                {
                                    this.drag_gap = Some(gap);
                                    cx.notify();
                                }
                            },
                        ))
                        .drag_over::<GameDrag>(move |style, _, _, _| {
                            let style = style.border_color(drop_color);
                            if drop_after {
                                style.border_b_2()
                            } else {
                                style.border_t_2()
                            }
                        })
                        .on_drop(cx.listener(move |this, drag: &GameDrag, _, cx| {
                            let after = this
                                .drag_gap
                                .take()
                                .is_some_and(|(exe, after)| exe == drop_target && after);
                            this.c.reorder_game(&drag.exe, &drop_target, after);
                            this.filter();
                            cx.notify();
                        }))
                },
            )
            .on_click(cx.listener(move |this, _, window, cx| {
                this.act(Command::Focus(focus.clone()), window, cx);
            }))
            .when(self.c.focus.as_ref() == Some(&exe), |d| {
                d.bg(cx.theme().secondary)
            })
            .child(
                Checkbox::new(SharedString::from(format!("game-check:{}", game.exe)))
                    .focus_ring(crate::ui_focus::visible(cx))
                    .checked(selected)
                    .on_click(cx.listener(move |this, yes, window, cx| {
                        cx.stop_propagation();
                        this.act(Command::Game(exe.clone(), *yes), window, cx)
                    })),
            )
            .child(
                div()
                    .size_12()
                    .flex_shrink_0()
                    .h_flex()
                    .justify_center()
                    .child(game_icon),
            )
            .child(
                div()
                    .id(SharedString::from(format!("game-label:{}", game.exe)))
                    .v_flex()
                    .min_w_0()
                    .flex_1()
                    .gap_0p5()
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
                                    .child(self.c.game_name(game)),
                            )
                            .child(parameter_button),
                    )
                    .child(
                        div()
                            .h_flex()
                            .gap_2()
                            .flex_wrap()
                            .min_w_0()
                            .child(self.deployment_status(&game.exe, cx))
                            .when(
                                self.c
                                    .deployments
                                    .get(&game.exe)
                                    .is_some_and(|d| !d.issues().is_empty()),
                                |row| {
                                    let review_exe = game.exe.clone();
                                    row.child(
                                        Button::new(SharedString::from(format!(
                                            "review-game:{}",
                                            game.exe
                                        )))
                                        .xsmall()
                                        .ghost()
                                        .label(self.t("查看原因"))
                                        .focus_ring(crate::ui_focus::visible(cx))
                                        .disabled(self.c.busy)
                                        .on_click(
                                            cx.listener(move |this, _, window, cx| {
                                                cx.stop_propagation();
                                                this.act(
                                                    Command::ReviewGame(review_exe.clone()),
                                                    window,
                                                    cx,
                                                );
                                            }),
                                        ),
                                    )
                                },
                            ),
                    )
                    .child(
                        div()
                            .v_flex()
                            .min_w_0()
                            .gap_0p5()
                            .children(directories.iter().map(|path| {
                                let folder = path.clone();
                                let open_tip = format!("{}\n{path}", self.t("打开目录"));
                                let opened = self
                                    .folder_feedback
                                    .as_ref()
                                    .is_some_and(|(recent, _)| recent == path);
                                div()
                                    .id(SharedString::from(format!(
                                        "game-directory:{}:{path}",
                                        game.exe
                                    )))
                                    .h_flex()
                                    .items_start()
                                    .gap_2()
                                    .min_w_0()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(
                                        Button::new(SharedString::from(format!(
                                            "open-directory:{}:{path}",
                                            game.exe
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
                                        .accessibility_label(open_tip.clone())
                                        .tooltip(open_tip)
                                        .flex_shrink_0()
                                        .on_click(
                                            cx.listener(move |this, _, window, cx| {
                                                cx.stop_propagation();
                                                // Brief activation feedback, not a claim that
                                                // Explorer remains open after this moment.
                                                this.folder_feedback =
                                                    Some((folder.clone(), Instant::now()));
                                                this.act(Command::Open(folder.clone()), window, cx);
                                            }),
                                        ),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .whitespace_normal()
                                            .child(path.clone()),
                                    )
                            })),
                    ),
            )
            .context_menu(move |menu, _, _| {
                let exe = menu_exe.clone();
                let open_view = view.clone();
                let folder = open_folder.clone();
                let view = view.clone();
                let rename_view = view.clone();
                let rename_exe = exe.clone();
                menu.item(PopupMenuItem::new(rename_label.clone()).on_click(
                    move |_, window, cx| {
                        let _ = rename_view.update(cx, |this, cx| {
                            this.act(Command::RenameGame(rename_exe.clone()), window, cx)
                        });
                    },
                ))
                .item(
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
    fn preset_panel(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let context = self.c.preset_context(self.c.focus.as_deref().unwrap_or(""));
        if context.profile == rtx_fg_manager::encore::PROFILE
            && let Some(exe) = self.c.focus.clone()
        {
            let mut editor = self.encore_editor.borrow_mut();
            if editor.as_ref().is_none_or(|(current, _)| current != &exe) {
                self.encore_invalid.set(false);
                let owner = cx.entity();
                let invalid = self.encore_invalid.clone();
                let entity = cx.new(|cx| {
                    encore_editor::EncoreEditor::new(
                        exe.clone(),
                        owner.downgrade(),
                        invalid,
                        window,
                        cx,
                    )
                });
                *editor = Some((exe, entity));
            }
            return editor.as_ref().unwrap().1.clone().into_any_element();
        }
        let options = self.c.focus.as_deref().map(|exe| self.c.preset_values(exe));
        let mut fields = div().v_flex().gap_3().w_full().min_w_0();
        let mut common = div().v_flex().gap_3().w_full().min_w_0();
        if let Some(options) = options {
            let mfg_version = self
                .c
                .focus
                .as_ref()
                .and_then(|e| self.c.deployments.get(e))
                .filter(|s| s.installed > 0)
                .and_then(|s| {
                    s.neural
                        .iter()
                        .filter_map(|n| n.mfg_version.as_deref())
                        .filter_map(|v| semver::Version::parse(v).ok())
                        .min()
                });
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
                    .unwrap_or(value);
                let old_mfg = context.profile == rtx_fg_manager::rtxmfg::PROFILE
                    && mfg_version
                        .as_ref()
                        .is_some_and(|v| *v < semver::Version::new(1, 7, 0))
                    && rtx_fg_manager::rtxmfg::is_170_key(param.key);
                let disabled = self.c.busy
                    || self.c.read_only
                    || old_mfg
                    || !context.parameter_enabled(param.key, &options)
                    || self.c.focus.as_ref().is_some_and(|e| {
                        self.c.running_games.contains(e) || self.c.preset_reads.contains(e)
                    })
                    || (param.key == "hardware_bilinear" && self.c.cloud_series() == 0);
                let mut choices: Vec<_> = param
                    .choices
                    .iter()
                    .filter(|(v, _)| {
                        if context.profile == rtx_fg_manager::upstream040::PROFILE_041
                            && param.key == "max_generated_frames"
                            && options.get("fg_engine").is_some_and(|s| s == "XeSS")
                        {
                            return !matches!(*v, "4" | "5");
                        }
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
                if !param.choices.iter().any(|(v, _)| *v == value) {
                    choices.push((
                        self.t(value),
                        Command::GameOption(param.key.into(), value.clone()),
                        true,
                    ));
                }
                let field = div()
                    .v_flex()
                    .gap_1()
                    .child(div().text_xs().child(self.t(param.label)))
                    .child(self.menu(param.key, self.t(selected), choices, disabled, window, cx))
                    .when(old_mfg, |d| {
                        d.child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(self.t("需要先升级 MFG Unlocker 至 1.7.0。")),
                        )
                    })
                    .when(
                        context.profile == rtx_fg_manager::upstream040::PROFILE_041,
                        |d| {
                            d.when_some(rtx_fg_manager::upstream040::help(param.key), |d, help| {
                                d.child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(self.t(help)),
                                )
                            })
                        },
                    )
                    .when(context.profile == rtx_fg_manager::rtxmfg::PROFILE, |d| {
                        d.when_some(rtx_fg_manager::rtxmfg::help(param.key), |d, help| {
                            d.child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(self.t(help)),
                            )
                        })
                    });
                if matches!(
                    param.key,
                    "max_generated_frames"
                        | "max_interpolated_frames"
                        | "force_multiplier"
                        | "rtx_mode"
                        | "tf_mode"
                ) {
                    common = common.child(field);
                } else {
                    fields = fields.child(field);
                }
            }
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
                    .focus_ring(crate::ui_focus::visible(cx))
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
            .when(self.c.boolean("preset_help_open", false), |d| d.child(Alert::info("preset-inline-help", self.t(if context.profile == rtx_fg_manager::rtxmfg::PROFILE {
                "MFG 1.7：Backspace 打开菜单；插件可使用 OptiScaler 菜单。NR 与独立 XeSS 多帧生成需另装运行库。Vulkan 的 DLSS 动态倍率不可用，独立 XeMFG 动态倍率另行支持。"
            } else if context.profile == rtx_fg_manager::transfusion::PROFILE {
                "默认跟随游戏；固定倍率可选2X—6X，5X/6X可能增加显存与延迟。Ctrl+Alt+2/3/4切换倍率，Ctrl+Alt+G跟随游戏，Ctrl+Alt+D切换动态。RTX20及旧显卡Vulkan仍需实机验证。"
            } else if context.delta {
                "三角洲可选跟随游戏、2X、3X、4X，默认4X。4X不是保证四倍FPS，开启后的延迟需实测；旧组件已加载时会保留原运行时。"
            } else if context.profile == rtx_fg_manager::presets::MFG_VULKAN {
                "作者默认上限6X，请求倍率跟随游戏。上限与请求是两项独立设置；请求倍率请勿超过上限。支持情况取决于游戏和运行库，不会添加游戏菜单，也不保证成倍FPS。"
            } else {
                "仅保存当前游戏、当前方案的参数。完全退出游戏后点击安装补丁以应用；已部署相同 DLL 时只更新这些参数，保留其他 INI 内容。恢复默认只重置这里的选项。"
            })).small()))
            .child(common)
            .child(
                Button::new("preset-accordion")
                    .label(self.t("高级参数"))
                    .icon(if self.c.boolean("preset_expanded", false) { IconName::ChevronDown } else { IconName::ChevronRight })
                    .ghost().small().w_full().justify_start().flex_shrink_0()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.c.state["preset_expanded"] = json!(!this.c.boolean("preset_expanded", false));
                        this.c.save(); cx.notify();
                    })),
            )
            .when(self.c.boolean("preset_expanded", false), |d| d.child(fields.flex_shrink_0()))
            .into_any_element()
    }
    fn proxy_panel(&self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let proxies = self.c.proxies();
        let mut checks = div().grid().grid_cols(2).gap_2().w_full().min_w_0();
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
                    .focus_ring(crate::ui_focus::visible(cx))
                    // The component's built-in label forces line-height: 1 inside
                    // an overflow-hidden wrapper, clipping descenders with our font.
                    // A child keeps the standard checkbox interaction but supplies
                    // its own full line box, including the bottom of "g".
                    .child(
                        div()
                            .line_height(rems(1.5))
                            .min_h(rems(1.5))
                            .min_w_0()
                            .truncate()
                            .child(n),
                    )
                    .tooltip(n)
                    .checked(proxies.iter().any(|s| s == n))
                    .disabled(self.c.busy)
                    .w_full()
                    .min_w_0()
                    .flex_shrink_0()
                    .items_center()
                    .min_h(rems(1.5))
                    .on_click(cx.listener(move |this, yes, window, cx| {
                        this.act(Command::Proxy(n.into(), *yes), window, cx)
                    })),
            );
        }
        let proxy_hint = if self.c.parameter_profile() == rtx_fg_manager::rtxmfg::PROFILE {
            "MFG 菜单 Backspace。DLL 可多选，OptiScaler 插件与 DLL 模式互斥。上游建议只装一个副本，多选可能重复加载。"
        } else if self.c.parameter_profile() == rtx_fg_manager::encore::PROFILE {
            "Encore 菜单 Insert。DLL 可多选，ASI 与 DLL 模式互斥。上游建议单入口，多选可能重复加载或崩溃。Bink 需保留原始 Hooked 文件。"
        } else if self.c.parameter_profile() == rtx_fg_manager::presets::MFG_VULKAN {
            "此上游版本仅提供version.dll，请勿改名或混装其他方案的DLL。"
        } else if self.c.parameter_profile() == rtx_fg_manager::transfusion::PROFILE {
            "Transfusion 每次只安装一个入口；优先 version.dll，冲突时可改选 dinput8、dxgi 或 winmm。保留其他 MOD，配置使用独立 JSON。"
        } else if rtx_fg_manager::upstream040::is_profile(&self.c.parameter_profile()) {
            "0.4.x 各代理使用对应文件，组件共用完整包。允许多选；上游建议只放一个入口，多选兼容性需自行测试。"
        } else if self.c.cloud_scheme().starts_with("upstream") {
            "上游支持多选入口；游戏先加载的 DLL 生效，其余仅转发。优先 version/winmm/dinput8/dbghelp；dxgi/d3d12 按需使用。"
        } else {
            "优先单选 version.dll；不兼容时卸载后换入口。多选可能冲突。"
        };
        div()
            .v_flex()
            .w_full()
            .flex_shrink_0()
            .gap_3()
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .child(self.t("DLL 加载入口（可多选）")),
            )
            .child(checks)
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(self.t(proxy_hint)),
            )
            .into_any_element()
    }
    fn current_preset_pending(&self) -> bool {
        self.c
            .focus
            .as_deref()
            .is_some_and(|exe| self.c.preset_dirty(exe))
            || (self.c.parameter_profile() == rtx_fg_manager::encore::PROFILE
                && self.encore_invalid.get())
    }
    fn any_preset_pending(&self) -> bool {
        self.current_preset_pending() || self.c.first_pending_preset().is_some()
    }

    fn game_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.dialog_active {
            return;
        }
        self.dialog_active = true;
        self.feedback_window = Some(window.window_handle());
        self.editor_open = true;
        self.c.state["preset_help_open"] = json!(false);
        let owner = cx.entity();
        let editor = cx.new(|cx| {
            cx.observe(&owner, |_, _, cx| cx.notify()).detach();
            GameEditor {
                owner: owner.downgrade(),
            }
        });
        let view = cx.entity().downgrade();
        let title = self.t("预设参数");
        let width = (window.viewport_size().width - px(64.)).min(px(720.));
        window.open_dialog(cx, move |dialog, _, cx| {
            let close_view = view.clone();
            let cancel_view = view.clone();
            dialog
                .title(crate::ui_focus::dialog_title(title.clone(), cx))
                .close_button(false)
                .width(width)
                .child(editor.clone())
                .on_cancel(move |_, _, cx| {
                    cancel_view
                        .update(cx, |this, cx| {
                            if this.preset_close.take().is_some() {
                                cx.notify();
                                return false;
                            }
                            if this.current_preset_pending() {
                                this.preset_close = Some(false);
                                cx.notify();
                                return false;
                            }
                            true
                        })
                        .unwrap_or(true)
                })
                .on_close(move |_, _, cx| {
                    let _ = close_view.update(cx, |this, cx| {
                        this.dialog_active = false;
                        this.editor_open = false;
                        cx.notify();
                    });
                })
        });
    }
    fn game_editor_body(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        if self.preset_close.is_some() {
            return div()
                .v_flex()
                .gap_4()
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(self.t("参数尚未应用")),
                )
                .child(self.t(if self.preset_close == Some(true) {
                    "关闭管理器将放弃所有尚未应用的参数修改，不会暂存。"
                } else {
                    "修改尚未写入游戏。选择“不修改”将放弃本次修改并关闭，不会暂存。"
                }))
                .child(
                    div()
                        .h_flex()
                        .flex_wrap()
                        .justify_end()
                        .gap_2()
                        .child(
                            self.button(
                                "preset-close-later",
                                "不修改",
                                Command::ClosePresetWithoutApplying,
                                cx,
                            )
                            .small(),
                        )
                        .child(
                            self.button(
                                "preset-keep-editing",
                                "返回修改",
                                Command::KeepEditing,
                                cx,
                            )
                            .small()
                            .primary(),
                        ),
                )
                .into_any_element();
        }
        let game = self
            .c
            .focus
            .as_ref()
            .and_then(|e| self.c.games.iter().find(|g| &g.exe == e));
        let name = game.map(|g| self.c.game_name(g)).unwrap_or_default();
        let mut content = div()
            .v_flex()
            .flex_shrink_0()
            .gap_3()
            .child(div().font_weight(FontWeight::SEMIBOLD).child(name))
            .child(
                self.c
                    .cloud_label(self.c.catalog.selected(&self.c.cloud_scheme())),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(self.t("修改仅在点击应用后写入游戏；选择“不修改”将放弃修改。")),
            );
        content = content.child(self.preset_panel(window, cx));
        let disabled = self.c.busy
            || self.c.read_only
            || !self.c.can_apply_parameters()
            || self
                .c
                .focus
                .as_ref()
                .is_some_and(|e| self.c.preset_reads.contains(e))
            || (self.c.parameter_profile() == rtx_fg_manager::encore::PROFILE
                && self.encore_invalid.get());
        let locked = self.c.busy
            || self.c.read_only
            || self.c.focus.as_ref().is_some_and(|e| {
                self.c.running_games.contains(e) || self.c.preset_reads.contains(e)
            });
        let resets = if self.c.parameter_profile() == rtx_fg_manager::encore::PROFILE {
            self.encore_editor
                .borrow()
                .as_ref()
                .map(|(_, editor)| {
                    encore_editor::EncoreEditor::reset_buttons(editor, locked, &self.c.tr, cx)
                })
                .unwrap_or_else(|| div().into_any_element())
        } else {
            self.button(
                "preset-reset-fixed",
                "恢复默认参数",
                Command::GameOption("reset".into(), "".into()),
                cx,
            )
            .small()
            .ghost()
            .disabled(locked)
            .into_any_element()
        };
        let pending = self.current_preset_pending();
        div()
            .v_flex()
            .w_full()
            .min_h_0()
            .h((window.viewport_size().height - px(180.)).max(px(180.)))
            .child(
                div()
                    .id("game-editor-scroll")
                    .v_flex()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .pb_3()
                    .child(content),
            )
            .child(
                div()
                    .v_flex()
                    .flex_shrink_0()
                    .gap_2()
                    .pt_3()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().background)
                    .when(!self.c.can_apply_parameters(), |d| d.child(
                        div().text_sm().text_color(cx.theme().muted_foreground)
                            .child(self.t("当前游戏未安装所选方案，或安装状态不完整；请先安装对应方案后再应用参数。"))
                    ))
                    .when(pending, |d| {
                        d.child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(if self.encore_invalid.get() {
                                    cx.theme().danger
                                } else {
                                    cx.theme().warning
                                })
                                .child(self.t(if self.encore_invalid.get() {
                                    "存在无效输入，请修正后应用"
                                } else {
                                    "已修改，待应用"
                                })),
                        )
                    })
                    .child(
                        div()
                            .h_flex()
                            .flex_wrap()
                            .justify_between()
                            .gap_2()
                            .child(resets)
                            .child(
                                self.button(
                                    "apply-preset-fixed",
                                    "应用参数到当前游戏",
                                    Command::ApplyPreset,
                                    cx,
                                )
                                .small()
                                .primary()
                                .disabled(disabled),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn scheme_menu(&self, selected: usize, series: usize, cx: &mut Context<Self>) -> AnyElement {
        let schemes = self.c.catalog.schemes();
        let label = self.c.cloud_label(schemes[selected]);
        let items = schemes
            .iter()
            .enumerate()
            .map(|(i, p)| {
                (
                    i,
                    self.c.cloud_label(p),
                    self.c.catalog.supports_series(&p.scheme_id, series),
                    self.c
                        .catalog
                        .scheme_source_url(&p.scheme_id)
                        .unwrap_or("https://github.com/pandaligx/RTX-FG-Manager")
                        .to_owned(),
                )
            })
            .collect::<Vec<_>>();
        let incompatible = self.t("不兼容当前显卡系列");
        let scale = self.scale();
        let menu_width = self
            .split_state
            .read(cx)
            .sizes()
            .get(1)
            .copied()
            .map(f32::from)
            .unwrap_or(384. * scale)
            .clamp(288. * scale, 432. * scale)
            - 32. * scale;
        let view = cx.entity().downgrade();
        Button::new("scheme-picker")
            .focus_ring(crate::ui_focus::visible(cx))
            .small()
            .tooltip(label.clone())
            .label(label)
            .w_full()
            .icon(IconName::ChevronDown)
            .disabled(self.c.busy)
            .dropdown_menu(move |mut menu, _, _| {
                menu = menu.min_w(px(menu_width)).max_w(px(menu_width));
                for (i, label, supported, url) in &items {
                    let (i, supported) = (*i, *supported);
                    let text = if supported {
                        label.clone()
                    } else {
                        format!("{label} · {incompatible}")
                    };
                    let url = url.clone();
                    let mut parts = text.split(" · ");
                    let name = parts.next().unwrap_or_default().to_owned();
                    let detail = parts.collect::<Vec<_>>().join(" · ");
                    let view_link = view.clone();
                    let view_select = view.clone();
                    menu = menu.item(
                        PopupMenuItem::element(move |_, cx| {
                            let url = url.clone();
                            let view = view_link.clone();
                            div()
                                .h_flex()
                                .w_full()
                                .gap_2()
                                .min_w_0()
                                .child(
                                    div()
                                        .v_flex()
                                        .min_w_0()
                                        .w(px(menu_width - 88. * scale))
                                        .flex_shrink_0()
                                        .py_1()
                                        .child(div().text_sm().truncate().child(name.clone()))
                                        .child(
                                            div()
                                                .text_xs()
                                                .text_color(cx.theme().muted_foreground)
                                                .child(detail.clone()),
                                        ),
                                )
                                .child(
                                    Button::new(("scheme-link", i))
                                        .focus_ring(crate::ui_focus::visible(cx))
                                        .w_6()
                                        .flex_shrink_0()
                                        .small()
                                        .ghost()
                                        .icon(IconName::ExternalLink)
                                        .on_click(move |_, window, cx| {
                                            cx.stop_propagation();
                                            let _ = view.update(cx, |this, cx| {
                                                this.act(Command::Open(url.clone()), window, cx)
                                            });
                                        }),
                                )
                        })
                        .disabled(!supported)
                        .checked(i == selected)
                        .on_click(move |_, window, cx| {
                            let _ = view_select
                                .update(cx, |this, cx| this.act(Command::Scheme(i), window, cx));
                        }),
                    );
                }
                menu
            })
            .into_any_element()
    }
    fn hardware_summary(&self, cx: &Context<Self>) -> AnyElement {
        let device = self.c.devices.get(self.c.device_index);
        let gpu = device
            .and_then(|d| d.original_name.as_deref())
            .unwrap_or(&self.c.gpu)
            .to_owned();
        let names = std::iter::once(("inspector-gpu-name", self.t("实际型号"), gpu)).chain(
            device
                .and_then(|d| d.modified_name.clone())
                .map(|name| ("inspector-gpu-modified", self.t("修改后型号"), name)),
        );
        div()
            .v_flex()
            .flex_shrink_0()
            .p_4()
            .pb_3()
            .gap_3()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .v_flex()
                    .w_full()
                    .min_w_0()
                    .gap_2()
                    .children(names.map(|(id, label, value)| {
                        let full = value.clone();
                        div()
                            .id(id)
                            .v_flex()
                            .w_full()
                            .min_w_0()
                            .gap_0p5()
                            .text_sm()
                            .whitespace_normal()
                            .tooltip(move |window, cx| Tooltip::new(full.clone()).build(window, cx))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(label),
                            )
                            .child(value)
                    })),
            )
            .child(
                self.button("gpu-editor", "修改显卡型号", Command::GpuEditor, cx)
                    .w_full(),
            )
            .when_some(self.recommended_gpu_name(), |d, name| {
                d.child(
                    div()
                        .text_xs()
                        .whitespace_normal()
                        .text_color(cx.theme().warning)
                        .child(self.t("当前游戏推荐显卡名称：{0}").replace("{0}", &name)),
                )
            })
            .child(
                div()
                    .h_flex()
                    .gap_2()
                    .justify_between()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(self.t("硬件加速 GPU 调度")),
                    )
                    .child(
                        div()
                            .id("hags-switch-surface")
                            .track_focus(&self.hags_focus)
                            .on_mouse_down_out({
                                let focus = self.hags_focus.clone();
                                move |_, window, cx| {
                                    let focus = focus.clone();
                                    window.defer(cx, move |window, cx| {
                                        if focus.contains_focused(window, cx) {
                                            window.blur(cx);
                                        }
                                    });
                                }
                            })
                            .child(
                                Switch::new("hardware-gpu-scheduling")
                                    .focus_ring(crate::ui_focus::visible(cx))
                                    .small()
                                    .flex_shrink_0()
                                    .accessibility_label(self.t("硬件加速 GPU 调度"))
                                    .checked(
                                        self.c
                                            .hags
                                            .as_ref()
                                            .and_then(hags::State::selected)
                                            .unwrap_or(false),
                                    )
                                    .disabled(!self.c.can_set_hags())
                                    .on_click(cx.listener(|this, enabled, window, cx| {
                                        this.act(Command::Hags(*enabled), window, cx)
                                    })),
                            ),
                    ),
            )
            .children(self.hags_feedback(cx))
            .into_any_element()
    }
    fn hags_feedback(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let state = self.c.hags.as_ref();
        let message = if self.c.hags_busy && (self.c.hags_writing || state.is_none()) {
            Some(if self.c.hags_writing {
                "正在保存…"
            } else {
                "正在检测…"
            })
        } else if self.c.hags_error.is_some() {
            Some("无法读取或保存 GPU 调度设置，请刷新后重试。")
        } else if state.is_some_and(hags::State::needs_restart) {
            Some("你需要重启电脑才能使此更改生效")
        } else if state.is_some_and(|s| s.state == "unsupported") {
            Some("当前系统或显卡驱动不支持此功能")
        } else if state.and_then(hags::State::selected).is_none() {
            Some("无法确认 GPU 调度状态，请刷新后重试。")
        } else {
            None
        };
        let message = message?;
        let retry = !self.c.hags_busy
            && (self.c.hags_error.is_some() || state.is_some_and(|s| s.state == "unknown"));
        Some(
            div()
                .v_flex()
                .gap_1()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(self.t(message))
                .when(retry, |d| {
                    d.child(
                        self.button("hags-retry", "刷新", Command::RefreshHags, cx)
                            .ghost()
                            .tooltip(
                                self.c
                                    .hags_error
                                    .as_deref()
                                    .map(|s| self.c.text(s))
                                    .unwrap_or_default(),
                            ),
                    )
                })
                .into_any_element(),
        )
    }
    fn neural_panel(&self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(exe) = &self.c.focus else {
            return div().into_any_element();
        };
        let snapshot = self.c.deployments.get(exe);
        let statuses = snapshot.map(|d| d.neural.as_slice()).unwrap_or_default();
        let version = self
            .c
            .neural_versions
            .get(exe)
            .copied()
            .unwrap_or(rtx_fg_manager::neural_runtime::RECOMMENDED);
        let evidence = self.c.evidence.get(exe).and_then(Option::as_ref);
        let reason = if statuses.is_empty() {
            "正在检测…"
        } else if statuses
            .iter()
            .any(|n| n.encore_version.is_none() && n.mfg_version.is_none())
        {
            "请先在所有部署目录安装 Encore 或 MFG 1.7+。"
        } else if statuses.iter().any(|n| !n.supports(version)) {
            "NR 需要 Encore 1.1.0+ 或 MFG 1.7.0+；MFG 使用 310.9.16 运行库"
        } else if evidence.is_none() {
            "正在检测…"
        } else if evidence.is_some_and(|e| !e.neural_input() && e.limited) {
            "DLSS 检测未完成，请刷新后重试。"
        } else if evidence.is_some_and(|e| !e.neural_input() && e.super_resolution_plugin) {
            "仅发现 DLSS 插件，尚未找到超分运行库。"
        } else if evidence.is_some_and(|e| !e.neural_input()) {
            "当前游戏目录未发现 DLSS 超分或光线重建组件。"
        } else {
            ""
        };
        let active = self.c.busy
            && self
                .c
                .payload_progress
                .as_ref()
                .is_some_and(|p| p.proxy == "nvngx_dlssnr.dll");
        let disabled = self.c.busy || self.c.read_only || !reason.is_empty();
        let label = if active { "安装中…" } else { "安装" };
        let present = statuses.iter().filter(|n| n.present).count();
        let mut status = if statuses.is_empty() {
            self.c.text("正在检测…")
        } else if present == 0 {
            self.c.text("未安装")
        } else if present < statuses.len() {
            format!("{} · {present}/{}", self.c.text("部分安装"), statuses.len())
        } else {
            self.c.text("已安装")
        };
        let versions = statuses
            .iter()
            .filter(|n| n.present)
            .map(|n| n.version.as_deref().unwrap_or("?"))
            .collect::<std::collections::BTreeSet<_>>();
        if !versions.is_empty() {
            status.push_str(&format!(
                " · {}",
                versions.into_iter().collect::<Vec<_>>().join(" / ")
            ));
        }
        if statuses.iter().any(|n| n.warning.is_some()) {
            status.push_str(&format!(" · {}", self.c.text("需检查")));
        }
        let tip = format!(
            "{}\n\n{}",
            statuses
                .iter()
                .map(|n| format!("{}\n{}", self.c.text(&n.label()), n.path.display()))
                .collect::<Vec<_>>()
                .join("\n\n"),
            self.c
                .text("仅安装运行库；游戏内开启 DLSS/DLAA 后，Encore 按 Insert，MFG 按 Backspace 设置 NR。")
        );
        let runtime_items = [
            ("310.9.16", self.t("310.9.16 · 推荐 · A/B 模型")),
            ("310.8.0", self.t("310.8.0 · 旧版回退")),
        ];
        let runtime_label = runtime_items
            .iter()
            .find(|(v, _)| *v == version)
            .map(|(_, label)| label.clone())
            .unwrap_or_else(|| version.into());
        let view = cx.entity().downgrade();
        let runtime_picker = Button::new("nr-version")
            .focus_ring(crate::ui_focus::visible(cx))
            .small()
            .tooltip(runtime_label.clone())
            .label(runtime_label)
            .icon(IconName::ChevronDown)
            .disabled(self.c.busy)
            .dropdown_menu(move |mut menu, _, _| {
                for (value, label) in &runtime_items {
                    let value = *value;
                    let view = view.clone();
                    menu = menu.item(
                        PopupMenuItem::new(label.clone())
                            .checked(value == version)
                            .on_click(move |_, window, cx| {
                                let _ = view.update(cx, |this, cx| {
                                    this.act(Command::NeuralVersion(value), window, cx)
                                });
                            }),
                    );
                }
                menu
            });
        let mut panel = div()
            .v_flex()
            .min_w_0()
            .gap_2()
            .child(
                div()
                    .id("nr-summary")
                    .h_flex()
                    .flex_wrap()
                    .gap_2()
                    .justify_between()
                    .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .child(self.t("DLSS神经渲染")),
                    )
                    .child(runtime_version_indicator(status, version, cx)),
            )
            .children(statuses.iter().map(|n| self.runtime_path(&n.path, cx)))
            .child(
                div()
                    .h_flex()
                    .flex_wrap()
                    .gap_2()
                    .justify_end()
                    .child(runtime_picker)
                    .child(
                        self.button("nr-install", label, Command::InstallNeural(version), cx)
                            .small()
                            .flex_shrink_0()
                            .disabled(disabled),
                    )
                    .child(
                        self.button("nr-remove", "卸载", Command::RemoveNeural, cx)
                            .small()
                            .flex_shrink_0()
                            .disabled(
                                self.c.busy
                                    || self.c.read_only
                                    || !statuses.iter().any(|n| n.managed),
                            ),
                    ),
            );
        let hint = if !reason.is_empty() {
            reason
        } else if self.c.running_games.contains(exe) {
            "游戏仍在后台运行；操作前将提示处理。"
        } else if snapshot.is_some_and(|s| s.total > 1) {
            "NR 安装和卸载会处理此游戏的所有部署目录。"
        } else {
            ""
        };
        if !hint.is_empty() {
            let tip = self.t(hint);
            panel = panel.child(
                div()
                    .id("nr-reason")
                    .text_xs()
                    .min_w_0()
                    .whitespace_normal()
                    .text_color(cx.theme().muted_foreground)
                    .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                    .child(self.t(hint)),
            );
        }
        panel.into_any_element()
    }

    fn details(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let (scheme, series) = self.profile();
        let game = self
            .c
            .focus
            .as_ref()
            .and_then(|exe| self.c.games.iter().find(|g| &g.exe == exe));
        let name = game
            .map(|g| self.c.game_name(g))
            .unwrap_or_else(|| self.c.text("请选择游戏"));
        let game_icon = game
            .and_then(|g| self.c.icons.get(&g.exe))
            .and_then(|i| i.as_ref())
            .map(|i| img(i.clone()).size_8().flex_shrink_0().into_any_element())
            .unwrap_or_else(|| {
                Self::icon(IconName::LayoutDashboard)
                    .size_8()
                    .text_color(cx.theme().muted_foreground)
                    .into_any_element()
            });
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
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(self.t("当前游戏")),
                    )
                    .child(
                        div()
                            .h_flex()
                            .gap_3()
                            .min_w_0()
                            .child(game_icon)
                            .child(
                                div()
                                    .id("inspector-game-name")
                                    .flex_1()
                                    .min_w_0()
                                    .text_sm()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(name),
                            )
                            .when_some(game, |row, g| {
                                row.child(
                                    self.button(
                                        "rename-current-game",
                                        "修改游戏名称",
                                        Command::RenameGame(g.exe.clone()),
                                        cx,
                                    )
                                    .label(self.t("改名"))
                                    .ghost()
                                    .small()
                                    .disabled(self.c.read_only),
                                )
                            }),
                    ),
            )
            .when_some(game, |panel, game| {
                panel.child(self.recommendation_entry(game.exe.clone(), cx))
            })
            .child(
                div()
                    .v_flex()
                    .gap_2()
                    .child(div().text_sm().child(self.t("此游戏的适配方案")))
                    .child(self.scheme_menu(scheme, series, cx)),
            );
        if let Some(d) = self
            .c
            .focus
            .as_ref()
            .and_then(|e| self.c.deployments.get(e))
            && !d.issues().is_empty()
        {
            panel = panel.child(
                div()
                    .v_flex()
                    .gap_2()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .child(self.t(d.issue_label().unwrap_or("查看原因"))),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(self.t(d.issues()[0].kind().help())),
                    )
                    .child(
                        self.button(
                            "cleanup-details",
                            if d.cleanup_pending {
                                "清理残留"
                            } else {
                                "查看原因"
                            },
                            Command::CleanupDetails,
                            cx,
                        )
                        .small()
                        .disabled(self.c.busy),
                    ),
            );
        }
        panel = panel
            .child(
                self.button(
                    "runtime-files",
                    "DLSS / XeSS 运行库",
                    Command::RuntimeFiles,
                    cx,
                )
                .small()
                .w_full()
                .disabled(self.c.busy || self.c.focus.is_none()),
            )
            .child(self.proxy_panel(window, cx));
        if self.c.cloud_scheme().starts_with("upstream") {
            panel = panel
                .child(Alert::info("upstream-scope", self.t("此上游方案使用 D3D12；Vulkan 可选择三角洲扩展或独立 Vulkan 方案，兼容性需按游戏实测。")).small());
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
                self.c.target_game_count()
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
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(self.t(dlss)),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(self.t(anti)),
                );
            if let Some(g) = game {
                if g.targets.len() > 1 {
                    panel = panel.child(div().text_xs().child(self.t("将处理以下游戏本体目录：")));
                    for path in &g.targets {
                        panel = panel.child(div().text_xs().child(path.clone()));
                    }
                }
                for reason in &g.reasons {
                    panel = panel.child(div().text_xs().child(self.t(reason)));
                }
            }
        }
        div()
            .v_flex()
            .size_full()
            .min_h_0()
            .border_l_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .overflow_hidden()
            .child(self.hardware_summary(cx))
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
                        div()
                            .w_full()
                            .min_w_0()
                            .text_xs()
                            .whitespace_normal()
                            .text_color(cx.theme().muted_foreground)
                            .child(self.t("温馨提示：网络游戏使用任何第三方注入工具，均存在被检测或封禁的可能，请酌情考虑！")),
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
        let layout = self.layout(window);
        if layout.compact {
            return self.library(cx);
        }
        let scale = self.scale();
        let width = self.c.state["inspector_width"]
            .as_f64()
            .filter(|v| v.is_finite())
            .unwrap_or(384. * scale as f64) as f32;
        let available = f32::from(window.viewport_size().width) - layout.nav;
        let width = width.clamp(320. * scale, (available - 320. * scale).max(320. * scale));
        h_resizable("library-inspector-split")
            .with_state(&self.split_state)
            .child(
                resizable_panel()
                    .size(px(available - width))
                    .size_range(px(320. * scale)..Pixels::MAX)
                    .child(self.library(cx)),
            )
            .child(
                resizable_panel()
                    .size(px(width))
                    .size_range(px(320. * scale)..Pixels::MAX)
                    .child(self.details(window, cx)),
            )
            .into_any_element()
    }
    fn update_panel(&self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let mut panel = div().v_flex().w_full().min_w_0().gap_3().child(
            div()
                .h_flex()
                .gap_2()
                .flex_wrap()
                .child(
                    self.button("check-update", "检查更新", Command::CheckUpdate, cx)
                        .with_size(gpui_component::Size::Medium)
                        .disabled(self.c.update_busy),
                )
                .child(
                    self.button("release-notes", "更新日志", Command::ReleaseNotes, cx)
                        .with_size(gpui_component::Size::Medium),
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
    fn recommended_gpu_name(&self) -> Option<String> {
        let game = self
            .c
            .games
            .iter()
            .find(|g| Some(g.exe.as_str()) == self.c.focus.as_deref())?;
        let requirement = game.extra.get("recommendation_gpu")?;
        let name = requirement.get("name")?.as_str()?;
        if requirement.get("scheme")?.as_str()? != self.c.cloud_scheme()
            || !gpu_alias::NAMES.contains(&name)
        {
            return None;
        }
        Some(name.into())
    }
    fn gpu_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.dialog_active {
            return;
        }
        if let Some(name) = self.recommended_gpu_name()
            && let Some(index) = gpu_alias::NAMES.iter().position(|v| *v == name)
        {
            self.c.alias_index = index;
        }
        self.dialog_active = true;
        let owner = cx.entity();
        let editor = cx.new(|cx| {
            cx.observe(&owner, |_, _, cx| cx.notify()).detach();
            crate::ui_focus::observe(cx);
            GpuEditor {
                owner: owner.downgrade(),
            }
        });
        let view = owner.downgrade();
        let title = self.t("修改显卡型号");
        let width =
            (window.viewport_size().width - window.rem_size() * 4.).min(window.rem_size() * 38.);
        window.open_dialog(cx, move |dialog, _, cx| {
            let view = view.clone();
            dialog
                .title(crate::ui_focus::dialog_title(title.clone(), cx))
                .close_button(false)
                .width(width)
                .child(editor.clone())
                .on_close(move |_, _, cx| {
                    let _ = view.update(cx, |this, cx| {
                        this.dialog_active = false;
                        cx.notify();
                    });
                })
        });
    }
    fn gpu_editor_body(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let disabled = self.c.busy || self.c.devices.is_empty();
        let fields = div()
            .id("gpu-scroll")
            .v_flex()
            .gap_4()
            .min_w_0()
            .min_h_0()
            .max_h(
                (window.viewport_size().height - window.rem_size() * 15.)
                    .max(window.rem_size() * 8.),
            )
            .overflow_y_scroll()
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(self.t(
                    "仅修改 Windows 显示名称，不改变显卡性能或硬件能力。修改和还原后请重启电脑。",
                )),
            )
            .child(
                div().v_flex().gap_2().child(self.t("当前显卡")).child(
                    div()
                        .h_flex()
                        .gap_2()
                        .child(
                            div().flex_1().min_w_0().child(
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
                                    disabled,
                                    window,
                                    cx,
                                ),
                            ),
                        )
                        .child(
                            self.button("gpu-refresh", "刷新", Command::RefreshGpu, cx)
                                .flex_shrink_0()
                                .disabled(self.c.busy),
                        ),
                ),
            )
            .when_some(self.c.devices.get(self.c.device_index), |d, device| {
                d.child(
                    div()
                        .text_xs()
                        .whitespace_normal()
                        .text_color(cx.theme().muted_foreground)
                        .child(format!("{} · {}", device.version, device.instance)),
                )
            })
            .when(self.c.devices.is_empty(), |d| {
                d.child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(self.t("未检测到可修改的 NVIDIA 显卡，请刷新。")),
                )
            })
            .child(
                div().v_flex().gap_2().child(self.t("修改为")).child(
                    self.menu(
                        "gpu-alias",
                        gpu_alias::NAMES[self.c.alias_index].into(),
                        gpu_alias::NAMES
                            .iter()
                            .enumerate()
                            .map(|(i, s)| ((*s).into(), Command::Alias(i), i == self.c.alias_index))
                            .collect(),
                        disabled,
                        window,
                        cx,
                    ),
                ),
            );
        div()
            .v_flex()
            .gap_4()
            .min_w_0()
            .text_sm()
            .child(fields)
            .child(
                div()
                    .h_flex()
                    .flex_wrap()
                    .gap_2()
                    .justify_end()
                    .child(
                        self.button(
                            "alias",
                            "修改显卡型号",
                            Command::Confirm("alias".into()),
                            cx,
                        )
                        .primary()
                        .disabled(disabled),
                    )
                    .child(
                        self.button(
                            "restore",
                            "还原原始名称",
                            Command::Confirm("restore".into()),
                            cx,
                        )
                        .disabled(disabled),
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
        if profile == rtx_fg_manager::transfusion::PROFILE {
            sections = vec![
                (self.t("当前方案"), self.c.cloud_label(self.c.catalog.selected(&self.c.cloud_scheme())).into()),
                (self.t("请求倍率"), self.t("Transfusion 使用独立 JSON，默认跟随游戏。固定支持2X—6X，5X/6X为实验；动态默认最高4X，目标0跟随显示器刷新率。")),
                (self.t("兼容性"), self.t("游戏须已有 Streamline DLSS 帧生成；RTX20及RTX20/30 Vulkan仍待实体测试。Smooth Motion不是普通帧生成功能，本版不提供它的开关。")),
                (self.t("使用说明"), self.t("仅选 version.dll、dinput8.dll、dxgi.dll、winmm.dll 中一个入口。Ctrl+Alt+2—6切换倍率，Ctrl+Alt+G跟随游戏，Ctrl+Alt+D切换动态，Ctrl+Alt+O显示统计；Vulkan统计需要可选ReShade，本版不捆绑。")),
            ];
        } else if profile == rtx_fg_manager::rtxmfg::PROFILE {
            sections = vec![
                (self.t("当前方案"), self.c.cloud_label(self.c.catalog.selected(&self.c.cloud_scheme())).into()),
                (self.t("使用说明"), self.t("MFG 1.7 支持 NR 与独立 XeSS 多帧生成。运行库可在右侧单独安装；管理器参数需退出游戏后应用。独立 XeMFG 使用时请关闭游戏原生 FSR/XeSS 帧生成。")),
                (self.t("兼容性"), self.t("MFG Unlocker 使用独立 JSON。独立代理按 Backspace；OptiScaler 10 插件使用 OptiScaler 菜单。两种入口只能选择一种，管理器不会安装 OptiScaler。实际可用功能由显卡、驱动和游戏决定。")),
            ];
        } else if profile == rtx_fg_manager::presets::MFG_VULKAN {
            sections.remove(2);
            sections[1] = (self.t("倍率上限与请求倍率"), self.t("作者默认上限6X，请求倍率跟随游戏。上限与请求是两项独立设置；请求倍率请勿超过上限。支持情况取决于游戏和运行库，不会添加游戏菜单，也不保证成倍FPS。"));
            sections.insert(2, (self.t("动态多帧与优化"), self.t("动态多帧默认关闭，仅兼容的DX12路径支持；目标帧率0跟随游戏。四项优化按作者默认开启，可能影响画质或兼容性，出问题可逐项关闭。日志是开关，不是0—3级别。")));
            sections.insert(3, (self.t("保留作者配置"), self.t("RenderScale保持游戏控制；BlackwellScatter/BlackwellBlend默认关闭，作者记录过GPU挂起，不在此提供开关。其余高级设置和快捷键原样保留。Ctrl+F2—F6请求2X—6X，Ctrl+F11跟随游戏，Ctrl+F12恢复启动INI。")));
            sections.insert(1, (self.t("Vulkan 支持范围"), self.t("sm86-7发布说明新增Vulkan（含RTX Remix示例），不等于所有Vulkan游戏可用。作者仅有RTX3070，RTX20支持尚待实测。此方案独立于Smooth Motion和三角洲专项，只提供version.dll。")));
        } else if profile == rtx_fg_manager::upstream040::PROFILE_041 {
            sections = vec![
                (self.t("当前方案"), self.c.cloud_label(self.c.catalog.selected(&self.c.cloud_scheme())).into()),
                (self.t("使用说明"), self.t("0.4.1 完整包包含 DLSS / XeSS 后端和超分组件。默认 Optimized=1、Preset=A，超分关闭；升级保留已有设置。")),
                (self.t("兼容性"), self.t("帧生成需要 R580 或更新驱动；XeSS 最高4X，不需另装 Intel DLL。旧代仅2X的 DLSS-G 接口不受支持，游戏仍须自行兼容。")),
                (self.t("超分辨率"), self.t("M/L 模式会覆盖全部画质档。旧版 On 无等价迁移，请先明确选择；严格无 HUD 模式可能使缺少输入的游戏退出，建议自动。")),
            ];
        } else if profile == "upstream040" {
            sections = vec![
                (self.t("当前方案"), self.c.cloud_label(self.c.catalog.selected(&self.c.cloud_scheme())).into()),
                (self.t("使用说明"), self.t("首次完整下载 0.4.0 组件，六种代理只安装一个入口。默认 Optimized=2，SR 关闭；升级时保留已有设置。")),
                (self.t("怎样生效"), self.t("游戏完全退出后安装或应用参数。帧生成、超分与 Runtime 分别设置；选择方案不会增加游戏原本没有的图形接口支持。")),
            ];
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
        window.open_dialog(cx, move |dialog, _, cx| {
            let view = view.clone();
            dialog
                .title(crate::ui_focus::dialog_title(title.clone(), cx))
                .close_button(false)
                .width(width)
                .button_props(DialogButtonProps::default().ok_text(ok.clone()))
                .footer(
                    DialogFooter::new().child(
                        Button::new("config-help-ok")
                            .focus_ring(crate::ui_focus::visible(cx))
                            .small()
                            .label(ok.clone())
                            .primary()
                            .on_click(|_, window, cx| {
                                window.dispatch_action(Box::new(Confirm { secondary: false }), cx)
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
    fn release_notes(&mut self, _: &mut Window, _: &mut Context<Self>) {
        let url = if self.c.choice("download_source", "domestic") == "github" {
            "https://github.com/pandaligx/RTX-FG-Manager/releases"
        } else {
            "https://gitcode.com/gcw_qWIjmve7/RTX-FG-Manager/releases"
        };
        self.c.channel.job(move |_| win::open(url));
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
        let notes = rtx_fg_manager::release_notes::select(&m.notes, &self.language)
            .map(str::to_owned)
            .unwrap_or_else(|| self.c.text("此版本未附更新日志，可在项目发布页查看。"));
        let accept = self.t("下载并更新");
        let later = self.t("稍后");
        let width = (window.viewport_size().width - px(80.)).min(px(520.));
        let top = ((window.viewport_size().height - px(320.)) / 2.).max(px(40.));
        let view = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _, cx| {
            let ok_view = view.clone();
            let close_view = view.clone();
            dialog
                .title(crate::ui_focus::dialog_title(title.clone(), cx))
                .close_button(false)
                .width(width)
                .margin_top(top)
                .footer(
                    DialogFooter::new()
                        .child(
                            Button::new("update-later")
                                .focus_ring(crate::ui_focus::visible(cx))
                                .small()
                                .label(later.clone())
                                .on_click(|_, window, cx| {
                                    window.dispatch_action(Box::new(Cancel), cx);
                                }),
                        )
                        .child(
                            Button::new("update-accept")
                                .focus_ring(crate::ui_focus::visible(cx))
                                .small()
                                .label(accept.clone())
                                .primary()
                                .on_click(|_, window, cx| {
                                    window.dispatch_action(
                                        Box::new(Confirm { secondary: false }),
                                        cx,
                                    );
                                }),
                        ),
                )
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
                        .child(explanation.clone())
                        .child(
                            div()
                                .id("update-notes-scroll")
                                .max_h(px(210.))
                                .overflow_y_scroll()
                                .text_sm()
                                .child(notes.clone()),
                        ),
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
    fn accepts_feedback(&self, window: &Window) -> bool {
        self.feedback_window.unwrap_or(self.main_window).window_id()
            == window.window_handle().window_id()
    }
    /// GPUI owns whether a dialog is still open. Do not let a skipped close
    /// callback or a removed utility leave a permanent application-wide lock.
    fn reconcile_dialogs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.utility_opening {
            return;
        }
        let current = window.window_handle().window_id();
        let active = cx.windows().into_iter().find(|handle| {
            if handle.window_id() == current {
                window.has_active_dialog(cx)
            } else {
                handle
                    .update(cx, |_, window, cx| window.has_active_dialog(cx))
                    .unwrap_or(false)
            }
        });
        if let Some(handle) = active {
            self.dialog_active = true;
            // An actual modal, not an unrelated utility, receives feedback and
            // focus when the user clicks a different native window.
            self.feedback_window = Some(handle);
        } else if self.dialog_active || self.theme_preview.is_active() {
            self.dialog_active = false;
            self.editor_open = false;
            self.preset_close = None;
            // Restore the saved theme; do not save a hover-only preview or
            // discard the Controller's pending parameter changes.
            if self.theme_preview.clear() {
                self.theme(cx);
            }
            cx.notify();
        }
    }
    fn dialogs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.dialog_active || !self.accepts_feedback(window) {
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
                "end_processes" => {
                    "检测到以下游戏相关进程仍在运行。结束进程会丢失未保存的游戏进度；确认后仅结束列出的进程，再重新检测并重试本次操作。身份无法核实的进程不会结束。"
                }
                "clean_batch" => "确认卸载以下游戏的补丁？请先完全退出这些游戏。",
                "recover_patch" => {
                    "仅卸载以下异常目录中可确认归属的补丁，并恢复已记录的原文件。其他目录、存档和未知文件保留；完成后可重新安装。"
                }
                "remove_nr" => {
                    "仅撤销管理器安装的 NR 运行库；如安装前已有文件则恢复原件，保留 Encore 和其他补丁。请先退出游戏。"
                }
                "delete_cleanup" => {
                    "确认永久删除以下当前游戏缓存和安装临时文件？目录内列出的自定义文件也会删除，无法撤销。游戏存档、备份和其他游戏的目录不在此列表内。删除后将自动重新清理。"
                }
                "remove" | "remove_checked" => "仅从游戏库移除；游戏文件和已部署补丁保持不变。",
                "update" => "更新将关闭管理器并替换程序，成功启动后删除旧版。是否继续？",
                _ => "仅修改 Windows 显示名称，不改变显卡性能或硬件能力。修改和还原后请重启电脑。",
            };
            let mut message = self.c.text(text);
            if matches!(a.as_str(), "alias" | "restore")
                && let Some(device) = self.c.devices.get(self.c.device_index)
            {
                let target = if a == "alias" {
                    gpu_alias::NAMES[self.c.alias_index].to_owned()
                } else {
                    self.c.text("还原原始名称")
                };
                message.push_str(&format!("\n\n{} → {}", device.name, target));
            }
            if a == "delete_cleanup" {
                message.push_str("\n\n");
                message.push_str(&self.c.cleanup_deletion_list());
            }
            if a == "end_processes"
                && let Some(blocked) = &self.c.blocked
            {
                for p in &blocked.processes {
                    message.push_str(&format!(
                        "\n\n{} · PID {}\n{}",
                        p.name,
                        p.pid,
                        p.path
                            .as_ref()
                            .map(|v| v.display().to_string())
                            .unwrap_or_else(|| self.c.text("路径无法核实，请在任务管理器中处理"))
                    ));
                }
            }
            if a == "deploy_batch" || a == "clean_batch" || a == "recover_patch" {
                message.push_str("\n\n");
                message.push_str(&self.c.pending_patch_list());
            }

            (self.t("请确认"), message.into(), Some(a), IconName::Info)
        } else {
            return;
        };
        let removal_dialog = matches!(action.as_deref(), Some("remove" | "remove_checked"));
        // Freeze the displayed identities so a background refresh cannot change
        // which games the confirmation removes.
        let removal_rows = self
            .c
            .games
            .iter()
            .filter(|game| {
                if action.as_deref() == Some("remove_checked") {
                    self.c.selected.contains(&game.exe)
                } else {
                    removal_dialog && self.c.focus.as_ref() == Some(&game.exe)
                }
            })
            .map(|game| {
                (
                    game.exe.clone(),
                    self.c.game_name(game),
                    if game.deployment_dir.is_empty() {
                        game.exe.clone()
                    } else {
                        format!("{}\n{}", game.exe, game.deployment_dir)
                    },
                    self.c.icons.get(&game.exe).and_then(|i| i.clone()),
                )
            })
            .collect::<Vec<_>>();
        let removal_paths = removal_rows
            .iter()
            .map(|row| row.0.clone())
            .collect::<std::collections::BTreeSet<_>>();
        let title = if removal_dialog {
            self.t("移除游戏")
        } else {
            title
        };
        self.dialog_active = true;
        let view = cx.entity().downgrade();
        // The pinned AlertDialog overwrites its base on_close callback during
        // conversion. Release our guard in both standard button handlers instead.
        let process_dialog = action.as_deref() == Some("end_processes");
        let cannot_end =
            process_dialog && !self.c.blocked.as_ref().is_some_and(|b| b.can_terminate());
        let recheck_text = self.t("重新检测并重试");
        let ok_text = self.t(if removal_dialog {
            "移除游戏"
        } else if process_dialog {
            "结束并重试"
        } else if action.as_deref() == Some("delete_cleanup") {
            "删除并重新清理"
        } else if action.as_deref() == Some("recover_patch") {
            "卸载并恢复"
        } else if action.is_some() {
            "继续"
        } else {
            "确定"
        });
        let cancel_text = self.t("取消");
        let ok_variant = if matches!(
            action.as_deref(),
            Some(
                "clean_batch" | "recover_patch" | "remove_nr" | "end_processes" | "delete_cleanup"
            )
        ) {
            ButtonVariant::Danger
        } else {
            ButtonVariant::Primary
        };
        let props = DialogButtonProps::default()
            .ok_text(ok_text.clone())
            .cancel_text(cancel_text.clone())
            .show_cancel(action.is_some())
            .ok_variant(ok_variant);
        let width = (window.viewport_size().width - px(90.)).min(px(600.));
        let height = (window.viewport_size().height - px(240.)).max(px(120.));
        window.open_alert_dialog(cx, move |dialog, _, cx| {
            let ok_view = view.clone();
            let cancel_view = view.clone();
            let action = action.clone();
            let recheck_view = view.clone();
            let removal_paths = removal_paths.clone();
            let mut body = div()
                .v_flex()
                .gap_3()
                .when(removal_dialog, |d| d.text_sm())
                .child(text.clone());
            for (exe, name, path, icon) in &removal_rows {
                let icon = icon
                    .as_ref()
                    .map(|i| img(i.clone()).size_8().into_any_element())
                    .unwrap_or_else(|| {
                        Self::icon(IconName::LayoutDashboard)
                            .size_8()
                            .text_color(cx.theme().muted_foreground)
                            .into_any_element()
                    });
                let full_path = path.clone();
                body = body.child(
                    div()
                        .h_flex()
                        .items_center()
                        .gap_3()
                        .py_2()
                        .border_b_1()
                        .border_color(cx.theme().border)
                        .child(div().flex_shrink_0().child(icon))
                        .child(
                            div()
                                .v_flex()
                                .gap_1()
                                .flex_1()
                                .min_w_0()
                                .child(
                                    div()
                                        .text_sm()
                                        .font_weight(FontWeight::MEDIUM)
                                        .child(name.clone()),
                                )
                                .child(
                                    div()
                                        .id(SharedString::from(format!("remove-path:{exe}")))
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .truncate()
                                        .tooltip(move |window, cx| {
                                            Tooltip::new(full_path.clone()).build(window, cx)
                                        })
                                        .child(path.clone()),
                                ),
                        ),
                );
            }

            dialog
                .title(crate::ui_focus::dialog_title(title.clone(), cx))
                .close_button(false)
                .icon(Self::icon(status_icon.clone()).size(px(28.)))
                .width(width)
                .button_props(props.clone())
                .footer(
                    DialogFooter::new()
                        .when(process_dialog, |footer| {
                            footer.child(
                                Button::new("process-recheck")
                                    .focus_ring(crate::ui_focus::visible(cx))
                                    .small()
                                    .label(recheck_text.clone())
                                    .on_click(move |_, window, cx| {
                                        let _ = recheck_view.update(cx, |this, cx| {
                                            this.act(Command::RecheckBlocked, window, cx)
                                        });
                                    }),
                            )
                        })
                        .when(action.is_some(), |footer| {
                            footer.child(
                                Button::new("message-cancel")
                                    .focus_ring(crate::ui_focus::visible(cx))
                                    .small()
                                    .label(cancel_text.clone())
                                    .on_click(|_, window, cx| {
                                        window.dispatch_action(Box::new(Cancel), cx);
                                    }),
                            )
                        })
                        .child(
                            Button::new("message-ok")
                                .focus_ring(crate::ui_focus::visible(cx))
                                .disabled(cannot_end)
                                .small()
                                .label(ok_text.clone())
                                .with_variant(ok_variant)
                                .on_click(|_, window, cx| {
                                    window.dispatch_action(
                                        Box::new(Confirm { secondary: false }),
                                        cx,
                                    );
                                }),
                        ),
                )
                .child(
                    div()
                        .id("message-scroll")
                        .max_h(height)
                        .overflow_y_scroll()
                        .child(body),
                )
                .on_ok(move |_, window, cx| {
                    let _ = ok_view.update(cx, |this, cx| {
                        this.dialog_active = false;
                        if removal_dialog {
                            this.c.forget(&removal_paths);
                            this.filter();
                        } else if let Some(action) = &action {
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
                        this.c.blocked = None;
                        cx.notify();
                    });
                    true
                })
        });
    }
    fn tick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.reconcile_runtime_window(cx);
        self.reconcile_settings_window(cx);
        self.reconcile_dialogs(window, cx);
        if self
            .folder_feedback
            .as_ref()
            .is_some_and(|(_, at)| at.elapsed() >= Duration::from_secs(2))
        {
            self.folder_feedback = None;
            cx.notify();
        }
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
        if self.c.events() {
            self.filter();
            cx.notify();
        }
        if self.c.install_pending && !self.c.busy && !self.c.update_busy && !self.c.closing {
            self.c.install_update();
            cx.notify();
        }
        if self.system_poll.elapsed() > Duration::from_secs(2) {
            self.system_poll = Instant::now();
            cx.set_reduce_motion(preferred_reduce_motion(
                &self.c.state,
                win::reduced_motion(),
            ));
            self.c.poll_hags();
            let choice = self.c.choice("theme", "跟随系统");
            if choice != self.theme_choice || choice == "跟随系统" {
                self.theme_choice = choice;
                self.theme(cx)
            }
            if self.c.smoke.is_none()
                && !self.c.closing
                && self.c.boolean("auto_update", true)
                && self.c.last_check.elapsed() > Duration::from_secs(21600)
            {
                self.c.check_update(true);
                cx.notify();
            }
        }
        if !self.c.closing && self.accepts_feedback(window) {
            if self.dialog_active && self.c.confirm.as_deref() == Some("end_processes") {
                window.close_dialog(cx);
                self.dialog_active = false;
                self.editor_open = false;
                self.preset_close = None;
            }
            self.dialogs(window, cx);
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
                    1 => {
                        if self.c.focus.is_some() && self.c.state["smoke_editor"].is_string() {
                            self.game_editor(window, cx);
                        }
                    }
                    2 => {
                        window.close_dialog(cx);
                        self.dialog_active = false;
                        self.gpu_editor(window, cx);
                    }
                    3 => {
                        window.close_dialog(cx);
                        self.dialog_active = false;
                        self.c.page = 2;
                    }
                    4 => self.c.page = if self.layout(window).compact { 3 } else { 0 },
                    5 => {
                        let report = json!({"single_config_editor":true,"community_enabled":false,"help_enabled":false,"log_panel_enabled":false,"hardware_in_inspector":true,"first_ui_ms":self.first_render_ms,"frames":self.frames,"heartbeats":self.ticks,"max_event_gap_ms":self.max_gap_ms,"language":self.c.tr.language,"dark":cx.theme().mode.is_dark(),"light_palette":cx.theme().light_theme.name,"dark_palette":cx.theme().dark_theme.name,"renderer":"GPUI Fast D3D11","python_runtime":false,"scan_exercised":self.c.boolean("smoke_scan",false),"scan_busy_at_close":self.c.busy,"width":f32::from(window.viewport_size().width),"height":f32::from(window.viewport_size().height)});
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
            3 if self.layout(window).compact => self.details(window, cx),
            _ => self.home(window, cx),
        };
        let automatic_sidebar = f32::from(window.viewport_size().width) < 760. * self.scale();
        let toggle_label = self.t(if automatic_sidebar {
            "窗口较窄，侧栏已自动收起"
        } else if self.c.boolean("sidebar_collapsed", true) {
            "展开侧边栏"
        } else {
            "收起侧边栏"
        });
        // Let the native title bar own the entire band (background, border and
        // height). A rem-sized sibling drifts above/below its fixed DPI-aware
        // window controls when the user changes the content font size.
        let topbar =
            TitleBar::new().pl(px(8.)).child(
                div()
                    .h_flex()
                    .gap(px(4.))
                    .h_full()
                    .flex_shrink_0()
                    // Exclude command buttons from the parent's native drag
                    // hitbox; keep empty title-bar space draggable.
                    .occlude()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(
                        Button::new("titlebar-sidebar-toggle")
                            .focus_ring(crate::ui_focus::visible(cx))
                            .disabled(automatic_sidebar)
                            .ghost()
                            .small()
                            .size(px(28.))
                            .icon(Icon::new(IconName::PanelLeft).size(px(16.)))
                            .accessibility_label(toggle_label.clone())
                            .tooltip(toggle_label)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.act(Command::ToggleSidebar, window, cx)
                            })),
                    )
                    .child(
                        Button::new("titlebar-theme")
                            .focus_ring(crate::ui_focus::visible(cx))
                            .ghost()
                            .small()
                            .size(px(28.))
                            .icon(Icon::new(IconName::Palette).size(px(16.)))
                            .accessibility_label(self.t("主题"))
                            .tooltip(self.t("主题"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.act(Command::Themes, window, cx)
                            })),
                    )
                    .when(self.layout(window).compact, |d| {
                        d.child(
                            Button::new("titlebar-inspector")
                                .focus_ring(crate::ui_focus::visible(cx))
                                .ghost()
                                .small()
                                .size(px(28.))
                                .icon(Icon::new(IconName::PanelRight).size(px(16.)))
                                .accessibility_label(self.t("补丁设置"))
                                .tooltip(self.t("补丁设置"))
                                .selected(self.c.page == 3)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.act(
                                        Command::Page(if this.c.page == 3 { 0 } else { 3 }),
                                        window,
                                        cx,
                                    )
                                })),
                        )
                    }),
            );
        div()
            .h_flex()
            .items_stretch()
            .size_full()
            .min_h_0()
            .font_family("Microsoft YaHei UI")
            .text_sm()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(self.nav(window, cx))
            .child(
                div()
                    .v_flex()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .h_full()
                    .child(topbar)
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
                                        .child(self.t("设置未保存，请检查目录权限后重试。")),
                                )
                                .child(
                                    self.button("retry-save", "重试", Command::RetrySave, cx)
                                        .small(),
                                ),
                        )
                    })
                    .child(div().v_flex().flex_1().min_h_0().min_w_0().child(content))
                    .when(self.c.update_busy && !self.c.checking, |d| {
                        d.child(self.download_bar(cx))
                    })
                    .when(self.c.closing, |d| d.child(self.t(self.c.close_status()))),
            )
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
    gpui::application()
        .with_assets(crate::ui_assets::Assets)
        .run(move |cx| {
            gpui::init(cx);
            crate::ui_focus::init(cx);
            if let Err(error) = crate::ui_themes::register(cx) {
                win::message(
                    "RTX Manager",
                    &format!("Unable to load bundled themes: {error:#}"),
                );
                cx.quit();
                return;
            }
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
            crate::ui_themes::apply(
                mode,
                state["ui_theme_light"].as_str().unwrap_or("Default Light"),
                state["ui_theme_dark"].as_str().unwrap_or("Default Dark"),
                cx,
            );
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
            let mut bounds = fitted_bounds(work);
            if smoke.is_some()
                || (cfg!(debug_assertions) && std::env::var_os("RTXFG_RECIPE_PREVIEW").is_some())
            {
                let w = state["smoke_window_width"]
                    .as_f64()
                    .unwrap_or(f32::from(bounds.size.width) as f64) as f32;
                let h = state["smoke_window_height"]
                    .as_f64()
                    .unwrap_or(f32::from(bounds.size.height) as f64) as f32;
                bounds = Bounds::centered_at(
                    work.center(),
                    size(px(w).min(work.size.width), px(h).min(work.size.height)),
                );
            }
            let window_size = bounds.size;
            let options = WindowOptions {
                titlebar: Some(TitlebarOptions {
                    title: Some("RTX 帧生成管理器".into()),
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
                window.on_window_should_close(cx, move |window, cx| {
                    close
                        .update(cx, |this, cx| {
                            this.reconcile_runtime_window(cx);
                            this.reconcile_settings_window(cx);
                            if this.any_preset_pending() {
                                if !this.editor_open {
                                    if let Some((exe, scheme)) = this.c.first_pending_preset() {
                                        this.c.focus = Some(exe);
                                        if let Some(index) = this
                                            .c
                                            .catalog
                                            .schemes()
                                            .iter()
                                            .position(|p| p.scheme_id == scheme)
                                        {
                                            this.profile_set(index, this.c.cloud_series());
                                        }
                                    }
                                    window.close_dialog(cx);
                                    this.dialog_active = false;
                                    this.game_editor(window, cx);
                                }
                                this.preset_close = Some(true);
                            } else {
                                this.c.close();
                            }
                            cx.notify();
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

fn runtime_version_indicator(current: String, selected: &str, cx: &App) -> AnyElement {
    div()
        .h_flex()
        .flex_wrap()
        .gap_1p5()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(current)
        .child("→")
        .child(
            div()
                .text_color(cx.theme().foreground)
                .child(selected.to_owned()),
        )
        .into_any_element()
}

fn component_installation_label(
    name: &str,
    present: usize,
    total: usize,
    missing: bool,
    warning: bool,
    tr: &rtx_fg_manager::i18n::Translator,
) -> Option<(String, bool)> {
    let (status, needs_check) = if missing {
        (tr.t("文件缺失"), true)
    } else if warning {
        (tr.t("需检查"), true)
    } else if present == 0 {
        return None;
    } else if present < total {
        (format!("{} {present}/{total}", tr.t("部分安装")), true)
    } else {
        (tr.t("已安装"), false)
    };
    Some((format!("{name} {status}"), needs_check))
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
        } else if matches!(profile, "SM75_R2" | "RTX20" | "RTX30") {
            "R2".into()
        } else {
            profile.into()
        };
        (
            format!(
                "已安装 · {} · {}",
                version,
                parts.next().unwrap_or_default()
            ),
            TagVariant::Success,
        )
    } else if status == "未部署" {
        ("未安装".into(), TagVariant::Secondary)
    } else if status.starts_with("补丁已移除，缓存待清理") {
        ("残留文件待清理".into(), TagVariant::Warning)
    } else if status.starts_with("正在") {
        (status.into(), TagVariant::Info)
    } else {
        (
            rtx_fg_manager::deployment::IssueKind::from_status(status)
                .map(|k| k.label())
                .unwrap_or("查看补丁状态")
                .into(),
            TagVariant::Warning,
        )
    }
}
fn preferred_reduce_motion(state: &Value, system: bool) -> bool {
    match state["ui_motion"].as_str() {
        Some("system") => system,
        Some("off") => true,
        _ => false,
    }
}
fn preferred_font_size(state: &Value) -> f32 {
    state["ui_font_size"]
        .as_f64()
        .filter(|n| n.is_finite())
        .unwrap_or(13.)
        .round()
        .clamp(12., 18.) as f32
}
struct LayoutMetrics {
    nav: f32,
    collapsed: bool,
    compact: bool,
}
impl LayoutMetrics {
    fn new(width: f32, scale: f32, collapsed: bool) -> Self {
        let collapsed = collapsed || width < 760. * scale;
        let nav = if collapsed { 56. } else { 208. } * scale;
        Self {
            nav,
            collapsed,
            compact: width - nav < 720. * scale,
        }
    }
}
fn fitted_bounds(work: Bounds<Pixels>) -> Bounds<Pixels> {
    let width = px(1200.).min((work.size.width - px(32.)).max(px(1.)));
    let height = px(790.).min((work.size.height - px(32.)).max(px(1.)));
    Bounds::centered_at(work.center(), size(width, height))
}

#[cfg(test)]
fn game_display_name(title: &str, exe: &str) -> String {
    rtx_fg_manager::game_names::original(&rtx_fg_manager::scanner::Game {
        title: title.into(),
        exe: exe.into(),
        ..Default::default()
    })
}

#[cfg(test)]
mod layout_tests {
    use super::{GameFilter, deployment_label, fitted_bounds, game_display_name};
    use gpui::component::tag::TagVariant;
    use gpui::{Bounds, point, px, size};
    #[test]
    fn motion_defaults_on_and_preserves_explicit_choices() {
        for system in [false, true] {
            for value in [
                serde_json::Value::Null,
                serde_json::json!("on"),
                serde_json::json!("invalid"),
            ] {
                assert!(!super::preferred_reduce_motion(
                    &serde_json::json!({"ui_motion":value}),
                    system
                ));
            }
            assert_eq!(
                super::preferred_reduce_motion(&serde_json::json!({"ui_motion":"system"}), system),
                system
            );
            assert!(super::preferred_reduce_motion(
                &serde_json::json!({"ui_motion":"off"}),
                system
            ));
        }
    }
    #[test]
    fn font_preferences_and_responsive_panes_remain_bounded() {
        for (value, expected) in [
            (serde_json::Value::Null, 13.),
            (serde_json::json!("large"), 13.),
            (serde_json::json!(17), 17.),
            (serde_json::json!(0), 12.),
            (serde_json::json!(40), 18.),
        ] {
            assert_eq!(
                super::preferred_font_size(&serde_json::json!({"ui_font_size":value})),
                expected
            );
        }
        for dpi in [1., 1.25, 1.5, 2., 3.] {
            for physical_width in [800., 1366., 1920., 2560., 3840.] {
                for font in [12., 13., 14., 18.] {
                    for collapsed in [false, true] {
                        let scale = font / 14.;
                        let width = physical_width / dpi;
                        let layout = super::LayoutMetrics::new(width, scale, collapsed);
                        assert!(layout.nav < width);
                        if !layout.compact {
                            assert!(width - layout.nav >= 640. * scale);
                        }
                        if width < 760. * scale {
                            assert!(layout.collapsed);
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn inspector_uses_executable_name_when_scanned_title_is_empty() {
        assert_eq!(
            game_display_name("Stormgate", "C:\\Games\\Game.exe"),
            "Stormgate"
        );
        assert_eq!(game_display_name("", "C:\\Games\\Game.exe"), "Game");
        assert_eq!(game_display_name("  ", "/games/Other.exe"), "Other");
    }
    #[test]
    fn library_directories_show_current_targets_without_duplicates_or_cleanup_remnants() {
        let root = std::env::temp_dir().join("library-directory-display");
        let first = root.join("Engine/Binaries/Win64");
        let second = root.join("Engine/Binaries/Win64rh");
        let mut game = rtx_fg_manager::scanner::Game {
            exe: first.join("game.exe").display().to_string(),
            root: root.display().to_string(),
            targets: vec![
                first.join("game.exe").display().to_string(),
                second.join("game.exe").display().to_string(),
                first.join("alternate.exe").display().to_string(),
            ],
            cleanup_only: vec![root.join("old/game.exe").display().to_string()],
            ..Default::default()
        };
        let directories = super::game_directories(&game);
        assert_eq!(directories.len(), 2);
        assert_eq!(directories[0], first.display().to_string());
        assert_eq!(directories[1], second.display().to_string());
        game.deployment_dir = root.join("plugins").display().to_string();
        assert_eq!(
            super::game_directories(&game),
            [game.deployment_dir.clone()]
        );
        game.deployment_dir.clear();
        game.targets.clear();
        assert_eq!(super::game_directories(&game).len(), 1);
    }
    #[test]
    fn rescan_added_directory_updates_same_game_and_survives_save() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("Game");
        let first = root
            .join("Binaries/Win64/Game-Win64-Shipping.exe")
            .display()
            .to_string();
        let second = root
            .join("Binaries/Win64rh/Game-Win64-Shipping.exe")
            .display()
            .to_string();
        let mut c = crate::controller::Controller::new(
            dir.path().into(),
            serde_json::json!({"language":"zh-CN", "games":[], "auto_update":false}),
            None,
            Some(dir.path().join("smoke.json")),
        );
        let mut game = rtx_fg_manager::scanner::Game {
            exe: first.clone(),
            root: root.display().to_string(),
            targets: vec![first],
            ..Default::default()
        };
        c.merge_scan(vec![game.clone()]);
        c.focus = Some(game.exe.clone());
        let revision = c.library_revision;
        game.targets.push(second.clone());
        c.merge_scan(vec![game]);
        assert_eq!(c.games.len(), 1);
        assert_ne!(c.library_revision, revision);
        assert_eq!(super::game_directories(&c.games[0]).len(), 2);
        assert!(c.patch_targets(false).contains(&second));
        let saved: rtx_fg_manager::scanner::Game =
            serde_json::from_value(c.state["games"][0].clone()).unwrap();
        assert_eq!(
            super::game_directories(&saved),
            super::game_directories(&c.games[0])
        );
        c.close();
        c.tick_close();
    }
    #[test]
    fn search_rebuilds_after_drag_reorder_and_same_count_rescan_relabels_library() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = crate::controller::Controller::new(
            dir.path().into(),
            serde_json::json!({"language":"zh-CN", "games":[], "auto_update":false}),
            None,
            Some(dir.path().join("smoke.json")),
        );
        let rows = ["Alpha", "Beta"].map(|name| {
            let exe = dir
                .path()
                .join(name)
                .join("Binaries/Win64")
                .join(format!("{name}-Win64-Shipping.exe"));
            rtx_fg_manager::scanner::Game {
                title: name.into(),
                root: rtx_fg_manager::scanner::installation_root(&exe)
                    .display()
                    .to_string(),
                exe: exe.display().to_string(),
                ..Default::default()
            }
        });
        c.merge(rows.to_vec());
        c.search = "alpha".into();
        let mut filter = GameFilter::default();
        filter.refresh(&c);
        assert_eq!(filter.indices, [0]);
        let revision = c.library_revision;
        c.merge_scan(vec![rows[0].clone()]);
        assert_eq!(c.games.len(), 2);
        // Rescans preserve user order (since the drag-sort feature in 4.3.2).
        assert_eq!(c.games[0].title, "Alpha");
        c.reorder_game(&rows[0].exe, &rows[1].exe, true);
        assert_eq!(c.games[0].title, "Beta");
        assert_eq!(c.games[1].title, "Alpha");
        assert_ne!(c.library_revision, revision);
        filter.refresh(&c);
        assert_eq!(filter.indices, [1]);
        assert_eq!(c.games[filter.indices[0]].title, "Alpha");

        // Query changes and metadata-only replacements invalidate the same cache.
        c.search = "renamed".into();
        filter.refresh(&c);
        assert!(filter.indices.is_empty());
        let mut renamed = rows[0].clone();
        renamed.title = "Renamed".into();
        c.merge_scan(vec![renamed]);
        filter.refresh(&c);
        assert_eq!(filter.indices, [1]);
        let exe = c.games[1].exe.clone();
        c.forget(&std::collections::BTreeSet::from([exe]));
        filter.refresh(&c);
        assert!(filter.indices.is_empty());
        c.close();
        c.tick_close();
    }
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
                "已安装 · 0.2.5 · version.dll",
            ),
            (
                "已部署 NATIVE_X6_SM75 / winmm.dll, dxgi.dll",
                "已安装 · 0.2.4 5X/6X · winmm.dll, dxgi.dll",
            ),
            ("已部署 SM75_R2 / version.dll", "已安装 · R2 · version.dll"),
            (
                "已部署 NATIVE30 @0.2.6 / version.dll",
                "已安装 · 0.2.6 · version.dll",
            ),
            (
                "已部署 NATIVE_X6_20 @0.2.6 / winmm.dll",
                "已安装 · 0.2.6 5X/6X · winmm.dll",
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
        assert_eq!(deployment_label("未部署").0, "未安装");
    }

    #[test]
    fn component_labels_distinguish_partial_missing_and_uninstalled() {
        let tr = rtx_fg_manager::i18n::Translator::new("zh-CN");
        for name in ["DLSSNR", "XeSS"] {
            assert_eq!(
                super::component_installation_label(name, 0, 2, false, false, &tr),
                None
            );
            for (count, missing, warning, status, needs_check) in [
                (2, false, false, "已安装", false),
                (1, false, false, "部分安装 1/2", true),
                (1, true, false, "文件缺失", true),
                (0, true, true, "文件缺失", true),
                (2, false, true, "需检查", true),
            ] {
                assert_eq!(
                    super::component_installation_label(name, count, 2, missing, warning, &tr),
                    Some((format!("{name} {status}"), needs_check))
                );
            }
        }
        let en = rtx_fg_manager::i18n::Translator::new("en");
        assert_eq!(
            super::component_installation_label("XeSS", 0, 1, true, false, &en),
            Some(("XeSS Files missing".into(), true))
        );
    }
}
