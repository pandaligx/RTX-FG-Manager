//! A single game-scoped dialog; browsing, login, sharing and preview keep context.
use super::*;
use gpui_component::avatar::Avatar;
use gpui_component::button::DropdownButton;
use gpui_component::input::{Textarea, TextareaState};
use gpui_component::scroll::{Scrollbar, ScrollbarMode};
use gpui_component::table::{Table, TableBody, TableCell, TableHead, TableHeader, TableRow};
use rtx_fg_manager::{cloud, game_names, recommendations as rec, scanner::Game};
use rtx_fg_manager::{encore, recommendation_presets, recommendation_runtimes};
use rtx_fg_manager::{presets, recommendation_presets::Action as RecipeAction};
use std::sync::{Arc, mpsc};

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Browse,
    Identity,
    Login,
    Share,
    Preview,
    Selected,
    Remove,
    Report,
}
enum Update {
    Init(rec::Client),
    Page(rec::FeedPage, bool),
    Games(String, Vec<rec::GameCandidate>),
    Draft(rec::Recipe, bool),
    Code(rec::Challenge, String),
    Login(rec::Client),
    Logout(rec::Client),
    Card(rec::Card),
    Voted(rec::Card),
    Removed,
    Reported(rec::Card),
}
struct Recommendations {
    owner: WeakEntity<Manager>,
    game: Game,
    identity: Option<rec::GameIdentity>,
    link_suggestion: Option<rec::GameLink>,
    show_history: bool,
    catalog: cloud::Catalog,
    names: Arc<game_names::Catalog>,
    labels: HashMap<String, String>,
    tr: i18n::Translator,
    client: Option<rec::Client>,
    mode: Mode,
    cards: Vec<rec::Card>,
    scroll_handles: HashMap<String, ScrollHandle>,
    candidates: Vec<rec::GameCandidate>,
    next: Option<rec::FeedCursor>,
    visible: usize,
    total: u64,
    loaded: bool,
    mine: bool,
    reported: bool,
    report_reason: String,
    cached: bool,
    busy: bool,
    message: String,
    gpu: String,
    hardware_gpu: String,
    driver: String,
    draft: Option<rec::Recipe>,
    editing: Option<rec::Card>,
    preview: Option<rec::Card>,
    preview_generation: u64,
    runtime_plan:
        Option<std::collections::BTreeMap<(String, String), recommendation_runtimes::RuntimeStep>>,
    recheck_after_install: bool,
    selected_action: RecipeAction,
    selected_parts: [bool; 3],
    operation_busy: bool,
    submission: String,
    title: Entity<InputState>,
    edition: Entity<InputState>,
    notes: Entity<TextareaState>,
    gpu_input: Entity<InputState>,
    email: Entity<InputState>,
    email_domain: String,
    code: Entity<InputState>,
    query: Entity<InputState>,
    challenge: Option<(rec::Challenge, String)>,
    resend: Instant,
    cooldown: bool,
    refresh_at: Instant,
    share_after_login: bool,
    tx: mpsc::Sender<std::result::Result<Update, String>>,
    rx: mpsc::Receiver<std::result::Result<Update, String>>,
}
impl Recommendations {
    fn new(
        owner: WeakEntity<Manager>,
        game: Game,
        c: &Controller,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        crate::ui_focus::observe(cx);
        if let Some(owner) = owner.upgrade() {
            cx.observe(&owner, |_, _, cx| cx.notify()).detach();
        }
        cx.observe_global::<Theme>(|_, cx| cx.notify()).detach();
        let identity = rec::identify(&game, &c.game_names);
        let input = |value: &str, window: &mut Window, cx: &mut Context<Self>| {
            cx.new(|cx| {
                let mut state = InputState::new(window, cx);
                state.set_value(value, window, cx);
                state
            })
        };
        let suggested_link = rec::local_link_suggestion(&game, &c.game_names);
        let query = input(&game_names::display(&game, &c.game_names, true), window, cx);
        cx.subscribe(&query, |this, _, event, cx| {
            if matches!(event, InputEvent::Change) {
                this.update_candidates(&[], cx);
                cx.notify();
            }
        })
        .detach();
        let title = input("", window, cx);
        let edition = input("", window, cx);
        let notes = cx.new(|cx| TextareaState::new(window, cx).auto_grow(4, 8));
        cx.subscribe(&notes, |_, _, _: &InputEvent, cx| cx.notify())
            .detach();
        let email = input("", window, cx);
        cx.subscribe_in(&email, window, |this, _, event, window, cx| {
            if matches!(event, InputEvent::Blur) {
                let value = this.email.read(cx).value().to_string();
                let complete = rec::complete_email(&value, &this.email_domain);
                if complete != value {
                    this.email
                        .update(cx, |s, cx| s.set_value(complete, window, cx));
                }
            }
            if matches!(event, InputEvent::Change | InputEvent::Blur) {
                cx.notify();
            }
        })
        .detach();
        let code = input("", window, cx);
        cx.subscribe(&code, |_, _, event, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        })
        .detach();
        let (tx, rx) = mpsc::channel();
        let gpu = c
            .devices
            .get(c.device_index)
            .map(|d| d.original_name.as_ref().unwrap_or(&d.name).clone())
            .unwrap_or_else(|| c.gpu.clone());
        let driver = c
            .devices
            .get(c.device_index)
            .map(|d| d.version.clone())
            .unwrap_or_default();
        let gpu_input = input(&gpu, window, cx);
        let mut view = Self {
            owner,
            game,
            identity: identity.clone(),
            link_suggestion: suggested_link,
            show_history: false,
            catalog: c.catalog.clone(),
            names: c.game_names.clone(),
            labels: c
                .catalog
                .scheme_policies
                .keys()
                .map(|id| (id.clone(), c.cloud_label(c.catalog.selected(id))))
                .collect(),
            tr: i18n::Translator::new(&c.tr.language),
            client: None,
            mode: if identity.is_some() {
                Mode::Browse
            } else {
                Mode::Identity
            },
            cards: vec![],
            scroll_handles: HashMap::new(),
            candidates: vec![],
            next: None,
            visible: 12,
            total: 0,
            loaded: false,
            mine: false,
            reported: false,
            report_reason: "方案信息不实".into(),
            cached: false,
            busy: false,
            message: String::new(),
            hardware_gpu: gpu.clone(),
            gpu,
            driver,
            draft: None,
            editing: None,
            preview: None,
            preview_generation: 0,
            runtime_plan: None,
            recheck_after_install: false,
            selected_action: RecipeAction::InstallAndApply,
            selected_parts: [true; 3],
            operation_busy: false,
            submission: String::new(),
            title,
            edition,
            notes,
            gpu_input,
            email,
            email_domain: "qq.com".into(),
            code,
            query,
            challenge: None,
            resend: Instant::now(),
            cooldown: false,
            refresh_at: Instant::now() + Duration::from_secs(45),
            share_after_login: false,
            tx,
            rx,
        };
        // Development-only visual review uses a validated public sample. It never
        // sends requests, and the hook is absent from signed release builds.
        #[cfg(debug_assertions)]
        if let Ok(path) = std::env::var("RTXFG_RECIPE_PREVIEW")
            && let Ok(bytes) = std::fs::read(path)
            && bytes.len() <= 65536
            && let Ok(card) = serde_json::from_slice::<rec::Card>(&bytes)
            && card.content.validate(&view.catalog).is_ok()
        {
            if std::env::var("RTXFG_RECIPE_PREVIEW_MODE").as_deref() == Ok("share") {
                view.title.update(cx, |s, cx| {
                    s.set_value(card.content.title.clone(), window, cx)
                });
                view.notes.update(cx, |s, cx| {
                    s.set_value(card.content.notes.clone(), window, cx)
                });
                view.draft = Some(card.content);
                view.mode = Mode::Share;
            } else {
                view.preview = Some(card);
                view.mode = Mode::Preview;
                view.inspect_runtime_plan(cx);
            }
            return view;
        }
        let data = c.data.clone();
        view.job(move || Ok(Update::Init(rec::Client::new(&data)?)));
        cx.spawn_in(window, async move |this, cx| {
            loop {
                smol::Timer::after(Duration::from_millis(120)).await;
                if cx
                    .update(|window, cx| this.update(cx, |this, cx| this.poll(window, cx)))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        view
    }
    fn t(&self, s: &str) -> String {
        self.tr.t(s)
    }
    fn avatar(cx: &App) -> Avatar {
        Avatar::new()
            .xsmall()
            .text_color(cx.theme().foreground)
            .placeholder(Icon::new(IconName::User).size_3())
            .flex_shrink_0()
    }
    fn button(&self, id: impl Into<ElementId>, label: &str, cx: &Context<Self>) -> Button {
        Button::new(id)
            .small()
            .label(self.t(label))
            .focus_ring(crate::ui_focus::visible(cx))
            .disabled(self.busy)
    }
    fn job(&mut self, f: impl FnOnce() -> anyhow::Result<Update> + Send + 'static) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.message = self.t("正在处理…");
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f))
                .map_err(|_| "后台操作异常终止".to_string())
                .and_then(|r| r.map_err(|e| e.to_string()));
            let _ = tx.send(result);
        });
    }
    fn poll(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let _ = self.owner.update(cx, |owner, cx| {
            if owner.accepts_feedback(window) && owner.c.confirm.is_some() {
                owner.dialogs(window, cx);
            }
            if self.operation_busy != owner.c.busy {
                self.operation_busy = owner.c.busy;
                cx.notify();
            }
        });
        if self.recheck_after_install && !self.operation_busy {
            self.recheck_after_install = false;
            if self.mode == Mode::Preview {
                self.inspect_runtime_plan(cx);
            }
        }
        while let Ok(result) = self.rx.try_recv() {
            self.busy = false;
            self.message.clear();
            if !self.logged_in() {
                self.mine = false;
                self.reported = false;
                for card in self.cards.iter_mut().chain(self.preview.iter_mut()) {
                    card.mine = false;
                    card.liked = false;
                    card.disliked = false;
                    card.reported = false;
                }
            }
            match result {
                Err(e) => {
                    if e == "登录已过期，请重新获取邮箱验证码" {
                        self.login(false, window, cx);
                    }
                    super::tasks::notify(super::tasks::FeedbackKind::Error, self.t(&e), window, cx);
                }
                Ok(Update::Init(client)) => {
                    self.client = Some(client);
                    if self.identity.is_some() {
                        self.refresh(false);
                        self.find_historical_shares(cx);
                    } else {
                        self.find_candidates(true, cx);
                    }
                }
                Ok(Update::Page(page, append)) => {
                    if !append {
                        self.cards.clear();
                        self.visible = 12;
                        self.total = page.total;
                    }
                    for card in page.items {
                        if let Some(old) = self.cards.iter_mut().find(|v| v.id == card.id) {
                            *old = card;
                        } else {
                            self.cards.push(card);
                        }
                    }
                    rec::rank(&mut self.cards);
                    self.next = page.next;
                    self.loaded = true;
                    self.cached = page.notice.is_some();
                    if append {
                        self.visible += 12;
                    }
                    if let Some(message) = page.notice {
                        self.message = self.t(message);
                    }
                }
                Ok(Update::Games(query, games)) => {
                    if self.query.read(cx).value().as_str() != query {
                        cx.notify();
                        continue;
                    }
                    self.update_candidates(&games, cx);
                    if self.candidates.is_empty() {
                        self.message = self.t("未找到候选，请核对完整游戏名后确认");
                    }
                }
                Ok(Update::Draft(draft, reload)) => {
                    if reload
                        && self.editing.is_none()
                        && self.draft.as_ref().is_some_and(|old| {
                            self.title.read(cx).value().as_str() == old.suggested_title()
                        })
                    {
                        self.title
                            .update(cx, |s, cx| s.set_value(draft.title.clone(), window, cx));
                    }
                    if !reload {
                        self.editing = None;
                        self.title
                            .update(cx, |s, cx| s.set_value(self.tr.t(&draft.title), window, cx));
                        self.notes
                            .update(cx, |s, cx| s.set_value(draft.notes.clone(), window, cx));
                        self.edition
                            .update(cx, |s, cx| s.set_value(draft.edition.clone(), window, cx));
                        self.submission = uuid::Uuid::new_v4().to_string();
                    }
                    self.gpu_input
                        .update(cx, |s, cx| s.set_value(draft.gpu_model.clone(), window, cx));
                    self.draft = Some(draft);
                    self.mode = Mode::Share;
                }
                Ok(Update::Code(code, email)) => {
                    self.cooldown = true;
                    self.resend = Instant::now() + Duration::from_secs(u64::from(code.retry_after));
                    self.challenge = Some((code, email));
                    self.message = self.t("验证码已发送，请查看邮箱（含垃圾箱）");
                }
                Ok(Update::Login(client)) => {
                    self.client = Some(client);
                    self.code.update(cx, |s, cx| s.set_value("", window, cx));
                    self.challenge = None;
                    let close_login = self.mode == Mode::Login;
                    self.mode = Mode::Browse;
                    if close_login {
                        window.defer(cx, |window, cx| window.close_dialog(cx));
                    }
                    if std::mem::take(&mut self.share_after_login) {
                        self.open_share(window, cx);
                    } else {
                        self.refresh(false);
                    }
                }
                Ok(Update::Logout(client)) => {
                    self.client = Some(client);
                    self.clear_page();
                    self.mine = false;
                    self.reported = false;
                    self.mode = Mode::Browse;
                    self.refresh(false);
                }
                Ok(Update::Card(card)) => {
                    let published_in_other_group = self
                        .browse_identity()
                        .is_some_and(|id| !id.accepts_group(&card.content.game.id, &self.names));
                    self.editing = None;
                    self.mode = Mode::Browse;
                    if published_in_other_group {
                        self.show_history = false;
                        self.cards.clear();
                        self.refresh(false);
                        cx.notify();
                        continue;
                    }
                    if let Some(old) = self.cards.iter_mut().find(|v| v.id == card.id) {
                        *old = card;
                    } else {
                        self.cards.insert(0, card);
                        self.total += 1;
                    }
                    rec::rank(&mut self.cards);
                    self.cached = false;
                    self.message = self.t("已更新");
                }
                Ok(Update::Voted(card)) => {
                    if let Some(preview) = self.preview.as_mut().filter(|p| p.id == card.id) {
                        preview.clone_from(&card);
                    }
                    if let Some(existing) = self.cards.iter_mut().find(|c| c.id == card.id) {
                        *existing = card;
                        rec::rank(&mut self.cards);
                    }
                    self.message.clear();
                }
                Ok(Update::Removed) => {
                    self.preview = None;
                    self.mode = Mode::Browse;
                    self.refresh(false);
                }
                Ok(Update::Reported(card)) => {
                    self.message = self.t(if card.reported {
                        "已举报，可在我的举报中撤销"
                    } else {
                        "已撤销举报"
                    });
                    if let Some(old) = self.cards.iter_mut().find(|v| v.id == card.id) {
                        *old = card;
                    }
                    self.mode = Mode::Browse;
                    self.refresh(false);
                }
            }
            cx.notify();
        }
        if !self.busy && !self.operation_busy && !self.message.is_empty() {
            super::tasks::notify(
                super::tasks::FeedbackKind::Info,
                std::mem::take(&mut self.message),
                window,
                cx,
            );
            cx.notify();
        }
        if self.cooldown && Instant::now() >= self.resend {
            self.cooldown = false;
            cx.notify();
        }
        if self.mode == Mode::Browse
            && !self.busy
            && self.identity.is_some()
            && Instant::now() >= self.refresh_at
        {
            self.refresh(false);
            cx.notify();
        }
    }
    fn refresh(&mut self, append: bool) {
        let (Some(client), Some(id)) = (self.client.clone(), self.browse_identity()) else {
            return;
        };
        let catalog = self.catalog.clone();
        let names = self.names.clone();
        let more = if append { self.next.clone() } else { None };
        let filter = if self.mine {
            rec::FeedFilter::Mine
        } else if self.reported {
            rec::FeedFilter::Reported
        } else {
            rec::FeedFilter::Public
        };
        self.refresh_at = Instant::now() + Duration::from_secs(45);
        self.job(move || {
            Ok(Update::Page(
                client.browse_feeds(&id, &names, &catalog, filter, more)?,
                append,
            ))
        });
    }
    fn browse_identity(&self) -> Option<rec::GameIdentity> {
        let primary = self.identity.as_ref()?;
        if self.show_history
            && let Some(link) = rec::GameLink::saved(&self.game, primary)
        {
            return Some(link.historical);
        }
        Some(primary.clone())
    }

    fn update_candidates(&mut self, online: &[rec::GameCandidate], cx: &Context<Self>) {
        let query = self.query.read(cx).value();
        let local = self.names.search(rec::search_title(&query));
        self.candidates = rec::game_choices(&self.names, online, &local);
        if rec::platform(&self.game) == "netease"
            && let Some(entry) = game_names::identified(&self.game, &self.names)
        {
            let key = format!("catalog:{}", entry.id().to_ascii_lowercase());
            if let Some(hit) = self
                .candidates
                .iter_mut()
                .find(|c| c.identity.accepts_group(&key, &self.names))
            {
                let label = rec::platform_label("netease");
                if !hit.platforms.iter().any(|p| p == label) {
                    hit.platforms.push(label.into());
                }
            }
        }
    }

    fn find_candidates(&mut self, automatic: bool, cx: &Context<Self>) {
        self.update_candidates(&[], cx);
        let Some(client) = self.client.clone() else {
            return;
        };
        let query = self.query.read(cx).value().to_string();
        let term = if automatic {
            let Some(title) = rec::public_search_title(&self.names, &query) else {
                return;
            };
            title
        } else {
            query.clone()
        };
        self.job(move || Ok(Update::Games(query, client.search(&term)?)));
    }

    fn find_historical_shares(&mut self, cx: &mut Context<Self>) {
        let (Some(primary), Some(client)) = (self.identity.clone(), self.client.clone()) else {
            return;
        };
        if primary.id.starts_with("custom:") || rec::GameLink::saved(&self.game, &primary).is_some()
        {
            return;
        }
        let Some(term) = rec::historical_search_term(&self.game, &self.names, &primary) else {
            return;
        };
        cx.spawn(async move |this, cx| {
            let historical =
                smol::unblock(move || rec::historical_match(&term, &client.search(&term).ok()?))
                    .await;
            if let Some(historical) = historical {
                let _ = this.update(cx, |view, cx| {
                    if view.identity.as_ref() == Some(&primary)
                        && rec::GameLink::saved(&view.game, &primary).is_none()
                    {
                        view.link_suggestion = Some(rec::GameLink {
                            platform: primary,
                            historical,
                        });
                        cx.notify();
                    }
                });
            }
        })
        .detach();
    }

    fn inspect_runtime_plan(&mut self, cx: &mut Context<Self>) {
        self.runtime_plan = None;
        self.preview_generation += 1;
        let generation = self.preview_generation;
        let Some(card) = &self.preview else { return };
        let recipe = card.content.clone();
        if recommendation_runtimes::displayed_components(&recipe).is_empty() {
            return;
        }
        let game = self.game.clone();
        cx.spawn(async move |this, cx| {
            let plan = smol::unblock(move || {
                recommendation_runtimes::preview(&recipe, &game.deployment_targets())
            })
            .await;
            let _ = this.update(cx, |view, cx| {
                if view.preview_generation == generation {
                    view.runtime_plan = Some(plan);
                    cx.notify();
                }
            });
        })
        .detach();
    }
    fn logged_in(&self) -> bool {
        self.client.as_ref().is_some_and(|c| c.logged_in())
    }
    fn login(&mut self, share: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.mode == Mode::Login || window.has_active_dialog(cx) {
            return;
        }
        self.share_after_login = share;
        self.mode = Mode::Login;
        self.message.clear();
        let owner = cx.weak_entity();
        let parent = cx.entity();
        let editor = cx.new(|cx| {
            cx.observe(&parent, |_, _, cx| cx.notify()).detach();
            LoginForm(parent.downgrade())
        });
        let title: SharedString = self.t("邮箱登录").into();
        window.open_dialog(cx, move |dialog, window, cx| {
            let owner = owner.clone();
            dialog
                .centered()
                .width(
                    (window.rem_size() * 26.)
                        .min(window.viewport_size().width - window.rem_size() * 2.),
                )
                .title(crate::ui_focus::dialog_title(title.clone(), cx))
                .close_button(false)
                .child(editor.clone())
                .on_close(move |_, _, cx| {
                    let _ = owner.update(cx, |view, cx| {
                        if view.mode == Mode::Login {
                            view.mode = Mode::Browse;
                            view.share_after_login = false;
                            cx.notify();
                        }
                    });
                })
        });
        let email = self.email.clone();
        window.defer(cx, move |window, cx| {
            email.read(cx).focus_handle(cx).focus(window, cx)
        });
    }
    fn open_share(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.logged_in() {
            self.login(true, window, cx);
            return;
        }
        self.read_installed(false);
    }
    fn read_installed(&mut self, reload: bool) {
        if self.operation_busy {
            self.message = self.t("当前正在处理游戏，请稍后再试");
            return;
        }
        let Some(id) = self
            .editing
            .as_ref()
            .filter(|_| reload)
            .map(|c| c.content.game.clone())
            .or_else(|| self.identity.clone())
        else {
            self.mode = Mode::Identity;
            return;
        };
        let game = self.game.clone();
        let catalog = self.catalog.clone();
        let gpu = self.hardware_gpu.clone();
        let driver = self.driver.clone();
        self.job(move || {
            Ok(Update::Draft(
                rec::installed_recipe(&game, id, &catalog, gpu, driver)?,
                reload,
            ))
        });
    }
    fn edit_share(&mut self, card: rec::Card, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || self.cached || !card.mine || !self.logged_in() {
            return;
        }
        self.title.update(cx, |s, cx| {
            s.set_value(card.content.display_title(), window, cx)
        });
        self.notes.update(cx, |s, cx| {
            s.set_value(card.content.notes.clone(), window, cx)
        });
        self.edition.update(cx, |s, cx| {
            s.set_value(card.content.edition.clone(), window, cx)
        });
        self.gpu_input.update(cx, |s, cx| {
            s.set_value(card.content.gpu_model.clone(), window, cx)
        });
        self.draft = Some(card.content.clone());
        self.editing = Some(card);
        self.submission.clear();
        self.message.clear();
        self.mode = Mode::Share;
        cx.notify();
    }
    fn save_game_choice(&mut self, cx: &mut Context<Self>, change: impl FnOnce(&mut Game)) -> bool {
        if self.busy {
            return false;
        }
        let exe = &self.game.exe;
        let result = self.owner.update(cx, |owner, cx| {
            anyhow::ensure!(!owner.dialog_active, "请先关闭当前参数或确认窗口");
            anyhow::ensure!(
                !owner.c.busy && !owner.c.read_only,
                "当前正在处理游戏，请稍后再试"
            );
            let game = owner
                .c
                .games
                .iter_mut()
                .find(|g| &g.exe == exe)
                .ok_or_else(|| anyhow::anyhow!("游戏已从库中移除"))?;
            change(game);
            let saved = game.clone();
            owner.c.save();
            cx.notify();
            Ok::<_, anyhow::Error>(saved)
        });
        match result {
            Ok(Ok(game)) => {
                self.game = game;
                true
            }
            Ok(Err(e)) => {
                self.message = self.t(&e.to_string());
                false
            }
            Err(_) => {
                self.message = self.t("游戏已从库中移除");
                false
            }
        }
    }
    fn clear_page(&mut self) {
        self.cards.clear();
        self.total = 0;
        self.loaded = false;
        self.next = None;
        self.visible = 12;
        self.cached = false;
    }
    fn use_group(&mut self, history: bool) {
        self.show_history = history;
        self.clear_page();
        self.mode = if self.identity.is_some() {
            Mode::Browse
        } else {
            Mode::Identity
        };
        self.refresh(false);
    }
    fn set_game_choice(
        &mut self,
        id: Option<rec::GameIdentity>,
        link: Option<rec::GameLink>,
        cx: &mut Context<Self>,
    ) {
        if self.save_game_choice(cx, |game| {
            for key in [
                "recommendation_game",
                "recommendation_link",
                "recommendation_link_ignored",
            ] {
                game.extra.remove(key);
            }
            if let Some(id) = id {
                game.extra
                    .insert("recommendation_game".into(), serde_json::json!(id));
            }
            if let Some(link) = link {
                game.extra
                    .insert("recommendation_link".into(), serde_json::json!(link));
            }
        }) {
            self.identity = rec::identify(&self.game, &self.names);
            self.link_suggestion = rec::local_link_suggestion(&self.game, &self.names);
            self.use_group(false);
            self.find_historical_shares(cx);
        }
        cx.notify();
    }
    fn bind(&mut self, id: rec::GameIdentity, cx: &mut Context<Self>) {
        let link = self
            .link_suggestion
            .as_ref()
            .filter(|pair| {
                pair.platform.id == id.id
                    && self
                        .identity
                        .as_ref()
                        .is_some_and(|old| old.id == pair.historical.id)
            })
            .cloned();
        self.set_game_choice(Some(id), link, cx);
    }
    fn reset_identity(&mut self, cx: &mut Context<Self>) {
        self.set_game_choice(None, None, cx);
    }
    fn apply_selection(
        &mut self,
        action: RecipeAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(card) = self.preview.clone() else {
            return;
        };
        let exe = self.game.exe.clone();
        if self.mode != Mode::Selected
            && card
                .content
                .required_gpu_name()
                .is_some_and(|name| self.gpu != name)
        {
            self.selected_action = action;
            self.mode = Mode::Selected;
            self.message.clear();
            cx.notify();
            return;
        }
        let result = self.owner.update(cx, |owner, cx| {
            anyhow::ensure!(!owner.dialog_active, "请先关闭当前参数或确认窗口");
            anyhow::ensure!(
                !owner.c.busy && !owner.c.read_only && owner.c.games.iter().any(|g| g.exe == exe),
                "当前游戏已变化，请关闭后重新打开推荐"
            );
            anyhow::ensure!(
                self.browse_identity()
                    .is_some_and(|id| id.accepts_group(&card.content.game.id, &self.names)),
                "推荐不属于当前游戏"
            );
            card.content.available_version(&owner.c.catalog)?;
            owner.c.focus = Some(exe.clone());
            owner.feedback_window = Some(window.window_handle());
            owner.c.use_recommendation(card.content.clone(), action)?;
            cx.notify();
            Ok::<_, anyhow::Error>(())
        });
        match result {
            Ok(Ok(())) => {
                self.mode = Mode::Preview;
                self.operation_busy = true;
                self.recheck_after_install = true;
                self.message.clear();
            }
            Ok(Err(e)) => self.message = self.t(&e.to_string()),
            Err(_) => self.message = self.t("游戏已从库中移除"),
        }
        cx.notify();
    }
    fn parameter_comparison(&self, recipe: &rec::Recipe, cx: &Context<Self>) -> Div {
        use gpui_component::progress::Progress;
        let mut panel = div().v_flex().w_full().min_w_0();
        let Some(snapshot) = &recipe.presets else {
            return panel
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(self.t("此分享未附带参数，可仅安装方案。"));
        };
        let changes = snapshot.changes();
        panel = panel.child(
            div()
                .h_flex()
                .gap_2()
                .pb_2()
                .child(
                    div()
                        .font_weight(FontWeight::MEDIUM)
                        .child(self.t("预设参数")),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(self.t("仅显示修改项")),
                ),
        );
        if changes.is_empty() {
            return panel.child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(self.t("使用默认参数")),
            );
        }
        let fields = presets::parameters_for_scheme(&recipe.scheme, &snapshot.profile);
        let mut rows = Vec::new();
        for (key, before, after) in changes {
            let field = fields.iter().find(|p| p.key == key);
            let label = field.map(|p| p.label).unwrap_or(key);
            let value = |v: &str| {
                if snapshot.profile == encore::PROFILE
                    && encore::field(key).is_some_and(|f| {
                        matches!(
                            f.kind,
                            encore::FieldKind::Bool | encore::FieldKind::InheritBool
                        )
                    })
                {
                    return self.t(match v {
                        "0" => "关闭",
                        "1" => "开启",
                        "inherit" => "跟随游戏",
                        _ => v,
                    });
                }
                self.t(field
                    .and_then(|p| p.choices.iter().find(|(value, _)| *value == v))
                    .map(|(_, label)| *label)
                    .unwrap_or(v))
            };
            let range = recommendation_presets::numeric_range(&snapshot.profile, key).filter(
                |(min, max)| {
                    [before, after].iter().all(|v| {
                        v.parse::<f64>()
                            .is_ok_and(|n| n.is_finite() && n >= *min && n <= *max)
                    })
                },
            );
            let mut cells = vec![
                div()
                    .whitespace_normal()
                    .child(self.t(label))
                    .into_any_element(),
            ];
            for (side, v) in [before, after].iter().enumerate() {
                let color = if side == 1 {
                    cx.theme().primary
                } else {
                    cx.theme().muted_foreground
                };
                let mut cell = div().flex_1().min_w_0().v_flex().gap_1();
                if let Some((min, max)) = range {
                    let n = v.parse::<f64>().unwrap_or(min);
                    cell = cell.child(div().text_color(color).child(value(v))).child(
                        Progress::new((SharedString::from(format!("recipe-compare-{key}")), side))
                            .value(((n - min) / (max - min) * 100.) as f32)
                            .color(color)
                            .xsmall()
                            .w_full()
                            .accessibility_label(format!(
                                "{} {}: {}",
                                self.t(label),
                                self.t(if side == 0 { "默认值" } else { "玩家值" }),
                                v
                            )),
                    );
                } else {
                    cell = cell.child(
                        div()
                            .text_color(if side == 1 {
                                cx.theme().primary
                            } else {
                                cx.theme().foreground
                            })
                            .whitespace_normal()
                            .child(value(v)),
                    );
                }
                cells.push(cell.into_any_element());
            }
            rows.push(cells);
        }
        panel.child(self.recipe_table("recipe-parameters", &["参数", "默认值", "玩家值"], rows, cx))
    }
    fn runtime_inventory(
        &self,
        recipe: &rec::Recipe,
        available: Option<&str>,
        compatible: bool,
        cx: &Context<Self>,
    ) -> Div {
        let panel = div().v_flex().min_w_0().child(
            div()
                .pb_2()
                .font_weight(FontWeight::MEDIUM)
                .child(self.t("配套运行库")),
        );
        let components = recommendation_runtimes::displayed_components(recipe);
        if components.is_empty() {
            return panel.child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(self.t(if recipe.runtimes.is_some() {
                        "未检测到可共享的运行库"
                    } else {
                        "此分享未提供运行库版本"
                    })),
            );
        }
        let inspecting = self.mode == Mode::Preview;
        let sources: Vec<_> = [
            ("game", "游戏目录"),
            ("patch", "补丁内置"),
            ("managed", "管理器安装"),
        ]
        .into_iter()
        .filter(|(key, _)| components.iter().any(|c| c.source == *key))
        .collect();
        let mut headings = vec!["组件"];
        headings.extend(sources.iter().map(|(_, label)| *label));
        if inspecting {
            headings.push("本次操作");
        }
        let mut rows = Vec::new();
        for (kind, group) in recommendation_runtimes::grouped(&components) {
            let mut row = vec![
                div()
                    .child(self.t(recommendation_runtimes::label(kind)))
                    .into_any_element(),
            ];
            let mut actions = div().v_flex().min_w_0().gap_1();
            for &(source, label) in &sources {
                let Some(c) = group.iter().find(|c| c.source == source) else {
                    row.push(
                        div()
                            .text_color(cx.theme().muted_foreground)
                            .child("—")
                            .into_any_element(),
                    );
                    continue;
                };
                let versions = c
                    .versions
                    .iter()
                    .map(|v| {
                        if v == "unknown" {
                            self.t("版本未知")
                        } else {
                            v.clone()
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(" / ");
                let cell = div()
                    .v_flex()
                    .min_w_0()
                    .whitespace_normal()
                    .child(versions)
                    .when_some(c.package_version.as_ref(), |d, p| {
                        d.child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(format!("SDK {p}")),
                        )
                    });
                row.push(cell.into_any_element());
                if inspecting {
                    let outcome = if available.is_none() {
                        "当前无法安装"
                    } else {
                        recommendation_runtimes::preview_action(
                            source,
                            self.selected_parts[0],
                            self.selected_parts[2],
                            compatible,
                            self.runtime_plan.as_ref().and_then(|p| {
                                p.get(&(kind.to_owned(), source.to_owned())).copied()
                            }),
                        )
                    };
                    let mut line = div().min_w_0().whitespace_normal();
                    if group.len() > 1 {
                        line = line.child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(self.t(label)),
                        );
                    }
                    actions = actions.child(line.child(self.t(outcome)));
                }
            }
            if inspecting {
                row.push(actions.into_any_element());
            }
            rows.push(row);
        }
        let panel = panel.child(self.recipe_table("recipe-runtimes", &headings, rows, cx));
        let action = RecipeAction::Selected {
            patch: self.selected_parts[0],
            parameters: false,
            runtimes: self.selected_parts[2],
        };
        panel.when(
            inspecting && available.is_some_and(|v| recipe.completes_encore(v, action)),
            |d| {
                d.child(
                    div()
                        .pt_2()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(self.t("Encore 基础运行库会按游戏需要补齐。")),
                )
            },
        )
    }
    fn picker(
        &self,
        key: &'static str,
        value: &str,
        choices: &'static [(&'static str, &'static str)],
        cx: &Context<Self>,
    ) -> AnyElement {
        let owner = cx.entity().downgrade();
        let label = choices
            .iter()
            .find(|v| v.0 == value)
            .map(|v| v.1)
            .unwrap_or("未说明");
        let tr = i18n::Translator::new(&self.tr.language);
        self.button(key, label, cx)
            .icon(IconName::ChevronDown)
            .w_full()
            .dropdown_menu(move |mut menu, _, _| {
                for (value, label) in choices {
                    let owner = owner.clone();
                    menu = menu.item(PopupMenuItem::new(tr.t(label)).on_click(move |_, _, cx| {
                        let _ = owner.update(cx, |this, cx| {
                            if key == "recipe-report-reason" {
                                this.report_reason = (*value).into();
                            }
                            if let Some(d) = &mut this.draft {
                                match key {
                                    "recipe-renderer" => d.renderer = (*value).into(),
                                    "recipe-alias" => d.gpu_alias = (*value).into(),
                                    "recipe-platform" => d.platform = (*value).into(),
                                    _ => {}
                                }
                            }
                            cx.notify();
                        });
                    }));
                }
                menu
            })
            .into_any_element()
    }
    fn recipe_summary(&self, recipe: &rec::Recipe, cx: &Context<Self>) -> Div {
        let name = self
            .labels
            .get(&recipe.scheme)
            .cloned()
            .unwrap_or_else(|| recipe.scheme.clone());
        let base = name.split('·').next().unwrap_or(&name).trim();
        let displayed_version = if recipe.scheme == rtx_fg_manager::anr::SCHEME {
            rtx_fg_manager::anr::display_version(&recipe.patch_version)
        } else {
            recipe.patch_version.clone()
        };
        let alias = match recipe.gpu_alias.as_str() {
            "unchanged" => self.t("不需要修改显卡型号"),
            "rtx4060" => format!("{} RTX 4060", self.t("修改显卡型号为")),
            "rtx5090" => format!("{} RTX 5090", self.t("修改显卡型号为")),
            _ => self.t("显卡型号修改：玩家未说明"),
        };
        let renderer = match recipe.renderer.as_str() {
            "dx12" => "DX12",
            "dx11" => "DX11",
            "vulkan" => "Vulkan",
            _ => "—",
        };
        let neural = if let Some(component) = recipe
            .runtimes
            .as_ref()
            .and_then(|s| s.components.iter().find(|c| c.kind == "nr"))
        {
            component
                .versions
                .iter()
                .map(|v| {
                    if v == "unknown" {
                        self.t("版本未知")
                    } else {
                        v.clone()
                    }
                })
                .collect::<Vec<_>>()
                .join(" / ")
        } else {
            match recipe.neural_runtime.as_str() {
                "none" => self.t("未安装"),
                "unknown" => self.t("版本未知"),
                v => v.into(),
            }
        };
        let platform = format!(
            "{}{}",
            self.t(rec::platform_label(&recipe.platform)),
            if recipe.edition.is_empty() {
                String::new()
            } else {
                format!(" / {}", recipe.edition)
            }
        );
        let fields = [
            ("补丁方案", format!("{base} {displayed_version}")),
            ("DLL 入口", recipe.entries.join(" / ")),
            ("显卡改名", alias),
            ("图形 / NR", format!("{renderer} / {neural}")),
            ("游戏版本", platform),
            ("玩家硬件", recipe.gpu_model.clone()),
            ("驱动版本", recipe.driver.clone()),
        ];
        let rows = fields
            .into_iter()
            // Editable fields appear once in the confirmation table below.
            .filter(|(label, _)| {
                self.mode != Mode::Share || !matches!(*label, "显卡改名" | "游戏版本" | "玩家硬件")
            })
            .map(|(label, value)| {
                vec![
                    div()
                        .text_color(cx.theme().muted_foreground)
                        .child(self.t(label))
                        .into_any_element(),
                    div()
                        .whitespace_normal()
                        .when(
                            label == "显卡改名" && recipe.required_gpu_name().is_some(),
                            |d| d.text_color(cx.theme().warning),
                        )
                        .child(if value.is_empty() {
                            "—".to_owned()
                        } else {
                            value
                        })
                        .into_any_element(),
                ]
            })
            .collect();
        self.recipe_table("recipe-summary", &["项目", "方案内容"], rows, cx)
    }

    // Every section uses the same label lane, insets and grid lines. A short
    // static table preserves wrapping without adding DataTable selection state.
    fn recipe_table(
        &self,
        id: &'static str,
        headings: &[&str],
        rows: Vec<Vec<AnyElement>>,
        cx: &Context<Self>,
    ) -> Div {
        let last = headings.len() - 1;
        let label_width = if id == "recipe-runtimes" { 6.0 } else { 12.0 };
        let header = TableRow::new().children(headings.iter().enumerate().map(|(i, label)| {
            TableHead::new()
                .min_w_0()
                .text_color(cx.theme().foreground)
                .font_weight(FontWeight::MEDIUM)
                .px_2()
                .py_1()
                .when(i == 0, |cell| cell.w(rems(label_width)).flex_shrink_0())
                .when(i > 0, |cell| cell.flex_1())
                .when(i < last, |cell| {
                    cell.border_r_1().border_color(cx.theme().border)
                })
                .child(div().min_w_0().whitespace_normal().child(self.t(label)))
        }));
        let body = TableBody::new().children(rows.into_iter().map(|cells| {
            TableRow::new().children(cells.into_iter().enumerate().map(|(i, value)| {
                TableCell::new()
                    .min_w_0()
                    .px_2()
                    .py_1()
                    .when(i == 0, |cell| cell.w(rems(label_width)).flex_shrink_0())
                    .when(i > 0, |cell| cell.flex_1())
                    .when(i < last, |cell| {
                        cell.border_r_1().border_color(cx.theme().border)
                    })
                    .child(div().w_full().min_w_0().whitespace_normal().child(value))
            }))
        }));
        div().w_full().min_w_0().flex_shrink_0().child(
            div()
                .id(id)
                .w_full()
                .min_w_0()
                .border_1()
                .border_color(cx.theme().border)
                .rounded_md()
                .overflow_hidden()
                .child(
                    Table::new()
                        .small()
                        .accessibility_label(self.t(headings[1]))
                        .child(TableHeader::new().child(header))
                        .child(body),
                ),
        )
    }

    fn preview_sections(
        &self,
        recipe: &rec::Recipe,
        available: Option<&str>,
        compatible: bool,
        cx: &Context<Self>,
    ) -> Div {
        div()
            .v_flex()
            .gap_3()
            .min_w_0()
            .flex_shrink_0()
            .child(self.recipe_summary(recipe, cx))
            .child(self.parameter_comparison(recipe, cx))
            .child(self.runtime_inventory(recipe, available, compatible, cx))
            .child(self.player_notes(&recipe.notes, cx))
    }

    fn player_notes(&self, notes: &str, cx: &Context<Self>) -> Div {
        let copy = notes.to_owned();
        div()
            .v_flex()
            .gap_2()
            .min_w_0()
            .flex_shrink_0()
            .pt_3()
            .border_t_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .h_flex()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(self.t("玩家备注")),
                    )
                    .child(
                        self.button("recipe-copy-notes", "复制备注", cx)
                            .small()
                            .ghost()
                            .icon(IconName::Copy)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()));
                                this.message = this.t("备注已复制");
                                cx.notify();
                            })),
                    ),
            )
            .children(notes.lines().map(|line| {
                div()
                    .min_w_0()
                    .w_full()
                    .flex_shrink_0()
                    .text_sm()
                    .whitespace_normal()
                    .line_height(relative(1.4))
                    .when(line.is_empty(), |d| d.h_1())
                    .child(line.to_owned())
            }))
    }
    fn card_date(&self, card: &rec::Card) -> String {
        let (label, stamp) = if card.has_update() {
            ("更新于 {0}", card.updated_at)
        } else {
            ("分享于 {0}", card.created_at.unwrap_or(card.updated_at))
        };
        self.t(label)
            .replace("{0}", &rec::date(stamp).unwrap_or_else(|| "—".into()))
    }
    fn vote_button(&self, card: &rec::Card, useful: bool, cx: &Context<Self>) -> Button {
        let selected = if useful { card.liked } else { card.disliked };
        let vote = card.clone();
        let (icon, count, label) = if useful {
            (gpui::assets::IconName::ThumbsUp, card.likes, "有用")
        } else {
            (gpui::assets::IconName::ThumbsDown, card.dislikes, "无用")
        };
        let hint = format!("{} ({count})", self.t(label));
        Button::new(SharedString::from(format!(
            "recipe-vote-{}-{useful}",
            card.id
        )))
        .small()
        .ghost()
        .min_w(rems(2.2))
        .h(rems(2.4))
        .px_1()
        .focus_ring(crate::ui_focus::visible(cx))
        .child(
            div()
                .v_flex()
                .items_center()
                .justify_center()
                .gap(px(1.))
                .child(Icon::new(icon).size(rems(1.05)))
                .child(
                    div()
                        .text_xs()
                        .line_height(rems(1.))
                        .child(count.to_string()),
                ),
        )
        .accessibility_label(hint.clone())
        .tooltip(hint)
        .selected(selected)
        .when(selected, |b| {
            b.text_color(if useful {
                cx.theme().success
            } else {
                cx.theme().danger
            })
        })
        .disabled(self.busy || card.mine || self.cached)
        .on_click(cx.listener(move |this, _, window, cx| {
            if !this.logged_in() {
                this.login(false, window, cx);
            } else if let Some(client) = this.client.clone() {
                let card = vote.clone();
                let value = if selected {
                    0
                } else if useful {
                    1
                } else {
                    -1
                };
                let catalog = this.catalog.clone();
                this.job(move || Ok(Update::Voted(client.vote(&card, value, &catalog)?)));
            }
            cx.notify();
        }))
    }
    fn email_suffix(&self, cx: &Context<Self>) -> AnyElement {
        let owner = cx.entity().downgrade();
        let tr = i18n::Translator::new(&self.tr.language);
        let label = if self.email_domain.is_empty() {
            self.t("其他邮箱")
        } else {
            format!("@{}", self.email_domain)
        };
        self.button("recipe-email-domain", "", cx)
            .label(label)
            .icon(IconName::ChevronDown)
            .flex_shrink_0()
            .dropdown_menu(move |mut menu, _, _| {
                for domain in [
                    "qq.com",
                    "163.com",
                    "126.com",
                    "outlook.com",
                    "gmail.com",
                    "",
                ] {
                    let owner = owner.clone();
                    let label = if domain.is_empty() {
                        tr.t("其他邮箱")
                    } else {
                        format!("@{domain}")
                    };
                    menu = menu.item(PopupMenuItem::new(label).on_click(move |_, window, cx| {
                        let _ = owner.update(cx, |this, cx| {
                            this.email_domain = domain.into();
                            let value = this.email.read(cx).value().to_string();
                            if !domain.is_empty() {
                                let local = value.split('@').next().unwrap_or_default();
                                let next = rec::complete_email(local, domain);
                                this.email.update(cx, |s, cx| s.set_value(next, window, cx));
                            }
                            cx.notify();
                        });
                    }));
                }
                menu
            })
            .into_any_element()
    }
    fn browse(&self, cx: &Context<Self>) -> Div {
        let mut body = div()
            .v_flex()
            .gap_2()
            .px_0p5()
            .py_0p5()
            .flex_shrink_0()
            .min_w_0();
        // A failed request is not evidence that this game has no shared setups.
        // Its actual connection error is already shown beside the refresh controls.
        if self.loaded && self.cards.is_empty() && !self.busy && self.message.is_empty() {
            body = body.child(div().py_6().child(self.t(if self.reported {
                "你还没有举报这款游戏的方案"
            } else if self.mine {
                "此邮箱还没有分享这款游戏的方案。请使用分享时的邮箱登录。"
            } else {
                "这款游戏还没有推荐方案。测试可用后，欢迎分享。"
            })));
        }
        for card in self.cards.iter().take(self.visible) {
            let choose = card.clone();
            let edit = card.clone();
            let remove = card.clone();
            let report = card.clone();
            let title = div()
                .flex_1()
                .min_w_0()
                .whitespace_normal()
                .font_weight(FontWeight::SEMIBOLD)
                .child(card.content.display_title());
            let header = div()
                .h_flex()
                .items_center()
                .bg(cx.theme().muted)
                .px_2p5()
                .py_1p5()
                .gap_2()
                .min_w_0()
                .child(Self::avatar(cx))
                .child(title)
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .flex_shrink_0()
                        .child(self.card_date(card)),
                );
            let mut actions = div().h_flex().gap_2().flex_wrap().child(
                self.button(
                    SharedString::from(format!("recipe-preview-{}", card.id)),
                    "查看完整信息",
                    cx,
                )
                .disabled(self.busy || card.status == "hidden")
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.selected_parts = [true; 3];
                    this.preview = Some(choose.clone());
                    this.mode = Mode::Preview;
                    this.inspect_runtime_plan(cx);
                    this.message.clear();
                    cx.notify();
                })),
            );
            let mut feedback = div()
                .h_flex()
                .gap_1()
                .items_start()
                .child(self.vote_button(card, true, cx))
                .child(self.vote_button(card, false, cx));
            if card.mine {
                actions = actions
                    .child(
                        self.button(
                            SharedString::from(format!("recipe-edit-{}", card.id)),
                            "编辑方案",
                            cx,
                        )
                        .on_click(cx.listener(
                            move |this, _, window, cx| this.edit_share(edit.clone(), window, cx),
                        )),
                    )
                    .child(
                        self.button(
                            SharedString::from(format!("recipe-remove-{}", card.id)),
                            "删除分享",
                            cx,
                        )
                        .ghost()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.preview = Some(remove.clone());
                            this.mode = Mode::Remove;
                            cx.notify();
                        })),
                    );
            } else {
                let report_hint = self.t(if card.reported {
                    "撤销举报"
                } else {
                    "举报"
                });
                feedback = feedback.child(
                    self.button(
                        SharedString::from(format!("recipe-report-{}", card.id)),
                        "",
                        cx,
                    )
                    .small()
                    .ghost()
                    .icon(gpui::assets::IconName::Flag)
                    .h(rems(1.55))
                    .selected(card.reported)
                    .accessibility_label(report_hint.clone())
                    .tooltip(report_hint)
                    .disabled(self.busy || self.cached)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if !this.logged_in() {
                            this.login(false, window, cx);
                        } else if let Some(client) = this.client.clone() {
                            let card = report.clone();
                            if card.reported {
                                let catalog = this.catalog.clone();
                                this.job(move || {
                                    Ok(Update::Reported(client.report(&card, None, &catalog)?))
                                });
                            } else {
                                this.preview = Some(card);
                                this.mode = Mode::Report;
                            }
                        }
                        cx.notify();
                    })),
                );
            }
            let detail = div()
                .v_flex()
                .gap_1()
                .min_w_0()
                .flex_shrink_0()
                .px_2p5()
                .py_1p5()
                .border_t_1()
                .border_color(cx.theme().border)
                .child(
                    div()
                        .h_flex()
                        .flex_wrap()
                        .justify_between()
                        .items_center()
                        .gap_1()
                        .child(actions)
                        .child(feedback),
                )
                .when(card.status == "hidden", |d| {
                    d.child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().warning)
                            .child(self.t("此方案已隐藏，等待维护者审核")),
                    )
                });
            body = body.child(
                div()
                    .v_flex()
                    .min_w_0()
                    .flex_shrink_0()
                    .rounded_lg()
                    .border_1()
                    .border_color(cx.theme().input)
                    .bg(cx.theme().popover)
                    .shadow_xs()
                    .overflow_hidden()
                    .child(header)
                    .child(detail),
            );
        }
        if self.visible < self.cards.len() || self.next.is_some() {
            body = body.child(
                self.button("recipe-more", "加载更多", cx)
                    .on_click(cx.listener(|this, _, _, cx| {
                        if this.visible < this.cards.len() {
                            this.visible += 12;
                        } else {
                            this.refresh(true);
                        }
                        cx.notify();
                    })),
            );
        }
        body
    }
}
impl Render for Recommendations {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut game_image = None;
        let mut game_title = game_names::original(&self.game);
        if let Some(owner) = self.owner.upgrade() {
            let owner = owner.read(cx);
            if self.tr.language != owner.c.tr.language {
                self.tr = i18n::Translator::new(&owner.c.tr.language);
            }
            if self.catalog.revision != owner.c.catalog.revision {
                self.catalog = owner.c.catalog.clone();
            }
            self.operation_busy = owner.c.busy;
            self.hardware_gpu = owner
                .c
                .devices
                .get(owner.c.device_index)
                .map(|d| d.original_name.as_ref().unwrap_or(&d.name).clone())
                .unwrap_or_else(|| owner.c.gpu.clone());
            self.gpu = owner
                .c
                .devices
                .get(owner.c.device_index)
                .map(|d| {
                    d.modified_name
                        .as_ref()
                        .or(d.original_name.as_ref())
                        .unwrap_or(&d.name)
                        .clone()
                })
                .unwrap_or_else(|| owner.c.gpu.clone());
            game_image = owner.c.icons.get(&self.game.exe).and_then(|v| v.clone());
            if let Some(game) = owner.c.games.iter().find(|g| g.exe == self.game.exe) {
                self.game = game.clone();
            }
            game_title = owner.c.game_name(&self.game);
        }
        let top = self.page_header(cx);
        let mut primary = None;
        let mut install = InstallFooter::default();
        let body = match self.mode {
            Mode::Browse | Mode::Login => self.browse(cx),
            Mode::Identity => self.identity_page(cx),
            Mode::Share => self.share_page(window, &mut primary, cx),
            Mode::Preview => self.preview_page(&mut install, cx),
            Mode::Selected => {
                let name = self
                    .preview
                    .as_ref()
                    .and_then(|c| c.content.required_gpu_name())
                    .unwrap_or_default();
                primary = Some(
                    self.button("recipe-gpu-editor", "修改显卡型号", cx)
                        .disabled(self.operation_busy)
                        .primary()
                        .on_click(cx.listener(|this, _, window, cx| {
                            if this.owner.upgrade().is_none_or(|owner| {
                                let owner = owner.read(cx);
                                owner.dialog_active || owner.c.busy || owner.c.read_only
                            }) {
                                this.message = this.t("请先关闭当前参数或确认窗口");
                                cx.notify();
                                return;
                            }
                            if let Some(card)=&this.preview {
                                let _=this.owner.update(cx,|owner,_|{
                                    owner.c.focus = Some(this.game.exe.clone());
                                    owner.c.set_selection(card.content.scheme.clone(),owner.c.cloud_series(),card.content.entries.clone());
                                    if let Some(game)=owner.c.games.iter_mut().find(|g|g.exe==this.game.exe) {
                                        game.extra.insert("recommendation_gpu".into(),json!({"scheme":card.content.scheme,"name":card.content.required_gpu_name()}));
                                    }
                                    owner.c.save();
                                });
                            }

                            let _ = this
                                .owner
                                .update(cx, |owner, cx| { owner.feedback_window = Some(window.window_handle()); owner.gpu_editor(window, cx) });
                        })),
                );
                div().v_flex().gap_3()
                    .child(div().font_weight(FontWeight::SEMIBOLD).child(self.t("还需核对显卡型号")))
                    .child(self.t("此推荐要求 Windows 显卡显示名称为 {0}。仅选择补丁不会修改名称；若已符合则无需重复修改。").replace("{0}",name))
                    .child(div().text_sm().text_color(cx.theme().muted_foreground).child(self.t("修改后请按提示重启电脑，再安装补丁并按玩家备注设置游戏。")))
                    .child(self.button("recipe-finish-selection", "已了解，继续", cx).on_click(cx.listener(|this,_,window,cx| this.apply_selection(this.selected_action,window,cx))))
            }
            Mode::Report => {
                primary = Some(
                    self.button("recipe-send-report", "提交举报", cx)
                        .primary()
                        .on_click(cx.listener(|this, _, _, cx| {
                            if let (Some(client), Some(card)) =
                                (this.client.clone(), this.preview.clone())
                            {
                                let reason = this.report_reason.clone();
                                let catalog = this.catalog.clone();
                                this.job(move || {
                                    Ok(Update::Reported(client.report(
                                        &card,
                                        Some(&reason),
                                        &catalog,
                                    )?))
                                });
                            }
                            cx.notify();
                        })),
                );
                div().v_flex().gap_3()
                    .when_some(self.preview.as_ref(), |d,card| d.child(div().font_weight(FontWeight::SEMIBOLD).child(card.content.display_title())))
                    .child(self.t("举报原因"))
                    .child(self.picker("recipe-report-reason", &self.report_reason, &[("方案信息不实","方案信息不实"),("广告或无关内容","广告或无关内容"),("重复分享","重复分享"),("其他问题","其他问题")],cx))
                    .child(div().text_sm().text_color(cx.theme().muted_foreground).child(self.t("仅在你电脑上无效可点“无用”。举报可撤销；三个不同已验证邮箱举报后暂时隐藏，由管理员审核。")))
            }
            Mode::Remove => div()
                .v_flex()
                .gap_3()
                .when_some(self.preview.as_ref(), |d, card| {
                    d.child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(card.content.display_title()),
                    )
                })
                .child(self.t("删除后，其他玩家将无法再查看或评价这个分享。此操作不可撤销。"))
                .child(
                    self.button("recipe-confirm-remove", "确认删除分享", cx)
                        .danger()
                        .on_click(cx.listener(|this, _, _, cx| {
                            if let (Some(client), Some(card)) =
                                (this.client.clone(), this.preview.clone())
                            {
                                this.job(move || {
                                    client.remove(&card)?;
                                    Ok(Update::Removed)
                                });
                            }
                            cx.notify();
                        })),
                ),
        };
        let scroll_id = format!(
            "recommendations-scroll-{}-{}",
            self.mode as usize,
            if matches!(self.mode, Mode::Browse | Mode::Login) {
                "list"
            } else {
                self.preview
                    .as_ref()
                    .map(|c| c.id.as_str())
                    .unwrap_or("form")
            }
        );
        let scroll = self
            .scroll_handles
            .entry(scroll_id.clone())
            .or_default()
            .clone();
        let download_overlay = self
            .owner
            .update(cx, |owner, cx| owner.download_overlay(window, cx))
            .unwrap_or_else(|_| div().into_any_element());
        let content = div()
            .v_flex()
            .gap_2()
            .min_w_0()
            .text_sm()
            .when(self.mode != Mode::Preview, |d| d.child(top.flex_shrink_0()))
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .child(
                        div()
                            .id(SharedString::from(scroll_id))
                            .v_flex()
                            .min_h_0()
                            .min_w_0()
                            .h_full()
                            .track_scroll(&scroll)
                            .overflow_y_scroll()
                            .child(body.flex_shrink_0().min_w_0().pr_3()),
                    )
                    .child(
                        div()
                            .absolute()
                            .inset_0()
                            .child(Scrollbar::vertical(&scroll).mode(ScrollbarMode::Always)),
                    ),
            )
            .children(install.notice)
            .child(
                div()
                    .h_flex()
                    .flex_wrap()
                    .flex_shrink_0()
                    .pt_2()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .justify_between()
                    .gap_2()
                    .child(
                        self.button(
                            "recipe-back",
                            if self.mode == Mode::Browse {
                                "更换游戏关联"
                            } else {
                                "返回推荐"
                            },
                            cx,
                        )
                        .ghost()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.mode = if this.mode == Mode::Browse || this.identity.is_none() {
                                this.find_candidates(true, cx);
                                Mode::Identity
                            } else {
                                Mode::Browse
                            };
                            this.message.clear();
                            cx.notify();
                        })),
                    )
                    .child(
                        div()
                            .v_flex()
                            .min_w_0()
                            .max_w_full()
                            .items_end()
                            .gap_1()
                            .when_some(install.selection, |d, selected| {
                                d.child(
                                    div()
                                        .text_sm()
                                        .text_right()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(selected),
                                )
                            })
                            .child(
                                div()
                                    .h_flex()
                                    .gap_2()
                                    .flex_wrap()
                                    .children(install.menu)
                                    .children(primary),
                            ),
                    ),
            );
        div()
            .v_flex()
            .size_full()
            .min_w_0()
            .min_h_0()
            .font_family("Microsoft YaHei UI")
            .text_sm()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(
                TitleBar::new().child(
                    div()
                        .h_flex()
                        .gap_2()
                        .pl_3()
                        .min_w_0()
                        .child(match game_image {
                            Some(icon) => img(icon)
                                .size_4()
                                .object_fit(ObjectFit::Contain)
                                .into_any_element(),
                            None => Icon::new(IconName::LayoutDashboard)
                                .size_4()
                                .into_any_element(),
                        })
                        .child(div().min_w_0().text_sm().child(game_title)),
                ),
            )
            .child(content.p_3().flex_1().min_h_0())
            .relative()
            .child(download_overlay)
    }
}
#[derive(Default)]
struct InstallFooter {
    menu: Option<AnyElement>,
    selection: Option<String>,
    notice: Option<AnyElement>,
}
struct LoginForm(WeakEntity<Recommendations>);

