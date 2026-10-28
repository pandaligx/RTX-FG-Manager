//! Game-scoped deployment recipes. Community content never supplies file paths,
//! download URLs or commands. Installation and parameter application are explicit actions.
use crate::{cloud, core, game_names, neural_runtime, scanner::Game};
use anyhow::{Context, Result, ensure};
use reqwest::{Method, blocking::Client as Http};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};
use zeroize::{Zeroize, Zeroizing};

const ORIGIN: &str = "https://recommend.lgxng.cn";
// Existing service origin, used only after normal read routes fail. Keep the
// original HTTPS authority/SNI and certificate checks; never use an IP URL.
// If hosting changes, update this fallback; primary routing always uses DNS.
const ORIGIN_HOST: &str = "recommend.lgxng.cn";
const ORIGIN_IPV4: std::net::Ipv4Addr = std::net::Ipv4Addr::new(111, 230, 186, 38);
pub const NOTES_LIMIT: usize = 5000;
const RESPONSE_LIMIT: u64 = 2 * 1024 * 1024;
// Window instances share one on-disk session. Compare and replace it under the
// same lock so a late response cannot delete a newer window's successful login.
static SESSION_FILE_LOCK: Mutex<()> = Mutex::new(());
fn read_session(root: &Path) -> Result<Option<Session>> {
    let path = core::no_links(&root.join("session.json"))?;
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    ensure!(file.metadata()?.len() <= 65536, "登录凭据无效");
    let mut bytes = Vec::new();
    file.take(65537).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 65536, "登录凭据无效");
    let protected: Vec<u8> = serde_json::from_slice(&bytes)?;
    Ok(Some(serde_json::from_slice(&unseal(&protected, false)?)?))
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct GameIdentity {
    pub id: String,
    pub name: String,
}
impl GameIdentity {
    pub fn new(id: String, name: String) -> Result<Self> {
        let value = Self { id, name };
        value.validate()?;
        Ok(value)
    }
    pub fn custom(name: &str) -> Result<Self> {
        let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
        Self::new(
            format!(
                "custom:{:x}",
                Sha256::digest(name.to_lowercase().as_bytes())
            ),
            name,
        )
    }
    fn validate(&self) -> Result<()> {
        let catalog = self.id.strip_prefix("catalog:q").is_some_and(|s| {
            !s.is_empty() && s.len() <= 19 && s.bytes().all(|b| b.is_ascii_digit())
        });
        let custom = self
            .id
            .strip_prefix("custom:")
            .is_some_and(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()));
        ensure!(
            game_names::valid_key(&self.id) || catalog || custom,
            "推荐的游戏身份无效"
        );
        public_text(&self.name, 120, false)?;
        if custom {
            ensure!(
                self.id
                    == format!(
                        "custom:{:x}",
                        Sha256::digest(
                            self.name
                                .split_whitespace()
                                .collect::<Vec<_>>()
                                .join(" ")
                                .to_lowercase()
                                .as_bytes()
                        )
                    ),
                "推荐的游戏名称不匹配"
            );
        }
        Ok(())
    }
}
pub fn identify(game: &Game, catalog: &game_names::Catalog) -> Option<GameIdentity> {
    if let Some(value) = game.extra.get("recommendation_game")
        && let Ok(id) = serde_json::from_value::<GameIdentity>(value.clone())
        && id.validate().is_ok()
    {
        return Some(id);
    }
    let keys = game_names::keys(game);
    if let Some(entry) = game_names::identified(game, catalog) {
        let stable = entry
            .platform_keys()
            .iter()
            .find(|k| k.starts_with("steam:"))
            .cloned()
            .unwrap_or_else(|| format!("catalog:{}", entry.id().to_lowercase()));
        return GameIdentity::new(stable, entry.chinese().into()).ok();
    }
    // Without a reviewed cross-store mapping, do not guess that separate IDs are the same game.
    let key = (keys.len() == 1).then(|| &keys[0])?;
    GameIdentity::new(key.clone(), game_names::original(game)).ok()
}
pub fn platform(game: &Game) -> &'static str {
    let keys = game_names::keys(game);
    let names: BTreeSet<_> = keys
        .iter()
        .filter_map(|k| k.split_once(':').map(|v| v.0))
        .map(|s| match s {
            "gog_slug" => "gog",
            "epic_slug" => "epic",
            v => v,
        })
        .collect();
    if names.len() == 1 {
        match names.first().copied().unwrap_or("") {
            "steam" => "steam",
            "epic" => "epic",
            "gog" => "gog",
            "ea" => "ea",
            "ubisoft" => "ubisoft",
            "xbox" => "xbox",
            "battlenet" => "battlenet",
            "wegame" => "wegame",
            "hoyo" => "hoyo",
            "cube" => "cube",
            "sonkwo" => "sonkwo",
            _ => "unknown",
        }
    } else {
        "unknown"
    }
}
pub fn platform_label(platform: &str) -> &'static str {
    match platform {
        "steam" => "Steam",
        "epic" | "epic_slug" => "Epic Games",
        "gog" | "gog_slug" => "GOG",
        "ea" => "EA App",
        "ubisoft" => "Ubisoft Connect",
        "xbox" => "Xbox / Microsoft Store",
        "battlenet" => "战网",
        "wegame" => "WeGame",
        "hoyo" => "米哈游",
        "cube" => "方块游戏",
        "sonkwo" => "杉果",
        "standalone" => "独立客户端",
        _ => "来源未注明",
    }
}
#[derive(Clone)]
pub struct GameCandidate {
    pub identity: GameIdentity,
    pub original: String,
    pub platforms: Vec<String>,
}
impl GameCandidate {
    pub fn from_entry(entry: &game_names::Entry) -> Option<Self> {
        let id = entry
            .platform_keys()
            .iter()
            .find(|k| k.starts_with("steam:"))
            .cloned()
            .unwrap_or_else(|| format!("catalog:{}", entry.id().to_lowercase()));
        Some(Self {
            identity: GameIdentity::new(id, entry.chinese().into()).ok()?,
            original: entry.original().into(),
            platforms: candidate_platforms(entry.platform_keys()),
        })
    }
}
fn candidate_platforms(keys: &[String]) -> Vec<String> {
    keys.iter()
        .filter_map(|k| k.split_once(':'))
        .map(|(p, _)| platform_label(p))
        .filter(|p| *p != "来源未注明")
        .map(str::to_owned)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}
