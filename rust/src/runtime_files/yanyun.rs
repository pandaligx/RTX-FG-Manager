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

impl Component {
    pub fn bundled() -> Result<Self> {
        let app = std::env::current_exe()?;
        Self::from_directory(&app.with_file_name("components").join(KIND))
    }

    fn from_directory(folder: &Path) -> Result<Self> {
        let mut sources = BTreeMap::new();
        for (name, expected) in FILES {
            let path = core::no_links(&folder.join(name))?;
            ensure!(
                matches(&path, expected)?,
                "燕云神经渲染组件缺失或校验失败，请完整解压测试包：{name}"
            );
            sources.insert(name.to_owned(), path);
        }
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
    fn bad_bundle_is_rejected_before_any_game_write() -> Result<()> {
        let dir = tempfile::tempdir()?;
        fs::write(dir.path().join("dxgi.dll"), b"wrong file")?;
        assert!(Component::from_directory(dir.path()).is_err());
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

    // Uses the distribution archives and exact signed sidecar from the release
    // workspace. No game is started. The caller supplies an isolated cache.
    #[test]
    #[ignore = "requires signed test bundle and both cached 0.4.2 packages"]
    fn signed_packages_install_retry_and_full_uninstall() -> Result<()> {
        let cache = PathBuf::from(std::env::var("RTXFG_YANYUN_QA")?);
        ensure!(
            PathBuf::from(std::env::var("LOCALAPPDATA")?).starts_with(&cache),
            "use isolated cache"
        );
        let addon =
            Component::from_directory(&PathBuf::from(std::env::var("RTXFG_YANYUN_BUNDLE")?))?;
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
            core::deploy_prepared_context_at_with_overrides(
                &target,
                &payload.backend,
                &proxies,
                None,
                payload.files,
                Some(&payload.version),
                Some(&ctx),
                None,
                payload.upstream_version.as_deref(),
            )?;
            assert!(!game.path().join("dxgi.dll").exists());
            assert_eq!(fs::read(&ini)?, first_ini);
            addon.install(&target)?;
            for (name, hash) in FILES {
                assert!(matches(&game.path().join(name), hash)?);
            }
            fs::write(&ini, b"game changed the ini")?;
            addon.install(&target)?;
            assert_eq!(fs::read(&ini)?, b"game changed the ini");
            assert!(cleanup::clean_outcome_at(&target)?.complete);
            assert!(!game.path().join("dxgi.dll").exists());
            assert!(!game.path().join("version.dll").exists());
            assert!(!root(&target, KIND)?.exists());
            assert_eq!(fs::read(&ini)?, first_ini);
            assert_eq!(fs::read(root_nr)?, b"preexisting runtime");
            assert_eq!(fs::read(sl.join("sl.dlss_nr.dll"))?, b"game's plugin");
            assert_eq!(
                fs::read(sl.join("nvngx_dlssnr.dll"))?,
                b"game's native runtime"
            );
        }
        Ok(())
    }
}
