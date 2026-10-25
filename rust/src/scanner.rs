use crate::core;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
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
    /// Explicit plugin folder; the real EXE still supplies identity and process guards.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub deployment_dir: String,
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
impl Game {
    /// Repair the known compiler false positive in saved libraries without a
    /// disk-wide rescan. Old owned paths remain available only for cleanup.
    pub fn repair_renderer(&mut self) {
        if !self.deployment_dir.is_empty() {
            return;
        }
        let root = installation_root(Path::new(&self.exe));
        let dx12 = root.join("bin/x64_dx12/witcher3.exe");
        if is_name(&root, &["The Witcher 3"]) && dx12.is_file() && core::pe64(&dx12, false).is_ok()
        {
            let old = std::iter::once(self.exe.clone())
                .chain(self.targets.clone())
                .collect::<Vec<_>>();
            for p in old {
                if let Some(dir) = Path::new(&p).parent()
                    && core::key(dir) != core::key(dx12.parent().unwrap())
                    && dir.join(core::OWN).is_dir()
                    && !self.cleanup_only.contains(&p)
                {
                    self.cleanup_only.push(p);
                }
            }
            self.exe = dx12.to_string_lossy().into();
            self.targets = vec![self.exe.clone()];
            self.root = root.to_string_lossy().into();
        }
    }
    pub fn deployment_target(&self, exe: &Path) -> core::DeploymentTarget {
        let mut target = if self.deployment_dir.is_empty() {
            core::DeploymentTarget::for_game(exe)
        } else {
            core::DeploymentTarget::custom(exe, Path::new(&self.deployment_dir))
        };
        let mut guards = BTreeMap::new();
        for path in std::iter::once(&self.exe)
            .chain(&self.targets)
            .chain(&self.cleanup_only)
        {
            if core::key(Path::new(path)) != core::key(exe) {
                guards.insert(core::key(Path::new(path)), PathBuf::from(path));
            }
        }
        target.guard_exes = guards.into_values().collect();
        target
    }
    pub fn deployment_directory(&self) -> PathBuf {
        self.deployment_target(Path::new(&self.exe)).directory
    }
    pub fn deployment_executables(&self) -> Vec<String> {
        if !self.deployment_dir.is_empty() || self.targets.is_empty() {
            vec![self.exe.clone()]
        } else {
            self.targets.clone()
        }
    }
    /// One current target per directory, in library order. Cleanup-only paths
    /// remain process guards, never installation destinations.
    pub fn deployment_targets(&self) -> Vec<core::DeploymentTarget> {
        let mut seen = BTreeSet::new();
        self.deployment_executables()
            .iter()
            .map(|exe| self.deployment_target(Path::new(exe)))
            .filter(|target| seen.insert(core::key(&target.directory)))
            .collect()
    }
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

/// Strip only conventional terminal build/platform suffixes. Keep original case
/// so role boundaries such as GameServer remain distinct from a title like Observer.
fn executable_role(name: &str) -> &str {
    let mut role = name
        .rsplit_once('.')
        .filter(|(_, extension)| extension.eq_ignore_ascii_case("exe"))
        .map_or(name, |(stem, _)| stem);
    for suffixes in [
        &["-shipping", "-development", "-debuggame", "-debug", "-test"][..],
        &["-win64", "-win32", "-x64", "-x86"][..],
    ] {
        if let Some(prefix) = suffixes.iter().find_map(|suffix| {
            role.len()
                .checked_sub(suffix.len())
                .and_then(|start| role.get(start..).map(|tail| (start, tail)))
                .filter(|(_, tail)| tail.eq_ignore_ascii_case(suffix))
                .and_then(|(start, _)| role.get(..start))
        }) {
            role = prefix;
        }
    }
    role
}
fn role_suffix(name: &str, suffix: &str) -> bool {
    let Some(start) = name.len().checked_sub(suffix.len()) else {
        return false;
    };
    let Some(tail) = name.get(start..) else {
        return false;
    };
    if !tail.eq_ignore_ascii_case(suffix) {
        return false;
    }
    start == 0
        || name[..start]
            .chars()
            .next_back()
            .is_some_and(|c| !c.is_alphanumeric())
        || (tail.starts_with(|c: char| c.is_ascii_uppercase())
            && name[..start]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit()))
}
/// Exact utility basenames only: NVIDIA/Vulkan words in a game's title are not evidence.
pub fn is_gpu_diagnostic_tool(exe: &Path) -> bool {
    exe.file_name().is_some_and(|name| {
        matches!(
            executable_role(&name.to_string_lossy())
                .to_ascii_lowercase()
                .as_str(),
            "vulkaninfo"
                | "vulkaninfo32"
                | "vulkaninfo64"
                | "nvidia-smi"
                | "nvidia-debugdump"
                | "d3d12stateobjectcompiler"
                | "d3d12warpcompiler"
        )
    })
}
fn auxiliary_exe(name: &str) -> bool {
    if is_gpu_diagnostic_tool(Path::new(name)) {
        return true;
    }
    let role = executable_role(name);
    if [
        "launcher",
        "bootstrap",
        "bootstrapper",
        "crashhandler",
        "dedicatedserver",
        "shadercompiler",
        "workshoptool",
        "workshoputility",
        "updater",
        "downloader",
        "webhelper",
        "subprocess",
        "helper",
        "patcher",
        "crashreporter",
        "uninstaller",
        "installer",
    ]
    .iter()
    .any(|suffix| role_suffix(role, suffix))
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
            "benchmarkreport|reporter|helper|cefsubprocess|epicwebhelper|qtwebengineprocess|",
            "steamcmd|hlmv|hammer|vpk|vvis|vrad|vbsp|srcds(?:_win64)?|",
            "sandbox|shadercachegen|rc|crysbsrenderapp|gameserver|",
            "iipshostapp|fixdownload|",
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

#[cfg(test)]
mod engine_scan_tests {
    use super::*;
    use crate::game_installations::{self as installs, Install};
    use serde_json::json;
    fn exe(path: &Path) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut bytes = vec![0u8; 512];
        bytes[..2].copy_from_slice(b"MZ");
        bytes[60..64].copy_from_slice(&128u32.to_le_bytes());
        bytes[128..132].copy_from_slice(b"PE\0\0");
        bytes[132..134].copy_from_slice(&0x8664u16.to_le_bytes());
        fs::write(path, bytes).unwrap();
    }
    #[test]
    fn epic_declared_entry_does_not_require_engine_or_dlss_and_rejects_helper() {
        let t = tempfile::tempdir().unwrap();
        let game = t.path().join("Unusual client/bin/Render.exe");
        exe(&game);
        let manifest = t
            .path()
            .join("ProgramData/Epic/EpicGamesLauncher/Data/Manifests/game.item");
        let root = game.parent().unwrap().parent().unwrap();
        let mut value = json!({"InstallLocation":root,"DisplayName":"Epic Test",
            "AppName":"title","MainGameAppName":"title","CatalogNamespace":"test","CatalogItemId":"42",
            "LaunchExecutable":"bin/Render.exe"});
        core::atomic_json(&manifest, &value).unwrap();
        let cancel = AtomicBool::new(false);
        let records = installs::epic(&t.path().join("ProgramData"), &cancel);
        let report = scan_with_records(&[root.into()], &cancel, |_, _, _| {}, records).unwrap();
        assert_eq!(report.rows.len(), 1);
        assert_eq!(core::key(Path::new(&report.rows[0].exe)), core::key(&game));
        assert_eq!(report.rows[0].title, "Epic Test");
        let helper = root.join("RenderHelper.exe");
        exe(&helper);
        value["LaunchExecutable"] = json!("RenderHelper.exe");
        core::atomic_json(&manifest, &value).unwrap();
        let records = installs::epic(&t.path().join("ProgramData"), &cancel);
        assert!(
            scan_with_records(&[root.into()], &cancel, |_, _, _| {}, records)
                .unwrap()
                .rows
                .is_empty()
        );
    }
    #[test]
    fn declared_root_bootstrap_does_not_replace_nested_unreal_renderer() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("Epic Game");
        let launcher = root.join("Start.exe");
        let game = root.join("Project/Binaries/Win64/Real-Win64-Shipping.exe");
        exe(&launcher);
        exe(&game);
        fs::create_dir(root.join("Engine")).unwrap();
        let mut record = Install::new(root.clone(), "Epic", "Epic Game");
        record.executable = Some(launcher);
        let report =
            scan_with_records(&[root], &AtomicBool::new(false), |_, _, _| {}, vec![record])
                .unwrap();
        assert_eq!(report.rows.len(), 1);
        assert_eq!(report.rows[0].targets.len(), 1);
        assert_eq!(core::key(Path::new(&report.rows[0].exe)), core::key(&game));
    }
    #[test]
    fn platform_directory_only_admits_matching_game_not_unrelated_programs() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("EA Title");
        let game = root.join("EATitle.exe");
        exe(&game);
        exe(&root.join("Other.exe"));
        exe(&root.join("EATitleUpdater.exe"));
        let records = vec![Install::new(root.clone(), "EA", "EA Title")];
        let report =
            scan_with_records(&[root], &AtomicBool::new(false), |_, _, _| {}, records).unwrap();
        assert_eq!(report.rows.len(), 1);
        assert_eq!(report.rows[0].targets.len(), 1);
        assert_eq!(core::key(Path::new(&report.rows[0].exe)), core::key(&game));
    }
}

