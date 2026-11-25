//! Where Winds Meet's optional native NR companion. The frame-generation
//! package keeps version.dll; the companion owns a separate DXGI entry.
use super::*;

pub(super) const KIND: &str = "yanyun-nr";
const FILES: [(&str, &str); 3] = [
    (
        "dxgi.dll",
        "34735755119eebb7ffea38ba99e57a7652fbc204a2863efa7fed3b725fdaeaca",
    ),
    (
        "nvngx_dlssnr.dll",
        "0cfba6ea095929eed6c61832e617282e5c438314971cbe9af15629489225a4a7",
    ),
    (
        "OptiScaler.ini",
        "2925e7d5a11a6e45977b58ad39e67ce3d4d63b0dd9f8a76f8b6b205dce88aadf",
    ),
];

fn game(target: &core::DeploymentTarget) -> bool {
    !target.is_custom()
        && target
            .game_exe
            .file_name()
            .is_some_and(|n| n.eq_ignore_ascii_case("yysls.exe"))
}

pub fn eligible(target: &core::DeploymentTarget, scheme: &str, version: &str) -> bool {
    cfg!(windows)
        && game(target)
        && matches!(
            (scheme, version),
            (crate::upstream040::SCHEME, "0.4.2")
                | (crate::cloud::DELTA_041_SCHEME, crate::delta::MILES_R3)
        )
}

pub fn version_entry(proxies: &[String]) -> bool {
    proxies.len() == 1 && proxies[0].eq_ignore_ascii_case("version.dll")
}

pub(super) fn valid_destination(target: &core::DeploymentTarget, path: &Path) -> bool {
    game(target)
        && path
            .parent()
            .is_some_and(|p| core::key(p) == core::key(&target.directory))
        && path
            .file_name()
            .is_some_and(|n| FILES.iter().any(|(name, _)| n.eq_ignore_ascii_case(name)))
}

fn installed(target: &core::DeploymentTarget) -> Result<bool> {
    let Some(record) = read(target, KIND)?.filter(|r| r.complete && r.entries.len() == FILES.len())
    else {
        return Ok(false);
    };
    for (name, hash) in FILES {
        let path = core::no_links(&target.directory.join(name))?;
        if !record
            .entries
            .iter()
            .any(|e| core::key(Path::new(&e.path)) == core::key(&path) && e.installed == hash)
        {
            return Ok(false);
        }
        // The mod may rewrite its INI. Preserve it on reinstallation.
        if name == "OptiScaler.ini" {
            if !path.is_file() || path.metadata()?.len() > 1024 * 1024 {
                return Ok(false);
            }
        } else if !matches(&path, hash)? {
            return Ok(false);
        }
    }
    Ok(true)
}

pub struct Component {
    sources: BTreeMap<String, PathBuf>,
}

fn package() -> Result<Package> {
    Ok(serde_json::from_str(include_str!(
        "../../assets/yanyun-nr.json"
    ))?)
}

pub fn download_file() -> Result<transfer::DownloadFile> {
    Ok(package()?.download_file())
}

impl Component {
    pub fn prepare(
        github: bool,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(transfer::Progress),
    ) -> Result<Self> {
        let sources = prepare_package(package()?, github, cancel, progress)?;
        Ok(Self { sources })
    }

    fn paths(&self, target: &core::DeploymentTarget) -> Vec<PathBuf> {
        FILES
            .iter()
            .map(|(name, _)| target.directory.join(name))
            .collect()
    }

    pub fn preflight(
        &self,
        target: &core::DeploymentTarget,
        scheme: &str,
        version: &str,
        proxies: &[String],
    ) -> Result<bool> {
        ensure!(
            eligible(target, scheme, version) && version_entry(proxies),
            "请在燕云十六声中选择 0.4.2 原版或三角洲 R3，并只使用 version.dll 入口"
        );
        ensure!(
            core::no_links(&target.directory.join("Streamline").join("sl.dlss_nr.dll"))?.is_file(),
            "未找到游戏自带的神经渲染插件，请先更新或修复游戏"
        );
        ensure!(
            !core::no_links(&target.directory.join(".rtx-fg-nr"))?.exists(),
            "请先还原单独安装的神经渲染运行库"
        );
        let dxgi = core::no_links(&target.directory.join("dxgi.dll"))?;
        ensure!(
            !dxgi.exists() || matches(&dxgi, FILES[0].1)?,
            "dxgi.dll 已被其他补丁占用，请先卸载该补丁"
        );
        let log = core::no_links(&target.directory.join(LOG))?;
        ensure!(
            !log.try_exists()? || log.is_file(),
            "OptiScaler.log 被目录占用"
        );
        let already_installed = installed(target)?;
        if !already_installed {
            plan_entries(target, KIND, &self.paths(target), &self.sources)?;
        }
        Ok(already_installed)
    }

