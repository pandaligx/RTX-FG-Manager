//! Shareable parameter envelopes and byte-exact, conflict-aware configuration history.
use crate::{
    core,
    presets::{self, Values},
    win,
};
use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::OnceLock,
};

fn protocols() -> &'static serde_json::Value {
    static DATA: OnceLock<serde_json::Value> = OnceLock::new();
    DATA.get_or_init(|| {
        serde_json::from_str(include_str!("../assets/community-protocols.json"))
            .expect("embedded protocols")
    })
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Envelope {
    pub schema: u32,
    pub game: String,
    pub scheme: String,
    pub protocol: String,
    pub name: String,
    pub settings: Values,
}
#[derive(Clone, Debug, Serialize)]
pub struct Difference {
    pub key: String,
    pub before: Option<String>,
    pub after: String,
}
#[derive(Clone, Debug)]
pub struct Preview {
    pub differences: Vec<Difference>,
    pub rejected: BTreeMap<String, String>,
    before: Vec<u8>,
    after: Vec<u8>,
    scheme: String,
    protocol: String,
    filename: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct History {
    pub id: String,
    pub name: String,
    pub created: String,
    pub scheme: String,
    pub protocol: String,
    pub filename: String,
    pub before_hash: String,
    pub after_hash: String,
    before_base64: String,
    after_base64: String,
    pub state: String,
}
pub fn valid_game(game: &str) -> bool {
    if let Some(id) = game.strip_prefix("steam:") {
        return !id.is_empty()
            && id.len() <= 10
            && !id.starts_with('0')
            && id.bytes().all(|c| c.is_ascii_digit());
    }
    game.strip_prefix("game:").is_some_and(|id| {
        (2..=80).contains(&id.len())
            && id.as_bytes()[0].is_ascii_alphanumeric()
            && id
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
    })
}
pub fn safe_name(name: &str) -> bool {
    !name.trim().is_empty()
        && name.chars().count() <= 80
        && !name.chars().any(|c| c.is_control() || "<>".contains(c))
}
pub fn accepted(
    scheme: &str,
    protocol: &str,
    values: &Values,
) -> Result<(Values, BTreeMap<String, String>)> {
    static NUMBER: OnceLock<regex::Regex> = OnceLock::new();
    let fields = protocols()
        .get(scheme)
        .and_then(|p| p.get(protocol))
        .and_then(|p| p.as_object())
        .context("当前补丁配置协议不支持社区导入")?;
    // Encore 1.4 adds fields to a full local snapshot. The protocol allowlist
    // below still rejects private/unknown settings individually.
    ensure!(values.len() <= 256, "配置参数过多");
    let mut allowed = Values::new();
    let mut rejected = BTreeMap::new();
    let number = NUMBER.get_or_init(|| regex::Regex::new(r"^-?(?:0|[1-9]\d*)(?:\.\d+)?$").unwrap());
    for (key, value) in values {
        let valid = fields.get(key).is_some_and(|field| {
            if value.len() > 128 {
                return false;
            }
            if let Some(choices) = field["values"].as_array() {
                return choices.iter().any(|v| v.as_str() == Some(value));
            }
            let lexical = number.is_match(value);
            lexical
                && value.parse::<f64>().is_ok_and(|n| {
                    n.is_finite()
                        && (!field["integer"].as_bool().unwrap_or(false) || n.fract() == 0.0)
                        && field["min"].as_f64().is_none_or(|m| n >= m)
                        && field["max"].as_f64().is_none_or(|m| n <= m)
                })
        });
        if valid {
            allowed.insert(key.clone(), value.clone());
        } else {
            rejected.insert(
                key.clone(),
                if fields.contains_key(key) {
                    "值不在当前协议允许范围"
                } else {
                    "未知字段、快捷键、诊断或本机路径不参与导入"
                }
                .into(),
            );
        }
    }
    Ok((allowed, rejected))
}
pub fn read_envelope(path: &Path, game: &str, scheme: &str, protocol: &str) -> Result<Envelope> {
    let envelope: Envelope = serde_json::from_value(core::read_json(path, 65536)?)?;
    ensure!(
        envelope.schema == 1 && valid_game(&envelope.game) && safe_name(&envelope.name),
        "配置文件格式无效"
    );
    ensure!(
        envelope.game == game && envelope.scheme == scheme && envelope.protocol == protocol,
        "配置属于不同游戏、补丁或协议"
    );
    accepted(scheme, protocol, &envelope.settings)?;
    Ok(envelope)
}
pub fn save_envelope(path: &Path, envelope: &Envelope) -> Result<()> {
    ensure!(
        envelope.schema == 1 && valid_game(&envelope.game) && safe_name(&envelope.name),
        "配置名称或游戏无效"
    );
    let (settings, rejected) = accepted(&envelope.scheme, &envelope.protocol, &envelope.settings)?;
    ensure!(
        !settings.is_empty() && rejected.is_empty(),
        "导出只接受合法的可共享参数"
    );
    core::no_links(path)?;
    core::write_new(path, &serde_json::to_vec_pretty(envelope)?)
}
pub fn snapshot(
    target: &core::DeploymentTarget,
    context: &presets::Context,
    game: &str,
    name: &str,
) -> Result<Envelope> {
    let target = target.validate(true)?;
    let path = config_path(&target, &context.scheme, &context.profile)?;
    let values = presets::read_values(&fs::read(path)?, &context.profile)?;
    let (settings, _) = accepted(&context.scheme, &context.profile, &values)?;
    ensure!(
        valid_game(game) && safe_name(name) && !settings.is_empty(),
        "请选择游戏 ID 并输入配置名称"
    );
    Ok(Envelope {
        schema: 1,
        game: game.into(),
        scheme: context.scheme.clone(),
        protocol: context.profile.clone(),
        name: name.into(),
        settings,
    })
}
pub fn named(
    target: &core::DeploymentTarget,
    game: &str,
    scheme: &str,
    protocol: &str,
) -> Result<Vec<(String, Envelope)>> {
    let root = history_root(target)?.join("named");
    if !root.try_exists()? {
        return Ok(Vec::new());
    }
    core::no_links(&root)?;
    let mut result = Vec::new();
    for entry in fs::read_dir(root)?.take(500) {
        let path = entry?.path();
        let Some(id) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        if uuid::Uuid::parse_str(id).is_ok()
            && path.extension().is_some_and(|s| s == "json")
            && let Ok(envelope) = read_envelope(&path, game, scheme, protocol)
        {
            result.push((id.into(), envelope));
        }
    }
    result.sort_by(|a, b| a.1.name.cmp(&b.1.name));
    Ok(result)
}
pub fn save_named(target: &core::DeploymentTarget, envelope: &Envelope) -> Result<String> {
    let root = history_root(target)?.join("named");
    core::no_links(&root)?;
    fs::create_dir_all(&root)?;
    ensure!(
        fs::read_dir(&root)?.take(500).count() < 500,
        "本地配置已达到 500 份，请先导出整理"
    );
    let id = uuid::Uuid::new_v4().to_string();
    save_envelope(&root.join(format!("{id}.json")), envelope)?;
    Ok(id)
}
fn config_path(target: &core::DeploymentTarget, scheme: &str, protocol: &str) -> Result<PathBuf> {
    let record = core::record(&target.directory)?.context("请先安装补丁")?;
    target.validate_record(&record)?;
    ensure!(
        record.scheme_id.as_deref() == Some(scheme)
            && core::backend_matches_profile(&record.backend, protocol),
        "已安装的补丁或配置协议不同"
    );
    ensure!(
        core::status_at(target).starts_with("已部署"),
        "补丁文件已变化，请先检查部署记录"
    );
    let path = core::no_links(&target.directory.join(core::config_name(&record.backend)))?;
    ensure!(path.metadata()?.len() <= 1024 * 1024, "配置文件过大");
    Ok(path)
}
pub fn preview(
    target: &core::DeploymentTarget,
    context: &presets::Context,
    values: &Values,
) -> Result<Preview> {
    let target = target.validate(true)?;
    let path = config_path(&target, &context.scheme, &context.profile)?;
    let before = fs::read(&path)?;
    preview_bytes(
        &before,
        &context.scheme,
        &context.profile,
        path.file_name()
            .unwrap()
            .to_str()
            .context("配置文件名无效")?,
        values,
    )
}
pub fn select_preview(
    preview: &Preview,
    keys: &std::collections::BTreeSet<String>,
) -> Result<Preview> {
    let values = preview
        .differences
        .iter()
        .filter(|d| keys.contains(&d.key))
        .map(|d| (d.key.clone(), d.after.clone()))
        .collect();
    preview_bytes(
        &preview.before,
        &preview.scheme,
        &preview.protocol,
        &preview.filename,
        &values,
    )
}
fn preview_bytes(
    before: &[u8],
    scheme: &str,
    protocol: &str,
    filename: &str,
    values: &Values,
) -> Result<Preview> {
    let (allowed, rejected) = accepted(scheme, protocol, values)?;
    let original = presets::read_values(before, protocol)?;
    let differences = allowed
        .iter()
        .filter(|(k, v)| original.get(*k) != Some(*v))
        .map(|(key, after)| Difference {
            key: key.clone(),
            before: original.get(key).cloned(),
            after: after.clone(),
        })
        .collect();
    let after = presets::configure(before, protocol, &allowed)?;
    Ok(Preview {
        differences,
        rejected,
        before: before.to_vec(),
        after,
        scheme: scheme.into(),
        protocol: protocol.into(),
        filename: filename.into(),
    })
}
pub fn history_root(target: &core::DeploymentTarget) -> Result<PathBuf> {
    let base = PathBuf::from(std::env::var_os("LOCALAPPDATA").context("LOCALAPPDATA unavailable")?)
        .join("RTXFGManager")
        .join("configuration-history");
    let identity = core::hash(
        format!(
            "{}\n{}",
            core::key(&target.game_exe),
            core::key(&target.directory)
        )
        .as_bytes(),
    );
    core::no_links(&base.join(identity))
}
fn replace_expected(path: &Path, before: &[u8], after: &[u8]) -> Result<()> {
    core::no_links(path)?;
    ensure!(
        fs::read(path)? == before,
        "配置在预览后已被修改，请重新预览差异"
    );
    let mut stage = tempfile::NamedTempFile::new_in(path.parent().context("配置路径无效")?)?;
    stage.write_all(after)?;
    stage.as_file().sync_all()?;
    core::no_links(path)?;
    ensure!(
        fs::read(path)? == before,
        "配置发生并发修改，未覆盖后续改动"
    );
    stage.persist(path)?;
    Ok(())
}
pub fn apply(target: &core::DeploymentTarget, preview: &Preview, name: &str) -> Result<String> {
    ensure!(safe_name(name), "配置名称无效");
    ensure!(!preview.differences.is_empty(), "没有可导入的变化");
    let target = target.validate(true)?;
    let _lock = win::game_lock(&target.directory)?;
    core::assert_target_stopped(&target)?;
    let path = config_path(&target, &preview.scheme, &preview.protocol)?;
    ensure!(
        path.file_name().and_then(|v| v.to_str()) == Some(&preview.filename),
        "配置目标已变化"
    );
    ensure!(
        fs::read(&path)? == preview.before,
        "配置在预览后已被修改，请重新预览"
    );
    let directory = history_root(&target)?;
    fs::create_dir_all(&directory)?;
    let id = uuid::Uuid::new_v4().to_string();
    let mut record = History {
        id: id.clone(),
        name: name.into(),
        created: chrono::Utc::now().to_rfc3339(),
        scheme: preview.scheme.clone(),
        protocol: preview.protocol.clone(),
        filename: preview.filename.clone(),
        before_hash: core::hash(&preview.before),
        after_hash: core::hash(&preview.after),
        before_base64: STANDARD.encode(&preview.before),
        after_base64: STANDARD.encode(&preview.after),
        state: "prepared".into(),
    };
    let history = directory.join(format!("{id}.json"));
    core::write_new(&history, &serde_json::to_vec(&record)?)?;
    core::assert_target_stopped(&target)?;
    replace_expected(&path, &preview.before, &preview.after)?;
    record.state = "applied".into();
    core::atomic_json(&history, &record)?;
    Ok(id)
}
pub fn history(target: &core::DeploymentTarget) -> Result<Vec<History>> {
    let root = history_root(target)?;
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut result = Vec::new();
    for item in fs::read_dir(root)?.take(2000) {
        let path = item?.path();
        if path.extension().is_none_or(|e| e != "json") {
            continue;
        }
        if let Ok(value) = core::read_json(&path, 3 * 1024 * 1024)
            && let Ok(entry) = serde_json::from_value::<History>(value)
        {
            result.push(entry);
        }
    }
    result.sort_by(|a, b| b.created.cmp(&a.created));
    Ok(result)
}
pub fn undo(target: &core::DeploymentTarget, id: &str) -> Result<()> {
    ensure!(
        uuid::Uuid::parse_str(id).is_ok_and(|u| u.to_string() == id),
        "恢复编号无效"
    );
    let target = target.validate(true)?;
    let _lock = win::game_lock(&target.directory)?;
    core::assert_target_stopped(&target)?;
    let history = history_root(&target)?.join(format!("{id}.json"));
    let mut record: History = serde_json::from_value(core::read_json(&history, 3 * 1024 * 1024)?)?;
    ensure!(
        record.id == id && ["prepared", "applied"].contains(&record.state.as_str()),
        "此记录不可撤销"
    );
    let path = config_path(&target, &record.scheme, &record.protocol)?;
    ensure!(
        path.file_name().and_then(|v| v.to_str()) == Some(&record.filename),
        "恢复记录不属于当前配置"
    );
    let before = STANDARD.decode(&record.before_base64)?;
    let after = STANDARD.decode(&record.after_base64)?;
    ensure!(
        core::hash(&before) == record.before_hash && core::hash(&after) == record.after_hash,
        "恢复记录损坏"
    );
    if fs::read(&path)? != before {
        replace_expected(&path, &after, &before)?;
    }
    record.state = "undone".into();
    core::atomic_json(&history, &record)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn imports_reject_local_fields_and_keep_unknown_jsonc_bytes() {
        let raw=b"\xef\xbb\xbf{\r\n // Keep me\r\n \"frameGeneration\":{\"mode\":\"game\",\"multiplier\":4}, \"unknown\":123\r\n}";
        let values = Values::from([
            ("tf_mode".into(), "3".into()),
            ("logDirectory".into(), "C:\\Users\\private".into()),
            ("unknown".into(), "1".into()),
        ]);
        let preview = preview_bytes(
            raw,
            crate::encore::SCHEME,
            crate::encore::PROFILE,
            crate::encore::CONFIG,
            &values,
        )
        .unwrap();
        assert_eq!(preview.rejected.len(), 2);
        assert_eq!(preview.differences.len(), 1);
        assert_eq!(crate::encore::read(&preview.after).unwrap()["tf_mode"], "3");
        assert!(!String::from_utf8_lossy(&preview.after).contains("private"));
        assert!(String::from_utf8_lossy(&preview.after).contains("// Keep me"));
        assert!(String::from_utf8_lossy(&preview.after).contains("\"unknown\":123"));
    }
    #[test]
    fn exact_restore_refuses_later_changes() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.jsonc");
        let original = b"\xef\xbb\xbf{\r\n // original\r\n}\r\n";
        fs::write(&path, original).unwrap();
        replace_expected(&path, original, b"imported").unwrap();
        fs::write(&path, b"game changed this").unwrap();
        assert!(replace_expected(&path, b"imported", original).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"game changed this");
        fs::write(&path, b"imported").unwrap();
        replace_expected(&path, b"imported", original).unwrap();
        assert_eq!(fs::read(&path).unwrap(), original);
    }
}
