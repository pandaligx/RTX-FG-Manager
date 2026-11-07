//! Opt-in runtime updates. Existing game files are backed up; later edits win.
//! Added FG is a separate matched set for Encore 2 games with no native FG.
//! Native Streamline plugins update together; the game's interposer stays intact.
use crate::{assets, core, scanner, transfer, win};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

const OWN: &str = ".rtx-fg-runtimes";
const LIMIT: u64 = 512 * 1024 * 1024;
pub const KINDS: [&str; 5] = ["sr", "rr", "fg", "xemfg", "encorefg"];
pub const ENCORE_FG_FILES: [&str; 7] = [
    "nvngx_dlssg.dll",
    "sl.interposer.dll",
    "sl.common.dll",
    "sl.dlss_g.dll",
    "sl.reflex.dll",
    "sl.pcl.dll",
    "RTXFG-Streamline-LICENSE.txt",
];
const STREAMLINE_PLUGINS: [&str; 10] = [
    "sl.common.dll",
    "sl.deepdvc.dll",
    "sl.directsr.dll",
    "sl.dlss.dll",
    "sl.dlss_d.dll",
    "sl.dlss_g.dll",
    "sl.nis.dll",
    "sl.nvperf.dll",
    "sl.pcl.dll",
    "sl.reflex.dll",
];
const STREAMLINE_LICENSE: &str = "RTXFG-Streamline-LICENSE.txt";

/// Update the native plugin locations, never add an interposer beside a different
/// executable. This is independent from Encore's added-FG integration.
fn native_streamline_paths(target: &core::DeploymentTarget) -> Result<Vec<PathBuf>> {
    let evidence = scanner::inspect_for_installation(&target.game_exe);
    native_streamline_paths_from(target, &evidence, |path| {
        core::pe64(path, true)?;
        Ok(win::file_version(path)?.0)
    })
}
fn native_streamline_paths_from(
    target: &core::DeploymentTarget,
    evidence: &scanner::Evidence,
    mut version: impl FnMut(&Path) -> Result<String>,
) -> Result<Vec<PathBuf>> {
    ensure!(
        !evidence.limited,
        "组件检测未完成，请刷新后重试；不会向未知位置添加运行库"
    );
    let mut folders = BTreeSet::new();
    let prior = read(target, "fg")?;
    let mut tracked = BTreeSet::new();
    if let Some(record) = &prior {
        for entry in &record.entries {
            let path = destination(target, "fg", entry)?;
            if path.file_name().is_some_and(|name| {
                STREAMLINE_PLUGINS
                    .iter()
                    .any(|n| name.eq_ignore_ascii_case(n))
            }) {
                folders.insert(path.parent().context("Streamline 目录无效")?.to_path_buf());
                tracked.insert(core::key(&path));
            }
        }
    }
    for path in &evidence.frame_integration_paths {
        if path.file_name().is_some_and(|n| {
            n.eq_ignore_ascii_case("sl.interposer.dll")
                || n.eq_ignore_ascii_case("sl.common.dll")
                || n.eq_ignore_ascii_case("sl.dlss_g.dll")
        }) {
            folders.insert(path.parent().context("Streamline 目录无效")?.to_path_buf());
        }
    }
    let mut paths = Vec::new();
    for folder in folders {
        // An installed, added-FG set has its own matched-package ownership.
        if let Some(record) = read(target, "encorefg")?
            && record.entries.iter().any(|e| {
                Path::new(&e.path)
                    .parent()
                    .is_some_and(|p| core::key(p) == core::key(&folder))
            })
        {
            continue;
        }
        for entry in fs::read_dir(&folder)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().to_lowercase();
            ensure!(
                !name.starts_with("sl.")
                    || !name.ends_with(".dll")
                    || name == "sl.interposer.dll"
                    || STREAMLINE_PLUGINS.contains(&name.as_str()),
                "发现不在匹配组件包中的 Streamline 插件，保留现有文件：{}",
                entry.path().display()
            );
        }
        let has_fg = folder.join("sl.dlss_g.dll").is_file();
        for name in STREAMLINE_PLUGINS {
            let path = core::no_links(&folder.join(name))?;
            if path.is_file()
                || tracked.contains(&core::key(&path))
                || has_fg && ["sl.common.dll", "sl.pcl.dll", "sl.reflex.dll"].contains(&name)
            {
                if path.is_file() {
                    let v = version(&path)?;
                    ensure!(
                        version_numbers(&v) <= version_numbers("2.14.16"),
                        "游戏已有更新的 Streamline 插件，保留现有文件：{}",
                        path.display()
                    );
                }
                ensure!(
                    !protected(target, &scanner::installation_root(&target.game_exe), &path)?,
                    "Streamline 文件属于其他补丁，保留现有文件：{}",
                    path.display()
                );
                paths.push(path);
            }
        }
        paths.push(core::no_links(&folder.join(STREAMLINE_LICENSE))?);
    }
    Ok(paths)
}

fn version_numbers(version: &str) -> Vec<u32> {
    let mut parts = version
        .split('.')
        .map(|v| v.parse().unwrap_or(0))
        .collect::<Vec<_>>();
    while parts.last() == Some(&0) {
        parts.pop();
    }
    parts
}
pub fn label(kind: &str) -> &'static str {
    match kind {
        "sr" => "DLSS 超分辨率",
        "rr" => "DLSS 光线重建",
        "fg" => "DLSS 帧生成",
        "xemfg" => "XeSS 多帧生成",
        "encorefg" => "新增 DLSS 帧生成（无原生帧生成）",
        _ => "运行库",
    }
}
fn filename(kind: &str) -> Result<&'static str> {
    Ok(match kind {
        "sr" => "nvngx_dlss.dll",
        "rr" => "nvngx_dlssd.dll",
        "fg" => "nvngx_dlssg.dll",
        "xemfg" => "libxess_fg.dll",
        "encorefg" => "sl.interposer.dll",
        _ => anyhow::bail!("未知运行库"),
    })
}
#[derive(Clone, Deserialize, Serialize, PartialEq)]
pub(crate) struct Package {
    pub kind: String,
    pub version: String,
    pub bytes: u64,
    pub sha256: String,
    pub urls: Vec<String>,
    pub files: Vec<Asset>,
}
#[derive(Clone, Deserialize, Serialize, PartialEq)]
pub(crate) struct Asset {
    pub name: String,
    pub bytes: u64,
    pub sha256: String,
}
fn package(kind: &str) -> Result<Package> {
    if let Some(p) = crate::runtime_catalog::selected(kind) {
        return Ok(p);
    }
    bundled_package(kind)
}
pub(crate) fn bundled_package(kind: &str) -> Result<Package> {
    serde_json::from_str::<Vec<Package>>(include_str!("../assets/runtime-files.json"))?
        .into_iter()
        .find(|p| p.kind == kind)
        .context("未提供此运行库")
}
pub fn recommended(kind: &str) -> String {
    package(kind).map(|p| p.version).unwrap_or_default()
}
fn version_package(kind: &str, version: &str) -> Result<Package> {
    let current = package(kind)?;
    let bundled = bundled_package(kind)?;
    std::iter::once(current)
        .chain(std::iter::once(bundled))
        .chain(crate::runtime_catalog::history(kind))
        .find(|p| p.version == version)
        .context("此运行库版本已不可用，请重新选择")
}
pub fn versions(kind: &str) -> Vec<String> {
    let mut versions: Vec<_> = package(kind)
        .into_iter()
        .chain(bundled_package(kind))
        .chain(crate::runtime_catalog::history(kind))
        .map(|p| p.version)
        .collect();
    versions.sort_by_key(|v| std::cmp::Reverse(semver::Version::parse(v).ok()));
    versions.dedup();
    versions
}

