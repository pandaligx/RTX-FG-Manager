use crate::core;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    io::Read,
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
    "_commonredist",
    "redist",
    "redistributables",
    "prerequisites",
    "thirdparty",
    "tools",
    "tool",
    "support",
    "supportfiles",
    "support_files",
    "__installer",
    "editors",
    "sdk",
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
    pub title: String,
    #[serde(default)]
    pub root: String,
    /// Verified rendering EXEs, one representative per deployment directory.
    #[serde(default)]
    pub targets: Vec<String>,
    /// Old deployments outside the current scan remain removable, never install targets.
    #[serde(default)]
    pub cleanup_only: Vec<String>,
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
    let stem = name
        .strip_suffix(".exe")
        .or_else(|| name.strip_suffix(".EXE"))
        .unwrap_or(name)
        .to_ascii_lowercase();
    let role = [
        "-win64-shipping",
        "-win64-development",
        "-win32-shipping",
        "-shipping",
    ]
    .iter()
    .find_map(|suffix| stem.strip_suffix(suffix))
    .unwrap_or(&stem);
    if [
        "launcher",
        "bootstrap",
        "bootstrapper",
        "crashhandler",
        "dedicatedserver",
        "shadercompiler",
        "workshoptool",
        "workshoputility",
    ]
    .iter()
    .any(|suffix| role.ends_with(suffix))
    {
        return true;
    }
    static TOOL: OnceLock<regex::Regex> = OnceLock::new();
    // Match known tool basenames, not game-name fragments such as "Crash".
    let tool = TOOL.get_or_init(|| {
        regex::Regex::new(concat!(
            "(?i)^(?:crashreportclient(?:editor)?(?:-(?:win64|win32)-(?:shipping|development|debug))?|",
            "crashpad_handler|crashsender[0-9]*|unitycrashhandler(?:32|64)?|",
            "unins[0-9]*|uninstall|uninstaller|",
            "setup(?:32|64)?|install|installer|launcher|updater|",
            "benchmarkreport|reporter|helper|cefsubprocess|epicwebhelper|",
            "easyanticheat(?:_eos)?(?:_setup)?|beservice(?:_x64)?|",
            "start_protected_game|startprotectedgame|eaanticheat(?:_game_service_launcher)?|",
            "steamservice|steamerrorreporter|steamwebhelper|",
            "battleye|ue(?:4)?prereqsetup_x(?:64|86)|",
            "(?:unrealeditor|ue4editor|unrealpak|shadercompileworker|",
            "unrealversionselector|unrealfrontend|unrealinsights|",
            "unrealtraceserver|unreallightmass|swarmagent|",
            "swarmcoordinator|bootstrap)(?:-cmd|-win64-(?:shipping|development))?)\\.exe$"
        ))
        .expect("constant scanner tool pattern")
    });
    // Protected launchers also appear with Unreal's Shipping suffix. Match
    // both their original filename and the normalized role above.
    tool.is_match(name) || tool.is_match(&format!("{role}.exe"))
}

fn steam_root(path: &Path) -> Option<PathBuf> {
    for ancestor in path.ancestors() {
        let parent = ancestor.parent()?;
        if is_name(parent, &["common"])
            && parent.parent().is_some_and(|p| is_name(p, &["steamapps"]))
        {
            return Some(ancestor.to_path_buf());
        }
    }
    None
}

fn steam_apps(path: &Path) -> Option<PathBuf> {
    path.ancestors()
        .find(|p| is_name(p, &["steamapps"]))
        .map(Path::to_path_buf)
}

/// Steam itself and its helpers are never game deployment targets.
pub fn is_steam_client_binary(path: &Path) -> bool {
    if steam_root(path).is_some() {
        return false;
    }
    path.ancestors()
        .any(|p| is_name(p, &["steam"]) && p.join("steamapps").is_dir())
}

