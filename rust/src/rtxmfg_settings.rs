//! Bounded MFG JSON/UI edits with recoverable, per-game ownership.
//! The UI INI lives beside the EXE even when the proxy uses a custom directory.
use crate::{core, presets::Values, rtxmfg};
use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use std::{fs, io::Write, path::PathBuf};

const JOURNAL: &str = "mfg-settings.json";
const LIMIT: usize = 1024 * 1024;

pub fn has_pending(directory: &std::path::Path) -> Result<bool> {
    Ok(core::no_links(&directory.join(core::OWN).join(JOURNAL))?.exists())
}

#[derive(Serialize, Deserialize)]
struct Edit {
    name: String,
    before: Option<String>,
    after: String,
}
#[derive(Serialize, Deserialize)]
struct Journal {
    schema: u8,
    exe: String,
    directory: String,
    edits: Vec<Edit>,
}

fn path(target: &core::DeploymentTarget, name: &str) -> Result<PathBuf> {
    let directory = match name {
        rtxmfg::CONFIG => &target.directory,
        rtxmfg::UI_CONFIG => target.game_exe.parent().context("游戏目录无效")?,
        _ => anyhow::bail!("MFG 参数恢复记录的文件范围无效"),
    };
    core::no_links(&directory.join(name))
}
fn read(path: &std::path::Path) -> Result<Option<Vec<u8>>> {
    let path = core::no_links(path)?;
    if !path.exists() {
        return Ok(None);
    }
    let mut file = fs::File::open(path)?;
    ensure!(
        file.metadata()?.is_file() && file.metadata()?.len() <= LIMIT as u64,
        "MFG 配置文件过大或不是普通文件"
    );
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(&mut file, &mut bytes)?;
    ensure!(bytes.len() <= LIMIT, "MFG 配置文件过大");
    Ok(Some(bytes))
}
fn decode(value: &str) -> Result<Vec<u8>> {
    let bytes = STANDARD.decode(value)?;
    ensure!(bytes.len() <= LIMIT, "MFG 参数恢复记录过大");
    Ok(bytes)
}
fn replace(path: &std::path::Path, expected: Option<&[u8]>, next: Option<&[u8]>) -> Result<()> {
    let checked = core::no_links(path)?;
    ensure!(
        read(&checked)?.as_deref() == expected,
        "MFG 参数已被修改，请刷新后重试"
    );
    if let Some(bytes) = next {
        let mut temp = tempfile::NamedTempFile::new_in(checked.parent().context("配置目录无效")?)?;
        temp.write_all(bytes)?;
        temp.as_file().sync_all()?;
        ensure!(
            read(&checked)?.as_deref() == expected,
            "MFG 参数已被修改，请刷新后重试"
        );
        temp.persist(checked)?;
    } else if expected.is_some() {
        fs::remove_file(checked)?;
    }
    Ok(())
}
fn ui_edit(target: &core::DeploymentTarget, values: &Values) -> Result<Option<Edit>> {
    if !values.keys().any(|k| rtxmfg::is_sidecar_key(k)) {
        return Ok(None);
    }
    let before = read(&path(target, rtxmfg::UI_CONFIG)?)?;
    let after = rtxmfg::configure_ui(before.as_deref().unwrap_or_default(), values)?;
    ensure!(after.len() <= LIMIT, "MFG 配置文件过大");
    Ok((before.as_deref() != Some(&after)).then(|| Edit {
        name: rtxmfg::UI_CONFIG.into(),
        before: before.map(|b| STANDARD.encode(b)),
        after: STANDARD.encode(after),
    }))
}

pub fn preflight(
    target: &core::DeploymentTarget,
    version: Option<&str>,
    values: &Values,
) -> Result<()> {
    rtxmfg::validate_version(version, values)?;
    ui_edit(target, values)?;
    Ok(())
}
pub fn read_ui_at(target: &core::DeploymentTarget) -> Result<Values> {
    rtxmfg::read_ui(&read(&path(target, rtxmfg::UI_CONFIG)?)?.unwrap_or_default())
}

