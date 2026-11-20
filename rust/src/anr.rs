//! ANR FG full-package integration. No scripts are executed and no existing
//! OptiScaler installation is adopted or overwritten.
use crate::{
    core, diagnostics,
    presets::{Parameter, Values},
};
use anyhow::{Context, Result, ensure};
use object::{LittleEndian as LE, pe, read::pe::PeFile64};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{ErrorKind, Read},
    path::{Path, PathBuf},
};

pub const BACKEND: &str = "anr";
// B15 through B20 use the same editable settings, even as model files change.
pub const PROFILE: &str = "anr_fg_b15";
pub const SCHEME: &str = "anr-fg";
pub const SOURCE: &str = "https://github.com/apex-compute/ANR";
pub const CONFIG: &str = "OptiScaler/custom/Apex Neural Rendering FG/dlssg/nr_fg.ini";
pub const PROVIDER_DIR: &str = "OptiScaler/custom/Apex Neural Rendering FG/dlssg";
const LOADER: &str = "anr-vkloader.dll";
pub const RESIDUE_STATUS: &str = "ANR 运行库残留";
pub const PROXIES: [&str; 3] = ["dxgi.dll", "winmm.dll", "version.dll"];

/// Display the upstream beta label without leaking our sortable transport version.
pub fn display_version(version: &str) -> String {
    match semver::Version::parse(version) {
        Ok(v)
            if v.major == 0
                && v.minor == 0
                && v.pre.as_str() == format!("B{}", v.patch)
                && v.build.is_empty() =>
        {
            format!("V.B{}", v.patch)
        }
        _ => version.to_owned(),
    }
}
pub const COMPONENTS: [&str; 32] = [
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
    "OptiScaler/custom/Apex Neural Rendering FG/dlssg/programs/fg-lite-3840x2160-coopmat-nvidia-blackwell-pq10/pack.nrx",
    "OptiScaler/custom/Apex Neural Rendering FG/dlssg/programs/fg-lite-3840x2160-coopmat-nvidia-blackwell-scrgb16/pack.nrx",
    "OptiScaler/custom/Apex Neural Rendering FG/dlssg/programs/fg-lite-3840x2160-coopmat-nvidia-blackwell-srgb8/pack.nrx",
    "OptiScaler/custom/Apex Neural Rendering FG/dlssg/programs/fg-lite-any-coopmat-nvidia-blackwell-pq10/pack.nrx",
    "OptiScaler/custom/Apex Neural Rendering FG/dlssg/programs/fg-lite-any-coopmat-nvidia-blackwell-scrgb16/pack.nrx",
    "OptiScaler/custom/Apex Neural Rendering FG/dlssg/programs/fg-lite-any-coopmat-nvidia-blackwell-srgb8/pack.nrx",
    "OptiScaler/custom/Apex Neural Rendering FG/dlssg/programs/fg-lite-any-coopmat-scrgb16/pack.nrx",
    "OptiScaler/custom/Apex Neural Rendering FG/dlssg/programs/fg-lite-any-dp4a-scrgb16/pack.nrx",
    "OptiScaler/custom/Apex Neural Rendering FG/dlssg/programs/fg-lite-any-packed-scrgb16/pack.nrx",
    "OptiScaler/custom/Apex Neural Rendering FG/dlssg/programs/fg-lite-any-portable-linalg-scrgb16/pack.nrx",
    "OptiScaler/custom/Apex Neural Rendering FG/dlssg/programs/fg-lite-any-portable-scrgb16/pack.nrx",
];

/// Use the inventory of the recorded release when validating an old installation.
pub fn components(version: Option<&str>) -> &'static [&'static str] {
    if version
        .and_then(|v| semver::Version::parse(v).ok())
        .is_some_and(|v| v.major == 0 && v.minor == 0 && v.patch >= 20)
    {
        &COMPONENTS
    } else {
        &COMPONENTS[..21]
    }
}

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
pub(crate) fn loader_path(directory: &Path) -> PathBuf {
    directory.join(PROVIDER_DIR).join(LOADER)
}

// ANR copies the system Vulkan loader here on first use. Check the original
// module name and actual PE exports so driver updates do not break recognition.
fn loader_digest(path: &Path) -> Result<Option<String>> {
    const LIMIT: u64 = 32 * 1024 * 1024;
    let path = core::no_links(path)?;
    if !path.try_exists()? || !path.is_file() || path.metadata()?.len() > LIMIT {
        return Ok(None);
    }
    let mut bytes = Vec::new();
    fs::File::open(&path)
        .and_then(|f| f.take(LIMIT + 1).read_to_end(&mut bytes))
        .with_context(|| format!("无法读取 ANR 运行库：{}", path.display()))?;
    if bytes.len() as u64 > LIMIT {
        return Ok(None);
    }
    let Ok(pe) = PeFile64::parse(bytes.as_slice()) else {
        return Ok(None);
    };
    let header = &pe.nt_headers().file_header;
    if header.machine.get(LE) != pe::IMAGE_FILE_MACHINE_AMD64
        || header.characteristics.get(LE) & pe::IMAGE_FILE_DLL == 0
    {
        return Ok(None);
    }
    let Ok(Some(exports)) = pe.export_table() else {
        return Ok(None);
    };
    let dir = exports.directory();
    if !exports
        .name_from_pointer(dir.name.get(LE))
        .is_ok_and(|name| name.eq_ignore_ascii_case(b"vulkan-1.dll"))
        || dir.number_of_names.get(LE) > 4096
    {
        return Ok(None);
    }
    let mut instance = false;
    let mut device = false;
    for (pointer, _) in exports.name_iter() {
        match exports.name_from_pointer(pointer) {
            Ok(b"vkGetInstanceProcAddr") => instance = true,
            Ok(b"vkGetDeviceProcAddr") => device = true,
            _ => {}
        }
        if instance && device {
            return Ok(Some(core::hash(&bytes)));
        }
    }
    Ok(None)
}