fn steam_manifests(dirs: &HashSet<PathBuf>) -> HashMap<String, String> {
    static FIELD: OnceLock<regex::Regex> = OnceLock::new();
    let field = FIELD
        .get_or_init(|| regex::Regex::new(r#"(?m)^\s*"([A-Za-z]+)"\s*"([^"\r\n]*)""#).unwrap());
    let mut installs = HashMap::new();
    for dir in dirs {
        let Ok(entries) = fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.take(4096).flatten() {
            let name = entry.file_name().to_string_lossy().to_lowercase();
            if !name.starts_with("appmanifest_") || !name.ends_with(".acf") {
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            if !meta.is_file() || core::is_link(&meta) || meta.len() > 1024 * 1024 {
                continue;
            }
            let Ok(file) = fs::File::open(entry.path()) else {
                continue;
            };
            let mut bytes = Vec::new();
            if file.take(1024 * 1024 + 1).read_to_end(&mut bytes).is_err()
                || bytes.len() > 1024 * 1024
            {
                continue;
            }
            let text = String::from_utf8_lossy(&bytes);
            let mut title = None;
            let mut folder = None;
            for captures in field.captures_iter(&text) {
                if captures[1].eq_ignore_ascii_case("name") {
                    title = Some(captures[2].to_string());
                }
                if captures[1].eq_ignore_ascii_case("installdir") {
                    folder = Some(captures[2].to_string());
                }
            }
            let (Some(title), Some(folder)) = (title, folder) else {
                continue;
            };
            if title.is_empty()
                || title.chars().count() > 160
                || title.chars().any(char::is_control)
                || folder.is_empty()
                || folder.chars().count() > 255
                || folder == "."
                || folder == ".."
                || folder.contains(['/', '\\', ':'])
                || folder.chars().any(char::is_control)
            {
                continue;
            }
            let root = dir.join("common").join(folder);
            if root.is_dir() && core::no_links(&root).is_ok() {
                installs.insert(core::key(&root), title);
            }
        }
    }
    installs
}

fn renderer_name(path: &Path, root: &Path) -> bool {
    let stem = path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase();
    let folder = root
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase();
    let compact = |s: &str| {
        s.chars()
            .filter(|c| c.is_alphanumeric())
            .collect::<String>()
    };
    let stem = compact(&stem);
    let folder = compact(&folder);
    stem == folder
        || (!folder.is_empty() && stem.starts_with(&folder))
        || (stem.len() >= 4 && folder.starts_with(&stem))
        || stem.contains("dx12")
        || stem.contains("d3d12")
        || stem.contains("vulkan")
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
/// One Steam manifest or a shared Engine folder can contain separate Unreal
/// projects. Only collapse their binaries when there is a single project.
fn shared_unreal_projects(project: &Path) -> bool {
    let Some(parent) = project.parent() else {
        return false;
    };
    let Ok(entries) = fs::read_dir(parent) else {
        return true;
    };
    let mut checked = 0;
    for entry in entries {
        checked += 1;
        if checked > 1024 {
            return true;
        }
        let Ok(entry) = entry else {
            return true;
        };
        let sibling = entry.path();
        if core::key(&sibling) == core::key(project)
            || (is_name(&sibling, &["Engine"]) && !is_name(project, &["Engine"]))
        {
            continue;
        }
        let Ok(meta) = fs::symlink_metadata(&sibling) else {
            return true;
        };
        if core::is_link(&meta) || !meta.is_dir() {
            continue;
        }
        let binary = sibling.join("Binaries");
        if [
            "Win64",
            "Win64r",
            "Win64h",
            "Win64rh",
            "Win64hr",
            "Win64_shipping",
        ]
        .iter()
        .any(|name| binary.join(name).is_dir())
        {
            return true;
        }
    }
    false
}
pub fn installation_root(exe: &Path) -> PathBuf {
    if let Some(dir) = exe.parent()
        && unreal_binary_dir(dir)
        && let Some(project) = dir.parent().and_then(Path::parent)
        && shared_unreal_projects(project)
    {
        return project.to_path_buf();
    }
    steam_root(exe).unwrap_or_else(|| game_root(exe))
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
    let mut steam_dirs = HashSet::new();
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
        if let Some(dir) = steam_apps(&root) {
            steam_dirs.insert(dir);
        }
        let mut stack = vec![root];
        while let Some(dir) = stack.pop() {
            if cancel.load(Ordering::Relaxed) {
                break;
            }
            if !seen.insert(core::key(&dir)) {
                continue;
            }
            if is_name(&dir, &["steamapps"]) {
                steam_dirs.insert(dir.clone());
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
                    || is_steam_client_binary(&path)
                    || core::pe64(&path, false).is_err()
                {
                    continue;
                }
                let mut row = Game {
                    exe: path.to_string_lossy().into(),
                    root: installation_root(&path).to_string_lossy().into(),
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
    let manifests = steam_manifests(&steam_dirs);
    let mut groups: BTreeMap<String, Vec<Game>> = BTreeMap::new();
    for mut row in candidates {
        let base = Path::new(&row.root);
        let exe = Path::new(&row.exe);
        if let Some((steam, title)) = steam_root(exe).and_then(|steam| {
            manifests
                .get(&core::key(&steam))
                .map(|title| (steam, title))
        }) {
            row.title = if core::key(base) == core::key(&steam) {
                title.clone()
            } else {
                format!(
                    "{} · {}",
                    title,
                    base.file_name().unwrap_or_default().to_string_lossy()
                )
            };
            row.rank += 35;
            row.reasons.push("Steam 已安装游戏".into());
        }
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
        let name = exe
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_lowercase();
        if name.ends_with("server")
            || name.ends_with("editor")
            || name.ends_with("diagnostics")
            || name.ends_with("benchmark")
            || name.ends_with("config")
        {
            continue;
        }
        if renderer_name(exe, base) {
            row.rank += 15;
        }
        if name.contains("dx12") || name.contains("d3d12") {
            row.rank += 10;
        }
        row.anti = anti.contains(&core::key(base));
        if row.anti {
            row.reasons.push("发现反作弊组件，兼容性需确认".into());
        }
        groups.entry(core::key(base)).or_default().push(row);
    }
    for mut rows in groups.into_values() {
        rows.sort_by(|a, b| {
            b.rank
                .cmp(&a.rank)
                .then_with(|| core::key(Path::new(&a.exe)).cmp(&core::key(Path::new(&b.exe))))
        });
        let has_unreal = rows.iter().any(|g| {
            let p = Path::new(&g.exe);
            unreal_binary_dir(p.parent().unwrap())
                && p.file_stem()
                    .is_some_and(|s| s.to_string_lossy().to_lowercase().contains("shipping"))
        });
        let has_named = rows
            .iter()
            .any(|g| renderer_name(Path::new(&g.exe), Path::new(&g.root)));
        let has_unity = rows
            .iter()
            .any(|g| g.reasons.iter().any(|r| r.contains("Unity 游戏本体")));
        let owned = rows.iter().any(|g| {
            Path::new(&g.exe)
                .parent()
                .is_some_and(|p| p.join(core::OWN).is_dir())
        });
        if rows[0].title.is_empty() && !has_unreal && !has_unity && !has_named && !owned {
            report.skip(
                Path::new(&rows[0].root),
                "缺少可确认的游戏身份线索，仍可手动添加",
            );
            continue;
        }
        let mut targets = Vec::new();
        let mut directories = HashSet::new();
        for row in &rows {
            let path = Path::new(&row.exe);
            let unreal = unreal_binary_dir(path.parent().unwrap())
                && path
                    .file_stem()
                    .is_some_and(|s| s.to_string_lossy().to_lowercase().contains("shipping"));
            let unity = row.reasons.iter().any(|r| r.contains("Unity 游戏本体"));
            let local_fg = row.reasons.iter().any(|r| r == "同目录有 DLSS 帧生成组件");
            let owned = path.parent().unwrap().join(core::OWN).is_dir();
            // A Steam manifest identifies the installation, not every EXE in
            // it. Root-wide DLSS evidence likewise cannot identify a sidecar
            // utility in another directory as a rendering process.
            if (has_unreal && !unreal)
                || (!unreal
                    && !unity
                    && !renderer_name(path, Path::new(&row.root))
                    && !local_fg
                    && !owned)
            {
                continue;
            }
            if directories.insert(core::key(path.parent().unwrap())) {
                targets.push(row.exe.clone());
            }
        }
        if targets.is_empty() {
            continue;
        }
        let mut primary = rows
            .into_iter()
            .find(|row| row.exe == targets[0])
            .expect("first deployment target belongs to scan group");
        // The first surviving renderer is the display icon and parameter target.
        primary.targets = targets;
        report.rows.push(primary);
    }
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