/// Caller holds the deployment lock and has checked the game is stopped.
pub fn recover_locked(target: &core::DeploymentTarget) -> Result<()> {
    let journal = core::no_links(&target.directory.join(core::OWN).join(JOURNAL))?;
    if !journal.exists() {
        return Ok(());
    }
    core::assert_target_stopped(target)?;
    ensure!(
        fs::metadata(&journal)?.len() <= (6 * LIMIT) as u64,
        "MFG 参数恢复记录过大"
    );
    let raw = fs::read(&journal)?;
    ensure!(raw.len() <= 6 * LIMIT, "MFG 参数恢复记录过大");
    let j: Journal = serde_json::from_slice(&raw)?;
    ensure!(
        j.schema == 1
            && j.exe == target.game_exe.to_string_lossy()
            && j.directory == target.directory.to_string_lossy()
            && (1..=2).contains(&j.edits.len()),
        "MFG 参数恢复记录不属于当前游戏"
    );
    let record = core::record(&target.directory)?.context("MFG 参数恢复缺少部署记录")?;
    target.validate_record(&record)?;
    ensure!(
        record.backend == rtxmfg::BACKEND,
        "MFG 参数恢复的方案不匹配"
    );
    let mut checked = Vec::new();
    for edit in &j.edits {
        let p = path(target, &edit.name)?;
        ensure!(
            !checked.iter().any(|(q, _, _, _)| q == &p),
            "MFG 参数恢复包含重复文件"
        );
        let before = edit.before.as_deref().map(decode).transpose()?;
        let after = decode(&edit.after)?;
        let current = read(&p)?;
        ensure!(
            current == before || current.as_deref() == Some(&after),
            "MFG 参数已被其他程序修改，恢复记录已保留"
        );
        checked.push((p, before, after, current));
    }
    if !checked
        .iter()
        .all(|(_, _, after, current)| current.as_deref() == Some(after))
    {
        for (p, before, after, current) in &checked {
            if current.as_deref() == Some(after) {
                replace(p, Some(after), before.as_deref())?;
            }
        }
    }
    ensure!(fs::read(&journal)? == raw, "MFG 参数恢复记录已变化");
    fs::remove_file(journal)?;
    Ok(())
}

pub fn apply_locked(
    target: &core::DeploymentTarget,
    json: Option<(&[u8], &[u8])>,
    values: &Values,
) -> Result<()> {
    recover_locked(target)?;
    core::assert_target_stopped(target)?;
    let mut edits = Vec::new();
    if let Some((before, after)) = json.filter(|(before, after)| before != after) {
        ensure!(
            before.len() <= LIMIT && after.len() <= LIMIT,
            "MFG 配置文件过大"
        );
        edits.push(Edit {
            name: rtxmfg::CONFIG.into(),
            before: Some(STANDARD.encode(before)),
            after: STANDARD.encode(after),
        });
    }
    if let Some(edit) = ui_edit(target, values)? {
        edits.push(edit);
    }
    if edits.is_empty() {
        return Ok(());
    }
    let journal = core::no_links(&target.directory.join(core::OWN).join(JOURNAL))?;
    let j = Journal {
        schema: 1,
        exe: target.game_exe.to_string_lossy().into(),
        directory: target.directory.to_string_lossy().into(),
        edits,
    };
    for edit in &j.edits {
        ensure!(
            read(&path(target, &edit.name)?)? == edit.before.as_deref().map(decode).transpose()?,
            "MFG 参数已被修改，请刷新后重试"
        );
    }
    let raw = serde_json::to_vec(&j)?;
    core::write_new(&journal, &raw)?;
    let result = (|| -> Result<()> {
        for edit in &j.edits {
            core::assert_target_stopped(target)?;
            replace(
                &path(target, &edit.name)?,
                edit.before.as_deref().map(decode).transpose()?.as_deref(),
                Some(&decode(&edit.after)?),
            )?;
        }
        ensure!(fs::read(&journal)? == raw, "MFG 参数恢复记录已变化");
        fs::remove_file(journal)?;
        Ok(())
    })();
    if let Err(error) = result {
        recover_locked(target).context(format!("参数应用失败：{error}；自动恢复未完成"))?;
        return Err(error);
    }
    Ok(())
}
