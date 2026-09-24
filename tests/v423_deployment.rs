//! Isolated filesystem regressions. The synthetic PE files are never executed.
use anyhow::Result;
use rtx_fg_manager::{cleanup, core, diagnostics, presets};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    os::windows::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};

const SCHEME: &str = "test-upstream-035";
const ORIGINAL_INI: &str = "; 用户保留的注释\r\n[FrameGeneration]\r\nGameSpecific=keep\r\n\r\n[Runtime]\r\nMode=Bundled\r\nCacheDirectory=\r\n\r\n[Logging]\r\nLevel=1\r\nDirectory=dlssg_sm86\\logs\r\n\r\n[PrivateGameSection]\r\nUntouched=42\r\n";

fn synthetic_pe(dll: bool, marker: u8) -> Vec<u8> {
    let mut bytes = vec![0; 512];
    bytes[..2].copy_from_slice(b"MZ");
    bytes[60..64].copy_from_slice(&128u32.to_le_bytes());
    bytes[128..132].copy_from_slice(b"PE\0\0");
    bytes[132..134].copy_from_slice(&0x8664u16.to_le_bytes());
    bytes[148..150].copy_from_slice(&240u16.to_le_bytes());
    bytes[150..152].copy_from_slice(&(if dll { 0x2000u16 } else { 2 }).to_le_bytes());
    bytes[152..154].copy_from_slice(&0x20bu16.to_le_bytes());
    bytes[511] = marker;
    bytes
}

struct Game {
    directory: tempfile::TempDir,
    exe: PathBuf,
}

impl Game {
    fn new() -> Result<Self> {
        let directory = tempfile::tempdir()?;
        let exe = directory.path().join("游戏本体.exe");
        fs::write(&exe, synthetic_pe(false, 1))?;
        fs::write(
            directory.path().join("original-game.dll"),
            b"game component",
        )?;
        fs::create_dir(directory.path().join("Saved"))?;
        fs::write(
            directory.path().join("Saved/progress.dat"),
            b"user progress",
        )?;
        Ok(Self { directory, exe })
    }

    fn path(&self, name: &str) -> PathBuf {
        self.directory.path().join(name)
    }

    fn installed(&self, ini: &[u8]) -> Result<()> {
        let dll = synthetic_pe(true, 2);
        fs::write(self.path("version.dll"), &dll)?;
        fs::write(self.path(core::INI), ini)?;
        let record = core::Record {
            schema: 3,
            backend: "upstream_sm86".into(),
            payload_version: Some("0.3.5".into()),
            proxy: "version.dll".into(),
            proxies: vec!["version.dll".into()],
            hashes: BTreeMap::from([
                ("version.dll".into(), core::hash(&dll)),
                (core::INI.into(), core::hash(ini)),
            ]),
            cleanup_dirs: BTreeMap::new(),
            scheme_id: Some(SCHEME.into()),
            delta_cache_ids: Vec::new(),
            delta_legacy_cache: false,
            cache_pending: false,
        };
        record.validate()?;
        core::atomic_json(&self.path(core::OWN).join(core::MARKER), &record)
    }

    fn snapshot(&self) -> Result<BTreeMap<PathBuf, Vec<u8>>> {
        fn visit(root: &Path, path: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) -> Result<()> {
            for entry in fs::read_dir(path)? {
                let path = entry?.path();
                if path.is_dir() {
                    visit(root, &path, files)?;
                } else {
                    files.insert(path.strip_prefix(root)?.into(), fs::read(path)?);
                }
            }
            Ok(())
        }
        let mut files = BTreeMap::new();
        visit(self.directory.path(), self.directory.path(), &mut files)?;
        Ok(files)
    }
}

fn context(profile: &str) -> presets::Context {
    presets::Context {
        scheme: SCHEME.into(),
        profile: profile.into(),
        delta: false,
        delta_capable: false,
    }
}