enum HostTree {
    Empty,
    Residue(String),
    Occupied,
}

// Without an install record, accept only the exact, otherwise empty ANR tree.
// Stop at the first sibling, unknown file or link. Never recurse into plugins.
fn host_tree(directory: &Path) -> Result<HostTree> {
    let rel = Path::new(PROVIDER_DIR).join(LOADER);
    let mut parts = rel.iter();
    let mut path = directory.join(parts.next().expect("fixed OptiScaler path"));
    for part in parts {
        match fs::symlink_metadata(&path) {
            Ok(m) if m.is_dir() && !core::is_link(&m) => {}
            Ok(_) => return Ok(HostTree::Occupied),
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(HostTree::Empty),
            Err(e) => return Err(e.into()),
        }
        let mut entries = fs::read_dir(&path)?;
        let Some(entry) = entries.next().transpose()? else {
            return Ok(HostTree::Empty);
        };
        let name = entry.file_name();
        if !(name == part || cfg!(windows) && name.eq_ignore_ascii_case(part))
            || entries.next().transpose()?.is_some()
        {
            return Ok(HostTree::Occupied);
        }
        path.push(name);
    }
    if core::is_link(&fs::symlink_metadata(&path)?) {
        return Ok(HostTree::Occupied);
    }
    Ok(loader_digest(&path)?.map_or(HostTree::Occupied, HostTree::Residue))
}

fn other_host_file(directory: &Path) -> Result<Option<&'static str>> {
    for name in [
        "OptiScaler.ini",
        "OptiScaler.dll",
        "OptiScaler.asi",
        "OptiScaler.log",
    ] {
        if core::no_links(&directory.join(name))?.try_exists()? {
            return Ok(Some(name));
        }
    }
    for name in core::PROXIES {
        let path = core::no_links(&directory.join(name))?;
        if path.is_file()
            && let Ok((_, internal)) = crate::win::file_version(&path)
            && internal.to_ascii_lowercase().contains("optiscaler")
        {
            return Ok(Some(name));
        }
    }
    Ok(None)
}

pub(crate) fn legacy_loader(directory: &Path) -> Result<Option<String>> {
    if loader_path(directory).try_exists()?
        && let HostTree::Residue(hash) = host_tree(directory)?
        && other_host_file(directory)?.is_none()
    {
        return Ok(Some(hash));
    }
    Ok(None)
}

pub(crate) fn remove_legacy_loader(directory: &Path, expected: &str) -> Result<()> {
    let directory = core::no_links(directory)?;
    ensure!(
        legacy_loader(&directory)?.as_deref() == Some(expected),
        "ANR 目录已变化，请刷新后重试"
    );
    let path = core::no_links(&loader_path(&directory))?;
    fs::remove_file(&path)
        .with_context(|| format!("请关闭占用 ANR 运行库的程序后重试：{}", path.display()))?;
    for parent in path
        .ancestors()
        .skip(1)
        .take_while(|p| *p != directory.as_path())
    {
        let parent = core::no_links(parent)?;
        match fs::remove_dir(parent) {
            Ok(()) => {}
            Err(e) if e.kind() == ErrorKind::NotFound => {}
            Err(e) if e.kind() == ErrorKind::DirectoryNotEmpty => break,
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

pub fn preflight(target: &core::DeploymentTarget, managed: bool) -> Result<()> {
    install_residue(target, managed).map(|_| ())
}

/// Capture the confirmed old loader before staging. Installation revalidates it
/// under the game lock after checking all other file conflicts.
pub(crate) fn install_residue(
    target: &core::DeploymentTarget,
    managed: bool,
) -> Result<Option<String>> {
    ensure!(
        target.game_exe.parent() == Some(target.directory.as_path()),
        "ANR 完整方案请安装到游戏 EXE 所在目录"
    );
    if managed {
        return Ok(None);
    }
    let dir = &target.directory;
    let tree = host_tree(dir)?;
    let conflict = if matches!(tree, HostTree::Occupied) {
        Some("OptiScaler")
    } else {
        other_host_file(dir)?
    };
    if let Some(name) = conflict {
        anyhow::bail!(
            "检测到已有 {name}。ANR 完整包包含 OptiScaler，请先用原安装方式还原；已有 OptiScaler 的游戏应使用上游 addon，管理器不会覆盖原文件"
        );
    }
    Ok(match tree {
        HostTree::Residue(hash) => Some(hash),
        _ => None,
    })
}

pub(crate) fn collect_loader(
    directory: &Path,
    targets: &mut BTreeMap<PathBuf, String>,
    kept: &mut BTreeSet<PathBuf>,
) -> Result<()> {
    let path = loader_path(directory);
    if let Some(hash) = loader_digest(&path)? {
        targets.insert(path, hash);
    } else if path.try_exists()? {
        kept.insert(path);
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
    // The full package documents the provider directory, while the B20
    // release also names its parent as a log location.
    for log_dir in [
        provider.as_path(),
        provider.parent().expect("fixed provider path"),
    ] {
        if !log_dir.is_dir() {
            continue;
        }
        for entry in fs::read_dir(log_dir)? {
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
    fn beta_labels_match_their_transport_versions() {
        assert_eq!(display_version("0.0.15-B15"), "V.B15");
        assert_eq!(display_version("0.0.16-B16"), "V.B16");
        assert_eq!(display_version("0.0.16-B15"), "0.0.16-B15");
        assert_eq!(display_version("1.0.0"), "1.0.0");
    }
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
