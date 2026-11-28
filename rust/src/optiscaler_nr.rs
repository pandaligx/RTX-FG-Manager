//! The standalone OptiScaler fork. It owns one entry beside the rendering EXE.
use crate::{core, presets::Parameter};
use anyhow::{Result, ensure};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::OnceLock,
};

pub const SCHEME: &str = "optiscaler-nr-presr-mfg";
pub const PROFILE: &str = "optiscaler_nr_ini";
pub const BACKEND: &str = "optiscaler_nr";
pub const CONFIG: &str = "OptiScaler.ini";
pub const SOURCE: &str = "https://github.com/ShyVortex/OptiScaler-NR-PreSR-MFG";
pub const HELP: &str =
    "按 Insert 打开菜单。更改后点击「保存设置」，再重启游戏。帧生成输入和输出默认为关闭。";
pub const ENTRIES: [&str; 8] = [
    "dxgi.dll",
    "winmm.dll",
    "version.dll",
    "dbghelp.dll",
    "d3d12.dll",
    "wininet.dll",
    "winhttp.dll",
    "OptiScaler.asi",
];
pub fn components() -> &'static [&'static str] {
    static LIST: OnceLock<Vec<&'static str>> = OnceLock::new();
    LIST.get_or_init(|| {
        include_str!("../assets/optiscaler-nr-files.txt")
            .lines()
            .collect()
    })
}
pub fn parameters() -> Vec<Parameter> {
    let toggle = || vec![("auto", "自动"), ("true", "开启"), ("false", "关闭")];
    let field = |key, section, ini_key, label, default, choices| Parameter {
        key,
        section,
        ini_key,
        label,
        default,
        choices,
    };
    vec![
        field(
            "nr_enabled",
            "DlssNr",
            "Enabled",
            "DLSS 神经渲染",
            "auto",
            toggle(),
        ),
        field(
            "nr_before_sr",
            "DlssNr",
            "RunBeforeSR",
            "在超分辨率前运行",
            "auto",
            toggle(),
        ),
        field(
            "nr_passes",
            "DlssNr",
            "Passes",
            "模型运行次数",
            "auto",
            vec![("auto", "自动"), ("1", "1"), ("2", "2"), ("3", "3")],
        ),
        field(
            "nr_model",
            "DlssNr",
            "Preset",
            "神经渲染模型",
            "auto",
            vec![
                ("auto", "自动"),
                ("0", "默认"),
                ("1", "原始模型"),
                ("2", "轻量模型（FP8）"),
            ],
        ),
        field(
            "nr_style",
            "DlssNr",
            "Style",
            "画面风格",
            "auto",
            vec![
                ("auto", "自动"),
                ("0", "标准"),
                ("1", "自然"),
                ("2", "电影"),
            ],
        ),
        field(
            "nr_scale",
            "DlssNr",
            "WorkingScale",
            "模型分辨率",
            "auto",
            vec![
                ("auto", "自动"),
                ("0.5", "50%"),
                ("0.75", "75%"),
                ("1.0", "100%"),
            ],
        ),
        field(
            "fg_enabled",
            "FrameGen",
            "Enabled",
            "帧生成",
            "auto",
            toggle(),
        ),
        field(
            "fg_input",
            "FrameGen",
            "FGInput",
            "帧生成输入",
            "auto",
            vec![
                ("auto", "自动"),
                ("nofg", "关闭"),
                ("dlssg", "DLSS-G"),
                ("nvngxfg", "NGX FG"),
                ("fsrfg", "FSR FG"),
                ("upscaler", "超分辨率"),
                ("fsrfg30", "FSR 3.0 FG"),
            ],
        ),
        field(
            "fg_output",
            "FrameGen",
            "FGOutput",
            "帧生成输出",
            "auto",
            vec![
                ("auto", "自动"),
                ("nofg", "关闭"),
                ("dlssg", "DLSS-G"),
                ("xefg", "XeFG"),
                ("fsrfg", "FSR FG"),
            ],
        ),
        field(
            "xe_enabled",
            "XeMFG",
            "UnlockMFG",
            "XeSS 多帧生成",
            "auto",
            toggle(),
        ),
        field(
            "xe_source",
            "XeMFG",
            "Source",
            "XeSS 帧生成输入",
            "Game",
            vec![("Game", "游戏"), ("OptiScaler", "OptiScaler")],
        ),
        field(
            "xe_frames",
            "XeMFG",
            "MaxInterpolatedFrames",
            "XeSS 倍率上限",
            "3",
            vec![
                ("1", "2X"),
                ("2", "3X"),
                ("3", "4X"),
                ("4", "5X"),
                ("5", "6X"),
            ],
        ),
    ]
}

