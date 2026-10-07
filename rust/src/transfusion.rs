//! DLSSG-Transfusion 1.4.5.3: independent JSONC v3, one exact proxy per game.
use crate::presets::{Parameter, Values};
use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};
use std::{collections::BTreeMap, ops::Range};

pub const PROFILE: &str = "transfusion_json_v3";
pub const BACKEND: &str = "transfusion";
pub const CONFIG: &str = "DLSSG-Transfusion.json";
pub const SOURCE: &str = "https://github.com/SilyNoMeta/DLSSG-Transfusion";
pub const PROXIES: [&str; 4] = ["version.dll", "dinput8.dll", "dxgi.dll", "winmm.dll"];

pub fn parameters() -> Vec<Parameter> {
    vec![
        Parameter {
            key: "tf_mode",
            section: "frameGeneration",
            ini_key: "mode",
            label: "请求倍率",
            default: "game",
            choices: vec![
                ("game", "跟随游戏"),
                ("2", "2X"),
                ("3", "3X"),
                ("4", "4X"),
                ("5", "5X（实验）"),
                ("6", "6X（实验）"),
                ("dynamic", "动态倍率"),
            ],
        },
        Parameter {
            key: "tf_target",
            section: "frameGeneration",
            ini_key: "dynamicTargetFrameRate",
            label: "动态目标帧率",
            default: "0",
            choices: vec![
                ("0", "跟随显示器刷新率"),
                ("60", "60 FPS"),
                ("90", "90 FPS"),
                ("120", "120 FPS"),
                ("144", "144 FPS"),
                ("165", "165 FPS"),
                ("180", "180 FPS"),
                ("240", "240 FPS"),
                ("360", "360 FPS"),
            ],
        },
        Parameter {
            key: "tf_dynamic56",
            section: "frameGeneration",
            ini_key: "dynamicExperimental56",
            label: "动态允许 5X/6X",
            default: "0",
            choices: vec![("0", "关闭"), ("1", "开启")],
        },
        Parameter {
            key: "tf_overlay",
            section: "overlay",
            ini_key: "showOverlay",
            label: "显示帧生成统计",
            default: "0",
            choices: vec![("0", "关闭"), ("1", "开启")],
        },
    ]
}

pub fn valid_value(key: &str, value: &str) -> bool {
    (key == "tf_target"
        && !value.is_empty()
        && value.bytes().all(|b| b.is_ascii_digit())
        && value.parse::<u32>().is_ok_and(|n| n <= 1000))
        || parameters()
            .iter()
            .any(|p| p.key == key && p.choices.iter().any(|(v, _)| *v == value))
}

/// Remove comments while preserving byte offsets (including Unicode comments).
fn uncomment(text: &str) -> Result<String> {
    let b = text.as_bytes();
    let mut out = b.to_vec();
    let (mut i, mut string, mut escaped) = (0, false, false);
    while i < b.len() {
        if string {
            if escaped {
                escaped = false;
            } else if b[i] == b'\\' {
                escaped = true;
            } else if b[i] == b'"' {
                string = false;
            }
            i += 1;
        } else if b[i] == b'"' {
            string = true;
            i += 1;
        } else if b[i..].starts_with(b"//") {
            while i < b.len() && b[i] != b'\n' && b[i] != b'\r' {
                out[i] = b' ';
                i += 1;
            }
        } else if b[i..].starts_with(b"/*") {
            out[i] = b' ';
            out[i + 1] = b' ';
            i += 2;
            while i < b.len() && !b[i..].starts_with(b"*/") {
                if !matches!(b[i], b'\r' | b'\n') {
                    out[i] = b' ';
                }
                i += 1;
            }
            ensure!(i + 1 < b.len(), "Unterminated JSON comment");
            out[i] = b' ';
            out[i + 1] = b' ';
            i += 2;
        } else {
            i += 1;
        }
    }
    String::from_utf8(out).context("Invalid JSON text")
}

