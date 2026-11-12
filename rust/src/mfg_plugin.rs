//! Only the explicitly selected MFG plugin may change OptiScaler's ASI switch.
//! The journal stores original bytes; later user changes always win on restore.
use crate::{core, diagnostics, win};
use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use std::{fs, io::Write, path::Path};

pub const ENTRY: &str = "OptiScaler/plugins/MFGUnlocker.asi";
pub const JOURNAL: &str = "mfg-optiscaler.json";
const CONFIG: &str = "OptiScaler.ini";
const SECTION: &str = "Plugins";
const KEY: &str = "LoadAsiPlugins";
const LIMIT: u64 = 1024 * 1024;

pub fn supported(version: Option<&str>) -> bool {
    version
        .and_then(|s| semver::Version::parse(s.trim_start_matches('v')).ok())
        .is_some_and(|v| {
            let hotfix = v.pre.as_str().strip_prefix("hotfix.").is_some_and(|n| {
                !n.is_empty()
                    && n.bytes().all(|c| c.is_ascii_digit())
                    && n.parse::<u16>().is_ok_and(|n| n > 0)
            });
            (v.major, v.minor, v.patch) >= (1, 5, 0) && (v.pre.is_empty() || hotfix)
        })
}

fn read_config(dir: &Path) -> Result<Vec<u8>> {
    let path = core::no_links(&dir.join(CONFIG))?;
    ensure!(
        path.is_file() && path.metadata()?.len() <= LIMIT,
        "插件模式需要游戏 EXE 旁已有的 OptiScaler.ini；请先安装 OptiScaler 10"
    );
    Ok(fs::read(path)?)
}

pub fn validate_host(target: &core::DeploymentTarget) -> Result<()> {
    ensure!(
        target.game_exe.parent() == Some(target.directory.as_path()),
        "MFG 插件模式请使用游戏 EXE 所在目录；配置和 OptiScaler 主文件必须在此目录"
    );
    let mut found = false;
    for name in core::PROXIES
        .iter()
        .copied()
        .filter(|n| n.ends_with(".dll"))
        .chain(["OptiScaler.dll", "OptiScaler.asi"])
    {
        let path = core::no_links(&target.directory.join(name))?;
        if path.is_file()
            && let Ok((version, internal)) = win::file_version(&path)
            && internal.eq_ignore_ascii_case("OptiScaler")
        {
            ensure!(
                version
                    .split('.')
                    .next()
                    .and_then(|n| n.parse::<u32>().ok())
                    == Some(10),
                "检测到 OptiScaler {version}；MFG 插件需要 OptiScaler 10"
            );
            core::pe64(&path, true)?;
            found = true;
        }
    }
    ensure!(
        found,
        "未在游戏 EXE 旁识别到 OptiScaler 10；管理器不会自动安装 OptiScaler"
    );
    let bytes = read_config(&target.directory)?;
    let (text, _) = diagnostics::decode_ini(&bytes)?;
    for (section, key, expected) in [
        ("Plugins", "Path", "OptiScaler/plugins"),
        ("Libraries", "OptiDllPath", "OptiScaler"),
    ] {
        if let Some(value) = diagnostics::ini_value(&text, section, key)? {
            let normalized = value.replace('\\', "/");
            ensure!(
                normalized.is_empty()
                    || normalized.eq_ignore_ascii_case("auto")
                    || normalized
                        .trim_end_matches('/')
                        .eq_ignore_ascii_case(expected)
                    || Path::new(&value) == target.directory.join(expected),
                "OptiScaler 使用自定义 {section}/{key}；请保留原设置，当前自动插件安装仅支持默认 OptiScaler/plugins 目录"
            );
        }
    }
    diagnostics::edit_ini(&bytes, SECTION, KEY, "true")?;
    Ok(())
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    schema: u8,
    game_exe: String,
    before: String,
    applied: String,
}

fn replace(target: &core::DeploymentTarget, before: &[u8], after: &[u8]) -> Result<()> {
    if before == after {
        return Ok(());
    }
    let path = core::no_links(&target.directory.join(CONFIG))?;
    let mut temp = tempfile::NamedTempFile::new_in(&target.directory)?;
    temp.write_all(after)?;
    temp.as_file().sync_all()?;
    core::assert_target_stopped(target)?;
    ensure!(
        read_config(&target.directory)? == before,
        "OptiScaler.ini 已变化，请刷新重试"
    );
    temp.persist(&path)?;
    Ok(())
}

