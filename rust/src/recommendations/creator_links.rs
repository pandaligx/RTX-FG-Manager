use anyhow::{Result, bail, ensure};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use std::{sync::LazyLock, time::Duration};

const LINK_ERROR: &str = "请粘贴包含 HTTPS 链接的分享内容";
const REDIRECT_ERROR: &str = "暂时打不开主页短链，请重试或粘贴浏览器中的主页地址";

/// Extract one web address from a platform's clipboard message.
pub fn extract_share_url(text: &str) -> Result<String> {
    static URL: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r#"(?i)https://[^\s\p{Cc}<>【】《》“”「」（）。，；！？"]+"#)
            .expect("valid share URL pattern")
    });
    let text = text.trim();
    if text.is_empty() {
        return Ok(String::new());
    }
    ensure!(text.len() <= 32768, "{LINK_ERROR}");
    let text = text.replace(r"https\://", "https://");
    let mut urls = URL.find_iter(&text);
    let found = urls.next().ok_or_else(|| anyhow::anyhow!(LINK_ERROR))?;
    ensure!(urls.next().is_none(), "每个输入框只粘贴一个链接");
    let url = found.as_str();
    let url = if text[..found.start()].ends_with('\'') {
        url.strip_suffix('\'').unwrap_or(url)
    } else {
        url
    };
    let url = if url.ends_with(')') && !url.contains('(') {
        url.trim_end_matches(')')
    } else {
        url
    };
    ensure!(url.len() <= 2048, "{LINK_ERROR}");
    Ok(url.to_owned())
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CreatorLinks {
    #[serde(skip_serializing_if = "String::is_empty")]
    pub douyin: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub bilibili: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub heybox: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub tutorial: String,
}

impl CreatorLinks {
    /// Prepare clipboard inputs on the sharing worker before sending a recipe.
    pub fn prepare_for_sharing(&self) -> Result<Self> {
        self.prepare_with(|url| {
            // Read only the shortener's response. Never fetch the destination or
            // send recommendation-service credentials to the sharing platforms.
            let client = reqwest::blocking::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(8))
                .build()?;
            let response = client
                .get(url.clone())
                .send()
                .map_err(|_| anyhow::anyhow!(REDIRECT_ERROR))?;
            ensure!(response.status().is_redirection(), "{REDIRECT_ERROR}");
            let destination = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|h| h.to_str().ok())
                .ok_or_else(|| anyhow::anyhow!(REDIRECT_ERROR))?;
            Ok(url
                .join(destination)
                .map_err(|_| anyhow::anyhow!(REDIRECT_ERROR))?
                .to_string())
        })
    }

    fn prepare_with(&self, mut resolve: impl FnMut(&Url) -> Result<String>) -> Result<Self> {
        let mut profile = |kind: &str, text: &str, short_host: &str| -> Result<String> {
            let text = extract_share_url(text)?;
            if text.is_empty() {
                return Ok(text);
            }
            let mut url = Url::parse(&text).map_err(|_| anyhow::anyhow!(LINK_ERROR))?;
            if url.domain() == Some(short_host) {
                let token = url
                    .path()
                    .strip_prefix('/')
                    .unwrap_or("")
                    .trim_end_matches('/');
                ensure!(plain_https(&url) && account_id(token), "{LINK_ERROR}");
                // A trailing slash on b23.tv can return a landing page instead of a redirect.
                let ending = if short_host == "b23.tv" { "" } else { "/" };
                url.set_path(&format!("/{token}{ending}"));
                url.set_query(None);
                url.set_fragment(None);
                url = Url::parse(&resolve(&url)?).map_err(|_| anyhow::anyhow!(REDIRECT_ERROR))?;
            }
            ensure!(plain_https(&url), "{LINK_ERROR}");
            // Mobile share pages use different hosts and paths than desktop profiles.
            let path = url.path().trim_end_matches('/');
            let canonical = match (kind, url.domain()) {
                ("douyin", Some("www.iesdouyin.com" | "www.douyin.com" | "iesdouyin.com"))
                    if let Some(id) = path
                        .strip_prefix("/share/user/")
                        .filter(|id| account_id(id)) =>
                {
                    format!("https://www.douyin.com/user/{id}")
                }
                ("bilibili", Some("m.bilibili.com" | "www.bilibili.com"))
                    if let Some(id) = path
                        .strip_prefix("/space/")
                        .filter(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())) =>
                {
                    format!("https://space.bilibili.com/{id}")
                }
                _ => url.to_string(),
            };
            link(kind, &canonical)
        };
        Ok(Self {
            douyin: profile("douyin", &self.douyin, "v.douyin.com")?,
            bilibili: profile("bilibili", &self.bilibili, "b23.tv")?,
            heybox: link("heybox", &extract_share_url(&self.heybox)?)?,
            tutorial: link("tutorial", &extract_share_url(&self.tutorial)?)?,
        })
    }

    pub fn is_empty(&self) -> bool {
        [&self.douyin, &self.bilibili, &self.heybox, &self.tutorial]
            .iter()
            .all(|v| v.is_empty())
    }

    pub fn normalized(&self) -> Result<Self> {
        Ok(Self {
            douyin: link("douyin", &self.douyin)?,
            bilibili: link("bilibili", &self.bilibili)?,
            heybox: link("heybox", &self.heybox)?,
            tutorial: link("tutorial", &self.tutorial)?,
        })
    }
}