struct Node {
    range: Range<usize>,
    fields: Option<BTreeMap<String, Node>>,
}
fn whitespace(b: &[u8], pos: &mut usize) {
    while *pos < b.len() && b[*pos].is_ascii_whitespace() {
        *pos += 1;
    }
}
fn string_end(b: &[u8], start: usize) -> usize {
    let mut p = start + 1;
    while p < b.len() {
        if b[p] == b'\\' {
            p += 2;
        } else if b[p] == b'"' {
            return p + 1;
        } else {
            p += 1;
        }
    }
    b.len()
}
// Called only after serde_json has validated the complete document. Reject duplicate
// keys so an edit can never silently update a different value than the engine reads.
fn node(text: &str, pos: &mut usize) -> Result<Node> {
    let b = text.as_bytes();
    whitespace(b, pos);
    let start = *pos;
    let fields = match b[*pos] {
        b'{' => {
            *pos += 1;
            whitespace(b, pos);
            let mut fields = BTreeMap::new();
            while b[*pos] != b'}' {
                let end = string_end(b, *pos);
                let key: String = serde_json::from_str(&text[*pos..end])?;
                *pos = end;
                whitespace(b, pos);
                *pos += 1;
                let value = node(text, pos)?;
                ensure!(fields.insert(key, value).is_none(), "Duplicate JSON key");
                whitespace(b, pos);
                if b[*pos] == b',' {
                    *pos += 1;
                    whitespace(b, pos);
                }
            }
            *pos += 1;
            Some(fields)
        }
        b'[' => {
            *pos += 1;
            whitespace(b, pos);
            while b[*pos] != b']' {
                node(text, pos)?;
                whitespace(b, pos);
                if b[*pos] == b',' {
                    *pos += 1;
                    whitespace(b, pos);
                }
            }
            *pos += 1;
            None
        }
        b'"' => {
            *pos = string_end(b, *pos);
            None
        }
        _ => {
            while *pos < b.len() && !b",}] \r\n\t".contains(&b[*pos]) {
                *pos += 1;
            }
            None
        }
    };
    Ok(Node {
        range: start..*pos,
        fields,
    })
}
fn document(bytes: &[u8]) -> Result<(String, Value, Node)> {
    let text = std::str::from_utf8(bytes.strip_prefix(&[239, 187, 191]).unwrap_or(bytes))?;
    let stripped = uncomment(text)?;
    let data: Value = serde_json::from_str(&stripped)?;
    ensure!(
        data.is_object(),
        "Transfusion configuration must be an object"
    );
    if let Some(version) = data.get("configVersion") {
        ensure!(
            version.as_u64().is_some_and(|v| (1..=3).contains(&v)),
            "Unsupported Transfusion config version"
        );
    }
    let tree = node(&stripped, &mut 0)?;
    Ok((text.into(), data, tree))
}
fn setting<'a>(data: &'a Value, section: &str, key: &str) -> Option<&'a Value> {
    data.get(section)
        .and_then(|v| v.get(key))
        .or_else(|| data.get(key))
}
pub fn read(bytes: &[u8]) -> Result<Values> {
    let (_, data, _) = document(bytes)?;
    for section in ["frameGeneration", "overlay"] {
        ensure!(
            data.get(section).is_none_or(Value::is_object),
            "Transfusion section must be an object: {section}"
        );
    }
    let mode = match setting(&data, "frameGeneration", "mode") {
        Some(value) => value.as_str().context("Invalid Transfusion mode type")?,
        None => "game",
    };
    let multiplier = match setting(&data, "frameGeneration", "multiplier") {
        Some(value) => value
            .as_u64()
            .context("Invalid Transfusion multiplier type")?,
        None => 4,
    };
    ensure!(
        (2..=6).contains(&multiplier),
        "Invalid Transfusion multiplier"
    );
    let selected = match mode {
        "game" | "dynamic" => mode.into(),
        "fixed" => multiplier.to_string(),
        _ => bail!("Invalid Transfusion mode"),
    };
    let mut values = Values::from([("tf_mode".into(), selected)]);
    for p in parameters().into_iter().skip(1) {
        if let Some(v) = setting(&data, p.section, p.ini_key) {
            let value = if p.key == "tf_target" {
                v.as_u64().context("Invalid target FPS")?.to_string()
            } else {
                if v.as_bool().context("Invalid Transfusion toggle")? {
                    "1"
                } else {
                    "0"
                }
                .into()
            };
            ensure!(
                valid_value(p.key, &value),
                "Invalid Transfusion setting: {}",
                p.key
            );
            values.insert(p.key.into(), value);
        }
    }
    crate::presets::validate(PROFILE, &values)?;
    Ok(values)
}
fn edit(bytes: &[u8], section: &str, key: &str, value: Value) -> Result<Vec<u8>> {
    let (mut text, _, tree) = document(bytes)?;
    let fields = tree.fields.as_ref().expect("object validated");
    let encoded = serde_json::to_string(&value)?;
    let mut edits = Vec::new();
    if let Some(old) = fields.get(key) {
        edits.push((old.range.clone(), encoded.clone()));
    }
    if let Some(parent) = fields.get(section) {
        let children = parent
            .fields
            .as_ref()
            .context("Transfusion section must be an object")?;
        if let Some(old) = children.get(key) {
            edits.push((old.range.clone(), encoded));
        } else if !fields.contains_key(key) {
            let at = parent.range.end - 1;
            edits.push((
                at..at,
                format!(
                    "\n{}\"{key}\": {encoded}\n",
                    if children.is_empty() { "" } else { "," }
                ),
            ));
        }
    } else if !fields.contains_key(key) {
        let at = tree.range.end - 1;
        edits.push((
            at..at,
            format!(
                "\n{}\"{section}\": {{\"{key}\": {encoded}}}\n",
                if fields.is_empty() { "" } else { "," }
            ),
        ));
    }
    edits.sort_by_key(|(range, _)| std::cmp::Reverse(range.start));
    for (range, value) in edits {
        text.replace_range(range, &value);
    }
    let mut out = if bytes.starts_with(&[239, 187, 191]) {
        vec![239, 187, 191]
    } else {
        Vec::new()
    };
    out.extend(text.as_bytes());
    document(&out)?;
    Ok(out)
}
pub fn configure(bytes: &[u8], values: &Values) -> Result<Vec<u8>> {
    crate::presets::validate(PROFILE, values)?;
    document(bytes)?;
    let mut out = bytes.to_vec();
    if let Some(mode) = values.get("tf_mode") {
        out = edit(
            &out,
            "frameGeneration",
            "mode",
            json!(if mode == "game" || mode == "dynamic" {
                mode.as_str()
            } else {
                "fixed"
            }),
        )?;
        if let Ok(n) = mode.parse::<u32>() {
            out = edit(&out, "frameGeneration", "multiplier", json!(n))?;
        }
    }
    for p in parameters().into_iter().skip(1) {
        if let Some(value) = values.get(p.key) {
            out = edit(
                &out,
                p.section,
                p.ini_key,
                if p.key == "tf_target" {
                    json!(value.parse::<u32>()?)
                } else {
                    json!(value == "1")
                },
            )?;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn comments_unknown_values_and_inactive_multiplier_survive() {
        let text = "{\r\n// 中文 https://example/\r\n\"configVersion\":3,\"frameGeneration\":{\"mode\":\"fixed\",\"multiplier\":3},\"unknown\":\"a//b\",\"compatibility\":{\"smoothMotionSm86\":false}}";
        let out = configure(
            text.as_bytes(),
            &Values::from([
                ("tf_mode".into(), "dynamic".into()),
                ("tf_target".into(), "237".into()),
                ("tf_overlay".into(), "1".into()),
            ]),
        )
        .unwrap();
        let s = std::str::from_utf8(&out).unwrap();
        assert!(s.contains("// 中文 https://example/\r\n"));
        assert!(s.contains("\"unknown\":\"a//b\""));
        assert!(s.contains("\"multiplier\":3"));
        let read = read(&out).unwrap();
        assert_eq!(read["tf_mode"], "dynamic");
        assert_eq!(read["tf_target"], "237");
        assert_eq!(read["tf_overlay"], "1");
    }
    #[test]
    fn flat_legacy_and_nested_values_are_updated_without_duplicate_conflict() {
        let out = configure(
            br#"{"mode":"fixed","multiplier":2,"frameGeneration":{"mode":"fixed","multiplier":2}}"#,
            &Values::from([("tf_mode".into(), "4".into())]),
        )
        .unwrap();
        let (_, data, _) = document(&out).unwrap();
        assert_eq!(data["mode"], "fixed");
        assert_eq!(data["multiplier"], 4);
        assert_eq!(data["frameGeneration"]["multiplier"], 4);
    }
    #[test]
    fn malformed_future_duplicate_and_out_of_range_configs_are_rejected() {
        for input in [
            r#"{"configVersion":4}"#,
            r#"{"mode":"game","mode":"fixed"}"#,
            r#"{"mode":"fixed","multiplier":99}"#,
            r#"{/* unfinished"#,
        ] {
            assert!(read(input.as_bytes()).is_err());
        }
        assert!(configure(b"{}", &Values::from([("tf_target".into(), "1001".into())])).is_err());
    }
    #[test]
    fn missing_sections_and_comments_at_end_are_supported() {
        let out = configure(
            b"{\"unknown\":1 // retain this comment\n}",
            &crate::presets::defaults(PROFILE, &Values::new()),
        )
        .unwrap();
        assert_eq!(read(&out).unwrap()["tf_mode"], "game");
        assert!(
            std::str::from_utf8(&out)
                .unwrap()
                .contains("// retain this comment")
        );
    }
}
