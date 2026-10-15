//! Community API and local matching. Browsing sends only game/scheme/protocol.
use crate::{
    assets,
    community_identity::{self, Identity},
    config_store, core,
    presets::Values,
    win,
};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

pub const ORIGIN: &str = "https://rtx-fg-community.1547245493.workers.dev";
#[derive(Debug)]
struct VerificationRequired;
impl std::fmt::Display for VerificationRequired {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("首次分享需要完成浏览器验证")
    }
}
impl std::error::Error for VerificationRequired {}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Hardware {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gpu_model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub driver: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Measurement {
    #[serde(default, skip_serializing)]
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_fps: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_fps: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub one_percent_low: Option<f64>,
    pub scene: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Content {
    pub title: String,
    #[serde(default)]
    pub notes: String,
    pub settings: Values,
    #[serde(default)]
    pub hardware: Hardware,
    pub patch_version: String,
    /// Only a reviewed loader name; never an absolute installation path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry: Option<String>,
    #[serde(default)]
    pub runtime_versions: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dlss_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ray_tracing: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub measurement: Option<Measurement>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Profile {
    pub id: String,
    pub author: String,
    pub game: String,
    pub scheme: String,
    pub protocol: String,
    pub revision: u32,
    pub official: bool,
    pub updated_at: i64,
    pub likes: u32,
    pub works: u32,
    pub content: Content,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Game {
    pub id: String,
    pub name: String,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Interactions {
    pub votes: BTreeSet<String>,
    pub reported: bool,
}
pub fn interactions(identity: &Identity, profile: &Profile) -> Result<Interactions> {
    ensure!(uuid::Uuid::parse_str(&profile.id).is_ok(), "配置编号无效");
    let state: Interactions = serde_json::from_value(signed_request(
        identity,
        "POST",
        &format!("/v1/profiles/{}/interactions", profile.id),
        &serde_json::json!({"revision":profile.revision}),
    )?)?;
    ensure!(
        state
            .votes
            .iter()
            .all(|v| ["like", "works"].contains(&v.as_str())),
        "互动状态无效"
    );
    Ok(state)
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Index {
    pub fetched_at: i64,
    pub profiles: Vec<Profile>,
}
pub struct BrowseResult {
    pub index: Index,
    pub cached: bool,
    pub message: Option<String>,
}
pub fn scheme_name(scheme: &str) -> &str {
    match scheme {
        crate::encore::SCHEME => "RTX Encore",
        crate::upstream040::SCHEME => "dlssg_for_sm86",
        "rtx40mfg-1.3.3-hf2" => "MFG Unlocker",
        _ => scheme,
    }
}
pub fn entry_allowed(scheme: &str, entry: &str) -> bool {
    match scheme {
        crate::encore::SCHEME => crate::encore::ENTRIES.contains(&entry),
        crate::upstream040::SCHEME => crate::upstream040::PROXIES.contains(&entry),
        "rtx40mfg-1.3.3-hf2" => crate::rtxmfg::ENTRIES.contains(&entry),
        _ => false,
    }
}
/// Compare actual installation metadata, never the currently selected UI option.
pub fn installation_issue(profile: &Profile, record: Option<&core::Record>) -> Option<String> {
    let Some(record) = record else {
        return Some("当前游戏尚未安装补丁".into());
    };
    if record.scheme_id.as_deref() != Some(&profile.scheme)
        || !core::backend_matches_profile(&record.backend, &profile.protocol)
    {
        return Some("当前安装的补丁方案与玩家推荐方案不同".into());
    }
    if let Some(entry) = &profile.content.entry
        && record.selected() != [entry.clone()]
    {
        return Some("当前安装的 DLL / 插件入口与玩家推荐方案不同".into());
    }
    None
}
fn client() -> Result<reqwest::blocking::Client> {
    static CLIENT: std::sync::OnceLock<reqwest::blocking::Client> = std::sync::OnceLock::new();
    if let Some(client) = CLIENT.get() {
        return Ok(client.clone());
    }
    let client = reqwest::blocking::Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(8))
        .timeout(Duration::from_secs(25))
        .user_agent(concat!("RTXFGManager/", env!("CARGO_PKG_VERSION")))
        .build()?;
    let _ = CLIENT.set(client.clone());
    Ok(CLIENT.get().cloned().unwrap_or(client))
}
fn response(response: reqwest::blocking::Response) -> Result<serde_json::Value> {
    let status = response.status();
    let mut bytes = Vec::new();
    response.take(2 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 2 * 1024 * 1024, "社区响应过大");
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).context("社区服务返回了无效内容")?;
    if !status.is_success() {
        if status == reqwest::StatusCode::UNAUTHORIZED
            && value["error"] == "identity_not_registered"
        {
            return Err(VerificationRequired.into());
        }
        bail!(
            "{}",
            match value["error"].as_str().unwrap_or("") {
                "identity_not_registered" => "请先完成社区身份验证",
                "identity_blocked" => "此社区身份已被限制",
                "verification_failed" | "verification_required" => "验证失效，请重新验证",
                "five_profiles_per_game" =>
                    "每个身份在每款游戏最多保留 5 个配置，请更新或删除已有配置",
                "revision_conflict" | "replay_or_revision_conflict" =>
                    "配置或请求状态已改变，请刷新后重试",
                "rate_limited" => "请求过于频繁，请稍后重试",
                "private_data_rejected" => "提交内容包含本机路径或联系信息，请移除后重试",
                _ => "社区服务暂不可用，本地配置与补丁管理仍可使用",
            }
        );
    }
    Ok(value)
}
fn validate_route(game: &str, scheme: &str, protocol: &str) -> Result<()> {
    ensure!(config_store::valid_game(game), "请选择明确的游戏 ID");
    config_store::accepted(scheme, protocol, &Values::new())?;
    Ok(())
}
pub(crate) fn safe_text(text: &str, max: usize) -> bool {
    static PRIVATE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    text.chars().count()<=max && !text.chars().any(|c|c.is_control()||"<>".contains(c)) && !PRIVATE.get_or_init(||regex::Regex::new(r"(?i)([a-z]:[\\/]|\\\\|/(home|users|tmp|var)/|%[a-z_]+%|https?://|\S+@\S+\.\S+)").unwrap()).is_match(text)
}
pub fn validate_content(content: &Content, scheme: &str, protocol: &str) -> Result<()> {
    ensure!(
        content
            .entry
            .as_deref()
            .is_none_or(|entry| entry_allowed(scheme, entry)),
        "DLL / 插件入口无效"
    );
    ensure!(
        !content.title.trim().is_empty()
            && safe_text(&content.title, 80)
            && safe_text(&content.notes, 500),
        "名称或备注包含不允许的内容"
    );
    let (settings, rejected) = config_store::accepted(scheme, protocol, &content.settings)?;
    ensure!(
        !settings.is_empty() && rejected.is_empty(),
        "配置包含不支持、私密或无效参数"
    );
    let h = &content.hardware;
    ensure!(
        [&h.gpu_model, &h.driver]
            .into_iter()
            .all(|s| s.as_ref().is_none_or(|s| safe_text(s, 80))),
        "硬件说明包含不允许的内容"
    );
    ensure!(
        content.patch_version.len() <= 40 && semver::Version::parse(&content.patch_version).is_ok(),
        "补丁版本无效"
    );
    ensure!(
        content
            .runtime_versions
            .iter()
            .all(
                |(k, v)| ["fg", "sr", "rr", "streamline"].contains(&k.as_str())
                    && (2..=4).contains(&v.split('.').count())
                    && v.len() <= 64
                    && v.split('.')
                        .all(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
            ),
        "运行库版本无效"
    );
    ensure!(
        [&content.dlss_mode, &content.ray_tracing]
            .into_iter()
            .all(|s| s.as_ref().is_none_or(|s| safe_text(s, 40))),
        "画质说明无效"
    );
    if let Some(m) = &content.measurement {
        ensure!(
            safe_text(&m.scene, 120)
                && [m.base_fps, m.output_fps, m.one_percent_low]
                    .into_iter()
                    .flatten()
                    .all(|n| n.is_finite() && (0.0..=10000.0).contains(&n)),
            "测量数据无效"
        );
    }
    Ok(())
}
fn check_profiles(profiles: &[Profile], game: &str, scheme: &str, protocol: &str) -> Result<()> {
    let mut ids = BTreeSet::new();
    for p in profiles {
        ensure!(
            p.game == game
                && (scheme.is_empty() || p.scheme == scheme)
                && (protocol.is_empty() || p.protocol == protocol)
                && p.revision > 0
                && uuid::Uuid::parse_str(&p.id).is_ok()
                && ids.insert(&p.id),
            "社区配置游戏或协议不一致"
        );
        ensure!(
            p.author.len() == 64 && p.author.bytes().all(|b| b.is_ascii_hexdigit()),
            "社区作者身份无效"
        );
        validate_route(&p.game, &p.scheme, &p.protocol)?;
        validate_content(&p.content, &p.scheme, &p.protocol)?;
    }
    Ok(())
}
pub fn games() -> Result<Vec<Game>> {
    #[derive(Deserialize)]
    struct List {
        games: Vec<Game>,
    }
    let data: List = serde_json::from_value(response(
        client()?
            .get(format!("{ORIGIN}/v1/games"))
            .timeout(Duration::from_secs(8))
            .send()?,
    )?)?;
    ensure!(
        data.games.len() <= 2000
            && data
                .games
                .iter()
                .all(|g| config_store::valid_game(&g.id) && safe_text(&g.name, 120)),
        "游戏列表无效"
    );
    let _ = core::atomic_json(
        &assets::cache_root()?.join("community-games.json"),
        &data.games,
    );
    Ok(data.games)
}
pub fn cached_games() -> Result<Vec<Game>> {
    let data: Vec<Game> = serde_json::from_value(core::read_json(
        &assets::cache_root()?.join("community-games.json"),
        1024 * 1024,
    )?)?;
    ensure!(
        data.len() <= 2000
            && data
                .iter()
                .all(|g| config_store::valid_game(&g.id) && safe_text(&g.name, 120)),
        "游戏列表无效"
    );
    Ok(data)
}
fn index_path(game: &str, scheme: &str, protocol: &str) -> Result<std::path::PathBuf> {
    Ok(assets::cache_root()?.join("community-index").join(format!(
        "{}.json",
        community_identity::digest(format!("{game}\n{scheme}\n{protocol}").as_bytes())
    )))
}
pub fn cached_browse(game: &str, scheme: &str, protocol: &str) -> Result<BrowseResult> {
    ensure!(config_store::valid_game(game), "请选择明确的游戏 ID");
    let index: Index = serde_json::from_value(core::read_json(
        &index_path(game, scheme, protocol)?,
        32 * 1024 * 1024,
    )?)?;
    check_profiles(&index.profiles, game, scheme, protocol)?;
    Ok(BrowseResult {
        index,
        cached: true,
        message: Some("已显示上次缓存，正在后台刷新；缓存方案可能已被修改或下架".into()),
    })
}
pub fn browse(game: &str, scheme: &str, protocol: &str) -> Result<BrowseResult> {
    ensure!(config_store::valid_game(game), "请选择明确的游戏 ID");
    if !scheme.is_empty() || !protocol.is_empty() {
        validate_route(game, scheme, protocol)?;
    }
    let path = index_path(game, scheme, protocol)?;
    let online = (|| -> Result<Index> {
        let client = client()?;
        let mut profiles = Vec::new();
        let mut cursor = None::<String>;
        let mut seen = BTreeSet::new();
        loop {
            let mut url = reqwest::Url::parse(&format!("{ORIGIN}/v1/profiles"))?;
            url.query_pairs_mut().append_pair("game", game);
            if !scheme.is_empty() {
                url.query_pairs_mut()
                    .append_pair("scheme", scheme)
                    .append_pair("protocol", protocol);
            }
            if let Some(after) = &cursor {
                url.query_pairs_mut().append_pair("after", after);
            }
            #[derive(Deserialize)]
            struct Page {
                profiles: Vec<Profile>,
                next: Option<String>,
            }
            let page: Page = serde_json::from_value(response(
                client.get(url).timeout(Duration::from_secs(8)).send()?,
            )?)?;
            ensure!(page.profiles.len() <= 120, "社区分页无效");
            profiles.extend(page.profiles);
            ensure!(profiles.len() <= 12000, "社区配置过多，请稍后重试");
            match page.next {
                None => break,
                Some(next) => {
                    ensure!(
                        uuid::Uuid::parse_str(&next).is_ok() && seen.insert(next.clone()),
                        "社区分页无效"
                    );
                    cursor = Some(next);
                }
            }
        }
        check_profiles(&profiles, game, scheme, protocol)?;
        Ok(Index {
            fetched_at: chrono::Utc::now().timestamp(),
            profiles,
        })
    })();
    match online {
        Ok(index) => {
            let message = core::atomic_json(&path, &index)
                .err()
                .map(|_| "已取得最新配置，但无法保存离线缓存".into());
            Ok(BrowseResult {
                index,
                cached: false,
                message,
            })
        }
        Err(error) => {
            let index: Index = serde_json::from_value(
                core::read_json(&path, 32 * 1024 * 1024).with_context(|| error.to_string())?,
            )?;
            check_profiles(&index.profiles, game, scheme, protocol)?;
            Ok(BrowseResult {
                index,
                cached: true,
                message: Some("社区暂不可用，正在显示此前缓存；配置可能已被修改或下架".into()),
            })
        }
    }
}
pub fn signed_request(
    identity: &Identity,
    method: &str,
    route: &str,
    data: &serde_json::Value,
) -> Result<serde_json::Value> {
    ensure!(
        route.starts_with("/v1/") && !route.contains(['\\', '?', '#']) && !route.contains(".."),
        "社区请求路径无效"
    );
    let bytes = serde_json::to_vec(data)?;
    ensure!(bytes.len() <= 32768, "提交内容过大");
    let signed = identity.sign(method, route, &bytes)?;
    let response = client()?
        .request(method.parse()?, format!("{ORIGIN}{route}"))
        .header("content-type", "application/json")
        .header("x-rtx-identity", signed.identity)
        .header("x-rtx-timestamp", signed.timestamp)
        .header("x-rtx-nonce", signed.nonce)
        .header("x-rtx-signature", signed.signature)
        .body(bytes)
        .send()?;
    let result = self::response(response)?;
    if route == "/v1/profiles"
        || (route.starts_with("/v1/profiles/") && route.matches('/').count() == 3)
    {
        let _ = invalidate_cache();
    }
    Ok(result)
}
/// A confirmed write may request one first-use verification. Never replay a
/// timeout or other ambiguous write: only an explicit pre-write rejection.
pub fn signed_with_verification(
    identity: &Identity,
    method: &str,
    route: &str,
    data: &serde_json::Value,
    cancel: &AtomicBool,
) -> Result<serde_json::Value> {
    match signed_request(identity, method, route, data) {
        Err(error) if error.is::<VerificationRequired>() => {
            register(identity, cancel)?;
            ensure!(!cancel.load(Ordering::Relaxed), "操作已取消");
            signed_request(identity, method, route, data)
        }
        result => result,
    }
}
fn invalidate_cache() -> Result<()> {
    let root = assets::cache_root()?.join("community-index");
    core::no_links(&root)?;
    if !root.try_exists()? {
        return Ok(());
    }
    for entry in std::fs::read_dir(root)?.take(10000) {
        let path = entry?.path();
        if path.extension().is_some_and(|s| s == "json")
            && path
                .file_stem()
                .and_then(|s| s.to_str())
                .is_some_and(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
        {
            core::no_links(&path)?;
            std::fs::remove_file(path)?;
        }
    }
    Ok(())
}
pub fn mine(identity: &Identity, game: &str, scheme: &str, protocol: &str) -> Result<BrowseResult> {
    ensure!(config_store::valid_game(game), "请选择明确的游戏 ID");
    if !scheme.is_empty() || !protocol.is_empty() {
        validate_route(game, scheme, protocol)?;
    }
    #[derive(Deserialize)]
    struct List {
        profiles: Vec<Profile>,
    }
    let value = match signed_request(identity, "POST", "/v1/mine", &serde_json::json!({})) {
        Err(error) if error.is::<VerificationRequired>() => serde_json::json!({"profiles":[]}),
        result => result?,
    };
    let list: List = serde_json::from_value(value)?;
    ensure!(list.profiles.len() <= 1000, "个人配置列表过大");
    let profiles = list
        .profiles
        .into_iter()
        .filter(|p| {
            p.game == game
                && (scheme.is_empty() || p.scheme == scheme)
                && (protocol.is_empty() || p.protocol == protocol)
        })
        .collect::<Vec<_>>();
    check_profiles(&profiles, game, scheme, protocol)?;
    Ok(BrowseResult {
        index: Index {
            fetched_at: chrono::Utc::now().timestamp(),
            profiles,
        },
        cached: false,
        message: Some("正在显示此游戏的个人配置（含待审核项）".into()),
    })
}
/// User invokes this explicitly; only their browser can complete Turnstile.
pub fn register(identity: &Identity, cancel: &AtomicBool) -> Result<()> {
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    listener.set_nonblocking(true)?;
    let port = listener.local_addr()?.port();
    let state = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    win::open(&format!("{ORIGIN}/verify#port={port}&state={state}"))?;
    let deadline = Instant::now() + Duration::from_secs(300);
    loop {
        ensure!(!cancel.load(Ordering::Relaxed), "验证已取消");
        ensure!(Instant::now() < deadline, "验证超时，请重新打开");
        match listener.accept() {
            Ok((mut stream, peer)) => {
                if !peer.ip().is_loopback() {
                    continue;
                }
                let token = callback(&mut stream, port, &state);
                let success = token.is_ok();
                let message = if success {
                    "验证已收到，请返回管理器。"
                } else {
                    "无效的验证请求，请从管理器重新打开。"
                };
                let code = if success { "200 OK" } else { "400 Bad Request" };
                let _ = write!(
                    stream,
                    "HTTP/1.1 {code}\r\nContent-Type: text/plain; charset=utf-8\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{message}",
                    message.len()
                );
                if let Ok(token) = token {
                    signed_request(
                        identity,
                        "POST",
                        "/v1/identities",
                        &serde_json::json!({"publicKey":identity.public().public_key,"turnstileToken":token}),
                    )?;
                    return Ok(());
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(100))
            }
            Err(e) => return Err(e.into()),
        }
    }
}
fn callback(stream: &mut TcpStream, port: u16, state: &str) -> Result<String> {
    // Windows can inherit the listener's nonblocking mode. Browsers may connect
    // before sending the form, so use bounded blocking reads on this socket.
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    let mut head = Vec::new();
    let end = Instant::now() + Duration::from_secs(3);
    while !head.ends_with(b"\r\n\r\n") {
        ensure!(head.len() < 8192 && Instant::now() < end, "验证请求头过长");
        let mut b = [0];
        stream.read_exact(&mut b)?;
        head.push(b[0]);
    }
    let text = std::str::from_utf8(&head)?;
    let mut lines = text.split("\r\n");
    ensure!(
        lines.next() == Some("POST /verify HTTP/1.1"),
        "验证请求无效"
    );
    let mut fields = BTreeMap::new();
    for line in lines.filter(|s| !s.is_empty()) {
        let (name, value) = line.split_once(':').context("请求头无效")?;
        ensure!(
            fields
                .insert(name.to_ascii_lowercase(), value.trim().to_owned())
                .is_none(),
            "重复请求头"
        );
    }
    ensure!(
        fields.get("host") == Some(&format!("127.0.0.1:{port}"))
            && fields.get("origin").map(String::as_str) == Some(ORIGIN)
            && !fields.contains_key("transfer-encoding"),
        "验证请求来源无效"
    );
    ensure!(
        fields
            .get("content-type")
            .is_some_and(|v| v.split(';').next() == Some("application/x-www-form-urlencoded")),
        "验证请求类型无效"
    );
    let size: usize = fields
        .get("content-length")
        .context("缺少验证内容")?
        .parse()?;
    ensure!(size <= 8192, "验证内容过长");
    let mut bytes = vec![0; size];
    let mut offset = 0;
    while offset < size {
        ensure!(Instant::now() < end, "验证请求超时");
        let n = stream.read(&mut bytes[offset..])?;
        ensure!(n > 0, "验证请求不完整");
        offset += n;
    }
    let body = std::str::from_utf8(&bytes)?;
    let parsed = reqwest::Url::parse(&format!("http://localhost/?{body}"))?;
    let pairs = parsed.query_pairs().collect::<Vec<_>>();
    ensure!(
        pairs.len() == 2
            && pairs
                .iter()
                .filter(|(k, v)| k == "state" && v == state)
                .count()
                == 1,
        "验证状态不匹配"
    );
    let token = pairs
        .iter()
        .find(|(k, _)| k == "token")
        .context("缺少验证结果")?
        .1
        .to_string();
    ensure!(
        !token.is_empty() && token.len() <= 2048 && !token.chars().any(char::is_control),
        "验证结果无效"
    );
    Ok(token)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn community_game_scope_and_installation_entry_are_checked_separately() {
        let content: Content = serde_json::from_value(serde_json::json!({
            "title":"fixture", "settings":{"optimized":"2"},"patchVersion":"0.4.0","entry":"dxgi.dll"
        })).unwrap();
        let profile = Profile {
            id: uuid::Uuid::new_v4().to_string(),
            author: "a".repeat(64),
            game: "steam:123".into(),
            scheme: crate::upstream040::SCHEME.into(),
            protocol: crate::upstream040::PROFILE.into(),
            revision: 1,
            official: false,
            updated_at: 1,
            likes: 0,
            works: 0,
            status: None,
            content,
        };
        assert!(check_profiles(std::slice::from_ref(&profile), "steam:123", "", "").is_ok());
        assert!(check_profiles(std::slice::from_ref(&profile), "steam:456", "", "").is_err());
        assert!(
            check_profiles(
                std::slice::from_ref(&profile),
                "steam:123",
                crate::encore::SCHEME,
                crate::encore::PROFILE
            )
            .is_err()
        );
        let mut record: core::Record = serde_json::from_value(serde_json::json!({
            "schema":3,"backend":"upstream040","scheme_id":crate::upstream040::SCHEME,
            "proxy":"dxgi.dll","hashes":{}
        }))
        .unwrap();
        assert!(installation_issue(&profile, Some(&record)).is_none());
        record.proxy = "version.dll".into();
        assert!(installation_issue(&profile, Some(&record)).is_some());
        record.proxy = "dxgi.dll".into();
        record.scheme_id = Some(crate::encore::SCHEME.into());
        assert!(installation_issue(&profile, Some(&record)).is_some());
        assert!(installation_issue(&profile, None).is_some());
        for entry in [
            "../dxgi.dll",
            "C:\\game\\dxgi.dll",
            crate::mfg_plugin::ENTRY,
            "rtx-encore.asi",
        ] {
            assert!(!entry_allowed(crate::upstream040::SCHEME, entry));
        }
        assert!(entry_allowed(
            "rtx40mfg-1.3.3-hf2",
            crate::mfg_plugin::ENTRY
        ));
        assert!(entry_allowed(crate::encore::SCHEME, "rtx-encore.asi"));
    }
    #[test]
    fn sensitive_fields_and_cross_scheme_content_are_rejected() {
        let mut content = Content {
            title: "Balanced".into(),
            notes: String::new(),
            settings: Values::from([("optimized".into(), "2".into())]),
            hardware: Hardware {
                gpu_model: Some("RTX 3060".into()),
                ..Default::default()
            },
            patch_version: "0.4.0".into(),
            entry: Some("version.dll".into()),
            runtime_versions: BTreeMap::new(),
            dlss_mode: None,
            ray_tracing: None,
            measurement: None,
        };
        assert_eq!(
            serde_json::to_value(&content.hardware).unwrap(),
            serde_json::json!({"gpuModel":"RTX 3060"})
        );
        assert!(
            validate_content(
                &content,
                crate::upstream040::SCHEME,
                crate::upstream040::PROFILE
            )
            .is_ok()
        );
        content.notes = r"C:\Users\Someone\Game".into();
        assert!(
            validate_content(
                &content,
                crate::upstream040::SCHEME,
                crate::upstream040::PROFILE
            )
            .is_err()
        );
        content.notes.clear();
        assert!(validate_content(&content, crate::encore::SCHEME, crate::encore::PROFILE).is_err());
    }
    #[test]
    fn verification_callback_accepts_a_browser_that_sends_after_connecting() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        listener.set_nonblocking(true).unwrap();
        let state = "a".repeat(64);
        let expected = state.clone();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = loop {
                match listener.accept() {
                    Ok(pair) => break pair,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(e) => panic!("{e}"),
                }
            };
            stream.set_nonblocking(true).unwrap();
            callback(&mut stream, port, &expected)
        });
        let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
        std::thread::sleep(Duration::from_millis(100));
        let body = format!("state={state}&token=synthetic-unit-test");
        write!(client, "POST /verify HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nOrigin: {ORIGIN}\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: {}\r\n\r\n", body.len()).unwrap();
        std::thread::sleep(Duration::from_millis(100));
        client.write_all(body.as_bytes()).unwrap();
        assert_eq!(server.join().unwrap().unwrap(), "synthetic-unit-test");
    }
    fn callback_request(origin: &str, state: &str, duplicate: bool) -> Result<String> {
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        let port = listener.local_addr()?.port();
        let expected = "a".repeat(64);
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            callback(&mut stream, port, &expected)
        });
        let body = format!("state={state}&token=test-token");
        let extra = if duplicate {
            format!("Origin: {ORIGIN}\r\n")
        } else {
            String::new()
        };
        let mut stream = TcpStream::connect(("127.0.0.1", port))?;
        write!(
            stream,
            "POST /verify HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nOrigin: {origin}\r\n{extra}Content-Type: application/x-www-form-urlencoded\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        )?;
        server.join().unwrap()
    }
    #[test]
    fn verification_callback_binds_origin_state_and_host() {
        assert_eq!(
            callback_request(ORIGIN, &"a".repeat(64), false).unwrap(),
            "test-token"
        );
        assert!(callback_request("https://unrelated.example", &"a".repeat(64), false).is_err());
        assert!(callback_request(ORIGIN, &"b".repeat(64), false).is_err());
        assert!(callback_request(ORIGIN, &"a".repeat(64), true).is_err());
    }
}
