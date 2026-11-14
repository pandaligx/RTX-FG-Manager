//! ANR FG full-package integration. No scripts are executed and no existing
//! OptiScaler installation is adopted or overwritten.
use crate::{
    core, diagnostics,
    presets::{Parameter, Values},
};
use anyhow::{Result, ensure};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

pub const BACKEND: &str = "anr";
pub const PROFILE: &str = "anr_fg_b15";
pub const SCHEME: &str = "anr-fg";
pub const SOURCE: &str = "https://github.com/apex-compute/ANR";
pub const CONFIG: &str = "OptiScaler/custom/Apex Neural Rendering FG/dlssg/nr_fg.ini";
pub const PROVIDER_DIR: &str = "OptiScaler/custom/Apex Neural Rendering FG/dlssg";
pub const PROXIES: [&str; 3] = ["dxgi.dll", "winmm.dll", "version.dll"];
pub const COMPONENTS: [&str; 21] = [
    "OptiScaler.ini",
    "ANR-THIRD-PARTY-NOTICES.txt",
    "ANR-OptiScaler-GPL-3.0.txt",
    "OptiScaler/custom/Apex Neural Rendering FG/dlssg/nvngx_dlssg.dll",
    "OptiScaler/custom/Apex Neural Rendering FG/dlssg/path_table.json",
    "OptiScaler/custom/Apex Neural Rendering FG/dlssg/size_table.json",
    "OptiScaler/custom/Apex Neural Rendering FG/dlssg/adapters/pack.nrx",
    "OptiScaler/custom/Apex Neural Rendering FG/dlssg/kernels/pack.nrx",
    "OptiScaler/custom/Apex Neural Rendering FG/dlssg/kernels-portable/pack.nrx",
    "OptiScaler/custom/Apex Neural Rendering FG/dlssg/programs/fg-lite-any-coopmat-pq10/pack.nrx",
    "OptiScaler/custom/Apex Neural Rendering FG/dlssg/programs/fg-lite-any-coopmat-srgb8/pack.nrx",
    "OptiScaler/custom/Apex Neural Rendering FG/dlssg/programs/fg-lite-any-dp4a-pq10/pack.nrx",
    "OptiScaler/custom/Apex Neural Rendering FG/dlssg/programs/fg-lite-any-dp4a-srgb8/pack.nrx",
    "OptiScaler/custom/Apex Neural Rendering FG/dlssg/programs/fg-lite-any-packed-pq10/pack.nrx",
    "OptiScaler/custom/Apex Neural Rendering FG/dlssg/programs/fg-lite-any-packed-srgb8/pack.nrx",
    "OptiScaler/custom/Apex Neural Rendering FG/dlssg/programs/fg-lite-any-portable-linalg-pq10/pack.nrx",
    "OptiScaler/custom/Apex Neural Rendering FG/dlssg/programs/fg-lite-any-portable-linalg-srgb8/pack.nrx",
    "OptiScaler/custom/Apex Neural Rendering FG/dlssg/programs/fg-lite-any-portable-pq10/pack.nrx",
    "OptiScaler/custom/Apex Neural Rendering FG/dlssg/programs/fg-lite-any-portable-srgb8/pack.nrx",
    "OptiScaler/custom/Apex Neural Rendering FG/dlssg/programs/fg-lite-any-portable-xmx-pq10/pack.nrx",
    "OptiScaler/custom/Apex Neural Rendering FG/dlssg/programs/fg-lite-any-portable-xmx-srgb8/pack.nrx",
];

pub fn parameters() -> Vec<Parameter> {
    vec![
        Parameter {
            key: "max_generated",
            section: "",
            ini_key: "max_generated",
            label: "倍率上限",
            default: "5",
            choices: vec![
                ("1", "2X"),
                ("2", "3X"),
                ("3", "4X"),
                ("4", "5X"),
                ("5", "6X"),
            ],
        },
        Parameter {
            key: "scene_cut",
            section: "",
            ini_key: "scene_cut",
            label: "场景切换保护",
            default: "1",
            choices: vec![("1", "开启"), ("0", "关闭")],
        },
        Parameter {
            key: "path",
            section: "",
            ini_key: "path",
            label: "计算路径",
            default: "auto",
            choices: vec![
                ("auto", "自动"),
                ("coopmat", "Cooperative Matrix"),
                ("packed", "Packed"),
                ("dp4a", "DP4A"),
                ("portable", "Portable"),
                ("portable-linalg", "Portable Linear Algebra"),
                ("portable-xmx", "Portable XMX"),
            ],
        },
    ]
}

