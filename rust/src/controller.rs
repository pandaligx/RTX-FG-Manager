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
    Deployments(u64, String, rtx_fg_manager::deployment::Snapshot),
    Scan(scanner::Report),
    Progress(String),
    PayloadProgress(cloud::CloudProgress),
    PatchResult(bool),
    Games(Vec<Game>),
    CustomFolder(String, String),
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
    PresetReadFailed(String, String, bool, u64),
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
type DisplayCache = Arc<
    Mutex<
        BTreeMap<
            String,
            (
                Vec<(u64, u64)>,
                Instant,
                rtx_fg_manager::deployment::Snapshot,
            ),
        >,
    >,
>;

// The active data directory can differ from LOCALAPPDATA after --data-dir or
// UAC elevation. Write guards must protect the actual controller directory.
fn ensure_directory_outside_data(
    directory: &std::path::Path,
    data: &std::path::Path,
) -> Result<()> {
    let directory = core::no_links(directory)?;
    let data = core::no_links(data)?;
    anyhow::ensure!(
        !core::within(&directory, &data),
        "不能部署到管理器缓存或设置目录"
    );
    Ok(())
}

// Display-only metadata cache. All writes and ownership checks use core's full validation.
fn parameter_target(game: &Game) -> core::DeploymentTarget {
    // A library card may be a launcher in a different directory. Settings are
    // represented by the first real deployment, just as installation is.
    let paths = game.deployment_executables();
    game.deployment_target(std::path::Path::new(paths.first().unwrap_or(&game.exe)))
}

fn uses_explicit_edits(profile: &str) -> bool {
    matches!(
        profile,
        rtx_fg_manager::encore::PROFILE | rtx_fg_manager::rtxmfg::PROFILE
    )
}

fn read_json_preset_values(
    target: &core::DeploymentTarget,
    profile: &str,
) -> Result<rtx_fg_manager::presets::Values> {
    let config = match profile {
        rtx_fg_manager::encore::PROFILE => rtx_fg_manager::encore::CONFIG,
        rtx_fg_manager::rtxmfg::PROFILE => rtx_fg_manager::rtxmfg::CONFIG,
        _ => anyhow::bail!("不支持的配置协议"),
    };
    let path = core::no_links(&target.directory.join(config))?;
    anyhow::ensure!(path.metadata()?.len() <= 1024 * 1024, "配置文件过大");
    let bytes = std::fs::read(path)?;
    if profile == rtx_fg_manager::encore::PROFILE {
        rtx_fg_manager::encore::read(&bytes)
    } else {
        rtx_fg_manager::rtxmfg::read(&bytes)
    }
}

