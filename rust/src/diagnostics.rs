//! Per-game configuration edits and bounded, local-only diagnostic exports.
use crate::{cleanup, core, selftest, win};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    collections::BTreeSet,
    fs,
    io::{Read, Seek, SeekFrom, Write},
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
#[derive(Clone, Copy)]
pub enum Encoding {
    Utf8,
    Utf8Bom,
    Utf16Le,
    Utf16Be,
}
pub fn decode_ini(bytes: &[u8]) -> Result<(String, Encoding)> {
    if bytes.starts_with(&[255, 254]) || bytes.starts_with(&[254, 255]) {
        ensure!(bytes.len().is_multiple_of(2), "INI 编码无效");
        let le = bytes[0] == 255;
        let words = bytes[2..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| {
                if le {
                    u16::from_le_bytes([b[0], b[1]])
                } else {
                    u16::from_be_bytes([b[0], b[1]])
                }
            })
            .collect::<Vec<_>>();
        return Ok((
            String::from_utf16(&words)?,
            if le {
                Encoding::Utf16Le
            } else {
                Encoding::Utf16Be
            },
        ));
    }
    let bom = bytes.starts_with(&[239, 187, 191]);
    Ok((
        String::from_utf8(bytes[if bom { 3 } else { 0 }..].to_vec())?,
        if bom {
            Encoding::Utf8Bom
        } else {
            Encoding::Utf8
        },
    ))
}
fn encode_ini(text: &str, encoding: Encoding) -> Vec<u8> {
    match encoding {
        Encoding::Utf8 => text.as_bytes().to_vec(),
        Encoding::Utf8Bom => [&[239, 187, 191][..], text.as_bytes()].concat(),
        Encoding::Utf16Le | Encoding::Utf16Be => {
            let le = matches!(encoding, Encoding::Utf16Le);
            let mut v = if le { vec![255, 254] } else { vec![254, 255] };
            for c in text.encode_utf16() {
                v.extend(if le { c.to_le_bytes() } else { c.to_be_bytes() })
            }
            v
        }
    }
}
pub fn edit_ini(bytes: &[u8], section: &str, key: &str, value: &str) -> Result<Vec<u8>> {
    ensure!(!value.contains(['\r', '\n']), "INI 参数无效");
    let (text, encoding) = decode_ini(bytes)?;
    let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let mut lines = text
        .split_inclusive('\n')
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let mut inside = false;
    let mut found_section = 0;
    let mut location = None;
    let mut insertion = lines.len();
    for (i, line) in lines.iter().enumerate() {
        let s = line.trim();
        if s.starts_with('[') && s.ends_with(']') {
            if inside {
                insertion = i;
            }
            inside = s[1..s.len() - 1].trim().eq_ignore_ascii_case(section);
            if inside {
                found_section += 1;
                insertion = lines.len();
            }
        } else if inside
            && !s.starts_with([';', '#'])
            && let Some((k, _)) = s.split_once('=')
            && k.trim().eq_ignore_ascii_case(key)
        {
            ensure!(location.is_none(), "INI 存在重复参数，未修改");
            location = Some(i);
        }
    }
    ensure!(found_section <= 1, "INI 存在重复分节，未修改");
    if let Some(i) = location {
        let line = &lines[i];
        let eq = line.find('=').unwrap();
        let ending = if line.ends_with("\r\n") {
            "\r\n"
        } else if line.ends_with('\n') {
            "\n"
        } else {
            ""
        };
        let rest = line[eq + 1..].trim_end_matches(['\r', '\n']);
        let split = rest.find([';', '#']);
        let comment = split.map(|n| &rest[n..]).unwrap_or("");
        lines[i] = format!(
            "{}={}{}{}{}",
            &line[..eq],
            value,
            if comment.is_empty() { "" } else { " " },
            comment,
            ending
        );
    } else {
        if insertion == lines.len()
            && let Some(last) = lines.last_mut()
            && !last.ends_with('\n')
        {
            last.push_str(newline)
        }
        if found_section == 0 {
            lines.push(format!("[{section}]{newline}{key}={value}{newline}"));
        } else {
            lines.insert(insertion, format!("{key}={value}{newline}"));
        }
    }
    Ok(encode_ini(&lines.concat(), encoding))
}
pub fn game_level(exe: &Path) -> Result<Option<u8>> {
    let path = core::location(exe, false)?
        .parent()
        .unwrap()
        .join(core::INI);
    if !path.exists() {
        return Ok(None);
    }
    core::no_links(&path)?;
    ensure!(path.metadata()?.len() <= 1024 * 1024, "INI 文件过大");
    let bytes = fs::read(path)?;
    let (text, _) = decode_ini(&bytes)?;
    let Some(value) = ini_value(&text, "Logging", "Level")? else {
        return Ok(Some(1));
    };
    let level = value
        .split([';', '#'])
        .next()
        .unwrap_or_default()
        .trim()
        .parse::<u8>()
        .context("日志级别无效")?;
    ensure!(level <= 3, "日志级别无效");
    Ok(Some(level))
}
pub fn ini_value(text: &str, section: &str, key: &str) -> Result<Option<String>> {
    let mut active = false;
    let mut count = 0;
    let mut value = None;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') && line.ends_with(']') {
            active = line[1..line.len() - 1].trim().eq_ignore_ascii_case(section);
            if active {
                count += 1;
                ensure!(count == 1, "INI 存在重复分节，未修改");
            }
        } else if active
            && !line.starts_with([';', '#'])
            && let Some((k, v)) = line.split_once('=')
            && k.trim().eq_ignore_ascii_case(key)
        {
            ensure!(value.is_none(), "INI 存在重复参数，未修改");
            value = Some(
                v.split([';', '#'])
                    .next()
                    .unwrap_or_default()
                    .trim()
                    .to_string(),
            );
        }
    }
    Ok(value)
}
pub fn apply_level(exe: &Path, level: u8) -> Result<bool> {
    ensure!(level <= 3, "日志级别无效");
    let path = core::location(exe, false)?;
    let dir = path.parent().unwrap();
    let _lock = win::game_lock(dir)?;
    let ini = core::no_links(&dir.join(core::INI))?;
    if !ini.exists() {
        return Ok(false);
    }
    core::assert_stopped(&path)?;
    let mut known = false;
    for name in core::PROXIES {
        if core::no_links(&dir.join(name)).is_ok() && cleanup::known_proxy(&dir.join(name))? {
            known = true;
            break;
        }
    }
    ensure!(known, "未确认补丁归属，未修改 INI");
    ensure!(ini.metadata()?.len() <= 1024 * 1024, "INI 文件过大");
    let before = fs::read(&ini)?;
    let after = edit_ini(&before, "Logging", "Level", &level.to_string())?;
    let mut temp = tempfile::NamedTempFile::new_in(dir)?;
    temp.write_all(&after)?;
    temp.as_file().sync_all()?;
    ensure!(fs::read(&ini)? == before, "INI 已变化，请刷新后重试");
    core::assert_stopped(&path)?;
    temp.persist(&ini)?;
    Ok(true)
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExportRequest {
    pub game: Option<PathBuf>,
    pub selftest: Option<PathBuf>,
    pub attachments: Vec<PathBuf>,
    pub manager_log: Vec<String>,
}
#[derive(Serialize)]
struct Entry {
    source: String,
    name: String,
    original_bytes: u64,
    exported_bytes: u64,
    truncated: bool,
    note: String,
}
const MAX_FILES: usize = 256;
const MAX_BYTES: u64 = 128 * 1024 * 1024;
const MAX_TEXT: u64 = 32 * 1024 * 1024;
fn cancelled(cancel: &AtomicBool) -> Result<()> {
    ensure!(!cancel.load(Ordering::Relaxed), "已取消");
    Ok(())
}
/// A fixed per-game operation log in manager data, never written into the game.
pub fn record_operation(data: &Path, exe: &Path, operation: &str, result: &str) -> Result<()> {
    let dir = core::no_links(&data.join("diagnostics/operations"))?;
    fs::create_dir_all(&dir)?;
    let file =
        core::no_links(&dir.join(format!("{}.jsonl", core::hash(core::key(exe).as_bytes()))))?;
    if file.is_file() && file.metadata()?.len() > 1024 * 1024 {
        fs::remove_file(&file)?;
    }
    let mut log = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(file)?;
    serde_json::to_writer(
        &mut log,
        &json!({"time":chrono::Utc::now().to_rfc3339(),"exe":exe,"operation":operation,"result":result}),
    )?;
    log.write_all(b"\n")?;
    Ok(())
}
fn log_name(path: &Path) -> bool {
    let n = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase();
    n.ends_with(".log")
        || n.ends_with(".jsonl")
        || n == "report.wer"
        || n == "crashcontext.runtime-xml"
}
fn collect(
    root: &Path,
    depth: u32,
    files: &mut BTreeSet<PathBuf>,
    visited: &mut usize,
    cancel: &AtomicBool,
) -> Result<()> {
    if !root.is_dir() || core::no_links(root).is_err() || *visited >= 2000 {
        return Ok(());
    }
    for entry in fs::read_dir(root)? {
        cancelled(cancel)?;
        *visited += 1;
        if *visited > 2000 {
            break;
        }
        let p = entry?.path();
        if core::no_links(&p).is_err() {
            continue;
        }
        if p.is_file() && log_name(&p) {
            files.insert(p);
        } else if depth > 0 && p.is_dir() {
            collect(&p, depth - 1, files, visited, cancel)?;
        }
    }
    Ok(())
}
pub fn matching_events(xml: &str, exe: &Path) -> Vec<String> {
    let re = regex::Regex::new(r"(?s)<Event(?:\s[^>]*)?>.*?</Event>").unwrap();
    let values = regex::Regex::new(r"(?s)<Data(?:\s[^>]*)?>(.*?)</Data>").unwrap();
    re.find_iter(xml)
        .filter(|m| {
            values.captures_iter(m.as_str()).any(|c| {
                quick_xml::escape::unescape(&c[1])
                    .ok()
                    .is_some_and(|s| core::key(Path::new(s.trim())) == core::key(exe))
            })
        })
        .map(|m| m.as_str().to_string())
        .collect()
}
fn events(exe: &Path, cancel: &AtomicBool) -> Result<Vec<String>> {
    let temp = tempfile::tempfile()?;
    let reader = temp.try_clone()?;
    let system = PathBuf::from(std::env::var_os("WINDIR").unwrap_or_else(|| "C:\\Windows".into()))
        .join("System32/wevtutil.exe");
    let mut child=Command::new(system).args(["qe","Application","/q:*[System[(EventID=1000 or EventID=1001) and TimeCreated[timediff(@SystemTime)<=86400000]]]","/f:xml","/uni:true","/c:256","/rd:true"]).creation_flags(0x08000000).stdin(Stdio::null()).stdout(temp).stderr(Stdio::null()).spawn()?;
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            ensure!(status.success(), "无法读取 Windows 错误事件");
            break;
        }
        if cancel.load(Ordering::Relaxed) || start.elapsed() > Duration::from_secs(5) {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!("Windows 错误事件读取已停止")
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let mut reader = reader;
    reader.seek(SeekFrom::Start(0))?;
    let mut data = Vec::new();
    reader.take(4 * 1024 * 1024).read_to_end(&mut data)?;
    let text = if data.starts_with(&[255, 254]) {
        decode_ini(&data)?.0
    } else {
        String::from_utf8_lossy(&data).into_owned()
    };
    Ok(matching_events(&text, exe))
}
pub fn export(
    target: &Path,
    request: &ExportRequest,
    cancel: &AtomicBool,
    progress: impl Fn(String),
) -> Result<()> {
    use zip::{ZipWriter, write::FileOptions};
    let target = core::no_links(target)?;
    ensure!(
        target
            .extension()
            .is_some_and(|s| s.eq_ignore_ascii_case("zip")),
        "诊断包必须保存为 ZIP 文件"
    );
    ensure!(request.attachments.len() <= 256, "附加日志过多");
    for path in &request.attachments {
        ensure!(
            path.extension()
                .is_some_and(|s| ["log", "jsonl", "txt", "xml", "wer"]
                    .iter()
                    .any(|x| s.eq_ignore_ascii_case(x))),
            "附加文件必须是文本日志"
        );
    }
    cancelled(cancel)?;
    let parent = target.parent().context("保存目录无效")?;
    ensure!(
        !request
            .attachments
            .iter()
            .any(|p| core::key(p) == core::key(&target)),
        "导出文件不能覆盖日志源文件"
    );
    let options = FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    let mut archive = ZipWriter::new(temp.as_file_mut());
    let mut files = BTreeSet::new();
    let mut notes = Vec::<String>::new();
    let mut visited = 0;
    let mut environment = json!({"schema":1,"manager":crate::VERSION,"created":chrono::Utc::now().to_rfc3339(),"windows_build":win::build(),"scope":"Local diagnostic evidence, not proof of game compatibility","game":request.game,"gpus":selftest::adapters().map_err(|e|e.to_string())});
    if let Some(exe) = &request.game {
        let exe = core::location(exe, false)?;
        let dir = exe.parent().unwrap();
        let mut dlls = Vec::new();
        for name in core::PROXIES {
            let p = dir.join(name);
            if p.is_file() && core::no_links(&p).is_ok() {
                dlls.push(json!({"name":name,"bytes":p.metadata()?.len(),"sha256":core::digest(&p).ok(),"version":win::file_version(&p).ok(),"owned":cleanup::known_proxy(&p).unwrap_or(false)}));
            }
        }
        environment["dlls"] = json!(dlls);
        environment["deployment"] = json!(core::record(dir).map_err(|e| e.to_string()));
        let ini = dir.join(core::INI);
        if ini.is_file() {
            files.insert(ini.clone());
        }
        collect(dir, 0, &mut files, &mut visited, cancel)?;
        for sub in [
            ".rtx-fg-v3/logs",
            "dlssg_sm86/logs",
            "dlssg_sm75_logs_r2",
            "dlssg_sm75_logs",
            "dlssg_sm86_logs",
            "Saved/Logs",
            "Saved/Crashes",
            "logs",
            "Logs",
        ] {
            if !dir.join(sub).is_dir() {
                notes.push(format!("Log directory absent: {}", dir.join(sub).display()));
            }
            if let Err(e) = collect(&dir.join(sub), 3, &mut files, &mut visited, cancel) {
                notes.push(e.to_string())
            }
        }
        if ini.is_file()
            && core::no_links(&ini).is_ok()
            && ini.metadata()?.len() <= 1024 * 1024
            && let Ok((text, _)) = decode_ini(&fs::read(&ini)?)
        {
            let c = cleanup::parse_ini(&text);
            if let Some(p) = c.get("Logging").and_then(|s| s.get("Directory"))
                && let Ok(Some(relative)) = cleanup::relative_dir(dir, p)
            {
                let _ = collect(&dir.join(relative), 2, &mut files, &mut visited, cancel);
            }
        }
        // Unreal's standard Win64/Binaries layout: only inspect this game's inferred root.
        if dir
            .file_name()
            .is_some_and(|n| n.eq_ignore_ascii_case("Win64"))
            && let Some(binaries) = dir.parent()
            && binaries
                .file_name()
                .is_some_and(|n| n.eq_ignore_ascii_case("Binaries"))
            && let Some(root) = binaries.parent()
        {
            let _ = collect(
                &root.join("Saved/Logs"),
                2,
                &mut files,
                &mut visited,
                cancel,
            );
            let _ = collect(
                &root.join("Saved/Crashes"),
                2,
                &mut files,
                &mut visited,
                cancel,
            );
        }
        // Unity publishes company/product in this specific game's *_Data/app.info.
        // Never enumerate all LocalLow vendors or other user profiles.
        let info = dir.join(format!(
            "{}_Data/app.info",
            exe.file_stem().unwrap_or_default().to_string_lossy()
        ));
        if core::no_links(&info).is_ok()
            && info.is_file()
            && info.metadata()?.len() < 8192
            && let (Ok(info), Some(home)) =
                (fs::read_to_string(info), std::env::var_os("USERPROFILE"))
        {
            let parts = info.lines().take(2).map(str::trim).collect::<Vec<_>>();
            if parts.len() == 2
                && parts.iter().all(|s| {
                    !s.is_empty() && !s.contains(['/', '\\', ':']) && *s != "." && *s != ".."
                })
            {
                let root = PathBuf::from(home)
                    .join("AppData/LocalLow")
                    .join(parts[0])
                    .join(parts[1]);
                for name in ["Player.log", "Player-prev.log"] {
                    files.insert(root.join(name));
                }
            }
        }
        if exe
            .file_stem()
            .is_some_and(|n| n.to_string_lossy().to_lowercase().contains("endfield"))
            && let Some(home) = std::env::var_os("USERPROFILE")
        {
            let low = PathBuf::from(home).join("AppData/LocalLow/Hypergryph/Endfield");
            for name in ["Player.log", "Player-prev.log"] {
                files.insert(low.join(name));
            }
        }
        environment["application_events"] = match events(&exe, cancel) {
            Ok(v) => json!(v),
            Err(e) => {
                notes.push(e.to_string());
                json!([])
            }
        };
    }
    if let Some(root) = &request.selftest {
        core::no_links(root)?;
        let report = root.join("report.json");
        if report.is_file() {
            files.insert(report);
        }
        files.insert(root.join("files.json"));
        collect(root, 2, &mut files, &mut visited, cancel)?;
        for entry in fs::read_dir(root)? {
            let p = entry?.path();
            if p.is_dir() && core::no_links(&p).is_ok() {
                for name in ["trace.txt", "request.json", core::INI] {
                    if p.join(name).is_file() {
                        files.insert(p.join(name));
                    }
                }
            }
        }
    }
    files.extend(request.attachments.iter().cloned());
    ensure!(
        !files.iter().any(|p| core::key(p) == core::key(&target)),
        "导出文件不能覆盖日志源文件"
    );
    archive.start_file("environment.json", options)?;
    archive.write_all(serde_json::to_string_pretty(&environment)?.as_bytes())?;
    archive.start_file("manager.log", options)?;
    archive.write_all(request.manager_log.join("\n").as_bytes())?;
    let mut entries = Vec::new();
    let mut total = 0u64;
    // Always retain the summary/identity/events when a full run exceeds the file cap.
    // Remaining logs prefer recent evidence over old sessions.
    let mut ordered = files.into_iter().collect::<Vec<_>>();
    ordered.sort_by_cached_key(|p| {
        let summary = request.selftest.as_ref().is_some_and(|root| {
            ["report.json", "files.json", "events.jsonl"]
                .iter()
                .any(|n| *p == root.join(n))
        });
        let config = request
            .game
            .as_ref()
            .and_then(|g| g.parent())
            .is_some_and(|root| *p == root.join(core::INI));
        (
            if summary {
                0
            } else if config {
                1
            } else {
                2
            },
            std::cmp::Reverse(
                p.metadata()
                    .and_then(|m| m.modified())
                    .unwrap_or(std::time::UNIX_EPOCH),
            ),
            p.clone(),
        )
    });
    for (index, path) in ordered.into_iter().enumerate() {
        cancelled(cancel)?;
        progress(format!("正在导出日志：{}", index + 1));
        let mut e = Entry {
            source: path.display().to_string(),
            name: String::new(),
            original_bytes: 0,
            exported_bytes: 0,
            truncated: false,
            note: String::new(),
        };
        let mut entry_started = false;
        let result = (|| -> Result<()> {
            core::no_links(&path)?;
            let mut file = fs::File::open(&path)?;
            e.original_bytes = file.metadata()?.len();
            ensure!(
                index < MAX_FILES - 3 && total < MAX_BYTES - 8 * 1024 * 1024,
                "已达到诊断包大小限制"
            );
            let bytes = e
                .original_bytes
                .min(MAX_TEXT)
                .min(MAX_BYTES - 8 * 1024 * 1024 - total);
            e.truncated = bytes < e.original_bytes;
            file.seek(SeekFrom::Start(e.original_bytes - bytes))?;
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            e.name = format!("logs/{index:03}-{}", name.replace(['/', '\\', ':'], "_"));
            entry_started = true;
            archive.start_file(&e.name, options)?;
            let mut buffer = [0u8; 65536];
            let mut remaining = bytes;
            while remaining > 0 {
                cancelled(cancel)?;
                let take = buffer.len().min(remaining as usize);
                let n = file.read(&mut buffer[..take])?;
                if n == 0 {
                    e.note = "日志读取期间已变化".into();
                    break;
                }
                archive.write_all(&buffer[..n])?;
                remaining -= n as u64;
                e.exported_bytes += n as u64;
            }
            total += e.exported_bytes;
            Ok(())
        })();
        if let Err(error) = result {
            if entry_started {
                return Err(error);
            }
            e.note = error.to_string();
        }
        entries.push(e);
    }
    cancelled(cancel)?;
    archive.start_file("manifest.json", options)?;
    let manifest = serde_json::to_vec_pretty(
        &json!({"files":entries,"notes":notes,"collected_at":chrono::Utc::now().to_rfc3339(),"limits":{"files":MAX_FILES,"total_bytes":MAX_BYTES,"per_text_bytes":MAX_TEXT,"metadata_reserve_bytes":8*1024*1024},"snapshot":"Files may still be changing; inspect timestamps and notes before sharing"}),
    )?;
    ensure!(
        total
            + manifest.len() as u64
            + serde_json::to_vec_pretty(&environment)?.len() as u64
            + request
                .manager_log
                .iter()
                .map(|s| s.len() as u64 + 1)
                .sum::<u64>()
            <= MAX_BYTES,
        "已达到诊断包大小限制"
    );
    archive.write_all(&manifest)?;
    archive.finish()?;
    drop(archive);
    temp.as_file().sync_all()?;
    temp.persist(target)?;
    Ok(())
}
