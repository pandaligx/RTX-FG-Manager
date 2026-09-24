use anyhow::Result;
use rtx_fg_manager::{
    cleanup, cloud, core, gpu_alias, i18n, preferences,
    scanner::{self, Game},
    updater, win,
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
pub enum Event {
    Catalog(cloud::Catalog),
    Payload(Result<cloud::Prepared>, BTreeSet<String>, Arc<AtomicBool>),
    Scan(scanner::Report),
    Progress(String),
    PayloadProgress(cloud::CloudProgress),
    PatchResult(bool),
    Games(Vec<Game>),
    Statuses(u64, Vec<(String, String)>),
    RefreshDone,
    Status(String, String),
    Evidence(String, scanner::Evidence),
    PresetRead(
        String,
        Option<(String, rtx_fg_manager::presets::Values)>,
        bool,
        u64,
    ),
    PresetApplied(String, String, rtx_fg_manager::presets::Values),
    Devices(Vec<gpu_alias::Device>),
    Gpu(Vec<(String, i32)>),
    Icons(Vec<(String, Option<image::RgbaImage>)>),
    Log(String),
    Warning(String),
    Notice(String),
    Done,
    Checked(Result<Option<updater::Manifest>>, bool, Arc<AtomicBool>),
    Download(PathBuf),
    DownloadProgress(updater::DownloadProgress),
    UpdateDone,
    Installed,
    Saved(Result<()>),
    SmokeWritten,
}
pub struct Channel {
    tx: mpsc::Sender<Event>,
}
struct Completion(Channel);
struct RefreshCompletion(Channel);
impl Drop for RefreshCompletion {
    fn drop(&mut self) {
        self.0.send(Event::RefreshDone);
    }
}
impl Drop for Completion {
    fn drop(&mut self) {
        self.0.send(Event::Done);
    }
}
impl Clone for Channel {
    fn clone(&self) -> Self {
        Self {
            tx: self.tx.clone(),
        }
    }
}
impl Channel {
    pub fn send(&self, e: Event) {
        let _ = self.tx.send(e);
    }
    pub fn job(&self, f: impl FnOnce(&Channel) -> Result<()> + Send + 'static) {
        let c = self.clone();
        std::thread::spawn(move || {
            if let Err(e) = f(&c) {
                c.send(Event::Warning(format!("{e:#}")));
            }
        });
    }
    pub fn operation(&self, f: impl FnOnce(&Channel) -> Result<()> + Send + 'static) {
        self.job(move |c| {
            let _completion = Completion(c.clone());
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(c))) {
                Ok(result) => result,
                Err(_) => anyhow::bail!("后台操作异常终止，请检查操作日志后重试"),
            }
        });
    }
}
struct PatchRequest {
    clean: bool,
    targets: BTreeSet<String>,
}
#[derive(Default)]
struct PatchSummary {
    total: usize,
    succeeded: usize,
    failed: usize,
}
type DisplayCache = Arc<Mutex<BTreeMap<String, (Vec<(u64, u64)>, Instant, String)>>>;