    pub fn install(&self, target: &core::DeploymentTarget) -> Result<()> {
        let target = target.validate(true)?;
        let _lock = win::game_lock(&target.directory)?;
        let record = core::record(&target.directory)?.context("请先安装帧生成补丁")?;
        target.validate_record(&record)?;
        ensure!(!record.cache_pending, "请先完成补丁卸载");
        if self.preflight(
            &target,
            record.scheme_id.as_deref().unwrap_or_default(),
            record
                .upstream_version
                .as_deref()
                .or(record.payload_version.as_deref())
                .unwrap_or_default(),
            &record.selected(),
        )? {
            return Ok(());
        }
        install_locked_with(
            &target,
            KIND,
            &self.paths(&target),
            &self.sources,
            |source, dest| {
                let expected = FILES
                    .iter()
                    .find(|(name, _)| {
                        dest.file_name()
                            .is_some_and(|n| n.eq_ignore_ascii_case(name))
                    })
                    .context("组件文件名无效")?
                    .1;
                let mut temp = tempfile::NamedTempFile::new_in(&target.directory)?;
                std::io::copy(&mut fs::File::open(core::no_links(source)?)?, &mut temp)?;
                temp.as_file().sync_all()?;
                ensure!(matches(temp.path(), expected)?, "燕云组件暂存校验失败");
                temp.persist(core::no_links(dest)?)?;
                Ok(())
            },
        )
    }
}

const LOG: &str = "OptiScaler.log";
const LOG_STATE: &str = "log-before.json";
const LOG_BACKUP: &str = "OptiScaler.log.bak";

pub(super) fn snapshot_log(target: &core::DeploymentTarget, folder: &Path) -> Result<()> {
    let path = core::no_links(&target.directory.join(LOG))?;
    let before = if path.try_exists()? {
        ensure!(path.is_file(), "OptiScaler.log 被目录占用");
        let backup = core::no_links(&folder.join(LOG_BACKUP))?;
        replace(&path, &backup)?;
        Some(core::digest(&backup)?)
    } else {
        None
    };
    core::atomic_json(&core::no_links(&folder.join(LOG_STATE))?, &before)
}

fn generated_log(path: &Path) -> Result<bool> {
    let mut header = Vec::new();
    fs::File::open(path)?.take(8192).read_to_end(&mut header)?;
    Ok(String::from_utf8_lossy(&header)
        .lines()
        .take(12)
        .any(|s| s.contains("DLSS Unlocked - OptiScaler v0.9.96") && s.contains(" loaded")))
}

pub(super) fn restore_log(
    target: &core::DeploymentTarget,
    folder: &Path,
    complete: bool,
) -> Result<()> {
    let state = core::no_links(&folder.join(LOG_STATE))?;
    let path = core::no_links(&target.directory.join(LOG))?;
    let owned = path.is_file() && generated_log(&path)?;
    if !state.try_exists()? {
        // The first local 4.3.44 test build did not record the previous log.
        // Its own header and creation after installation identify a new log.
        if complete
            && owned
            && let (Ok(created), Ok(installed)) =
                (path.metadata()?.created(), folder.metadata()?.created())
            && created > installed
        {
            fs::remove_file(path)?;
        }
        return Ok(());
    }
    let original: Option<String> = serde_json::from_value(core::read_json(&state, 256)?)?;
    // A game crash can leave its freshly opened log empty.
    let owned = owned || (path.is_file() && path.metadata()?.len() == 0);
    if let Some(hash) = original {
        let backup = core::no_links(&folder.join(LOG_BACKUP))?;
        ensure!(
            core::valid_hash(&hash) && core::digest(&backup)? == hash,
            "原有 OptiScaler.log 备份校验失败"
        );
        if !path.try_exists()? || owned {
            replace(&backup, &path)?;
        } else {
            ensure!(
                path.is_file() && core::digest(&path)? == hash,
                "OptiScaler.log 已被其他程序替换，原件备份已保留"
            );
        }
    } else if owned {
        fs::remove_file(path)?;
    }
    Ok(())
}

pub(super) fn forget_log(folder: &Path) -> Result<()> {
    for name in [LOG_STATE, LOG_BACKUP] {
        let p = core::no_links(&folder.join(name))?;
        if p.try_exists()? {
            fs::remove_file(p)?;
        }
    }
    Ok(())
}