fn non_rendering_role(exe: &Path) -> bool {
    let name = exe.file_name().unwrap_or_default().to_string_lossy();
    let role = executable_role(&name);
    if ["server", "editor", "diagnostics", "benchmark", "config"]
        .iter()
        .any(|suffix| role_suffix(role, suffix))
    {
        return true;
    }
    // Lowercase/all-caps concatenated server names have no word boundary. Only
    // exclude them with explicit nearby server-directory evidence.
    role.to_ascii_lowercase().ends_with("server")
        && exe.parent().is_some_and(|dir| {
            dir.ancestors().take(4).any(|p| {
                is_name(
                    p,
                    &["server", "servers", "dedicatedserver", "dedicated_server"],
                )
            })
        })
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
        || (stem == "witcher3" && folder == "thewitcher3")
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
    /// Streamline SR wrapper alone is a clue, not a complete SR runtime.
    pub super_resolution_plugin: bool,
    pub component_paths: Vec<PathBuf>,
    pub anti_cheat: bool,
    pub limited: bool,
}
impl Evidence {
    pub fn neural_input(&self) -> bool {
        self.super_resolution
            || self.component_paths.iter().any(|p| {
                p.file_name()
                    .is_some_and(|n| n.eq_ignore_ascii_case("nvngx_dlssd.dll"))
            })
    }
}
/// Bounded, read-only component discovery, called only in a background worker.
pub(crate) fn patch_runtime_directory(name: &str) -> bool {
    [
        "dlssg_sm86",
        "rtx-encore-runtimes",
        "rtxmfg-xess",
        "optiscaler",
        ".rtx-fg-nr",
        ".rtx-fg-runtimes",
        ".rtx-fg-v3",
        "backup",
        "backups",
    ]
    .contains(&name.to_ascii_lowercase().as_str())
}
pub fn inspect(exe: &Path) -> Evidence {
    use std::{
        cmp::Reverse,
        collections::{BinaryHeap, HashSet},
        os::windows::fs::MetadataExt,
    };
    let mut result = Evidence::default();
    let Ok(root) = core::no_links(&installation_root(exe)) else {
        result.limited = true;
        return result;
    };
    // Component discovery differs from EXE discovery: ThirdParty contains the
    // actual Unreal DLSS DLLs and must never inherit the launcher's exclusion.
    // Prioritize executable/plugin paths, then search other folders within this
    // installation. No title-specific paths and no traversal into other games.
    let mut pending = BinaryHeap::from([Reverse((0u32, 0u32, root.clone()))]);
    if let Some(dir) = exe.parent()
        && let Ok(dir) = core::no_links(dir)
        && dir.starts_with(&root)
    {
        pending.push(Reverse((0, 0, dir)));
    }
    let started = Instant::now();
    let mut visited = HashSet::new();
    let mut entries = 0;
    while let Some(Reverse((cost, depth, dir))) = pending.pop() {
        if !visited.insert(core::key(&dir)) {
            continue;
        }
        if visited.len() > 1024 || started.elapsed().as_millis() > 1000 {
            result.limited = true;
            break;
        }
        let Ok(rows) = fs::read_dir(dir) else {
            result.limited = true;
            continue;
        };
        for row in rows {
            entries += 1;
            if entries > 50000 || started.elapsed().as_millis() > 1000 {
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
                let fg = ["nvngx_dlssg.dll", "sl.dlss_g.dll"].contains(&name.as_str());
                let sr = name == "nvngx_dlss.dll";
                let plugin = name == "sl.dlss.dll";
                result.frame_generation |= fg;
                result.super_resolution |= sr;
                result.super_resolution_plugin |= plugin;
                if (fg || sr || plugin || name == "nvngx_dlssd.dll")
                    && result.component_paths.len() < 32
                {
                    result.component_paths.push(row.path());
                }
            } else if meta.is_dir()
                && (name == "thirdparty" || !SKIP.contains(&name.as_str()))
                && !patch_runtime_directory(&name)
            {
                if depth < 16 {
                    let priority = if [
                        "engine",
                        "plugins",
                        "plugin",
                        "binaries",
                        "bin",
                        "bin64",
                        "x64",
                        "x86_64",
                        "win64",
                        "x64_dx12",
                        "thirdparty",
                        "nvidia",
                        "dlss",
                        "streamline",
                        "streamlinecore",
                        "marketplace",
                        "runtime",
                    ]
                    .contains(&name.as_str())
                        || name.ends_with("_data")
                    {
                        0
                    } else if ["content", "paks", "movies", "videos", "audio", "textures"]
                        .contains(&name.as_str())
                    {
                        32
                    } else {
                        1
                    };
                    pending.push(Reverse((cost + priority, depth + 1, row.path())));
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
            "wingdk",
            "win64_shipping",
        ],
    ) && dir.parent().is_some_and(|p| is_name(p, &["binaries"]))
}
pub fn game_root(exe: &Path) -> PathBuf {
    if let Some(install) = crate::game_installations::wegame_renderer(exe) {
        return install.root;
    }
    let d = exe.parent().unwrap();
    if unreal_binary_dir(d) {
        let project = d.parent().unwrap().parent().unwrap();
        return if project.parent().is_some_and(|p| p.join("Engine").is_dir()) {
            project.parent().unwrap().into()
        } else {
            project.into()
        };
    }
    if is_name(
        d,
        &["x64", "x64_dx12", "win64", "win_x64", "win_x64_release"],
    ) && d.parent().is_some_and(|p| is_name(p, &["bin"]))
    {
        let root = d.parent().unwrap().parent().unwrap();
        return if is_name(root, &["game"])
            && d.join("engine2.dll").is_file()
            && d.join("tier0.dll").is_file()
        {
            root.parent().unwrap_or(root).into()
        } else {
            root.into()
        };
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
    progress: impl FnMut(u64, u64, &Path),
) -> Result<Report> {
    let records = if cancel.load(Ordering::Relaxed) {
        vec![]
    } else {
        crate::game_installations::scan_records(cancel)
    };
    scan_with_records(roots, cancel, progress, records)
}
fn scan_with_records(
    roots: &[PathBuf],
    cancel: &AtomicBool,
    mut progress: impl FnMut(u64, u64, &Path),
    mut records: Vec<crate::game_installations::Install>,
) -> Result<Report> {
    let start = Instant::now();
    let mut report = Report::default();
    let mut seen = HashSet::new();
    let mut evidence = HashSet::new();
    let mut sr_evidence = HashSet::new();
    let mut anti = HashSet::new();
    let mut candidates = Vec::new();
    let mut steam_dirs = HashSet::new();
    let mut engine_entries = HashSet::new();
    let mut platform_entries = HashSet::new();
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
                    subnames.push(lower.clone());
                    if !SKIP.contains(&lower.as_str()) {
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
            if subnames.iter().any(|n| n == "__installer")
                && let Some(record) = crate::game_installations::ea(&dir)
            {
                records.push(record);
            }
            if names.contains(".build.info")
                && let Some(record) = crate::game_installations::build_info(&dir)
            {
                records.push(record);
            }
            for (_, path) in files.iter().filter(|(name, _)| {
                let name = name.to_ascii_lowercase();
                (name.starts_with("goggame-") || name.starts_with("goggame_"))
                    && name.ends_with(".info")
            }) {
                records.extend(crate::game_installations::gog_entries(path));
            }
            for (name, path) in files {
                // Crytek's canonical GameLauncher is the renderer itself, but
                // only when its project/asset/engine layout is verified.
                let cry = name
                    .eq_ignore_ascii_case("GameLauncher.exe")
                    .then(|| crate::game_engines::cryengine(&path))
                    .flatten();
                if !name.to_lowercase().ends_with(".exe")
                    || auxiliary_exe(&name) && cry.is_none()
                    || is_steam_client_binary(&path)
                {
                    continue;
                }
                if let Err(error) = core::pe64(&path, false) {
                    if error.chain().any(|e| {
                        e.downcast_ref::<std::io::Error>()
                            .is_some_and(|io| io.kind() == std::io::ErrorKind::PermissionDenied)
                    }) {
                        report.skip(&path, &crate::xbox::access_hint(&path));
                    }
                    continue;
                }
                let xbox = crate::xbox::identified(&path);
                let wegame = crate::game_installations::wegame_renderer(&path);
                let engine = cry.or_else(|| crate::game_engines::probe(&path, &names));
                let mut row = Game {
                    exe: path.to_string_lossy().into(),
                    root: xbox
                        .as_ref()
                        .map(|g| g.root.clone())
                        .unwrap_or_else(|| installation_root(&path))
                        .to_string_lossy()
                        .into(),
                    title: xbox.as_ref().map(|g| g.title.clone()).unwrap_or_default(),
                    ..Default::default()
                };
                if xbox.is_some() {
                    row.rank += 40;
                    row.reasons.push("Xbox PC 安装信息已确认入口".into());
                }
                if let Some(install) = wegame {
                    row.root = install.root.display().to_string();
                    row.title = install.title;
                    row.rank += 40;
                    row.reasons.push("WeGame 安装信息已确认游戏本体".into());
                }
                if let Some(entry) = engine {
                    row.root = entry.root.display().to_string();
                    if !entry.title.is_empty() {
                        row.title = entry.title;
                    }
                    row.rank += 30;
                    row.reasons
                        .push(format!("{} 游戏结构已确认（兼容性待确认）", entry.engine));
                    engine_entries.insert(core::key(&path));
                }
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
        if let Some((record, _)) = crate::game_installations::scan_match(&row, &records) {
            row.root = record.root.display().to_string();
            if !record.title.is_empty() {
                row.title = record.title.clone();
            }
            row.rank += 40;
            row.reasons
                .push(format!("{} 安装记录已确认游戏入口", record.source));
            platform_entries.insert(core::key(Path::new(&row.exe)));
        }
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
            if !renderer_name(exe, base)
                && crate::game_engines::graphics_import(exe)
                && crate::game_name_windows::products(exe)
                    .iter()
                    .any(|product| {
                        crate::game_engines::normalized(product)
                            == crate::game_engines::normalized(title)
                    })
            {
                platform_entries.insert(core::key(exe));
                row.reasons.push("游戏产品信息与图形入口已确认".into());
            }
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
        if non_rendering_role(exe) {
            continue;
        }
        if renderer_name(exe, base) {
            row.rank += 15;
        }
        if name.contains("dx12")
            || name.contains("d3d12")
            || exe.parent().is_some_and(|dir| is_name(dir, &["x64_dx12"]))
        {
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
        let has_engine = rows
            .iter()
            .any(|g| engine_entries.contains(&core::key(Path::new(&g.exe))));
        let owned = rows.iter().any(|g| {
            Path::new(&g.exe)
                .parent()
                .is_some_and(|p| p.join(core::OWN).is_dir())
        });
        if rows[0].title.is_empty()
            && !has_unreal
            && !has_unity
            && !has_engine
            && !has_named
            && !owned
            && !rows
                .iter()
                .any(|g| platform_entries.contains(&core::key(Path::new(&g.exe))))
        {
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
            let xbox = row
                .reasons
                .iter()
                .any(|r| r == "Xbox PC 安装信息已确认入口");
            let wegame = row
                .reasons
                .iter()
                .any(|r| r == "WeGame 安装信息已确认游戏本体");
            let engine = engine_entries.contains(&core::key(path));
            let platform = platform_entries.contains(&core::key(path));
            // A Steam manifest identifies the installation, not every EXE in
            // it. Root-wide DLSS evidence likewise cannot identify a sidecar
            // utility in another directory as a rendering process.
            if (has_unreal && !unreal && !xbox && !wegame)
                || ((has_engine || has_unity) && !engine && !unity && !unreal && !owned)
                || (!unreal
                    && !unity
                    && !engine
                    && !platform
                    && !renderer_name(path, Path::new(&row.root))
                    && !local_fg
                    && !xbox
                    && !wegame
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
        primary.repair_renderer();
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