fn prevent_writes_and_removal(path: &Path) -> Result<File> {
    // FILE_SHARE_READ: reads are allowed, but writes and renames/deletes are not.
    Ok(OpenOptions::new().read(true).share_mode(1).open(path)?)
}

#[test]
fn apply_parameters_uses_the_selected_protocol_and_only_changes_ini() -> Result<()> {
    for (profile, key, ini_key, value) in [
        (
            "upstream035",
            "max_generated_frames",
            "MaxGeneratedFrames",
            "2",
        ),
        (
            "native026",
            "max_generated_frames",
            "MaxGeneratedFrames",
            "1",
        ),
        (
            presets::MFG_VULKAN,
            "max_interpolated_frames",
            "MaxInterpolatedFrames",
            "4",
        ),
    ] {
        let game = Game::new()?;
        game.installed(ORIGINAL_INI.as_bytes())?;
        let before = game.snapshot()?;
        let _dll_guard = prevent_writes_and_removal(&game.path("version.dll"))?;
        let _game_guard = prevent_writes_and_removal(&game.exe)?;
        let values = BTreeMap::from([(key.into(), value.into())]);
        core::apply_parameters(&game.exe, &context(profile), &values)?;
        let after = game.snapshot()?;
        assert_eq!(
            before.keys().collect::<Vec<_>>(),
            after.keys().collect::<Vec<_>>()
        );
        for (path, contents) in &before {
            if path != Path::new(core::INI) {
                assert_eq!(after[path], *contents, "unexpected modification: {path:?}");
            }
        }
        let text = String::from_utf8(after[Path::new(core::INI)].clone())?;
        assert!(text.contains("; 用户保留的注释\r\n"));
        assert!(text.contains("GameSpecific=keep\r\n"));
        assert!(text.contains("[PrivateGameSection]\r\nUntouched=42\r\n"));
        assert!(text.contains("CacheDirectory=\r\n"));
        assert_eq!(
            diagnostics::ini_value(&text, "FrameGeneration", ini_key)?,
            Some(value.into())
        );
        let other_protocol_key = if profile == presets::MFG_VULKAN {
            "MaxGeneratedFrames"
        } else {
            "MaxInterpolatedFrames"
        };
        assert_eq!(
            diagnostics::ini_value(&text, "FrameGeneration", other_protocol_key)?,
            None
        );
        assert!(core::status(&game.exe).starts_with("已部署"));
    }
    Ok(())
}

#[test]
fn apply_keeps_manual_ini_edits_and_utf16_encoding() -> Result<()> {
    let game = Game::new()?;
    game.installed(ORIGINAL_INI.as_bytes())?;
    let manual = format!("{ORIGINAL_INI}; 安装后手工追加\r\nUserOverride=retain\r\n");
    let bytes = [255, 254]
        .into_iter()
        .chain(manual.encode_utf16().flat_map(u16::to_le_bytes))
        .collect::<Vec<_>>();
    fs::write(game.path(core::INI), &bytes)?;
    assert!(core::status(&game.exe).contains("配置已修改"));
    core::apply_parameters(
        &game.exe,
        &context("upstream035"),
        &BTreeMap::from([("logging_level".into(), "3".into())]),
    )?;
    let result = fs::read(game.path(core::INI))?;
    assert!(result.starts_with(&[255, 254]));
    let (text, _) = diagnostics::decode_ini(&result)?;
    assert!(text.contains("; 安装后手工追加\r\nUserOverride=retain\r\n"));
    assert_eq!(
        diagnostics::ini_value(&text, "Logging", "Level")?,
        Some("3".into())
    );
    Ok(())
}

