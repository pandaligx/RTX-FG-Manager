use super::*;
#[path = "ui_community_metadata.rs"]
mod metadata;
use anyhow::Context as _;
use rtx_fg_manager::{
    community::{self, Content, Hardware, Profile},
    community_game, community_identity,
    community_match::{self, Selection},
    config_store::{self, Envelope, History, Preview},
    presets,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, atomic::AtomicBool, mpsc},
};

enum Update {
    AutoProfiles(String, std::result::Result<community::BrowseResult, String>),
    Interactions(String, u32, community::Interactions),
    Games(Vec<community::Game>),
    BoundGame(String),
    Local(
        Option<String>,
        Vec<History>,
        Vec<rtx_fg_manager::operation_history::Entry>,
        Option<community_identity::PublicIdentity>,
        Vec<Hardware>,
        Option<String>,
        Option<community::BrowseResult>,
        Vec<community::Game>,
    ),
    Named(Vec<(String, Envelope)>),
    Profiles(String, community::BrowseResult),
    Mine(
        String,
        community::BrowseResult,
        community_identity::PublicIdentity,
    ),
    Setup(Profile, String),
    CommunityPreview(Profile, Preview, String),
    Identity(community_identity::PublicIdentity, String),
    Preview(String, Preview),
    Publish(String, Content, presets::Context),
    Published(community_identity::PublicIdentity),
    Restore(community_identity::Identity, Option<String>),
    Notice(String),
    Changed(String),
}
pub struct CommunityView {
    tr: rtx_fg_manager::i18n::Translator,
    owner: WeakEntity<Manager>,
    target: core::DeploymentTarget,
    targets: Vec<core::DeploymentTarget>,
    context: presets::Context,
    patch_version: String,
    hardware: Hardware,
    hardware_options: Vec<Hardware>,
    metadata: metadata::MetadataFields,
    metadata_open: bool,
    advanced_open: bool,
    share_open: bool,
    mine_open: bool,
    refresh_inflight: bool,
    next_refresh: Instant,
    last_tab: u8,
    game: Entity<InputState>,
    game_search: Entity<InputState>,
    games: Vec<community::Game>,
    name: Entity<InputState>,
    notes: Entity<InputState>,
    password: Entity<InputState>,
    report_reason: Entity<InputState>,
    tx: mpsc::Sender<std::result::Result<Update, String>>,
    rx: mpsc::Receiver<std::result::Result<Update, String>>,
    busy: bool,
    message: String,
    history: Vec<History>,
    operations: Vec<rtx_fg_manager::operation_history::Entry>,
    named: Vec<(String, Envelope)>,
    profiles: Vec<Profile>,
    interactions: BTreeMap<(String, u32), community::Interactions>,
    selection: Selection,
    identity: Option<community_identity::PublicIdentity>,
    preview: Option<(String, Preview)>,
    preview_source: Option<Profile>,
    setup: Option<(Profile, String)>,
    observed_game: String,
    selected_keys: BTreeSet<String>,
    publish: Option<(String, Content)>,
    publish_details: bool,
    restore: Option<(community_identity::Identity, Option<String>)>,
    edit_target: Option<(String, u32)>,
    edit_profile: Option<Profile>,
    delete_target: Option<Profile>,
    cancel: Arc<AtomicBool>,
    pub tab: u8,
    cached: bool,
}
impl Drop for CommunityView {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}
pub struct GameDetails {
    pub initial_tab: u8,
    pub targets: Vec<core::DeploymentTarget>,
    pub context: presets::Context,
    pub patch_version: String,
    pub hardware: Hardware,
    pub language: String,
}
impl CommunityView {
    pub fn new(
        owner: WeakEntity<Manager>,
        game: GameDetails,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        // Construction happens inside Manager::update. Read its immutable
        // context before creating this child, never reborrow the parent here.
        let GameDetails {
            initial_tab,
            targets,
            context,
            patch_version,
            hardware,
            language,
        } = game;
        let tr = rtx_fg_manager::i18n::Translator::new(&language);
        let (tx, rx) = mpsc::channel();
        let target = targets
            .first()
            .expect("game has a deployment executable")
            .clone();
        let mut view = Self {
            owner,
            target,
            targets,
            context,
            patch_version,
            hardware,
            hardware_options: Vec::new(),
            metadata: metadata::MetadataFields::new(&tr, window, cx),
            metadata_open: false,
            advanced_open: false,
            share_open: false,
            mine_open: false,
            refresh_inflight: false,
            next_refresh: Instant::now(),
            last_tab: initial_tab,
            game: cx.new(|cx| {
                InputState::new(window, cx).placeholder(tr.t("Steam App ID 或已登记的游戏 ID"))
            }),
            game_search: cx.new(|cx| {
                InputState::new(window, cx).placeholder(tr.t("搜索游戏，或填写未收录的游戏名称"))
            }),
            games: Vec::new(),
            name: cx.new(|cx| InputState::new(window, cx).placeholder(tr.t("配置名称"))),
            notes: cx.new(|cx| {
                InputState::new(window, cx).placeholder(tr.t("备注（不填写路径或个人信息）"))
            }),
            password: cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(tr.t("身份备份密码，至少 12 字符"))
                    .masked(true)
            }),
            report_reason: cx.new(|cx| InputState::new(window, cx).placeholder(tr.t("举报原因"))),
            tx,
            rx,
            busy: false,
            message: String::new(),
            history: Vec::new(),
            operations: Vec::new(),
            named: Vec::new(),
            profiles: Vec::new(),
            interactions: BTreeMap::new(),
            selection: Selection::default(),
            identity: None,
            preview: None,
            preview_source: None,
            setup: None,
            observed_game: String::new(),
            selected_keys: BTreeSet::new(),
            publish: None,
            publish_details: false,
            restore: None,
            edit_target: None,
            edit_profile: None,
            delete_target: None,
            cancel: Arc::new(AtomicBool::new(false)),
            tab: initial_tab,
            cached: false,
            tr,
        };
        view.metadata.set_hardware(&view.hardware, window, cx);
        view.local();
        cx.spawn_in(window, async move |this, cx| {
            loop {
                smol::Timer::after(Duration::from_millis(80)).await;
                if this
                    .update_in(cx, |this, window, cx| this.poll(window, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        view
    }
    fn job(&mut self, f: impl FnOnce() -> anyhow::Result<Update> + Send + 'static) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.message = "正在处理…".into();
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f))
                .map_err(|_| "后台操作异常终止".to_owned())
                .and_then(|r| r.map_err(|e| format!("{e:#}")));
            let _ = tx.send(result);
        });
    }
    fn t(&self, text: &str) -> SharedString {
        self.tr.t(text).into()
    }
    fn local(&mut self) {
        let target = self.target.clone();
        self.job(move || {
            let game = community_game::identify(&target)?;
            let cached = game
                .as_ref()
                .and_then(|id| community::cached_browse(id, "", "").ok());
            Ok(Update::Local(
                game,
                config_store::history(&target)?,
                rtx_fg_manager::operation_history::read(&target)?,
                community_identity::current()?,
                rtx_fg_manager::community_hardware::detect().unwrap_or_default(),
                core::record(&target.directory)?
                    .and_then(|r| r.upstream_version.or(r.payload_version)),
                cached,
                community::cached_games().unwrap_or_default(),
            ))
        });
    }
    fn game_id(&self, cx: &App) -> String {
        let text = self.game.read(cx).value().trim().to_owned();
        if text.bytes().all(|b| b.is_ascii_digit()) && !text.is_empty() {
            format!("steam:{text}")
        } else {
            text
        }
    }
    fn poll(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let game = self.game_id(cx);
        if game != self.observed_game {
            self.observed_game = game;
            self.profiles.clear();
            self.interactions.clear();
            self.preview = None;
            self.preview_source = None;
            self.publish = None;
            self.setup = None;
            self.edit_target = None;
            self.edit_profile = None;
            self.delete_target = None;
            self.mine_open = false;
            self.next_refresh = Instant::now();
        }
        while let Ok(result) = self.rx.try_recv() {
            if let Ok(Update::AutoProfiles(game, result)) = result {
                self.refresh_inflight = false;
                if game == self.game_id(cx) && !self.mine_open {
                    match result {
                        Ok(result) => {
                            self.cached = result.cached;
                            self.profiles = result.index.profiles;
                            if !self.busy && !self.share_open && self.preview.is_none() {
                                self.message = result
                                    .message
                                    .unwrap_or_else(|| "玩家推荐方案已刷新".into());
                            }
                        }
                        Err(_) => {
                            if !self.busy && !self.share_open {
                                self.cached = true;
                                self.message =
                                    "社区暂不可用，稍后自动重试；本地配置仍可使用".into();
                            }
                        }
                    }
                    cx.notify();
                }
                continue;
            }
            self.busy = false;
            match result {
                Ok(Update::AutoProfiles(..)) => unreachable!(),
                Ok(Update::Interactions(id, revision, state)) => {
                    self.interactions.insert((id, revision), state);
                    self.next_refresh = Instant::now();
                    self.message = "反馈已更新".into();
                }
                Err(error) => self.message = error,
                Ok(Update::BoundGame(game)) => {
                    self.game
                        .update(cx, |s, cx| s.set_value(game.clone(), window, cx));
                    self.profiles.clear();
                    self.interactions.clear();
                    self.preview = None;
                    self.publish = None;
                    self.edit_target = None;
                    self.edit_profile = None;
                    self.next_refresh = Instant::now();
                    self.message = "已记住此游戏；分享时自动收录到游戏目录".into();
                }
                Ok(Update::Games(games)) => {
                    self.games = games;
                    self.message =
                        "选择对应游戏后保存本机映射；列表允许为空，Steam 游戏可直接填写 App ID"
                            .into();
                }
                Ok(Update::Local(
                    game,
                    history,
                    operations,
                    identity,
                    hardware,
                    installed_version,
                    cached,
                    games,
                )) => {
                    if let Some(version) = installed_version {
                        self.patch_version = version;
                    }
                    if let Some(game) = game {
                        self.observed_game = game.clone();
                        self.game.update(cx, |s, cx| s.set_value(game, window, cx));
                    }
                    self.history = history;
                    self.operations = operations;
                    self.identity = identity;
                    if let Some(h) = hardware.first() {
                        self.hardware = h.clone();
                        self.metadata.set_hardware(h, window, cx);
                    }
                    self.hardware_options = hardware;
                    self.games = games;
                    self.message = "已读取本地配置记录".into();
                    if let Some(cached) = cached {
                        self.profiles = cached.index.profiles;
                        self.cached = true;
                        self.message = cached.message.unwrap_or_default();
                    }
                }
                Ok(Update::Named(named)) => {
                    self.named = named;
                    self.message = format!("{} 份本地命名配置", self.named.len());
                }
                Ok(Update::Profiles(game, result)) => {
                    if game != self.game_id(cx) {
                        continue;
                    }
                    self.mine_open = false;
                    self.interactions.clear();
                    self.cached = result.cached;
                    self.profiles = result.index.profiles;
                    self.message = result
                        .message
                        .unwrap_or_else(|| format!("{} 份玩家推荐方案", self.profiles.len()));
                    self.selection.visible = 12;
                }
                Ok(Update::Mine(game, result, identity)) => {
                    if game != self.game_id(cx) {
                        continue;
                    }
                    self.mine_open = true;
                    self.interactions.clear();
                    self.identity = Some(identity);
                    self.cached = false;
                    self.profiles = result.index.profiles;
                    self.message = "正在显示此游戏的个人配置（含待审核项）".into();
                    self.selection.visible = 12;
                }
                Ok(Update::Identity(identity, message)) => {
                    self.interactions.clear();
                    self.identity = Some(identity);
                    self.message = message;
                }
                Ok(Update::Preview(name, preview)) => {
                    self.preview_source = None;
                    self.selected_keys =
                        preview.differences.iter().map(|d| d.key.clone()).collect();
                    self.preview = Some((name, preview));
                    self.message = "请逐项检查差异，勾选需要导入的设置".into();
                }
                Ok(Update::Setup(profile, reason)) => {
                    if profile.game != self.game_id(cx) {
                        continue;
                    }
                    self.setup = Some((profile, reason));
                    self.preview = None;
                    self.preview_source = None;
                }
                Ok(Update::CommunityPreview(profile, preview, installed_version)) => {
                    if profile.game != self.game_id(cx) {
                        continue;
                    }
                    self.selected_keys =
                        preview.differences.iter().map(|d| d.key.clone()).collect();
                    self.preview = Some((profile.content.title.clone(), preview));
                    self.preview_source = Some(profile);
                    self.patch_version = installed_version;
                    self.setup = None;
                    self.message =
                        "已核对当前游戏、补丁方案和入口；请检查参数差异，版本不同需自行确认。"
                            .into();
                }
                Ok(Update::Publish(game, content, context)) => {
                    if game != self.game_id(cx) {
                        continue;
                    }
                    self.name
                        .update(cx, |s, cx| s.set_value(content.title.clone(), window, cx));
                    self.context = context;
                    self.patch_version = content.patch_version.clone();
                    self.publish = Some((game, content));
                    self.publish_details = false;
                    self.message = "以下为完整提交内容；确认后才会上传".into();
                }
                Ok(Update::Published(identity)) => {
                    self.identity = Some(identity);
                    self.share_open = false;
                    self.mine_open = false;
                    self.publish = None;
                    self.edit_target = None;
                    self.edit_profile = None;
                    self.next_refresh = Instant::now();
                    self.message = "配置已分享，正在刷新玩家列表".into();
                }
                Ok(Update::Restore(identity, current)) => {
                    self.restore = Some((identity, current));
                    self.message = "请确认将恢复的公开身份；恢复不会合并两个身份".into();
                }
                Ok(Update::Notice(message)) => self.message = message,
                Ok(Update::Changed(message)) => {
                    self.share_open = false;
                    self.next_refresh = Instant::now();
                    self.preview = None;
                    self.publish = None;
                    self.edit_target = None;
                    self.edit_profile = None;
                    self.delete_target = None;
                    self.profiles.clear();
                    self.message = message;
                    let exe = self.target.game_exe.to_string_lossy().to_string();
                    let _ = self.owner.update(cx, |owner, cx| {
                        owner.c.inspect(exe);
                        cx.notify();
                    });
                }
            }
            cx.notify();
        }
        if self.tab != self.last_tab {
            self.last_tab = self.tab;
            self.next_refresh = Instant::now();
        }
        if self.tab == 2
            && !self.busy
            && !self.refresh_inflight
            && !self.mine_open
            && !self.share_open
            && self.preview.is_none()
            && self.publish.is_none()
            && Instant::now() >= self.next_refresh
            && config_store::valid_game(&self.game_id(cx))
        {
            let game = self.game_id(cx);
            let tx = self.tx.clone();
            self.refresh_inflight = true;
            self.next_refresh = Instant::now() + Duration::from_secs(30);
            std::thread::spawn(move || {
                let result = community::browse(&game, "", "").map_err(|e| e.to_string());
                let _ = tx.send(Ok(Update::AutoProfiles(game, result)));
            });
        }
    }
    fn refresh(&mut self, cx: &App) {
        self.mine_open = false;
        self.next_refresh = Instant::now() + Duration::from_secs(30);
        let game = self.game_id(cx);
        self.job(move || {
            let result = community::browse(&game, "", "")?;
            Ok(Update::Profiles(game, result))
        });
    }
    fn prepare_publish(&mut self, edit: Option<Profile>, cx: &App) {
        let target = self.target.clone();
        let Some(owner) = self.owner.upgrade() else {
            return;
        };
        let policies = owner.read(cx).c.catalog.scheme_policies.clone();
        let game = self.game_id(cx);
        let name = if self.name.read(cx).value().trim().is_empty() {
            format!(
                "{} · {}",
                "我的有效方案",
                self.hardware.gpu_model.as_deref().unwrap_or("我的方案")
            )
            .chars()
            .take(80)
            .collect::<String>()
        } else {
            self.name.read(cx).value().trim().to_owned()
        };
        let notes = self.notes.read(cx).value().to_string();
        self.edit_target = edit.as_ref().map(|p| (p.id.clone(), p.revision));
        let hardware = match self.metadata.hardware(cx) {
            Ok(h) => h,
            Err(e) => {
                self.message = e.to_string();
                return;
            }
        };
        let measurement = match self.metadata.measurement(cx) {
            Ok(m) => m,
            Err(e) => {
                self.message = e.to_string();
                return;
            }
        };
        let (dlss_mode, ray_tracing) = self.metadata.quality(cx);
        self.job(move || {
            let record =
                core::record(&target.directory)?.context("请先安装并试用补丁，再分享有效方案")?;
            let scheme = record
                .scheme_id
                .as_deref()
                .context("安装记录缺少补丁方案")?;
            let policy = policies.get(scheme).context("此已安装方案尚不支持分享")?;
            let context = presets::Context::new(scheme, policy, &target.game_exe);
            if let Some(p) = edit {
                anyhow::ensure!(
                    p.game == game && p.scheme == context.scheme && p.protocol == context.profile,
                    "游戏或补丁已改变，请重新刷新配置列表"
                );
            }
            let snapshot = config_store::snapshot(&target, &context, &game, &name)?;
            anyhow::ensure!(
                community_game::identify(&target)?.as_deref() == Some(&game),
                "请先确认并保存当前目录对应的游戏"
            );
            let entries = record.selected();
            anyhow::ensure!(entries.len() == 1, "分享前请选择一个 DLL / 插件入口");
            let patch_version = record
                .upstream_version
                .or(record.payload_version)
                .context("安装记录缺少补丁版本")?;
            let content = Content {
                title: name,
                notes,
                settings: snapshot.settings,
                hardware,
                patch_version,
                entry: Some(entries[0].clone()),
                runtime_versions: rtx_fg_manager::community_hardware::runtime_versions(&target),
                dlss_mode,
                ray_tracing,
                measurement,
            };
            community::validate_content(&content, &context.scheme, &context.profile)?;
            Ok(Update::Publish(game, content, context))
        });
    }
    fn share_content(&self, cx: &App) -> anyhow::Result<Content> {
        let mut content = self.publish.as_ref().context("请先读取当前方案")?.1.clone();
        content.title = self.name.read(cx).value().trim().to_owned();
        content.notes = self.notes.read(cx).value().to_string();
        content.hardware = self.metadata.hardware(cx)?;
        content.measurement = self.metadata.measurement(cx)?;
        (content.dlss_mode, content.ray_tracing) = self.metadata.quality(cx);
        community::validate_content(&content, &self.context.scheme, &self.context.profile)?;
        Ok(content)
    }
    fn submit_share(&mut self, cx: &App) {
        let Some((game, _)) = self.publish.clone() else {
            return;
        };
        let content = match self.share_content(cx) {
            Ok(content) => content,
            Err(error) => {
                self.message = error.to_string();
                return;
            }
        };
        let target = self.target.clone();
        let context = self.context.clone();
        let edit = self.edit_target.clone();
        let cancel = self.cancel.clone();
        self.job(move || {
            community::validate_content(&content,&context.scheme,&context.profile)?;
            let identity=community_identity::load_or_create()?;
            if let Some((id, revision))=edit {
                community::signed_with_verification(&identity,"PUT",&format!("/v1/profiles/{id}"),&json!({"revision":revision,"content":content}),&cancel)?;
            } else {
                let mut data = json!({"game":game,"scheme":context.scheme,"protocol":context.profile,"content":content});
                if let Some(name) = community_game::mapped_name(&target, &game)? { data["gameName"] = json!(name); }
                community::signed_with_verification(&identity,"POST","/v1/profiles",&data,&cancel)?;
            }
            Ok(Update::Published(identity.public()))
        });
        self.message = "正在分享；首次使用请在浏览器完成验证，成功后自动继续。".into();
    }
    fn preview_values(&mut self, name: String, values: presets::Values) {
        let target = self.target.clone();
        let context = self.context.clone();
        self.job(move || {
            Ok(Update::Preview(
                name,
                config_store::preview(&target, &context, &values)?,
            ))
        });
    }
    fn preview_profile(&mut self, profile: Profile, cx: &App) {
        let Some(owner) = self.owner.upgrade() else {
            return;
        };
        let Some(policy) = owner
            .read(cx)
            .c
            .catalog
            .scheme_policies
            .get(&profile.scheme)
        else {
            self.message = "此补丁方案需要更新管理器后才能使用".into();
            return;
        };
        let context = presets::Context::new(&profile.scheme, policy, &self.target.game_exe);
        if context.profile != profile.protocol {
            self.message = "配置协议不兼容，请先更新管理器".into();
            return;
        }
        let target = self.target.clone();
        self.job(move || {
            anyhow::ensure!(
                community_game::identify(&target)?.as_deref() == Some(&profile.game),
                "请先确认此目录对应的游戏"
            );
            community::validate_content(&profile.content, &profile.scheme, &profile.protocol)?;
            let record = core::record(&target.directory)?;
            if let Some(reason) = community::installation_issue(&profile, record.as_ref()) {
                return Ok(Update::Setup(profile, reason));
            }
            let preview = config_store::preview(&target, &context, &profile.content.settings)?;
            let record = record.context("安装记录缺失")?;
            let version = record
                .upstream_version
                .or(record.payload_version)
                .unwrap_or_else(|| "未知".into());
            Ok(Update::CommunityPreview(profile, preview, version))
        });
    }
    #[inline(never)]
    fn setup_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some((profile, reason)) = &self.setup else {
            return div().into_any_element();
        };
        let profile = profile.clone();
        let current = self
            .owner
            .upgrade()
            .map(|o| {
                let c = &o.read(cx).c;
                let p = c.catalog.selected(&profile.scheme);
                p.upstream_version
                    .as_deref()
                    .unwrap_or(&p.version)
                    .to_string()
            })
            .unwrap_or_default();
        div().v_flex().gap_2().p_3().border_1().border_color(cx.theme().border)
            .child(self.t(reason))
            .child(format!("{} · {} · {}", community::scheme_name(&profile.scheme), profile.content.patch_version,
                profile.content.entry.as_deref().unwrap_or("未提供加载入口")))
            .child(format!("管理器当前提供版本：{current}。版本不同请参考玩家备注，不能保证效果相同。"))
            .child(self.t("先退出游戏；已有不同补丁时先卸载，再安装此方案。安装后重新打开玩家推荐方案，预览并应用参数。"))
            .child(self.t("ASI 需要已有加载器；MFG 插件需要已有 OptiScaler 10。入口只决定加载方式，不增加图形 API 支持。"))
            .child(Button::new("community-setup-select").primary().label(self.t("选用此方案，返回安装"))
                .disabled(self.busy || profile.game != self.game_id(cx) || profile.content.entry.is_none())
                .on_click(cx.listener(move |this, _, window, cx| {
                    if profile.game != this.game_id(cx) { return; }
                    let Some(entry) = profile.content.entry.clone() else { return; };
                    let target = this.target.clone();
                    let result = this.owner.update(cx, |owner, cx| -> anyhow::Result<()> {
                        anyhow::ensure!(!owner.c.busy && !owner.c.read_only, "补丁操作尚未结束");
                        let game = owner.c.games.iter().find(|g| g.deployment_executables().iter().any(|e| core::key(std::path::Path::new(e)) == core::key(&target.game_exe)))
                            .context("游戏已从管理器移除")?;
                        let exe = game.exe.clone();
                        anyhow::ensure!(owner.c.catalog.schemes().iter().any(|s| s.scheme_id == profile.scheme)
                            && owner.c.catalog.proxies(&profile.scheme).contains(&entry), "当前资源清单不提供此方案或入口");
                        owner.c.focus = Some(exe);
                        let series = owner.c.cloud_series();
                        owner.c.set_selection(profile.scheme.clone(), series, vec![entry]);
                        owner.c.notice = Some("已为此游戏选好方案与入口；请按正常流程卸载旧补丁并安装，再返回玩家推荐方案应用参数。".into());
                        cx.notify();
                        Ok(())
                    });
                    match result {
                        Ok(Ok(())) => { window.close_dialog(cx); }
                        Ok(Err(e)) => this.message = e.to_string(),
                        Err(_) => this.message = "主窗口已关闭".into(),
                    }
                    cx.notify();
                })))
            .child(Button::new("community-setup-cancel").label(self.t("取消"))
                .on_click(cx.listener(|this, _, _, cx| { this.setup = None; cx.notify(); })))
            .into_any_element()
    }
    fn configs(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut body = div()
            .v_flex()
            .gap_2()
            .child(self.t("命名配置与导入"))
            .child(Input::new(&self.name))
            .child(
                div()
                    .h_flex()
                    .flex_wrap()
                    .gap_2()
                    .child(
                        Button::new("named-save")
                            .label(self.t("保存当前配置"))
                            .disabled(self.busy)
                            .on_click(cx.listener(|this, _, _, cx| {
                                let target = this.target.clone();
                                let context = this.context.clone();
                                let game = this.game_id(cx);
                                let name = this.name.read(cx).value().to_string();
                                this.job(move || {
                                    let envelope =
                                        config_store::snapshot(&target, &context, &game, &name)?;
                                    config_store::save_named(&target, &envelope)?;
                                    Ok(Update::Named(config_store::named(
                                        &target,
                                        &game,
                                        &context.scheme,
                                        &context.profile,
                                    )?))
                                });
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("named-list")
                            .label(self.t("读取本地配置"))
                            .disabled(self.busy)
                            .on_click(cx.listener(|this, _, _, cx| {
                                let target = this.target.clone();
                                let context = this.context.clone();
                                let game = this.game_id(cx);
                                this.job(move || {
                                    Ok(Update::Named(config_store::named(
                                        &target,
                                        &game,
                                        &context.scheme,
                                        &context.profile,
                                    )?))
                                });
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("config-export")
                            .label(self.t("导出当前配置"))
                            .disabled(self.busy)
                            .on_click(cx.listener(|this, _, _, cx| {
                                let target = this.target.clone();
                                let context = this.context.clone();
                                let game = this.game_id(cx);
                                let name = this.name.read(cx).value().to_string();
                                this.job(move || {
                                    let envelope =
                                        config_store::snapshot(&target, &context, &game, &name)?;
                                    if let Some(path) = rfd::FileDialog::new()
                                        .set_file_name("RTX-FG-config.json")
                                        .add_filter("JSON", &["json"])
                                        .save_file()
                                    {
                                        config_store::save_envelope(&path, &envelope)?;
                                    }
                                    Ok(Update::Notice(
                                        "导出操作结束；已有同名文件不会被覆盖".into(),
                                    ))
                                });
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("config-import")
                            .label(self.t("导入配置文件"))
                            .disabled(self.busy)
                            .on_click(cx.listener(|this, _, _, cx| {
                                let target = this.target.clone();
                                let context = this.context.clone();
                                let game = this.game_id(cx);
                                this.job(move || {
                                    let Some(path) = rfd::FileDialog::new()
                                        .add_filter("JSON", &["json"])
                                        .pick_file()
                                    else {
                                        return Ok(Update::Notice("已取消导入".into()));
                                    };
                                    let envelope = config_store::read_envelope(
                                        &path,
                                        &game,
                                        &context.scheme,
                                        &context.profile,
                                    )?;
                                    Ok(Update::Preview(
                                        envelope.name,
                                        config_store::preview(
                                            &target,
                                            &context,
                                            &envelope.settings,
                                        )?,
                                    ))
                                });
                                cx.notify();
                            })),
                    ),
            );
        for (id, envelope) in &self.named {
            let envelope = envelope.clone();
            body = body.child(
                Button::new(SharedString::from(format!("named-{id}")))
                    .label(format!("预览：{}", envelope.name))
                    .disabled(self.busy)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.preview_values(envelope.name.clone(), envelope.settings.clone());
                        cx.notify();
                    })),
            );
        }
        body.into_any_element()
    }
    fn preview_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some((name, preview)) = &self.preview else {
            return div().into_any_element();
        };
        let mut panel = div()
            .v_flex()
            .gap_2()
            .p_3()
            .border_1()
            .border_color(cx.theme().border)
            .child(format!("导入预览 · {name}"));
        if let Some(profile) = &self.preview_source {
            panel = panel.child(format!(
                "{} · {} · 玩家版本 {} · 当前版本 {} · {}",
                profile.game,
                community::scheme_name(&profile.scheme),
                profile.content.patch_version,
                self.patch_version,
                profile
                    .content
                    .entry
                    .as_deref()
                    .unwrap_or("未提供加载入口：仅参考参数")
            ));
        }
        for diff in &preview.differences {
            let key = diff.key.clone();
            panel = panel.child(
                Checkbox::new(SharedString::from(format!("diff-{key}")))
                    .label(format!(
                        "{}：{} → {}",
                        key,
                        diff.before.as_deref().unwrap_or("默认"),
                        diff.after
                    ))
                    .checked(self.selected_keys.contains(&key))
                    .on_click(cx.listener(move |this, yes, _, cx| {
                        if *yes {
                            this.selected_keys.insert(key.clone());
                        } else {
                            this.selected_keys.remove(&key);
                        }
                        cx.notify();
                    })),
            );
        }
        for (key, reason) in &preview.rejected {
            panel = panel.child(div().text_sm().child(format!("未导入 {key}：{reason}")));
        }
        panel
            .child(
                Button::new("import-confirm")
                    .primary()
                    .label(self.t("确认导入所选参数"))
                    .disabled(self.busy || self.selected_keys.is_empty())
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Some((name, preview)) = this.preview.clone() {
                            let keys = this.selected_keys.clone();
                            let target = this.target.clone();
                            let source = this.preview_source.clone();
                            if source.as_ref().is_some_and(|p| p.game != this.game_id(cx)) {
                                this.preview = None;
                                this.message = "游戏已改变，请重新选择玩家推荐方案".into();
                                cx.notify();
                                return;
                            }
                            this.job(move || {
                                if let Some(profile) = source {
                                    anyhow::ensure!(
                                        community_game::identify(&target)?.as_deref()
                                            == Some(&profile.game),
                                        "游戏映射已改变，请重新预览"
                                    );
                                    let record = core::record(&target.directory)?;
                                    anyhow::ensure!(
                                        community::installation_issue(&profile, record.as_ref())
                                            .is_none(),
                                        "补丁安装已改变，请重新预览"
                                    );
                                }
                                let selected = config_store::select_preview(&preview, &keys)?;
                                config_store::apply(&target, &selected, &name)?;
                                Ok(Update::Changed(
                                    "已导入；可在记录与恢复中撤销。重启游戏后生效".into(),
                                ))
                            });
                        }
                        cx.notify();
                    })),
            )
            .child(
                Button::new("import-cancel")
                    .label(self.t("取消预览"))
                    .disabled(self.busy)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.preview = None;
                        cx.notify();
                    })),
            )
            .into_any_element()
    }
    fn history_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut panel = div()
            .v_flex()
            .gap_2()
            .child(self.t("只恢复管理器保存的配置；后续改动发生冲突时停止恢复。"))
            .child(
                Button::new("history-refresh")
                    .label(self.t("刷新记录"))
                    .disabled(self.busy)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.local();
                        cx.notify();
                    })),
            );
        for record in &self.operations {
            panel = panel.child(
                div()
                    .v_flex()
                    .gap_1()
                    .child(format!(
                        "{} · {} · {} {}",
                        record.created,
                        self.tr.t(&record.action),
                        record.scheme,
                        record.version
                    ))
                    .child(self.t(if record.complete {
                        "已完成"
                    } else {
                        "未完成；请检查后重试"
                    }))
                    .child(record.detail.clone()),
            );
        }
        for record in &self.history {
            let id = record.id.clone();
            panel = panel.child(
                div()
                    .v_flex()
                    .gap_1()
                    .child(format!(
                        "{} · {} · {}",
                        record.created, record.name, record.state
                    ))
                    .child(
                        Button::new(SharedString::from(format!("undo-{id}")))
                            .label(self.t("撤销到导入之前"))
                            .disabled(
                                self.busy
                                    || !["prepared", "applied"].contains(&record.state.as_str()),
                            )
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let target = this.target.clone();
                                let id = id.clone();
                                this.job(move || {
                                    config_store::undo(&target, &id)?;
                                    Ok(Update::Changed("已恢复导入前的原始配置".into()))
                                });
                                cx.notify();
                            })),
                    ),
            );
        }
        panel.into_any_element()
    }
    fn profile_card(
        &self,
        matched: &community_match::Matched<'_>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = matched.profile.clone();
        let id = p.id.clone();
        let mut card = div()
            .v_flex()
            .gap_2()
            .p_3()
            .border_1()
            .border_color(cx.theme().border)
            .child(format!(
                "{} · {} · r{}",
                p.content.title,
                p.content
                    .hardware
                    .gpu_model
                    .as_deref()
                    .unwrap_or("显卡未填写"),
                p.revision
            ))
            .child(
                matched
                    .reasons
                    .iter()
                    .map(|s| self.tr.t(s))
                    .collect::<Vec<_>>()
                    .join(" · "),
            )
            .child(format!(
                "{}: {}",
                self.tr.t("驱动版本"),
                p.content.hardware.driver.as_deref().unwrap_or("—")
            ))
            .child(p.content.notes.clone())
            .child(format!(
                "{} · {} · {}",
                community::scheme_name(&p.scheme),
                p.content.patch_version,
                p.content.entry.as_deref().unwrap_or("未提供加载入口")
            ))
            .child(format!(
                "{} {} · {} {}{}",
                self.tr.t("点赞"),
                p.likes,
                self.tr.t("我能用"),
                p.works,
                if p.official {
                    format!(" · {}", self.tr.t("官方"))
                } else {
                    String::new()
                }
            ));
        if p.content.measurement.is_some() {
            card = card.child(self.t("测量数据由玩家提供"));
        }
        let preview = p.clone();
        card = card.child(
            Button::new(SharedString::from(format!("preview-{id}")))
                .label(self.t("应用此配置…"))
                .disabled(self.busy)
                .on_click(cx.listener(move |this, _, _, cx| {
                    if this.game_id(cx) != preview.game {
                        this.message = "游戏 ID 已改变，请先刷新社区列表".into();
                    } else {
                        this.preview_profile(preview.clone(), cx);
                    }
                    cx.notify();
                })),
        );
        let state = self.interactions.get(&(p.id.clone(), p.revision));
        let own = self
            .identity
            .as_ref()
            .is_some_and(|i| i.identity == p.author);
        if self.advanced_open
            && let Some(state) = state
        {
            card = card.child(format!(
                "我的状态：{} · {} · {}",
                if state.votes.contains("like") {
                    "已点赞"
                } else {
                    "未点赞"
                },
                if state.votes.contains("works") {
                    "已确认可用"
                } else {
                    "未确认"
                },
                if state.reported {
                    "已举报"
                } else {
                    "未举报"
                }
            ));
        } else if self.advanced_open && !own {
            let query = p.clone();
            card = card.child(
                Button::new(SharedString::from(format!("interactions-{id}")))
                    .small()
                    .label(self.t("读取我的互动状态"))
                    .disabled(self.busy || self.cached)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let p = query.clone();
                        this.job(move || {
                            let identity = community_identity::load_or_create()?;
                            let state = community::interactions(&identity, &p)?;
                            Ok(Update::Interactions(p.id, p.revision, state))
                        });
                        cx.notify();
                    })),
            );
        }
        let mut actions = div().h_flex().flex_wrap().gap_2();
        if !self.advanced_open {
            for (kind, label) in [("like", "点赞"), ("works", "我也能用")] {
                let p = p.clone();
                let active = state.is_some_and(|s| s.votes.contains(kind));
                actions = actions.child(
                    Button::new(SharedString::from(format!("simple-{kind}-{id}")))
                        .small()
                        .label(self.t(if active {
                            if kind == "like" {
                                "撤销赞"
                            } else {
                                "撤销确认"
                            }
                        } else {
                            label
                        }))
                        .disabled(self.busy || self.cached || own)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.feedback(p.clone(), kind);
                            cx.notify();
                        })),
                );
            }
            actions = actions.child(
                Button::new(SharedString::from(format!("more-{id}")))
                    .small()
                    .label(self.t("更多…"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.advanced_open = true;
                        cx.notify();
                    })),
            );
        }
        for (label, operation, kind, remove) in [
            ("点赞", "vote", "like", false),
            ("撤销赞", "vote", "like", true),
            ("我能用", "vote", "works", false),
            ("撤销确认", "vote", "works", true),
            ("举报", "report", "", false),
            ("撤销举报", "report", "", true),
        ] {
            if !self.advanced_open {
                break;
            }
            let p = p.clone();
            let allowed = state.is_none_or(|s| {
                let active = if operation == "vote" {
                    s.votes.contains(kind)
                } else {
                    s.reported
                };
                active == remove
            });
            actions = actions.child(
                Button::new(SharedString::from(format!(
                    "{operation}-{kind}-{remove}-{id}"
                )))
                .small()
                .label(self.t(label))
                .disabled(self.busy || self.cached || own || !allowed)
                .on_click(cx.listener(move |this, _, _, cx| {
                    let p = p.clone();
                    let reason = this.report_reason.read(cx).value().to_string();
                    this.job(move || {
                        let identity = community_identity::load_or_create()?;
                        let data = if operation == "vote" {
                            json!({"revision":p.revision,"kind":kind})
                        } else {
                            json!({"revision":p.revision,"reason":reason})
                        };
                        community::signed_request(
                            &identity,
                            if remove { "DELETE" } else { "POST" },
                            &format!("/v1/profiles/{}/{operation}", p.id),
                            &data,
                        )?;
                        let state = community::interactions(&identity, &p)?;
                        Ok(Update::Interactions(p.id, p.revision, state))
                    });
                    cx.notify();
                })),
            );
        }
        card = card.child(actions);
        if self
            .identity
            .as_ref()
            .is_some_and(|i| i.identity == p.author)
        {
            let edit = p.clone();
            let delete = p.clone();
            card = card
                .child(
                    Button::new(SharedString::from(format!("edit-{id}")))
                        .label(self.t("编辑并更新此配置…"))
                        .disabled(self.busy || self.cached)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.name.update(cx, |s, cx| {
                                s.set_value(edit.content.title.clone(), window, cx)
                            });
                            this.notes.update(cx, |s, cx| {
                                s.set_value(edit.content.notes.clone(), window, cx)
                            });
                            this.edit_target = Some((edit.id.clone(), edit.revision));
                            this.share_open = true;
                            this.edit_profile = Some(edit.clone());
                            this.publish = None;
                            this.prepare_publish(Some(edit.clone()), cx);
                            cx.notify();
                        })),
                )
                .child(
                    Button::new(SharedString::from(format!("delete-{id}")))
                        .label(self.t("删除此配置…"))
                        .disabled(self.busy || self.cached)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.delete_target = Some(delete.clone());
                            cx.notify();
                        })),
                );
        }
        if let Some(status) = &p.status {
            card = card.child(format!("状态：{status}"));
        }
        card.into_any_element()
    }
    fn feedback(&mut self, profile: Profile, kind: &'static str) {
        let cancel = self.cancel.clone();
        self.job(move || {
            let identity = community_identity::load_or_create()?;
            let route = format!("/v1/profiles/{}/interactions", profile.id);
            let state: community::Interactions =
                serde_json::from_value(community::signed_with_verification(
                    &identity,
                    "POST",
                    &route,
                    &json!({"revision":profile.revision}),
                    &cancel,
                )?)?;
            let remove = state.votes.contains(kind);
            community::signed_request(
                &identity,
                if remove { "DELETE" } else { "POST" },
                &format!("/v1/profiles/{}/vote", profile.id),
                &json!({"revision":profile.revision,"kind":kind}),
            )?;
            let state = community::interactions(&identity, &profile)?;
            Ok(Update::Interactions(profile.id, profile.revision, state))
        });
    }
    fn community_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let matches = community_match::select(
            &self.game_id(cx),
            &self.profiles,
            &self.hardware,
            &self.context.scheme,
            &self.patch_version,
            self.identity.as_ref().map(|i| i.identity.as_str()),
            &self.selection,
        );
        let mut panel = div()
            .v_flex()
            .gap_3()
            .child(
                div()
                    .h_flex()
                    .flex_wrap()
                    .gap_2()
                    .child(
                        Button::new("community-refresh")
                            .label(self.t("刷新配置"))
                            .disabled(self.busy || self.refresh_inflight)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.refresh(cx);
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("community-share")
                            .primary()
                            .label(self.t("分享我的方案"))
                            .disabled(self.busy || !config_store::valid_game(&self.game_id(cx)))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.share_open = !this.share_open;
                                this.publish = None;
                                if this.share_open { this.prepare_publish(this.edit_profile.clone(), cx); }
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("community-mine")
                            .label(self.t(if self.mine_open {
                                "返回玩家推荐方案"
                            } else {
                                "我的分享"
                            }))
                            .disabled(self.busy)
                            .on_click(cx.listener(|this, _, _, cx| {
                                if this.mine_open {
                                    this.refresh(cx);
                                } else {
                                    let game = this.game_id(cx);
                                    this.selection.show_other_gpus = true;
                                    this.selection.filters.clear();
                                    this.job(move || {
                                        let identity = community_identity::load_or_create()?;
                                        Ok(Update::Mine(
                                            game.clone(),
                                            community::mine(
                                                &identity,
                                                &game,
                                                "",
                                                "",
                                            )?,
                                            identity.public(),
                                        ))
                                    });
                                }
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("community-advanced")
                            .label(self.t(if self.advanced_open {
                                "收起高级选项"
                            } else {
                                "高级选项"
                            }))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.advanced_open = !this.advanced_open;
                                cx.notify();
                            })),
                    ),
            )
            .child(self.t("仅推荐当前游戏的配置"))
            .child(self.t("同一游戏内比较不同方案，先匹配显卡型号和驱动版本，再参考可用确认和点赞；每 30 秒自动刷新。"))
            .child(
                Checkbox::new("other-gpus")
                    .label(self.t("显示其他显卡"))
                    .checked(self.selection.show_other_gpus)
                    .on_click(cx.listener(|this, yes, _, cx| {
                        this.selection.show_other_gpus = *yes;
                        cx.notify();
                    })),
            );
        if self.share_open {
            panel = panel.child(self.share_panel(cx));
            if self.publish.is_some() {
                panel = panel.child(self.publish_panel(cx));
            }
        }
        if self.advanced_open {
            let mut filters = div().h_flex().flex_wrap().gap_2();
            for filter in community_match::FILTERS {
                filters = filters.child(
                    Checkbox::new(SharedString::from(format!("filter-{filter:?}")))
                        .label(format!(
                            "{} ({})",
                            self.t(filter.label()),
                            matches.counts[&filter]
                        ))
                        .checked(self.selection.filters.contains(&filter))
                        .on_click(cx.listener(move |this, yes, _, cx| {
                            if *yes {
                                this.selection.filters.insert(filter);
                            } else {
                                this.selection.filters.remove(&filter);
                            }
                            cx.notify();
                        })),
                );
            }
            panel = panel.child(filters).child(Input::new(&self.report_reason));
        }
        if matches.total == 0 {
            panel = panel.child(self.t(if !config_store::valid_game(&self.game_id(cx)) {
                "先确认当前游戏，即可查看对应推荐方案。"
            } else if self.profiles.is_empty() && (self.cached || self.refresh_inflight) {
                "暂未取得推荐方案，正在等待云端响应；可稍后刷新。"
            } else if self.profiles.is_empty() {
                "这里还没有玩家分享。你可以分享当前有效配置，帮助相近硬件的玩家。"
            } else {
                "当前筛选没有结果，可显示其他显卡或调整高级筛选。"
            }));
        }
        for (index, matched) in matches.page.iter().enumerate() {
            if index == 0
                && !self.mine_open
                && (matched.profile.works > 0 || matched.profile.likes > 0)
            {
                panel = panel.child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(self.t("优先参考 · 玩家推荐")),
                );
            }
            panel = panel.child(self.profile_card(matched, cx));
        }
        if matches.total > self.selection.visible.max(12) {
            panel = panel.child(
                Button::new("community-more")
                    .label(self.t("再显示 12 条"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.selection.visible = this.selection.visible.max(12) + 12;
                        cx.notify();
                    })),
            );
        }
        panel
            .child(
                div()
                    .text_xs()
                    .child(self.t("玩家反馈仅供参考；应用前可查看差异，应用后可撤销。")),
            )
            .into_any_element()
    }
    fn share_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut panel = div()
            .v_flex()
            .gap_2()
            .p_3()
            .border_1()
            .border_color(cx.theme().border)
            .child(self.t("分享当前游戏已安装补丁的配置"))
            .child(Input::new(&self.name))
            .child(Input::new(&self.notes))
            .child(self.t("硬件信息自动填写，可展开修改。不会上传游戏路径、日志或快捷键。"))
            .child(
                Button::new("metadata-toggle")
                    .label(self.t(if self.metadata_open {
                        "收起硬件与测量信息"
                    } else {
                        "显卡、驱动与可选测量…"
                    }))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.metadata_open = !this.metadata_open;
                        cx.notify();
                    })),
            );
        if self.metadata_open {
            for (i, hardware) in self.hardware_options.iter().enumerate() {
                let h = hardware.clone();
                panel = panel.child(
                    Button::new(SharedString::from(format!("hardware-{i}")))
                        .label(h.gpu_model.clone().unwrap_or_default())
                        .disabled(self.busy)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.hardware = h.clone();
                            this.metadata.set_hardware(&h, window, cx);
                            cx.notify();
                        })),
                );
            }
            panel = panel
                .child(self.metadata.render(&self.tr))
                .child(self.t("测量数据由玩家提供"));
        }
        if self.publish.is_none() || self.metadata_open {
            panel = panel.child(
                Button::new("publish-preview")
                    .primary()
                    .label(self.t("重新读取当前方案"))
                    .disabled(self.busy)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.prepare_publish(this.edit_profile.clone(), cx);
                        cx.notify();
                    })),
            );
        }
        if self.edit_profile.is_some() {
            panel = panel.child(
                Button::new("edit-abandon")
                    .label(self.t("取消修改，改为发布新配置"))
                    .disabled(self.busy)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.edit_profile = None;
                        this.edit_target = None;
                        this.publish = None;
                        cx.notify();
                    })),
            );
        }
        panel.into_any_element()
    }
}
impl CommunityView {
    // Keep render branches in separate stack frames: GPUI builder temporaries
    // otherwise exhaust the Windows UI stack when nested forms are opened.
    #[inline(never)]
    fn context_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut panel = div().v_flex().gap_3();
        if self.tab != 3 && (self.advanced_open || !config_store::valid_game(&self.game_id(cx))) {
            panel = panel.child(
                Button::new("game-catalog")
                    .label(self.t("从社区游戏目录选择…"))
                    .disabled(self.busy)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.job(|| Ok(Update::Games(community::games()?)));
                        cx.notify();
                    })),
            );
        }
        if self.tab == 2 && !config_store::valid_game(&self.game_id(cx)) {
            panel = panel
                .child(self.t("Steam、Epic、GOG、Xbox、WeGame 或独立安装均可分享。先搜索同一游戏；未收录时填写正式名称，并区分国服、国际服或重制版。"))
                .child(Input::new(&self.game_search))
                .child(Button::new("game-create-name")
                    .label(self.t("未收录：使用此游戏名称"))
                    .disabled(self.busy || self.game_search.read(cx).value().trim().is_empty())
                    .on_click(cx.listener(|this, _, _, cx| {
                        let name = this.game_search.read(cx).value().to_string();
                        let target = this.target.clone();
                        this.job(move || Ok(Update::BoundGame(community_game::save_named_mapping(&target, &name)?)));
                        cx.notify();
                    })));
        }
        let fallback_notice = match self.tr.language.as_str() {
            "ru" => Some("Новые разделы сообщества пока частично доступны на английском."),
            "ja" => Some("コミュニティの新しい項目は、未翻訳部分を英語で表示します。"),
            "ko" => Some("커뮤니티의 새 항목 중 미번역 부분은 영어로 표시됩니다."),
            _ => None,
        };
        if let Some(notice) = fallback_notice {
            panel = panel.child(div().text_sm().child(notice));
        }
        if self.tab != 2 || self.advanced_open {
            panel = panel.child(format!(
                "{}: {}",
                self.t("当前配置目录"),
                self.target.directory.display()
            ));
        }
        if self.targets.len() > 1 {
            panel = panel.child(self.t("本游戏有多个运行目录，请选择需要配置或恢复的目录。"));
            for (index, target) in self.targets.iter().enumerate() {
                let target = target.clone();
                panel = panel.child(
                    Button::new(SharedString::from(format!("config-target-{index}")))
                        .label(target.directory.to_string_lossy().into_owned())
                        .disabled(
                            self.busy
                                || core::key(&target.directory)
                                    == core::key(&self.target.directory),
                        )
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.target = target.clone();
                            this.history.clear();
                            this.named.clear();
                            this.profiles.clear();
                            this.interactions.clear();
                            this.preview = None;
                            this.publish = None;
                            this.edit_target = None;
                            this.edit_profile = None;
                            this.delete_target = None;
                            this.game.update(cx, |s, cx| s.set_value("", window, cx));
                            this.local();
                            cx.notify();
                        })),
                );
            }
        }
        if self.tab != 3
            && (self.advanced_open || !config_store::valid_game(&self.game_id(cx)))
            && !self.games.is_empty()
        {
            if config_store::valid_game(&self.game_id(cx)) {
                panel = panel.child(Input::new(&self.game_search));
            }
            let query = self.game_search.read(cx).value().to_lowercase();
            for game in self
                .games
                .iter()
                .filter(|g| {
                    format!("{} {}", g.id, g.name)
                        .to_lowercase()
                        .contains(query.trim())
                })
                .take(20)
            {
                let game = game.clone();
                panel = panel.child(
                    Button::new(SharedString::from(format!("select-{}", game.id)))
                        .label(format!("{} · {}", game.name, game.id))
                        .disabled(self.busy)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.game
                                .update(cx, |s, cx| s.set_value(game.id.clone(), window, cx));
                            this.profiles.clear();
                            this.preview = None;
                            this.publish = None;
                            this.edit_target = None;
                            this.edit_profile = None;
                            let target = this.target.clone();
                            let id = game.id.clone();
                            this.next_refresh = Instant::now();
                            this.job(move || {
                                community_game::save_mapping(&target, &id)?;
                                Ok(Update::Notice("已选择游戏，正在查找玩家推荐方案".into()))
                            });
                            cx.notify();
                        })),
                );
            }
        }
        if self.tab != 3 && (self.tab != 2 || self.advanced_open) {
            panel = panel.child(Input::new(&self.game)).child(
                Button::new("game-bind")
                    .label(self.t("确认并保存此目录的游戏 ID"))
                    .disabled(self.busy)
                    .on_click(cx.listener(|this, _, _, cx| {
                        let game = this.game_id(cx);
                        let target = this.target.clone();
                        this.job(move || {
                            community_game::save_mapping(&target, &game)?;
                            Ok(Update::Notice("已保存本机游戏映射".into()))
                        });
                        cx.notify();
                    })),
            );
        }
        panel = panel.child(div().text_sm().child(self.t(&self.message)));
        panel.into_any_element()
    }
    #[inline(never)]
    fn publish_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut panel = div().v_flex().gap_2();
        if let Some((game, content)) = &self.publish {
            panel = panel.child(
                self.edit_target
                    .as_ref()
                    .map(|(id, revision)| {
                        format!("更新配置 {id} · 当前修订 {revision}；旧修订的确认数不会保留")
                    })
                    .unwrap_or_else(|| "发布新配置".into()),
            );
            panel = panel
                .child(self.t("首次分享可能打开浏览器验证，完成后自动继续上传；之后无需重复验证。"))
                .child(format!(
                    "{} · {} · {}",
                    content.title,
                    content.hardware.gpu_model.as_deref().unwrap_or("—"),
                    content.patch_version
                ))
                .child(content.notes.clone())
                .child(format!(
                    "{} · {} · {} · {}",
                    game,
                    community::scheme_name(&self.context.scheme),
                    content.patch_version,
                    content.entry.as_deref().unwrap_or("未提供加载入口")
                ))
                .child(
                    Button::new("publish-details")
                        .label(self.t(if self.publish_details {
                            "收起上传详情"
                        } else {
                            "查看完整上传内容"
                        }))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.publish_details = !this.publish_details;
                            cx.notify();
                        })),
                );
            if self.publish_details {
                let details = self.share_content(cx).and_then(|content| Ok(serde_json::to_string_pretty(&json!({"game":game,"scheme":self.context.scheme,"protocol":self.context.profile,"content":content}))?)).unwrap_or_else(|error| error.to_string());
                panel = panel.child(div().text_sm().child(details));
            }
            panel = panel
                .child(
                    Button::new("publish-confirm")
                        .primary()
                        .label(self.t("一键分享"))
                        .disabled(self.busy)
                        .on_click(cx.listener(|this, _, _, cx| {
                            if this
                                .publish
                                .as_ref()
                                .is_some_and(|(game, _)| game != &this.game_id(cx))
                            {
                                this.publish = None;
                                this.message = "游戏已改变，请重新预览分享内容".into();
                                cx.notify();
                                return;
                            }
                            this.submit_share(cx);
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("publish-cancel")
                        .label(self.t("取消发布"))
                        .disabled(self.busy)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.publish = None;
                            this.share_open = false;
                            this.edit_target = None;
                            cx.notify();
                        })),
                );
        }

        panel.into_any_element()
    }
    #[inline(never)]
    fn delete_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut panel = div().v_flex().gap_2();
        if let Some(profile) = &self.delete_target {
            panel = panel
                .child(format!(
                    "确认删除：{}（修订 {}）？删除后玩家将无法浏览此配置。",
                    profile.content.title, profile.revision
                ))
                .child(
                    Button::new("profile-delete-confirm")
                        .label(self.t("确认删除此配置"))
                        .disabled(self.busy)
                        .on_click(cx.listener(|this, _, _, cx| {
                            if let Some(p) = this.delete_target.clone() {
                                this.job(move || {
                                    let identity = community_identity::load_or_create()?;
                                    community::signed_request(
                                        &identity,
                                        "DELETE",
                                        &format!("/v1/profiles/{}", p.id),
                                        &json!({"revision":p.revision}),
                                    )?;
                                    Ok(Update::Changed("配置已删除，请刷新列表".into()))
                                });
                            }
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("profile-delete-cancel")
                        .label(self.t("取消删除"))
                        .disabled(self.busy)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.delete_target = None;
                            cx.notify();
                        })),
                );
        }
        panel.into_any_element()
    }
    #[inline(never)]
    fn identity_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut panel = div().v_flex().gap_2();
        if self.tab == 2 && self.advanced_open {
            panel = panel
                .child(
                    Button::new("community-verify-recovery")
                        .label(self.t("重新验证分享权限"))
                        .disabled(self.busy)
                        .on_click(cx.listener(|this, _, _, cx| {
                            let cancel = this.cancel.clone();
                            this.job(move || {
                                let identity = community_identity::load_or_create()?;
                                community::register(&identity, &cancel)?;
                                Ok(Update::Identity(
                                    identity.public(),
                                    "身份已验证，可以发布配置或参与互动".into(),
                                ))
                            });
                            cx.notify();
                        })),
                )
                .child(
                    div()
                        .text_sm()
                        .child(self.t("社区身份备份与恢复（与维护者审核身份分开）")),
                )
                .child(Input::new(&self.password))
                .child(
                    Button::new("identity-backup")
                        .label(self.t("保存加密身份备份"))
                        .disabled(self.busy)
                        .on_click(cx.listener(|this, _, window, cx| {
                            let password =
                                zeroize::Zeroizing::new(this.password.read(cx).value().to_string());
                            this.password
                                .update(cx, |s, cx| s.set_value("", window, cx));
                            this.job(move || {
                                let identity = community_identity::load_or_create()?;
                                let Some(path) = rfd::FileDialog::new()
                                    .set_file_name("RTX-FG-community-identity.json")
                                    .save_file()
                                else {
                                    return Ok(Update::Notice("已取消备份".into()));
                                };
                                community_identity::save_backup(&path, &identity, &password)?;
                                Ok(Update::Identity(
                                    identity.public(),
                                    "已保存加密身份备份，请保管密码".into(),
                                ))
                            });
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("identity-restore-preview")
                        .label(self.t("选择备份并预览身份"))
                        .disabled(self.busy)
                        .on_click(cx.listener(|this, _, window, cx| {
                            let password =
                                zeroize::Zeroizing::new(this.password.read(cx).value().to_string());
                            this.password
                                .update(cx, |s, cx| s.set_value("", window, cx));
                            this.job(move || {
                                let Some(path) = rfd::FileDialog::new()
                                    .add_filter("JSON", &["json"])
                                    .pick_file()
                                else {
                                    return Ok(Update::Notice("已取消恢复".into()));
                                };
                                let bytes = serde_json::to_vec(&core::read_json(&path, 4096)?)?;
                                let identity =
                                    community_identity::Identity::from_backup(&bytes, &password)?;
                                let current = community_identity::current()?.map(|i| i.identity);
                                Ok(Update::Restore(identity, current))
                            });
                            cx.notify();
                        })),
                );
            if let Some((identity, current)) = &self.restore {
                panel = panel
                    .child(format!(
                        "当前：{}\n将恢复：{}",
                        current.as_deref().unwrap_or("无"),
                        identity.public().identity
                    ))
                    .child(
                        Button::new("identity-restore-confirm")
                            .label(self.t("确认恢复此身份"))
                            .disabled(self.busy)
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Some((identity, current)) = this.restore.take() {
                                    this.job(move || {
                                        community_identity::restore(&identity, current.as_deref())?;
                                        Ok(Update::Identity(
                                            identity.public(),
                                            "已恢复社区身份".into(),
                                        ))
                                    });
                                }
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("identity-restore-cancel")
                            .label(self.t("取消恢复"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.restore = None;
                                cx.notify();
                            })),
                    );
            }
        }
        panel.into_any_element()
    }
}
impl Render for CommunityView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.owner.upgrade().is_none_or(|owner| {
            let c = &owner.read(cx).c;
            c.read_only || c.busy || c.closing
        }) {
            return div().child(
                self.t("当前处于只读状态或补丁操作尚未结束，请完成后再使用配置与社区操作。"),
            );
        }
        let mut panel = div().v_flex().gap_3().child(self.context_panel(cx));
        panel = panel.child(match self.tab {
            2 => self.community_panel(cx),
            3 => self.history_panel(cx),
            _ => self.configs(cx),
        });
        panel = panel.child(self.preview_panel(cx));
        if self.setup.is_some() {
            panel = panel.child(self.setup_panel(cx));
        }
        if self.publish.is_some() && !self.share_open {
            panel = panel.child(self.publish_panel(cx));
        }
        if self.delete_target.is_some() {
            panel = panel.child(self.delete_panel(cx));
        }
        if self.tab == 2 && self.advanced_open {
            panel = panel.child(self.identity_panel(cx));
        }
        panel
    }
}