impl Render for LoginForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.0
            .update(cx, |view, cx| view.login_form(cx))
            .unwrap_or_else(|_| div())
    }
}

impl Recommendations {
    fn page_header(&self, cx: &Context<Self>) -> Div {
        let mut top = div().v_flex().gap_2();
        if self.mode == Mode::Identity {
            top = top.child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(self.t("更换游戏关联")),
            );
        }
        if matches!(self.mode, Mode::Browse | Mode::Login) {
            let game_title = game_names::display(&self.game, &self.names, true);
            if let Some(group) = self.browse_identity().filter(|id| id.name != game_title) {
                top = top.child(div().text_sm().child(group.name));
            }
            if let Some(active) = self.identity.as_ref()
                && rec::GameLink::saved(&self.game, active).is_some()
            {
                let historical = self.show_history;
                top = top.child(
                    div().h_flex().gap_2().child(
                        self.button(
                            "recipe-switch-history",
                            if historical {
                                "返回平台条目"
                            } else {
                                "查看历史条目的分享"
                            },
                            cx,
                        )
                        .disabled(self.busy || self.operation_busy)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.use_group(!historical);
                            cx.notify();
                        })),
                    ),
                );
            } else if let Some(pair) = self.link_suggestion.clone()
                && self.game.extra.get("recommendation_link_ignored")
                    != Some(&serde_json::json!(pair))
            {
                let dismissed = serde_json::json!(pair);
                top = top.child(
                    div()
                        .v_flex()
                        .gap_1()
                        .child(
                            div().text_sm().whitespace_normal().child(
                                self.t("这两个条目是同一款游戏吗？{0} / {1}")
                                    .replace("{0}", &pair.platform.name)
                                    .replace("{1}", &pair.historical.name),
                            ),
                        )
                        .child(
                            div()
                                .h_flex()
                                .gap_2()
                                .flex_wrap()
                                .child(
                                    self.button("recipe-confirm-link", "关联并保留历史分享", cx)
                                        .disabled(self.busy || self.operation_busy)
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.set_game_choice(
                                                Some(pair.platform.clone()),
                                                Some(pair.clone()),
                                                cx,
                                            );
                                        })),
                                )
                                .child(
                                    self.button("recipe-ignore-link", "忽略此关联", cx)
                                        .ghost()
                                        .disabled(self.busy || self.operation_busy)
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.save_game_choice(cx, |game| {
                                                game.extra.insert(
                                                    "recommendation_link_ignored".into(),
                                                    dismissed.clone(),
                                                );
                                            });
                                            cx.notify();
                                        })),
                                ),
                        ),
                );
            }
            top = top.child(
                div()
                    .h_flex()
                    .gap_2()
                    .flex_wrap()
                    .child(
                        self.button("recipe-refresh", "刷新", cx)
                            .icon(IconName::RefreshCw)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.refresh(false);
                                cx.notify();
                            })),
                    )
                    .child(
                        self.button("recipe-share", "分享我的方案", cx)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_share(window, cx);
                                cx.notify();
                            })),
                    )
                    .child(
                        self.button(
                            "recipe-mine",
                            if self.mine {
                                "全部推荐"
                            } else {
                                "我的分享"
                            },
                            cx,
                        )
                        .on_click(cx.listener(|this, _, window, cx| {
                            if !this.logged_in() {
                                this.login(false, window, cx);
                            } else {
                                this.mine = !this.mine;
                                this.reported = false;
                                this.clear_page();
                                this.refresh(false);
                            }
                            cx.notify();
                        })),
                    )
                    .child(
                        self.button(
                            "recipe-my-reports",
                            if self.reported {
                                "全部推荐"
                            } else {
                                "我的举报"
                            },
                            cx,
                        )
                        .on_click(cx.listener(|this, _, window, cx| {
                            if !this.logged_in() {
                                this.login(false, window, cx);
                            } else {
                                this.reported = !this.reported;
                                this.mine = false;
                                this.clear_page();
                                this.refresh(false);
                            }
                            cx.notify();
                        })),
                    )
                    .child(
                        self.button(
                            "recipe-account",
                            if self.logged_in() {
                                "退出登录"
                            } else {
                                "邮箱登录"
                            },
                            cx,
                        )
                        .on_click(cx.listener(|this, _, window, cx| {
                            if let Some(mut client) = this.client.clone().filter(|c| c.logged_in())
                            {
                                this.job(move || {
                                    client.logout()?;
                                    Ok(Update::Logout(client))
                                });
                            } else {
                                this.login(false, window, cx);
                            }
                            cx.notify();
                        })),
                    ),
            );
            if self.loaded || self.busy {
                let summary = if self.loaded {
                    self.t("{0} 个方案，按有用数排序")
                        .replace("{0}", &self.total.to_string())
                } else {
                    self.t("正在加载推荐…")
                };
                top = top.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(summary),
                );
            }
        }
        if self.mode == Mode::Browse && self.client.as_ref().is_some_and(rec::Client::login_expired)
        {
            top = top.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(self.t("登录已失效，仍可浏览最新推荐；分享或评价时请重新登录。")),
            );
        }
        top
    }

    fn identity_page(&self, cx: &Context<Self>) -> Div {
        let ring = crate::ui_focus::visible(cx);
        let mut body = div().v_flex().gap_2().child(
            div()
                .h_flex()
                .gap_2()
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(Input::new(&self.query).small().focus_ring(ring)),
                )
                .child(
                    self.button("recipe-find-game", "查找已有游戏", cx)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.find_candidates(false, cx);
                            cx.notify();
                        })),
                ),
        );
        for candidate in self.candidates.iter().take(20) {
            let id = candidate.identity.clone();
            let mut metadata = Vec::new();
            if !candidate.original.is_empty() && candidate.original != id.name {
                metadata.push(candidate.original.clone());
            }
            metadata.extend(candidate.platforms.iter().map(|platform| self.t(platform)));
            if candidate.shared_count > 0 {
                metadata.push(
                    self.t("{0} 个公开分享")
                        .replace("{0}", &candidate.shared_count.to_string()),
                );
            }
            if metadata.is_empty() {
                metadata.push(self.t("玩家补充的游戏名，请核对后选择"));
            }
            body = body.child(
                div()
                    .h_flex()
                    .gap_2()
                    .px_2()
                    .py_1p5()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        div()
                            .v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_0p5()
                            .child(div().font_weight(FontWeight::MEDIUM).child(id.name.clone()))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(metadata.join(" / ")),
                            ),
                    )
                    .child(
                        self.button(
                            SharedString::from(format!("recipe-game-{}", id.id)),
                            "选择这款游戏",
                            cx,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| this.bind(id.clone(), cx))),
                    ),
            );
        }
        body.when(self.game.extra.contains_key("recommendation_game"), |d| {
            d.child(
                self.button("recipe-reset-game", "恢复自动识别", cx)
                    .on_click(cx.listener(|this, _, _, cx| this.reset_identity(cx))),
            )
        })
        .child(
            div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(self.t("没有找到？请填写完整游戏名，再创建独立条目。以后可更换关联。")),
        )
        .child(
            self.button("recipe-custom-game", "使用此名称创建游戏条目", cx)
                .on_click(cx.listener(|this, _, _, cx| {
                    match rec::GameIdentity::custom(&this.query.read(cx).value()) {
                        Ok(id) => this.bind(id, cx),
                        Err(e) => this.message = this.t(&e.to_string()),
                    }
                    cx.notify();
                })),
        )
    }

    fn login_form(&self, cx: &Context<Self>) -> Div {
        let ring = crate::ui_focus::visible(cx);
        let code = self.code.read(cx).value();
        let address = rec::complete_email(&self.email.read(cx).value(), &self.email_domain);
        let matches_address = self
            .challenge
            .as_ref()
            .is_some_and(|(_, email)| email.eq_ignore_ascii_case(&address));
        let login = self
            .button("recipe-login", "登录", cx)
            .primary()
            .disabled(
                self.busy
                    || !matches_address
                    || code.len() != 6
                    || !code.bytes().all(|b| b.is_ascii_digit()),
            )
            .on_click(cx.listener(|this, _, _, cx| {
                if let (Some(mut client), Some((challenge, _))) =
                    (this.client.clone(), this.challenge.clone())
                {
                    let code = this.code.read(cx).value().to_string();
                    this.job(move || {
                        client.verify(&challenge.challenge, &code)?;
                        Ok(Update::Login(client))
                    });
                }
                cx.notify();
            }));
        let send = self
            .button("recipe-send-code", "发送验证码", cx)
            .disabled(self.busy || self.cooldown || self.email.read(cx).value().trim().is_empty())
            .on_click(cx.listener(|this, _, window, cx| {
                if let Some(client) = this.client.clone() {
                    let email =
                        rec::complete_email(&this.email.read(cx).value(), &this.email_domain);
                    this.email
                        .update(cx, |s, cx| s.set_value(email.clone(), window, cx));
                    this.job(move || Ok(Update::Code(client.send_code(&email)?, email)));
                }
                cx.notify();
            }));
        div()
            .v_flex()
            .w_full()
            .gap_3()
            .text_sm()
            .child(
                div()
                    .v_flex()
                    .gap_1()
                    .child(self.t("邮箱账号或完整邮箱"))
                    .child(
                        div()
                            .h_flex()
                            .gap_2()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(Input::new(&self.email).small().focus_ring(ring)),
                            )
                            .child(self.email_suffix(cx)),
                    ),
            )
            .child(
                div().v_flex().gap_1().child(self.t("验证码")).child(
                    div()
                        .h_flex()
                        .gap_2()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(Input::new(&self.code).small().focus_ring(ring)),
                        )
                        .child(send),
                ),
            )
            .child(div().h_flex().justify_end().child(login))
    }

    fn share_page(&self, window: &Window, primary: &mut Option<Button>, cx: &Context<Self>) -> Div {
        let ring = crate::ui_focus::visible(cx);
        let mut body = div().v_flex().gap_4().min_w_0();
        if self.editing.is_some() {
            body = body
                .child(
                    div()
                        .text_sm()
                        .child(self.t("编辑原分享，更新后保留原有评价，不新增重复方案。")),
                )
                .child(
                    self.button("recipe-reload-installed", "读取当前安装方案", cx)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.read_installed(true);
                            cx.notify();
                        })),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(
                            self.t(
                                "如已更换补丁、DLL 或应用新参数，可重新读取；名称和备注会保留。",
                            ),
                        ),
                );
        }
        if let Some(draft) = &self.draft {
            let field = |label: &str, control: AnyElement| {
                vec![
                    div()
                        .text_color(cx.theme().muted_foreground)
                        .child(self.t(label))
                        .into_any_element(),
                    control,
                ]
            };
            let notes = div()
                .v_flex()
                .w_full()
                .gap_1()
                .child(
                    div()
                        .p_2()
                        .rounded_md()
                        .border_1()
                        .border_color(
                            if ring && self.notes.read(cx).focus_handle(cx).is_focused(window) {
                                cx.theme().ring
                            } else {
                                cx.theme().border
                            },
                        )
                        .child(Textarea::new(&self.notes).appearance(false).bordered(false)),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(
                            if self.notes.read(cx).value().chars().count() > rec::NOTES_LIMIT {
                                cx.theme().danger
                            } else {
                                cx.theme().muted_foreground
                            },
                        )
                        .child(format!(
                            "{} / {}",
                            self.notes.read(cx).value().chars().count(),
                            rec::NOTES_LIMIT
                        )),
                )
                .into_any_element();
            let fields = vec![
                field(
                    "方案名称",
                    Input::new(&self.title).focus_ring(ring).into_any_element(),
                ),
                field(
                    "游戏平台",
                    self.picker("recipe-platform", &draft.platform, rec::PLATFORMS, cx),
                ),
                field(
                    "游戏版本",
                    Input::new(&self.edition)
                        .focus_ring(ring)
                        .into_any_element(),
                ),
                field(
                    "显卡型号（实际型号）",
                    Input::new(&self.gpu_input)
                        .focus_ring(ring)
                        .into_any_element(),
                ),
                field(
                    "图形接口",
                    self.picker(
                        "recipe-renderer",
                        &draft.renderer,
                        &[
                            ("unknown", "图形接口：未说明"),
                            ("dx12", "DX12"),
                            ("vulkan", "Vulkan"),
                            ("dx11", "DX11"),
                        ],
                        cx,
                    ),
                ),
                field(
                    "显卡改名",
                    self.picker(
                        "recipe-alias",
                        &draft.gpu_alias,
                        &[
                            ("unknown", "显卡型号修改：未说明"),
                            ("unchanged", "不需要修改显卡型号"),
                            ("rtx4060", "修改为 RTX 4060"),
                            ("rtx5090", "修改为 RTX 5090"),
                        ],
                        cx,
                    ),
                ),
                field("玩家备注", notes),
            ];
            body = body
                .child(self.recipe_summary(draft, cx))
                .child(self.parameter_comparison(draft, cx))
                .child(self.runtime_inventory(draft, None, true, cx))
                .child(self.recipe_table("recipe-share-fields", &["项目", "分享内容"], fields, cx))
                .child(div().text_xs().text_color(cx.theme().muted_foreground)
                    .child(self.t("以上内容将公开。仅分享你在这款游戏中测试可用的方案；不上传路径、日志、邮箱或原始配置文件。")));
            *primary = Some(
                self.button(
                    "recipe-publish",
                    if self.editing.is_some() {
                        "更新分享"
                    } else {
                        "确认分享"
                    },
                    cx,
                )
                .primary()
                .disabled(
                    self.busy || self.notes.read(cx).value().chars().count() > rec::NOTES_LIMIT,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    if let (Some(client), Some(mut recipe)) =
                        (this.client.clone(), this.draft.clone())
                    {
                        let title = this.title.read(cx).value().trim().to_owned();
                        let update_suggestion =
                            this.editing.is_none() && title == recipe.suggested_title();
                        recipe.notes = this.notes.read(cx).value().trim().to_owned();
                        recipe.edition = this.edition.read(cx).value().trim().to_owned();
                        recipe.gpu_model = this.gpu_input.read(cx).value().trim().to_owned();
                        recipe.title = if update_suggestion {
                            recipe.suggested_title()
                        } else if let Some(old) = &this.editing
                            && title == old.content.display_title()
                        {
                            old.content.title.clone()
                        } else {
                            title
                        };
                        let catalog = this.catalog.clone();
                        let submission = this.submission.clone();
                        let editing = this.editing.clone();
                        this.job(move || {
                            let card = if let Some(original) = editing {
                                client.update_share(&original, &recipe, &catalog)?
                            } else {
                                client.share(&submission, &recipe, &catalog)?
                            };
                            Ok(Update::Card(card))
                        });
                    }
                    cx.notify();
                })),
            );
        }
        body
    }

    fn preview_page(&self, footer: &mut InstallFooter, cx: &Context<Self>) -> Div {
        let mut body = div().v_flex().gap_2();
        if let Some(card) = &self.preview {
            let available = card.content.available_version(&self.catalog);
            let runtime_error = card
                .content
                .check_runtime_selection()
                .err()
                .map(|e| e.to_string());
            let blocked_here = self.runtime_plan.as_ref().is_some_and(|p| {
                p.values()
                    .any(|step| *step == recommendation_runtimes::RuntimeStep::Unavailable)
            });
            let changed = available
                .as_ref()
                .is_ok_and(|v| *v != card.content.patch_version);
            body=body.child(
                div().h_flex().items_start().min_w_0().gap_2().pb_1()
                    .child(Self::avatar(cx))
                    .child(div().v_flex().flex_1().min_w_0().gap_1()
                        .child(div().font_weight(FontWeight::SEMIBOLD).whitespace_normal().child(card.content.display_title()))
                        .child(div().text_xs().text_color(cx.theme().muted_foreground).child(self.card_date(card))))
                    .child(div().h_flex().gap_1().flex_shrink_0()
                        .child(self.vote_button(card, true, cx))
                        .child(self.vote_button(card, false, cx))))
                .when(!card.content.platform.is_empty() && card.content.platform!="unknown" && card.content.platform!=rec::platform(&self.game),|d|d.child(div().text_sm().text_color(cx.theme().warning).child(self.t("此方案来自其他平台，请核对游戏版本和图形接口后使用。"))))
                .when(changed,|d|d.child(div().text_sm().text_color(cx.theme().warning).child(self.t("玩家测试版本 {0}；将使用同项目当前版本 {1}。原点赞针对玩家分享，不代表新版已通过测试。").replace("{0}",&card.content.patch_version).replace("{1}",available.as_deref().unwrap_or_default()))))
                .when_some(available.as_ref().err(),|d,e|d.child(div().text_sm().text_color(cx.theme().warning).child(self.t(&e.to_string()))))
                .child(self.preview_sections(&card.content,available.as_deref().ok(),runtime_error.is_none(),cx));
            let parameter_error = available
                .as_ref()
                .map_err(|e| e.to_string())
                .and_then(|v| {
                    card.content
                        .presets
                        .as_ref()
                        .ok_or_else(|| self.t("此分享未附带参数，可仅安装方案。"))?
                        .for_version(&card.content.scheme, &card.content.patch_version, v)
                        .map_err(|e| e.to_string())
                })
                .err();
            if let Some(error) = &parameter_error
                && card.content.presets.is_some()
            {
                body = body.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().warning)
                        .child(self.t(error)),
                );
            }
            if changed
                && parameter_error.is_none()
                && card
                    .content
                    .presets
                    .as_ref()
                    .is_some_and(|s| s.profile == rtx_fg_manager::upstream040::PROFILE)
                && available.as_ref().is_ok_and(|v| {
                    rtx_fg_manager::upstream040::profile_for_version(Some(v))
                        == rtx_fg_manager::upstream040::PROFILE_041
                })
            {
                body = body.child(div().text_xs().text_color(cx.theme().warning)
                    .child(self.t("将按 0.4.1 转换旧参数：无 HUD 开启改为自动，Auto 预设改为 A；已取消的作用档位不再写入。")));
            }
            if changed
                && parameter_error.is_none()
                && card.content.presets.as_ref().is_some_and(|s| {
                    s.profile == rtx_fg_manager::encore::PROFILE
                        && (s.values.contains_key("vsyncOff")
                            || s.values.contains_key("smoothMotionTuringVariant"))
                })
                && available.as_ref().is_ok_and(|v| {
                    rtx_fg_manager::encore::field_available(
                        rtx_fg_manager::encore::field("vsync").expect("V-Sync metadata"),
                        Some(v),
                    )
                })
            {
                body =
                    body.child(div().text_xs().text_color(cx.theme().warning).child(self.t(
                        "旧版垂直同步设置将转换；RTX 20 平滑运动从 Encore 1.5.1 起固定使用 V2。",
                    )));
            }
            let runtime_notice = runtime_error
                .as_deref()
                .or(blocked_here.then_some("当前游戏的部分运行库无法安装。"));
            if let Some(error) = runtime_notice
                && self.selected_parts[2]
            {
                footer.notice = Some(
                    div()
                        .h_flex()
                        .flex_wrap()
                        .gap_2()
                        .flex_shrink_0()
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().warning)
                                .child(self.t(error)),
                        )
                        .child(
                            self.button("recipe-skip-runtimes", "取消勾选运行库", cx)
                                .disabled(self.busy || self.operation_busy)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.selected_parts[2] = false;
                                    this.message.clear();
                                    cx.notify();
                                })),
                        )
                        .when(blocked_here, |panel| {
                            panel.child(
                                self.button("recipe-recheck-local", "重新检查", cx)
                                    .disabled(self.operation_busy)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.inspect_runtime_plan(cx);
                                        cx.notify();
                                    })),
                            )
                        })
                        .into_any_element(),
                );
            }
            let has_parameters = card.content.presets.is_some();
            let has_runtimes = card
                .content
                .runtimes
                .as_ref()
                .is_some_and(|r| !r.components.is_empty())
                || card.content.neural_runtime == "310.9.16";
            let parts = self.selected_parts;
            let action = RecipeAction::Selected {
                patch: parts[0],
                parameters: parts[1] && has_parameters,
                runtimes: parts[2] && has_runtimes,
            };
            let choices = [
                (0, self.t("补丁方案"), true),
                (1, self.t("参数"), has_parameters),
                (2, self.t("运行库"), has_runtimes),
            ];
            let selected = choices
                .iter()
                .filter(|(index, _, visible)| *visible && parts[*index])
                .map(|(_, label, _)| label.as_str())
                .collect::<Vec<_>>()
                .join(" / ");
            footer.selection = Some(if action.is_empty() {
                self.t("未选择安装内容")
            } else {
                self.t("已选择：{0}").replace("{0}", &selected)
            });
            let view = cx.entity().downgrade();
            footer.menu = Some(
                DropdownButton::new("recipe-install-options")
                    .button(
                        self.button("recipe-apply", "安装并应用", cx)
                            .disabled(
                                self.busy
                                    || self.operation_busy
                                    || action.is_empty()
                                    || available.is_err()
                                    || (action.parameters() && parameter_error.is_some())
                                    || (action.runtimes() && runtime_notice.is_some()),
                            )
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.apply_selection(action, window, cx)
                            })),
                    )
                    .primary()
                    .small()
                    .disabled(self.busy || self.operation_busy)
                    .menu_accessibility_label(self.t("选择安装内容"))
                    .dropdown_menu_with_anchor(Anchor::BottomRight, move |mut menu, _, _| {
                        for (index, label, visible) in &choices {
                            if !visible {
                                continue;
                            }
                            let index = *index;
                            let view = view.clone();
                            menu = menu.item(
                                PopupMenuItem::new(label.clone())
                                    .checked(parts[index])
                                    .on_click(move |_, _, cx| {
                                        let _ = view.update(cx, |this, cx| {
                                            if this.busy || this.operation_busy {
                                                return;
                                            }
                                            this.selected_parts[index] =
                                                !this.selected_parts[index];
                                            cx.notify();
                                        });
                                    }),
                            );
                        }
                        menu
                    })
                    .into_any_element(),
            );
        }
        body
    }
}