fn public_text(value: &str, max: usize, empty: bool) -> Result<()> {
    ensure!(
        (empty || !value.trim().is_empty())
            && value.chars().count() <= max
            && !value
                .chars()
                .any(|c| c.is_control() && c != '\n' && c != '\t'
                    || matches!(c,'<'|'>'|'\u{202a}'..='\u{202e}'|'\u{2066}'..='\u{2069}'))
            && !regex::Regex::new(r"(?i)([a-z]:[\\/]|\\\\|https?://|/Users/|/home/)")?
                .is_match(value),
        "备注或名称含有路径、链接或无效字符"
    );
    Ok(())
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[non_exhaustive]
pub struct Recipe {
    pub schema: u32,
    pub game: GameIdentity,
    pub title: String,
    pub notes: String,
    pub scheme: String,
    pub patch_version: String,
    pub entries: Vec<String>,
    pub renderer: String,
    pub gpu_model: String,
    pub driver: String,
    pub gpu_alias: String,
    pub neural_runtime: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub presets: Option<crate::recommendation_presets::Snapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtimes: Option<crate::recommendation_runtimes::Snapshot>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub platform: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub edition: String,
}
impl Recipe {
    pub fn validate(&self, catalog: &cloud::Catalog) -> Result<()> {
        self.validate_public()?;
        ensure!(
            catalog.schemes().iter().any(|p| p.scheme_id == self.scheme),
            "此方案已下架，不能自动替换为其他项目"
        );
        let allowed = catalog.proxies(&self.scheme);
        ensure!(
            self.entries.len() <= catalog.selection_limit(&self.scheme)
                && self.entries.iter().all(|p| allowed.contains(p)),
            "当前版本不再支持此推荐的 DLL 入口，请选择其他分享"
        );
        Ok(())
    }
    /// Reading historical cards must not depend on the currently available downloads.
    fn validate_public(&self) -> Result<()> {
        ensure!(self.schema == 1, "此推荐需要更新管理器");
        crate::recommendation_presets::validate_scheme_version(&self.scheme, &self.patch_version)?;
        if self.scheme == crate::recommendation_presets::MFG_161_COMPAT {
            ensure!(
                self.neural_runtime == "none"
                    && self.runtimes.as_ref().is_none_or(|r| r
                        .components
                        .iter()
                        .all(|c| c.kind != "nr" && c.source != "managed")),
                "MFG 1.6.1 不支持 DLSSNR 或独立 XeSS 运行库"
            );
        }
        if let Some(presets) = &self.presets {
            presets.validate(&self.scheme, &self.patch_version)?;
        }
        if let Some(runtimes) = &self.runtimes {
            runtimes.validate()?;
        }
        self.game.validate()?;
        ensure!(
            [
                "",
                "unknown",
                "steam",
                "epic",
                "gog",
                "ea",
                "ubisoft",
                "xbox",
                "battlenet",
                "wegame",
                "hoyo",
                "cube",
                "sonkwo",
                "standalone"
            ]
            .contains(&self.platform.as_str()),
            "推荐游戏平台无效"
        );
        public_text(&self.edition, 80, true)?;
        for (text, max, empty) in [
            (&self.title, 80, false),
            (&self.notes, NOTES_LIMIT, true),
            (&self.gpu_model, 80, true),
            (&self.driver, 40, true),
        ] {
            public_text(text, max, empty)?;
        }
        ensure!(
            self.scheme.len() <= 80
                && regex::Regex::new(r"^[a-z0-9][a-z0-9.-]+$")?.is_match(&self.scheme),
            "推荐方案标识无效"
        );
        let loader = regex::Regex::new(r"^[a-zA-Z0-9_-]+\.(dll|asi)$")?;
        ensure!(
            !self.entries.is_empty()
                && self.entries.len() <= 6
                && self.entries.len() == self.entries.iter().collect::<BTreeSet<_>>().len()
                && self.entries.iter().all(|p| p.len() <= 80
                    && (loader.is_match(p) || p == "OptiScaler/plugins/MFGUnlocker.asi")),
            "推荐包含不支持的 DLL 入口"
        );
        ensure!(
            !self
                .entries
                .iter()
                .any(|p| p.to_lowercase().ends_with(".asi"))
                || self.entries.len() == 1,
            "插件与代理入口不能同时使用"
        );
        ensure!(
            self.patch_version.len() <= 40
                && regex::Regex::new(r"^\d+\.\d+\.\d+(?:[-+][a-zA-Z0-9.-]+)?$")?
                    .is_match(&self.patch_version),
            "推荐的补丁版本无效"
        );
        ensure!(
            ["dx12", "dx11", "vulkan", "unknown"].contains(&self.renderer.as_str())
                && ["unchanged", "rtx4060", "rtx5090", "unknown"]
                    .contains(&self.gpu_alias.as_str())
                && ["none", "310.9.16", "unknown"].contains(&self.neural_runtime.as_str()),
            "推荐包含未知选项"
        );
        Ok(())
    }
    /// Stable scheme IDs identify projects; version differences always remain visible.
    pub fn available_version(&self, catalog: &cloud::Catalog) -> Result<String> {
        self.validate(catalog)?;
        let payload = catalog.selected(&self.scheme);
        let current = payload
            .upstream_version
            .as_deref()
            .unwrap_or(&payload.version);
        ensure!(
            semver::Version::parse(current)? >= semver::Version::parse(&self.patch_version)?,
            "推荐使用的补丁比当前版本新，请先刷新资源或更新管理器"
        );
        Ok(current.to_owned())
    }
    pub fn required_gpu_name(&self) -> Option<&'static str> {
        match self.gpu_alias.as_str() {
            "rtx4060" => Some("NVIDIA GeForce RTX 4060"),
            "rtx5090" => Some("NVIDIA GeForce RTX 5090"),
            _ => None,
        }
    }
}
/// Read applied settings and runtime version resources. Known managed XeSS SDK
/// association checks its two owned DLL hashes; no game DLL is uploaded or run.
pub fn installed_recipe(
    game: &Game,
    identity: GameIdentity,
    catalog: &cloud::Catalog,
    gpu: String,
    driver: String,
) -> Result<Recipe> {
    let mut found: Option<Recipe> = None;
    for target in game.deployment_targets() {
        let record = core::record(&target.directory)?
            .context("请先为当前游戏安装补丁，再分享已测试的方案")?;
        target.validate_record(&record)?;
        ensure!(!record.cache_pending, "请先处理当前游戏的残留文件");
        let nr = neural_runtime::inspect(&target);
        let mut entries = record.selected();
        entries.sort();
        let current = Recipe {
            schema: 1,
            platform: platform(game).into(),
            edition: String::new(),
            game: identity.clone(),
            title: "我的可用方案".into(),
            notes: String::new(),
            scheme: record
                .scheme_id
                .clone()
                .context("此安装记录没有方案信息，请先升级补丁")?,
            patch_version: record
                .upstream_version
                .clone()
                .or(record.payload_version.clone())
                .context("此安装记录没有补丁版本")?,
            entries,
            renderer: "unknown".into(),
            gpu_model: gpu.clone(),
            driver: driver.clone(),
            gpu_alias: "unknown".into(),
            neural_runtime: if nr.present {
                nr.version
                    .filter(|v| v == "310.9.16")
                    .unwrap_or("unknown".into())
            } else {
                "none".into()
            },
            presets: None,
            runtimes: Some(crate::recommendation_runtimes::capture(&target)?),
        };
        let mut current = current;
        if current.scheme == crate::recommendation_presets::MFG_161_COMPAT {
            // Unrelated runtime files can remain after switching patches. They
            // must not become an installation requirement for this older MFG.
            current.neural_runtime = "none".into();
            if let Some(runtimes) = &mut current.runtimes {
                runtimes
                    .components
                    .retain(|c| c.kind != "nr" && c.source != "managed");
            }
        }
        current.presets = Some(crate::recommendation_presets::capture(
            &target,
            catalog,
            &current.scheme,
            &current.patch_version,
        )?);
        current.validate(catalog)?;
        if let Some(old) = &found {
            ensure!(
                old.scheme == current.scheme
                    && old.entries == current.entries
                    && old.patch_version == current.patch_version
                    && old.presets == current.presets
                    && old.runtimes == current.runtimes
                    && old.neural_runtime == current.neural_runtime,
                "多个目录安装的方案不一致，请先统一后再分享"
            );
        } else {
            found = Some(current);
        }
    }
    found.context("未找到游戏的部署目录")
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[non_exhaustive]
pub struct Card {
    pub id: String,
    pub author: String,
    pub revision: u64,
    pub updated_at: u64,
    #[serde(default)]
    pub created_at: Option<u64>,
    pub likes: u64,
    pub liked: bool,
    #[serde(default)]
    pub dislikes: u64,
    #[serde(default)]
    pub disliked: bool,
    #[serde(default)]
    pub reported: bool,
    pub mine: bool,
    pub status: String,
    pub content: Recipe,
}
impl Card {
    fn validate(&self, game: &str, _catalog: &cloud::Catalog) -> Result<()> {
        ensure!(
            self.content.game.id == game
                && uuid::Uuid::parse_str(&self.id).is_ok()
                && uuid::Uuid::parse_str(&self.author).is_ok()
                && self.revision > 0
                && !(self.liked && self.disliked)
                && ["visible", "hidden"].contains(&self.status.as_str()),
            "推荐不属于当前游戏或格式无效"
        );
        self.content.validate_public()
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[non_exhaustive]
pub struct Page {
    pub items: Vec<Card>,
    pub total: u64,
    pub next_offset: Option<u32>,
}
impl Page {
    fn validate(&self, id: &str, catalog: &cloud::Catalog) -> Result<()> {
        ensure!(
            self.items.len() <= 24
                && self.total <= 1000000
                && self.next_offset.is_none_or(|v| v <= 10000),
            "推荐列表格式无效"
        );
        for card in &self.items {
            card.validate(id, catalog)?;
        }
        Ok(())
    }
}
/// A full address takes precedence; a selected suffix only completes an account name.
pub fn complete_email(value: &str, domain: &str) -> String {
    let value = value.trim();
    if value.is_empty() || value.contains('@') || domain.is_empty() {
        value.to_owned()
    } else {
        format!("{value}@{domain}")
    }
}
pub fn rank(items: &mut [Card]) {
    // Match the server's global pagination order, including ties.
    items.sort_by(|a, b| {
        b.likes
            .cmp(&a.likes)
            .then(b.updated_at.cmp(&a.updated_at))
            .then(a.id.cmp(&b.id))
    });
}
pub fn date(timestamp: u64) -> Option<String> {
    chrono::DateTime::from_timestamp(i64::try_from(timestamp).ok()?, 0).map(|v| {
        v.with_timezone(&chrono::Local)
            .format("%Y-%m-%d")
            .to_string()
    })
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[non_exhaustive]
pub struct Session {
    token: String,
    pub expires_at: i64,
    pub user: User,
}
impl Drop for Session {
    fn drop(&mut self) {
        self.token.zeroize();
    }
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct User {
    pub id: String,
    pub admin: bool,
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[non_exhaustive]
pub struct Challenge {
    pub challenge: String,
    pub expires_in: u32,
    pub retry_after: u32,
}
#[derive(Clone)]
pub struct Client {
    network: Arc<Mutex<NetworkRoute>>,
    root: PathBuf,
    auth: Arc<Mutex<AuthState>>,
    #[cfg(test)]
    origin: String,
    #[cfg(test)]
    http_factory: Option<HttpFactory>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NetworkRoute {
    System,
    DirectIpv4,
    OriginIpv4,
}
#[cfg(test)]
type HttpFactory = Arc<dyn Fn(NetworkRoute) -> Result<Http> + Send + Sync>;
fn preferred_network(root: &Path) -> NetworkRoute {
    // Reuse only a recent successful route from our bounded diagnostic. The
    // file cannot supply a URL, proxy or IP; failures always try other routes.
    let read = || -> Option<NetworkRoute> {
        let report = core::read_json(&root.join("connection-status.json"), 4096).ok()?;
        let checked = chrono::DateTime::parse_from_rfc3339(report["checkedAt"].as_str()?).ok()?;
        let age = chrono::Utc::now()
            .signed_duration_since(checked)
            .num_seconds();
        if !(0..=7 * 86400).contains(&age) {
            return None;
        }
        let attempt = report["attempts"]
            .as_array()?
            .iter()
            .rev()
            .find(|a| a["outcome"] == "ok")?;
        match attempt["route"].as_str()? {
            "system" => Some(NetworkRoute::System),
            "direct_ipv4" => Some(NetworkRoute::DirectIpv4),
            "origin_ipv4" => Some(NetworkRoute::OriginIpv4),
            _ => None,
        }
    };
    read().unwrap_or(NetworkRoute::System)
}
#[derive(Default)]
struct AuthState {
    session: Option<Session>,
    expired: bool,
}
#[derive(Debug)]
enum ServiceError {
    LoginExpired,
    Unavailable,
    Connection,
    TimedOut,
    Gateway,
    Api(&'static str),
}
impl std::fmt::Display for ServiceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::LoginExpired => "登录已过期，请重新获取邮箱验证码",
            Self::Unavailable => "推荐服务暂时无法连接，请稍后刷新",
            Self::Connection => "连接推荐服务失败，请稍后刷新",
            Self::TimedOut => "连接推荐服务超时，请稍后刷新",
            Self::Gateway => "推荐服务返回非数据页面，请稍后刷新或联系维护者",
            Self::Api(message) => message,
        })
    }
}
impl std::error::Error for ServiceError {}

/// Only connection/server outages may be presented as offline browsing.
pub fn allows_offline_cache(error: &anyhow::Error) -> bool {
    matches!(
        error.downcast_ref::<ServiceError>(),
        Some(
            ServiceError::Unavailable
                | ServiceError::Connection
                | ServiceError::TimedOut
                | ServiceError::Gateway
        )
    )
}
pub fn offline_cache_notice(error: &anyhow::Error) -> &'static str {
    match error.downcast_ref::<ServiceError>() {
        Some(ServiceError::TimedOut) => "连接超时，暂显示缓存；请稍后刷新",
        Some(ServiceError::Connection) => "连接失败，暂显示缓存；请稍后刷新",
        Some(ServiceError::Gateway) => "服务响应异常，暂显示缓存；请稍后刷新或联系维护者",
        _ => "推荐服务暂时不可用，当前显示缓存，请稍后刷新",
    }
}
fn connection_attempt<T>(route: NetworkRoute, elapsed: Duration, result: &Result<T>) -> Value {
    let outcome = match result {
        Ok(_) => "ok",
        Err(e) => match e.downcast_ref::<ServiceError>() {
            Some(ServiceError::Connection) => "connection_failed",
            Some(ServiceError::TimedOut) => "timeout",
            Some(ServiceError::Gateway) => "non_json_response",
            Some(ServiceError::Unavailable) => "server_unavailable",
            Some(ServiceError::LoginExpired) => "login_expired",
            Some(ServiceError::Api(_)) => "api_rejected",
            None => "invalid_response",
        },
    };
    json!({
        "route": match route { NetworkRoute::System => "system", NetworkRoute::DirectIpv4 => "direct_ipv4", NetworkRoute::OriginIpv4 => "origin_ipv4" },
        "elapsedMs": elapsed.as_millis() as u64,
        "outcome": outcome,
    })
}
impl Client {
    pub fn new(data: &Path) -> Result<Self> {
        let root = data.join("recommendations");
        fs::create_dir_all(&root)?;
        let session_file = SESSION_FILE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let session = read_session(&root).ok().flatten();
        let expired = session
            .as_ref()
            .is_some_and(|s| s.expires_at <= chrono::Utc::now().timestamp());
        if expired {
            fs::remove_file(root.join("session.json"))?;
        }
        drop(session_file);
        Ok(Self {
            network: Arc::new(Mutex::new(preferred_network(&root))),
            root,
            auth: Arc::new(Mutex::new(AuthState {
                session: session.filter(|_| !expired),
                expired,
            })),
            #[cfg(test)]
            origin: ORIGIN.to_string(),
            #[cfg(test)]
            http_factory: None,
        })
    }
    pub fn logged_in(&self) -> bool {
        self.auth
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .session
            .as_ref()
            .is_some_and(|s| s.expires_at > chrono::Utc::now().timestamp())
    }
    pub fn login_expired(&self) -> bool {
        let auth = self.auth.lock().unwrap_or_else(|e| e.into_inner());
        auth.expired
            || auth
                .session
                .as_ref()
                .is_some_and(|s| s.expires_at <= chrono::Utc::now().timestamp())
    }
    fn active_session(&self) -> Result<Option<Session>> {
        let session = self
            .auth
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .session
            .clone();
        if let Some(s) = &session
            && s.expires_at <= chrono::Utc::now().timestamp()
        {
            self.clear_session(&s.token, true)?;
            return Ok(None);
        }
        Ok(session)
    }
    fn clear_session(&self, rejected_token: &str, expired: bool) -> Result<()> {
        let mut auth = self.auth.lock().unwrap_or_else(|e| e.into_inner());
        // A delayed response from an older request must not log out a new login.
        if auth
            .session
            .as_ref()
            .is_some_and(|s| s.token == rejected_token)
        {
            auth.session = None;
            auth.expired = expired;
            let _file_lock = SESSION_FILE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            if read_session(&self.root)?.is_some_and(|s| s.token == rejected_token) {
                fs::remove_file(self.root.join("session.json"))
                    .context("无法清除已失效的登录记录")?;
            }
        } else if !expired && auth.session.is_none() {
            auth.expired = false;
        }
        Ok(())
    }
    fn origin(&self) -> &str {
        #[cfg(test)]
        {
            &self.origin
        }
        #[cfg(not(test))]
        {
            ORIGIN
        }
    }
    fn request<T: DeserializeOwned>(
        &self,
        method: Method,
        route: &str,
        body: Option<Value>,
        authenticated: bool,
    ) -> Result<T> {
        ensure!(
            route.starts_with("/v1/") && !route.contains(['\r', '\n', '#']),
            "推荐请求无效"
        );
        let session = if authenticated {
            Some(self.active_session()?.ok_or(ServiceError::LoginExpired)?)
        } else {
            None
        };
        // Re-read the current proxy configuration and resolve DNS for every logical
        // request. A window must not retain a dead VPN proxy or pooled connection.
        // Avoid paying the same failed-route timeouts on every refresh/window.
        // Each route still rebuilds its transport and observes current settings.
        let network = *self.network.lock().unwrap_or_else(|e| e.into_inner());
        let started = std::time::Instant::now();
        let mut result = self.request_once(
            method.clone(),
            route,
            body.as_ref(),
            session.as_ref(),
            network,
        );
        let mut attempts = vec![connection_attempt(network, started.elapsed(), &result)];
        for fallback in [
            NetworkRoute::System,
            NetworkRoute::DirectIpv4,
            NetworkRoute::OriginIpv4,
        ]
        .into_iter()
        .filter(|r| *r != network)
        {
            let retry = method == Method::GET
                && result.as_ref().err().is_some_and(|error| {
                    matches!(
                        error.downcast_ref::<ServiceError>(),
                        Some(
                            ServiceError::Connection
                                | ServiceError::TimedOut
                                | ServiceError::Gateway
                        )
                    )
                });
            if !retry {
                break;
            }
            let started = std::time::Instant::now();
            result = self.request_once(
                method.clone(),
                route,
                body.as_ref(),
                session.as_ref(),
                fallback,
            );
            attempts.push(connection_attempt(fallback, started.elapsed(), &result));
        }
        if method == Method::GET {
            // Keep one bounded local connection report, with no URLs, game IDs,
            // email addresses, tokens, proxy settings or server response bodies.
            let _ = core::atomic_json(
                &self.root.join("connection-status.json"),
                &json!({
                    "version": crate::VERSION,
                    "checkedAt": chrono::Utc::now().to_rfc3339(),
                    "attempts": attempts,
                }),
            );
        }
        result
    }
    fn http(&self, network: NetworkRoute) -> Result<Http> {
        #[cfg(test)]
        if let Some(factory) = &self.http_factory {
            return factory(network);
        }
        let builder = Http::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(15))
            .http1_only()
            .retry(reqwest::retry::never())
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(format!("RTXFG/{}", crate::VERSION));
        // This only bypasses application proxies. A TUN VPN still owns OS routing.
        let builder = match network {
            NetworkRoute::System => builder,
            NetworkRoute::DirectIpv4 => builder
                .no_proxy()
                .local_address(std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED)),
            NetworkRoute::OriginIpv4 => builder
                .no_proxy()
                .local_address(std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED))
                .resolve(
                    ORIGIN_HOST,
                    std::net::SocketAddr::new(ORIGIN_IPV4.into(), 443),
                ),
        };
        builder.build().map_err(|_| ServiceError::Connection.into())
    }
    fn request_once<T: DeserializeOwned>(
        &self,
        method: Method,
        route: &str,
        body: Option<&Value>,
        session: Option<&Session>,
        network: NetworkRoute,
    ) -> Result<T> {
        let mut request = self
            .http(network)?
            .request(method.clone(), format!("{}{route}", self.origin()))
            .header("X-RTXFG-Feedback", "10")
            .header("Accept", "application/json")
            .header("Cache-Control", "no-cache");
        if method == Method::GET {
            // Three read routes remain bounded. Writes retain the former 15 s
            // timeout for SMTP/API work and are never replayed automatically.
            request = request.timeout(Duration::from_secs(8));
        }
        if let Some(s) = &session {
            request = request.bearer_auth(&s.token);
        }
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request.send().map_err(|e| {
            if e.is_timeout() {
                ServiceError::TimedOut
            } else {
                ServiceError::Connection
            }
        })?;
        let status = response.status();
        let json_response = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|h| h.to_str().ok())
            .is_some_and(|h| {
                h.split(';')
                    .next()
                    .unwrap_or("")
                    .trim()
                    .eq_ignore_ascii_case("application/json")
            });
        // A CDN challenge or error page is not an API authorization response.
        // Never discard an existing login because an edge returned HTML.
        if !json_response {
            return Err(ServiceError::Gateway.into());
        }
        if status == reqwest::StatusCode::UNAUTHORIZED
            && let Some(s) = &session
        {
            self.clear_session(&s.token, true)?;
            return Err(ServiceError::LoginExpired.into());
        }
        let mut bytes = Vec::new();
        response
            .take(RESPONSE_LIMIT + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| ServiceError::Connection)?;
        ensure!(
            bytes.len() <= RESPONSE_LIMIT as usize,
            "推荐服务返回内容过大"
        );
        if !status.is_success() {
            if status.is_server_error() {
                return Err(ServiceError::Unavailable.into());
            }
            let value: Value = serde_json::from_slice(&bytes).unwrap_or_default();
            return Err(ServiceError::Api(error_message(
                value["error"].as_str().unwrap_or("unavailable"),
            ))
            .into());
        }
        let value = serde_json::from_slice(&bytes).context("推荐服务返回格式无效")?;
        // Writes use the last route that actually returned valid service data,
        // but are never replayed after an ambiguous failure (email/votes/shares).
        *self.network.lock().unwrap_or_else(|e| e.into_inner()) = network;
        Ok(value)
    }
    pub fn browse(
        &self,
        game: &GameIdentity,
        catalog: &cloud::Catalog,
        offset: u32,
        mine: bool,
        reported: bool,
    ) -> Result<Page> {
        game.validate()?;
        let query = reqwest::Url::parse_with_params(
            &format!("{ORIGIN}/v1/recipes"),
            &[
                ("game", game.id.clone()),
                ("offset", offset.to_string()),
                ("mine", if mine { "1" } else { "0" }.into()),
                ("reported", if reported { "1" } else { "0" }.into()),
            ],
        )?;
        let route = format!("{}?{}", query.path(), query.query().unwrap_or_default());
        let authenticated = self.active_session()?.is_some() || mine || reported;
        let page: Page = match self.request(Method::GET, &route, None, authenticated) {
            Err(e)
                if !mine
                    && !reported
                    && matches!(
                        e.downcast_ref::<ServiceError>(),
                        Some(ServiceError::LoginExpired)
                    ) =>
            {
                // Public browsing never depends on a retained account session.
                // Retry once, anonymously; never replay writes or private filters.
                self.request(Method::GET, &route, None, false)?
            }
            result => result?,
        };
        page.validate(&game.id, catalog)?;
        if offset == 0 && !mine && !reported {
            let _ = core::atomic_json(&self.cache_path(&game.id), &page);
        }
        Ok(page)
    }
    fn cache_path(&self, id: &str) -> PathBuf {
        self.root
            .join(format!("{:x}.setup-v5.json", Sha256::digest(id.as_bytes())))
    }
    pub fn cached(&self, game: &GameIdentity, catalog: &cloud::Catalog) -> Option<Page> {
        let b = fs::read(self.cache_path(&game.id))
            .or_else(|_| {
                fs::read(self.root.join(format!(
                    "{:x}.setup-v4.json",
                    Sha256::digest(game.id.as_bytes())
                )))
            })
            .or_else(|_| {
                fs::read(self.root.join(format!(
                    "{:x}.setup-v2.json",
                    Sha256::digest(game.id.as_bytes())
                )))
            })
            .or_else(|_| {
                fs::read(self.root.join(format!(
                    "{:x}.presets-v1.json",
                    Sha256::digest(game.id.as_bytes())
                )))
            })
            .or_else(|_| {
                fs::read(
                    self.root
                        .join(format!("{:x}.json", Sha256::digest(game.id.as_bytes()))),
                )
            })
            .ok()?;
        if b.len() > RESPONSE_LIMIT as usize {
            return None;
        }
        let mut p: Page = serde_json::from_slice(&b).ok()?;
        p.validate(&game.id, catalog).ok()?;
        for c in &mut p.items {
            c.mine = false;
            c.liked = false;
            c.disliked = false;
            c.reported = false;
        }
        Some(p)
    }
    pub fn send_code(&self, email: &str) -> Result<Challenge> {
        self.request(
            Method::POST,
            "/v1/auth/code",
            Some(json!({"email":email})),
            false,
        )
    }
    pub fn verify(&mut self, challenge: &str, code: &str) -> Result<()> {
        let s: Session = self.request(
            Method::POST,
            "/v1/auth/verify",
            Some(json!({"challenge":challenge,"code":code})),
            false,
        )?;
        ensure!(
            s.token.len() == 43
                && s.token
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b)),
            "登录凭据格式无效"
        );
        let bytes = Zeroizing::new(serde_json::to_vec(&s)?);
        let protected = unseal(&bytes, true)?;
        let path = self.root.join("session.json");
        let mut auth = self.auth.lock().unwrap_or_else(|e| e.into_inner());
        let _file_lock = SESSION_FILE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        core::atomic_json(&path, &*protected)?;
        auth.session = Some(s);
        auth.expired = false;
        Ok(())
    }
    pub fn logout(&mut self) -> Result<()> {
        let session = self.active_session()?;
        if let Some(session) = session {
            // Signing out locally still works during an outage. Persistence errors
            // are propagated so the UI never claims a retained login was removed.
            let _ = self.request::<Value>(Method::POST, "/v1/auth/logout", Some(json!({})), true);
            self.clear_session(&session.token, false)?;
        }
        Ok(())
    }
    pub fn share(
        &self,
        submission: &str,
        recipe: &Recipe,
        catalog: &cloud::Catalog,
    ) -> Result<Card> {
        recipe.validate(catalog)?;
        let card: Card = self.request(
            Method::POST,
            "/v1/recipes",
            Some(json!({"submission":submission,"content":recipe})),
            true,
        )?;
        card.validate(&recipe.game.id, catalog)?;
        Ok(card)
    }
    pub fn vote(&self, card: &Card, value: i8, catalog: &cloud::Catalog) -> Result<Card> {
        ensure!([-1, 0, 1].contains(&value), "推荐评价无效");
        card.validate(&card.content.game.id, catalog)?;
        let updated: Card = self.request(
            Method::PUT,
            &recipe_route(card, "vote")?,
            Some(json!({"revision":card.revision,"value":value})),
            true,
        )?;
        updated.validate(&card.content.game.id, catalog)?;
        ensure!(
            updated.id == card.id && updated.revision == card.revision,
            "推荐内容已更新，请刷新"
        );
        Ok(updated)
    }
    pub fn update_share(
        &self,
        card: &Card,
        recipe: &Recipe,
        catalog: &cloud::Catalog,
    ) -> Result<Card> {
        card.validate(&card.content.game.id, catalog)?;
        recipe.validate(catalog)?;
        ensure!(card.mine, "只能修改自己的分享");
        ensure!(
            recipe.game.id == card.content.game.id,
            "不能更换已分享方案的游戏"
        );
        let updated: Card = self.request(
            Method::PUT,
            &recipe_route(card, "")?,
            Some(json!({"revision":card.revision,"content":recipe})),
            true,
        )?;
        updated.validate(&recipe.game.id, catalog)?;
        ensure!(
            updated.id == card.id && updated.revision == card.revision.saturating_add(1),
            "推荐内容已更新，请刷新"
        );
        Ok(updated)
    }
    pub fn remove(&self, card: &Card) -> Result<()> {
        let _: Value = self.request(
            Method::DELETE,
            &recipe_route(card, "")?,
            Some(json!({"revision":card.revision})),
            true,
        )?;
        Ok(())
    }
    pub fn report(
        &self,
        card: &Card,
        reason: Option<&str>,
        catalog: &cloud::Catalog,
    ) -> Result<Card> {
        ensure!(!card.mine, "不能举报自己的方案");
        let updated: Card = self.request(
            if reason.is_some() {
                Method::PUT
            } else {
                Method::DELETE
            },
            &recipe_route(card, "report")?,
            Some(reason.map_or_else(|| json!({}), |reason| json!({"reason":reason}))),
            true,
        )?;
        updated.validate(&card.content.game.id, catalog)?;
        ensure!(
            updated.id == card.id && updated.reported == reason.is_some(),
            "举报状态无效"
        );
        Ok(updated)
    }
    pub fn search(&self, name: &str) -> Result<Vec<GameCandidate>> {
        #[derive(Deserialize)]
        struct Hit {
            id: String,
            name: String,
            #[serde(default)]
            original: String,
            #[serde(default)]
            keys: Vec<String>,
        }
        #[derive(Deserialize)]
        struct Hits {
            items: Vec<Hit>,
        }
        let url = reqwest::Url::parse_with_params(&format!("{ORIGIN}/v1/games"), &[("q", name)])?;
        let hits: Hits = self.request(
            Method::GET,
            &format!("/v1/games?{}", url.query().unwrap_or_default()),
            None,
            false,
        )?;
        ensure!(hits.items.len() <= 20, "游戏候选过多");
        hits.items
            .into_iter()
            .map(|h| {
                public_text(&h.original, 160, true)?;
                ensure!(h.keys.len() <= 512, "游戏候选过多");
                Ok(GameCandidate {
                    identity: GameIdentity::new(h.id, h.name)?,
                    original: h.original,
                    platforms: candidate_platforms(&h.keys),
                })
            })
            .collect()
    }
}
fn recipe_route(card: &Card, action: &str) -> Result<String> {
    let suffix = if action.is_empty() {
        String::new()
    } else {
        format!("/{action}")
    };
    let url = reqwest::Url::parse_with_params(
        &format!("{ORIGIN}/v1/recipes/{}{suffix}", card.id),
        &[("game", &card.content.game.id)],
    )?;
    Ok(format!(
        "{}?{}",
        url.path(),
        url.query().unwrap_or_default()
    ))
}
fn error_message(code: &str) -> &'static str {
    match code {
        "login_required" | "session_expired" => "登录已过期，请重新获取邮箱验证码",
        "invalid_or_expired_code" | "invalid_code" => "验证码不正确或已过期，请重新获取",
        "invalid_email" => "请输入有效邮箱地址",
        "mail_unavailable" => "验证码暂时无法发送，请稍后再试",
        "rate_limited" | "wait_before_resending" => "操作过于频繁，请稍后再试",
        "five_recipes_per_game" => "每款游戏最多分享 5 个方案，请先移除旧方案",
        "revision_changed" => "此方案已更新，请刷新后再操作",
        "game_mismatch" | "game_cannot_change" => "游戏关联已变化，请刷新后重新选择",
        "private_or_unsafe_text" => "备注不能包含路径、链接或敏感信息",
        "invalid_preset_protocol" | "unsupported_preset_version" => {
            "分享参数与补丁版本不匹配，请重新读取已安装方案"
        }
        "invalid_preset_snapshot" | "invalid_preset_value" | "invalid_preset_combination" => {
            "分享参数不完整或数值无效，请检查当前游戏设置"
        }
        "not_found" => "此方案已被移除或隐藏，请刷新",
        "own_recipe_like" => "不能评价自己的方案",
        "own_recipe_report" => "不能举报自己的方案",
        "report_reviewed" => "管理员已审核此版本，请勿重复举报",
        _ => "推荐服务暂时不可用，请稍后再试",
    }
}
fn unseal(input: &[u8], protect: bool) -> Result<Zeroizing<Vec<u8>>> {
    use windows::{
        Win32::{
            Foundation::{HLOCAL, LocalFree},
            Security::Cryptography::{
                CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
            },
        },
        core::PCWSTR,
    };
    ensure!(!input.is_empty() && input.len() <= 16384, "登录凭据无效");
    let source = CRYPT_INTEGER_BLOB {
        cbData: input.len() as u32,
        pbData: input.as_ptr().cast_mut(),
    };
    let mut output = CRYPT_INTEGER_BLOB::default();
    // SAFETY: DPAPI owns the returned buffer; caller-owned source outlives both calls.
    unsafe {
        if protect {
            CryptProtectData(
                &source,
                PCWSTR::null(),
                None,
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        } else {
            CryptUnprotectData(
                &source,
                None,
                None,
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        }
    }
    .context("Windows 无法读取登录凭据，请重新登录")?;
    // SAFETY: successful DPAPI allocation is copied, wiped and released exactly once.
    let bytes = unsafe { std::slice::from_raw_parts_mut(output.pbData, output.cbData as usize) };
    let result = Zeroizing::new(bytes.to_vec());
    bytes.zeroize();
    // SAFETY: output.pbData is the buffer returned by DPAPI, not a Rust allocation.
    unsafe {
        let _ = LocalFree(Some(HLOCAL(output.pbData.cast())));
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};

    fn mock_service(replies: Vec<(u16, Value)>) -> (String, std::thread::JoinHandle<Vec<String>>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let worker = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(8);
            let mut requests = Vec::new();
            for (status, body) in replies {
                let mut stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            assert!(
                                std::time::Instant::now() < deadline,
                                "Expected request did not arrive"
                            );
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        Err(e) => panic!("{e}"),
                    }
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut reader = BufReader::new(&mut stream);
                let mut request = String::new();
                let mut length = 0;
                loop {
                    let mut line = String::new();
                    assert!(reader.read_line(&mut line).unwrap() > 0);
                    if line == "\r\n" {
                        break;
                    }
                    if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        length = value.trim().parse::<usize>().unwrap();
                    }
                    request.push_str(&line);
                }
                let mut body_bytes = vec![0; length];
                reader.read_exact(&mut body_bytes).unwrap();
                requests.push(request);
                if status == 0 {
                    // Server received the request but the response was lost.
                    continue;
                }
                let (content_type, body) = match body.as_str() {
                    Some(html) => ("text/html", html.as_bytes().to_vec()),
                    None => ("application/json", serde_json::to_vec(&body).unwrap()),
                };
                write!(stream, "HTTP/1.1 {status} Mock\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).unwrap();
                stream.write_all(&body).unwrap();
            }
            requests
        });
        (origin, worker)
    }
    fn save_test_session(data: &Path, token: &str, expires_at: i64) {
        let root = data.join("recommendations");
        fs::create_dir_all(&root).unwrap();
        let session = json!({"token": token, "expiresAt": expires_at,
            "user": {"id": uuid::Uuid::new_v4().to_string(), "admin": false}});
        let protected = unseal(&serde_json::to_vec(&session).unwrap(), true).unwrap();
        core::atomic_json(&root.join("session.json"), &*protected).unwrap();
    }
    fn online_test_client(data: &Path, origin: String) -> Client {
        let mut client = Client::new(data).unwrap();
        client.http_factory = Some(Arc::new(|_| {
            Ok(Http::builder()
                .no_proxy()
                .retry(reqwest::retry::never())
                .timeout(Duration::from_secs(3))
                .build()?)
        }));
        client.origin = origin;
        client
    }
    #[test]
    fn compatibility_sharing_cache_keeps_4314_offline_records_and_uses_new_protocol() {
        let data = tempfile::tempdir().unwrap();
        let old = card("RTX 3060", "581.80", 1);
        let game = old.content.game.clone();
        let page = json!({"items":[old],"total":1,"nextOffset":null});
        let (origin, server) = mock_service(vec![(200, page.clone())]);
        let client = online_test_client(data.path(), origin);
        let previous = client.root.join(format!(
            "{:x}.setup-v4.json",
            Sha256::digest(game.id.as_bytes())
        ));
        core::atomic_json(&previous, &page).unwrap();
        assert_eq!(client.cached(&game, &cloud::bundled()).unwrap().total, 1);
        client
            .browse(&game, &cloud::bundled(), 0, false, false)
            .unwrap();
        assert!(client.cache_path(&game.id).is_file());
        assert_eq!(
            serde_json::from_slice::<Value>(&fs::read(previous).unwrap()).unwrap(),
            page
        );
        assert!(
            server.join().unwrap()[0]
                .to_ascii_lowercase()
                .contains("x-rtxfg-feedback: 10")
        );
    }
    #[test]
    fn recommendation_read_recovers_from_unreachable_proxy() {
        let data = tempfile::tempdir().unwrap();
        let game = card("RTX 3060", "581.80", 1).content.game;
        let (origin, server) =
            mock_service(vec![(200, json!({"items":[],"total":0,"nextOffset":null}))]);
        let mut client = online_test_client(data.path(), origin);
        let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let proxy = format!("http://{}", closed.local_addr().unwrap());
        drop(closed);
        client.http_factory = Some(Arc::new(move |route| {
            let builder = Http::builder()
                .timeout(Duration::from_secs(2))
                .retry(reqwest::retry::never());
            Ok(match route {
                NetworkRoute::System => builder.proxy(reqwest::Proxy::all(&proxy)?),
                NetworkRoute::DirectIpv4 | NetworkRoute::OriginIpv4 => builder.no_proxy(),
            }
            .build()?)
        }));
        assert!(
            client
                .browse(&game, &cloud::bundled(), 0, false, false)
                .is_ok()
        );
        assert_eq!(*client.network.lock().unwrap(), NetworkRoute::DirectIpv4);
        assert_eq!(server.join().unwrap().len(), 1);
    }
    #[test]
    fn edge_html_does_not_log_out_or_replace_cache_and_retry_is_bounded() {
        for status in [200, 401, 403, 502] {
            let data = tempfile::tempdir().unwrap();
            let token = "s".repeat(43);
            save_test_session(data.path(), &token, chrono::Utc::now().timestamp() + 86400);
            let game = card("RTX 3060", "581.80", 1).content.game;
            let (origin, server) = mock_service(vec![
                (status, json!("<html>Challenge</html>")),
                (status, json!("<html>Challenge</html>")),
                (status, json!("<html>Challenge</html>")),
                (200, json!({"items":[],"total":0,"nextOffset":null})),
            ]);
            let client = online_test_client(data.path(), origin);
            let error = client
                .browse(&game, &cloud::bundled(), 0, false, false)
                .unwrap_err();
            assert!(allows_offline_cache(&error));
            assert!(client.logged_in());
            assert!(client.cached(&game, &cloud::bundled()).is_none());
            let report = fs::read_to_string(client.root.join("connection-status.json")).unwrap();
            assert!(
                !report.contains(&token)
                    && !report.contains("Authorization")
                    && !report.contains("Challenge")
            );
            let report: Value = serde_json::from_str(&report).unwrap();
            assert_eq!(report["attempts"].as_array().unwrap().len(), 3);
            assert_eq!(report["attempts"][1]["outcome"], "non_json_response");
            assert!(
                client
                    .browse(&game, &cloud::bundled(), 0, false, false)
                    .is_ok()
            );
            assert!(client.logged_in());
            assert_eq!(server.join().unwrap().len(), 4);
        }
    }
    #[test]
    fn refresh_rebuilds_transport_and_recovers_with_439_anonymous_cache_retained() {
        let data = tempfile::tempdir().unwrap();
        let old = card("RTX 3060", "581.80", 1);
        let game = old.content.game.clone();
        let latest = card("RTX 5090", "581.80", 9);
        let latest_id = latest.id.clone();
        let (origin, server) = mock_service(vec![
            (503, json!({"error":"unavailable"})),
            (200, json!({"items":[latest],"total":1,"nextOffset":null})),
        ]);
        let mut client = online_test_client(data.path(), origin);
        let old_path = client
            .root
            .join(format!("{:x}.json", Sha256::digest(game.id.as_bytes())));
        let legacy =
            serde_json::to_vec(&json!({"items":[old],"total":1,"nextOffset":null})).unwrap();
        fs::write(&old_path, &legacy).unwrap();
        let connections = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let count = connections.clone();
        client.http_factory = Some(Arc::new(move |_| {
            count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(Http::builder()
                .no_proxy()
                .timeout(Duration::from_secs(3))
                .build()?)
        }));
        let error = client
            .browse(&game, &cloud::bundled(), 0, false, false)
            .unwrap_err();
        assert!(allows_offline_cache(&error));
        assert_eq!(
            client.cached(&game, &cloud::bundled()).unwrap().items[0].id,
            old.id
        );
        let page = client
            .browse(&game, &cloud::bundled(), 0, false, false)
            .unwrap();
        assert_eq!(page.items[0].id, latest_id);
        assert_eq!(
            client.cached(&game, &cloud::bundled()).unwrap().items[0].id,
            latest_id
        );
        assert_eq!(connections.load(std::sync::atomic::Ordering::SeqCst), 2);
        assert_eq!(fs::read(old_path).unwrap(), legacy);
        assert!(!client.logged_in() && !client.root.join("session.json").exists());
        let requests = server.join().unwrap();
        assert!(
            requests
                .iter()
                .all(|r| !r.to_lowercase().contains("authorization:"))
        );
    }
    #[test]
    fn read_prefers_last_success_and_write_uses_it_without_replay() {
        let data = tempfile::tempdir().unwrap();
        let (origin, server) = mock_service(vec![
            (200, json!({"ok":true})),
            (0, json!({})),
            (200, json!({"ok":true})),
        ]);
        let mut client = online_test_client(data.path(), origin);
        let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let proxy = format!("http://{}", closed.local_addr().unwrap());
        drop(closed);
        let routes = Arc::new(Mutex::new(Vec::new()));
        let seen = routes.clone();
        client.http_factory = Some(Arc::new(move |route| {
            let mut seen = seen.lock().unwrap();
            let first = seen.is_empty();
            seen.push(route);
            let builder = Http::builder()
                .retry(reqwest::retry::never())
                .timeout(Duration::from_secs(2));
            Ok(if first {
                builder.proxy(reqwest::Proxy::all(&proxy)?)
            } else {
                builder.no_proxy()
            }
            .build()?)
        }));
        client
            .request::<Value>(Method::GET, "/v1/games", None, false)
            .unwrap();
        assert!(
            client
                .request::<Value>(Method::POST, "/v1/auth/code", Some(json!({})), false)
                .is_err()
        );
        client
            .request::<Value>(Method::GET, "/v1/games", None, false)
            .unwrap();
        assert_eq!(
            *routes.lock().unwrap(),
            vec![
                NetworkRoute::System,
                NetworkRoute::DirectIpv4,
                NetworkRoute::DirectIpv4,
                NetworkRoute::DirectIpv4
            ]
        );
        assert_eq!(*client.network.lock().unwrap(), NetworkRoute::DirectIpv4);
        let requests = server.join().unwrap();
        assert_eq!(requests.len(), 3);
        assert_eq!(requests.iter().filter(|r| r.starts_with("POST")).count(), 1);
    }
    #[test]
    #[ignore = "read-only live recommendation API; no account, votes, email or DLL downloads"]
    fn live_recommendation_connection() {
        let data = tempfile::tempdir().unwrap();
        let client = Client::new(data.path()).unwrap();
        let game = card("RTX 3060", "581.80", 1).content.game;
        let page = client
            .browse(&game, &cloud::bundled(), 0, false, false)
            .unwrap();
        println!(
            "Public recommendations read and validated: {} records",
            page.total
        );
        let origin: Page = client
            .request_once(
                Method::GET,
                "/v1/recipes?game=steam%3A292030&offset=0&mine=0&reported=0",
                None,
                None,
                NetworkRoute::OriginIpv4,
            )
            .unwrap();
        assert_eq!(origin.total, page.total);
        println!(
            "Origin fallback read with original HTTPS authority and certificate verification: {} records",
            origin.total
        );
        core::atomic_json(
            &client.root.join("connection-status.json"),
            &json!({
                "checkedAt":chrono::Utc::now().to_rfc3339(),
                "attempts":[{"route":"origin_ipv4","elapsedMs":83,"outcome":"ok"}]
            }),
        )
        .unwrap();
        let reopened = Client::new(data.path()).unwrap();
        let start = std::time::Instant::now();
        reopened
            .browse(&game, &cloud::bundled(), 0, false, false)
            .unwrap();
        println!(
            "Reopened client preferred-route read: {} ms",
            start.elapsed().as_millis()
        );
        let report = core::read_json(&client.root.join("connection-status.json"), 4096).unwrap();
        assert_eq!(report["attempts"].as_array().unwrap().len(), 1);
        assert_eq!(report["attempts"][0]["route"], "origin_ipv4");
    }
    #[test]
    fn successful_route_survives_reopen_and_recovers_after_network_change() {
        let data = tempfile::tempdir().unwrap();
        let root = data.path().join("recommendations");
        fs::create_dir_all(&root).unwrap();
        core::atomic_json(
            &root.join("connection-status.json"),
            &json!({
                "version":"4.3.12", "checkedAt":chrono::Utc::now().to_rfc3339(),
                "attempts":[{"route":"system","elapsedMs":5014,"outcome":"timeout"},
                    {"route":"direct_ipv4","elapsedMs":5010,"outcome":"timeout"},
                    {"route":"origin_ipv4","elapsedMs":83,"outcome":"ok"}]
            }),
        )
        .unwrap();
        let (origin, server) =
            mock_service(vec![(200, json!({"ok":true})), (200, json!({"ok":true}))]);
        let routes = Arc::new(Mutex::new(Vec::new()));
        let seen = routes.clone();
        let mut client = online_test_client(data.path(), origin.clone());
        client.http_factory = Some(Arc::new(move |route| {
            seen.lock().unwrap().push(route);
            assert_eq!(
                route,
                NetworkRoute::OriginIpv4,
                "Must skip the two known timeouts"
            );
            Ok(Http::builder()
                .no_proxy()
                .timeout(Duration::from_secs(2))
                .build()?)
        }));
        client
            .request::<Value>(Method::GET, "/v1/games", None, false)
            .unwrap();
        drop(client);
        let mut reopened = online_test_client(data.path(), origin);
        let seen = routes.clone();
        reopened.http_factory = Some(Arc::new(move |route| {
            seen.lock().unwrap().push(route);
            if route == NetworkRoute::OriginIpv4 {
                return Err(ServiceError::Connection.into());
            }
            Ok(Http::builder()
                .no_proxy()
                .timeout(Duration::from_secs(2))
                .build()?)
        }));
        reopened
            .request::<Value>(Method::GET, "/v1/games", None, false)
            .unwrap();
        assert_eq!(
            *routes.lock().unwrap(),
            vec![
                NetworkRoute::OriginIpv4,
                NetworkRoute::OriginIpv4,
                NetworkRoute::System
            ]
        );
        assert_eq!(
            *Client::new(data.path()).unwrap().network.lock().unwrap(),
            NetworkRoute::System
        );
        assert_eq!(server.join().unwrap().len(), 2);
    }
    #[test]
    fn saved_route_rejects_unknown_stale_future_failed_and_oversized_hints() {
        let data = tempfile::tempdir().unwrap();
        let root = data.path().join("recommendations");
        fs::create_dir_all(&root).unwrap();
        for (age, route, outcome) in [
            (0, "https://other.invalid", "ok"),
            (8 * 86400, "origin_ipv4", "ok"),
            (-86400, "origin_ipv4", "ok"),
            (0, "origin_ipv4", "timeout"),
        ] {
            core::atomic_json(
                &root.join("connection-status.json"),
                &json!({
                    "checkedAt":(chrono::Utc::now()-chrono::Duration::seconds(age)).to_rfc3339(),
                    "attempts":[{"route":route,"outcome":outcome}]
                }),
            )
            .unwrap();
            assert_eq!(
                *Client::new(data.path()).unwrap().network.lock().unwrap(),
                NetworkRoute::System
            );
        }
        for invalid in ["not json".to_string(), " ".repeat(4097)] {
            fs::write(root.join("connection-status.json"), invalid).unwrap();
            assert_eq!(
                *Client::new(data.path()).unwrap().network.lock().unwrap(),
                NetworkRoute::System
            );
        }
    }
    #[test]
    fn read_recovers_from_failed_dns_routes_and_does_not_replay_origin_write() {
        let data = tempfile::tempdir().unwrap();
        let (origin, server) = mock_service(vec![(200, json!({"ok":true})), (0, json!({}))]);
        let mut client = online_test_client(data.path(), origin);
        let routes = Arc::new(Mutex::new(Vec::new()));
        let seen = routes.clone();
        client.http_factory = Some(Arc::new(move |route| {
            seen.lock().unwrap().push(route);
            if route != NetworkRoute::OriginIpv4 {
                return Err(ServiceError::Connection.into());
            }
            Ok(Http::builder()
                .no_proxy()
                .retry(reqwest::retry::never())
                .timeout(Duration::from_secs(2))
                .build()?)
        }));
        client
            .request::<Value>(Method::GET, "/v1/games", None, false)
            .unwrap();
        assert_eq!(*client.network.lock().unwrap(), NetworkRoute::OriginIpv4);
        assert!(
            client
                .request::<Value>(Method::POST, "/v1/auth/code", Some(json!({})), false)
                .is_err()
        );
        assert_eq!(
            *routes.lock().unwrap(),
            vec![
                NetworkRoute::System,
                NetworkRoute::DirectIpv4,
                NetworkRoute::OriginIpv4,
                NetworkRoute::OriginIpv4
            ]
        );
        let requests = server.join().unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests.iter().filter(|r| r.starts_with("POST")).count(), 1);
    }
    #[test]
    fn retained_revoked_login_refreshes_public_cards_and_clears_saved_login() {
        let data = tempfile::tempdir().unwrap();
        save_test_session(
            data.path(),
            &"a".repeat(43),
            chrono::Utc::now().timestamp() + 86400,
        );
        let new_card = card("RTX 5090", "581.80", 4);
        let page = json!({"items":[new_card],"total":1,"nextOffset":null});
        let (origin, server) = mock_service(vec![
            (401, json!({"error":"session_expired"})),
            (200, page.clone()),
            (200, page),
        ]);
        let client = online_test_client(data.path(), origin);
        let worker_client = client.clone();
        let game = GameIdentity::new("steam:292030".into(), "The Witcher 3".into()).unwrap();
        core::atomic_json(
            &client.cache_path(&game.id),
            &json!({"items":[],"total":0,"nextOffset":null}),
        )
        .unwrap();
        let fresh = worker_client
            .browse(&game, &cloud::bundled(), 0, false, false)
            .unwrap();
        assert_eq!(fresh.items[0].id, new_card.id);
        assert!(!client.logged_in());
        assert!(client.login_expired());
        assert!(!client.root.join("session.json").exists());
        assert!(!Client::new(data.path()).unwrap().logged_in());
        assert_eq!(client.cached(&game, &cloud::bundled()).unwrap().total, 1);
        client
            .browse(&game, &cloud::bundled(), 12, false, false)
            .unwrap();
        let requests = server.join().unwrap();
        assert!(
            requests[0]
                .to_ascii_lowercase()
                .contains("authorization: bearer ")
        );
        assert!(!requests[1].to_ascii_lowercase().contains("authorization:"));
        assert!(!requests[2].to_ascii_lowercase().contains("authorization:"));
    }
    #[test]
    fn valid_login_keeps_personal_feedback_and_saved_session() {
        let data = tempfile::tempdir().unwrap();
        save_test_session(
            data.path(),
            &"b".repeat(43),
            chrono::Utc::now().timestamp() + 86400,
        );
        let mut voted = card("RTX 3060", "581.80", 2);
        voted.liked = true;
        let (origin, server) = mock_service(vec![(
            200,
            json!({"items":[voted],"total":1,"nextOffset":null}),
        )]);
        let client = online_test_client(data.path(), origin);
        let result = client
            .browse(&voted.content.game, &cloud::bundled(), 0, false, false)
            .unwrap();
        assert!(result.items[0].liked && client.logged_in());
        assert!(!client.login_expired());
        assert!(client.root.join("session.json").exists());
        assert_eq!(server.join().unwrap().len(), 1);
    }
    #[test]
    fn private_filters_and_writes_never_retry_anonymously_or_use_cache() {
        for operation in ["mine", "reported", "vote"] {
            let data = tempfile::tempdir().unwrap();
            save_test_session(
                data.path(),
                &"c".repeat(43),
                chrono::Utc::now().timestamp() + 86400,
            );
            let (origin, server) = mock_service(vec![(401, json!({"error":"session_expired"}))]);
            let client = online_test_client(data.path(), origin);
            let c = card("RTX 3060", "581.80", 1);
            let error = if operation == "vote" {
                client.vote(&c, 1, &cloud::bundled()).unwrap_err()
            } else {
                client
                    .browse(
                        &c.content.game,
                        &cloud::bundled(),
                        0,
                        operation == "mine",
                        operation == "reported",
                    )
                    .unwrap_err()
            };
            assert!(matches!(
                error.downcast_ref::<ServiceError>(),
                Some(ServiceError::LoginExpired)
            ));
            assert!(!allows_offline_cache(&error));
            assert!(!client.logged_in());
            assert_eq!(server.join().unwrap().len(), 1);
        }
    }
    #[test]
    fn only_service_outages_allow_offline_cache_without_discarding_login() {
        for (status, body, offline) in [
            (503, json!({"error":"unavailable"}), true),
            (403, json!({"error":"forbidden"}), false),
            (429, json!({"error":"rate_limited"}), false),
            (200, json!({"invalid":"page"}), false),
        ] {
            let data = tempfile::tempdir().unwrap();
            save_test_session(
                data.path(),
                &"d".repeat(43),
                chrono::Utc::now().timestamp() + 86400,
            );
            let (origin, server) = mock_service(vec![(status, body)]);
            let client = online_test_client(data.path(), origin);
            let game = card("RTX 3060", "581.80", 1).content.game;
            let error = client
                .browse(&game, &cloud::bundled(), 0, false, false)
                .unwrap_err();
            assert_eq!(allows_offline_cache(&error), offline);
            assert!(client.logged_in());
            assert_eq!(server.join().unwrap().len(), 1);
        }
    }
    #[test]
    fn local_expiry_and_late_rejection_cannot_restore_or_clear_the_wrong_session() {
        let data = tempfile::tempdir().unwrap();
        save_test_session(
            data.path(),
            &"e".repeat(43),
            chrono::Utc::now().timestamp() - 1,
        );
        let client = Client::new(data.path()).unwrap();
        assert!(!client.logged_in() && client.login_expired());
        assert!(!client.root.join("session.json").exists());
        save_test_session(
            data.path(),
            &"f".repeat(43),
            chrono::Utc::now().timestamp() + 86400,
        );
        let client = Client::new(data.path()).unwrap();
        client.clear_session(&"e".repeat(43), true).unwrap();
        assert!(client.logged_in() && !client.login_expired());
        assert!(client.root.join("session.json").exists());
        client
            .auth
            .lock()
            .unwrap()
            .session
            .as_mut()
            .unwrap()
            .expires_at = 0;
        assert!(client.active_session().unwrap().is_none());
        assert!(client.login_expired());
        assert!(!client.root.join("session.json").exists());
    }
    #[test]
    fn closed_window_request_cannot_delete_a_new_windows_saved_login() {
        let data = tempfile::tempdir().unwrap();
        let expires = chrono::Utc::now().timestamp() + 86400;
        save_test_session(data.path(), &"a".repeat(43), expires);
        let old_window = Client::new(data.path()).unwrap();
        // Another window finishes a fresh login before the old request's 401 arrives.
        save_test_session(data.path(), &"b".repeat(43), expires);
        let new_window = Client::new(data.path()).unwrap();
        old_window.clear_session(&"a".repeat(43), true).unwrap();
        assert!(!old_window.logged_in());
        assert!(new_window.logged_in());
        assert!(Client::new(data.path()).unwrap().logged_in());
    }
    #[test]
    fn logout_removes_local_session_even_if_server_unavailable_or_already_revoked() {
        for status in [503, 401] {
            let data = tempfile::tempdir().unwrap();
            save_test_session(
                data.path(),
                &"g".repeat(43),
                chrono::Utc::now().timestamp() + 86400,
            );
            let (origin, server) = mock_service(vec![(status, json!({"error":"unavailable"}))]);
            let mut client = online_test_client(data.path(), origin);
            client.logout().unwrap();
            assert!(!client.logged_in() && !client.login_expired());
            assert!(!Client::new(data.path()).unwrap().logged_in());
            assert_eq!(server.join().unwrap().len(), 1);
        }
    }
    fn card(gpu: &str, driver: &str, likes: u64) -> Card {
        serde_json::from_value(json!({"id":uuid::Uuid::new_v4().to_string(),"author":uuid::Uuid::new_v4().to_string(),"revision":1,"updatedAt":100,"likes":likes,"liked":false,"mine":false,"status":"visible",
          "content":{"schema":1,"game":{"id":"steam:292030","name":"The Witcher 3"},"title":"Working setup","notes":"DX12","scheme":crate::encore::SCHEME,"patchVersion":"1.1.4","entries":["dxgi.dll"],"renderer":"dx12","gpuModel":gpu,"driver":driver,"gpuAlias":"unchanged","neuralRuntime":"none"}})).unwrap()
    }
    #[test]
    fn long_notes_count_unicode_and_remain_bounded() {
        let mut c = card("RTX 3060", "581.80", 1);
        c.content.notes = "游戏🙂".repeat(1666) + "完成";
        assert_eq!(c.content.notes.chars().count(), NOTES_LIMIT);
        c.content.validate_public().unwrap();
        c.content.notes.push('多');
        assert!(c.content.validate_public().is_err());
        c.content.notes = "C:/Users/private/log.txt".into();
        assert!(c.content.validate_public().is_err());
    }
    #[test]
    fn server_data_cannot_select_paths_plugins_or_other_games() {
        let catalog = cloud::bundled();
        let mut c = card("RTX 3060", "581.80", 1);
        c.content.validate(&catalog).unwrap();
        c.content.entries = vec!["../../other.dll".into()];
        assert!(c.content.validate(&catalog).is_err());
        c.content.entries = vec!["dxgi.dll".into(), "rtx-encore.asi".into()];
        assert!(c.content.validate(&catalog).is_err());
        let page = Page {
            items: vec![card("RTX 3060", "581.80", 1)],
            total: 1,
            next_offset: None,
        };
        assert!(page.validate("steam:100", &catalog).is_err());
        let value = json!({"schema":1,"game":{"id":"steam:292030","name":"The Witcher 3"},"downloadUrl":"https://invalid.example/evil.dll"});
        assert!(serde_json::from_value::<Recipe>(value).is_err());
    }
    #[test]
    fn ranking_prioritizes_likes_over_hardware_match() {
        let mut cards = vec![
            card("RTX 5090", "581.80", 100),
            card("NVIDIA GeForce RTX 3060", "580.00", 20),
            card("RTX 3060", "581.80", 1),
        ];
        rank(&mut cards);
        assert_eq!(
            cards.iter().map(|c| c.likes).collect::<Vec<_>>(),
            vec![100, 20, 1]
        );
    }
    #[test]
    fn identity_never_infers_a_game_from_an_executable_name() {
        let game: Game = serde_json::from_value(
            json!({"exe":"D:/Games/Example/Launcher.exe","title":"Example"}),
        )
        .unwrap();
        let catalog =
            game_names::Catalog::parse(include_bytes!("../assets/game-names.json")).unwrap();
        assert!(identify(&game, &catalog).is_none());
    }
    #[test]
    fn sharing_reuses_reviewed_rules_but_never_stale_or_conflicting_matches() {
        let names = game_names::bundled();
        let mut game = Game {
            title: "Client-Win64-Shipping".into(),
            ..Default::default()
        };
        assert!(identify(&game, &names).is_none());
        game.extra.insert(
            "name_identity".into(),
            json!({"catalog_id":"Q117750031","revision":names.revision()}),
        );
        let found = identify(&game, &names).unwrap();
        assert_eq!(found.name, "鸣潮");
        assert_eq!(found.id, "steam:3513350");
        assert_eq!(platform(&game), "unknown");
        game.extra.get_mut("name_identity").unwrap()["revision"] = json!(names.revision() - 1);
        assert!(identify(&game, &names).is_none());
        game.extra.get_mut("name_identity").unwrap()["keys"] =
            json!(["steam:3513350", "steam:292030"]);
        game.extra.get_mut("name_identity").unwrap()["revision"] = json!(names.revision());
        assert!(identify(&game, &names).is_none());
        let manual = GameIdentity::custom("玩家明确选择的游戏").unwrap();
        game.extra.insert(
            "recommendation_game".into(),
            serde_json::to_value(&manual).unwrap(),
        );
        assert_eq!(identify(&game, &names).unwrap(), manual);
    }
    #[test]
    fn platform_context_is_optional_private_paths_rejected_and_routes_keep_game_context() {
        let mut c = card("RTX 3060", "581.80", 1);
        assert!(c.content.platform.is_empty() && c.content.edition.is_empty());
        c.content.platform = "epic".into();
        c.content.edition = "1.4 · 重制版".into();
        c.content.validate(&cloud::bundled()).unwrap();
        let route = recipe_route(&c, "vote").unwrap();
        let url = reqwest::Url::parse(&format!("{ORIGIN}{route}")).unwrap();
        assert_eq!(
            url.query_pairs().find(|(k, _)| k == "game").unwrap().1,
            "steam:292030"
        );
        c.content.edition = "D:/Games/Private".into();
        assert!(c.content.validate(&cloud::bundled()).is_err());
        let game:Game=serde_json::from_value(json!({"exe":"D:/Games/Game.exe","title":"Game","name_identity":{"keys":["epic:namespace/item","epic_slug:game"]}})).unwrap();
        assert_eq!(platform(&game), "epic");
    }
    #[test]
    fn custom_game_identity_and_private_content() {
        assert_eq!(
            GameIdentity::custom(" My   GAME ").unwrap().id,
            GameIdentity::custom("my game").unwrap().id
        );
        assert!(GameIdentity::custom("C:\\Users\\name\\game").is_err());
        assert!(public_text("RTX 3060，DX12", 80, false).is_ok());
        assert!(public_text("https://download.invalid/file.dll", 80, false).is_err());
    }
    #[test]
    fn saved_session_is_dpapi_protected() {
        let input = b"test-opaque-session";
        let sealed = unseal(input, true).unwrap();
        assert!(!sealed.windows(input.len()).any(|w| w == input));
        assert_eq!(&*unseal(&sealed, false).unwrap(), input);
    }
    #[test]
    fn email_suffix_never_overwrites_a_full_address() {
        assert_eq!(complete_email(" 12345678 ", "qq.com"), "12345678@qq.com");
        assert_eq!(complete_email("player", "163.com"), "player@163.com");
        assert_eq!(
            complete_email("player@example.org", "qq.com"),
            "player@example.org"
        );
        assert_eq!(complete_email("player", ""), "player");
        assert_eq!(complete_email("  ", "qq.com"), "");
    }
    #[test]
    fn negative_feedback_does_not_override_like_order() {
        let mut old = card("RTX 3060", "581.80", 3);
        assert!(!old.disliked && old.dislikes == 0);
        old.dislikes = 4;
        let mut cards = vec![old.clone(), card("RTX 3060", "581.80", 1)];
        rank(&mut cards);
        assert_eq!(cards[0].likes, 3);
        old.liked = true;
        old.disliked = true;
        assert!(old.validate("steam:292030", &cloud::bundled()).is_err());
    }
    #[test]
    fn unavailable_recommendations_remain_readable_but_cannot_be_applied() {
        let catalog = cloud::bundled();
        let mut old = card("RTX 3060", "581.80", 1);
        old.content.patch_version = "1.0.0".into();
        assert!(old.content.available_version(&catalog).is_ok());
        old.content.patch_version = "99.0.0".into();
        assert!(old.content.available_version(&catalog).is_err());
        old.content.scheme = "retired-project".into();
        old.validate("steam:292030", &catalog).unwrap();
        assert!(old.content.available_version(&catalog).is_err());
        old.content.scheme = crate::encore::SCHEME.into();
        old.content.entries = vec!["retired.dll".into()];
        old.validate("steam:292030", &catalog).unwrap();
        assert!(old.content.available_version(&catalog).is_err());
        old.content.entries = vec!["../evil.dll".into()];
        assert!(old.validate("steam:292030", &catalog).is_err());
    }
    #[test]
    fn gpu_requirement_and_publication_date_are_explicit() {
        let mut c = card("RTX 3060", "581.80", 0);
        assert!(c.created_at.is_none());
        assert!(c.content.required_gpu_name().is_none());
        c.content.gpu_alias = "rtx5090".into();
        assert_eq!(
            c.content.required_gpu_name(),
            Some(crate::gpu_alias::NAMES[1])
        );
        assert!(date(1792636800).unwrap().starts_with("2026-10-"));
        assert!(date(u64::MAX).is_none());
    }
}
