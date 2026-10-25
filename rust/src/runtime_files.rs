//! Opt-in runtime updates. Existing game files are backed up; later edits win.
//! No Streamline replacement, arbitrary path input, or DLL execution.
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
pub const KINDS: [&str; 4] = ["sr", "rr", "fg", "xemfg"];
pub fn label(kind: &str) -> &'static str {
    match kind {
        "sr" => "DLSS 超分辨率",
        "rr" => "DLSS 光线重建",
        "fg" => "DLSS 帧生成",
        "xemfg" => "XeSS 多帧生成",
        _ => "运行库",
    }
}
fn filename(kind: &str) -> Result<&'static str> {
    Ok(match kind {
        "sr" => "nvngx_dlss.dll",
        "rr" => "nvngx_dlssd.dll",
        "fg" => "nvngx_dlssg.dll",
        "xemfg" => "libxess_fg.dll",
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
fn prepare(
    kind: &str,
    github: bool,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(transfer::Progress),
) -> Result<BTreeMap<String, PathBuf>> {
    let p = package(kind)?;
    prepare_package(p, github, cancel, progress)
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
    pub warning: Option<String>,
}
fn paths(target: &core::DeploymentTarget, kind: &str) -> Result<Vec<PathBuf>> {
    paths_from(target, kind, None)
}
// Patch dependencies and game runtimes are different ownership domains. Check
// names even for manual installations, then consult journals for custom paths.
fn protected(target: &core::DeploymentTarget, root: &Path, path: &Path) -> Result<bool> {
    let path = core::no_links(path)?;
    ensure!(path.starts_with(root), "运行库路径超出游戏目录");
    if path
        .strip_prefix(root)?
        .components()
        .any(|c| scanner::patch_runtime_directory(&c.as_os_str().to_string_lossy()))
    {
        return Ok(true);
    }
    let mut owners = path
        .ancestors()
        .skip(1)
        .take_while(|p| p.starts_with(root))
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
    if kind == "xemfg" {
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
        return Ok(["libxess_fg.dll", "libxell.dll", "LICENSE.txt"]
            .iter()
            .map(|n| target.directory.join("RTXMFG-XeSS").join(n))
            .collect());
    }
    let name = filename(kind)?;
    let root = core::no_links(&scanner::installation_root(&target.game_exe))?;
    let mut found = evidence
        .map(|e| e.component_paths.clone())
        .unwrap_or_else(|| scanner::inspect(&target.game_exe).component_paths);
    let local = target.directory.join(name);
    if local.is_file() {
        found.push(local);
    }
    found.retain(|p| {
        p.starts_with(&root) && p.file_name().is_some_and(|n| n.eq_ignore_ascii_case(name))
    });
    let mut eligible = Vec::new();
    for p in found {
        if !protected(target, &root, &p)? {
            eligible.push(p);
        }
    }
    let mut found = eligible;
    found.sort_by_key(|p| core::key(p));
    found.dedup_by_key(|p| core::key(p));
    ensure!(
        !found.is_empty(),
        "没有可更新的游戏运行库；补丁专用文件不在此处更新"
    );
    Ok(found)
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
fn destination(target: &core::DeploymentTarget, kind: &str, entry: &Entry) -> Result<PathBuf> {
    let p = core::no_links(Path::new(&entry.path))?;
    let valid_name = p.file_name().is_some_and(|n| {
        if kind == "xemfg" {
            ["libxess_fg.dll", "libxell.dll", "LICENSE.txt"]
                .iter()
                .any(|s| n.eq_ignore_ascii_case(s))
        } else {
            n.eq_ignore_ascii_case(filename(kind).unwrap_or(""))
        }
    });
    ensure!(
        valid_name
            && if kind == "xemfg" {
                p.parent() == Some(target.directory.join("RTXMFG-XeSS").as_path())
            } else {
                p.starts_with(core::no_links(&scanner::installation_root(
                    &target.game_exe,
                ))?)
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
            && r.game == target.game_exe.to_string_lossy()
            && r.directory == target.directory.to_string_lossy()
            && r.kind == kind
            && !r.entries.is_empty()
            && r.entries.len() <= 32,
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
        for p in &s.paths {
            if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("dll")) && p.is_file() {
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
    if kind == "xemfg" {
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
            path: p.to_string_lossy().into(),
            installed: core::digest(source)?,
            previous: before.clone(),
            original: existing.map(|e| e.original.clone()).unwrap_or(before),
        });
    }
    // Keep prior paths if a game update changed which folder the scanner finds.
    if let Some(prior) = prior {
        for e in prior.entries {
            if !entries
                .iter()
                .any(|n| core::key(Path::new(&n.path)) == core::key(Path::new(&e.path)))
            {
                entries.push(e);
            }
        }
    }
    ensure!(entries.len() <= 32, "运行库文件数量超出限制");
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
        game: target.game_exe.to_string_lossy().into(),
        directory: target.directory.to_string_lossy().into(),
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
    install_many_with(targets, kind, cancel, || {
        prepare(kind, github, cancel, progress)
    })
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
    install_many_with(targets, kind, cancel, || {
        prepare_package(selected, github, cancel, progress)
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
        for kind in KINDS {
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
    fn sources(dir: &Path, kind: &str) -> Result<BTreeMap<String, PathBuf>> {
        fs::create_dir_all(dir)?;
        let names = if kind == "xemfg" {
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
