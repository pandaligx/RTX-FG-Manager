//! Non-DLSS discovery and negative fixtures. Never execute fixture programs.
use anyhow::Result;
use rtx_fg_manager::{core, scanner};
use serde_json::json;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};

fn pe(path: &Path) -> Result<()> {
    let mut b = vec![0; 1024];
    b[..2].copy_from_slice(b"MZ");
    b[60..64].copy_from_slice(&128u32.to_le_bytes());
    b[128..132].copy_from_slice(b"PE\0\0");
    b[132..134].copy_from_slice(&0x8664u16.to_le_bytes());
    b[148..150].copy_from_slice(&240u16.to_le_bytes());
    b[150..152].copy_from_slice(&2u16.to_le_bytes());
    b[152..154].copy_from_slice(&0x20bu16.to_le_bytes());
    write(path, &b)
}
fn write(path: &Path, data: &[u8]) -> Result<()> {
    fs::create_dir_all(path.parent().unwrap())?;
    fs::write(path, data)?;
    Ok(())
}
fn pck() -> Vec<u8> {
    let mut p = vec![0; 128];
    p[..4].copy_from_slice(b"GDPC");
    p[4..8].copy_from_slice(&2u32.to_le_bytes());
    p[8..12].copy_from_slice(&4u32.to_le_bytes());
    p
}
fn scan(root: &Path) -> Result<Vec<scanner::Game>> {
    Ok(scanner::scan(&[root.into()], &AtomicBool::new(false), |_, _, _| {})?.rows)
}
fn targets(games: &[scanner::Game]) -> Vec<String> {
    let mut paths: Vec<_> = games
        .iter()
        .flat_map(|g| g.targets.iter())
        .map(|s| core::key(Path::new(s)))
        .collect();
    paths.sort();
    paths
}
fn keys(paths: &[PathBuf]) -> Vec<String> {
    let mut paths: Vec<_> = paths.iter().map(|p| core::key(p)).collect();
    paths.sort();
    paths
}
fn steam(root: &Path, title: &str, folder: &str) -> Result<PathBuf> {
    let dir = root.join("steamapps/common").join(folder);
    fs::create_dir_all(&dir)?;
    write(
        &root.join("steamapps/appmanifest_42.acf"),
        format!("\"name\" \"{title}\"\n\"installdir\" \"{folder}\"\n").as_bytes(),
    )?;
    Ok(dir)
}
#[test]
fn godot_separate_embedded_and_signed_exports_need_real_pck_headers() -> Result<()> {
    let t = tempfile::tempdir()?;
    let separate = t.path().join("Portable A/Amber.exe");
    pe(&separate)?;
    write(&separate.with_extension("pck"), &pck())?;
    let embedded = t.path().join("Portable B/Blue.exe");
    pe(&embedded)?;
    let mut data = fs::read(&embedded)?;
    data.extend(pck());
    data.extend(128u64.to_le_bytes());
    data.extend(b"GDPC");
    fs::write(&embedded, &data)?;
    let signed = t.path().join("Portable C/Coral.exe");
    pe(&signed)?;
    let cert_offset = data.len().next_multiple_of(8);
    data.resize(cert_offset, 0);
    data[296..300].copy_from_slice(&(cert_offset as u32).to_le_bytes());
    data[300..304].copy_from_slice(&16u32.to_le_bytes());
    data.resize(cert_offset + 16, 0);
    fs::write(&signed, &data)?;
    // A sibling tool cannot inherit another executable's PCK.
    pe(&separate.parent().unwrap().join("Other.exe"))?;
    let bad = t.path().join("Bad/Bad.exe");
    pe(&bad)?;
    write(&bad.with_extension("pck"), b"not a Godot pack")?;
    let rows = scan(t.path())?;
    assert_eq!(targets(&rows), keys(&[separate, embedded, signed]));
    assert!(
        rows.iter()
            .all(|g| !g.reasons.iter().any(|r| r.contains("DLSS")))
    );
    Ok(())
}
#[test]
fn godot_pe_section_works_without_footer_and_rejects_out_of_range_offsets() -> Result<()> {
    let t = tempfile::tempdir()?;
    let exe = t.path().join("Game/Green.exe");
    pe(&exe)?;
    let mut data = fs::read(&exe)?;
    data[134..136].copy_from_slice(&1u16.to_le_bytes());
    data[392..396].copy_from_slice(b"pck\0");
    data[408..412].copy_from_slice(&128u32.to_le_bytes());
    data[412..416].copy_from_slice(&1024u32.to_le_bytes());
    data.extend(pck());
    fs::write(&exe, &data)?;
    let invalid = t.path().join("Invalid/Green.exe");
    data[412..416].copy_from_slice(&u32::MAX.to_le_bytes());
    write(&invalid, &data)?;
    assert_eq!(targets(&scan(t.path())?), keys(&[exe]));
    Ok(())
}
#[test]
fn source_clients_require_matching_content_and_keep_helpers_out() -> Result<()> {
    let t = tempfile::tempdir()?;
    let one = t.path().join("Source One/hl2.exe");
    pe(&one)?;
    for dll in ["bin/engine.dll", "bin/tier0.dll"] {
        write(&one.parent().unwrap().join(dll), b"engine")?;
    }
    write(
        &one.parent().unwrap().join("hl2/gameinfo.txt"),
        b"\"GameInfo\"\n{\ngame \"Half-Life 2\"\nFileSystem\n{\n}\n}",
    )?;
    let two = t.path().join("Source Two/game/bin/win64/cs2.exe");
    pe(&two)?;
    for dll in ["engine2.dll", "tier0.dll"] {
        write(&two.parent().unwrap().join(dll), b"engine")?;
    }
    write(
        &t.path().join("Source Two/game/csgo/gameinfo.gi"),
        b"\"GameInfo\"\n{\ngame \"Counter-Strike 2\"\nFileSystem\n{\n}\n}",
    )?;
    for main in [&one, &two] {
        for helper in [
            "hammer.exe",
            "hlmv.exe",
            "vpk.exe",
            "srcds.exe",
            "Other.exe",
        ] {
            pe(&main.parent().unwrap().join(helper))?;
        }
    }
    let rows = scan(t.path())?;
    assert_eq!(targets(&rows), keys(&[one, two]));
    assert!(rows.iter().any(|g| g.title == "Counter-Strike 2"));
    Ok(())
}
#[test]
fn cryengine_launcher_is_only_a_renderer_with_its_complete_project_layout() -> Result<()> {
    let t = tempfile::tempdir()?;
    let good = t.path().join("Cry Game");
    let exe = good.join("bin/win_x64_release/GameLauncher.exe");
    pe(&exe)?;
    fs::create_dir_all(good.join("Assets"))?;
    core::atomic_json(
        &good.join("Game.cryproject"),
        &json!({"info":{"name":"Cry Game"},"content":{"assets":["Assets"]}}),
    )?;
    write(&good.join("Engine/engine.pak"), b"engine")?;
    for helper in [
        "Sandbox.exe",
        "ShaderCacheGen.exe",
        "GameServer.exe",
        "Other.exe",
    ] {
        pe(&exe.parent().unwrap().join(helper))?;
    }
    let bad = t.path().join("Bad/bin/win_x64_release/GameLauncher.exe");
    pe(&bad)?;
    core::atomic_json(
        &t.path().join("Bad/Game.cryproject"),
        &json!({"info":{"name":"Bad"},"content":{"assets":["../Cry Game/Assets"]}}),
    )?;
    assert_eq!(targets(&scan(t.path())?), keys(&[exe]));
    Ok(())
}
#[test]
fn gog_declared_game_without_dlss_ignores_launcher_support_dlc_and_escape() -> Result<()> {
    let t = tempfile::tempdir()?;
    let dir = t.path().join("Unrelated display folder");
    let exe = dir.join("client/RenderClient.exe");
    pe(&exe)?;
    for tool in ["Bootstrap.exe", "RenderHelper.exe", "SupportApp.exe"] {
        pe(&dir.join(tool))?;
    }
    core::atomic_json(
        &dir.join("goggame-123.info"),
        &json!({
        "gameId":"123","rootGameId":"123","name":"Portable GOG Game","playTasks":[
            {"category":"game","type":"FileTask","path":"client/RenderClient.exe"},
            {"category":"launcher","path":"Bootstrap.exe"},
            {"category":"game","path":"RenderHelper.exe"},
            {"category":"support","path":"SupportApp.exe"},
            {"category":"game","path":"../Escape.exe"}
        ]}),
    )?;
    pe(&t.path().join("Escape.exe"))?;
    let rows = scan(t.path())?;
    assert_eq!(targets(&rows), keys(std::slice::from_ref(&exe)));
    assert_eq!(rows[0].title, "Portable GOG Game");
    assert_eq!(core::key(Path::new(&rows[0].root)), core::key(&dir));
    let doc = dir.join("goggame-123.info");
    let mut value: serde_json::Value = serde_json::from_slice(&fs::read(&doc)?)?;
    value["rootGameId"] = json!("999");
    core::atomic_json(&doc, &value)?;
    assert!(scan(t.path())?.is_empty());
    // Discovery never created a deployment record.
    assert!(!exe.parent().unwrap().join(core::OWN).exists());
    Ok(())
}
#[test]
fn steam_non_dlss_game_keeps_title_and_excludes_named_utility_suffixes() -> Result<()> {
    let t = tempfile::tempdir()?;
    let dir = steam(t.path(), "Example Game", "Example")?;
    let exe = dir.join("Example.exe");
    pe(&exe)?;
    for helper in [
        "ExampleLauncher.exe",
        "ExampleUpdater.exe",
        "ExampleHelper.exe",
        "ExamplePatcher.exe",
        "ExampleInstaller.exe",
        "ExampleServer.exe",
        "ExampleCrashReporter.exe",
        "ExampleDownloader.exe",
    ] {
        pe(&dir.join(helper))?;
    }
    pe(&dir.join("Tools/Example.exe"))?;
    let rows = scan(t.path())?;
    assert_eq!(targets(&rows), keys(&[exe]));
    assert_eq!(rows[0].title, "Example Game");
    Ok(())
}
#[test]
fn distinct_engines_in_a_collection_and_cancellation_remain_separate() -> Result<()> {
    let t = tempfile::tempdir()?;
    for (folder, title) in [("A", "Amber"), ("B", "Blue")] {
        let exe = t.path().join(folder).join(format!("{title}.exe"));
        pe(&exe)?;
        write(&exe.with_extension("pck"), &pck())?;
    }
    assert_eq!(scan(t.path())?.len(), 2);
    let report = scanner::scan(&[t.path().into()], &AtomicBool::new(true), |_, _, _| {})?;
    assert!(report.rows.is_empty() && report.cancelled && report.directories == 0);
    Ok(())
}