#[test]
fn same_protocol_does_not_authorize_editing_another_deployed_scheme() -> Result<()> {
    let game = Game::new()?;
    game.installed(ORIGINAL_INI.as_bytes())?;
    let before = game.snapshot()?;
    let mut other = context("upstream035");
    other.scheme = "different-scheme-same-parameter-protocol".into();
    let error = core::apply_parameters(
        &game.exe,
        &other,
        &BTreeMap::from([("logging_level".into(), "3".into())]),
    )
    .unwrap_err();
    assert!(error.to_string().contains("方案不同"));
    assert_eq!(game.snapshot()?, before);
    Ok(())
}

#[test]
fn changed_proxy_and_invalid_parameters_cannot_modify_the_ini() -> Result<()> {
    let game = Game::new()?;
    game.installed(ORIGINAL_INI.as_bytes())?;
    let before = game.snapshot()?;
    assert!(
        core::apply_parameters(
            &game.exe,
            &context("upstream035"),
            &BTreeMap::from([("logging_level".into(), "99".into())])
        )
        .is_err()
    );
    assert_eq!(game.snapshot()?, before);
    fs::write(game.path("version.dll"), synthetic_pe(true, 9))?;
    let changed = game.snapshot()?;
    let error = core::apply_parameters(
        &game.exe,
        &context("upstream035"),
        &BTreeMap::from([("logging_level".into(), "3".into())]),
    )
    .unwrap_err();
    assert!(error.to_string().contains("补丁文件已变化"));
    assert_eq!(game.snapshot()?, changed);
    Ok(())
}

#[test]
fn early_preflight_refuses_unrecognized_files_and_directory_collisions() -> Result<()> {
    let game = Game::new()?;
    let proxies = vec!["version.dll".into()];
    let initial = game.snapshot()?;
    core::preflight_install(&game.exe, &proxies)?;
    assert_eq!(game.snapshot()?, initial);
    fs::write(game.path("version.dll"), synthetic_pe(true, 9))?;
    let with_conflict = game.snapshot()?;
    assert!(core::preflight_install(&game.exe, &proxies).is_err());
    assert_eq!(game.snapshot()?, with_conflict);
    fs::create_dir(game.path("dxgi.dll"))?;
    assert!(core::preflight_install(&game.exe, &["dxgi.dll".into()]).is_err());
    assert!(game.path("dxgi.dll").is_dir());
    assert_eq!(game.snapshot()?, with_conflict);
    Ok(())
}

#[test]
fn early_preflight_checks_current_identity_even_when_a_record_names_the_entry() -> Result<()> {
    let game = Game::new()?;
    game.installed(ORIGINAL_INI.as_bytes())?;
    core::preflight_install(&game.exe, &["version.dll".into()])?;
    fs::write(game.path("version.dll"), synthetic_pe(true, 9))?;
    let before = game.snapshot()?;
    assert!(core::preflight_install(&game.exe, &["version.dll".into()]).is_err());
    assert_eq!(game.snapshot()?, before);
    Ok(())
}

#[test]
fn uninstall_after_parameter_edit_removes_only_recorded_patch_files() -> Result<()> {
    let game = Game::new()?;
    game.installed(ORIGINAL_INI.as_bytes())?;
    fs::write(game.path("d3d12.dll"), synthetic_pe(true, 8))?;
    let before = game.snapshot()?;
    core::apply_parameters(
        &game.exe,
        &context("upstream035"),
        &BTreeMap::from([("logging_level".into(), "3".into())]),
    )?;
    cleanup::clean(&game.exe)?;
    assert!(!game.path("version.dll").exists());
    assert!(!game.path(core::INI).exists());
    assert!(!game.path(core::OWN).join(core::MARKER).exists());
    let remaining = game.snapshot()?;
    for name in [
        "游戏本体.exe",
        "original-game.dll",
        "Saved/progress.dat",
        "d3d12.dll",
    ] {
        assert_eq!(
            remaining[Path::new(name)],
            before[Path::new(name)],
            "game file changed: {name}"
        );
    }
    cleanup::clean(&game.exe)?;
    assert_eq!(game.snapshot()?, remaining);
    Ok(())
}