/// Flat INI; do not use the sectioned INI writer, which would add an invalid [].
pub fn read(bytes: &[u8]) -> Result<Values> {
    let (text, _) = diagnostics::decode_ini(bytes)?;
    let mut values = Values::new();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with([';', '#']) || line.is_empty() {
            continue;
        }
        ensure!(!line.starts_with('['), "ANR 配置不使用 INI 分节");
        if let Some((key, value)) = line.split_once('=') {
            let key = key.trim();
            if parameters().iter().any(|p| p.key == key) {
                let value = value.split([';', '#']).next().unwrap_or("").trim();
                ensure!(
                    values.insert(key.into(), value.into()).is_none(),
                    "ANR 配置存在重复参数：{key}"
                );
            }
        }
    }
    crate::presets::validate(PROFILE, &values)?;
    Ok(values)
}
pub fn configure(bytes: &[u8], values: &Values) -> Result<Vec<u8>> {
    crate::presets::validate(PROFILE, values)?;
    read(bytes)?;
    // Preserve UTF-8 BOM, comments, unknown keys, line endings and inactive settings.
    let bom = bytes.starts_with(&[239, 187, 191]);
    let text = std::str::from_utf8(if bom { &bytes[3..] } else { bytes })?;
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let mut output = String::new();
    let mut seen = BTreeSet::new();
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if !trimmed.starts_with([';', '#'])
            && let Some((key, _)) = line.split_once('=')
            && let Some(value) = values.get(key.trim())
        {
            seen.insert(key.trim().to_owned());
            let rest = &line[line.find('=').unwrap() + 1..];
            let comment = rest
                .find([';', '#'])
                .map(|i| rest[i..].trim_end_matches(['\r', '\n']));
            let ending = if line.ends_with("\r\n") {
                "\r\n"
            } else if line.ends_with('\n') {
                "\n"
            } else {
                ""
            };
            output.push_str(&format!(
                "{key}= {value}{}{}{ending}",
                if comment.is_some() { " " } else { "" },
                comment.unwrap_or("")
            ));
        } else {
            output.push_str(line);
        }
    }
    for (key, value) in values {
        if !seen.contains(key) {
            if !output.is_empty() && !output.ends_with('\n') {
                output.push_str(newline);
            }
            output.push_str(&format!("{key} = {value}{newline}"));
        }
    }
    let mut result = if bom { vec![239, 187, 191] } else { Vec::new() };
    result.extend_from_slice(output.as_bytes());
    Ok(result)
}
pub fn preflight(target: &core::DeploymentTarget, managed: bool) -> Result<()> {
    ensure!(
        target.game_exe.parent() == Some(target.directory.as_path()),
        "ANR 完整方案请安装到游戏 EXE 所在目录"
    );
    if managed {
        return Ok(());
    }
    for name in [
        "OptiScaler",
        "OptiScaler.ini",
        "OptiScaler.dll",
        "OptiScaler.asi",
        "OptiScaler.log",
    ] {
        ensure!(
            !core::no_links(&target.directory.join(name))?.exists(),
            "检测到已有 {name}。ANR 完整包包含 OptiScaler，请先用原安装方式还原；已有 OptiScaler 的游戏应使用上游 addon，管理器不会覆盖原文件"
        );
    }
    for name in core::PROXIES {
        let path = core::no_links(&target.directory.join(name))?;
        if path.is_file()
            && let Ok((_, internal)) = crate::win::file_version(&path)
        {
            ensure!(
                !internal.to_ascii_lowercase().contains("optiscaler"),
                "检测到已有 OptiScaler 入口 {name}，请先还原原安装"
            );
        }
    }
    Ok(())
}

/// Only the recorded full installation owns the host INI/log and this provider's
/// logs. Preserve unrelated plugins, captures and files added after installation.
pub fn cleanup_targets(
    directory: &Path,
    targets: &mut BTreeMap<PathBuf, String>,
    dirs: &mut BTreeSet<PathBuf>,
) -> Result<()> {
    for name in ["OptiScaler.ini", "OptiScaler.log"] {
        let path = core::no_links(&directory.join(name))?;
        if path.is_file() {
            targets.insert(path.clone(), core::digest(&path)?);
        }
    }
    let provider = core::no_links(&directory.join(PROVIDER_DIR))?;
    if provider.is_dir() {
        for entry in fs::read_dir(&provider)? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with("nr_fg-") && name.ends_with(".log") {
                let path = core::no_links(&entry.path())?;
                if path.is_file() {
                    targets.insert(path.clone(), core::digest(&path)?);
                }
            }
        }
    }
    cleanup_directories(directory, dirs);
    Ok(())
}

/// Also used by residual cleanup; only empty owned ancestors are removed.
pub fn cleanup_directories(directory: &Path, dirs: &mut BTreeSet<PathBuf>) {
    for name in COMPONENTS.iter().copied().chain([CONFIG]) {
        let mut parent = directory.join(name);
        while parent.pop() && parent != directory && parent.starts_with(directory) {
            dirs.insert(parent.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn flat_config_preserves_unknown_comments_and_defaults() {
        let original=b"; ANR\r\npath = auto\r\nmax_generated = 5 ; limit\r\nscene_cut = 1\r\nnits_per_unit = 80\r\nmodel_size = large\r\n";
        assert_eq!(configure(original, &Values::new()).unwrap(), original);
        let changed = configure(
            original,
            &Values::from([("max_generated".into(), "3".into())]),
        )
        .unwrap();
        assert!(String::from_utf8_lossy(&changed).contains("max_generated = 3 ; limit\r\n"));
        assert!(
            String::from_utf8_lossy(&changed).contains("nits_per_unit = 80\r\nmodel_size = large")
        );
        assert_eq!(read(&changed).unwrap()["max_generated"], "3");
        assert!(!changed.contains(&b'['));
        assert!(read(b"path=auto\npath=dp4a\n").is_err());
        assert!(
            configure(
                original,
                &Values::from([("path".into(), "../../evil".into())])
            )
            .is_err()
        );
        assert!(
            configure(
                original,
                &Values::from([("model_size".into(), "small".into())])
            )
            .is_err()
        );
    }
}
