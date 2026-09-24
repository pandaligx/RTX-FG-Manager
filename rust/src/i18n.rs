use std::collections::BTreeMap;
pub const LANGUAGES: [(&str, &str); 6] = [
    ("system", "跟随系统 / System"),
    ("zh-CN", "中文"),
    ("en", "English"),
    ("ru", "Русский"),
    ("ja", "日本語"),
    ("ko", "한국어"),
];
pub struct Translator {
    pub language: String,
    catalog: BTreeMap<String, String>,
    patterns: Vec<(regex::Regex, String)>,
}
impl Translator {
    pub fn new(choice: &str) -> Self {
        let language = if choice == "system" {
            crate::win::language()
        } else {
            choice.into()
        };
        let data = match language.as_str() {
            "ru" => include_str!("../../app/locales/ru.json"),
            "ja" => include_str!("../../app/locales/ja.json"),
            "ko" => include_str!("../../app/locales/ko.json"),
            "zh-CN" => "{}",
            _ => include_str!("../../app/locales/en.json"),
        };
        let mut catalog: BTreeMap<String, String> =
            serde_json::from_str(data).expect("bundled translations");
        if let Some(index) = ["en", "ru", "ja", "ko"].iter().position(|s| *s == language) {
            let extra: BTreeMap<String, [String; 4]> =
                serde_json::from_str(include_str!("../ui-translations.json"))
                    .expect("bundled UI translations");
            catalog.extend(extra.into_iter().map(|(k, v)| (k, v[index].clone())));
        }
        let mut patterns = Vec::new();
        for (k, v) in &catalog {
            if k.contains("{0}") {
                let mut r = regex::escape(k);
                for i in 0..12 {
                    r = r.replace(&regex::escape(&format!("{{{i}}}")), "(.*?)");
                }
                if let Ok(r) = regex::Regex::new(&format!("(?s)^{r}$")) {
                    patterns.push((r, v.clone()));
                }
            }
        }
        patterns.sort_by_key(|(r, _)| std::cmp::Reverse(r.as_str().len()));
        Self {
            language,
            catalog,
            patterns,
        }
    }
    pub fn t(&self, s: &str) -> String {
        if self.language == "zh-CN" {
            return s.into();
        }
        if let Some(v) = self.catalog.get(s) {
            return v.clone();
        }
        if s.contains('\n') {
            return s
                .split('\n')
                .map(|line| self.t(line))
                .collect::<Vec<_>>()
                .join("\n");
        }
        for (r, v) in &self.patterns {
            if let Some(c) = r.captures(s) {
                let mut result = v.clone();
                for i in 1..c.len() {
                    let value = if &c[i] == s {
                        c[i].to_owned()
                    } else {
                        self.t(&c[i])
                    };
                    result = result.replace(&format!("{{{}}}", i - 1), &value);
                }
                return result;
            }
        }
        for delimiter in ["：", " · ", " / "] {
            if s.contains(delimiter) {
                return s
                    .split(delimiter)
                    .map(|p| self.t(p))
                    .collect::<Vec<_>>()
                    .join(delimiter);
            }
        }
        for prefix in [
            "已部署 ",
            "开始扫描：",
            "列表未载入：",
            "已写入显示名称：",
            "请先完全退出游戏及同目录程序：",
        ] {
            if let Some(rest) = s.strip_prefix(prefix)
                && let Some(translated) = self.catalog.get(prefix)
            {
                return format!("{translated}{rest}");
            }
        }
        s.into()
    }
}
