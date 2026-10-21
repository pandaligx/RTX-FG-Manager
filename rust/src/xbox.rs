//! GDK metadata for Xbox PC flat-file installs. Never decrypt executables,
//! change ACLs, or treat an arbitrary unreadable EXE as a compatible game.
use crate::core;
use anyhow::{Result, ensure};
use quick_xml::{Reader, events::Event};
use std::{
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
};

#[derive(Clone, Debug)]
pub struct Game {
    pub exe: PathBuf,
    pub root: PathBuf,
    pub title: String,
    pub store_id: String,
    pub machine: u16,
    pub logos: Vec<PathBuf>,
}

fn logos(
    root: &Path,
    attrs: &std::collections::BTreeMap<String, String>,
    names: &[&str],
) -> Vec<PathBuf> {
    names
        .iter()
        .filter_map(|name| {
            let path = Path::new(attrs.get(*name)?);
            (!path.as_os_str().is_empty()
                && path.components().all(|c| matches!(c, Component::Normal(_)))
                && !path.to_string_lossy().contains(':')
                && path
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("png")))
            .then(|| root.join(path))
        })
        .collect()
}

struct Manifest {
    games: Vec<Game>,
    identity: String,
    publisher: String,
}

pub fn manifest(path: &Path) -> Result<Vec<Game>> {
    Ok(read_manifest(path)?.games)
}

fn read_manifest(path: &Path) -> Result<Manifest> {
    let path = core::no_links(path)?;
    let file = fs::File::open(&path)?;
    ensure!(file.metadata()?.len() <= 256 * 1024, "Xbox 游戏清单过大");
    let mut bytes = Vec::new();
    file.take(256 * 1024 + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 256 * 1024, "Xbox 游戏清单过大");
    let xml = if bytes.starts_with(&[0xff, 0xfe]) {
        ensure!(bytes.len().is_multiple_of(2), "Xbox 游戏清单编码无效");
        String::from_utf16(
            &bytes[2..]
                .as_chunks::<2>()
                .0
                .iter()
                .map(|b| u16::from_le_bytes([b[0], b[1]]))
                .collect::<Vec<_>>(),
        )?
    } else {
        String::from_utf8(bytes)?
    };
    let mut reader = Reader::from_str(&xml);
    reader.config_mut().expand_empty_elements = true;
    let root = path.parent().unwrap();
    let mut stack = Vec::<Vec<u8>>::new();
    let mut games = Vec::new();
    let mut title = String::new();
    let mut store_id = String::new();
    let mut shell_logos = Vec::new();
    let mut identity = String::new();
    let mut publisher = String::new();
    loop {
        match reader.read_event()? {
            Event::Start(e) | Event::Empty(e) => {
                let name = e.name().as_ref().to_vec();
                let attrs = e
                    .attributes()
                    .map(|a| {
                        let a = a?;
                        Ok((
                            String::from_utf8(a.key.as_ref().to_vec())?,
                            a.decode_and_unescape_value(reader.decoder())?.into_owned(),
                        ))
                    })
                    .collect::<Result<std::collections::BTreeMap<_, _>>>()?;
                if name == b"Identity" && stack.as_slice() == [b"Game".to_vec()] {
                    identity = attrs.get("Name").cloned().unwrap_or_default();
                    publisher = attrs.get("Publisher").cloned().unwrap_or_default();
                }
                if name == b"ShellVisuals" && stack.as_slice() == [b"Game".to_vec()] {
                    shell_logos = logos(
                        root,
                        &attrs,
                        &[
                            "Square480x480Logo",
                            "Square150x150Logo",
                            "Square44x44Logo",
                            "StoreLogo",
                        ],
                    );
                    title = attrs
                        .get("DefaultDisplayName")
                        .or_else(|| attrs.get("DisplayName"))
                        .filter(|s| !s.starts_with("ms-resource:"))
                        .cloned()
                        .unwrap_or_default();
                }
                if name == b"Executable"
                    && stack.as_slice() == [b"Game".to_vec(), b"ExecutableList".to_vec()]
                    && attrs
                        .get("TargetDeviceFamily")
                        .is_some_and(|s| s.eq_ignore_ascii_case("PC"))
                    && !attrs
                        .get("IsDevOnly")
                        .is_some_and(|s| s == "true" || s == "1")
                {
                    let relative =
                        PathBuf::from(attrs.get("Name").map(String::as_str).unwrap_or(""));
                    if !relative.as_os_str().is_empty()
                        && relative
                            .components()
                            .all(|c| matches!(c, Component::Normal(_)))
                        && !relative.to_string_lossy().contains(':')
                        && relative
                            .extension()
                            .is_some_and(|x| x.eq_ignore_ascii_case("exe"))
                    {
                        let machine = match attrs
                            .get("Architecture")
                            .map(|s| s.to_ascii_lowercase())
                            .as_deref()
                        {
                            None | Some("x64") => 0x8664,
                            Some("x86") => 0x14c,
                            Some("arm64") => 0xaa64,
                            _ => 0,
                        };
                        if machine != 0 {
                            games.push(Game {
                                exe: root.join(relative),
                                root: root.into(),
                                title: attrs
                                    .get("OverrideDisplayName")
                                    .filter(|s| !s.starts_with("ms-resource:"))
                                    .cloned()
                                    .unwrap_or_default(),
                                machine,
                                store_id: String::new(),
                                logos: logos(
                                    root,
                                    &attrs,
                                    &[
                                        "OverrideSquare480x480Logo",
                                        "OverrideLogo",
                                        "OverrideSquare44x44Logo",
                                    ],
                                ),
                            });
                        }
                    }
                }
                // quick-xml does not expand empty elements by default. Use a
                // configured expansion so the same bounded stack handles both.
                stack.push(name);
            }
            Event::Text(e) if stack.as_slice() == [b"Game".to_vec(), b"StoreId".to_vec()] => {
                let id = e.unescape()?.trim().to_ascii_lowercase();
                if crate::game_names::valid_key(&format!("xbox:{id}")) {
                    store_id = id;
                }
            }
            Event::End(_) => {
                stack.pop();
            }
            Event::DocType(_) => anyhow::bail!("Xbox 游戏清单不支持 DTD"),
            Event::Eof => {
                ensure!(stack.is_empty(), "Xbox 游戏清单不完整");
                break;
            }
            _ => {}
        }
    }
    for game in &mut games {
        game.store_id = store_id.clone();
        game.logos.extend(shell_logos.iter().cloned());
        if game.title.is_empty() {
            game.title = title.clone();
        }
        if game.title.is_empty() {
            let folder = if root
                .file_name()
                .is_some_and(|n| n.eq_ignore_ascii_case("Content"))
            {
                root.parent().unwrap_or(root)
            } else {
                root
            };
            game.title = folder
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
        }
    }
    Ok(Manifest {
        games,
        identity,
        publisher,
    })
}