fn plain_https(url: &Url) -> bool {
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
}

fn account_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 160
        && s.bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-'))
}

fn link(kind: &str, value: &str) -> Result<String> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(String::new());
    }
    ensure!(
        value.len() <= 2048
            && value
                .get(..8)
                .is_some_and(|s| s.eq_ignore_ascii_case("https://"))
            && !value
                .chars()
                .any(|c| c.is_control() || c.is_whitespace() || c == '\\'),
        "请粘贴完整的 HTTPS 网页地址"
    );
    let mut url = Url::parse(value).map_err(|_| anyhow::anyhow!("请粘贴完整的 HTTPS 网页地址"))?;
    let host = url.domain().unwrap_or_default();
    ensure!(
        url.scheme() == "https"
            && url.username().is_empty()
            && url.password().is_none()
            && host.split('.').count() >= 2
            && host.split('.').all(|p| !p.is_empty())
            && ![
                ".local",
                ".localhost",
                ".internal",
                ".lan",
                ".home",
                ".arpa"
            ]
            .iter()
            .any(|s| host.ends_with(s)),
        "请输入公开的 HTTPS 网页地址"
    );
    if kind == "tutorial" {
        return Ok(url.to_string());
    }
    ensure!(url.port().is_none(), "请输入该平台的个人主页地址");
    let path = url.path().trim_end_matches('/');
    match kind {
        "douyin"
            if matches!(host, "douyin.com" | "www.douyin.com")
                && path.strip_prefix("/user/").is_some_and(account_id) =>
        {
            url.set_query(None)
        }
        "bilibili"
            if host == "space.bilibili.com"
                && path
                    .strip_prefix('/')
                    .is_some_and(|id| !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit())) =>
        {
            url.set_query(None)
        }
        "heybox"
            if matches!(host, "xiaoheihe.cn" | "www.xiaoheihe.cn")
                && path == "/bbs/user_profile_share" =>
        {
            let ids: Vec<_> = url
                .query_pairs()
                .filter(|(key, _)| key == "user_id")
                .map(|(_, v)| v.into_owned())
                .collect();
            ensure!(
                ids.len() == 1 && account_id(&ids[0]),
                "请输入小黑盒个人主页地址"
            );
            url.query_pairs_mut()
                .clear()
                .append_pair("user_id", &ids[0]);
        }
        "douyin" => bail!("请输入抖音个人主页地址，例如 www.douyin.com/user/…"),
        "bilibili" => bail!("请输入哔哩哔哩个人空间地址，例如 space.bilibili.com/…"),
        _ => bail!("请输入小黑盒个人主页地址"),
    }
    url.set_fragment(None);
    Ok(url.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "online check using publisher-supplied public share messages"]
    fn live_mobile_share_messages() {
        let messages: CreatorLinks = serde_json::from_str(
            &std::env::var("RTXFG_TEST_SHARE_TEXT").expect("supply clipboard messages as JSON"),
        )
        .unwrap();
        let expected: CreatorLinks = serde_json::from_str(
            &std::env::var("RTXFG_TEST_SHARE_LINKS").expect("supply expected public links as JSON"),
        )
        .unwrap();
        let result = messages.prepare_for_sharing().unwrap();
        assert_eq!(result, expected);
        assert_eq!(result.normalized().unwrap(), result);
    }
    #[test]
    fn mobile_share_messages_become_server_compatible_links() {
        let draft = CreatorLinks {
            douyin: "5- 长按复制此条消息，打开抖音搜索，查看TA的更多作品。 https://v.douyin.com/profile-1/ 5@3.com :3pm".into(),
            bilibili: "【作者的个人空间-哔哩哔哩】 https://b23.tv/Space1/".into(),
            tutorial: "5.66 复制打开抖音，看看【某作者的作品】教程 https://v.douyin.com/Video1/ kcA:/ :9pm 11/17".into(),
            ..Default::default()
        };
        let links = draft
            .prepare_with(|url| match url.as_str() {
                "https://v.douyin.com/profile-1/" => {
                    Ok("https://www.iesdouyin.com/share/user/MS4_example?iid=ignore".into())
                }
                "https://b23.tv/Space1" => {
                    Ok("https://m.bilibili.com/space/12345?share_source=COPY".into())
                }
                _ => panic!("unexpected lookup"),
            })
            .unwrap();
        assert_eq!(links.douyin, "https://www.douyin.com/user/MS4_example");
        assert_eq!(links.bilibili, "https://space.bilibili.com/12345");
        assert_eq!(links.tutorial, "https://v.douyin.com/Video1/");
        assert_eq!(links.normalized().unwrap(), links);
        assert_eq!(
            extract_share_url("【视频标题-哔哩哔哩】https://b23.tv/Video2。").unwrap(),
            "https://b23.tv/Video2"
        );
        assert!(extract_share_url("https://b23.tv/First https://b23.tv/Second").is_err());
        assert!(extract_share_url("5@3.com :3pm").is_err());
        assert!(
            CreatorLinks {
                bilibili: "https://b23.tv/Video2".into(),
                ..Default::default()
            }
            .prepare_with(|_| Ok("https://www.bilibili.com/video/BVexample".into()))
            .is_err()
        );
        assert!(
            CreatorLinks {
                douyin: "https://v.douyin.com.evil.example/X".into(),
                ..Default::default()
            }
            .prepare_with(|_| panic!("unexpected network request"))
            .is_err()
        );
    }
    #[test]
    fn profiles_are_platform_specific_and_strip_tracking() {
        assert_eq!(
            link("bilibili", "https://space.bilibili.com/42?share=1").unwrap(),
            "https://space.bilibili.com/42"
        );
        assert_eq!(
            link(
                "heybox",
                "https://www.xiaoheihe.cn/bbs/user_profile_share?user_id=42&h_src=abc"
            )
            .unwrap(),
            "https://www.xiaoheihe.cn/bbs/user_profile_share?user_id=42"
        );
        assert!(link("douyin", "https://www.douyin.com/video/42").is_err());
        assert!(link("bilibili", "https://space.bilibili.com.evil.org/42").is_err());
        for bad in [
            "file:///c:/video",
            "https://localhost/video",
            "https://[::1]/",
            "https://127.0.0.1/",
            "https://viewer@example.com/",
        ] {
            assert!(link("tutorial", bad).is_err());
        }
        let mut credentialed = Url::parse("https://video.example.com/watch").unwrap();
        credentialed.set_password(Some("test-only")).unwrap();
        assert!(link("tutorial", credentialed.as_str()).is_err());
        assert!(link("tutorial", "https://video.example.com/watch?v=42#t=30").is_ok());
    }
}