// Known outputs only. An existing host/log is a conflict on first install.
const OUTPUTS: &[&str] = &[
    "OptiScaler.log",
    "nvsmooth30.log",
    "OptiScaler/dlssg_sm86/dlssg_sm86/logs/dlssg_sm86.log",
    "dlssg_sm86/logs/dlssg_sm86.log",
];
pub fn preflight(t: &core::DeploymentTarget, managed: bool) -> Result<()> {
    ensure!(
        t.game_exe.parent() == Some(t.directory.as_path()),
        "请安装到游戏主程序所在目录"
    );
    if !managed {
        for &name in [
            "OptiScaler",
            "dlssg_sm86/logs",
            CONFIG,
            "OptiScaler.dll",
            "OptiScaler.asi",
        ]
        .iter()
        .chain(OUTPUTS)
        {
            ensure!(
                !core::no_links(&t.directory.join(name))?.try_exists()?,
                "游戏已有 {name}，请先还原已有安装"
            );
        }
        for item in fs::read_dir(&t.directory)? {
            let item = item?;
            ensure!(
                !rotated_log(&item.file_name().to_string_lossy()),
                "游戏已有 OptiScaler 历史日志，请先移走"
            );
        }
        for name in ENTRIES {
            let path = core::no_links(&t.directory.join(name))?;
            ensure!(
                !path.is_file()
                    || !crate::win::file_version(&path)
                        .is_ok_and(|(_, n)| n.to_ascii_lowercase().contains("optiscaler")),
                "游戏已有 OptiScaler 入口：{name}"
            );
        }
    }
    Ok(())
}
pub fn owned_sm86_ini(name: &str, path: &Path) -> Result<bool> {
    if name != "OptiScaler/dlssg_sm86/dlssg_sm86.ini"
        || !path.is_file()
        || path.metadata()?.len() > 1024 * 1024
    {
        return Ok(false);
    }
    let bytes = fs::read(path)?;
    let Ok(text) = std::str::from_utf8(&bytes) else {
        return Ok(false);
    };
    let sections = crate::cleanup::parse_ini(text);
    Ok([
        ("FrameGeneration", "Engine"),
        ("FrameGeneration", "MaxGeneratedFrames"),
        ("Logging", "Directory"),
    ]
    .iter()
    .all(|(section, key)| sections.get(*section).is_some_and(|s| s.contains_key(*key))))
}
fn generated_opti_log(path: &Path) -> Result<bool> {
    let mut header = Vec::new();
    fs::File::open(path)?.take(16384).read_to_end(&mut header)?;
    Ok(header.is_empty()
        || String::from_utf8_lossy(&header)
            .lines()
            .any(|l| l.contains("OptiScaler v0.9.97") || l.contains("OptiScaler working as")))
}
fn rotated_log(name: &str) -> bool {
    name.strip_prefix("OptiScaler_")
        .and_then(|s| s.strip_suffix(".log"))
        .is_some_and(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
}
pub fn cleanup(
    dir: &Path,
    targets: &mut BTreeMap<PathBuf, String>,
    dirs: &mut BTreeSet<PathBuf>,
) -> Result<()> {
    for &name in OUTPUTS {
        let p = core::no_links(&dir.join(name))?;
        if p.is_file() && (name != "OptiScaler.log" || generated_opti_log(&p)?) {
            targets.insert(p.clone(), core::digest(&p)?);
        }
    }
    for item in fs::read_dir(dir)? {
        let item = item?;
        let name = item.file_name();
        let name = name.to_string_lossy();
        if rotated_log(&name) {
            let p = core::no_links(&item.path())?;
            if p.is_file() && p.metadata()?.len() > 0 && generated_opti_log(&p)? {
                targets.insert(p.clone(), core::digest(&p)?);
            }
        }
    }
    for folder in ["dlssg_sm86/logs", "OptiScaler/dlssg_sm86/dlssg_sm86/logs"] {
        let root = core::no_links(&dir.join(folder))?;
        if root.is_dir() {
            for item in fs::read_dir(root)? {
                let item = item?;
                if crate::cleanup::is_log(&item.file_name().to_string_lossy()) {
                    let path = core::no_links(&item.path())?;
                    if path.is_file() {
                        targets.insert(path.clone(), core::digest(&path)?);
                    }
                }
            }
        }
    }
    for &name in components().iter().chain(OUTPUTS) {
        let mut p = dir.join(name);
        while p.pop() && p != dir && p.starts_with(dir) {
            dirs.insert(p.clone());
        }
    }
    Ok(())
}
