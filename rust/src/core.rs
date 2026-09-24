use crate::{assets, win};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};
pub const OWN: &str = ".rtx-fg-v3";
pub const MARKER: &str = "install.json";
pub const INI: &str = "dlssg_sm86.ini";
pub const PROXIES: [&str; 7] = [
    "version.dll",
    "winmm.dll",
    "dinput8.dll",
    "winhttp.dll",
    "dxgi.dll",
    "dbghelp.dll",
    "d3d12.dll",
];
pub const BACKENDS: [&str; 7] = [
    "native20",
    "native30",
    "native_x6_20",
    "native_x6_30",
    "rtx20",
    "rtx30",
    "upstream_sm86",
];
pub const SCHEMES: [&str; 3] = [
    "0.2.6 · DX12/Vulkan（正式）",
    "0.2.6 · 5X/6X · DX12/Vulkan（实验）",
    "初始方案 · GitHub 第一版",
];
pub const LEGACY: [(&str, &str); 2] = [
    (
        "14e10c5d16bf41372985f2e2b30e55b74f9cf8f848dadca26cc25d15eb825f4f",
        "106b4877495b6329bf2a085b7c01268a812e45996a140a3bafa27b8a19e8fb45",
    ),
    (
        "2a2bf3176c3f7f12c923828bbacb1335dc93d66173c38a7073acdffe89dea229",
        "3040ae0e8f6625c0ca541db64d3b4b6c4e38cf21768eb73897bc23971fcf0378",
    ),
];
pub fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub fn digest(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0; 65536];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}
pub fn valid_hash(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
pub fn key(p: &Path) -> String {
    p.to_string_lossy().replace('/', "\\").to_lowercase()
}
pub fn within(p: &Path, root: &Path) -> bool {
    let p = key(p);
    let r = key(root).trim_end_matches('\\').to_owned();
    p == r || p.starts_with(&(r + "\\"))
}
pub fn is_link(meta: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    meta.file_type().is_symlink() || meta.file_attributes() & 0x400 != 0
}
pub fn no_links(path: &Path) -> Result<PathBuf> {
    let p = std::path::absolute(path)?;
    for q in p.ancestors() {
        match fs::symlink_metadata(q) {
            Ok(m) => ensure!(!is_link(&m), "拒绝操作链接或目录联接：{}", q.display()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(p)
}
fn pe_machine(path: &Path, dll: bool) -> Result<u16> {
    let mut file = File::open(path)?;
    let size = file.metadata()?.len();
    let mut h = [0; 64];
    file.read_exact(&mut h).context("EXE 文件头不完整")?;
    ensure!(&h[..2] == b"MZ", "不是 Windows EXE 文件");
    let off = u32::from_le_bytes(h[60..64].try_into()?) as u64;
    ensure!(
        off.checked_add(24).is_some_and(|n| n <= size),
        "EXE 文件头不完整"
    );
    file.seek(SeekFrom::Start(off))?;
    let mut h = [0; 24];
    file.read_exact(&mut h)?;
    ensure!(&h[..4] == b"PE\0\0", "不是 Windows EXE 文件");
    ensure!(
        (u16::from_le_bytes([h[22], h[23]]) & 0x2000 != 0) == dll,
        "EXE / DLL 类型不匹配"
    );
    Ok(u16::from_le_bytes([h[4], h[5]]))
}
pub fn pe64(path: &Path, dll: bool) -> Result<()> {
    ensure!(
        pe_machine(path, dll)? == 0x8664,
        "此补丁仅支持 Windows x64（64 位）程序，当前文件架构不匹配，无法安装补丁。请确认选择的是游戏本体 EXE。"
    );
    Ok(())
}
/// Adding an EXE to the library does not imply it supports the x64 payload.
pub fn library_location(exe: &Path) -> Result<PathBuf> {
    let p = location(exe, false)?;
    pe_machine(&p, false)?;
    Ok(p)
}
pub fn location(exe: &Path, must_exist: bool) -> Result<PathBuf> {
    let p = no_links(exe)?;
    let s = p.to_string_lossy();
    ensure!(
        p.extension().is_some_and(|x| x.eq_ignore_ascii_case("exe")),
        "请选择游戏本体 EXE"
    );
    ensure!(
        !s.starts_with("\\\\") && !s.get(2..).unwrap_or_default().contains(':'),
        "不支持网络路径或备用数据流"
    );
    let windows = PathBuf::from(std::env::var_os("WINDIR").unwrap_or_else(|| "C:\\Windows".into()));
    ensure!(!within(&p, &windows), "不能部署到 Windows 系统目录");
    ensure!(
        key(&p) != key(&std::env::current_exe()?) && !within(&p, &assets::cache_root()?),
        "不能把管理器或随包运行环境当作游戏部署"
    );
    ensure!(p.parent().is_some_and(Path::is_dir), "游戏目录不存在");
    if must_exist {
        pe64(&p, false)?;
    }
    Ok(p)
}
pub fn normalize_proxies(proxies: &[String]) -> Result<Vec<String>> {
    ensure!(
        !proxies.is_empty() && proxies.len() <= 7,
        "DLL 入口选择无效"
    );
    ensure!(
        proxies.iter().all(|n| PROXIES.contains(&n.as_str()))
            && proxies
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len()
                == proxies.len(),
        "DLL 入口选择无效"
    );
    Ok(PROXIES
        .iter()
        .filter(|p| proxies.iter().any(|n| n == **p))
        .map(|s| (*s).into())
        .collect())
}
pub fn folder(backend: &str) -> Result<&str> {
    ensure!(
        BACKENDS.contains(&backend) || backend == "manual",
        "未知显卡方案"
    );
    Ok(if backend.starts_with("native_x6") {
        "native-x6"
    } else if backend.starts_with("native") {
        "native"
    } else {
        backend
    })
}
pub fn deployment_names(backend: &str, proxies: &[String]) -> Result<Vec<String>> {
    folder(backend)?;
    let mut names = normalize_proxies(proxies)?;
    ensure!(
        !backend.starts_with("native")
            || names
                .iter()
                .all(|n| !["dbghelp.dll", "d3d12.dll"].contains(&n.as_str())),
        "此方案没有所选 DLL 入口"
    );
    ensure!(
        backend != "upstream_sm86" || !names.iter().any(|n| n == "winhttp.dll"),
        "上游方案不提供 winhttp.dll 入口"
    );
    ensure!(
        backend.starts_with("native")
            || backend == "upstream_sm86"
            || backend == "manual"
            || names == ["version.dll"],
        "备用 DLL 入口需使用 Native 方案，现有 R2 不能仅改文件名"
    );
    names.push(INI.into());
    Ok(names)
}
pub fn package(backend: &str, proxies: &[String]) -> Result<BTreeMap<String, Vec<u8>>> {
    let root = folder(backend)?;
    let mut out = BTreeMap::new();
    for name in deployment_names(backend, proxies)? {
        let bytes = assets::bytes(&format!("payloads/{root}/{name}"))?;
        out.insert(name, bytes);
    }
    configure_package(backend, &mut out)?;
    Ok(out)
}
pub fn configure_package(backend: &str, out: &mut BTreeMap<String, Vec<u8>>) -> Result<()> {
    // Bundled upstream runtimes default to the user cache. Moving their extracted
    // components next to a game changes DLL loading behavior (reported by ZZZ).
    if backend == "upstream_sm86" {
        return Ok(());
    }
    let mut bytes = out[INI].clone();
    let sections = crate::cleanup::parse_ini(std::str::from_utf8(&bytes)?);
    if sections.contains_key("Runtime") {
        bytes = crate::diagnostics::edit_ini(
            &bytes,
            "Runtime",
            "CacheDirectory",
            &format!("{OWN}\\cache"),
        )?;
    }
    if backend.starts_with("native") {
        bytes = crate::diagnostics::edit_ini(
            &bytes,
            "Compatibility",
            "Router",
            if backend.ends_with("20") {
                "SM75"
            } else {
                "SM86"
            },
        )?;
    }
    bytes = crate::diagnostics::edit_ini(&bytes, "Logging", "Directory", &format!("{OWN}\\logs"))?;
    out.insert(INI.into(), bytes);
    Ok(())
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct UpstreamOptions {
    pub optimized: bool,
    pub max_generated_frames: u8,
    pub preset: String,
    pub logging_level: u8,
}
impl Default for UpstreamOptions {
    fn default() -> Self {
        Self {
            optimized: true,
            max_generated_frames: 3,
            preset: "Auto".into(),
            logging_level: 1,
        }
    }
}
impl UpstreamOptions {
    pub fn validate(&self) -> Result<()> {
        ensure!(self.max_generated_frames <= 5, "帧生成倍率上限无效");
        ensure!(
            matches!(self.preset.as_str(), "Auto" | "A" | "B"),
            "DLSSG 预设无效"
        );
        ensure!(self.logging_level <= 3, "日志级别无效");
        Ok(())
    }
}
pub fn configure_upstream_ini(bytes: &[u8], options: &UpstreamOptions) -> Result<Vec<u8>> {
    options.validate()?;
    let edits = [
        (
            "FrameGeneration",
            "Optimized",
            if options.optimized { "1" } else { "0" }.to_string(),
        ),
        (
            "FrameGeneration",
            "MaxGeneratedFrames",
            options.max_generated_frames.to_string(),
        ),
        ("Compatibility", "Preset", options.preset.clone()),
        ("Logging", "Level", options.logging_level.to_string()),
    ];
    let mut output = bytes.to_vec();
    for (section, key, value) in edits {
        output = crate::diagnostics::edit_ini(&output, section, key, &value)?;
    }
    Ok(output)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Record {
    pub schema: u32,
    pub backend: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload_version: Option<String>,
    #[serde(default = "default_proxy")]
    pub proxy: String,
    #[serde(default)]
    pub proxies: Vec<String>,
    pub hashes: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub cleanup_dirs: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scheme_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub delta_cache_ids: Vec<String>,
    #[serde(default)]
    pub delta_legacy_cache: bool,
    #[serde(default)]
    pub cache_pending: bool,
}
fn default_proxy() -> String {
    "version.dll".into()
}
impl Record {
    pub fn selected(&self) -> Vec<String> {
        if self.proxies.is_empty() {
            vec![self.proxy.clone()]
        } else {
            self.proxies.clone()
        }
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(self.schema == 3, "部署记录无效");
        ensure!(
            self.delta_cache_ids.len() <= 8
                && self
                    .delta_cache_ids
                    .iter()
                    .all(|id| crate::delta::valid_id(id)),
            "组件缓存记录无效"
        );
        ensure!(
            self.scheme_id
                .as_ref()
                .is_none_or(|id| !id.is_empty() && id.len() <= 150),
            "方案记录无效"
        );
        ensure!(
            self.payload_version
                .as_ref()
                .is_none_or(|v| crate::updater::version(v).is_ok()),
            "部署记录无效"
        );
        let p = normalize_proxies(&self.selected())?;
        ensure!(self.proxy == p[0], "部署记录的 DLL 入口不一致");
        let names = deployment_names(&self.backend, &p)?;
        ensure!(
            self.hashes.len() == names.len()
                && names
                    .iter()
                    .all(|n| self.hashes.get(n).is_some_and(|h| valid_hash(h))),
            "部署记录的 DLL 文件范围无效"
        );
        Ok(())
    }
}
pub fn read_json(path: &Path, max: u64) -> Result<serde_json::Value> {
    no_links(path)?;
    let file = File::open(path)?;
    ensure!(file.metadata()?.len() <= max, "文件超过大小限制");
    let mut data = Vec::new();
    file.take(max + 1).read_to_end(&mut data)?;
    ensure!(data.len() as u64 <= max, "文件超过大小限制");
    Ok(serde_json::from_slice(
        data.strip_prefix(&[239, 187, 191]).unwrap_or(&data),
    )?)
}
pub fn record(directory: &Path) -> Result<Option<Record>> {
    let p = no_links(&directory.join(OWN).join(MARKER))?;
    if !p.exists() {
        return Ok(None);
    }
    let data = read_json(&p, 20000)?;
    if let Some(proxies) = data.get("proxies") {
        ensure!(
            proxies.as_array().is_some_and(|a| !a.is_empty()),
            "部署记录的 DLL 入口无效"
        );
    }
    let r: Record = serde_json::from_value(data)?;
    r.validate()?;
    Ok(Some(r))
}
pub fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    no_links(path)?;
    let mut f = OpenOptions::new().write(true).create_new(true).open(path)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    Ok(())
}
pub fn atomic_json(path: &Path, value: &impl Serialize) -> Result<()> {
    no_links(path)?;
    let parent = path.parent().context("无效路径")?;
    no_links(parent)?;
    fs::create_dir_all(parent)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer_pretty(&mut temp, value)?;
    temp.as_file().sync_all()?;
    no_links(path)?;
    temp.persist(path)?;
    Ok(())
}
pub fn assert_stopped(exe: &Path) -> Result<()> {
    let names = win::running_in_directory(exe.parent().context("无效游戏路径")?)?;
    ensure!(
        names.is_empty(),
        "请先完全退出游戏及同目录程序：{}",
        names.join(", ")
    );
    Ok(())
}
/// Read-only early checks. Deployment must repeat them after acquiring its lock.
pub fn preflight_install(exe: &Path, proxies: &[String]) -> Result<()> {
    let exe = location(exe, true)?;
    assert_stopped(&exe)?;
    let dir = exe.parent().unwrap();
    let previous = record(dir)?;
    for name in normalize_proxies(proxies)? {
        let path = no_links(&dir.join(&name))?;
        if path.exists() {
            ensure!(path.is_file(), "DLL 入口被目录占用：{name}");
            ensure!(
                previous
                    .as_ref()
                    .and_then(|r| r.hashes.get(&name))
                    .is_some_and(|expected| digest(&path).is_ok_and(|actual| actual == *expected))
                    || crate::cleanup::known_proxy(&path)?,
                "已有其他文件占用 DLL 入口，请先处理冲突：{name}"
            );
        }
    }
    Ok(())
}
/// Apply only managed INI keys. No package download or proxy replacement occurs.
pub fn apply_parameters(
    exe: &Path,
    context: &crate::presets::Context,
    values: &crate::presets::Values,
) -> Result<()> {
    let exe = location(exe, true)?;
    let dir = exe.parent().unwrap();
    let _lock = win::game_lock(dir)?;
    assert_stopped(&exe)?;
    let record = record(dir)?.context("请先安装当前方案再应用参数")?;
    ensure!(
        record.scheme_id.as_deref() == Some(context.scheme.as_str()),
        "已部署方案不同，请先安装当前方案"
    );
    ensure!(
        status(&exe).starts_with("已部署"),
        "补丁文件已变化，请刷新后检查"
    );
    let ini = no_links(&dir.join(INI))?;
    ensure!(ini.metadata()?.len() <= 1024 * 1024, "INI 文件过大");
    let before = fs::read(&ini)?;
    let after = context.configure(&before, values)?;
    if before == after {
        return Ok(());
    }
    let mut temp = tempfile::NamedTempFile::new_in(dir)?;
    temp.write_all(&after)?;
    temp.as_file().sync_all()?;
    no_links(&ini)?;
    ensure!(fs::read(&ini)? == before, "INI 已变化，请刷新后重试");
    assert_stopped(&exe)?;
    temp.persist(&ini)?;
    Ok(())
}
pub fn status(exe: &Path) -> String {
    fn inner(exe: &Path) -> Result<String> {
        let p = location(exe, false)?;
        let dir = p.parent().unwrap();
        if let Some(r) = record(dir)? {
            if r.cache_pending {
                return Ok("补丁已移除，缓存待清理".into());
            }
            let mut edited = false;
            for (n, h) in &r.hashes {
                let q = no_links(&dir.join(n))?;
                if !q.is_file() {
                    return Ok("未完成 / 可清理恢复".into());
                }
                if digest(&q)? != *h {
                    if n == INI {
                        edited = true
                    } else {
                        return Ok("DLL 已调整 / 卸载会识别本项目文件并保留其他 MOD".into());
                    }
                }
            }
            return Ok(format!(
                "已部署 {}{} / {}{}",
                r.backend.to_uppercase(),
                r.payload_version
                    .as_ref()
                    .map(|v| format!(" @{v}"))
                    .unwrap_or_default(),
                r.selected().join(", "),
                if edited {
                    " / 配置已修改，可正常卸载"
                } else {
                    ""
                }
            ));
        }
        for name in &PROXIES[5..] {
            if crate::cleanup::known_proxy(&dir.join(name))? {
                return Ok("已有手动补丁或其他 MOD / 卸载会自动识别本项目文件".into());
            }
        }
        if PROXIES[..5]
            .iter()
            .chain(
                [
                    INI,
                    "rtxfg_vk_bridge.dll",
                    ".rtx-fg-script.json",
                    ".rtx-fg-manager.json",
                    ".rtx-fg-v3-legacy.json",
                ]
                .iter(),
            )
            .any(|n| dir.join(n).exists())
        {
            return Ok("已有手动补丁或其他 MOD / 卸载会自动识别本项目文件".into());
        }
        Ok("未部署".into())
    }
    inner(exe).unwrap_or_else(|e| format!("需检查：{e}"))
}
pub fn deploy(exe: &Path, backend: &str, proxies: &[String]) -> Result<String> {
    deploy_with_level(exe, backend, proxies, None)
}
pub fn deploy_with_level(
    exe: &Path,
    backend: &str,
    proxies: &[String],
    level: Option<u8>,
) -> Result<String> {
    let data = package(backend, proxies)?;
    deploy_prepared(
        exe,
        backend,
        proxies,
        level,
        data,
        backend.starts_with("native").then_some("0.2.6"),
    )
}
pub fn deploy_prepared(
    exe: &Path,
    backend: &str,
    proxies: &[String],
    level: Option<u8>,
    data: BTreeMap<String, Vec<u8>>,
    payload_version: Option<&str>,
) -> Result<String> {
    deploy_prepared_context(exe, backend, proxies, level, data, payload_version, None)
}
pub fn deploy_prepared_context(
    exe: &Path,
    backend: &str,
    proxies: &[String],
    level: Option<u8>,
    mut data: BTreeMap<String, Vec<u8>>,
    payload_version: Option<&str>,
    context: Option<&crate::presets::Context>,
) -> Result<String> {
    let expected = deployment_names(backend, proxies)?;
    ensure!(
        data.keys()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>()
            == expected.into_iter().collect(),
        "部署文件范围无效"
    );
    if let Some(v) = payload_version {
        crate::updater::version(v)?;
    }
    let p = location(exe, true)?;
    let dir = p.parent().unwrap();
    let _lock = win::game_lock(dir)?;
    assert_stopped(&p)?;
    let delta_cache_ids = if context.is_some_and(|c| c.delta) {
        ensure!(crate::delta::is_game(&p), "三角洲专项目标不匹配");
        let id = crate::delta::cache_id(&p);
        data.insert(
            INI.into(),
            crate::diagnostics::edit_ini(&data[INI], "Compatibility", crate::delta::ID_KEY, &id)?,
        );
        crate::delta::register_at(&crate::delta::root()?, &p, &id)?;
        vec![id]
    } else {
        Vec::new()
    };
    let selected = normalize_proxies(proxies)?;
    if let Some(level) = level {
        ensure!(level <= 3, "日志级别无效");
        let bytes =
            crate::diagnostics::edit_ini(&data[INI], "Logging", "Level", &level.to_string())?;
        data.insert(INI.into(), bytes);
    }
    let root = no_links(&dir.join(OWN))?;
    let mut existing = record(dir)?;
    // Adopt only identical, recognized proxies. This changes no game binary,
    // while allowing an already-tested manual package to retain custom INI text.
    if existing.is_none()
        && context.is_some()
        && dir.join(INI).is_file()
        && selected.iter().all(|name| {
            let path = dir.join(name);
            crate::cleanup::known_proxy(&path).unwrap_or(false)
                && digest(&path).is_ok_and(|h| h == hash(&data[name]))
        })
        && ![
            ".rtx-fg-script.json",
            ".rtx-fg-manager.json",
            ".rtx-fg-v3-legacy.json",
            "rtxfg_vk_bridge.dll",
        ]
        .iter()
        .any(|name| dir.join(name).exists())
    {
        for name in PROXIES
            .iter()
            .filter(|n| !selected.iter().any(|s| s == **n))
        {
            ensure!(
                !crate::cleanup::known_proxy(&no_links(&dir.join(name))?)?,
                "已有其他补丁入口，请先卸载再切换上游方案"
            );
        }
        let ini = no_links(&dir.join(INI))?;
        ensure!(ini.metadata()?.len() <= 1024 * 1024, "INI 文件过大");
        let mut hashes: BTreeMap<_, _> = data.iter().map(|(n, b)| (n.clone(), hash(b))).collect();
        hashes.insert(INI.into(), digest(&ini)?);
        let r = Record {
            schema: 3,
            backend: backend.into(),
            payload_version: payload_version.map(str::to_owned),
            proxy: selected[0].clone(),
            proxies: selected.clone(),
            hashes,
            cleanup_dirs: BTreeMap::new(),
            scheme_id: context.map(|c| c.scheme.clone()),
            delta_cache_ids: delta_cache_ids.clone(),
            delta_legacy_cache: context.is_some_and(|c| c.delta),
            cache_pending: false,
        };
        fs::create_dir_all(&root)?;
        write_new(&root.join(MARKER), &serde_json::to_vec(&r)?)?;
        existing = Some(r);
    }
    if let Some(mut r) = existing {
        if r.backend == backend && r.selected() == selected && status(&p).starts_with("已部署") {
            ensure!(
                data.iter()
                    .filter(|(name, _)| name.as_str() != INI)
                    .all(|(name, bytes)| r.hashes.get(name) == Some(&hash(bytes))),
                "当前安装与内置文件版本不同，请先卸载补丁，再安装新版；已有文件未覆盖"
            );
            {
                let ini = no_links(&dir.join(INI))?;
                let current = fs::read(&ini)?;
                let updated =
                    crate::presets::merge_context(&current, &data[INI], backend, context)?;
                if let Some(context) = context {
                    r.scheme_id = Some(context.scheme.clone());
                    for id in &delta_cache_ids {
                        if !r.delta_cache_ids.contains(id) {
                            r.delta_cache_ids.push(id.clone());
                        }
                    }
                    // Persist ownership before replacing the INI, so a crash
                    // cannot orphan runtime files released by the new settings.
                    atomic_json(&root.join(MARKER), &r)?;
                }
                if current != updated {
                    let stage = no_links(&root.join(format!("{INI}.config-stage")))?;
                    ensure!(!stage.exists(), "存在未完成的配置更新，请先卸载补丁");
                    write_new(&stage, &updated)?;
                    if let Err(error) = win::replace_existing(&stage, &ini) {
                        let _ = fs::remove_file(&stage);
                        return Err(error.context("配置更新失败，原文件未改变"));
                    }
                    return Ok("配置已更新；完全退出并重新启动游戏后生效".into());
                }
            }
            return Ok("已部署，无需重复操作".into());
        }
        bail!("已有部署或未完成操作，请先清理再重新部署")
    }
    if backend == "upstream_sm86" {
        for name in PROXIES
            .iter()
            .filter(|n| !selected.iter().any(|s| s == **n))
        {
            let path = no_links(&dir.join(name))?;
            ensure!(
                !path.is_file() || !crate::cleanup::known_proxy(&path)?,
                "已有其他补丁入口，请先卸载再切换上游方案"
            );
        }
    }
    let conflicts = if backend.starts_with("native") || backend == "upstream_sm86" {
        data.keys().cloned().collect::<Vec<_>>()
    } else {
        PROXIES[..5]
            .iter()
            .chain([INI].iter())
            .map(|s| (*s).into())
            .collect()
    };
    for name in conflicts.iter().map(String::as_str).chain([
        "rtxfg_vk_bridge.dll",
        ".rtx-fg-script.json",
        ".rtx-fg-manager.json",
        ".rtx-fg-v3-legacy.json",
    ]) {
        let q = no_links(&dir.join(name))?;
        ensure!(
            !q.exists(),
            "已有 {name}，未覆盖；请先清理旧版或移除冲突 MOD"
        );
    }
    for n in ["cache", "logs"] {
        let q = no_links(&root.join(n))?;
        ensure!(
            !q.exists() || q.is_dir(),
            "补丁工作目录中存在同名文件，已保留"
        );
    }
    for n in data.keys() {
        ensure!(
            !no_links(&root.join(format!("{n}.stage")))?.exists(),
            "存在待恢复的临时文件，请先卸载补丁"
        );
    }
    fs::create_dir_all(&root)?;
    let r = Record {
        schema: 3,
        backend: backend.into(),
        payload_version: payload_version.map(str::to_owned),
        proxy: selected[0].clone(),
        proxies: selected,
        hashes: data.iter().map(|(n, b)| (n.clone(), hash(b))).collect(),
        cleanup_dirs: BTreeMap::new(),
        scheme_id: context.map(|c| c.scheme.clone()),
        delta_cache_ids,
        delta_legacy_cache: false,
        cache_pending: false,
    };
    write_new(&root.join(MARKER), &serde_json::to_vec(&r)?)?;
    let mut published = Vec::new();
    let outcome = (|| -> Result<()> {
        fs::create_dir_all(root.join("cache"))?;
        fs::create_dir_all(root.join("logs"))?;
        for (n, b) in &data {
            let stage = root.join(format!("{n}.stage"));
            write_new(&stage, b)?;
            let dst = no_links(&dir.join(n))?;
            win::rename_no_replace(&stage, &dst)?;
            published.push(n.clone());
        }
        Ok(())
    })();
    if let Err(e) = outcome {
        for n in published.iter().rev() {
            let q = no_links(&dir.join(n))?;
            if q.is_file() && digest(&q)? == r.hashes[n] {
                let _ = fs::remove_file(q);
            }
        }
        return Err(e.context("部署未完成；恢复记录已保留，请先卸载补丁"));
    }
    Ok("部署完成；关闭管理器后仍然生效".into())
}
