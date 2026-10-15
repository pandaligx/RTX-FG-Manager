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
        },
    )
}
pub fn identify(target: &core::DeploymentTarget) -> Result<Option<String>> {
    if let Some(id) = steam(&target.game_exe) {
        return Ok(Some(id));
    }
    let path = mapping_path(target)?;
    if !path.try_exists()? {
        return Ok(None);
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