/// Complete Encore's manual-install dependencies for each real deployment target.
/// Native runtime locations are updated in place; only games without an existing
/// frame-generation integration receive the added-FG set. No feature is enabled.
pub fn complete_encore_runtimes(
    targets: &[core::DeploymentTarget],
    github: bool,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(transfer::Progress),
) -> Result<()> {
    ensure!(!targets.is_empty(), "未选择游戏");
    let mut inventories = Vec::with_capacity(targets.len());
    for target in targets {
        ensure!(encore_200_installed(target)?, "请先安装 RTX Encore 2.0+");
        core::assert_target_stopped(target)?;
        let evidence = scanner::inspect_for_installation(&target.game_exe);
        ensure!(
            !evidence.limited,
            "组件检测未完成，请刷新后重试；不会向未知位置添加运行库"
        );
        inventories.push(evidence);
    }
    let mut planned = Vec::new();
    for kind in ["sr", "rr", "fg"] {
        let mut selected = Vec::new();
        for (target, evidence) in targets.iter().zip(&inventories) {
            let destinations = candidate_paths(target, kind, Some(evidence))?;
            if !destinations.is_empty() {
                let current = destinations
                    .iter()
                    .filter(|p| p.is_file())
                    .map(|p| win::file_version(p).map(|v| v.0))
                    .collect::<Result<Vec<_>>>()?;
                // A game update may have provided a newer runtime. Automatic
                // completion never downgrades it or pairs it with older plugins.
                if current
                    .iter()
                    .any(|v| version_numbers(v) > version_numbers("310.9.16"))
                {
                    continue;
                }
                if kind == "fg" {
                    // Validate every native provider before writing any other
                    // runtime. A known layout conflict must not partially update
                    // SR/RR before it is reported to the player.
                    native_streamline_paths_from(target, evidence, |path| {
                        core::pe64(path, true)?;
                        Ok(win::file_version(path)?.0)
                    })?;
                }
                selected.push(target.clone());
            }
        }
        if !selected.is_empty() {
            planned.push((kind, selected));
        }
    }
    let mut added = Vec::new();
    for (target, evidence) in targets.iter().zip(&inventories) {
        // Native integrations intentionally do not need an additional FG set.
        // A broken or externally changed managed set, however, is an error.
        if evidence.frame_integration_paths.is_empty() || read(target, "encorefg")?.is_some() {
            preflight_added_fg(target, Some(evidence))?;
            added.push(target.clone());
        }
    }
    for (kind, selected) in planned {
        install_version(&selected, kind, "310.9.16", github, cancel, progress)?;
    }
    if !added.is_empty() {
        install_version(&added, "encorefg", "2.14.16", github, cancel, progress)?;
    }
    crate::neural_runtime::install_many(
        targets,
        crate::neural_runtime::RECOMMENDED,
        github,
        cancel,
        progress,
    )?;
    Ok(())
}
/// Only when sharing a managed component: verify that its recorded package is
/// still the installed package. No DLLs are executed or uploaded.
pub fn installed_package_version(
    target: &core::DeploymentTarget,
    kind: &str,
) -> Result<Option<String>> {
    let Some(record) = read(target, kind)? else {
        return Ok(None);
    };
    if !record.complete {
        return Ok(None);
    }
    for version in versions(kind) {
        let package = version_package(kind, &version)?;
        let dlls = package
            .files
            .iter()
            .filter(|a| a.name.ends_with(".dll"))
            .collect::<Vec<_>>();
        let mut same = true;
        for asset in dlls {
            let entry = record.entries.iter().find(|e| {
                std::path::Path::new(&e.path)
                    .file_name()
                    .is_some_and(|n| n.eq_ignore_ascii_case(&asset.name))
            });
            if let Some(entry) = entry {
                same &= entry.installed == asset.sha256
                    && matches(&destination(target, kind, entry)?, &asset.sha256)?;
            } else {
                same = false;
            }
        }
        if same {
            return Ok(Some(version));
        }
    }
    Ok(None)
}
fn prepare_package(
    p: Package,
    github: bool,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(transfer::Progress),
) -> Result<BTreeMap<String, PathBuf>> {
    let root = core::no_links(
        &assets::cache_root()?
            .join("optional-runtimes")
            .join(&p.sha256),
    )?;
    fs::create_dir_all(&root)?;
    let _lock = win::game_lock(&root)?;
    let mut sources: Vec<_> = p
        .urls
        .iter()
        .map(|url| transfer::Source {
            name: if url.contains("github.com/") {
                "github"
            } else {
                "gitcode"
            }
            .into(),
            url: url.clone(),
        })
        .collect();
    sources.sort_by_key(|s| s.name != if github { "github" } else { "gitcode" });
    let zip_path = root.join("runtime.zip");
    transfer::download(
        &transfer::Request {
            sources,
            bytes: Some(p.bytes),
            sha256: Some(p.sha256),
            limit: LIMIT,
            metadata: false,
            policy: transfer::UrlPolicy::Official,
        },
        &zip_path,
        cancel,
        progress,
    )?;
    let mut zip = zip::ZipArchive::new(fs::File::open(zip_path)?)?;
    ensure!(zip.len() == p.files.len(), "运行库压缩包内容异常");
    let mut out = BTreeMap::new();
    for asset in p.files {
        ensure!(
            !asset.name.contains(['/', '\\']) && asset.bytes <= LIMIT,
            "运行库文件名无效"
        );
        let dest = core::no_links(&root.join(&asset.name))?;
        if !matches(&dest, &asset.sha256)? {
            ensure!(!dest.exists(), "运行库缓存已变化，请先清理缓存");
            let mut entry = zip.by_name(&asset.name)?;
            ensure!(
                entry.size() == asset.bytes
                    && !entry.is_dir()
                    && entry.unix_mode().is_none_or(|m| m & 0o170000 != 0o120000),
                "运行库文件异常"
            );
            let mut stage = tempfile::NamedTempFile::new_in(&root)?;
            std::io::copy(&mut (&mut entry).take(asset.bytes + 1), &mut stage)?;
            stage.as_file().sync_all()?;
            ensure!(
                stage.as_file().metadata()?.len() == asset.bytes
                    && core::digest(stage.path())? == asset.sha256,
                "运行库摘要不匹配"
            );
            if asset.name.ends_with(".dll") {
                core::pe64(stage.path(), true)?;
            }
            ensure!(!cancel.load(Ordering::Relaxed), "下载已取消");
            stage.persist_noclobber(&dest)?;
        }
        out.insert(asset.name, dest);
    }
    Ok(out)
}

#[derive(Clone, Debug, Default)]
pub struct Status {
    pub kind: String,
    pub paths: Vec<PathBuf>,
    pub versions: BTreeSet<String>,
    /// All DLLs required by this component are physically present.
    pub present: bool,
    pub managed: bool,
    pub available: bool,
    /// Whether the installed scheme exposes this optional component, even when
    /// a native integration prevents installation.
    pub applicable: bool,
    pub warning: Option<String>,
}

/// Presentation policy only; Status::available still gates installation.
/// Keep the 4.3.21 runtime entries visible, explaining unmet prerequisites.
/// Only the new Encore FG set follows installed ownership for visibility.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    Hidden,
    Install,
    RestoreOnly,
}
pub fn access(kind: &str, statuses: &[&Status]) -> Access {
    if kind != "encorefg" || !statuses.is_empty() && statuses.iter().all(|s| s.applicable) {
        Access::Install
    } else if statuses.iter().any(|s| s.managed || s.applicable) {
        Access::RestoreOnly
    } else {
        Access::Hidden
    }
}