/// Caller holds the game's lock and has persisted the install marker.
pub fn enable_locked(target: &core::DeploymentTarget) -> Result<()> {
    validate_host(target)?;
    enable_switch_locked(target)
}
fn enable_switch_locked(target: &core::DeploymentTarget) -> Result<()> {
    let path = core::no_links(&target.directory.join(core::OWN).join(JOURNAL))?;
    ensure!(!path.exists(), "存在待恢复的 OptiScaler 设置，请先卸载补丁");
    let before = read_config(&target.directory)?;
    let applied = diagnostics::edit_ini(&before, SECTION, KEY, "true")?;
    // A switch that was already on belongs to the user; do not claim it.
    let (text, _) = diagnostics::decode_ini(&before)?;
    if diagnostics::ini_value(&text, SECTION, KEY)?.is_some_and(|v| v.eq_ignore_ascii_case("true"))
    {
        return Ok(());
    }
    let j = Journal {
        schema: 1,
        game_exe: target.game_exe.to_string_lossy().into_owned(),
        before: STANDARD.encode(&before),
        applied: STANDARD.encode(&applied),
    };
    core::write_new(&path, &serde_json::to_vec(&j)?)?;
    replace(target, &before, &applied)
}

fn restored(before: &[u8], applied: &[u8], current: &[u8]) -> Result<Vec<u8>> {
    ensure!(
        diagnostics::edit_ini(before, SECTION, KEY, "true")? == applied,
        "OptiScaler 恢复记录不匹配，仅允许恢复插件加载开关"
    );
    if current == applied {
        return Ok(before.to_vec());
    }
    if current == before {
        return Ok(current.to_vec());
    }
    let (now, _) = diagnostics::decode_ini(current)?;
    if !diagnostics::ini_value(&now, SECTION, KEY)?.is_some_and(|v| v.eq_ignore_ascii_case("true"))
    {
        return Ok(current.to_vec()); // User changed or removed the switch.
    }
    let (old, _) = diagnostics::decode_ini(before)?;
    match diagnostics::ini_value(&old, SECTION, KEY)? {
        Some(value) => diagnostics::edit_ini(current, SECTION, KEY, &value),
        None => diagnostics::remove_ini_key(current, SECTION, KEY),
    }
}

