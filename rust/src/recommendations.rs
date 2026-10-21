//! Game-scoped deployment recipes. Community content never supplies file paths,
//! download URLs or commands. Selecting a recipe only changes the local choices.
use crate::{cloud, core, game_names, neural_runtime, scanner::Game};
use anyhow::{Context, Result, bail, ensure};
use reqwest::{Method, blocking::Client as Http};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs,
    io::Read,
    path::{Path, PathBuf},
    time::Duration,
};
use zeroize::{Zeroize, Zeroizing};

const ORIGIN: &str = "https://recommend.lgxng.cn";
const RESPONSE_LIMIT: u64 = 512 * 1024;
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
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub platform: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub edition: String,
}
impl Recipe {
    pub fn validate(&self, catalog: &cloud::Catalog) -> Result<()> {
        ensure!(self.schema == 1, "此推荐需要更新管理器");
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
            (&self.notes, 500, true),
            (&self.gpu_model, 80, true),
            (&self.driver, 40, true),
        ] {
            public_text(text, max, empty)?;
        }
        ensure!(
            catalog.schemes().iter().any(|p| p.scheme_id == self.scheme),
            "当前管理器不支持此推荐方案"
        );
        let allowed = catalog.proxies(&self.scheme);
        ensure!(
            !self.entries.is_empty()
                && self.entries.len() <= catalog.selection_limit(&self.scheme)
                && self.entries.len() == self.entries.iter().collect::<BTreeSet<_>>().len()
                && self.entries.iter().all(|p| allowed.contains(p)),
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
}
/// Read records and lightweight NR metadata only. No hashing/downloading game DLLs.
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
        };
        current.validate(catalog)?;
        if let Some(old) = &found {
            ensure!(
                old.scheme == current.scheme
                    && old.entries == current.entries
                    && old.patch_version == current.patch_version
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
    fn validate(&self, game: &str, catalog: &cloud::Catalog) -> Result<()> {
        ensure!(
            self.content.game.id == game
                && uuid::Uuid::parse_str(&self.id).is_ok()
                && uuid::Uuid::parse_str(&self.author).is_ok()
                && self.revision > 0
                && !(self.liked && self.disliked)
                && ["visible", "hidden"].contains(&self.status.as_str()),
            "推荐不属于当前游戏或格式无效"
        );
        self.content.validate(catalog)
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
fn gpu_key(value: &str) -> String {
    value
        .to_ascii_lowercase()
        .replace([' ', '-', '_'], "")
        .replace("nvidia", "")
        .replace("geforce", "")
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
pub fn rank(items: &mut [Card], gpu: &str, driver: &str) {
    let gpu = gpu_key(gpu);
    items.sort_by_key(|c| {
        std::cmp::Reverse((
            !gpu.is_empty() && gpu_key(&c.content.gpu_model) == gpu,
            !driver.is_empty() && c.content.driver == driver,
            i128::from(c.likes) - i128::from(c.dislikes),
            c.likes,
            c.updated_at,
        ))
    });
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
    http: Http,
    root: PathBuf,
    session: Option<Session>,
}
impl Client {
    pub fn new(data: &Path) -> Result<Self> {
        let root = data.join("recommendations");
        fs::create_dir_all(&root)?;
        let session = fs::read(root.join("session.json"))
            .ok()
            .filter(|b| b.len() <= 65536)
            .and_then(|b| serde_json::from_slice::<Vec<u8>>(&b).ok())
            .and_then(|b| unseal(&b, false).ok())
            .and_then(|b| serde_json::from_slice::<Session>(&b).ok())
            .filter(|s| s.expires_at > chrono::Utc::now().timestamp());
        Ok(Self {
            http: Http::builder()
                .connect_timeout(Duration::from_secs(5))
                .timeout(Duration::from_secs(15))
                .redirect(reqwest::redirect::Policy::none())
                .user_agent(format!("RTXFG/{}", crate::VERSION))
                .build()?,
            root,
            session,
        })
    }
    pub fn logged_in(&self) -> bool {
        self.session
            .as_ref()
            .is_some_and(|s| s.expires_at > chrono::Utc::now().timestamp())
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
        let mut request = self
            .http
            .request(method, format!("{ORIGIN}{route}"))
            .header("X-RTXFG-Feedback", "4");
        if authenticated {
            let s = self.session.as_ref().context("请先用邮箱验证码登录")?;
            request = request.bearer_auth(&s.token);
        }
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request
            .send()
            .map_err(|_| anyhow::anyhow!("推荐服务暂时无法连接，请稍后刷新"))?;
        let status = response.status();
        let mut bytes = Vec::new();
        response.take(RESPONSE_LIMIT + 1).read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() <= RESPONSE_LIMIT as usize,
            "推荐服务返回内容过大"
        );
        if !status.is_success() {
            let value: Value = serde_json::from_slice(&bytes).unwrap_or_default();
            bail!(
                "{}",
                error_message(value["error"].as_str().unwrap_or("unavailable"))
            );
        }
        serde_json::from_slice(&bytes).context("推荐服务返回格式无效")
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
        let page: Page = self.request(Method::GET, &route, None, self.logged_in())?;
        page.validate(&game.id, catalog)?;
        if offset == 0 && !mine && !reported {
            let _ = core::atomic_json(&self.cache_path(&game.id), &page);
        }
        Ok(page)
    }
    fn cache_path(&self, id: &str) -> PathBuf {
        self.root
            .join(format!("{:x}.json", Sha256::digest(id.as_bytes())))
    }
    pub fn cached(&self, game: &GameIdentity, catalog: &cloud::Catalog) -> Option<Page> {
        let b = fs::read(self.cache_path(&game.id)).ok()?;
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
        core::atomic_json(&path, &*protected)?;
        self.session = Some(s);
        Ok(())
    }
    pub fn logout(&mut self) -> Result<()> {
        let result = self.request::<Value>(Method::POST, "/v1/auth/logout", Some(json!({})), true);
        self.session = None;
        let path = self.root.join("session.json");
        if path.exists() {
            fs::remove_file(path)?;
        }
        result.map(|_| ())
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
    fn card(gpu: &str, driver: &str, likes: u64) -> Card {
        serde_json::from_value(json!({"id":uuid::Uuid::new_v4().to_string(),"author":uuid::Uuid::new_v4().to_string(),"revision":1,"updatedAt":100,"likes":likes,"liked":false,"mine":false,"status":"visible",
          "content":{"schema":1,"game":{"id":"steam:292030","name":"The Witcher 3"},"title":"Working setup","notes":"DX12","scheme":crate::encore::SCHEME,"patchVersion":"1.1.4","entries":["dxgi.dll"],"renderer":"dx12","gpuModel":gpu,"driver":driver,"gpuAlias":"unchanged","neuralRuntime":"none"}})).unwrap()
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
    fn ranking_prioritizes_gpu_and_driver_over_popularity() {
        let mut cards = vec![
            card("RTX 5090", "581.80", 100),
            card("NVIDIA GeForce RTX 3060", "580.00", 20),
            card("RTX 3060", "581.80", 1),
        ];
        rank(&mut cards, "NVIDIA GeForce RTX 3060", "581.80");
        assert_eq!(
            cards.iter().map(|c| c.likes).collect::<Vec<_>>(),
            vec![1, 20, 100]
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
    fn feedback_preserves_old_cards_and_demotes_negative_results() {
        let mut old = card("RTX 3060", "581.80", 3);
        assert!(!old.disliked && old.dislikes == 0);
        old.dislikes = 4;
        let mut cards = vec![old.clone(), card("RTX 3060", "581.80", 1)];
        rank(&mut cards, "RTX 3060", "581.80");
        assert_eq!(cards[0].likes, 1);
        old.liked = true;
        old.disliked = true;
        assert!(old.validate("steam:292030", &cloud::bundled()).is_err());
    }
}
