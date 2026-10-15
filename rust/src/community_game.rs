//! Match Steam by its managed installation directory, never by executable name.
use crate::{config_store, core};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::OnceLock,
};
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Mapping {
    schema: u32,
    directory: String,
    game: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    name: Option<String>,
}
fn mapping_path(target: &core::DeploymentTarget) -> Result<PathBuf> {
    Ok(config_store::history_root(target)?.join("game-mapping.json"))
}
pub fn save_mapping(target: &core::DeploymentTarget, game: &str) -> Result<()> {
    ensure!(config_store::valid_game(game), "游戏 ID 无效");
    core::atomic_json(
        &mapping_path(target)?,
        &Mapping {
            schema: 1,
            directory: core::key(&target.directory),
            game: game.into(),
            name: None,
        },
    )
}
/// An explicit player-confirmed name, never inferred from a path or executable.
pub fn named_game(name: &str) -> Result<(String, String)> {
    ensure!(
        crate::community::safe_text(name, 120),
        "请填写游戏名称，不要填写路径或个人信息"
    );
    let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
    ensure!(!name.is_empty(), "请填写游戏名称");
    let id = format!(
        "game:n-{}",
        crate::community_identity::digest(name.to_ascii_lowercase().as_bytes())
    );
    Ok((id, name))
}
pub fn save_named_mapping(target: &core::DeploymentTarget, name: &str) -> Result<String> {
    let (game, name) = named_game(name)?;
    core::atomic_json(
        &mapping_path(target)?,
        &Mapping {
            schema: 1,
            directory: core::key(&target.directory),
            game: game.clone(),
            name: Some(name),
        },
    )?;
    Ok(game)
}
pub fn mapped_name(target: &core::DeploymentTarget, game: &str) -> Result<Option<String>> {
    let path = mapping_path(target)?;
    if !path.try_exists()? {
        return Ok(None);
    }
    let map: Mapping = serde_json::from_value(core::read_json(&path, 4096)?)?;
    ensure!(
        map.schema == 1 && map.directory == core::key(&target.directory),
        "本地游戏映射无效"
    );
    if map.game != game {
        return Ok(None);
    }
    if let Some(name) = map.name {
        ensure!(named_game(&name)?.0 == game, "游戏名称与标识不一致");
        return Ok(Some(name));
    }
    Ok(None)
}
pub fn identify(target: &core::DeploymentTarget) -> Result<Option<String>> {
    let path = mapping_path(target)?;
    if !path.try_exists()? {
        return Ok(steam(&target.game_exe));
    }
    let map: Mapping = serde_json::from_value(core::read_json(&path, 4096)?)?;
    ensure!(
        map.schema == 1
            && map.directory == core::key(&target.directory)
            && config_store::valid_game(&map.game),
        "本地游戏映射无效"
    );
    Ok(Some(map.game))
}
fn steam(exe: &Path) -> Option<String> {
    static FIELD: OnceLock<regex::Regex> = OnceLock::new();
    let field = FIELD
        .get_or_init(|| regex::Regex::new(r#"(?m)^\s*"([A-Za-z]+)"\s*"([^"\r\n]*)""#).unwrap());
    for directory in exe.ancestors().skip(1) {
        let common = directory.parent()?;
        if !common.file_name()?.eq_ignore_ascii_case("common") {
            continue;
        }
        let steamapps = common.parent()?;
        if !steamapps.file_name()?.eq_ignore_ascii_case("steamapps") {
            continue;
        }
        core::no_links(steamapps).ok()?;
        let mut matching = None;
        for entry in fs::read_dir(steamapps).ok()?.take(4096).flatten() {
            let filename = entry.file_name().to_string_lossy().to_string();
            let Some(id) = filename
                .strip_prefix("appmanifest_")
                .and_then(|s| s.strip_suffix(".acf"))
            else {
                continue;
            };
            if !config_store::valid_game(&format!("steam:{id}")) {
                continue;
            }
            let path = entry.path();
            if core::no_links(&path).is_err() {
                continue;
            }
            let Ok(file) = fs::File::open(path) else {
                continue;
            };
            let mut bytes = Vec::new();
            if file.take(1024 * 1024 + 1).read_to_end(&mut bytes).is_err()
                || bytes.len() > 1024 * 1024
            {
                continue;
            }
            let text = String::from_utf8_lossy(&bytes);
            let mut app = None;
            let mut folder = None;
            for c in field.captures_iter(&text) {
                if c[1].eq_ignore_ascii_case("appid") {
                    app = Some(c[2].to_string())
                }
                if c[1].eq_ignore_ascii_case("installdir") {
                    folder = Some(c[2].to_string())
                }
            }
            if app.as_deref() == Some(id)
                && folder.as_ref().is_some_and(|f| {
                    !f.contains(['/', '\\', ':'])
                        && directory
                            .file_name()
                            .is_some_and(|d| d.eq_ignore_ascii_case(f))
                })
            {
                if matching.is_some() {
                    return None;
                }
                matching = Some(format!("steam:{id}"));
            }
        }
        return matching;
    }
    None
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_names_are_stable_private_safe_and_game_specific() {
        assert_eq!(
            named_game("Example Game").unwrap().0,
            named_game(" example  GAME ").unwrap().0
        );
        assert_ne!(
            named_game("Example Game 2").unwrap().0,
            named_game("Example Game").unwrap().0
        );
        assert!(config_store::valid_game(
            &named_game("三角洲行动（国服）").unwrap().0
        ));
        assert!(named_game("C:\\Users\\Player\\game").is_err());
        assert!(named_game(" ").is_err());
    }
    #[test]
    fn steam_requires_manifest_id_and_installdir_not_exe_name() {
        let d = tempfile::tempdir().unwrap();
        let steamapps = d.path().join("steamapps");
        let game = steamapps.join("common/Actual Game/Binaries");
        fs::create_dir_all(&game).unwrap();
        fs::write(
            steamapps.join("appmanifest_123.acf"),
            "\"appid\" \"123\"\n\"installdir\" \"Actual Game\"\n",
        )
        .unwrap();
        assert_eq!(
            steam(&game.join("unrelated.exe")).as_deref(),
            Some("steam:123")
        );
        assert_eq!(
            steam(&steamapps.join("common/Other Game/Actual Game.exe")),
            None
        );
        fs::write(
            steamapps.join("appmanifest_123.acf"),
            "\"appid\" \"999\"\n\"installdir\" \"Actual Game\"\n",
        )
        .unwrap();
        assert_eq!(steam(&game.join("unrelated.exe")), None);
    }
}
