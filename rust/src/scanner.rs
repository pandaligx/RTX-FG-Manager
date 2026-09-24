use crate::core;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    sync::{
        OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};
const SKIP: &[&str] = &[
    "windows",
    "$recycle.bin",
    "system volume information",
    "recovery",
    "perflogs",
    "windowsapps",
    "node_modules",
    ".git",
    "__pycache__",
    core::OWN,
    "dlssg_sm75_cache",
    "dlssg_sm75_cache_r2",
    "dlssg_sm75_logs",
    "dlssg_sm75_logs_r2",
];
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Game {
    pub exe: String,
    #[serde(default)]
    pub root: String,
    #[serde(default)]
    pub rank: i32,
    #[serde(default)]
    pub reasons: Vec<String>,
    #[serde(default)]
    pub anti: bool,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}
#[derive(Debug, Serialize, Default)]
pub struct Report {
    pub directories: u64,
    pub skipped: u64,
    pub candidates: usize,
    pub cancelled: bool,
    pub seconds: f64,
    pub rows: Vec<Game>,
    pub skipped_details: Vec<SkipDetail>,
}
#[derive(Clone, Debug, Serialize)]
pub struct SkipDetail {
    pub path: String,
    pub reason: String,
}
pub const MAX_SKIP_DETAILS: usize = 32;
impl Report {
    fn skip(&mut self, path: &Path, reason: &str) {
        self.skipped += 1;
        if self.skipped_details.len() < MAX_SKIP_DETAILS {
            self.skipped_details.push(SkipDetail {
                path: path.to_string_lossy().chars().take(512).collect(),
                reason: reason.into(),
            });
        }
    }
}

fn auxiliary_exe(name: &str) -> bool {
    static TOOL: OnceLock<regex::Regex> = OnceLock::new();
    // Match known tool basenames, not game-name fragments such as "Crash".
    TOOL.get_or_init(|| {
        regex::Regex::new(concat!(
            "(?i)^(?:crashreportclient(?:editor)?(?:-(?:win64|win32)-(?:shipping|development|debug))?|",
            "crashpad_handler|crashsender[0-9]*|unitycrashhandler(?:32|64)?|",
            "unins[0-9]*|uninstall|uninstaller|",
            "setup(?:32|64)?|install|installer|launcher|updater|",
            "benchmarkreport|reporter|helper|cefsubprocess|epicwebhelper|",
            "easyanticheat(?:_eos)?(?:_setup)?|beservice(?:_x64)?|",
            "battleye|ue(?:4)?prereqsetup_x(?:64|86)|",
            "(?:unrealeditor|ue4editor|unrealpak|shadercompileworker|",
            "unrealversionselector|unrealfrontend|unrealinsights|",
            "unrealtraceserver|unreallightmass|swarmagent|",
            "swarmcoordinator|bootstrap)(?:-cmd|-win64-(?:shipping|development))?)\\.exe$"
        ))
        .expect("constant scanner tool pattern")
    })
    .is_match(name)
}