pub fn declared(exe: &Path) -> Option<Game> {
    for dir in exe.parent()?.ancestors().take(12) {
        let path = dir.join("MicrosoftGame.config");
        if path.is_file() {
            return manifest(&path)
                .ok()?
                .into_iter()
                .find(|g| core::key(&g.exe) == core::key(exe));
        }
    }
    None
}

/// Installation evidence for a renderer, including reviewed package-specific
/// child paths. This is not a proof of mod compatibility or a recommendation ID:
/// COD HQ's StoreId covers more than one game. `declared` stays exact-match only.
pub fn identified(exe: &Path) -> Option<Game> {
    let exe = core::no_links(exe).ok()?;
    for dir in exe.parent()?.ancestors().take(12) {
        let path = dir.join("MicrosoftGame.config");
        if !path.is_file() {
            continue;
        }
        let document = read_manifest(&path).ok()?;
        if let Some(game) = document
            .games
            .iter()
            .find(|g| core::key(&g.exe) == core::key(&exe))
        {
            return Some(game.clone());
        }
        // The supplied Xbox COD manifest exposes only bootstrapper.exe. Keep a
        // bounded renderer rule, never a generic "anything under Content" rule.
        // These are known x64 paths; readable PE headers still take precedence.
        if !document.identity.eq_ignore_ascii_case("38985CA0.COREBase")
            || !document
                .publisher
                .eq_ignore_ascii_case("CN=07A9AC0F-5502-4D92-BA69-01D5D39D1E92")
        {
            return None;
        }
        let mut game = document.games.into_iter().find(|game| {
            game.store_id == "9n201kqxs5bm"
                && game.machine == 0x8664
                && core::key(&game.exe) == core::key(&dir.join("bootstrapper.exe"))
                && game.exe.is_file()
        })?;
        let relative = ["cod.exe", "sp26/sp26-cod.exe"]
            .into_iter()
            .find(|relative| core::key(&exe) == core::key(&dir.join(relative)))?;
        if !exe.is_file() {
            return None;
        }
        game.exe = exe.clone();
        // Keep the child game separate from HQ so a scan does not silently
        // turn a child-only selection into installation in both directories.
        if relative.starts_with("sp26/") {
            game.root = exe.parent()?.into();
            game.title.push_str(" · sp26");
        }
        // Do not share HQ's cloud identity across unrelated COD campaigns.
        game.store_id.clear();
        return Some(game);
    }
    None
}

pub fn access_hint(path: &Path) -> String {
    format!(
        "无法读取游戏程序：{}。如果是 Xbox / Game Pass 游戏，需要可识别的 MicrosoftGame.config 和对应游戏入口；不能用启动器或辅助程序替代。请从 Xbox 应用的“管理 → 文件 → 浏览”找到完整游戏目录，使用“添加游戏文件夹”。若仍失败，请提供该目录及上级目录的 MicrosoftGame.config 供排查。管理员权限不能解除游戏包的读取保护。",
        path.display()
    )
}