impl Manager {
    pub(super) fn recommendations(
        &mut self,
        exe: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.dialog_active {
            return;
        }
        if let Some(handle) = self.recommendation_windows.get(&exe).copied() {
            cx.defer(move |cx| {
                let _ = handle.update(cx, |_, w, _| w.activate_window());
            });
            return;
        }
        let Some(game) = self.c.games.iter().find(|g| g.exe == exe).cloned() else {
            return;
        };
        let work = window
            .display(cx)
            .map(|d| d.visible_bounds())
            .unwrap_or(window.bounds());
        let options = WindowOptions {
            titlebar: Some(TitlebarOptions {
                title: Some(self.t("其他玩家分享的方案")),
                appears_transparent: true,
                ..Default::default()
            }),
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered_at(
                work.center(),
                size(
                    (window.rem_size() * 53.).min(work.size.width),
                    (window.rem_size() * 36.).min(work.size.height),
                ),
            ))),
            window_min_size: Some(size(
                px(500.).min(work.size.width),
                px(360.).min(work.size.height),
            )),
            ..Default::default()
        };
        let owner = cx.entity();
        // Release Manager before native creation can synchronously draw a view.
        self.utility_opening = true;
        cx.defer(move |cx| {
            let result_owner = owner.clone();
            let result = cx.open_window(options, move |window, cx| {
                window.on_window_should_close(cx, |window, cx| !window.has_active_dialog(cx));
                let view = owner.update(cx, |manager, cx| {
                    cx.new(|cx| {
                        Recommendations::new(owner.downgrade(), game, &manager.c, window, cx)
                    })
                });
                cx.new(|cx| Root::new(view, window, cx))
            });
            result_owner.update(cx, |owner, cx| {
                owner.utility_opening = false;
                match result {
                    Ok(handle) => {
                        owner.recommendation_windows.insert(exe, handle.into());
                    }
                    Err(error) => owner.c.warning = Some(format!("{error:#}")),
                }
                cx.notify();
            });
        });
    }
}