// Display-only metadata cache. All writes and ownership checks use core's full validation.
fn display_stamp(exe: &std::path::Path) -> Result<Vec<(u64, u64)>> {
    use std::os::windows::fs::MetadataExt;
    let exe = core::no_links(exe)?;
    let dir = exe
        .parent()
        .ok_or_else(|| anyhow::anyhow!("无效游戏路径"))?;
    let mut paths = vec![exe.clone(), dir.join(core::OWN).join(core::MARKER)];
    paths.extend(core::PROXIES.iter().map(|n| dir.join(n)));
    paths.extend(
        [
            core::INI,
            "rtxfg_vk_bridge.dll",
            ".rtx-fg-script.json",
            ".rtx-fg-manager.json",
            ".rtx-fg-v3-legacy.json",
        ]
        .iter()
        .map(|n| dir.join(n)),
    );
    paths
        .into_iter()
        .map(|p| {
            core::no_links(&p)?;
            match std::fs::metadata(&p) {
                Ok(m) => Ok((m.file_size(), m.last_write_time())),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok((0, 0)),
                Err(e) => Err(e.into()),
            }
        })
        .collect()
}
pub struct Controller {
    pub catalog: cloud::Catalog,
    pub data: PathBuf,
    pub state: Value,
    pub store: Option<preferences::Store>,
    pub read_only: bool,
    pub tr: i18n::Translator,
    pub channel: Channel,
    pub rx: mpsc::Receiver<Event>,
    pub games: Vec<Game>,
    pub selected: BTreeSet<String>,
    pub focus: Option<String>,
    pub statuses: BTreeMap<String, String>,
    pub evidence: BTreeMap<String, Option<scanner::Evidence>>,
    pub disk_presets: BTreeMap<(String, String), rtx_fg_manager::presets::Values>,
    pub running_games: BTreeSet<String>,
    pub preset_reads: BTreeSet<String>,
    pub logs: VecDeque<String>,
    pub page: u8,
    pub busy: bool,
    pub critical: bool,
    pub status_epoch: u64,
    pub refresh_busy: bool,
    pub refresh_pending: bool,
    pub icons: BTreeMap<String, Option<Arc<gpui::RenderImage>>>,
    pub cancel: Arc<AtomicBool>,
    pub progress: String,
    pub payload_progress: Option<cloud::CloudProgress>,
    display_cache: DisplayCache,
    patch_summary: Option<PatchSummary>,
    save_revision: u64,
    pub save_error: Option<String>,
    pub update_busy: bool,
    pub checking: bool,
    pub update_installing: bool,
    pub update_status: String,
    pub update_cancel: Arc<AtomicBool>,
    pub update: Option<updater::Manifest>,
    pub update_offer: bool,
    pub install_pending: bool,
    install_requested: bool,
    pub download: Option<PathBuf>,
    pub download_progress: updater::DownloadProgress,
    pub last_check: Instant,
    pub devices: Vec<gpu_alias::Device>,
    pub device_index: usize,
    pub alias_index: usize,
    pub gpu: String,
    pub warning: Option<String>,
    pub notice: Option<String>,
    pub confirm: Option<String>,
    pending_patch: Option<PatchRequest>,
    pub search: String,
    pub smoke: Option<PathBuf>,
    pub closing: bool,
    pub saved: bool,
}
impl Controller {
    pub fn new(
        data: PathBuf,
        mut state: Value,
        error: Option<String>,
        smoke: Option<PathBuf>,
    ) -> Self {
        if state.get("download_source").is_none() {
            let explicit = state["cloud_source"]
                .as_str()
                .or_else(|| state["update_source"].as_str());
            state["download_source"] = json!(if explicit == Some("github") {
                "github"
            } else {
                "domestic"
            });
        }
        let tr = i18n::Translator::new(state["language"].as_str().unwrap_or("system"));
        let (tx, rx) = mpsc::channel();
        let channel = Channel { tx };
        let games = state["games"]
            .as_array()
            .map(|v| {
                v.iter()
                    .filter_map(|g| serde_json::from_value(g.clone()).ok())
                    .collect()
            })
            .unwrap_or_default();
        let read_only = error.is_some();
        let mut app = Self {
            catalog: cloud::bundled(),
            data: data.clone(),
            state,
            store: (!read_only).then(|| preferences::Store::new(data)),
            read_only,
            tr,
            channel,
            rx,
            games,
            selected: BTreeSet::new(),
            focus: None,
            statuses: BTreeMap::new(),
            evidence: BTreeMap::new(),
            disk_presets: BTreeMap::new(),
            running_games: BTreeSet::new(),
            preset_reads: BTreeSet::new(),
            logs: VecDeque::new(),
            page: 0,
            busy: false,
            critical: false,
            status_epoch: 0,
            refresh_busy: false,
            refresh_pending: false,
            icons: BTreeMap::new(),
            cancel: Arc::new(AtomicBool::new(false)),
            progress: String::new(),
            payload_progress: None,
            display_cache: Arc::new(Mutex::new(BTreeMap::new())),
            patch_summary: None,
            save_revision: 0,
            save_error: None,
            update_busy: false,
            checking: false,
            update_installing: false,
            update_status: String::new(),
            update_cancel: Arc::new(AtomicBool::new(false)),
            update: None,
            update_offer: false,
            install_pending: false,
            install_requested: false,
            download: None,
            download_progress: Default::default(),
            last_check: Instant::now(),
            devices: Vec::new(),
            device_index: 0,
            alias_index: 0,
            gpu: String::new(),
            warning: error,
            notice: None,
            confirm: None,
            pending_patch: None,
            search: String::new(),
            smoke,
            closing: false,
            saved: false,
        };
        if app.smoke.is_some()
            && let Some(path) = app.state["smoke_catalog"].as_str()
        {
            match core::read_json(std::path::Path::new(path), 1024 * 1024)
                .and_then(|v| Ok(serde_json::from_value::<cloud::Catalog>(v)?))
                .and_then(|c| {
                    c.validate()?;
                    Ok(c)
                }) {
                Ok(c) => app.catalog = c,
                Err(e) => app.warning = Some(e.to_string()),
            }
        }
        app.migrate_presets();
        app.log("游戏库已载入。扫描仅查找游戏，安装前请退出游戏。");
        if let Some(message) = app
            .state
            .as_object_mut()
            .and_then(|s| s.remove("recovery_notice"))
            .and_then(|v| v.as_str().map(str::to_owned))
        {
            app.log(&message);
            app.notice = Some(message);
        }
        if app.smoke.is_some() && app.boolean("smoke_focus", false) {
            app.focus = app.games.first().map(|g| g.exe.clone());
        }
        app.refresh();
        if app.smoke.is_none() {
            let prefer_github = app.choice("download_source", "domestic") == "github";
            let cancel = app.cancel.clone();
            app.channel.job(move |c| {
                c.send(Event::Catalog(cloud::cached()));
                match cloud::refresh_with_options(prefer_github, &cancel, |p| {
                    if !p.detail.is_empty() {
                        c.send(Event::Log(p.detail));
                    }
                }) {
                    Ok(catalog) => c.send(Event::Catalog(catalog)),
                    Err(e) => c.send(Event::Log(e.to_string())),
                }
                Ok(())
            });
        }
        app.channel.job(|c| {
            if let Ok(g) = win::gpu_names() {
                c.send(Event::Gpu(g));
            }
            match gpu_alias::enumerate() {
                Ok(v) => c.send(Event::Devices(v)),
                Err(e) => c.send(Event::Log(e.to_string())),
            }
            Ok(())
        });
        if app.smoke.is_none() && app.boolean("auto_update", true) {
            app.check_update(true)
        }
        if app.smoke.is_some() && app.boolean("smoke_scan", false) {
            app.scan(win::drives());
        }
        // Explicit --ui-smoke only: exercise current-version downloads without
        // changing production version checks or normal saved preferences.
        if app.smoke.is_some() && app.boolean("smoke_update", false) {
            app.page = 2;
            if let Ok(m) = serde_json::from_value::<updater::Manifest>(
                app.state["smoke_update_manifest"].clone(),
            ) && m.validate(&m.version).is_ok()
                && updater::safe_url(&m.url).is_ok()
            {
                app.update = Some(m);
                app.update_offer = app.boolean("smoke_update_offer", false);
                if app.update_offer {
                    app.page = 0;
                }
            }
            if let Ok(p) = serde_json::from_value::<updater::DownloadProgress>(
                app.state["smoke_update_progress"].clone(),
            ) {
                app.update_status = p.label().into();
                app.download_progress = p;
            }
        }
        app
    }
    pub fn text(&self, s: &str) -> String {
        self.tr.t(s)
    }
    pub fn cloud_label(&self, p: &cloud::Package) -> String {
        p.labels
            .get(&self.tr.language)
            .cloned()
            .unwrap_or_else(|| self.text(&p.label))
    }
    pub fn cloud_scheme(&self) -> String {
        let requested = self.choice("cloud_scheme", &self.catalog.default_scheme);
        let requested = if requested.starts_with("initial-") {
            "initial".into()
        } else if requested.contains("experimental") {
            "native-0.2.6-stable".into()
        } else {
            requested
        };
        self.catalog.selected(&requested).scheme_id.clone()
    }
    pub fn cloud_series(&self) -> usize {
        if let Some(series) = self.state["cloud_series"].as_u64()
            && series <= 1
        {
            return series as usize;
        }
        if self.choice("backend", "native30").ends_with("20") {
            0
        } else {
            1
        }
    }
    pub fn choice(&self, key: &str, default: &str) -> String {
        self.state[key].as_str().unwrap_or(default).into()
    }
    pub fn boolean(&self, key: &str, default: bool) -> bool {
        self.state[key].as_bool().unwrap_or(default)
    }
    pub fn parameter_profile(&self) -> String {
        self.catalog.scheme_policies[&self.cloud_scheme()]
            .parameter_profile
            .clone()
    }
    pub fn preset_values(&self, exe: &str) -> rtx_fg_manager::presets::Values {
        self.preset_values_for(exe, &self.cloud_scheme())
    }
    fn preset_values_for(&self, exe: &str, scheme: &str) -> rtx_fg_manager::presets::Values {
        let policy = &self.catalog.scheme_policies[scheme];
        let profile = &policy.parameter_profile;
        let mut values = rtx_fg_manager::presets::defaults(profile, &policy.defaults);
        if let Some(game) = self.games.iter().find(|g| g.exe == exe) {
            if let Some(saved) = game
                .extra
                .get("preset_options")
                .and_then(|v| v.get(profile))
            {
                if let Ok(saved) =
                    serde_json::from_value::<rtx_fg_manager::presets::Values>(saved.clone())
                {
                    for (key, value) in saved {
                        let item = BTreeMap::from([(key.clone(), value.clone())]);
                        if rtx_fg_manager::presets::validate(profile, &item).is_ok() {
                            values.insert(key, value);
                        }
                    }
                }
            } else if profile.starts_with("upstream")
                && let Some(old) = game
                    .extra
                    .get("upstream_options")
                    .and_then(|v| serde_json::from_value::<core::UpstreamOptions>(v.clone()).ok())
                    .filter(|o| o.validate().is_ok())
            {
                values.extend(BTreeMap::from([
                    ("optimized".into(), u8::from(old.optimized).to_string()),
                    (
                        "max_generated_frames".into(),
                        old.max_generated_frames.to_string(),
                    ),
                    ("preset".into(), old.preset),
                    ("logging_level".into(), old.logging_level.to_string()),
                ]));
            }
        }
        if profile == "native026" && self.cloud_series() == 0 {
            values.insert("hardware_bilinear".into(), "0".into());
        }
        let scheme = scheme.to_owned();
        let game = self.games.iter().find(|g| g.exe == exe);
        let saved = game
            .and_then(|g| g.extra.get("preset_options_v2"))
            .and_then(|v| v.get(&scheme))
            .and_then(|v| {
                serde_json::from_value::<rtx_fg_manager::presets::Values>(v.clone()).ok()
            });
        let dirty = game
            .and_then(|g| g.extra.get("preset_dirty"))
            .and_then(|v| v.get(&scheme))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if let Some(saved) = &saved {
            for (key, value) in saved {
                if rtx_fg_manager::presets::validate(
                    profile,
                    &BTreeMap::from([(key.clone(), value.clone())]),
                )
                .is_ok()
                {
                    values.insert(key.clone(), value.clone());
                }
            }
        }
        if !dirty && let Some(disk) = self.disk_presets.get(&(exe.into(), scheme.clone())) {
            values.extend(disk.clone());
        }
        rtx_fg_manager::presets::Context::new(&scheme, policy, std::path::Path::new(exe))
            .normalize(&mut values);
        if profile == "native026" && self.cloud_series() == 0 {
            values.insert("hardware_bilinear".into(), "0".into());
        }
        values
    }
    fn migrate_presets(&mut self) {
        let mut updates = Vec::new();
        let mut clamped = Vec::new();
        for game in &self.games {
            for (scheme, policy) in &self.catalog.scheme_policies {
                let context = rtx_fg_manager::presets::Context::new(
                    scheme,
                    policy,
                    std::path::Path::new(&game.exe),
                );
                let saved = game
                    .extra
                    .get("preset_options_v2")
                    .and_then(|v| v.get(scheme));
                let old = saved.or_else(|| {
                    game.extra
                        .get("preset_options")
                        .and_then(|v| v.get(&policy.parameter_profile))
                });
                let old_count = old
                    .and_then(|v| v.get("max_generated_frames"))
                    .and_then(Value::as_str)
                    .and_then(|v| v.parse::<u32>().ok())
                    .or_else(|| {
                        game.extra
                            .get("upstream_options")
                            .and_then(|v| v.get("max_generated_frames"))
                            .and_then(Value::as_u64)
                            .and_then(|v| u32::try_from(v).ok())
                    });
                let clamp = context.delta && old_count.is_some_and(|v| (4..=5).contains(&v));
                if saved.is_none() || clamp {
                    updates.push((
                        game.exe.clone(),
                        scheme.clone(),
                        self.preset_values_for(&game.exe, scheme),
                    ));
                }
                if clamp && game.extra.get("delta_clamp_notified") != Some(&json!(true)) {
                    clamped.push(game.exe.clone());
                }
            }
        }
        if updates.is_empty() && clamped.is_empty() {
            return;
        }
        for (exe, scheme, values) in updates {
            let game = self.games.iter_mut().find(|g| g.exe == exe).unwrap();
            let entry = game
                .extra
                .entry("preset_options_v2")
                .or_insert_with(|| json!({}));
            if !entry.is_object() {
                *entry = json!({});
            }
            entry[scheme] = json!(values);
        }
        for exe in clamped {
            self.games
                .iter_mut()
                .find(|g| g.exe == exe)
                .unwrap()
                .extra
                .insert("delta_clamp_notified".into(), json!(true));
            self.log("三角洲专项最高支持4X，原5X/6X已调整为4X");
            self.notice = Some("三角洲专项最高支持4X，原5X/6X已调整为4X".into());
        }
        self.save();
    }
    pub fn preset_context(&self, exe: &str) -> rtx_fg_manager::presets::Context {
        let scheme = self.cloud_scheme();
        rtx_fg_manager::presets::Context::new(
            &scheme,
            &self.catalog.scheme_policies[&scheme],
            std::path::Path::new(exe),
        )
    }
    pub fn set_game_option(&mut self, exe: &str, key: &str, value: &str) {
        if self.busy || self.read_only {
            return;
        }
        if self.running_games.contains(exe) || self.preset_reads.contains(exe) {
            self.warning = Some(self.text("请先完全退出游戏再修改参数"));
            return;
        }
        let profile = self.parameter_profile();
        let scheme = self.cloud_scheme();
        let mut values = self.preset_values(exe);
        if key == "reset" {
            values = rtx_fg_manager::presets::defaults(
                &profile,
                &self.catalog.scheme_policies[&self.cloud_scheme()].defaults,
            );
        } else {
            values.insert(key.into(), value.into());
        }
        self.preset_context(exe).normalize(&mut values);
        if rtx_fg_manager::presets::validate(&profile, &values).is_err() {
            return;
        }
        let changed = self.disk_presets.get(&(exe.to_owned(), scheme.clone())) != Some(&values);
        if let Some(game) = self.games.iter_mut().find(|g| g.exe == exe) {
            let entry = game
                .extra
                .entry("preset_options_v2")
                .or_insert_with(|| json!({}));
            if !entry.is_object() {
                *entry = json!({});
            }
            entry[&scheme] = json!(values);
            let dirty = game
                .extra
                .entry("preset_dirty")
                .or_insert_with(|| json!({}));
            if !dirty.is_object() {
                *dirty = json!({});
            }
            dirty[&scheme] = json!(changed);
            self.save();
        }
    }
    pub fn save(&mut self) {
        if self.read_only {
            return;
        }
        self.state["schema"] = json!(3);
        self.state["games"] = serde_json::to_value(&self.games).unwrap_or(json!([]));
        if let Some(s) = &self.store {
            match s.save_tracked(self.state.clone()) {
                Ok(revision) => self.save_revision = revision,
                Err(e) => self.save_error = Some(e.to_string()),
            }
        }
    }
    pub fn preset_dirty(&self, exe: &str) -> bool {
        self.games
            .iter()
            .find(|g| g.exe == exe)
            .and_then(|g| g.extra.get("preset_dirty"))
            .and_then(|v| v.get(self.cloud_scheme()))
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }
    pub fn can_apply_parameters(&self) -> bool {
        self.focus.as_ref().is_some_and(|exe| {
            self.disk_presets
                .contains_key(&(exe.clone(), self.cloud_scheme()))
                && self
                    .statuses
                    .get(exe)
                    .is_some_and(|s| s.starts_with("已部署"))
        })
    }
    pub fn apply_focused_parameters(&mut self) {
        if self.busy || self.closing || self.read_only {
            return;
        }
        let Some(exe) = self.focus.clone() else {
            return;
        };
        let context = self.preset_context(&exe);
        let values = self.preset_values(&exe);
        self.busy = true;
        self.critical = true;
        self.status_epoch += 1;
        self.progress = "正在应用参数…".into();
        self.channel.operation(move |c| {
            let path = std::path::Path::new(&exe);
            match core::apply_parameters(path, &context, &values) {
                Ok(()) => {
                    c.send(Event::PresetApplied(exe.clone(), context.scheme, values));
                    c.send(Event::Log("参数已应用；下次启动游戏生效。".into()));
                }
                Err(e) => c.send(Event::Warning(e.to_string())),
            }
            c.send(Event::Status(exe.clone(), core::status(path)));
            Ok(())
        });
    }
    pub fn clear_cache(&mut self) {
        if self.busy || self.update_busy || self.read_only || self.closing {
            return;
        }
        self.busy = true;
        self.critical = true;
        self.download = None;
        self.download_progress = Default::default();
        let data = self.data.clone();
        self.channel.operation(move |c| {
            let _lock = rtx_fg_manager::cache::operation_lock()?;
            let r =
                rtx_fg_manager::cache::clean_scoped(&rtx_fg_manager::assets::cache_root()?, &data)?;
            let delta = rtx_fg_manager::delta::clear_available()?;
            c.send(Event::Notice(format!(
                "缓存清理完成：{0} 个文件，{1} MiB；跳过 {2} 项。",
                r.files + delta.files,
                (r.bytes + delta.bytes) / 1024 / 1024,
                r.skipped + delta.pending.len() as u64
            )));
            if !delta.pending.is_empty() {
                c.send(Event::Warning(format!(
                    "部分组件缓存待清理：{}",
                    delta.pending.join("；")
                )));
            }
            Ok(())
        });
    }
    pub fn log(&mut self, s: &str) {
        self.logs.push_back(format!(
            "{}  {}",
            chrono::Local::now().format("%H:%M:%S"),
            self.text(s)
        ));
        while self.logs.len() > 1000 {
            self.logs.pop_front();
        }
    }
    pub fn refresh(&mut self) {
        if self.closing {
            return;
        }
        if self.refresh_busy {
            self.refresh_pending = true;
            return;
        }
        self.refresh_busy = true;
        if let Some(exe) = self.focus.clone() {
            self.inspect(exe);
        }
        let epoch = self.status_epoch;
        let mut games = self.games.clone();
        games.sort_by_key(|g| self.focus.as_ref() != Some(&g.exe));
        let missing = games
            .iter()
            .filter(|g| !self.icons.contains_key(&g.exe))
            .map(|g| g.exe.clone())
            .collect::<Vec<_>>();
        for exe in &missing {
            self.icons.insert(exe.clone(), None);
        }
        if !missing.is_empty() {
            self.channel.job(move |c| {
                for chunk in missing.chunks(16) {
                    c.send(Event::Icons(
                        chunk
                            .iter()
                            .map(|exe| {
                                (
                                    exe.clone(),
                                    crate::game_icons::extract(std::path::Path::new(exe)).ok(),
                                )
                            })
                            .collect(),
                    ));
                }
                Ok(())
            });
        }
        let cache = self.display_cache.clone();
        self.channel.job(move |c| {
            let _completion = RefreshCompletion(c.clone());
            for g in games {
                let p = std::path::Path::new(&g.exe);
                let stamp = display_stamp(p).ok();
                let cached = cache.lock().ok().and_then(|m| {
                    m.get(&g.exe)
                        .filter(|(old, at, _)| {
                            Some(old) == stamp.as_ref() && at.elapsed() < Duration::from_secs(30)
                        })
                        .map(|(_, _, s)| s.clone())
                });
                let freshly_checked = cached.is_none();
                let status = cached.unwrap_or_else(|| core::status(p));
                if freshly_checked
                    && let Some(stamp) = stamp
                    && let Ok(mut m) = cache.lock()
                {
                    m.insert(g.exe.clone(), (stamp, Instant::now(), status.clone()));
                    if m.len() > 10000 {
                        m.clear();
                    }
                }
                c.send(Event::Statuses(epoch, vec![(g.exe, status)]));
            }
            Ok(())
        })
    }
    pub fn merge(&mut self, rows: Vec<Game>) {
        for g in rows {
            if self.games.len() >= 10000 {
                break;
            }
            if !self
                .games
                .iter()
                .any(|a| a.exe.eq_ignore_ascii_case(&g.exe))
            {
                self.games.push(g);
            }
        }
        self.save();
        self.refresh();
    }
    pub fn scan(&mut self, roots: Vec<PathBuf>) {
        if self.busy {
            return;
        }
        self.state["roots"] = json!(roots);
        self.save();
        self.busy = true;
        self.cancel = Arc::new(AtomicBool::new(false));
        let cancel = self.cancel.clone();
        self.log(&format!(
            "开始扫描：{}",
            roots
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join("；")
        ));
        self.channel.operation(move |c| {
            let result = scanner::scan(&roots, &cancel, |n, _, p| {
                c.send(Event::Progress(format!("{n}  ·  {}", p.display())))
            });
            match result {
                Ok(r) => c.send(Event::Scan(r)),
                Err(e) => c.send(Event::Warning(e.to_string())),
            }
            Ok(())
        });
    }
    pub fn pick(&mut self, folder: bool) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.cancel = Arc::new(AtomicBool::new(false));
        let cancel = self.cancel.clone();
        self.channel.operation(move |c| {
            (|| {
                if folder {
                    if let Some(p) = rfd::FileDialog::new().pick_folder() {
                        let r = scanner::scan(&[p], &cancel, |n, _, p| {
                            c.send(Event::Progress(format!("{n}  ·  {}", p.display())))
                        })?;
                        c.send(Event::Scan(r));
                    }
                } else if let Some(files) = rfd::FileDialog::new()
                    .add_filter("Windows EXE", &["exe"])
                    .pick_files()
                {
                    let mut rows = Vec::new();
                    for p in files {
                        let p = core::library_location(&p)?;
                        rows.push(Game {
                            exe: p.display().to_string(),
                            root: scanner::game_root(&p).display().to_string(),
                            ..Default::default()
                        });
                    }
                    c.send(Event::Games(rows));
                }
                Ok(())
            })()
        });
    }
    pub fn request_patch(&mut self, clean: bool) {
        if self.closing || self.busy || self.pending_patch.is_some() || (self.read_only && !clean) {
            return;
        }
        let targets = self.targets();
        if targets.len() > 1 {
            self.pending_patch = Some(PatchRequest { clean, targets });
            self.confirm = Some(if clean { "clean_batch" } else { "deploy_batch" }.into());
        } else {
            self.perform(clean, targets);
        }
    }
    pub fn pending_patch_list(&self) -> String {
        self.pending_patch
            .as_ref()
            .map(|p| {
                p.targets
                    .iter()
                    .enumerate()
                    .map(|(i, path)| format!("{}. {}", i + 1, path))
                    .collect::<Vec<_>>()
                    .join("\n\n")
            })
            .unwrap_or_default()
    }
    pub fn confirm_patch(&mut self) {
        if let Some(p) = self.pending_patch.take() {
            self.perform(p.clean, p.targets);
        }
    }
    pub fn cancel_patch(&mut self) {
        self.pending_patch = None;
        if matches!(
            self.confirm.as_deref(),
            Some("deploy_batch" | "clean_batch")
        ) {
            self.confirm = None;
        }
    }
    fn perform(&mut self, clean: bool, paths: BTreeSet<String>) {
        if self.closing || self.busy || paths.is_empty() || (self.read_only && !clean) {
            return;
        }
        self.status_epoch += 1;
        self.critical = clean;
        self.patch_summary = Some(PatchSummary {
            total: paths.len(),
            ..Default::default()
        });
        let proxies = self.proxies();
        self.busy = true;
        for exe in &paths {
            self.statuses.insert(
                exe.clone(),
                if clean {
                    "正在卸载…"
                } else {
                    "正在安装…"
                }
                .into(),
            );
        }
        self.cancel = Arc::new(AtomicBool::new(false));
        let cancel = self.cancel.clone();
        if !clean {
            let mut catalog = self.catalog.clone();
            catalog.prefer_github = self.choice("download_source", "domestic") == "github";
            let scheme = self.cloud_scheme();
            let series = self.cloud_series();
            self.progress = self.text("正在下载 DLL…");
            self.channel.job(move |c| {
                let mut eligible = BTreeSet::new();
                let mut errors = Vec::new();
                c.send(Event::Progress("正在检查安装条件…".into()));
                for exe in paths {
                    if cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    match core::preflight_install(std::path::Path::new(&exe), &proxies) {
                        Ok(()) => {
                            eligible.insert(exe);
                        }
                        Err(e) => {
                            errors.push(format!(
                                "{}：{e}",
                                std::path::Path::new(&exe)
                                    .file_name()
                                    .unwrap_or_default()
                                    .to_string_lossy()
                            ));
                            c.send(Event::PatchResult(false));
                            c.send(Event::Status(
                                exe.clone(),
                                core::status(std::path::Path::new(&exe)),
                            ));
                        }
                    }
                }
                if !errors.is_empty() {
                    c.send(Event::Warning(errors.join("\n\n")));
                }
                if eligible.is_empty() || cancel.load(Ordering::Relaxed) {
                    c.send(Event::Done);
                    return Ok(());
                }
                let result = cloud::prepare_with_progress(
                    &catalog,
                    &scheme,
                    series,
                    &proxies,
                    &cancel,
                    |p| c.send(Event::PayloadProgress(p)),
                );
                c.send(Event::Payload(result, eligible, cancel));
                Ok(())
            });
            return;
        }
        self.run_patch(paths, None);
    }
    fn run_patch(&mut self, paths: BTreeSet<String>, prepared: Option<cloud::Prepared>) {
        self.critical = true;
        let proxies = self.proxies();
        let options = paths
            .iter()
            .map(|exe| (exe.clone(), self.preset_values(exe)))
            .collect::<BTreeMap<_, _>>();
        let cancel = self.cancel.clone();
        self.channel.operation(move |c| {
            let mut errors = Vec::new();
            for exe in paths {
                if cancel.load(Ordering::Relaxed) {
                    break;
                }
                let p = PathBuf::from(&exe);
                let result = if let Some(ref payload) = prepared {
                    (|| {
                        let mut files = payload.files.clone();
                        let context = rtx_fg_manager::presets::Context::new(
                            &payload.scheme_id,
                            &payload.policy,
                            &p,
                        );
                        let configured = context.configure(&files[core::INI], &options[&exe])?;
                        files.insert(core::INI.into(), configured);
                        core::deploy_prepared_context(
                            &p,
                            &payload.backend,
                            &proxies,
                            None,
                            files,
                            Some(&payload.version),
                            Some(&context),
                        )
                    })()
                } else {
                    cleanup::clean(&p)
                };
                let name = p.file_name().unwrap_or_default().to_string_lossy();
                match result {
                    Ok(s) => {
                        c.send(Event::PatchResult(!s.starts_with("补丁已移除，缓存待清理")));
                        if s.starts_with("补丁已移除，缓存待清理") {
                            errors.push(format!("{name}：{s}"));
                        } else {
                            c.send(Event::Log(format!("{name}：{s}")));
                        }
                        if let Some(payload) = &prepared {
                            c.send(Event::PresetApplied(
                                exe.clone(),
                                payload.scheme_id.clone(),
                                options[&exe].clone(),
                            ));
                        }
                    }
                    Err(e) => {
                        c.send(Event::PatchResult(false));
                        let s = format!("{name}：{e}");
                        errors.push(s);
                    }
                }
                c.send(Event::Status(exe, core::status(&p)));
            }
            if !errors.is_empty() {
                c.send(Event::Warning(errors.join("\n\n")))
            }
            Ok(())
        });
    }
    /// Checked games are explicit batch targets; otherwise act on the focused row.
    pub fn targets(&self) -> BTreeSet<String> {
        self.games
            .iter()
            .filter(|g| {
                if self.selected.is_empty() {
                    self.focus.as_ref() == Some(&g.exe)
                } else {
                    self.selected.contains(&g.exe)
                }
            })
            .map(|g| g.exe.clone())
            .collect()
    }
    pub fn can_forget_focused_without_confirmation(&self) -> bool {
        !self.busy
            && !self.closing
            && self.focus.as_ref().is_some_and(|exe| {
                self.games.iter().any(|g| &g.exe == exe)
                    && self.statuses.get(exe).is_some_and(|s| s == "未部署")
            })
    }
    pub fn forget(&mut self, paths: &BTreeSet<String>) {
        if self.busy || self.closing {
            return;
        }
        self.status_epoch += 1;
        // Library metadata only: deployment records remain next to the game.
        self.games.retain(|g| !paths.contains(&g.exe));
        self.selected.retain(|p| !paths.contains(p));
        self.statuses.retain(|p, _| !paths.contains(p));
        self.icons.retain(|p, _| !paths.contains(p));
        self.evidence.retain(|p, _| !paths.contains(p));
        if self.focus.as_ref().is_some_and(|p| paths.contains(p)) {
            self.focus = None;
        }
        self.save();
    }
    pub fn inspect(&mut self, exe: String) {
        // A read started during a mutation has the mutation's epoch but can see
        // the old INI. Do not let that late result replace PresetApplied. Reads
        // already in flight are rejected by the epoch advanced at write start;
        // Done refreshes the focused game after all writes have completed.
        if !self.closing && !self.busy && self.preset_reads.insert(exe.clone()) {
            let target = exe.clone();
            let catalog = self.catalog.clone();
            let epoch = self.status_epoch;
            self.channel.job(move |c| {
                let p = std::path::Path::new(&target);
                let running = p
                    .parent()
                    .filter(|p| p.is_dir())
                    .is_some_and(|p| win::running_in_directory(p).map_or(true, |v| !v.is_empty()));
                let values = rtx_fg_manager::presets::inspect(p, &catalog);
                if let Err(e) = &values {
                    c.send(Event::Log(e.to_string()));
                }
                c.send(Event::PresetRead(
                    target,
                    values.unwrap_or(None),
                    running,
                    epoch,
                ));
                Ok(())
            });
        }
        if self.closing || self.evidence.contains_key(&exe) {
            return;
        }
        self.evidence.insert(exe.clone(), None);
        self.channel.job(move |c| {
            let evidence = scanner::inspect(std::path::Path::new(&exe));
            c.send(Event::Evidence(exe, evidence));
            Ok(())
        });
    }
    pub fn proxies(&self) -> Vec<String> {
        let allowed = self.catalog.proxies(&self.cloud_scheme());
        let limit = self.catalog.scheme_policies[&self.cloud_scheme()].max_selected_proxies;
        let selected: Vec<String> = self.state["proxies"]
            .as_array()
            .map(|v| {
                v.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .filter(|v: &Vec<String>| core::normalize_proxies(v).is_ok())
            .unwrap_or_else(|| vec!["version.dll".into()])
            .into_iter()
            .filter(|p| allowed.contains(p))
            .take(limit)
            .collect();
        if selected.is_empty() {
            vec![if allowed.iter().any(|p| p == "version.dll") {
                "version.dll".into()
            } else {
                allowed[0].clone()
            }]
        } else {
            selected
        }
    }
    pub fn cancel_update(&mut self) {
        if self.update_installing {
            return;
        }
        self.update_cancel.store(true, Ordering::Relaxed);
        self.install_requested = false;
        self.install_pending = false;
        self.download_progress.bytes_per_second = 0;
        self.update_status = "已取消".into();
        if self.checking {
            self.checking = false;
            self.update_busy = false;
        }
    }
    pub fn check_update(&mut self, automatic: bool) {
        if self.update_busy || self.closing {
            return;
        }
        self.update_busy = true;
        self.checking = true;
        self.update_cancel = Arc::new(AtomicBool::new(false));
        let cancel = self.update_cancel.clone();
        self.update_status = "正在检查更新…".into();
        self.log("正在检查更新…");
        self.last_check = Instant::now();
        let source = if self.choice("download_source", "domestic") == "github" {
            "github"
        } else {
            "gitee"
        }
        .to_owned();
        self.channel.job(move |c| {
            // Serializes cancelled in-flight HTTP work without blocking the UI or exit.
            static CHECK: std::sync::Mutex<()> = std::sync::Mutex::new(());
            let _guard = loop {
                if cancel.load(Ordering::Relaxed) {
                    return Ok(());
                }
                match CHECK.try_lock() {
                    Ok(guard) => break guard,
                    Err(std::sync::TryLockError::Poisoned(e)) => break e.into_inner(),
                    Err(std::sync::TryLockError::WouldBlock) => {
                        std::thread::sleep(std::time::Duration::from_millis(50))
                    }
                }
            };
            let result = updater::check_with_cancel(&source, &cancel);
            c.send(Event::Checked(result, automatic, cancel));
            Ok(())
        });
    }
    pub fn download_update(&mut self) {
        self.page = 2;
        self.update_offer = false;
        if self.closing || self.checking || self.update_installing {
            return;
        }
        // A click also accepts an in-flight automatic prefetch or a verified cache.
        self.install_requested = true;
        if self.download.is_some() {
            self.install_pending = self.smoke.is_none();
            return;
        }
        self.start_download(true);
    }
    fn start_download(&mut self, install_requested: bool) {
        if self.update_busy || self.closing {
            return;
        }
        let Some(m) = self.update.clone() else { return };
        let preference = self.choice("download_source", "domestic");
        self.install_requested = install_requested;
        self.update_status = "正在连接下载服务器…".into();
        self.download_progress = updater::DownloadProgress {
            total: m.bytes,
            source: if preference == "github" {
                "github"
            } else {
                "gitee"
            }
            .into(),
            ..Default::default()
        };
        self.update_busy = true;
        self.update_cancel = Arc::new(AtomicBool::new(false));
        let cancel = self.update_cancel.clone();
        let data = self.data.clone();
        self.channel.job(move |c| {
            let result = updater::download_with_preference(&m, &data, &preference, &cancel, |p| {
                c.send(Event::DownloadProgress(p))
            });
            match result {
                Ok(p) => c.send(Event::Download(p)),
                Err(_) if cancel.load(Ordering::Relaxed) => {
                    c.send(Event::Log("更新下载已取消".into()))
                }
                Err(e) => c.send(Event::Warning(e.to_string())),
            }
            c.send(Event::UpdateDone);
            Ok(())
        });
    }
    pub fn install_update(&mut self) {
        if self.busy || self.critical || self.update_busy || self.closing {
            return;
        }
        let (Some(path), Some(m)) = (self.download.clone(), self.update.clone()) else {
            return;
        };
        self.install_pending = false;
        self.install_requested = false;
        self.update_busy = true;
        self.update_installing = true;
        self.update_status = "正在安装更新，完成后将自动重启…".into();
        let data = self.data.clone();
        self.channel.job(move |c| {
            match updater::install(&path, &m, &data) {
                Ok(()) => c.send(Event::Installed),
                Err(e) => {
                    c.send(Event::Warning(e.to_string()));
                    c.send(Event::UpdateDone);
                }
            }
            Ok(())
        });
    }
    pub fn events(&mut self) -> bool {
        let mut changed = false;
        while let Some(result) = self.store.as_ref().and_then(preferences::Store::try_result) {
            if result.revision < self.save_revision {
                continue;
            }
            changed = true;
            match result.result {
                Ok(()) => self.save_error = None,
                Err(e) => {
                    self.log(&format!("设置未保存：{e}"));
                    self.save_error = Some(e);
                }
            }
            if let Some(message) = result.backup_warning {
                self.log(&message);
            }
        }
        for _ in 0..128 {
            let Ok(e) = self.rx.try_recv() else { break };
            changed = true;
            match e {
                Event::Scan(r) => {
                    self.log(&format!(
                        "扫描结束：{} 个目录，{} 个候选。",
                        r.directories, r.candidates
                    ));
                    if r.skipped > 0 {
                        self.log(&format!(
                            "跳过 {0} 项；可在日志查看原因并手动添加游戏。",
                            r.skipped
                        ));
                        for detail in &r.skipped_details {
                            self.log(&format!("{}：{}", self.text(&detail.reason), detail.path));
                        }
                    }
                    self.log(&format!(
                        "{:.2} s{}",
                        r.seconds,
                        if r.cancelled { " · 已取消" } else { "" }
                    ));
                    self.merge(r.rows)
                }
                Event::Progress(p) => self.progress = p,
                Event::PayloadProgress(p) => {
                    if !p.transfer.detail.is_empty()
                        && self
                            .payload_progress
                            .as_ref()
                            .is_none_or(|old| old.transfer.detail != p.transfer.detail)
                    {
                        self.log(&p.transfer.detail);
                    }
                    self.payload_progress = Some(p);
                }
                Event::PatchResult(success) => {
                    if let Some(s) = &mut self.patch_summary {
                        if success {
                            s.succeeded += 1;
                        } else {
                            s.failed += 1;
                        }
                    }
                }
                Event::Games(g) => self.merge(g),
                Event::Statuses(epoch, s) => {
                    if epoch == self.status_epoch {
                        self.statuses.extend(
                            s.into_iter()
                                .filter(|(p, _)| self.games.iter().any(|g| &g.exe == p)),
                        );
                    }
                }
                Event::RefreshDone => {
                    self.refresh_busy = false;
                    if self.refresh_pending {
                        self.refresh_pending = false;
                        self.refresh();
                    }
                }
                Event::Status(exe, status) => {
                    if let Ok(mut m) = self.display_cache.lock() {
                        m.remove(&exe);
                    }
                    self.statuses.insert(exe, status);
                }
                Event::Evidence(exe, evidence) => {
                    if self.games.iter().any(|g| g.exe == exe) {
                        self.evidence.insert(exe, Some(evidence));
                    }
                }
                Event::PresetRead(exe, values, running, epoch) => {
                    self.preset_reads.remove(&exe);
                    if epoch != self.status_epoch || !self.games.iter().any(|g| g.exe == exe) {
                        continue;
                    }
                    if running {
                        self.running_games.insert(exe.clone());
                    } else {
                        self.running_games.remove(&exe);
                    }
                    self.disk_presets.retain(|(game, _), _| game != &exe);
                    if let Some((scheme, mut values)) = values {
                        if let Some(policy) = self.catalog.scheme_policies.get(&scheme) {
                            let context = rtx_fg_manager::presets::Context::new(
                                &scheme,
                                policy,
                                std::path::Path::new(&exe),
                            );
                            if context.normalize(&mut values) && context.delta {
                                let game = self.games.iter_mut().find(|g| g.exe == exe).unwrap();
                                if game.extra.get("delta_clamp_notified") != Some(&json!(true)) {
                                    game.extra
                                        .insert("delta_clamp_notified".into(), json!(true));
                                    self.log("三角洲专项最高支持4X，原5X/6X已调整为4X");
                                    self.save();
                                }
                            }
                        }
                        self.disk_presets.insert((exe, scheme), values);
                    }
                }
                Event::PresetApplied(exe, scheme, values) => {
                    self.disk_presets
                        .insert((exe.clone(), scheme.clone()), values.clone());
                    if let Some(game) = self.games.iter_mut().find(|g| g.exe == exe) {
                        let saved = game
                            .extra
                            .entry("preset_options_v2")
                            .or_insert_with(|| json!({}));
                        if !saved.is_object() {
                            *saved = json!({});
                        }
                        saved[&scheme] = json!(values);
                        let dirty = game
                            .extra
                            .entry("preset_dirty")
                            .or_insert_with(|| json!({}));
                        if !dirty.is_object() {
                            *dirty = json!({});
                        }
                        dirty[&scheme] = json!(false);
                        self.save();
                    }
                }
                Event::Icons(rows) => {
                    for (exe, image) in rows {
                        if let Some(mut image) = image {
                            // GPUI uploads image bytes as BGRA.
                            for pixel in image.pixels_mut() {
                                pixel.0.swap(0, 2);
                            }
                            self.icons.insert(
                                exe,
                                Some(Arc::new(gpui::RenderImage::new(vec![image::Frame::new(
                                    image,
                                )]))),
                            );
                        }
                    }
                }
                Event::Gpu(g) => {
                    self.gpu = g
                        .iter()
                        .map(|(n, sm)| format!("{n} | SM{sm}"))
                        .collect::<Vec<_>>()
                        .join(" / ");
                    if self.state.get("backend_schema").is_none()
                        && let Some((_, sm)) = g.first()
                    {
                        self.state["backend"] =
                            json!(if *sm == 75 { "native20" } else { "native30" });
                        self.state["backend_schema"] = json!(1);
                        self.save();
                    }
                }
                Event::Devices(d) => {
                    self.devices = d;
                    self.device_index = self.device_index.min(self.devices.len().saturating_sub(1));
                }
                Event::Log(s) => self.log(&s),
                Event::Notice(s) => {
                    self.log(&s);
                    self.notice = Some(s);
                }
                Event::Warning(s) => {
                    self.log(&s);
                    self.warning = Some(s)
                }
                Event::Catalog(catalog) => {
                    if !self.busy && !self.closing {
                        for (scheme, reason) in &catalog.skipped_schemes {
                            self.log(&format!("{scheme}：{reason}"));
                        }
                        self.catalog = catalog;
                        self.migrate_presets();
                        if let Some(exe) = self.focus.clone() {
                            self.inspect(exe);
                        }
                    }
                }
                Event::Payload(result, paths, token) => {
                    if Arc::ptr_eq(&token, &self.cancel) {
                        if token.load(Ordering::Relaxed) || self.closing {
                            self.channel.send(Event::Done);
                        } else {
                            match result {
                                Ok(payload) => self.run_patch(paths, Some(payload)),
                                Err(e) => {
                                    if let Some(summary) = &mut self.patch_summary {
                                        summary.failed += paths.len();
                                    }
                                    self.channel.send(Event::Warning(e.to_string()));
                                    self.channel.send(Event::Done);
                                }
                            }
                        }
                    }
                }
                Event::Done => {
                    self.busy = false;
                    self.critical = false;
                    self.progress.clear();
                    self.payload_progress = None;
                    if let Some(s) = self.patch_summary.take() {
                        self.log(&format!(
                            "操作完成：成功 {0}，失败 {1}，未处理 {2}。",
                            s.succeeded,
                            s.failed,
                            s.total.saturating_sub(s.succeeded + s.failed)
                        ));
                    }
                    self.refresh()
                }
                Event::Checked(result, automatic, token) => {
                    if !Arc::ptr_eq(&token, &self.update_cancel)
                        || token.load(Ordering::Relaxed)
                        || self.closing
                    {
                        continue;
                    }
                    self.checking = false;
                    self.update_busy = false;
                    match result {
                        Ok(m) => {
                            self.update_status = if m.is_some() {
                                "发现新版本"
                            } else {
                                "当前已是最新版本"
                            }
                            .into();
                            self.log(&self.update_status.clone());
                            if !automatic && m.is_none() {
                                self.notice = Some(self.update_status.clone());
                            }
                            let same = self
                                .update
                                .as_ref()
                                .zip(m.as_ref())
                                .is_some_and(|(a, b)| a.same_file(b));
                            self.update = m;
                            self.update_offer = self.update.is_some() && (!same || !automatic);
                            if !same {
                                self.download = None;
                                self.download_progress = Default::default();
                                self.install_pending = false;
                                self.install_requested = false;
                            }
                            if self.update.is_some()
                                && self.download.is_none()
                                && self.boolean("auto_download", false)
                            {
                                // Prefetch preserves the saved option without silently
                                // restarting users who have not accepted this update.
                                self.start_download(false);
                            }
                        }
                        Err(e) => {
                            self.update_status = format!("{}：{e}", self.text("检查更新失败"));
                            self.log(&self.update_status.clone());
                            if !automatic {
                                self.warning = Some(self.update_status.clone());
                            }
                        }
                    }
                }
                Event::Download(p) => {
                    if self.closing || self.update_cancel.load(Ordering::Relaxed) {
                        continue;
                    }
                    self.download = Some(p);
                    self.update_status = "更新下载完成，已验证签名".into();
                    self.log("更新下载完成，已验证签名");
                    self.install_pending = self.install_requested && self.smoke.is_none();
                }
                Event::DownloadProgress(p) => {
                    if !self.update_cancel.load(Ordering::Relaxed) {
                        self.update_status = p.label().into();
                        self.download_progress = p;
                    }
                }
                Event::UpdateDone => {
                    let failed_install = self.update_installing;
                    self.update_busy = false;
                    self.update_installing = false;
                    if (failed_install || self.download.is_none())
                        && !self.update_cancel.load(Ordering::Relaxed)
                    {
                        self.update_status = "下载或安装未完成，请查看操作日志".into();
                    }
                }
                Event::Installed => {
                    self.update_busy = false;
                    self.update_installing = false;
                    self.closing = true;
                }
                Event::SmokeWritten => self.close(),
                Event::Saved(result) => match result {
                    Ok(()) => self.saved = true,
                    Err(e) => {
                        self.closing = false;
                        self.warning = Some(e.to_string());
                        self.read_only = true;
                    }
                },
            }
        }
        changed
    }

    pub fn language(&mut self, code: &str) {
        self.state["language"] = json!(code);
        self.tr = i18n::Translator::new(code);
        self.save();
    }
    pub fn close(&mut self) {
        self.closing = true;
        self.install_pending = false;
        self.cancel.store(true, Ordering::Relaxed);
        self.update_cancel.store(true, Ordering::Relaxed);
    }
    pub fn tick_close(&mut self) {
        if self.closing && !self.saved && !self.critical && (!self.update_busy || self.checking) {
            if let Some(store) = self.store.take() {
                self.channel.job(move |c| {
                    c.send(Event::Saved(store.finish()));
                    Ok(())
                });
            } else if self.read_only {
                self.saved = true;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn controller() -> (tempfile::TempDir, Controller) {
        let dir = tempfile::tempdir().unwrap();
        let c = Controller::new(
            dir.path().into(),
            json!({"language":"zh-CN", "games":[], "auto_update":false}),
            None,
            Some(dir.path().join("test.json")),
        );
        (dir, c)
    }
    #[test]
    fn close_does_not_wait_for_http_or_scan_but_waits_for_mutation() {
        let (_dir, mut c) = controller();
        c.update_busy = true;
        c.checking = true;
        c.busy = true;
        c.critical = true;
        c.close();
        c.tick_close();
        assert!(c.store.is_some());
        c.critical = false;
        c.tick_close();
        assert!(c.store.is_none());
        assert!(c.cancel.load(Ordering::Relaxed));
        assert!(c.update_cancel.load(Ordering::Relaxed));
    }
    #[test]
    fn cancelled_update_cannot_publish_stale_result_or_restart_download() {
        let (_dir, mut c) = controller();
        c.state["auto_download"] = json!(true);
        c.update_busy = true;
        c.checking = true;
        let old = c.update_cancel.clone();
        c.cancel_update();
        c.channel.send(Event::Checked(Ok(None), false, old));
        c.events();
        assert_eq!(c.update_status, "已取消");
        assert!(!c.update_busy);
        assert!(c.notice.is_none());
        c.channel.send(Event::UpdateDone);
        c.events();
        assert!(!c.update_busy);
        c.close();
        c.tick_close();
    }
    #[test]
    fn manual_check_has_result_feedback() {
        let (_dir, mut c) = controller();
        c.channel
            .send(Event::Checked(Ok(None), false, c.update_cancel.clone()));
        c.events();
        assert_eq!(c.notice.as_deref(), Some("当前已是最新版本"));
        c.channel.send(Event::Checked(
            Err(anyhow::anyhow!("offline")),
            false,
            c.update_cancel.clone(),
        ));
        c.events();
        assert!(c.warning.as_ref().unwrap().contains("offline"));
        c.close();
        c.tick_close();
    }
    fn update_manifest() -> updater::Manifest {
        updater::Manifest {
            schema: 1,
            version: "4.0.10".into(),
            file: "RTXManager-v4.0.10-x64.exe".into(),
            sha256: "a".repeat(64),
            bytes: 2_000_000,
            source: "gitee".into(),
            url: String::new(),
        }
    }
    #[test]
    fn startup_offers_update_once_and_manual_check_can_reopen_it() {
        let (_dir, mut c) = controller();
        for (automatic, expected_offer) in [(true, true), (true, false), (false, true)] {
            c.channel.send(Event::Checked(
                Ok(Some(update_manifest())),
                automatic,
                c.update_cancel.clone(),
            ));
            c.events();
            assert_eq!(c.update_offer, expected_offer);
            assert!(c.notice.is_none());
            c.update_offer = false;
        }
        c.close();
        c.tick_close();
    }
    #[test]
    fn accepted_download_installs_after_completion_but_not_after_cancel_or_close() {
        let (_dir, mut c) = controller();
        // Disable only the fixture suppression; no install worker is started here.
        c.smoke = None;
        c.update = Some(update_manifest());
        c.update_busy = true;
        c.download_update();
        c.channel
            .send(Event::Download(PathBuf::from("verified-cache.exe")));
        c.channel.send(Event::UpdateDone);
        c.events();
        assert!(c.install_pending);
        assert!(!c.update_busy);
        c.busy = true;
        c.install_update();
        assert!(c.install_pending && !c.update_installing);
        c.cancel_update();
        assert!(!c.install_pending);
        c.channel
            .send(Event::Download(PathBuf::from("stale-cache.exe")));
        c.events();
        assert_eq!(c.download, Some(PathBuf::from("verified-cache.exe")));
        c.close();
        c.tick_close();
    }
    #[test]
    fn prefetch_does_not_restart_without_acceptance() {
        let (_dir, mut c) = controller();
        c.smoke = None;
        c.update = Some(update_manifest());
        c.channel
            .send(Event::Download(PathBuf::from("verified-cache.exe")));
        c.events();
        assert!(!c.install_pending);
        c.download_update();
        assert!(c.install_pending);
        c.close();
        assert!(!c.install_pending);
        c.tick_close();
    }
    #[test]
    fn failed_operation_always_completes() {
        let (tx, rx) = mpsc::channel();
        let c = Channel { tx };
        c.operation(|_| anyhow::bail!("fixture error"));
        let events = [
            rx.recv_timeout(std::time::Duration::from_secs(2)).unwrap(),
            rx.recv_timeout(std::time::Duration::from_secs(2)).unwrap(),
        ];
        assert!(events.iter().any(|e| matches!(e, Event::Done)));
        assert!(events.iter().any(|e| matches!(e, Event::Warning(_))));
    }
    #[test]
    fn only_known_undeployed_single_game_skips_remove_confirmation() {
        let (_dir, mut c) = controller();
        c.games = vec![Game {
            exe: "A.exe".into(),
            ..Default::default()
        }];
        c.focus = Some("A.exe".into());
        assert!(!c.can_forget_focused_without_confirmation());
        for status in [
            "正在检测",
            "已部署 NATIVE30 / version.dll",
            "需检查",
            "未部署",
        ] {
            c.statuses.insert("A.exe".into(), status.into());
            assert_eq!(
                c.can_forget_focused_without_confirmation(),
                status == "未部署"
            );
        }
        c.busy = true;
        assert!(!c.can_forget_focused_without_confirmation());
        c.busy = false;
        c.games.clear();
        assert!(!c.can_forget_focused_without_confirmation());
        c.close();
        c.tick_close();
    }
    #[test]
    fn focused_game_and_checked_batch_are_independent() {
        let (_dir, mut c) = controller();
        c.games = ["a", "b", "c"]
            .into_iter()
            .map(|exe| Game {
                exe: exe.into(),
                ..Default::default()
            })
            .collect();
        assert!(c.targets().is_empty());
        c.focus = Some("a".into());
        assert!(c.selected.is_empty());
        assert_eq!(c.targets(), BTreeSet::from(["a".into()]));
        c.selected = BTreeSet::from(["b".into(), "c".into(), "stale".into()]);
        assert_eq!(c.targets(), BTreeSet::from(["b".into(), "c".into()]));
        c.focus = Some("b".into());
        assert_eq!(c.targets(), BTreeSet::from(["b".into(), "c".into()]));
        c.close();
        c.tick_close();
    }
    #[test]
    fn forget_batch_preserves_patch_files_and_rejects_stale_status() {
        let (dir, mut c) = controller();
        let mut paths = BTreeSet::new();
        for name in ["a.exe", "b.exe"] {
            let p = dir.path().join(name);
            std::fs::write(&p, b"game sentinel").unwrap();
            paths.insert(p.display().to_string());
        }
        let patch = dir.path().join("version.dll");
        std::fs::write(&patch, b"patch sentinel").unwrap();
        c.games = paths
            .iter()
            .map(|exe| Game {
                exe: exe.clone(),
                ..Default::default()
            })
            .collect();
        c.selected = paths.clone();
        c.focus = paths.first().cloned();
        let old_epoch = c.status_epoch;
        c.forget(&paths);
        c.channel.send(Event::Statuses(
            old_epoch,
            paths
                .iter()
                .map(|p| (p.clone(), "已部署 NATIVE_SM86 / version.dll".into()))
                .collect(),
        ));
        c.events();
        assert!(
            c.games.is_empty()
                && c.selected.is_empty()
                && c.focus.is_none()
                && c.statuses.is_empty()
        );
        for p in paths {
            assert_eq!(std::fs::read(p).unwrap(), b"game sentinel");
        }
        assert_eq!(std::fs::read(patch).unwrap(), b"patch sentinel");
        assert_eq!(c.state["games"], json!([]));
        c.close();
        c.tick_close();
    }
    #[test]
    fn batch_confirmation_is_numbered_frozen_and_cancellable() {
        let (_dir, mut c) = controller();
        c.games = ["A.exe", "B.exe", "C.exe"]
            .into_iter()
            .map(|exe| Game {
                exe: exe.into(),
                ..Default::default()
            })
            .collect();
        c.selected = BTreeSet::from(["A.exe".into(), "B.exe".into()]);
        for clean in [false, true] {
            c.request_patch(clean);
            assert!(!c.busy && !c.critical);
            assert_eq!(
                c.confirm.as_deref(),
                Some(if clean { "clean_batch" } else { "deploy_batch" })
            );
            c.selected = BTreeSet::from(["C.exe".into()]);
            assert_eq!(c.pending_patch_list(), "1. A.exe\n\n2. B.exe");
            c.cancel_patch();
            assert!(c.pending_patch.is_none() && c.confirm.is_none());
            c.selected = BTreeSet::from(["A.exe".into(), "B.exe".into()]);
        }
        c.close();
        c.tick_close();
    }
    #[test]
    fn upstream_options_are_independent_per_game_and_validated() {
        let (_dir, mut c) = controller();
        c.games = ["A.exe", "B.exe"]
            .into_iter()
            .map(|exe| Game {
                exe: exe.into(),
                ..Default::default()
            })
            .collect();
        c.set_game_option("A.exe", "max_generated_frames", "5");
        c.set_game_option("A.exe", "preset", "B");
        c.set_game_option("A.exe", "logging_level", "2");
        c.set_game_option("A.exe", "optimized", "0");
        let a = c.preset_values("A.exe");
        assert_eq!(a["max_generated_frames"], "5");
        assert_eq!(a["preset"], "B");
        assert_eq!(a["logging_level"], "2");
        assert_eq!(a["optimized"], "0");
        assert_eq!(
            c.preset_values("B.exe"),
            rtx_fg_manager::presets::defaults("upstream035", &BTreeMap::new())
        );
        c.set_game_option("A.exe", "max_generated_frames", "99");
        assert_eq!(c.preset_values("A.exe"), a);
        c.close();
        c.tick_close();
    }
    #[test]
    fn presets_do_not_leak_between_schemes_and_retired_selections_migrate() {
        let (_dir, mut c) = controller();
        c.games = vec![Game {
            exe: "A.exe".into(),
            ..Default::default()
        }];
        c.set_game_option("A.exe", "optimized", "3");
        c.set_game_option("A.exe", "max_generated_frames", "5");
        c.state["cloud_scheme"] = json!("native-0.2.6-experimental");
        assert_eq!(c.cloud_scheme(), "native-0.2.6-stable");
        assert!(!c.preset_values("A.exe").contains_key("optimized"));
        c.set_game_option("A.exe", "max_generated_frames", "2");
        c.state["cloud_scheme"] = json!("initial-rtx20");
        assert_eq!(c.cloud_scheme(), "initial");
        assert_eq!(c.preset_values("A.exe")["max_generated_frames"], "3");
        c.state["proxies"] = json!(["winhttp.dll", "version.dll"]);
        assert_eq!(c.proxies(), vec!["version.dll"]);
        c.state["cloud_scheme"] = json!("upstream-0.3.1-310-9");
        assert_eq!(c.cloud_scheme(), c.catalog.default_scheme);
        assert_eq!(c.preset_values("A.exe")["optimized"], "3");
        assert_eq!(c.preset_values("A.exe")["max_generated_frames"], "5");
        c.set_game_option("A.exe", "reset", "");
        assert_eq!(c.preset_values("A.exe")["optimized"], "1");
        c.close();
        c.tick_close();
    }
    #[test]
    fn same_protocol_schemes_migrate_once_and_keep_dirty_values() {
        let (_dir, mut c) = controller();
        let original = c.catalog.default_scheme.clone();
        let extended = "rtxfg-0.3.5-dx12-vulkan";
        let mut policy = c.catalog.scheme_policies[&original].clone();
        policy
            .capabilities
            .insert(rtx_fg_manager::delta::CAPABILITY.into());
        c.catalog.scheme_policies.insert(extended.into(), policy);
        let mut p = c.catalog.packages[0].clone();
        p.id = "delta-test-version".into();
        p.scheme_id = extended.into();
        c.catalog.packages.push(p);
        let exe = "C:/Games/DeltaForce/Binaries/Win64/DeltaForceClient-Win64-Shipping.exe";
        c.games = vec![Game {
            exe: exe.into(),
            extra: [(
                "preset_options".into(),
                json!({"upstream035":{"max_generated_frames":"5","optimized":"2"}}),
            )]
            .into_iter()
            .collect(),
            ..Default::default()
        }];
        c.migrate_presets();
        assert!(c.notice.is_some());
        c.notice = None;
        c.migrate_presets();
        assert!(c.notice.is_none());
        assert_eq!(c.preset_values(exe)["max_generated_frames"], "5");
        c.state["cloud_scheme"] = json!(extended);
        assert_eq!(c.preset_values(exe)["max_generated_frames"], "3");
        c.set_game_option(exe, "max_generated_frames", "2");
        c.channel.send(Event::PresetRead(
            exe.into(),
            Some((
                extended.into(),
                BTreeMap::from([("max_generated_frames".into(), "1".into())]),
            )),
            false,
            c.status_epoch,
        ));
        c.events();
        assert_eq!(c.preset_values(exe)["max_generated_frames"], "2");
        c.state["cloud_scheme"] = json!(original);
        assert_eq!(c.preset_values(exe)["max_generated_frames"], "5");
        c.state["cloud_scheme"] = json!(extended);
        c.channel.send(Event::PresetApplied(
            exe.into(),
            extended.into(),
            c.preset_values(exe),
        ));
        c.events();
        c.channel.send(Event::PresetRead(
            exe.into(),
            Some((
                extended.into(),
                BTreeMap::from([("max_generated_frames".into(), "1".into())]),
            )),
            false,
            c.status_epoch,
        ));
        c.events();
        assert_eq!(c.preset_values(exe)["max_generated_frames"], "1");
        c.running_games.insert(exe.into());
        c.set_game_option(exe, "max_generated_frames", "3");
        assert_eq!(c.preset_values(exe)["max_generated_frames"], "1");
        c.close();
        c.tick_close();
    }

    #[test]
    fn parameter_reads_cannot_cross_a_mutation_and_replace_its_applied_values() {
        let (directory, mut c) = controller();
        let exe = directory
            .path()
            .join("game.exe")
            .to_string_lossy()
            .into_owned();
        let scheme = c.cloud_scheme();
        c.games = vec![Game {
            exe: exe.clone(),
            ..Default::default()
        }];
        let mut applied = c.preset_values(&exe);
        applied.insert("max_generated_frames".into(), "2".into());
        let old_epoch = c.status_epoch;
        c.preset_reads.insert(exe.clone()); // A read issued before the write.
        c.status_epoch += 1; // apply_focused_parameters/perform invalidate it.
        c.busy = true;
        c.channel.send(Event::PresetApplied(
            exe.clone(),
            scheme.clone(),
            applied.clone(),
        ));
        c.channel.send(Event::PresetRead(
            exe.clone(),
            Some((
                scheme.clone(),
                BTreeMap::from([("max_generated_frames".into(), "1".into())]),
            )),
            false,
            old_epoch,
        ));
        c.events();
        assert_eq!(c.disk_presets[&(exe.clone(), scheme)], applied);
        assert!(!c.preset_reads.contains(&exe));
        c.inspect(exe.clone()); // Clicking a game while the write is still busy.
        assert!(!c.preset_reads.contains(&exe));
        c.busy = false;
        c.inspect(exe.clone());
        assert!(c.preset_reads.contains(&exe));
        c.close();
        c.tick_close();
    }
}