fn display_stamp(target: &core::DeploymentTarget) -> Result<Vec<(u64, u64)>> {
    use std::os::windows::fs::MetadataExt;
    let exe = core::no_links(&target.game_exe)?;
    let dir = &target.directory;
    let mut paths = vec![exe.clone(), dir.join(core::OWN).join(core::MARKER)];
    paths.extend(core::PROXIES.iter().map(|n| dir.join(n)));
    paths.extend(
        [
            core::INI,
            rtx_fg_manager::rtxmfg::CONFIG,
            rtx_fg_manager::transfusion::CONFIG,
            rtx_fg_manager::encore::CONFIG,
            rtx_fg_manager::encore::NOTICES,
            ".rtx-fg-v3/encore-upgrade.json",
            ".rtx-fg-v3/rtxmfg-upgrade.json",
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
    pending_catalog: Option<cloud::Catalog>,
    pub data: PathBuf,
    pub state: Value,
    pub store: Option<preferences::Store>,
    pub read_only: bool,
    pub tr: i18n::Translator,
    pub channel: Channel,
    pub rx: mpsc::Receiver<Event>,
    pub games: Vec<Game>,
    /// Changes whenever library contents or ordering can invalidate search indices.
    pub library_revision: u64,
    pub selected: BTreeSet<String>,
    pub focus: Option<String>,
    pub statuses: BTreeMap<String, String>,
    pub deployments: BTreeMap<String, rtx_fg_manager::deployment::Snapshot>,
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
            pending_catalog: None,
            data: data.clone(),
            state,
            store: (!read_only).then(|| preferences::Store::new(data)),
            read_only,
            tr,
            channel,
            rx,
            games,
            library_revision: 0,
            selected: BTreeSet::new(),
            focus: None,
            statuses: BTreeMap::new(),
            deployments: BTreeMap::new(),
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
        self.scheme_for(self.focus.as_deref())
    }
    pub fn scheme_for(&self, exe: Option<&str>) -> String {
        let requested = exe
            .and_then(|exe| self.games.iter().find(|g| g.exe == exe))
            .and_then(|g| g.extra.get("deployment_choice"))
            .and_then(|v| v.get("scheme"))
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| {
                exe.and_then(|e| self.deployments.get(e))
                    .and_then(|d| d.common.as_ref())
                    .map(|d| d.0.clone())
            })
            .unwrap_or_else(|| self.choice("cloud_scheme", &self.catalog.default_scheme));
        let requested = if requested.starts_with("initial-") {
            "initial".into()
        } else if requested.contains("experimental") {
            "native-0.2.6-stable".into()
        } else {
            requested
        };
        let id = self.catalog.selected(&requested).scheme_id.clone();
        let series = self.series_for(exe);
        if self.catalog.supports_series(&id, series) {
            id
        } else {
            self.compatible_scheme(exe, series)
        }
    }
    pub fn cloud_series(&self) -> usize {
        self.series_for(self.focus.as_deref())
    }
    pub fn series_for(&self, exe: Option<&str>) -> usize {
        if let Some(series) = exe
            .and_then(|e| self.games.iter().find(|g| g.exe == e))
            .and_then(|g| g.extra.get("deployment_choice"))
            .and_then(|v| v.get("series"))
            .and_then(Value::as_u64)
            .filter(|n| *n <= 2)
        {
            return series as usize;
        }
        if let Some(series) = exe
            .and_then(|e| self.deployments.get(e))
            .and_then(|d| d.common.as_ref())
            .and_then(|d| d.2)
            .filter(|n| *n <= 2)
        {
            return series;
        }
        if let Some(series) = self.state["cloud_series"].as_u64()
            && series <= 2
        {
            return series as usize;
        }
        if self.choice("backend", "native30").ends_with("20") {
            0
        } else {
            1
        }
    }
    fn compatible_scheme(&self, exe: Option<&str>, series: usize) -> String {
        let remembered = exe
            .and_then(|e| self.games.iter().find(|g| g.exe == e))
            .and_then(|g| g.extra.get("series_schemes"))
            .or_else(|| self.state.get("series_schemes"))
            .and_then(|v| v.get(series.to_string()))
            .and_then(Value::as_str);
        remembered
            .filter(|id| self.catalog.supports_series(id, series))
            .map(str::to_owned)
            .or_else(|| {
                self.catalog
                    .supports_series(&self.catalog.default_scheme, series)
                    .then(|| self.catalog.default_scheme.clone())
            })
            .or_else(|| {
                self.catalog
                    .schemes()
                    .iter()
                    .find(|s| self.catalog.supports_series(&s.scheme_id, series))
                    .map(|p| p.scheme_id.clone())
            })
            .unwrap_or_else(|| self.catalog.default_scheme.clone())
    }
    pub fn select_series(&mut self, series: usize) {
        if self.busy || series > 2 {
            return;
        }
        let old = self.cloud_scheme();
        let scheme = if self.catalog.supports_series(&old, series) {
            old.clone()
        } else {
            self.compatible_scheme(self.focus.as_deref(), series)
        };
        let proxies = self.catalog.proxies(&scheme);
        let first = if proxies.iter().any(|p| p == "version.dll") {
            "version.dll".into()
        } else {
            proxies.first().cloned().unwrap_or_default()
        };
        let selected = if scheme == old {
            self.proxies()
        } else {
            vec![first]
        };
        self.set_selection(scheme.clone(), series, selected);
        if old != scheme {
            self.log("已切换到此显卡系列的兼容方案；尚未修改游戏文件。");
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
        self.preset_values_for(exe, &self.scheme_for(Some(exe)))
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
        if profile == "native026" && self.series_for(Some(exe)) == 0 {
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
        if (!dirty || uses_explicit_edits(profile))
            && let Some(disk) = self.disk_presets.get(&(exe.into(), scheme.clone()))
        {
            values.extend(disk.clone());
            if uses_explicit_edits(profile) {
                values.extend(self.explicit_preset_values(exe, &scheme));
            }
        }
        rtx_fg_manager::presets::Context::new(&scheme, policy, std::path::Path::new(exe))
            .normalize(&mut values);
        if profile == "native026" && self.series_for(Some(exe)) == 0 {
            values.insert("hardware_bilinear".into(), "0".into());
        }
        values
    }
    /// Saved snapshots are not edits. In particular, game-menu changes must win
    /// over an old manager snapshot for every field the user has not edited.
    fn explicit_preset_values(&self, exe: &str, scheme: &str) -> rtx_fg_manager::presets::Values {
        let Some(game) = self.games.iter().find(|g| g.exe == exe) else {
            return Default::default();
        };
        let saved = game
            .extra
            .get("preset_options_v2")
            .and_then(|v| v.get(scheme));
        let keys = game
            .extra
            .get("preset_dirty_keys")
            .and_then(|v| v.get(scheme));
        let legacy_dirty = game
            .extra
            .get("preset_dirty")
            .and_then(|v| v.get(scheme))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let names: Vec<&str> = if let Some(keys) = keys.and_then(Value::as_array) {
            keys.iter().filter_map(Value::as_str).collect()
        } else if legacy_dirty
            && self.catalog.scheme_policies[scheme].parameter_profile
                == rtx_fg_manager::rtxmfg::PROFILE
        {
            vec![
                "rtx_mode",
                "rtx_target",
                "rtx_preset",
                "rtx_vsync",
                "rtx_reflex_limit",
            ]
        } else if legacy_dirty {
            vec!["tf_mode", "tf_target", "tf_dynamic56", "tf_overlay"]
        } else {
            Vec::new()
        };
        names
            .into_iter()
            .filter_map(|key| {
                let value = saved?.get(key)?.as_str()?;
                let item = BTreeMap::from([(key.to_owned(), value.to_owned())]);
                rtx_fg_manager::presets::validate(
                    &self.catalog.scheme_policies[scheme].parameter_profile,
                    &item,
                )
                .ok()
                .map(|()| (key.to_owned(), value.to_owned()))
            })
            .collect()
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
        let scheme = self.scheme_for(Some(exe));
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
        let scheme = self.scheme_for(Some(exe));
        let profile = self.catalog.scheme_policies[&scheme]
            .parameter_profile
            .clone();
        let mut values = self.preset_values(exe);
        if key == "reset" {
            values = rtx_fg_manager::presets::defaults(
                &profile,
                &self.catalog.scheme_policies[&scheme].defaults,
            );
        } else {
            values.insert(key.into(), value.into());
        }
        self.preset_context(exe).normalize(&mut values);
        if rtx_fg_manager::presets::validate(&profile, &values).is_err() {
            return;
        }
        let changed = self.disk_presets.get(&(exe.to_owned(), scheme.clone())) != Some(&values);
        let mut dirty_keys: BTreeSet<String> = self
            .explicit_preset_values(exe, &scheme)
            .into_keys()
            .collect();
        if uses_explicit_edits(&profile) {
            if key == "reset" {
                dirty_keys.extend(values.keys().cloned());
            } else {
                dirty_keys.insert(key.into());
            }
            if let Some(disk) = self.disk_presets.get(&(exe.to_owned(), scheme.clone())) {
                dirty_keys.retain(|key| disk.get(key) != values.get(key));
            }
        }
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
            if uses_explicit_edits(&profile) {
                dirty[&scheme] = json!(!dirty_keys.is_empty());
                let keys = game
                    .extra
                    .entry("preset_dirty_keys")
                    .or_insert_with(|| json!({}));
                if !keys.is_object() {
                    *keys = json!({});
                }
                keys[&scheme] = json!(dirty_keys);
            }
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
            .and_then(|v| v.get(self.scheme_for(Some(exe))))
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }
    pub fn can_apply_parameters(&self) -> bool {
        self.focus.as_ref().is_some_and(|exe| {
            self.deployments
                .get(exe)
                .is_some_and(|d| d.can_apply(&self.cloud_scheme()))
        })
    }
    pub fn set_selection(&mut self, scheme: String, series: usize, proxies: Vec<String>) {
        if self.busy || self.read_only {
            return;
        }
        if !self.catalog.supports_series(&scheme, series) {
            return;
        }
        if let Some(game) = self
            .focus
            .as_ref()
            .and_then(|e| self.games.iter_mut().find(|g| &g.exe == e))
        {
            let mut choices = game
                .extra
                .get("proxy_choices")
                .filter(|v| v.is_object())
                .cloned()
                .unwrap_or(json!({}));
            choices[&scheme] = json!(proxies);
            game.extra.insert("proxy_choices".into(), choices);
            let history = game
                .extra
                .entry("series_schemes")
                .or_insert_with(|| json!({}));
            if !history.is_object() {
                *history = json!({});
            }
            history[series.to_string()] = json!(scheme);
            game.extra.insert(
                "deployment_choice".into(),
                json!({"scheme":scheme,"series":series}),
            );
        } else {
            if !self.state["series_schemes"].is_object() {
                self.state["series_schemes"] = json!({});
            }
            self.state["series_schemes"][series.to_string()] = json!(scheme);
            self.state["cloud_scheme"] = json!(scheme);
            self.state["cloud_series"] = json!(series);
            self.state["proxies"] = json!(proxies);
        }
        self.save();
    }
    pub fn apply_focused_parameters(&mut self) {
        if self.busy || self.closing || self.read_only {
            return;
        }
        let Some(exe) = self.focus.clone() else {
            return;
        };
        let Some(game) = self.games.iter().find(|g| g.exe == exe).cloned() else {
            return;
        };
        let paths = game.deployment_executables();
        let scheme = self.cloud_scheme();
        let policy = self.catalog.scheme_policies[&scheme].clone();
        let values = if uses_explicit_edits(&policy.parameter_profile) {
            self.explicit_preset_values(&exe, &scheme)
        } else {
            self.preset_values(&exe)
        };
        let data = self.data.clone();
        self.busy = true;
        self.critical = true;
        self.status_epoch += 1;
        self.progress = "正在应用参数…".into();
        self.channel.operation(move |c| {
            let mut unavailable = Vec::new();
            for path in &paths {
                let target = std::path::Path::new(path);
                let location = game.deployment_target(target);
                if let Err(error) = ensure_directory_outside_data(&location.directory, &data) {
                    unavailable.push(format!("{path}：{error}"));
                    continue;
                }
                let status = core::status_at(&location);
                if !status.starts_with("已部署") {
                    unavailable.push(format!("{path}：{status}"));
                    continue;
                }
                match Some(core::record(&location.directory)) {
                    Some(Ok(Some(record)))
                        if record.scheme_id.as_deref() == Some(scheme.as_str())
                            && core::backend_matches_profile(
                                &record.backend,
                                &policy.parameter_profile,
                            ) => {}
                    Some(Ok(_)) => unavailable.push(format!("{path}：已部署方案不同")),
                    Some(Err(error)) => unavailable.push(format!("{path}：{error}")),
                    None => unavailable.push(format!("{path}：游戏目录无效")),
                }
            }
            if !unavailable.is_empty() {
                c.send(Event::Warning(format!(
                    "游戏目录的部署状态或方案不一致，参数未应用：{}",
                    unavailable.join("；")
                )));
                return Ok(());
            }
            for path in &paths {
                if let Err(e) =
                    core::assert_target_stopped(&game.deployment_target(std::path::Path::new(path)))
                {
                    c.send(Event::Warning(e.to_string()));
                    return Ok(());
                }
            }
            let mut errors = Vec::new();
            let mut applied = 0;
            let total = paths.len();
            for path in paths {
                let target = std::path::Path::new(&path);
                let context = rtx_fg_manager::presets::Context::new(&scheme, &policy, target);
                let location = game.deployment_target(target);
                let result = ensure_directory_outside_data(&location.directory, &data)
                    .and_then(|()| core::apply_parameters_at(&location, &context, &values));
                if let Err(e) = result {
                    errors.push(format!("{}：{e}", target.display()));
                } else {
                    applied += 1;
                }
                c.send(Event::Status(
                    path.clone(),
                    core::status_at(&game.deployment_target(target)),
                ));
            }
            if errors.is_empty() {
                let actual = if uses_explicit_edits(&policy.parameter_profile) {
                    read_json_preset_values(&parameter_target(&game), &policy.parameter_profile)
                } else {
                    Ok(values)
                };
                match actual {
                    Ok(actual) => {
                        c.send(Event::PresetApplied(exe.clone(), scheme, actual));
                        c.send(Event::Log("参数已应用；下次启动游戏生效。".into()));
                    }
                    Err(error) => c.send(Event::Warning(format!(
                        "参数写入后读取失败：{}：{error}",
                        parameter_target(&game).directory.display()
                    ))),
                }
            } else {
                c.send(Event::Warning(format!(
                    "参数仅应用于 {applied}/{total} 个目录；未完成项：\n{}",
                    errors.join("\n\n")
                )));
            }
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
        let catalog = self.catalog.clone();
        self.channel.job(move |c| {
            let _completion = RefreshCompletion(c.clone());
            for g in games {
                let stamp = std::iter::once(&g.exe)
                    .chain(g.targets.iter())
                    .chain(g.cleanup_only.iter())
                    .map(|e| display_stamp(&g.deployment_target(std::path::Path::new(e))))
                    .collect::<Result<Vec<_>>>()
                    .ok()
                    .map(|v| v.into_iter().flatten().collect::<Vec<_>>());
                let cached = cache.lock().ok().and_then(|m| {
                    m.get(&g.exe)
                        .filter(|(old, at, _)| {
                            Some(old) == stamp.as_ref() && at.elapsed() < Duration::from_secs(30)
                        })
                        .map(|(_, _, s)| s.clone())
                });
                let cache_hit = cached.is_some();
                let snapshot =
                    cached.unwrap_or_else(|| rtx_fg_manager::deployment::inspect(&g, &catalog));
                if !cache_hit
                    && let Some(stamp) = stamp
                    && let Ok(mut m) = cache.lock()
                {
                    m.insert(g.exe.clone(), (stamp, Instant::now(), snapshot.clone()));
                    if m.len() > 10000 {
                        m.clear();
                    }
                }
                let status = snapshot.status.clone();
                c.send(Event::Deployments(epoch, g.exe.clone(), snapshot));
                c.send(Event::Statuses(epoch, vec![(g.exe, status)]));
            }
            Ok(())
        })
    }
    pub fn merge(&mut self, rows: Vec<Game>) {
        let before = self.games.len();
        for g in rows {
            if self.games.len() >= 10000 {
                break;
            }
            let path = std::path::Path::new(&g.exe);
            let same_directory = g.targets.is_empty()
                && path.is_absolute()
                && path.parent().is_some_and(|dir| {
                    let dir = core::key(dir);
                    self.games.iter().any(|existing| {
                        existing
                            .targets
                            .iter()
                            .chain(std::iter::once(&existing.exe))
                            .filter_map(|target| std::path::Path::new(target).parent())
                            .any(|target_dir| core::key(target_dir) == dir)
                    })
                });
            if self.games.iter().any(|existing| {
                existing.exe.eq_ignore_ascii_case(&g.exe)
                    || existing
                        .targets
                        .iter()
                        .any(|target| target.eq_ignore_ascii_case(&g.exe))
            }) || same_directory
            {
                continue;
            }
            self.games.push(g);
        }
        if self.games.len() != before {
            self.library_revision += 1;
        }
        self.save();
        self.refresh();
    }
    /// Refresh scanned installations without discarding a previously deployed path.
    pub fn merge_scan(&mut self, rows: Vec<Game>) {
        // A same-count rescan can reorder entries and replace their title/root.
        self.library_revision += 1;
        // In-flight reads and status checks refer to the old library identities.
        self.status_epoch += 1;
        let obsolete = self
            .games
            .iter()
            .filter(|old| {
                !old.reasons.is_empty()
                    && (scanner::is_steam_client_binary(std::path::Path::new(&old.exe))
                        || scanner::is_gpu_diagnostic_tool(std::path::Path::new(&old.exe)))
                    && core::status_at(&old.deployment_target(std::path::Path::new(&old.exe)))
                        == "未部署"
            })
            .map(|old| old.exe.clone())
            .collect::<BTreeSet<_>>();
        self.games.retain(|old| !obsolete.contains(&old.exe));
        self.selected.retain(|exe| !obsolete.contains(exe));
        if self
            .focus
            .as_ref()
            .is_some_and(|exe| obsolete.contains(exe))
        {
            self.focus = None;
        }
        for exe in &obsolete {
            self.statuses.remove(exe);
            self.icons.remove(exe);
            self.evidence.remove(exe);
            self.preset_reads.remove(exe);
            self.running_games.remove(exe);
            self.disk_presets.retain(|(old, _), _| old != exe);
        }
        for mut fresh in rows {
            if self.games.iter().any(|g| {
                !g.deployment_dir.is_empty()
                    && core::key(&scanner::installation_root(std::path::Path::new(&g.exe)))
                        == core::key(&scanner::installation_root(std::path::Path::new(
                            &fresh.exe,
                        )))
            }) {
                continue;
            }
            let root = core::key(std::path::Path::new(&fresh.root));
            let matched = self
                .games
                .iter()
                .filter(|old| {
                    core::key(&scanner::installation_root(std::path::Path::new(&old.exe))) == root
                })
                .cloned()
                .collect::<Vec<_>>();
            if matched.is_empty() && self.games.len() >= 10000 {
                break;
            }
            let target_dirs = fresh
                .targets
                .iter()
                .filter_map(|p| std::path::Path::new(p).parent())
                .map(core::key)
                .collect::<BTreeSet<_>>();
            if let Some(chosen) = matched
                .iter()
                .find(|old| {
                    self.focus.as_ref() == Some(&old.exe)
                        && fresh
                            .targets
                            .iter()
                            .any(|p| p.eq_ignore_ascii_case(&old.exe))
                })
                .or_else(|| {
                    matched.iter().find(|old| {
                        self.selected.contains(&old.exe)
                            && fresh
                                .targets
                                .iter()
                                .any(|p| p.eq_ignore_ascii_case(&old.exe))
                    })
                })
            {
                fresh.exe = chosen.exe.clone();
            }
            for old in &matched {
                for path in old
                    .cleanup_only
                    .iter()
                    .chain(old.targets.iter())
                    .chain(std::iter::once(&old.exe))
                {
                    let p = std::path::Path::new(path);
                    let Some(dir) = p.parent() else { continue };
                    // A former card may have covered multiple independent games
                    // inside one Steam install. Never transfer its other project's
                    // deployment to this card's uninstall list.
                    if core::key(&scanner::installation_root(p)) != root {
                        continue;
                    }
                    if target_dirs.contains(&core::key(dir)) {
                        continue;
                    }
                    // An old launcher deployment remains reachable for uninstall, but
                    // is not silently reinstalled into that unverified location.
                    if (dir.join(core::OWN).exists() || dir.join(core::INI).exists())
                        && !fresh
                            .cleanup_only
                            .iter()
                            .any(|old| old.eq_ignore_ascii_case(path))
                    {
                        fresh.cleanup_only.push(path.clone());
                    }
                }
            }
            // Keep the focused game's parameters first, then any settings that
            // existed only on a second entry in this installation.
            let mut preference_order = matched.iter().collect::<Vec<_>>();
            preference_order.sort_by_key(|old| {
                if self.focus.as_ref() == Some(&old.exe) {
                    0
                } else if old.exe.eq_ignore_ascii_case(&fresh.exe) {
                    1
                } else if !old.extra.is_empty() {
                    2
                } else {
                    3
                }
            });
            for old in &preference_order {
                for (key, value) in &old.extra {
                    if let (Some(Value::Object(existing)), Value::Object(saved)) =
                        (fresh.extra.get_mut(key), value)
                    {
                        for (scheme, options) in saved {
                            existing
                                .entry(scheme.clone())
                                .or_insert_with(|| options.clone());
                        }
                    } else {
                        fresh
                            .extra
                            .entry(key.clone())
                            .or_insert_with(|| value.clone());
                    }
                }
            }
            for old in &preference_order {
                let presets = self
                    .disk_presets
                    .iter()
                    .filter(|((exe, _), _)| exe == &old.exe)
                    .map(|((_, scheme), values)| (scheme.clone(), values.clone()))
                    .collect::<Vec<_>>();
                for (scheme, values) in presets {
                    self.disk_presets
                        .entry((fresh.exe.clone(), scheme))
                        .or_insert(values);
                }
            }
            let old_paths = matched
                .iter()
                .map(|g| core::key(std::path::Path::new(&g.exe)))
                .collect::<BTreeSet<_>>();
            self.games
                .retain(|g| !old_paths.contains(&core::key(std::path::Path::new(&g.exe))));
            for old in &matched {
                if self.focus.as_ref() == Some(&old.exe) {
                    self.focus = Some(fresh.exe.clone());
                }
                if self.selected.remove(&old.exe) {
                    self.selected.insert(fresh.exe.clone());
                }
                self.statuses.remove(&old.exe);
                self.icons.remove(&old.exe);
                self.evidence.remove(&old.exe);
                self.preset_reads.remove(&old.exe);
                self.running_games.remove(&old.exe);
                if old.exe != fresh.exe {
                    self.disk_presets.retain(|(exe, _), _| exe != &old.exe);
                }
            }
            self.games.push(fresh);
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
    pub fn pick_custom_folder(&mut self) {
        if self.busy || self.read_only {
            return;
        }
        let existing = self
            .focus
            .as_ref()
            .and_then(|e| self.games.iter().find(|g| &g.exe == e))
            .cloned();
        let games = self.games.clone();
        let data = self.data.clone();
        let mut title = self.text("选择自定义部署目录（例如 OptiScaler 插件目录）");
        if let Some(game) = &existing {
            title.push_str(&format!(
                " · {}",
                std::path::Path::new(&game.exe)
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
            ));
        }
        let exe_title = self.text("选择关联的游戏本体 EXE");
        self.busy = true;
        self.channel.operation(move |c| {
            let Some(folder) = rfd::FileDialog::new().set_title(title).pick_folder() else {
                return Ok(());
            };
            ensure_directory_outside_data(&folder, &data)?;
            let game = if let Some(g) = existing {
                g
            } else {
                let Some(exe) = rfd::FileDialog::new()
                    .set_title(exe_title)
                    .add_filter("Windows EXE", &["exe"])
                    .pick_file()
                else {
                    return Ok(());
                };
                let exe = core::library_location(&exe)?;
                games
                    .iter()
                    .find(|g| core::key(std::path::Path::new(&g.exe)) == core::key(&exe))
                    .cloned()
                    .unwrap_or_else(|| Game {
                        exe: exe.display().to_string(),
                        root: scanner::game_root(&exe).display().to_string(),
                        ..Default::default()
                    })
            };
            let target = core::DeploymentTarget::custom(std::path::Path::new(&game.exe), &folder)
                .validate(false)?;
            core::assert_target_stopped(&target)?;
            if let Some(record) = core::record(&target.directory)? {
                target.validate_record(&record)?;
            }
            anyhow::ensure!(
                !games.iter().any(|g| g.exe != game.exe
                    && g.deployment_executables()
                        .iter()
                        .chain(g.cleanup_only.iter())
                        .any(|e| core::key(
                            &g.deployment_target(std::path::Path::new(e)).directory
                        ) == core::key(&target.directory))),
                "此目录已关联其他游戏"
            );
            if core::key(&game.deployment_directory()) == core::key(&target.directory)
                && games.iter().any(|g| g.exe == game.exe)
            {
                return Ok(());
            } else {
                for e in game
                    .deployment_executables()
                    .iter()
                    .chain(game.cleanup_only.iter())
                {
                    anyhow::ensure!(
                        core::status_at(&game.deployment_target(std::path::Path::new(e)))
                            == "未部署",
                        "请先卸载原目录补丁，再更改部署目录。"
                    );
                }
            }
            c.send(Event::CustomFolder(
                game.exe,
                if target.is_custom() {
                    target.directory.display().to_string()
                } else {
                    String::new()
                },
            ));
            Ok(())
        });
    }
    pub fn request_patch(&mut self, clean: bool) {
        if self.closing || self.busy || self.pending_patch.is_some() || (self.read_only && !clean) {
            return;
        }
        let targets = self.patch_targets(clean);
        if self.target_games().len() > 1 {
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
        self.critical = true;
        self.busy = true;
        self.patch_summary = Some(PatchSummary {
            total: paths.len(),
            ..Default::default()
        });
        let mut jobs = Vec::new();
        for game in &self.games {
            let targets: Vec<String> = paths
                .iter()
                .filter(|p| {
                    **p == game.exe
                        || game.targets.contains(p)
                        || (clean && game.cleanup_only.contains(p))
                })
                .cloned()
                .collect();
            if !targets.is_empty() {
                let scheme = self.scheme_for(Some(&game.exe));
                jobs.push((
                    game.exe.clone(),
                    targets,
                    game.clone(),
                    scheme.clone(),
                    self.series_for(Some(&game.exe)),
                    self.proxies_for(Some(&game.exe)),
                    self.preset_values_for(&game.exe, &scheme),
                    self.explicit_preset_values(&game.exe, &scheme),
                ));
            }
        }
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
        let mut catalog = self.catalog.clone();
        let data = self.data.clone();
        catalog.prefer_github = self.choice("download_source", "domestic") == "github";
        self.channel.operation(move |c| {
            let mut errors = Vec::new();
            for (primary, targets, game, scheme, series, proxies, options, overrides) in jobs {
                if cancel.load(Ordering::Relaxed) {
                    break;
                }
                let prepared = if clean {
                    Ok(None)
                } else {
                    (|| {
                        for exe in &targets {
                            ensure_directory_outside_data(
                                &game.deployment_target(std::path::Path::new(exe)).directory,
                                &data,
                            )?;
                            core::preflight_install_at(
                                &game.deployment_target(std::path::Path::new(exe)),
                                &proxies,
                            )?;
                        }
                        cloud::prepare_with_progress(
                            &catalog,
                            &scheme,
                            series,
                            &proxies,
                            &cancel,
                            |p| c.send(Event::PayloadProgress(p)),
                        )
                        .map(Some)
                    })()
                };
                let prepared = match prepared {
                    Ok(p) => p,
                    Err(e) => {
                        errors.push(format!("{primary}：{e}"));
                        for exe in targets {
                            c.send(Event::PatchResult(false));
                            c.send(Event::Status(
                                exe.clone(),
                                core::status_at(
                                    &game.deployment_target(std::path::Path::new(&exe)),
                                ),
                            ));
                        }
                        continue;
                    }
                };
                let mut all_ok = true;
                let read_target = targets
                    .first()
                    .map(|exe| game.deployment_target(std::path::Path::new(exe)));
                for exe in targets {
                    if cancel.load(Ordering::Relaxed) {
                        all_ok = false;
                        break;
                    }
                    let p = PathBuf::from(&exe);
                    let result = if let Some(ref payload) = prepared {
                        (|| {
                            ensure_directory_outside_data(
                                &game.deployment_target(&p).directory,
                                &data,
                            )?;
                            let mut files = payload.files.clone();
                            let context = rtx_fg_manager::presets::Context::new(
                                &payload.scheme_id,
                                &payload.policy,
                                &p,
                            );
                            let name = core::config_name(&payload.backend);
                            files.insert(name.into(), context.configure(&files[name], &options)?);
                            core::deploy_prepared_context_at_with_overrides(
                                &game.deployment_target(&p),
                                &payload.backend,
                                &proxies,
                                None,
                                files,
                                Some(&payload.version),
                                Some(&context),
                                Some(&overrides),
                                payload.upstream_version.as_deref(),
                            )
                            .map(|message| cleanup::CleanOutcome {
                                message,
                                complete: true,
                            })
                        })()
                    } else {
                        cleanup::clean_outcome_at(&game.deployment_target(&p))
                    };
                    match result {
                        Ok(outcome) => {
                            let ok = outcome.complete;
                            let s = outcome.message;
                            all_ok &= ok;
                            c.send(Event::PatchResult(ok));
                            if ok {
                                c.send(Event::Log(format!(
                                    "{}：{s}",
                                    p.file_name().unwrap_or_default().to_string_lossy()
                                )));
                            } else {
                                errors.push(format!("{exe}：{s}"));
                            }
                        }
                        Err(e) => {
                            all_ok = false;
                            c.send(Event::PatchResult(false));
                            errors.push(format!("{exe}：{e}"));
                        }
                    }
                    c.send(Event::Status(
                        exe,
                        core::status_at(&game.deployment_target(&p)),
                    ));
                }
                if all_ok && !clean {
                    let profile = &catalog.scheme_policies[&scheme].parameter_profile;
                    let actual = if uses_explicit_edits(profile) {
                        read_target
                            .as_ref()
                            .ok_or_else(|| anyhow::anyhow!("没有实际部署目录"))
                            .and_then(|target| read_json_preset_values(target, profile))
                    } else {
                        Ok(options)
                    };
                    match actual {
                        Ok(actual) => c.send(Event::PresetApplied(primary, scheme, actual)),
                        Err(error) => {
                            errors.push(format!("参数写入后读取失败：{primary}：{error}"))
                        }
                    }
                }
            }
            if !errors.is_empty() {
                c.send(Event::Warning(errors.join("\n\n")));
            }
            Ok(())
        });
    }
    fn target_games(&self) -> Vec<&Game> {
        self.games
            .iter()
            .filter(|g| {
                if self.selected.is_empty() {
                    self.focus.as_ref() == Some(&g.exe)
                } else {
                    self.selected.contains(&g.exe)
                }
            })
            .collect()
    }
    /// Installation operates once per verified rendering directory, never on a launcher.
    pub fn patch_targets(&self, clean: bool) -> BTreeSet<String> {
        let mut directories = BTreeMap::new();
        for game in self.target_games() {
            let targets = game.deployment_executables();
            for target in targets
                .iter()
                .chain(game.cleanup_only.iter().filter(|_| clean))
            {
                let path = std::path::Path::new(target);
                let custom_target = game.deployment_target(path);
                let dir = &custom_target.directory;
                // Persisted game paths are absolute. Keep legacy relative test
                // entries distinct instead of collapsing every bare name into
                // the same empty parent directory.
                let key = if path.is_absolute() {
                    core::key(dir)
                } else {
                    core::key(path)
                };
                directories
                    .entry(key)
                    .or_insert_with(|| target.as_str().to_owned());
            }
        }
        directories.into_values().collect()
    }
    pub fn targets(&self) -> BTreeSet<String> {
        self.patch_targets(false)
    }
    pub fn target_game_count(&self) -> usize {
        self.target_games().len()
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
        let before = self.games.len();
        self.games.retain(|g| !paths.contains(&g.exe));
        if self.games.len() != before {
            self.library_revision += 1;
        }
        self.selected.retain(|p| !paths.contains(p));
        self.statuses.retain(|p, _| !paths.contains(p));
        self.deployments.retain(|p, _| !paths.contains(p));
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
            let location = self
                .games
                .iter()
                .find(|g| g.exe == exe)
                .map(parameter_target)
                .unwrap_or_else(|| core::DeploymentTarget::for_game(std::path::Path::new(&exe)));
            let catalog = self.catalog.clone();
            let epoch = self.status_epoch;
            self.channel.job(move |c| {
                let running = core::assert_target_stopped(&location).is_err();
                let values = rtx_fg_manager::presets::inspect_at(&location, &catalog);
                match values {
                    Ok(values) => c.send(Event::PresetRead(target, values, running, epoch)),
                    Err(error) => c.send(Event::PresetReadFailed(
                        target,
                        format!(
                            "无法读取当前参数，保留上次成功读取的值：{}：{error}",
                            location.directory.display()
                        ),
                        running,
                        epoch,
                    )),
                }
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
        self.proxies_for(self.focus.as_deref())
    }
    pub fn proxies_for(&self, exe: Option<&str>) -> Vec<String> {
        let scheme = self.scheme_for(exe);
        let allowed = self.catalog.proxies(&scheme);
        let limit = self.catalog.scheme_policies[&scheme].max_selected_proxies;
        let saved = exe
            .and_then(|e| self.games.iter().find(|g| g.exe == e))
            .and_then(|g| g.extra.get("proxy_choices"))
            .and_then(|v| v.get(&scheme))
            .cloned()
            .or_else(|| {
                exe.and_then(|e| self.deployments.get(e))
                    .and_then(|d| d.common.as_ref())
                    .filter(|d| d.0 == scheme)
                    .map(|d| json!(d.1))
            })
            .unwrap_or_else(|| self.state["proxies"].clone());
        let selected: Vec<String> = saved
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
    fn apply_pending_catalog(&mut self) {
        if self.closing {
            self.pending_catalog = None;
            return;
        }
        if self.busy {
            return;
        }
        if let Some(catalog) = self.pending_catalog.take() {
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
                    if r.cancelled {
                        self.merge(r.rows)
                    } else {
                        self.merge_scan(r.rows)
                    }
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
                Event::CustomFolder(exe, folder) => {
                    if let Some(game) = self
                        .games
                        .iter_mut()
                        .find(|g| g.exe.eq_ignore_ascii_case(&exe))
                    {
                        game.deployment_dir = folder;
                    } else {
                        self.games.push(Game {
                            exe: exe.clone(),
                            deployment_dir: folder,
                            root: scanner::game_root(std::path::Path::new(&exe))
                                .display()
                                .to_string(),
                            ..Default::default()
                        });
                    }
                    self.library_revision += 1;
                    self.status_epoch += 1;
                    self.focus = Some(exe.clone());
                    self.deployments.remove(&exe);
                    self.preset_reads.remove(&exe);
                    self.disk_presets.retain(|(e, _), _| e != &exe);
                    if let Ok(mut cache) = self.display_cache.lock() {
                        cache.remove(&exe);
                    }
                    self.save();
                    self.log("已设置自定义部署目录；请确认加载方式后安装补丁。");
                    self.refresh();
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
                Event::PresetReadFailed(exe, error, running, epoch) => {
                    self.preset_reads.remove(&exe);
                    if epoch == self.status_epoch && self.games.iter().any(|g| g.exe == exe) {
                        if running {
                            self.running_games.insert(exe);
                        } else {
                            self.running_games.remove(&exe);
                        }
                        self.log(&error);
                        self.warning = Some(error);
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
                        if let Some(keys) = game
                            .extra
                            .get_mut("preset_dirty_keys")
                            .and_then(Value::as_object_mut)
                        {
                            keys.remove(&scheme);
                        }
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
                        if self.state.get("cloud_series").is_none() {
                            self.state["cloud_series"] = json!(if *sm == 75 {
                                0
                            } else if *sm == 89 {
                                2
                            } else {
                                1
                            });
                        }
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
                    // Keep the newest successful refresh while the active operation
                    // continues with its own catalog/payload snapshot.
                    self.pending_catalog = Some(catalog);
                    self.apply_pending_catalog();
                }
                Event::Deployments(epoch, exe, snapshot) => {
                    if epoch == self.status_epoch && self.games.iter().any(|g| g.exe == exe) {
                        self.deployments.insert(exe, snapshot);
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
                    self.apply_pending_catalog();
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
                    self.pending_catalog = None;
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
        self.pending_catalog = None;
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
    #[test]
    fn grouped_encore_settings_read_rendering_directory_and_failed_read_preserves_last_values() {
        use rtx_fg_manager::encore;
        let (dir, mut c) = controller();
        let render_dir = dir.path().join("Rendering/Binaries");
        std::fs::create_dir_all(&render_dir).unwrap();
        let game = Game {
            exe: dir.path().join("Launcher.exe").display().to_string(),
            targets: vec![render_dir.join("Game.exe").display().to_string()],
            ..Default::default()
        };
        std::fs::write(render_dir.join(encore::CONFIG), encore::DEFAULT_CONFIG).unwrap();
        assert_eq!(parameter_target(&game).directory, render_dir);
        assert_eq!(
            read_json_preset_values(&parameter_target(&game), encore::PROFILE).unwrap(),
            encore::defaults()
        );
        let exe = game.exe.clone();
        c.games = vec![game];
        let values = encore::defaults();
        c.disk_presets
            .insert((exe.clone(), encore::SCHEME.into()), values.clone());
        c.preset_reads.insert(exe.clone());
        c.channel.send(Event::PresetReadFailed(
            exe.clone(),
            "broken config".into(),
            false,
            c.status_epoch,
        ));
        c.events();
        assert_eq!(c.warning.as_deref(), Some("broken config"));
        assert_eq!(
            c.disk_presets[&(exe.clone(), encore::SCHEME.into())],
            values
        );
        assert!(!c.preset_reads.contains(&exe));
        c.warning = None;
        c.channel.send(Event::PresetReadFailed(
            exe,
            "stale failure".into(),
            false,
            c.status_epoch + 1,
        ));
        c.events();
        assert!(c.warning.is_none());
        c.close();
        c.tick_close();
    }
    #[test]
    fn encore_dirty_fields_merge_over_latest_menu_values_and_clear_after_apply() {
        use rtx_fg_manager::encore;
        let (_dir, mut c) = controller();
        let scheme = encore::SCHEME;
        c.catalog
            .scheme_policies
            .get_mut(scheme)
            .unwrap()
            .parameter_profile = encore::PROFILE.into();
        c.state["cloud_scheme"] = json!(scheme);
        c.games = vec![
            Game {
                exe: "Encore-A.exe".into(),
                ..Default::default()
            },
            Game {
                exe: "Encore-B.exe".into(),
                ..Default::default()
            },
        ];
        let mut disk = encore::defaults();
        disk.insert("tf_mode".into(), "3".into());
        disk.insert("reflexFrameLimit".into(), "117".into());
        c.disk_presets
            .insert(("Encore-A.exe".into(), scheme.into()), disk.clone());
        c.set_game_option("Encore-A.exe", "tf_mode", "4");
        assert_eq!(
            c.explicit_preset_values("Encore-A.exe", scheme),
            BTreeMap::from([("tf_mode".into(), "4".into())])
        );
        disk.insert("reflexFrameLimit".into(), "141".into());
        disk.insert("tf_mode".into(), "2".into());
        c.channel.send(Event::PresetRead(
            "Encore-A.exe".into(),
            Some((scheme.into(), disk)),
            false,
            c.status_epoch,
        ));
        c.events();
        assert_eq!(c.preset_values("Encore-A.exe")["reflexFrameLimit"], "141");
        assert_eq!(c.preset_values("Encore-A.exe")["tf_mode"], "4");
        assert_eq!(c.preset_values("Encore-B.exe")["tf_mode"], "game");
        c.set_game_option("Encore-A.exe", "reflexFrameLimit", "NaN");
        assert_eq!(c.preset_values("Encore-A.exe")["reflexFrameLimit"], "141");
        let applied = c.preset_values("Encore-A.exe");
        c.channel.send(Event::PresetApplied(
            "Encore-A.exe".into(),
            scheme.into(),
            applied,
        ));
        c.events();
        assert!(!c.preset_dirty("Encore-A.exe"));
        assert!(c.explicit_preset_values("Encore-A.exe", scheme).is_empty());
        c.close();
        c.tick_close();
    }

    #[test]
    fn rtxmfg_edits_merge_with_latest_disk_values_and_legacy_keys_stay_bounded() {
        use rtx_fg_manager::rtxmfg;
        let (_dir, mut c) = controller();
        let scheme = "rtx40mfg-1.3.3-hf2";
        let exe = "RTXMFG-A.exe";
        c.games = vec![Game {
            exe: exe.into(),
            extra: [
                (
                    "deployment_choice".into(),
                    json!({"scheme":scheme,"series":2}),
                ),
                (
                    "preset_options_v2".into(),
                    json!({scheme:{"rtx_mode":"6","rtx_target":"90"}}),
                ),
            ]
            .into_iter()
            .collect(),
            ..Default::default()
        }];
        let mut disk = rtx_fg_manager::presets::defaults(rtxmfg::PROFILE, &BTreeMap::new());
        disk.insert("rtx_mode".into(), "2".into());
        disk.insert("rtx_target".into(), "117".into());
        c.disk_presets
            .insert((exe.into(), scheme.into()), disk.clone());
        assert!(c.explicit_preset_values(exe, scheme).is_empty());
        assert_eq!(c.preset_values(exe)["rtx_mode"], "2");
        c.set_game_option(exe, "rtx_vsync", "1");
        assert_eq!(
            c.explicit_preset_values(exe, scheme),
            BTreeMap::from([("rtx_vsync".into(), "1".into())])
        );
        disk.insert("rtx_mode".into(), "4".into());
        disk.insert("rtx_target".into(), "141".into());
        c.disk_presets.insert((exe.into(), scheme.into()), disk);
        let values = c.preset_values(exe);
        assert_eq!(values["rtx_mode"], "4");
        assert_eq!(values["rtx_target"], "141");
        assert_eq!(values["rtx_vsync"], "1");
        c.channel
            .send(Event::PresetApplied(exe.into(), scheme.into(), values));
        c.events();
        assert!(!c.preset_dirty(exe));
        assert!(c.explicit_preset_values(exe, scheme).is_empty());
        let game = &mut c.games[0];
        game.extra.remove("preset_dirty_keys");
        game.extra
            .insert("preset_dirty".into(), json!({scheme:true}));
        game.extra.insert(
            "preset_options_v2".into(),
            json!({scheme:{"rtx_mode":"3","rtx_target":"144","unrelated":"1"}}),
        );
        assert_eq!(
            c.explicit_preset_values(exe, scheme),
            BTreeMap::from([
                ("rtx_mode".into(), "3".into()),
                ("rtx_target".into(), "144".into())
            ])
        );
        c.close();
        c.tick_close();
    }

    #[test]
    fn encore_legacy_dirty_flag_only_migrates_four_exposed_transfusion_preferences() {
        use rtx_fg_manager::encore;
        let (_dir, mut c) = controller();
        let scheme = encore::SCHEME;
        c.catalog
            .scheme_policies
            .get_mut(scheme)
            .unwrap()
            .parameter_profile = encore::PROFILE.into();
        c.state["cloud_scheme"] = json!(scheme);
        c.games = vec![Game {
            exe: "Encore-Old.exe".into(),
            extra: [
                (
                    "preset_options_v2".into(),
                    json!({scheme:{"tf_mode":"6","tf_target":"200","nrEnabled":"1"}}),
                ),
                ("preset_dirty".into(), json!({scheme:true})),
            ]
            .into_iter()
            .collect(),
            ..Default::default()
        }];
        c.disk_presets
            .insert(("Encore-Old.exe".into(), scheme.into()), encore::defaults());
        assert_eq!(
            c.explicit_preset_values("Encore-Old.exe", scheme),
            BTreeMap::from([
                ("tf_mode".into(), "6".into()),
                ("tf_target".into(), "200".into()),
            ])
        );
        assert_eq!(c.preset_values("Encore-Old.exe")["nrEnabled"], "0");
        c.running_games.insert("Encore-Old.exe".into());
        c.set_game_option("Encore-Old.exe", "tf_mode", "2");
        assert_eq!(c.preset_values("Encore-Old.exe")["tf_mode"], "6");
        c.close();
        c.tick_close();
    }
    #[test]
    fn active_data_directory_guard_normalizes_paths_without_blocking_siblings() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("自选管理器数据");
        std::fs::create_dir_all(data.join("child")).unwrap();
        assert!(ensure_directory_outside_data(&data, &data).is_err());
        assert!(ensure_directory_outside_data(&data.join("child"), &data).is_err());
        assert!(ensure_directory_outside_data(&data.join("child/.."), &data).is_err());
        assert!(
            ensure_directory_outside_data(
                &PathBuf::from(data.to_string_lossy().to_uppercase()).join("child"),
                &data,
            )
            .is_err()
        );
        assert!(
            ensure_directory_outside_data(&dir.path().join("自选管理器数据-游戏"), &data).is_ok()
        );
    }
    #[test]
    fn active_data_directory_blocks_install_and_presets_but_allows_cleanup() {
        let (_dir, mut c) = controller();
        let game_dir = tempfile::tempdir().unwrap();
        let exe = game_dir.path().join("RTXFG-ActiveData-Guard.exe");
        // Never executed. The write guard must reject before PE validation or downloads.
        std::fs::write(&exe, b"not executable").unwrap();
        let plugin = c.data.join("plugins");
        std::fs::create_dir(&plugin).unwrap();
        std::fs::write(plugin.join("OptiScaler.ini"), b"preserve unrelated config").unwrap();
        let exe = exe.display().to_string();
        c.games = vec![Game {
            exe: exe.clone(),
            deployment_dir: plugin.display().to_string(),
            ..Default::default()
        }];
        c.focus = Some(exe.clone());
        for operation in [0, 1, 2] {
            c.busy = false;
            c.critical = false;
            if operation == 1 {
                c.apply_focused_parameters();
            } else {
                c.perform(operation == 2, BTreeSet::from([exe.clone()]));
            }
            let deadline = Instant::now() + Duration::from_secs(5);
            let mut blocked = false;
            let mut cleaned = false;
            loop {
                let event =
                    c.rx.recv_timeout(deadline.saturating_duration_since(Instant::now()))
                        .expect("operation completed");
                match event {
                    Event::Warning(message) => {
                        blocked |= message.contains("不能部署到管理器缓存或设置目录");
                    }
                    Event::Log(message) => cleaned |= message.contains("未发现可确认归属的补丁"),
                    Event::PayloadProgress(_) => panic!("guard must run before cloud downloads"),
                    Event::Done => break,
                    _ => {}
                }
            }
            assert_eq!(blocked, operation != 2);
            assert_eq!(cleaned, operation == 2);
            assert_eq!(
                std::fs::read(plugin.join("OptiScaler.ini")).unwrap(),
                b"preserve unrelated config"
            );
            assert!(!plugin.join("version.dll").exists());
        }
        c.busy = false;
        c.critical = false;
        c.close();
        c.tick_close();
    }
    #[test]
    fn series_switch_selects_compatible_scheme_without_affecting_other_games() {
        let (_dir, mut c) = controller();
        c.games = vec![
            Game {
                exe: "A.exe".into(),
                ..Default::default()
            },
            Game {
                exe: "B.exe".into(),
                ..Default::default()
            },
        ];
        c.focus = Some("A.exe".into());
        c.set_selection("upstream-0.3.5-310-9".into(), 0, vec!["version.dll".into()]);
        c.select_series(2);
        assert_eq!(c.cloud_series(), 2);
        assert!(c.catalog.supports_series(&c.cloud_scheme(), 2));
        c.set_selection(
            "dlssg-transfusion-1.4.5.3".into(),
            2,
            vec!["dxgi.dll".into()],
        );
        c.select_series(0);
        assert_eq!(c.cloud_scheme(), "dlssg-transfusion-1.4.5.3");
        assert_eq!(c.proxies(), ["dxgi.dll"]);
        c.set_selection("rtx40mfg-1.3.3-hf2".into(), 0, vec!["version.dll".into()]);
        assert_eq!(c.cloud_scheme(), "dlssg-transfusion-1.4.5.3");
        c.focus = Some("B.exe".into());
        assert_ne!(c.cloud_scheme(), "dlssg-transfusion-1.4.5.3");
    }
    #[test]
    fn custom_folder_event_keeps_grouped_paths_for_restoration_and_rescan() {
        let (_dir, mut c) = controller();
        let paths: Vec<String> = vec![
            "C:/RTXFG-test/Win64/Game.exe".into(),
            "C:/RTXFG-test/Win64r/Game.exe".into(),
        ];
        c.games = vec![Game {
            exe: paths[0].clone(),
            targets: paths.clone(),
            ..Default::default()
        }];
        c.channel.send(Event::CustomFolder(
            paths[0].clone(),
            "C:/RTXFG-test/plugins".into(),
        ));
        c.events();
        assert_eq!(c.games[0].targets, paths);
        assert_eq!(c.games[0].deployment_executables(), [paths[0].clone()]);
        c.channel
            .send(Event::CustomFolder(paths[0].clone(), String::new()));
        c.events();
        assert_eq!(c.games[0].deployment_executables(), paths);
    }
    #[test]
    fn game_selection_follows_records_and_explicit_choices_do_not_leak() {
        let (_dir, mut c) = controller();
        c.games = vec![
            Game {
                exe: "A.exe".into(),
                ..Default::default()
            },
            Game {
                exe: "B.exe".into(),
                ..Default::default()
            },
        ];
        for (exe, scheme, proxy) in [
            ("A.exe", "upstream-0.3.5-310-9", "d3d12.dll"),
            ("B.exe", "rtxfg-0.3.5-dx12-vulkan", "version.dll"),
        ] {
            c.deployments.insert(
                exe.into(),
                rtx_fg_manager::deployment::Snapshot {
                    total: 1,
                    installed: 1,
                    common: Some((scheme.into(), vec![proxy.into()], None)),
                    ..Default::default()
                },
            );
        }
        c.focus = Some("A.exe".into());
        assert_eq!(c.cloud_scheme(), "upstream-0.3.5-310-9");
        assert_eq!(c.proxies(), vec!["d3d12.dll"]);
        c.set_game_option("A.exe", "logging_level", "3");
        c.set_selection(c.cloud_scheme(), 0, vec!["winmm.dll".into()]);
        c.focus = Some("B.exe".into());
        assert_eq!(c.cloud_scheme(), "rtxfg-0.3.5-dx12-vulkan");
        assert_eq!(c.proxies(), vec!["version.dll"]);
        assert_eq!(c.preset_values("B.exe")["logging_level"], "1");
        c.channel.send(Event::PresetRead(
            "A.exe".into(),
            Some((
                "upstream-0.3.5-310-9".into(),
                BTreeMap::from([("logging_level".into(), "0".into())]),
            )),
            false,
            c.status_epoch,
        ));
        c.events();
        assert_eq!(c.cloud_scheme(), "rtxfg-0.3.5-dx12-vulkan");
        c.focus = Some("A.exe".into());
        assert_eq!(c.proxies(), vec!["winmm.dll"]);
        assert_eq!(c.cloud_series(), 0);
        assert_eq!(c.preset_values("A.exe")["logging_level"], "3");
        assert_eq!(
            c.games[0].extra["deployment_choice"]["scheme"],
            "upstream-0.3.5-310-9"
        );
        assert_eq!(c.scheme_for(Some("B.exe")), "rtxfg-0.3.5-dx12-vulkan");
        c.close();
        c.tick_close();
    }
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
    fn busy_catalog_refresh_keeps_latest_success_until_operation_finishes() {
        for critical in [false, true] {
            let (_dir, mut c) = controller();
            c.busy = true;
            c.critical = critical;
            let operation_catalog = c.catalog.clone();
            let mut cached = c.catalog.clone();
            cached.revision = "cached-refresh".into();
            let mut latest = cached.clone();
            latest.revision = "latest-refresh".into();
            c.channel.send(Event::Catalog(cached));
            c.channel.send(Event::Catalog(latest));
            c.channel.send(Event::Log("later refresh failed".into()));
            c.events();
            assert_eq!(c.catalog.revision, operation_catalog.revision);
            assert_eq!(
                c.pending_catalog.as_ref().unwrap().revision,
                "latest-refresh"
            );
            c.channel.send(Event::Done);
            c.events();
            assert_eq!(c.catalog.revision, "latest-refresh");
            assert_ne!(operation_catalog.revision, c.catalog.revision);
            assert!(c.pending_catalog.is_none());
            assert!(!c.busy && !c.critical);
            c.close();
            c.tick_close();
        }
    }
    #[test]
    fn idle_catalog_applies_immediately_but_closing_discards_pending_and_late_refreshes() {
        let (_dir, mut c) = controller();
        let mut catalog = c.catalog.clone();
        catalog.revision = "idle-refresh".into();
        c.channel.send(Event::Catalog(catalog.clone()));
        c.events();
        assert_eq!(c.catalog.revision, "idle-refresh");
        c.busy = true;
        catalog.revision = "pending-refresh".into();
        c.channel.send(Event::Catalog(catalog.clone()));
        c.events();
        assert!(c.pending_catalog.is_some());
        c.close();
        assert!(c.pending_catalog.is_none());
        catalog.revision = "late-refresh".into();
        catalog
            .skipped_schemes
            .insert("ignored-on-close".into(), "must not be applied".into());
        c.channel.send(Event::Catalog(catalog));
        c.channel.send(Event::Done);
        c.events();
        assert_eq!(c.catalog.revision, "idle-refresh");
        assert!(c.pending_catalog.is_none());
        assert!(!c.logs.iter().any(|s| s.contains("ignored-on-close")));
        c.tick_close();
    }
    #[test]
    fn cleanup_with_unverified_stage_is_failed_and_warned_until_recovery_finishes() {
        let (_data, mut c) = controller();
        let game = tempfile::tempdir().unwrap();
        let exe = game.path().join("RTXFG-Controller-Stage-Recovery.exe");
        let mut pe = vec![0; 512];
        pe[..2].copy_from_slice(b"MZ");
        pe[60..64].copy_from_slice(&128u32.to_le_bytes());
        pe[128..132].copy_from_slice(b"PE\0\0");
        pe[132..134].copy_from_slice(&0x8664u16.to_le_bytes());
        pe[148..150].copy_from_slice(&240u16.to_le_bytes());
        pe[150..152].copy_from_slice(&2u16.to_le_bytes());
        pe[152..154].copy_from_slice(&0x20bu16.to_le_bytes());
        std::fs::write(&exe, &pe).unwrap();
        pe[150..152].copy_from_slice(&0x2000u16.to_le_bytes());
        core::deploy_prepared(
            &exe,
            rtx_fg_manager::transfusion::BACKEND,
            &["version.dll".into()],
            None,
            BTreeMap::from([
                ("version.dll".into(), pe.clone()),
                (
                    rtx_fg_manager::transfusion::CONFIG.into(),
                    br#"{"configVersion":3,"frameGeneration":{"mode":"game","multiplier":4}}"#
                        .to_vec(),
                ),
            ]),
            Some("1.4.5"),
        )
        .unwrap();
        let stage = game.path().join(core::OWN).join("version.dll.stage");
        std::fs::write(&stage, &pe[..100]).unwrap();
        let unknown = game.path().join("other-plugin.dll");
        std::fs::write(&unknown, b"preserve unrelated plugin").unwrap();
        let exe = exe.display().to_string();
        c.games = vec![Game {
            exe: exe.clone(),
            ..Default::default()
        }];
        for pending in [true, false] {
            c.warning = None;
            c.perform(true, BTreeSet::from([exe.clone()]));
            let deadline = Instant::now() + Duration::from_secs(5);
            while c.busy {
                c.events();
                assert!(Instant::now() < deadline, "cleanup operation completed");
                std::thread::sleep(Duration::from_millis(10));
            }
            assert!(!game.path().join("version.dll").exists());
            assert_eq!(
                std::fs::read(&unknown).unwrap(),
                b"preserve unrelated plugin"
            );
            if pending {
                assert!(c.warning.as_deref().unwrap().contains("临时文件待核验"));
                assert!(
                    c.logs
                        .iter()
                        .any(|s| s.contains("成功 0，失败 1，未处理 0"))
                );
                assert_eq!(std::fs::read(&stage).unwrap(), pe[..100]);
                assert!(core::record(game.path()).unwrap().unwrap().cache_pending);
                // Only the synthetic test fragment is removed; recovery is retried normally.
                std::fs::remove_file(&stage).unwrap();
            } else {
                assert!(c.warning.is_none());
                assert!(
                    c.logs
                        .iter()
                        .any(|s| s.contains("成功 1，失败 0，未处理 0"))
                );
                assert!(core::record(game.path()).unwrap().is_none());
            }
        }
        c.close();
        c.tick_close();
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
            notes: Default::default(),
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
    fn rescan_folds_legacy_entries_and_keeps_parameters_and_old_cleanup_path() {
        let (dir, mut c) = controller();
        let root = dir.path().join("steamapps").join("common").join("Signal");
        let renderer = root
            .join("SignalGame")
            .join("Binaries")
            .join("Win64")
            .join("Signal-Win64-Shipping.exe");
        let old_launcher = root.join("Signal.exe");
        let old_tool = root.join("tools").join("SignalHelper.exe");
        std::fs::create_dir_all(renderer.parent().unwrap()).unwrap();
        std::fs::create_dir_all(old_tool.parent().unwrap()).unwrap();
        std::fs::create_dir_all(root.join(core::OWN)).unwrap();
        let renderer = renderer.to_string_lossy().into_owned();
        let old_launcher = old_launcher.to_string_lossy().into_owned();
        let old_tool = old_tool.to_string_lossy().into_owned();
        c.games = vec![
            Game {
                exe: old_launcher.clone(),
                root: root.to_string_lossy().into_owned(),
                extra: [(
                    "preset_options_v2".into(),
                    json!({"upstream-0.3.5-310-9":{"logging_level":"3"}}),
                )]
                .into_iter()
                .collect(),
                ..Default::default()
            },
            Game {
                exe: old_tool.clone(),
                root: root.to_string_lossy().into_owned(),
                ..Default::default()
            },
        ];
        c.focus = Some(old_launcher.clone());
        c.selected.insert(old_launcher.clone());
        c.disk_presets.insert(
            (old_launcher.clone(), "upstream-0.3.5-310-9".into()),
            BTreeMap::from([("logging_level".into(), "3".into())]),
        );
        c.merge_scan(vec![Game {
            exe: renderer.clone(),
            root: root.to_string_lossy().into_owned(),
            title: "Signal".into(),
            targets: vec![renderer.clone()],
            reasons: vec!["Steam 已安装游戏".into()],
            ..Default::default()
        }]);
        assert_eq!(c.games.len(), 1);
        assert_eq!(c.focus.as_deref(), Some(renderer.as_str()));
        assert_eq!(c.selected, BTreeSet::from([renderer.clone()]));
        assert_eq!(c.target_game_count(), 1);
        assert_eq!(c.patch_targets(false), BTreeSet::from([renderer.clone()]));
        assert_eq!(
            c.patch_targets(true),
            BTreeSet::from([renderer.clone(), old_launcher.clone()])
        );
        assert!(!c.games[0].cleanup_only.contains(&old_tool));
        assert_eq!(
            c.games[0].extra["preset_options_v2"]["upstream-0.3.5-310-9"]["logging_level"],
            "3"
        );
        assert!(
            c.disk_presets
                .contains_key(&(renderer, "upstream-0.3.5-310-9".into()))
        );
        c.close();
        c.tick_close();
    }
    #[test]
    fn one_game_card_expands_to_verified_rendering_directories() {
        let (_dir, mut c) = controller();
        c.games = vec![Game {
            exe: "C:/Games/Game/Binaries/Win64/Game-Win64-Shipping.exe".into(),
            targets: vec![
                "C:/Games/Game/Binaries/Win64/Game-Win64-Shipping.exe".into(),
                "C:/Games/Game/Binaries/Win64DX12/Game-Win64-Shipping.exe".into(),
            ],
            cleanup_only: vec!["C:/Games/Game/OldLauncher.exe".into()],
            ..Default::default()
        }];
        c.focus = Some(c.games[0].exe.clone());
        assert_eq!(c.target_game_count(), 1);
        assert_eq!(c.patch_targets(false).len(), 2);
        assert_eq!(c.patch_targets(true).len(), 3);
        c.close();
        c.tick_close();
    }
    #[test]
    fn manual_add_in_an_existing_deployment_directory_does_not_duplicate_the_card() {
        let (dir, mut c) = controller();
        let first_dir = dir.path().join("Game/Binaries/Win64");
        let second_dir = dir.path().join("Game/Binaries/Win64DX12");
        let first = first_dir
            .join("Game-Win64-Shipping.exe")
            .display()
            .to_string();
        let alternate = first_dir.join("Game-DX12.exe").display().to_string();
        let second = second_dir.join("Game-DX12.exe").display().to_string();
        c.games = vec![Game {
            exe: first.clone(),
            targets: vec![first.clone(), second.clone()],
            ..Default::default()
        }];
        c.merge(vec![Game {
            exe: alternate,
            ..Default::default()
        }]);
        assert_eq!(c.games.len(), 1);
        c.focus = Some(first);
        assert_eq!(
            c.patch_targets(false),
            BTreeSet::from([c.games[0].exe.clone(), second])
        );
        c.close();
        c.tick_close();
    }
    #[test]
    fn batch_targets_process_a_shared_directory_only_once() {
        let (dir, mut c) = controller();
        let common = dir.path().join("Game/Binaries/Win64");
        let first = common.join("Game.exe").display().to_string();
        let second = common.join("Game-DX12.exe").display().to_string();
        c.games = vec![
            Game {
                exe: first.clone(),
                ..Default::default()
            },
            Game {
                exe: second.clone(),
                ..Default::default()
            },
        ];
        c.selected = BTreeSet::from([first.clone(), second]);
        assert_eq!(c.patch_targets(false), BTreeSet::from([first.clone()]));
        assert_eq!(c.patch_targets(true), BTreeSet::from([first]));
        c.close();
        c.tick_close();
    }
    #[test]
    fn partial_group_status_cannot_enable_parameter_application() {
        let (dir, mut c) = controller();
        let first = dir.path().join("Game/Win64/Game.exe").display().to_string();
        let second = dir
            .path()
            .join("Game/Win64DX12/Game.exe")
            .display()
            .to_string();
        c.games = vec![Game {
            exe: first.clone(),
            targets: vec![first.clone(), second],
            ..Default::default()
        }];
        c.focus = Some(first.clone());
        c.disk_presets.insert(
            (first.clone(), c.cloud_scheme()),
            BTreeMap::from([("logging_level".into(), "1".into())]),
        );
        c.statuses
            .insert(first.clone(), "已部署 1/2 个目录，请检查未完成项".into());
        assert!(!c.can_apply_parameters());
        let scheme = c.cloud_scheme();
        c.deployments.insert(
            first.clone(),
            rtx_fg_manager::deployment::Snapshot {
                total: 2,
                installed: 1,
                common: Some((scheme.clone(), vec!["version.dll".into()], None)),
                ..Default::default()
            },
        );
        assert!(!c.can_apply_parameters());
        c.statuses.insert(first.clone(), "已部署 2/2 个目录".into());
        c.deployments.get_mut(&first).unwrap().installed = 2;
        assert!(c.can_apply_parameters());
        c.deployments.get_mut(&first).unwrap().common = None;
        assert!(!c.can_apply_parameters());
        c.close();
        c.tick_close();
    }
    #[test]
    fn rescan_does_not_migrate_another_unreal_project_into_cleanup_only() {
        let (dir, mut c) = controller();
        let root = dir.path().join("Steam/steamapps/common/Collection");
        let first = root.join("First/Binaries/Win64/First-Win64-Shipping.exe");
        let second = root.join("Second/Binaries/Win64/Second-Win64-Shipping.exe");
        std::fs::create_dir_all(root.join("Engine")).unwrap();
        std::fs::create_dir_all(first.parent().unwrap()).unwrap();
        std::fs::create_dir_all(second.parent().unwrap()).unwrap();
        std::fs::write(second.parent().unwrap().join(core::INI), b"[Runtime]\n").unwrap();
        assert_ne!(
            scanner::installation_root(&first),
            scanner::installation_root(&second)
        );
        let first = first.display().to_string();
        let second = second.display().to_string();
        c.games = vec![Game {
            exe: first.clone(),
            root: root.display().to_string(),
            targets: vec![first.clone(), second.clone()],
            ..Default::default()
        }];
        c.merge_scan(vec![
            Game {
                exe: first.clone(),
                root: root.join("First").display().to_string(),
                targets: vec![first.clone()],
                ..Default::default()
            },
            Game {
                exe: second.clone(),
                root: root.join("Second").display().to_string(),
                targets: vec![second],
                ..Default::default()
            },
        ]);
        assert_eq!(c.games.len(), 2);
        assert!(c.games[0].cleanup_only.is_empty());
        c.close();
        c.tick_close();
    }
    #[test]
    fn rescan_removes_old_steam_client_false_positives_but_keeps_manual_entries() {
        let (dir, mut c) = controller();
        let steam = dir.path().join("Steam");
        std::fs::create_dir_all(steam.join("steamapps")).unwrap();
        let steam_exe = steam.join("steam.exe").to_string_lossy().into_owned();
        let manually_added = steam.join("tools.exe").to_string_lossy().into_owned();
        c.games = vec![
            Game {
                exe: steam_exe.clone(),
                reasons: vec!["旧扫描误报".into()],
                ..Default::default()
            },
            Game {
                exe: manually_added.clone(),
                ..Default::default()
            },
        ];
        c.focus = Some(steam_exe.clone());
        c.selected.insert(steam_exe.clone());
        c.merge_scan(vec![]);
        assert_eq!(c.games.len(), 1);
        assert_eq!(c.games[0].exe, manually_added);
        assert!(c.focus.is_none() && c.selected.is_empty());
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