/// Run after removing the owned ASI, before deleting install.json. Missing or
/// user-modified INI files are never resurrected. Failures retain both journals.
pub fn restore_locked(target: &core::DeploymentTarget) -> Result<()> {
    let path = core::no_links(&target.directory.join(core::OWN).join(JOURNAL))?;
    if !path.exists() {
        return Ok(());
    }
    ensure!(
        !core::no_links(&target.directory.join(ENTRY))?.exists(),
        "MFG 插件文件仍存在，OptiScaler 设置恢复暂缓"
    );
    let j: Journal = serde_json::from_value(core::read_json(&path, 3 * LIMIT)?)?;
    ensure!(
        j.schema == 1 && j.game_exe == target.game_exe.to_string_lossy(),
        "OptiScaler 恢复记录属于其他游戏"
    );
    let before = STANDARD
        .decode(&j.before)
        .context("OptiScaler 恢复记录无效")?;
    let applied = STANDARD
        .decode(&j.applied)
        .context("OptiScaler 恢复记录无效")?;
    ensure!(
        before.len() as u64 <= LIMIT && applied.len() as u64 <= LIMIT,
        "OptiScaler 恢复记录过大"
    );
    ensure!(
        diagnostics::edit_ini(&before, SECTION, KEY, "true")? == applied,
        "OptiScaler 恢复记录无效"
    );
    if core::no_links(&target.directory.join(CONFIG))?.exists() {
        let current = read_config(&target.directory)?;
        let after = restored(&before, &applied, &current)?;
        replace(target, &current, &after)?;
    }
    fs::remove_file(path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn uninstall_removes_owned_nested_plugin_and_restores_switch() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let mut exe = vec![0u8; 512];
        exe[..2].copy_from_slice(b"MZ");
        exe[60..64].copy_from_slice(&128u32.to_le_bytes());
        exe[128..134].copy_from_slice(b"PE\0\0\x64\x86");
        exe[148..150].copy_from_slice(&240u16.to_le_bytes());
        exe[150..152].copy_from_slice(&2u16.to_le_bytes());
        exe[152..154].copy_from_slice(&0x20bu16.to_le_bytes());
        let game = temp.path().join("synthetic-game.exe");
        fs::write(&game, exe)?;
        let target = core::DeploymentTarget::for_game(&game).validate(true)?;
        fs::create_dir(target.directory.join(core::OWN))?;
        let before = b"[Plugins]\r\nLoadAsiPlugins=false\r\n[Other]\r\nKeep=1\r\n";
        fs::write(target.directory.join(CONFIG), before)?;
        enable_switch_locked(&target)?;
        assert!(target.directory.join(core::OWN).join(JOURNAL).exists());
        fs::create_dir_all(target.directory.join("OptiScaler/plugins"))?;
        fs::write(
            target.directory.join(ENTRY),
            b"synthetic owned plugin; never loaded",
        )?;
        fs::write(
            target.directory.join("OptiScaler/plugins/other.asi"),
            b"other plugin",
        )?;
        fs::write(target.directory.join(crate::rtxmfg::CONFIG), b"{}")?;
        fs::write(
            target.directory.join(crate::rtxmfg::NOTICES),
            b"synthetic notice",
        )?;
        let record = core::Record {
            schema: 3,
            backend: crate::rtxmfg::BACKEND.into(),
            game_exe: target.record_exe(),
            payload_version: Some("1.5.0".into()),
            upstream_version: Some("1.5.0".into()),
            proxy: ENTRY.into(),
            proxies: vec![ENTRY.into()],
            hashes: [ENTRY, crate::rtxmfg::CONFIG, crate::rtxmfg::NOTICES]
                .into_iter()
                .map(|n| Ok((n.to_owned(), core::digest(&target.directory.join(n))?)))
                .collect::<Result<_>>()?,
            cleanup_dirs: Default::default(),
            scheme_id: Some(crate::rtxmfg_upgrade::SCHEME.into()),
            delta_cache_ids: vec![],
            delta_legacy_cache: false,
            cache_pending: false,
            cleanup_pending_reasons: Vec::new(),
        };
        core::write_new(
            &target.directory.join(core::OWN).join(core::MARKER),
            &serde_json::to_vec(&record)?,
        )?;
        assert!(restore_locked(&target).is_err()); // ASI still active: retain journal.
        let outcome = crate::cleanup::clean_outcome_at(&target)?;
        assert!(outcome.complete);
        assert_eq!(fs::read(target.directory.join(CONFIG))?, before);
        assert_eq!(
            fs::read(target.directory.join("OptiScaler/plugins/other.asi"))?,
            b"other plugin"
        );
        assert!(!target.directory.join(ENTRY).exists());
        assert!(!target.directory.join(core::OWN).join(JOURNAL).exists());
        assert!(!target.directory.join(crate::rtxmfg::NOTICES).exists());
        Ok(())
    }
    #[test]
    fn preserves_later_user_edits_and_exact_original_bytes() {
        let before = b"\xef\xbb\xbf; original\r\n[Plugins]\r\nLoadAsiPlugins = false ; keep\r\n[Other]\r\nValue=1\r\n";
        let applied = diagnostics::edit_ini(before, SECTION, KEY, "true").unwrap();
        assert_eq!(restored(before, &applied, &applied).unwrap(), before);
        let current = diagnostics::edit_ini(&applied, "Other", "Value", "2").unwrap();
        let after = restored(before, &applied, &current).unwrap();
        let (text, _) = diagnostics::decode_ini(&after).unwrap();
        assert_eq!(
            diagnostics::ini_value(&text, "Other", "Value")
                .unwrap()
                .as_deref(),
            Some("2")
        );
        assert_eq!(
            diagnostics::ini_value(&text, SECTION, KEY)
                .unwrap()
                .as_deref(),
            Some("false")
        );
        let current = diagnostics::edit_ini(&current, SECTION, KEY, "auto").unwrap();
        assert_eq!(restored(before, &applied, &current).unwrap(), current);
        assert!(restored(before, b"[Plugins]\nLoadAsiPlugins=true\n", &current).is_err());
    }
    #[test]
    fn removes_only_manager_added_key_in_utf16() {
        let mut before = vec![255, 254];
        for word in "[Plugins]\r\n; keep\r\n[Other]\r\nValue=1\r\n".encode_utf16() {
            before.extend(word.to_le_bytes());
        }
        let applied = diagnostics::edit_ini(&before, SECTION, KEY, "true").unwrap();
        let changed = diagnostics::edit_ini(&applied, "Other", "Value", "2").unwrap();
        let after = restored(&before, &applied, &changed).unwrap();
        assert_eq!(&after[..2], &[255, 254]);
        let (text, _) = diagnostics::decode_ini(&after).unwrap();
        assert!(
            diagnostics::ini_value(&text, SECTION, KEY)
                .unwrap()
                .is_none()
        );
        assert!(text.contains("; keep\r\n"));
        assert_eq!(
            diagnostics::ini_value(&text, "Other", "Value")
                .unwrap()
                .as_deref(),
            Some("2")
        );
    }
}