fn require_mfg_standalone(target: &core::DeploymentTarget) -> Result<()> {
    let r = core::record(&target.directory)?.context("请先安装 MFG Unlocker 1.7+")?;
    target.validate_record(&r)?;
    ensure!(
        r.backend == crate::rtxmfg::BACKEND
            && !r.cache_pending
            && r.upstream_version
                .as_deref()
                .or(r.payload_version.as_deref())
                .and_then(|v| semver::Version::parse(v).ok())
                .is_some_and(|v| v >= semver::Version::new(1, 7, 0)),
        "请先安装 MFG Unlocker 1.7+"
    );
    ensure!(
        !r.selected().iter().any(|p| p == crate::mfg_plugin::ENTRY),
        "OptiScaler 插件模式使用 OptiScaler 的 XeSS 运行库；此入口用于独立 XeMFG"
    );
    Ok(())
}
fn paths(target: &core::DeploymentTarget, kind: &str) -> Result<Vec<PathBuf>> {
    paths_from(target, kind, None)
}
// Patch dependencies and game runtimes are different ownership domains. Check
// names even for manual installations, then consult journals for custom paths.
fn protected(target: &core::DeploymentTarget, root: &Path, path: &Path) -> Result<bool> {
    let path = core::no_links(path)?;
    ensure!(core::within(&path, root), "运行库路径超出游戏目录");
    if path
        .components()
        .skip(root.components().count())
        .any(|c| scanner::patch_runtime_directory(&c.as_os_str().to_string_lossy()))
    {
        return Ok(true);
    }
    let mut owners = path
        .ancestors()
        .skip(1)
        .take_while(|p| core::within(p, root))
        .map(Path::to_path_buf)
        .collect::<Vec<_>>();
    owners.push(target.directory.clone());
    owners.sort();
    owners.dedup();
    for directory in owners {
        if let Some(r) = core::record(&directory)?
            && r.hashes
                .keys()
                .any(|n| core::key(&directory.join(n)) == core::key(&path))
        {
            return Ok(true);
        }
    }
    Ok(false)
}
fn paths_from(
    target: &core::DeploymentTarget,
    kind: &str,
    evidence: Option<&scanner::Evidence>,
) -> Result<Vec<PathBuf>> {
    let paths = candidate_paths(target, kind, evidence)?;
    ensure!(
        !paths.is_empty(),
        "没有可更新的游戏运行库；补丁专用文件不在此处更新"
    );
    Ok(paths)
}
// Absence is distinct from unreadable files or invalid ownership records.
fn candidate_paths(
    target: &core::DeploymentTarget,
    kind: &str,
    evidence: Option<&scanner::Evidence>,
) -> Result<Vec<PathBuf>> {
    if kind == "encorefg" {
        return added_fg_paths(target, evidence);
    }
    if kind == "xemfg" {
        require_mfg_standalone(target)?;
        return Ok(["libxess_fg.dll", "libxell.dll", "LICENSE.txt"]
            .iter()
            .map(|n| target.directory.join("RTXMFG-XeSS").join(n))
            .collect());
    }
    let name = filename(kind)?;
    let root = core::no_links(&scanner::installation_root(&target.game_exe))?;
    let mut found = evidence
        .map(|e| e.component_paths.clone())
        .unwrap_or_else(|| scanner::inspect_for_installation(&target.game_exe).component_paths);
    // A removed managed DLL still has a known destination and can be repaired.
    // Invalid journals are reported instead of being mistaken for absence.
    if let Some(record) = read(target, kind)? {
        for entry in &record.entries {
            found.push(destination(target, kind, entry)?);
        }
    }
    let local = target.directory.join(name);
    if local.is_file() {
        found.push(local);
    }
    found.retain(|p| {
        core::within(p, &root) && p.file_name().is_some_and(|n| n.eq_ignore_ascii_case(name))
    });
    let mut eligible = Vec::new();
    for p in found {
        if !protected(target, &root, &p)? {
            eligible.push(p);
        }
    }
    let mut found = eligible;
    if kind == "fg"
        && let Some(r) = read(target, "encorefg")?
    {
        // The FG DLL belongs to a qualified six-file set. Never update only one
        // part through the ordinary native-DLSS updater.
        found.retain(|p| {
            !r.entries
                .iter()
                .any(|e| core::key(Path::new(&e.path)) == core::key(p))
        });
    }
    found.sort_by_key(|p| core::key(p));
    found.dedup_by_key(|p| core::key(p));
    if found.is_empty() && kind == "sr" && encore_200_installed(target)? {
        let inspected;
        let e = if let Some(e) = evidence {
            e
        } else {
            inspected = scanner::inspect_for_installation(&target.game_exe);
            &inspected
        };
        ensure!(
            !e.limited,
            "组件检测未完成，请刷新后重试；不会向未知位置添加运行库"
        );
        if !e.neural_input() && !e.super_resolution_plugin {
            found.push(core::no_links(&target.directory.join(name))?);
        }
    }
    Ok(found)
}
pub(crate) fn encore_200_installed(target: &core::DeploymentTarget) -> Result<bool> {
    let Some(r) = core::record(&target.directory)? else {
        return Ok(false);
    };
    target.validate_record(&r)?;
    Ok(r.backend == crate::encore::BACKEND
        && !r.cache_pending
        && crate::encore::supports_200(
            r.upstream_version
                .as_deref()
                .or(r.payload_version.as_deref()),
        )
        && !crate::encore_upgrade::has_pending(&target.directory)?)
}
fn added_fg_paths(
    target: &core::DeploymentTarget,
    evidence: Option<&scanner::Evidence>,
) -> Result<Vec<PathBuf>> {
    ensure!(
        encore_200_installed(target)?,
        "此组件需要先安装 Encore 2.0+，仅用于无原生帧生成的游戏"
    );
    preflight_added_fg(target, evidence)
}
pub(crate) fn preflight_added_fg(
    target: &core::DeploymentTarget,
    evidence: Option<&scanner::Evidence>,
) -> Result<Vec<PathBuf>> {
    let inspected;
    let e = if let Some(e) = evidence {
        e
    } else {
        inspected = scanner::inspect_for_installation(&target.game_exe);
        &inspected
    };
    ensure!(
        !e.limited,
        "组件检测未完成，请刷新后重试；不会向未知位置添加运行库"
    );
    let prior = read(target, "encorefg")?;
    for p in &e.frame_integration_paths {
        let own = prior.as_ref().and_then(|r| {
            r.entries
                .iter()
                .find(|e| e.original.is_none() && core::key(Path::new(&e.path)) == core::key(p))
        });
        ensure!(
            own.is_some_and(|entry| matches(p, &entry.installed).unwrap_or(false)),
            "游戏已有原生帧生成或 Streamline；使用上方 DLSS 帧生成更新，无需添加第二套帧生成。"
        );
    }
    let directory = target.game_exe.parent().context("游戏目录无效")?;
    let mut paths = Vec::new();
    for name in ENCORE_FG_FILES {
        let p = core::no_links(&directory.join(name))?;
        if p.exists() {
            let own = prior.as_ref().and_then(|r| {
                r.entries
                    .iter()
                    .find(|e| core::key(Path::new(&e.path)) == core::key(&p))
            });
            ensure!(
                own.is_some_and(|entry| matches(&p, &entry.installed).unwrap_or(false)),
                "新增帧生成目标已存在外部文件，已保留：{}",
                p.display()
            );
        }
        paths.push(p);
    }
    Ok(paths)
}
/// Validate only the newly added paths; native features retain their previous
/// behavior. Called before the configured bytes replace the player's file.
pub(crate) fn validate_encore_features(
    target: &core::DeploymentTarget,
    bytes: &[u8],
) -> Result<()> {
    let values = crate::encore::read(bytes)?;
    let on = |key: &str| values.get(key).is_some_and(|v| v == "1");
    if on("nrWithoutGameDlss") {
        ensure!(
            crate::neural_runtime::inspect(target).present,
            "请先在 DLSS/XeSS 运行库中安装 DLSSNR，再启用无原生 DLSS 游戏的 NR"
        );
    }
    if on("dlssWithoutGameDlss")
        && values
            .get("dlssWithoutGameDlssEngine")
            .is_some_and(|v| v == "dlss")
    {
        ensure!(
            scanner::inspect(&target.game_exe).neural_input()
                || target.directory.join("nvngx_dlss.dll").is_file(),
            "请先在 DLSS/XeSS 运行库中安装 DLSS 超分辨率，再启用新增超分"
        );
    }
    if on("frameGenerationWithoutGameFg") {
        ensure!(
            !on("smoothMotionEnabled"),
            "新增帧生成不能与 Smooth Motion 同时开启，请关闭其中一项"
        );
        preflight_added_fg(target, None)?;
        if values
            .get("frameGenerationEngine")
            .is_none_or(|v| v != "xefg")
        {
            let p = bundled_package("encorefg")?;
            let dir = target.game_exe.parent().context("游戏目录无效")?;
            for f in p.files.iter().filter(|f| f.name.ends_with(".dll")) {
                ensure!(
                    matches(&dir.join(&f.name), &f.sha256)?,
                    "请先安装新增 DLSS 帧生成的完整组件（Streamline 2.14.16 + DLSS-G 310.9.16）"
                );
            }
        }
    }
    Ok(())
}
fn root(target: &core::DeploymentTarget, kind: &str) -> Result<PathBuf> {
    filename(kind)?;
    core::no_links(&target.directory.join(OWN).join(kind))
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    schema: u8,
    #[serde(default)]
    complete: bool,
    game: String,
    directory: String,
    kind: String,
    entries: Vec<Entry>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    path: String,
    installed: String,
    previous: Option<String>,
    original: Option<String>,
}
// Windows addresses may come from launcher manifests with '/' separators or
// different casing. Normalize identity without canonicalize(), which would
// follow links and require a still-existing game EXE during recovery.
fn same_record_path(recorded: &str, current: &Path) -> Result<bool> {
    let recorded = Path::new(recorded);
    Ok(recorded.is_absolute()
        && core::key(&core::no_links(recorded)?) == core::key(&core::no_links(current)?))
}
fn record_path(path: &Path) -> Result<String> {
    Ok(core::no_links(path)?.to_string_lossy().replace('/', "\\"))
}
fn destination(target: &core::DeploymentTarget, kind: &str, entry: &Entry) -> Result<PathBuf> {
    let p = core::no_links(Path::new(&entry.path))?;
    let valid_name = p.file_name().is_some_and(|n| {
        if kind == "encorefg" {
            ENCORE_FG_FILES.iter().any(|s| n.eq_ignore_ascii_case(s))
        } else if kind == "xemfg" {
            ["libxess_fg.dll", "libxell.dll", "LICENSE.txt"]
                .iter()
                .any(|s| n.eq_ignore_ascii_case(s))
        } else if kind == "fg" {
            n.eq_ignore_ascii_case("nvngx_dlssg.dll")
                || STREAMLINE_PLUGINS
                    .iter()
                    .any(|name| n.eq_ignore_ascii_case(name))
                || n.eq_ignore_ascii_case(STREAMLINE_LICENSE)
        } else {
            n.eq_ignore_ascii_case(filename(kind).unwrap_or(""))
        }
    });
    ensure!(
        valid_name
            && if kind == "encorefg" {
                p.parent()
                    .zip(target.game_exe.parent())
                    .is_some_and(|(a, b)| core::key(a) == core::key(b))
            } else if kind == "xemfg" {
                p.parent().is_some_and(|parent| {
                    core::key(parent) == core::key(&target.directory.join("RTXMFG-XeSS"))
                })
            } else {
                core::within(
                    &p,
                    &core::no_links(&scanner::installation_root(&target.game_exe))?,
                )
            },
        "运行库恢复路径超出当前游戏范围"
    );
    Ok(p)
}
fn read(target: &core::DeploymentTarget, kind: &str) -> Result<Option<Record>> {
    let p = core::no_links(&root(target, kind)?.join("ownership.json"))?;
    if !p.exists() {
        return Ok(None);
    }
    ensure!(p.metadata()?.len() <= 128 * 1024, "运行库记录过大");
    let r: Record = serde_json::from_slice(&fs::read(p)?)?;
    ensure!(
        r.schema == 1
            && same_record_path(&r.game, &target.game_exe)?
            && same_record_path(&r.directory, &target.directory)?
            && r.kind == kind
            && !r.entries.is_empty()
            && r.entries.len() <= 128,
        "运行库记录不属于当前游戏"
    );
    let mut seen = BTreeSet::new();
    for e in &r.entries {
        let p = destination(target, kind, e)?;
        ensure!(seen.insert(core::key(&p)), "运行库记录存在重复文件");
        for hash in [Some(&e.installed), e.previous.as_ref(), e.original.as_ref()]
            .into_iter()
            .flatten()
        {
            ensure!(
                hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()),
                "运行库记录摘要无效"
            );
        }
    }
    Ok(Some(r))
}
fn matches(path: &Path, hash: &str) -> Result<bool> {
    let p = core::no_links(path)?;
    Ok(p.is_file() && p.metadata()?.len() <= LIMIT && core::digest(&p)? == hash)
}
fn accepted(path: &Path, entry: &Entry) -> Result<bool> {
    if !path.exists() {
        return Ok(true);
    }
    for hash in [
        Some(&entry.installed),
        entry.previous.as_ref(),
        entry.original.as_ref(),
    ]
    .into_iter()
    .flatten()
    {
        if matches(path, hash)? {
            return Ok(true);
        }
    }
    Ok(false)
}
pub fn inspect(target: &core::DeploymentTarget, kind: &str) -> Status {
    inspect_with(target, kind, None)
}
pub fn inspect_all(target: &core::DeploymentTarget) -> Vec<Status> {
    let evidence = scanner::inspect(&target.game_exe);
    KINDS
        .iter()
        .map(|kind| inspect_with(target, kind, Some(&evidence)))
        .collect()
}
fn inspect_with(
    target: &core::DeploymentTarget,
    kind: &str,
    evidence: Option<&scanner::Evidence>,
) -> Status {
    let mut s = Status {
        kind: kind.into(),
        applicable: match kind {
            "encorefg" => encore_200_installed(target).unwrap_or(false),
            "xemfg" => require_mfg_standalone(target).is_ok(),
            _ => true,
        },
        ..Default::default()
    };
    let result = (|| -> Result<()> {
        if let Some(r) = read(target, kind)? {
            s.managed = true;
            s.paths = r.entries.iter().map(|e| PathBuf::from(&e.path)).collect();
            if !r.complete {
                s.warning = Some("运行库安装未完成，可恢复原件后重试。".into());
            }
        }
        match paths_from(target, kind, evidence) {
            Ok(p) => {
                s.available = true;
                s.paths.extend(p);
                s.paths.sort_by_key(|p| core::key(p));
                s.paths.dedup_by_key(|p| core::key(p));
            }
            Err(e) => {
                s.warning = Some(format!("{e:#}"));
            }
        }
        s.present = dll_set_present(kind, &s.paths);
        if kind == "encorefg" && s.available && s.warning.is_none() {
            s.warning = Some("实验性组件：仅添加运行库，不自动启用。请在参数设置中选择引擎，并关闭 Smooth Motion。".into());
        }
        if kind == "fg" && s.available && encore_200_installed(target)? && s.warning.is_none() {
            s.warning = Some(
                "更新 DLSS-G 310.9.16 时同步更新配套 Streamline 插件，保留游戏原有 interposer。"
                    .into(),
            );
        }
        for p in &s.paths {
            if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("dll"))
                && p.is_file()
                && (kind != "fg"
                    || p.file_name()
                        .is_some_and(|n| n.eq_ignore_ascii_case("nvngx_dlssg.dll")))
            {
                s.versions.insert(
                    win::file_version(p)
                        .map(|v| v.0)
                        .unwrap_or_else(|_| "版本未知".into()),
                );
            }
        }
        Ok(())
    })();
    if let Err(e) = result {
        s.warning = Some(format!("{e:#}"));
        s.available = false;
    }
    s
}
fn dll_set_present(kind: &str, paths: &[PathBuf]) -> bool {
    if kind == "encorefg" {
        ENCORE_FG_FILES
            .iter()
            .filter(|n| n.ends_with(".dll"))
            .all(|name| {
                paths.iter().any(|p| {
                    p.file_name().is_some_and(|n| n.eq_ignore_ascii_case(name)) && p.is_file()
                })
            })
    } else if kind == "xemfg" {
        ["libxess_fg.dll", "libxell.dll"].iter().all(|name| {
            paths
                .iter()
                .any(|p| p.file_name().is_some_and(|n| n.eq_ignore_ascii_case(name)) && p.is_file())
        })
    } else {
        let dlls: Vec<_> = paths
            .iter()
            .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("dll")))
            .collect();
        !dlls.is_empty() && dlls.iter().all(|p| p.is_file())
    }
}
fn replace(source: &Path, destination: &Path) -> Result<()> {
    let destination = core::no_links(destination)?;
    fs::create_dir_all(destination.parent().context("运行库目录无效")?)?;
    core::no_links(&destination)?;
    let mut temp = tempfile::NamedTempFile::new_in(destination.parent().unwrap())?;
    std::io::copy(&mut fs::File::open(source)?, &mut temp)?;
    temp.as_file().sync_all()?;
    ensure!(
        core::digest(temp.path())? == core::digest(source)?,
        "运行库暂存校验失败"
    );
    temp.persist(destination)?;
    Ok(())
}
fn plan_entries(
    target: &core::DeploymentTarget,
    kind: &str,
    destinations: &[PathBuf],
    sources: &BTreeMap<String, PathBuf>,
) -> Result<Vec<Entry>> {
    core::assert_target_stopped(target)?;
    let folder = root(target, kind)?;
    let prior = read(target, kind)?;
    ensure!(
        prior.is_some() || !folder.exists(),
        "发现未完成的运行库备份，请保留并检查"
    );
    let mut entries = Vec::new();
    for p in destinations {
        let p = core::no_links(p)?;
        if kind != "xemfg" {
            ensure!(
                !protected(
                    target,
                    &core::no_links(&scanner::installation_root(&target.game_exe))?,
                    &p
                )?,
                "此文件属于补丁专用运行库，请更新完整适配方案"
            );
        }
        let name = p.file_name().context("运行库名称无效")?.to_string_lossy();
        let source = sources
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(&name))
            .map(|(_, p)| p)
            .context("运行库包缺少文件")?;
        let existing = prior.as_ref().and_then(|r| {
            r.entries
                .iter()
                .find(|e| core::key(Path::new(&e.path)) == core::key(&p))
        });
        if kind == "encorefg" && p.exists() {
            // Recheck after downloading too: another mod/game update may have
            // created this file since the initial no-native-FG preflight.
            ensure!(
                existing.is_some_and(
                    |e| e.original.is_none() && matches(&p, &e.installed).unwrap_or(false)
                ),
                "新增帧生成目标已有外部文件，已保留：{}",
                p.display()
            );
        }
        if let Some(e) = existing {
            ensure!(
                accepted(&p, e)?,
                "运行库被其他程序修改，已保留：{}",
                p.display()
            );
            if let Some(h) = &e.original {
                ensure!(
                    matches(&folder.join(format!("{h}.bak")), h)?,
                    "运行库原始备份损坏"
                );
            }
        }
        ensure!(
            !p.exists() || (p.is_file() && p.metadata()?.len() <= LIMIT),
            "运行库文件类型或大小异常"
        );
        let before = if p.exists() {
            Some(core::digest(&p)?)
        } else {
            None
        };
        entries.push(Entry {
            path: record_path(&p)?,
            installed: core::digest(source)?,
            previous: before.clone(),
            original: existing.map(|e| e.original.clone()).unwrap_or(before),
        });
    }
    // Keep prior paths if a game update changed which folder the scanner finds.
    if let Some(prior) = prior {
        for mut e in prior.entries {
            if !entries
                .iter()
                .any(|n| core::key(Path::new(&n.path)) == core::key(Path::new(&e.path)))
            {
                e.path = record_path(Path::new(&e.path))?;
                entries.push(e);
            }
        }
    }
    ensure!(entries.len() <= 128, "运行库文件数量超出限制");
    for e in &entries {
        destination(target, kind, e)?;
    }
    Ok(entries)
}
fn install_locked(
    target: &core::DeploymentTarget,
    kind: &str,
    destinations: &[PathBuf],
    sources: &BTreeMap<String, PathBuf>,
) -> Result<()> {
    install_locked_with(target, kind, destinations, sources, replace)
}
fn install_locked_with(
    target: &core::DeploymentTarget,
    kind: &str,
    destinations: &[PathBuf],
    sources: &BTreeMap<String, PathBuf>,
    mut write_file: impl FnMut(&Path, &Path) -> Result<()>,
) -> Result<()> {
    let entries = plan_entries(target, kind, destinations, sources)?;
    let folder = root(target, kind)?;
    fs::create_dir_all(&folder)?;
    for e in &entries {
        if let Some(h) = &e.original {
            let backup = core::no_links(&folder.join(format!("{h}.bak")))?;
            if !backup.exists() {
                fs::copy(&e.path, &backup)?;
            }
            ensure!(matches(&backup, h)?, "运行库备份校验失败");
        }
    }
    let mut record = Record {
        schema: 1,
        complete: false,
        game: record_path(&target.game_exe)?,
        directory: record_path(&target.directory)?,
        kind: kind.into(),
        entries,
    };
    let mut temp = tempfile::NamedTempFile::new_in(&folder)?;
    temp.write_all(&serde_json::to_vec_pretty(&record)?)?;
    temp.as_file().sync_all()?;
    // Persist intent before replacement. An interrupted batch can always restore
    // originals; never delete its journal after only some files were written.
    temp.persist(folder.join("ownership.json"))?;
    let result = (|| -> Result<()> {
        for p in destinations {
            core::assert_target_stopped(target)?;
            let e = record
                .entries
                .iter()
                .find(|e| core::key(Path::new(&e.path)) == core::key(p))
                .unwrap();
            ensure!(
                match &e.previous {
                    Some(h) => matches(p, h)?,
                    None => !p.exists(),
                },
                "运行库在操作期间发生变化：{}",
                p.display()
            );
            write_file(
                sources
                    .iter()
                    .find(|(name, _)| p.file_name().unwrap().eq_ignore_ascii_case(name.as_str()))
                    .map(|(_, p)| p)
                    .context("运行库包缺少文件")?,
                p,
            )?;
        }
        Ok(())
    })();
    if let Err(error) = result {
        remove_locked(target, kind)
            .context(format!("安装失败：{error:#}；恢复未完成，请保留备份"))?;
        return Err(error.context("运行库安装失败，已恢复原件"));
    }
    record.complete = true;
    let mut temp = tempfile::NamedTempFile::new_in(&folder)?;
    temp.write_all(&serde_json::to_vec_pretty(&record)?)?;
    temp.as_file().sync_all()?;
    temp.persist(folder.join("ownership.json"))?;
    Ok(())
}
pub fn install_many(
    targets: &[core::DeploymentTarget],
    kind: &str,
    github: bool,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(transfer::Progress),
) -> Result<()> {
    install_version(targets, kind, &recommended(kind), github, cancel, progress)
}
pub fn install_version(
    targets: &[core::DeploymentTarget],
    kind: &str,
    version: &str,
    github: bool,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(transfer::Progress),
) -> Result<()> {
    // Resolve once before starting the operation; a background catalog refresh
    // cannot change the bytes selected by the user midway through a download.
    let selected = version_package(kind, version)?;
    let native_plugins = kind == "fg"
        && version == "310.9.16"
        && targets
            .iter()
            .map(encore_200_installed)
            .collect::<Result<Vec<_>>>()?
            .iter()
            .any(|v| *v);
    install_many_with(targets, kind, cancel, || {
        let mut sources = prepare_package(selected, github, cancel, progress)?;
        if native_plugins {
            sources.extend(prepare_package(
                bundled_package("streamline")?,
                github,
                cancel,
                progress,
            )?);
        }
        Ok(sources)
    })
}
fn install_many_with(
    targets: &[core::DeploymentTarget],
    kind: &str,
    cancel: &AtomicBool,
    prepare_source: impl FnOnce() -> Result<BTreeMap<String, PathBuf>>,
) -> Result<()> {
    filename(kind)?;
    ensure!(!targets.is_empty(), "未选择游戏");
    let _cache = crate::cache::wait_for_operation(cancel, || {})?;
    let mut ordered = targets
        .iter()
        .map(|t| t.validate(true))
        .collect::<Result<Vec<_>>>()?;
    ordered.sort_by_key(|t| core::key(&t.directory));
    ordered.dedup_by_key(|t| core::key(&t.directory));
    let mut locks = Vec::new();
    let mut plans = Vec::new();
    let mut seen = BTreeSet::new();
    for t in &ordered {
        core::assert_target_stopped(t)?;
        locks.push(win::game_lock(&t.directory)?);
        let mut ps = paths(t, kind)?;
        ps.retain(|p| seen.insert(core::key(p)));
        if !ps.is_empty() {
            plans.push((t, ps));
        }
    }
    let sources = prepare_source()?;
    if kind == "fg" && sources.contains_key("sl.common.dll") {
        for (t, ps) in &mut plans {
            if encore_200_installed(t)? {
                for path in native_streamline_paths(t)? {
                    if seen.insert(core::key(&path)) {
                        ps.push(path);
                    }
                }
            }
        }
    }
    // Detect an existing conflict in any directory before changing the first.
    for (t, ps) in &plans {
        plan_entries(t, kind, ps, &sources)?;
    }
    for (t, ps) in plans {
        ensure!(!cancel.load(Ordering::Relaxed), "下载已取消");
        install_locked(t, kind, &ps, &sources).with_context(|| {
            format!(
                "运行库安装未完成；请刷新状态，可使用恢复按钮：{}",
                t.directory.display()
            )
        })?;
    }
    Ok(())
}
pub fn remove_locked(target: &core::DeploymentTarget, kind: &str) -> Result<()> {
    let Some(r) = read(target, kind)? else {
        return Ok(());
    };
    core::assert_target_stopped(target)?;
    let folder = root(target, kind)?;
    for e in &r.entries {
        let p = destination(target, kind, e)?;
        ensure!(
            accepted(&p, e)?,
            "运行库已被其他程序修改，保留文件及备份：{}",
            p.display()
        );
        if let Some(h) = &e.original {
            ensure!(
                matches(&folder.join(format!("{h}.bak")), h)?,
                "运行库原件备份损坏"
            );
        }
    }
    for e in &r.entries {
        core::assert_target_stopped(target)?;
        let p = destination(target, kind, e)?;
        ensure!(accepted(&p, e)?, "运行库在恢复期间已变化");
        if let Some(h) = &e.original {
            replace(&folder.join(format!("{h}.bak")), &p)?;
        } else if p.exists() {
            fs::remove_file(p)?;
        }
    }
    fs::remove_file(folder.join("ownership.json"))?;
    for h in r
        .entries
        .iter()
        .filter_map(|e| e.original.as_ref())
        .collect::<BTreeSet<_>>()
    {
        fs::remove_file(core::no_links(&folder.join(format!("{h}.bak")))?)?;
    }
    let _ = fs::remove_dir(&folder);
    let _ = fs::remove_dir(target.directory.join(OWN));
    if kind == "xemfg" {
        let _ = fs::remove_dir(core::no_links(&target.directory.join("RTXMFG-XeSS"))?);
    }
    Ok(())
}
pub fn remove_many(targets: &[core::DeploymentTarget], kind: &str) -> Result<()> {
    let mut errors = Vec::new();
    for t in targets {
        let r = (|| {
            let t = t.validate(false)?;
            let _lock = win::game_lock(&t.directory)?;
            remove_locked(&t, kind)
        })();
        if let Err(e) = r {
            errors.push(format!("{}：{e:#}", t.directory.display()));
        }
    }
    ensure!(errors.is_empty(), "{}", errors.join("\n"));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_streamline_updates_plugins_in_place_and_restores_originals() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let t = target(&tmp.path().join("Game"))?;
        let folder = t
            .directory
            .join("Engine/Plugins/NVIDIA/Streamline/Binaries/Win64");
        fs::create_dir_all(&folder)?;
        let originals = [
            "nvngx_dlssg.dll",
            "sl.interposer.dll",
            "sl.common.dll",
            "sl.dlss_g.dll",
            "sl.deepdvc.dll",
            "sl.directsr.dll",
        ];
        for name in originals {
            fs::write(folder.join(name), format!("original {name}"))?;
        }
        let e = scanner::inspect(&t.game_exe);
        let mut ps = native_streamline_paths_from(&t, &e, |_| Ok("2.7.30.0".into()))?;
        assert!(ps.iter().all(|p| p.parent() == Some(folder.as_path())));
        assert!(!ps.contains(&folder.join("sl.interposer.dll")));
        assert!(
            ps.contains(&folder.join("sl.reflex.dll")),
            "missing dependencies added at the native provider"
        );
        ps.push(folder.join("nvngx_dlssg.dll"));
        let mut src = sources(&tmp.path().join("sources"), "fg")?;
        for name in STREAMLINE_PLUGINS.into_iter().chain([STREAMLINE_LICENSE]) {
            let path = tmp.path().join("sources").join(name);
            fs::write(&path, format!("new {name}"))?;
            src.insert(name.into(), path);
        }
        for boundary in 0..ps.len() {
            let mut writes = 0;
            assert!(
                install_locked_with(&t, "fg", &ps, &src, |s, d| {
                    if writes == boundary {
                        anyhow::bail!("injected failure");
                    }
                    writes += 1;
                    replace(s, d)
                })
                .is_err()
            );
            for name in originals {
                assert_eq!(
                    fs::read(folder.join(name))?,
                    format!("original {name}").as_bytes()
                );
            }
            assert!(!folder.join("sl.reflex.dll").exists());
        }
        install_locked(&t, "fg", &ps, &src)?;
        install_locked(&t, "fg", &ps, &src)?;
        assert_eq!(
            fs::read(folder.join("sl.interposer.dll"))?,
            b"original sl.interposer.dll"
        );
        assert!(!t.directory.join("sl.interposer.dll").exists());
        fs::remove_file(folder.join("sl.common.dll"))?;
        fs::remove_file(folder.join("nvngx_dlssg.dll"))?;
        let evidence = scanner::inspect(&t.game_exe);
        let mut repair = native_streamline_paths_from(&t, &evidence, |_| Ok("2.14.16".into()))?;
        repair.extend(candidate_paths(&t, "fg", Some(&evidence))?);
        assert!(repair.contains(&folder.join("sl.common.dll")));
        assert!(repair.contains(&folder.join("nvngx_dlssg.dll")));
        install_locked(&t, "fg", &repair, &src)?;
        remove_locked(&t, "fg")?;
        for name in originals {
            assert_eq!(
                fs::read(folder.join(name))?,
                format!("original {name}").as_bytes()
            );
        }
        assert!(!folder.join("sl.reflex.dll").exists());
        assert!(!folder.join(STREAMLINE_LICENSE).exists());
        Ok(())
    }
    #[test]
    fn native_streamline_refuses_incomplete_scan_unknown_plugins_and_downgrades() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let t = target(&tmp.path().join("Game"))?;
        fs::write(t.directory.join("sl.common.dll"), b"native plugin")?;
        let mut e = scanner::inspect(&t.game_exe);
        assert!(native_streamline_paths_from(&t, &e, |_| Ok("3.0.0.0".into())).is_err());
        fs::write(t.directory.join("sl.custom.dll"), b"unrecognized plugin")?;
        assert!(native_streamline_paths_from(&t, &e, |_| Ok("2.7.30.0".into())).is_err());
        fs::remove_file(t.directory.join("sl.custom.dll"))?;
        e.limited = true;
        assert!(native_streamline_paths_from(&t, &e, |_| Ok("2.7.30.0".into())).is_err());
        assert_eq!(
            fs::read(t.directory.join("sl.common.dll"))?,
            b"native plugin"
        );
        Ok(())
    }
    #[test]
    #[ignore = "publication smoke test: downloads one small FG ZIP per public source without cache"]
    fn published_runtime_downloads_without_cache() -> Result<()> {
        let p = package("fg")?;
        let tmp = tempfile::tempdir()?;
        for (i, url) in p.urls.iter().enumerate() {
            let name = if url.contains("github.com/") {
                "github"
            } else {
                "gitcode"
            };
            let start = std::time::Instant::now();
            let path = tmp.path().join(format!("{i}.zip"));
            ensure!(!path.exists(), "download target must start without a cache");
            transfer::download(
                &transfer::Request {
                    sources: vec![transfer::Source {
                        name: name.into(),
                        url: url.clone(),
                    }],
                    bytes: Some(p.bytes),
                    sha256: Some(p.sha256.clone()),
                    limit: LIMIT,
                    metadata: false,
                    policy: transfer::UrlPolicy::Official,
                },
                &path,
                &AtomicBool::new(false),
                &mut |_: transfer::Progress| {},
            )?;
            let archive = zip::ZipArchive::new(fs::File::open(&path)?)?;
            ensure!(archive.len() == p.files.len(), "invalid runtime archive");
            println!(
                "{name}: {} bytes, SHA256 verified, {:.2}s",
                p.bytes,
                start.elapsed().as_secs_f64()
            );
        }
        Ok(())
    }
    #[test]
    #[ignore = "requires RTXFG_RUNTIME_ARCHIVES; checks only four new local ZIPs, no network or DLL execution"]
    fn local_runtime_packages_extract_offline_install_and_restore() -> Result<()> {
        let archives = PathBuf::from(
            std::env::var_os("RTXFG_RUNTIME_ARCHIVES").context("runtime ZIP directory required")?,
        );
        if std::env::var_os("RTXFG_RUNTIME_TEST_CHILD").is_none() {
            let tmp = tempfile::tempdir()?;
            let out=std::process::Command::new(std::env::current_exe()?)
                .args(["--exact","runtime_files::tests::local_runtime_packages_extract_offline_install_and_restore","--ignored","--nocapture"])
                .env("LOCALAPPDATA",tmp.path().join("appdata")).env("RTXFG_RUNTIME_TEST_CHILD","1").output()?;
            ensure!(
                out.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            );
            return Ok(());
        }
        let tmp = tempfile::tempdir()?;
        let t = target(&tmp.path().join("Game"))?;
        let mut hashes = BTreeMap::new();
        for name in core::deployment_names(crate::rtxmfg::BACKEND, &["version.dll".into()])? {
            let p = t.directory.join(&name);
            fs::write(&p, b"synthetic MFG")?;
            hashes.insert(name, core::digest(&p)?);
        }
        core::atomic_json(
            &t.directory.join(core::OWN).join(core::MARKER),
            &serde_json::json!({"schema":3,"backend":crate::rtxmfg::BACKEND,"scheme_id":crate::rtxmfg_upgrade::SCHEME,"payload_version":"1.7.0","proxy":"version.dll","hashes":hashes}),
        )?;
        for kind in ["sr", "rr", "fg", "xemfg"] {
            let p = package(kind)?;
            let name = p.urls[0].rsplit('/').next().unwrap();
            let zip = archives.join(name);
            ensure!(
                zip.metadata()?.len() == p.bytes && core::digest(&zip)? == p.sha256,
                "new archive mismatch"
            );
            let cache = assets::cache_root()?
                .join("optional-runtimes")
                .join(&p.sha256);
            fs::create_dir_all(&cache)?;
            fs::copy(zip, cache.join("runtime.zip"))?;
            let dest = t.directory.join(filename(kind)?);
            if kind != "xemfg" {
                fs::write(&dest, b"original game runtime")?;
            }
            let mut phases = Vec::new();
            install_many(
                std::slice::from_ref(&t),
                kind,
                false,
                &AtomicBool::new(false),
                &mut |p| phases.push(p.phase),
            )?;
            let r = read(&t, kind)?.unwrap();
            assert!(r.complete);
            for e in &r.entries {
                assert!(matches(Path::new(&e.path), &e.installed)?);
            }
            // A valid cache must not start any transfer.
            assert!(!phases.contains(&transfer::Phase::Downloading));
            remove_many(std::slice::from_ref(&t), kind)?;
            if kind != "xemfg" {
                assert_eq!(fs::read(dest)?, b"original game runtime");
            } else {
                assert!(!t.directory.join("RTXMFG-XeSS").exists());
            }
        }
        Ok(())
    }
    fn target(root: &Path) -> Result<core::DeploymentTarget> {
        fs::create_dir_all(root)?;
        let mut pe = vec![0; 512];
        pe[..2].copy_from_slice(b"MZ");
        pe[60..64].copy_from_slice(&128u32.to_le_bytes());
        pe[128..132].copy_from_slice(b"PE\0\0");
        pe[132..134].copy_from_slice(&0x8664u16.to_le_bytes());
        pe[148..150].copy_from_slice(&240u16.to_le_bytes());
        pe[150..152].copy_from_slice(&2u16.to_le_bytes());
        pe[152..154].copy_from_slice(&0x20bu16.to_le_bytes());
        let exe = root.join("Game.exe");
        fs::write(&exe, pe)?;
        core::DeploymentTarget::for_game(&exe).validate(true)
    }
    #[test]
    fn legacy_runtime_entries_stay_visible_and_added_fg_retains_ownership_gate() {
        let hidden = Status::default();
        let installed = Status {
            applicable: true,
            ..Default::default()
        };
        let leftover = Status {
            managed: true,
            ..Default::default()
        };
        assert_eq!(access("encorefg", &[]), Access::Hidden);
        assert_eq!(access("encorefg", &[&hidden]), Access::Hidden);
        assert_eq!(access("encorefg", &[&installed]), Access::Install);
        assert_eq!(access("encorefg", &[&leftover]), Access::RestoreOnly);
        assert_eq!(
            access("encorefg", &[&installed, &hidden]),
            Access::RestoreOnly
        );
        for kind in ["sr", "rr", "fg", "xemfg"] {
            assert_eq!(access(kind, &[]), Access::Install);
            assert_eq!(access(kind, &[&hidden]), Access::Install);
            assert_eq!(access(kind, &[&installed, &hidden]), Access::Install);
            assert_eq!(access(kind, &[&leftover]), Access::Install);
            assert!(
                !hidden.available,
                "a visible entry cannot authorize installation"
            );
        }
    }
    #[test]
    fn xemfg_visibility_uses_record_version_and_loading_mode() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let t = target(&tmp.path().join("Game"))?;
        assert!(!inspect_with(&t, "xemfg", None).applicable);
        for (version, proxy, pending, applicable) in [
            ("1.6.1", "version.dll", false, false),
            ("1.7.0", "version.dll", false, true),
            ("1.7.0", crate::mfg_plugin::ENTRY, false, false),
            ("1.7.0", "version.dll", true, false),
        ] {
            let hashes: BTreeMap<_, _> =
                core::deployment_names(crate::rtxmfg::BACKEND, &[proxy.into()])?
                    .into_iter()
                    .map(|name| (name, "0".repeat(64)))
                    .collect();
            core::atomic_json(
                &t.directory.join(core::OWN).join(core::MARKER),
                &serde_json::json!({
                    "schema":3, "backend":crate::rtxmfg::BACKEND,
                    "scheme_id":crate::rtxmfg_upgrade::SCHEME, "payload_version":version,
                    "proxy":proxy, "cache_pending":pending, "hashes":hashes
                }),
            )?;
            assert_eq!(
                inspect_with(&t, "xemfg", None).applicable,
                applicable,
                "{version} {proxy} {pending}"
            );
            assert!(!inspect_with(&t, "encorefg", None).applicable);
        }
        Ok(())
    }
    fn sources(dir: &Path, kind: &str) -> Result<BTreeMap<String, PathBuf>> {
        fs::create_dir_all(dir)?;
        let names = if kind == "encorefg" {
            ENCORE_FG_FILES.to_vec()
        } else if kind == "xemfg" {
            vec!["libxess_fg.dll", "libxell.dll", "LICENSE.txt"]
        } else {
            vec![filename(kind)?]
        };
        names
            .into_iter()
            .map(|n| {
                let p = dir.join(n);
                fs::write(&p, format!("synthetic new {n}"))?;
                Ok((n.into(), p))
            })
            .collect()
    }
    #[test]
    fn added_fg_is_a_complete_owned_set_and_never_a_native_fg_update() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let t = target(&tmp.path().join("Game"))?;
        let s = sources(&tmp.path().join("sources"), "encorefg")?;
        let ps = preflight_added_fg(&t, None)?;
        install_locked(&t, "encorefg", &ps, &s)?;
        assert!(dll_set_present("encorefg", &ps));
        assert!(
            paths(&t, "fg").is_err(),
            "never replace a member independently"
        );
        assert_eq!(preflight_added_fg(&t, None)?, ps);
        fs::remove_file(t.directory.join("sl.reflex.dll"))?;
        assert!(!dll_set_present("encorefg", &ps));
        install_locked(&t, "encorefg", &ps, &s)?;
        fs::write(t.directory.join("sl.pcl.dll"), b"foreign edit")?;
        assert!(preflight_added_fg(&t, None).is_err());
        assert!(remove_locked(&t, "encorefg").is_err());
        assert_eq!(fs::read(t.directory.join("sl.pcl.dll"))?, b"foreign edit");
        fs::copy(&s["sl.pcl.dll"], t.directory.join("sl.pcl.dll"))?;
        remove_locked(&t, "encorefg")?;
        assert!(ps.iter().all(|p| !p.exists()));
        Ok(())
    }
    #[test]
    fn added_fg_rejects_native_integrations_and_rolls_back_each_write() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let t = target(&tmp.path().join("Game"))?;
        for n in [
            "nvngx_dlssg.dll",
            "sl.interposer.dll",
            "libxess_fg.dll",
            "amd_fidelityfx_dx12.dll",
        ] {
            let p = t.directory.join(n);
            fs::write(&p, b"game owned")?;
            assert!(preflight_added_fg(&t, None).is_err(), "{n}");
            assert_eq!(fs::read(&p)?, b"game owned");
            fs::remove_file(p)?;
        }
        let s = sources(&tmp.path().join("sources"), "encorefg")?;
        let ps = preflight_added_fg(&t, None)?;
        fs::write(&ps[0], b"file appeared during download")?;
        assert!(install_locked(&t, "encorefg", &ps, &s).is_err());
        assert_eq!(fs::read(&ps[0])?, b"file appeared during download");
        fs::remove_file(&ps[0])?;
        for boundary in 0..ps.len() {
            let mut writes = 0;
            assert!(
                install_locked_with(&t, "encorefg", &ps, &s, |src, dst| {
                    if writes == boundary {
                        anyhow::bail!("injected write failure");
                    }
                    writes += 1;
                    replace(src, dst)
                })
                .is_err()
            );
            assert!(ps.iter().all(|p| !p.exists()));
            assert!(read(&t, "encorefg")?.is_none());
        }
        Ok(())
    }
    #[test]
    fn added_fg_asi_target_installs_beside_exe_and_limits_cleanup_to_owned_files() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let game = target(&tmp.path().join("Game"))?;
        fs::create_dir_all(game.directory.join("plugins"))?;
        let t = core::DeploymentTarget::custom(&game.game_exe, &game.directory.join("plugins"))
            .validate(true)?;
        let s = sources(&tmp.path().join("sources"), "encorefg")?;
        let ps = preflight_added_fg(&t, None)?;
        assert!(
            ps.iter()
                .all(|p| p.parent() == Some(game.directory.as_path()))
        );
        install_locked(&t, "encorefg", &ps, &s)?;
        fs::write(game.directory.join("sl.dlss.dll"), b"unrelated SR")?;
        remove_locked(&t, "encorefg")?;
        assert_eq!(
            fs::read(game.directory.join("sl.dlss.dll"))?,
            b"unrelated SR"
        );
        assert!(ps.iter().all(|p| !p.exists()));
        Ok(())
    }
    #[test]
    fn patch_cleanup_restores_owned_dlss_but_preserves_external_changes() -> Result<()> {
        for foreign in [false, true] {
            let tmp = tempfile::tempdir()?;
            let t = target(&tmp.path().join("Game"))?;
            for kind in ["sr", "rr", "fg"] {
                let s = sources(&tmp.path().join("sources").join(kind), kind)?;
                let path = t.directory.join(filename(kind)?);
                fs::write(&path, format!("original {kind}"))?;
                install_locked(&t, kind, &[path], &s)?;
            }
            let foreign_path = t.directory.join(filename("sr")?);
            if foreign {
                fs::write(&foreign_path, b"external change")?;
            }
            let result = crate::cleanup::clean_outcome_at(&t);
            if foreign {
                assert!(result.is_err());
                assert_eq!(fs::read(&foreign_path)?, b"external change");
                assert!(
                    read(&t, "sr")?.is_some(),
                    "keep ownership and recovery backup"
                );
            } else {
                assert!(result?.complete);
                for kind in ["sr", "rr", "fg"] {
                    assert_eq!(
                        fs::read(t.directory.join(filename(kind)?))?,
                        format!("original {kind}").as_bytes()
                    );
                    assert!(read(&t, kind)?.is_none());
                }
            }
        }
        Ok(())
    }
    #[test]
    fn mixed_separators_keep_legacy_ownership_and_restore_originals() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let t = target(&tmp.path().join("The Witcher 3/bin/x64_dx12"))?;
        let mixed = core::DeploymentTarget::custom(
            &PathBuf::from(t.game_exe.to_string_lossy().replace("\\bin\\", "\\bin/")),
            &PathBuf::from(t.directory.to_string_lossy().replace("\\bin\\", "\\bin/")),
        );
        assert_ne!(mixed.game_exe.as_os_str(), t.game_exe.as_os_str());
        for kind in ["sr", "rr", "fg", "xemfg"] {
            let s = sources(&tmp.path().join("sources").join(kind), kind)?;
            let dir = if kind == "xemfg" {
                t.directory.join("RTXMFG-XeSS")
            } else {
                t.directory.clone()
            };
            fs::create_dir_all(&dir)?;
            let ps = s.keys().map(|n| dir.join(n)).collect::<Vec<_>>();
            for p in &ps {
                fs::write(p, b"original\0\xff")?;
            }
            install_locked(&t, kind, &ps, &s)?;
            let journal = root(&t, kind)?.join("ownership.json");
            let before = fs::read(&journal)?;
            // This is the issue #15 mismatch: validated installation paths,
            // followed by inspection using the game's mixed-separator address.
            assert!(read(&mixed, kind)?.unwrap().complete);
            assert_eq!(fs::read(&journal)?, before, "inspection must be read-only");
            if kind != "xemfg" {
                let status = inspect(&mixed, kind);
                assert!(status.managed && status.present && status.available);
                assert!(status.warning.is_none(), "{:?}", status.warning);
            }
            // Accept legacy records with mixed separators and case differences
            // in all three path fields, not just the game EXE.
            let mut legacy: Record = serde_json::from_slice(&before)?;
            legacy.game = legacy.game.replace('\\', "/").to_uppercase();
            legacy.directory = legacy.directory.replace('\\', "/").to_uppercase();
            for e in &mut legacy.entries {
                e.path = e.path.replace('\\', "/").to_uppercase();
            }
            fs::write(&journal, serde_json::to_vec(&legacy)?)?;
            assert!(read(&mixed, kind)?.unwrap().complete);
            for p in s.values() {
                fs::write(p, b"second upgrade")?;
            }
            install_locked(&mixed, kind, &ps, &s)?;
            let upgraded = read(&t, kind)?.unwrap();
            assert!(!upgraded.game.contains('/') && !upgraded.directory.contains('/'));
            assert!(upgraded.entries.iter().all(|e| !e.path.contains('/')));
            remove_locked(&mixed, kind)?;
            for p in &ps {
                assert_eq!(fs::read(p)?, b"original\0\xff");
            }
            assert!(!journal.exists());
        }
        Ok(())
    }
    #[test]
    fn normalized_ownership_still_rejects_foreign_paths_and_duplicate_files() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let t = target(&tmp.path().join("Game"))?;
        let p = t.directory.join("nvngx_dlss.dll");
        fs::write(&p, b"original")?;
        let s = sources(&tmp.path().join("source"), "sr")?;
        install_locked(&t, "sr", std::slice::from_ref(&p), &s)?;
        let journal = root(&t, "sr")?.join("ownership.json");
        let original = fs::read(&journal)?;
        let dll = fs::read(&p)?;
        let mut cases = Vec::new();
        for (field, value) in [
            ("game", t.directory.join("Other.exe").display().to_string()),
            (
                "directory",
                tmp.path().join("Game-other").display().to_string(),
            ),
            ("game", "Game.exe".into()),
        ] {
            let mut r: serde_json::Value = serde_json::from_slice(&original)?;
            r[field] = value.into();
            cases.push(r);
        }
        let mut escaped: serde_json::Value = serde_json::from_slice(&original)?;
        escaped["entries"][0]["path"] = tmp
            .path()
            .join("Game-other/nvngx_dlss.dll")
            .display()
            .to_string()
            .into();
        cases.push(escaped);
        let mut duplicate: serde_json::Value = serde_json::from_slice(&original)?;
        let mut entry = duplicate["entries"][0].clone();
        entry["path"] = p.to_string_lossy().replace('\\', "/").to_uppercase().into();
        duplicate["entries"].as_array_mut().unwrap().push(entry);
        cases.push(duplicate);
        for r in cases {
            let bytes = serde_json::to_vec(&r)?;
            fs::write(&journal, &bytes)?;
            assert!(read(&t, "sr").is_err());
            assert!(remove_locked(&t, "sr").is_err());
            assert_eq!(fs::read(&p)?, dll);
            assert_eq!(fs::read(&journal)?, bytes);
        }
        fs::write(&journal, original)?;
        remove_locked(&t, "sr")?;
        assert_eq!(fs::read(&p)?, b"original");
        Ok(())
    }
    #[test]
    fn xess_status_requires_both_dlls_not_just_a_license_or_one_file() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let paths: Vec<_> = ["libxess_fg.dll", "libxell.dll", "LICENSE.txt"]
            .iter()
            .map(|name| tmp.path().join(name))
            .collect();
        fs::write(&paths[2], b"license")?;
        assert!(!dll_set_present("xemfg", &paths));
        fs::write(&paths[1], b"synthetic XeLL")?;
        assert!(!dll_set_present("xemfg", &paths));
        fs::write(&paths[0], b"synthetic XeFG")?;
        assert!(dll_set_present("xemfg", &paths));
        fs::remove_file(&paths[1])?;
        assert!(!dll_set_present("xemfg", &paths));
        Ok(())
    }
    #[test]
    fn private_patch_files_are_excluded_even_without_a_journal() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let t = target(&temp.path().join("Game"))?;
        for folder in [
            "dlssg_sm86/sr",
            "dlssg_sm86/runtime",
            "rtx-encore-runtimes/intel",
            "RTXMFG-XeSS",
            "OptiScaler",
            "Backup",
        ] {
            let dir = t.directory.join(folder);
            fs::create_dir_all(&dir)?;
            fs::write(dir.join("nvngx_dlss.dll"), b"private dependency")?;
        }
        assert!(!scanner::inspect(&t.game_exe).super_resolution);
        assert!(paths(&t, "sr").is_err());
        let game_file = t
            .directory
            .join("Engine/Plugins/NVIDIA/DLSS/nvngx_dlss.dll");
        fs::create_dir_all(game_file.parent().unwrap())?;
        fs::write(&game_file, b"game SR")?;
        assert_eq!(paths(&t, "sr")?, vec![game_file]);
        let private = t.directory.join("dlssg_sm86/sr/nvngx_dlss.dll");
        assert!(
            plan_entries(
                &t,
                "sr",
                &[private],
                &sources(&temp.path().join("source"), "sr")?
            )
            .is_err()
        );
        Ok(())
    }
    #[test]
    fn broken_patch_ownership_fails_closed_before_writing() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let t = target(&temp.path().join("Game"))?;
        fs::write(t.directory.join("nvngx_dlss.dll"), b"preserve")?;
        fs::create_dir_all(t.directory.join(core::OWN))?;
        fs::write(
            t.directory.join(core::OWN).join(core::MARKER),
            b"invalid record",
        )?;
        assert!(paths(&t, "sr").is_err());
        assert_eq!(fs::read(t.directory.join("nvngx_dlss.dll"))?, b"preserve");
        Ok(())
    }
    #[test]
    fn upgrade_keeps_first_original_and_restores_exact_bytes() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let t = target(&tmp.path().join("Game"))?;
        let p = t.directory.join("nvngx_dlss.dll");
        fs::write(&p, b"original\0\xff")?;
        let s = sources(&tmp.path().join("source"), "sr")?;
        install_locked(&t, "sr", std::slice::from_ref(&p), &s)?;
        fs::write(&s["nvngx_dlss.dll"], b"next runtime")?;
        install_locked(&t, "sr", std::slice::from_ref(&p), &s)?;
        assert!(read(&t, "sr")?.unwrap().complete);
        remove_locked(&t, "sr")?;
        assert_eq!(fs::read(&p)?, b"original\0\xff");
        assert!(!root(&t, "sr")?.exists());
        Ok(())
    }
    #[test]
    fn unknown_version_is_rejected_before_any_game_or_network_access() {
        assert!(
            install_version(
                &[],
                "sr",
                "999.0.0",
                false,
                &AtomicBool::new(false),
                &mut |_| panic!("no download")
            )
            .is_err()
        );
        assert_eq!(versions("xemfg"), vec!["3.0.2"]);
        assert!(version_package("nr", "310.9.16").is_err());
    }
    #[test]
    fn later_edits_and_foreign_game_records_are_not_overwritten() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let t = target(&tmp.path().join("Game"))?;
        let p = t.directory.join("nvngx_dlssd.dll");
        fs::write(&p, b"original")?;
        let s = sources(&tmp.path().join("source"), "rr")?;
        install_locked(&t, "rr", std::slice::from_ref(&p), &s)?;
        fs::write(&p, b"game updater changed this")?;
        assert!(remove_locked(&t, "rr").is_err());
        assert!(install_locked(&t, "rr", std::slice::from_ref(&p), &s).is_err());
        assert_eq!(fs::read(&p)?, b"game updater changed this");
        let other = core::DeploymentTarget::custom(&t.directory.join("Other.exe"), &t.directory);
        assert!(read(&other, "rr").is_err());
        assert!(root(&t, "rr")?.join("ownership.json").is_file());
        Ok(())
    }
    #[test]
    fn paired_xess_failure_restores_original_and_removes_only_owned_additions() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let t = target(&tmp.path().join("Game"))?;
        let dir = t.directory.join("RTXMFG-XeSS");
        fs::create_dir_all(&dir)?;
        fs::write(dir.join("libxell.dll"), b"original xell")?;
        fs::write(dir.join("user-note.txt"), b"keep")?;
        let s = sources(&tmp.path().join("source"), "xemfg")?;
        let ps = vec![
            dir.join("libxess_fg.dll"),
            dir.join("libxell.dll"),
            dir.join("LICENSE.txt"),
        ];
        let mut calls = 0;
        let result = install_locked_with(&t, "xemfg", &ps, &s, |from, to| {
            calls += 1;
            ensure!(calls < 2, "injected write failure");
            replace(from, to)
        });
        assert!(result.is_err());
        assert_eq!(calls, 2);
        assert!(!ps[0].exists());
        assert!(!ps[2].exists());
        assert_eq!(fs::read(&ps[1])?, b"original xell");
        assert_eq!(fs::read(dir.join("user-note.txt"))?, b"keep");
        assert!(read(&t, "xemfg")?.is_none());
        Ok(())
    }
    #[test]
    fn tamper_during_write_retains_recovery_journal_and_player_file() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let t = target(&tmp.path().join("Game"))?;
        let p = t.directory.join("nvngx_dlssg.dll");
        fs::write(&p, b"original")?;
        let s = sources(&tmp.path().join("source"), "fg")?;
        let r = install_locked_with(&t, "fg", std::slice::from_ref(&p), &s, |_, to| {
            fs::write(to, b"external concurrent change")?;
            anyhow::bail!("injected failure")
        });
        assert!(r.is_err());
        assert_eq!(fs::read(&p)?, b"external concurrent change");
        assert!(!read(&t, "fg")?.unwrap().complete);
        assert!(inspect(&t, "fg").warning.is_some());
        Ok(())
    }
    #[test]
    fn all_directories_are_updated_once_and_conflict_is_preflighted() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let a = target(&tmp.path().join("A"))?;
        let b = target(&tmp.path().join("B"))?;
        for t in [&a, &b] {
            fs::write(t.directory.join("nvngx_dlss.dll"), b"original")?;
        }
        let s = sources(&tmp.path().join("source"), "sr")?;
        let mut calls = 0;
        install_many_with(
            &[b.clone(), a.clone(), a.clone()],
            "sr",
            &AtomicBool::new(false),
            || {
                calls += 1;
                Ok(s.clone())
            },
        )?;
        assert_eq!(calls, 1);
        for t in [&a, &b] {
            assert!(read(t, "sr")?.unwrap().complete);
        }
        let old = fs::read(a.directory.join("nvngx_dlss.dll"))?;
        fs::write(b.directory.join("nvngx_dlss.dll"), b"external edit")?;
        fs::write(&s["nvngx_dlss.dll"], b"newer runtime")?;
        assert!(
            install_many_with(&[a.clone(), b], "sr", &AtomicBool::new(false), || Ok(s)).is_err()
        );
        assert_eq!(fs::read(a.directory.join("nvngx_dlss.dll"))?, old);
        Ok(())
    }
    #[test]
    fn escaping_path_or_missing_original_backup_blocks_restore() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let t = target(&tmp.path().join("Game"))?;
        let p = t.directory.join("nvngx_dlss.dll");
        fs::write(&p, b"original")?;
        let s = sources(&tmp.path().join("source"), "sr")?;
        install_locked(&t, "sr", std::slice::from_ref(&p), &s)?;
        let mut r = read(&t, "sr")?.unwrap();
        fs::remove_file(
            root(&t, "sr")?.join(format!("{}.bak", r.entries[0].original.as_ref().unwrap())),
        )?;
        assert!(remove_locked(&t, "sr").is_err());
        assert_eq!(fs::read(&p)?, fs::read(&s["nvngx_dlss.dll"])?);
        r.entries[0].path = tmp.path().join("nvngx_dlss.dll").display().to_string();
        core::atomic_json(
            &root(&t, "sr")?.join("ownership.json"),
            &serde_json::to_value(r)?,
        )?;
        assert!(read(&t, "sr").is_err());
        Ok(())
    }
}