#[derive(Clone, Debug, Default)]
pub struct Evidence {
    pub frame_generation: bool,
    pub super_resolution: bool,
    pub anti_cheat: bool,
    pub limited: bool,
}
/// Bounded, read-only component discovery, called only in a background worker.
pub fn inspect(exe: &Path) -> Evidence {
    use std::os::windows::fs::MetadataExt;
    let mut result = Evidence::default();
    let Ok(root) = core::no_links(&game_root(exe)) else {
        result.limited = true;
        return result;
    };
    let started = Instant::now();
    let mut pending = std::collections::VecDeque::from([(root, 0)]);
    let mut entries = 0;
    let mut directories = 0;
    while let Some((dir, depth)) = pending.pop_front() {
        directories += 1;
        if directories > 256 || started.elapsed().as_millis() > 250 {
            result.limited = true;
            break;
        }
        let Ok(rows) = fs::read_dir(dir) else {
            result.limited = true;
            continue;
        };
        for row in rows {
            entries += 1;
            if entries > 16000 || started.elapsed().as_millis() > 250 {
                result.limited = true;
                return result;
            }
            let Ok(row) = row else {
                result.limited = true;
                continue;
            };
            let name = row.file_name().to_string_lossy().to_lowercase();
            let Ok(meta) = row.metadata() else {
                result.limited = true;
                continue;
            };
            if meta.file_attributes() & 0x400 != 0 {
                continue;
            }
            result.anti_cheat |= name.contains("easyanticheat")
                || name.contains("battleye")
                || name == "ace-base.sys"
                || name == "acesafe.sys"
                || name == "anticheatexpert";
            if meta.is_file() {
                result.frame_generation |=
                    ["nvngx_dlssg.dll", "sl.dlss_g.dll"].contains(&name.as_str());
                result.super_resolution |= name == "nvngx_dlss.dll";
            } else if meta.is_dir() && !SKIP.contains(&name.as_str()) {
                if depth < 6 {
                    pending.push_back((row.path(), depth + 1));
                } else {
                    result.limited = true;
                }
            }
        }
    }
    result
}
fn is_name(p: &Path, names: &[&str]) -> bool {
    p.file_name()
        .is_some_and(|n| names.iter().any(|s| n.eq_ignore_ascii_case(s)))
}
fn unreal_binary_dir(dir: &Path) -> bool {
    is_name(
        dir,
        &[
            "win64",
            "win64r",
            "win64h",
            "win64rh",
            "win64hr",
            "win64_shipping",
        ],
    ) && dir.parent().is_some_and(|p| is_name(p, &["binaries"]))
}
pub fn game_root(exe: &Path) -> PathBuf {
    let d = exe.parent().unwrap();
    if unreal_binary_dir(d) {
        let project = d.parent().unwrap().parent().unwrap();
        return if project.parent().is_some_and(|p| p.join("Engine").is_dir()) {
            project.parent().unwrap().into()
        } else {
            project.into()
        };
    }
    if is_name(d, &["x64"]) && d.parent().is_some_and(|p| is_name(p, &["bin"])) {
        return d.parent().unwrap().parent().unwrap().into();
    }
    if is_name(d, &["bin", "bin64", "x64", "win64"]) {
        return d.parent().unwrap_or(d).into();
    }
    d.into()
}
fn ancestors_into(p: &Path, set: &mut HashSet<String>) {
    for parent in p.ancestors() {
        set.insert(core::key(parent));
    }
}
pub fn scan(
    roots: &[PathBuf],
    cancel: &AtomicBool,
    mut progress: impl FnMut(u64, u64, &Path),
) -> Result<Report> {
    let start = Instant::now();
    let mut report = Report::default();
    let mut seen = HashSet::new();
    let mut evidence = HashSet::new();
    let mut sr_evidence = HashSet::new();
    let mut anti = HashSet::new();
    let mut candidates = Vec::new();
    for root in roots {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let root = match core::no_links(root) {
            Ok(p) if p.is_dir() => p,
            _ => {
                report.skip(root, "目录不可访问或属于目录联接");
                continue;
            }
        };
        let mut stack = vec![root];
        while let Some(dir) = stack.pop() {
            if cancel.load(Ordering::Relaxed) {
                break;
            }
            if !seen.insert(core::key(&dir)) {
                continue;
            }
            report.directories += 1;
            // Only root ancestors are checked repeatedly. Every discovered entry is
            // checked for reparse points before traversal. No link is followed.
            let entries = match fs::read_dir(&dir) {
                Ok(e) => e,
                Err(_) => {
                    report.skip(&dir, "无法读取目录");
                    continue;
                }
            };
            let mut files = Vec::new();
            let mut names = HashSet::new();
            let mut subnames = Vec::new();
            for entry in entries {
                if cancel.load(Ordering::Relaxed) {
                    break;
                }
                let Ok(entry) = entry else {
                    report.skip(&dir, "无法读取目录条目");
                    continue;
                };
                let name = entry.file_name().to_string_lossy().to_string();
                let lower = name.to_lowercase();
                let meta = match entry.metadata() {
                    Ok(m) => m,
                    Err(_) => {
                        report.skip(&entry.path(), "无法读取文件信息");
                        continue;
                    }
                };
                if core::is_link(&meta) {
                    report.skip(&entry.path(), "已跳过符号链接或目录联接");
                    continue;
                }
                if meta.is_dir() {
                    if !SKIP.contains(&lower.as_str()) {
                        subnames.push(lower);
                        stack.push(entry.path());
                    } else {
                        report.skip(&entry.path(), "已跳过系统或缓存目录");
                    }
                } else {
                    names.insert(lower);
                    files.push((name, entry.path()));
                }
            }
            if names.contains("nvngx_dlssg.dll") || names.contains("sl.dlss_g.dll") {
                ancestors_into(&dir, &mut evidence);
            }
            if names.contains("nvngx_dlss.dll") || names.contains("sl.dlss.dll") {
                ancestors_into(&dir, &mut sr_evidence);
            }
            if names
                .iter()
                .chain(subnames.iter())
                .any(|n| n.contains("easyanticheat") || n.contains("battleye"))
            {
                ancestors_into(&dir, &mut anti);
            }
            if report.directories % 100 == 0 {
                progress(report.directories, report.skipped, &dir);
            }
            for (name, path) in files {
                if !name.to_lowercase().ends_with(".exe")
                    || auxiliary_exe(&name)
                    || core::pe64(&path, false).is_err()
                {
                    continue;
                }
                let mut row = Game {
                    exe: path.to_string_lossy().into(),
                    root: game_root(&path).to_string_lossy().into(),
                    ..Default::default()
                };
                if names.contains("nvngx_dlssg.dll") || names.contains("sl.dlss_g.dll") {
                    row.rank += 80;
                    row.reasons.push("同目录有 DLSS 帧生成组件".into());
                }
                if path
                    .file_stem()
                    .is_some_and(|s| s.to_string_lossy().to_lowercase().contains("shipping"))
                {
                    row.rank += 25;
                    row.reasons.push("Unreal Shipping 本体".into());
                } else if unreal_binary_dir(&dir) {
                    row.rank += 20;
                    row.reasons
                        .push("Unreal 游戏目录候选（兼容性待确认）".into());
                }
                if names.contains("unityplayer.dll")
                    && path.file_stem().is_some_and(|s| {
                        subnames.contains(&format!("{}_data", s.to_string_lossy().to_lowercase()))
                    })
                {
                    row.rank += 20;
                    row.reasons
                        .push("Unity 游戏本体候选（兼容性待确认）".into());
                }
                if unreal_binary_dir(&dir) || is_name(&dir, &["win64", "x64", "bin64"]) {
                    row.rank += 5;
                }
                candidates.push(row);
            }
        }
    }
    let mut selected = Vec::new();
    for mut row in candidates {
        let base = Path::new(&row.root);
        let exe = Path::new(&row.exe);
        if base.parent().is_some()
            && evidence.contains(&core::key(base))
            && !row.reasons.iter().any(|r| r.contains("DLSS"))
        {
            row.rank += 60;
            row.reasons.push("游戏目录内有 DLSS 帧生成组件".into());
        }
        if base.parent().is_some()
            && sr_evidence.contains(&core::key(base))
            && !row.reasons.iter().any(|r| r.contains("DLSS"))
        {
            row.rank += 30;
            row.reasons
                .push("发现 DLSS 超分组件（不代表支持帧生成）".into());
        }
        if row.reasons.is_empty() && !exe.parent().unwrap().join(core::OWN).exists() {
            continue;
        }
        row.anti = anti.contains(&core::key(base));
        if row.anti {
            row.reasons.push("发现反作弊组件，兼容性需确认".into());
        }
        // A game may have distinct DX11/DX12 or campaign/multiplayer binaries
        // beside one another. Ranking orders candidates; it must not hide them.
        selected.push(row);
    }
    report.rows = selected;
    report.rows.sort_by(|a, b| {
        b.rank
            .cmp(&a.rank)
            .then_with(|| a.exe.to_lowercase().cmp(&b.exe.to_lowercase()))
    });
    report.candidates = report.rows.len();
    report.cancelled = cancel.load(Ordering::Relaxed);
    report.seconds = start.elapsed().as_secs_f64();
    Ok(report)
}