pub fn remove_locked(target: &core::DeploymentTarget) -> Result<()> {
    if !game(target) {
        return Ok(());
    }
    // OptiScaler can save its INI while the game is running. Preserve that
    // revision before the existing journal restores the pre-install state.
    if let Some(mut record) = read(target, KIND)?
        && record.complete
        && let Some(entry) = record.entries.iter_mut().find(|e| {
            Path::new(&e.path)
                .file_name()
                .is_some_and(|n| n.eq_ignore_ascii_case("OptiScaler.ini"))
        })
    {
        let ini = core::no_links(Path::new(&entry.path))?;
        if !accepted(&ini, entry)? {
            ensure!(
                ini.is_file() && ini.metadata()?.len() <= 1024 * 1024,
                "燕云配置文件异常，已保留"
            );
            let hash = core::digest(&ini)?;
            let saved =
                crate::config_store::history_root(target)?.join(format!("OptiScaler-{hash}.ini"));
            replace(&ini, &saved)?;
            ensure!(matches(&saved, &hash)?, "燕云配置备份校验失败");
            entry.previous = Some(hash);
            core::atomic_json(&root(target, KIND)?.join("ownership.json"), &record)?;
        }
    }
    super::remove_locked(target, KIND)
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use crate::{cleanup, cloud, delta, presets, upstream040};

    #[test]
    fn scope_requires_game_revision_and_separate_entries() {
        let t = core::DeploymentTarget::for_game(Path::new(r"C:\Game\yysls.exe"));
        assert!(eligible(&t, upstream040::SCHEME, "0.4.2"));
        assert!(eligible(&t, cloud::DELTA_041_SCHEME, delta::MILES_R3));
        assert!(!eligible(
            &t,
            cloud::DELTA_041_SCHEME,
            delta::MILES_INI_VERSION
        ));
        assert!(!eligible(&t, upstream040::PIPO_SCHEME, "0.4.2"));
        assert!(!eligible(
            &core::DeploymentTarget::for_game(Path::new(r"C:\Other\game.exe")),
            upstream040::SCHEME,
            "0.4.2"
        ));
        assert!(version_entry(&["version.dll".into()]));
        assert!(!version_entry(&["version.dll".into(), "dxgi.dll".into()]));
        assert!(!valid_destination(
            &t,
            &t.directory.join("Streamline/sl.dlss_nr.dll")
        ));
    }

    #[test]
    fn download_manifest_matches_the_three_allowed_game_files() -> Result<()> {
        let bundle = package()?;
        for (name, hash) in FILES {
            assert!(
                bundle
                    .files
                    .iter()
                    .any(|f| f.name == name && f.sha256 == hash)
            );
        }
        assert_eq!(bundle.kind, KIND);
        assert_eq!(bundle.urls.len(), 2);
        Ok(())
    }

    #[test]
    fn unrelated_replacement_log_is_preserved() -> Result<()> {
        let game = tempfile::tempdir()?;
        let record = tempfile::tempdir()?;
        let target = core::DeploymentTarget::for_game(&game.path().join("yysls.exe"));
        snapshot_log(&target, record.path())?;
        let log = game.path().join(LOG);
        fs::write(&log, b"other program")?;
        restore_log(&target, record.path(), true)?;
        assert_eq!(fs::read(&log)?, b"other program");
        fs::write(&log, b"")?;
        restore_log(&target, record.path(), true)?;
        assert!(!log.exists());
        Ok(())
    }

    #[test]
    fn interrupted_three_file_install_restores_files_and_removes_journal() -> Result<()> {
        let game = tempfile::tempdir()?;
        let bundle = tempfile::tempdir()?;
        let exe = game.path().join("yysls.exe");
        fs::copy(std::env::current_exe()?, &exe)?;
        let target = core::DeploymentTarget::for_game(&exe).validate(true)?;
        let mut sources = BTreeMap::new();
        let mut paths = Vec::new();
        for (name, _) in FILES {
            let source = bundle.path().join(name);
            fs::write(&source, b"test component")?;
            sources.insert(name.to_owned(), source);
            paths.push(game.path().join(name));
        }
        fs::write(&paths[1], b"original runtime")?;
        let result = install_locked_with(&target, KIND, &paths, &sources, |source, dest| {
            ensure!(dest != paths[2], "injected write failure");
            replace(source, dest)
        });
        assert!(result.is_err());
        assert!(!paths[0].exists() && !paths[2].exists());
        assert_eq!(fs::read(&paths[1])?, b"original runtime");
        assert!(!root(&target, KIND)?.exists());
        Ok(())
    }

    // Uses the signed distribution archives from the release
    // workspace. No game is started. The caller supplies an isolated cache.
    #[test]
    #[ignore = "requires all three locally cached signed distribution archives"]
    fn signed_packages_install_retry_and_full_uninstall() -> Result<()> {
        let cache = PathBuf::from(std::env::var("RTXFG_YANYUN_QA")?);
        ensure!(
            PathBuf::from(std::env::var("LOCALAPPDATA")?).starts_with(&cache),
            "use isolated cache"
        );
        let addon = Component::prepare(false, &AtomicBool::new(false), &mut |p| {
            assert_ne!(
                p.phase,
                transfer::Phase::Downloading,
                "use isolated offline fixtures"
            );
        })?;
        let catalog = cloud::bundled();
        let proxies = vec![String::from("version.dll")];
        for scheme in [upstream040::SCHEME, cloud::DELTA_041_SCHEME] {
            let game = tempfile::tempdir_in(&cache)?;
            let exe = game.path().join("yysls.exe");
            fs::copy(std::env::current_exe()?, &exe)?;
            let target = core::DeploymentTarget::for_game(&exe).validate(true)?;
            let sl = game.path().join("Streamline");
            fs::create_dir(&sl)?;
            fs::write(sl.join("sl.dlss_nr.dll"), b"game's plugin")?;
            fs::write(sl.join("nvngx_dlssnr.dll"), b"game's native runtime")?;
            let first_ini = b"old settings";
            let root_nr = game.path().join("nvngx_dlssnr.dll");
            let ini = game.path().join("OptiScaler.ini");
            let log = game.path().join(LOG);
            fs::write(&log, b"previous log")?;
            fs::write(&ini, first_ini)?;
            fs::write(&root_nr, b"preexisting runtime")?;
            let payload = cloud::prepare(
                &catalog,
                scheme,
                1,
                &proxies,
                &AtomicBool::new(false),
                |_| {},
            )?;
            let ctx = presets::Context::new(scheme, &payload.policy, &exe);
            let version = payload
                .upstream_version
                .as_deref()
                .unwrap_or(&payload.version);
            fs::write(game.path().join("dxgi.dll"), b"another mod")?;
            assert!(addon.preflight(&target, scheme, version, &proxies).is_err());
            assert_eq!(fs::read(game.path().join("dxgi.dll"))?, b"another mod");
            fs::remove_file(game.path().join("dxgi.dll"))?;
            addon.preflight(&target, scheme, version, &proxies)?;
            let install_base = || {
                core::deploy_prepared_context_at_with_overrides(
                    &target,
                    &payload.backend,
                    &proxies,
                    None,
                    payload.files.clone(),
                    Some(&payload.version),
                    Some(&ctx),
                    None,
                    payload.upstream_version.as_deref(),
                )
            };
            install_base()?;
            assert!(!game.path().join("dxgi.dll").exists());
            assert_eq!(fs::read(&ini)?, first_ini);
            assert!(!root(&target, KIND)?.exists());
            assert!(cleanup::clean_outcome_at(&target)?.complete);
            assert_eq!(fs::read(&log)?, b"previous log");
            if scheme == upstream040::SCHEME {
                fs::remove_file(&log)?;
            }
            install_base()?;
            addon.install(&target)?;
            for (name, hash) in FILES {
                assert!(matches(&game.path().join(name), hash)?);
            }
            fs::write(&ini, b"game changed the ini")?;
            fs::write(
                &log,
                b"[23:53:09] [W] DLSS Unlocked - OptiScaler v0.9.96-pre3 (unknown) loaded\n",
            )?;
            fs::write(game.path().join("game.log"), b"keep game's log")?;
            addon.install(&target)?;
            assert_eq!(fs::read(&ini)?, b"game changed the ini");
            assert!(cleanup::clean_outcome_at(&target)?.complete);
            assert!(!game.path().join("dxgi.dll").exists());
            assert!(!game.path().join("version.dll").exists());
            assert!(!root(&target, KIND)?.exists());
            assert_eq!(fs::read(&ini)?, first_ini);
            assert_eq!(fs::read(root_nr)?, b"preexisting runtime");
            if scheme == upstream040::SCHEME {
                assert!(!log.exists());
            } else {
                assert_eq!(fs::read(log)?, b"previous log");
            }
            assert_eq!(fs::read(game.path().join("game.log"))?, b"keep game's log");
            assert_eq!(fs::read(sl.join("sl.dlss_nr.dll"))?, b"game's plugin");
            assert_eq!(
                fs::read(sl.join("nvngx_dlssnr.dll"))?,
                b"game's native runtime"
            );
        }
        Ok(())
    }
}
