//! OptiScaler's private DLSS-G output uses the same six production binaries
//! as Encore's added-FG package, in a different directory and ownership journal.
use super::*;

pub const KIND: &str = "optiscaler-sl";
pub const VERSION: &str = "2.14.16";
const DIRECTORY: &str = "OptiScaler/streamline";

fn package() -> Result<Package> {
    let p = bundled_package("encorefg")?;
    ensure!(p.version == VERSION, "OptiScaler Streamline 版本不匹配");
    Ok(p)
}

pub(crate) fn verify_bundle_members(files: &BTreeMap<String, Vec<u8>>) -> Result<()> {
    #[cfg(windows)]
    let temp = tempfile::tempdir()?;
    for asset in package()?
        .files
        .into_iter()
        .filter(|a| a.name.ends_with(".dll"))
    {
        let bytes = files
            .get(&format!("{DIRECTORY}/{}", asset.name))
            .context("补丁包缺少配套 Streamline 文件")?;
        ensure!(
            bytes.len() as u64 == asset.bytes && core::hash(bytes) == asset.sha256,
            "Streamline 原件校验失败：{}",
            asset.name
        );
        #[cfg(windows)]
        {
            let path = temp.path().join(&asset.name);
            fs::write(&path, bytes)?;
            verify_signer(&asset.name, &path)?;
        }
    }
    Ok(())
}

#[cfg(windows)]
fn verify_signer(name: &str, path: &Path) -> Result<()> {
    let signer = if name == "nvngx_dlssg.dll" {
        "7B7B0B6697AFB438CF6F65A155F00E86676FB186"
    } else {
        "B66776FC8E70C58ED98199E8391264C827AAC534"
    };
    ensure!(
        win::authenticode_thumbprint(path)? == signer,
        "NVIDIA 文件签名不符：{name}"
    );
    Ok(())
}

pub fn download_file() -> Result<transfer::DownloadFile> {
    let mut file = package()?.download_file();
    file.name = format!("Streamline {VERSION}");
    Ok(file)
}

/// Reuse the runtime window's ZIP, cache, checksum and selected mirror for a patch install.
pub(crate) fn bundle_files(
    github: bool,
    cancel: &AtomicBool,
    report: &mut impl FnMut(transfer::Progress),
) -> Result<BTreeMap<String, Vec<u8>>> {
    let file = download_file()?;
    let package = package()?;
    let members = package.files.clone();
    let sources = prepare_package(package, github, cancel, &mut |mut p| {
        if p.queue.is_empty() {
            p.item.clone_from(&file.name);
            p.file = Some(file.clone());
            report(p);
        }
    })?;
    let mut files = BTreeMap::new();
    for member in members.iter().filter(|m| m.name.ends_with(".dll")) {
        ensure!(!cancel.load(Ordering::Relaxed), "下载已取消");
        let path = sources
            .get(&member.name)
            .context("配套 Streamline 缓存缺失")?;
        let mut data = Vec::new();
        fs::File::open(core::no_links(path)?)?
            .take(member.bytes + 1)
            .read_to_end(&mut data)?;
        files.insert(format!("{DIRECTORY}/{}", member.name), data);
    }
    verify_bundle_members(&files)?;
    Ok(files)
}

pub(super) fn valid_destination(target: &core::DeploymentTarget, path: &Path) -> bool {
    path.parent()
        .is_some_and(|p| core::key(p) == core::key(&target.directory.join(DIRECTORY)))
        && path.file_name().is_some_and(|n| {
            ENCORE_FG_FILES
                .iter()
                .any(|s| s.ends_with(".dll") && n.eq_ignore_ascii_case(s))
        })
}

pub fn installed(target: &core::DeploymentTarget) -> Result<bool> {
    let legacy = core::record(&target.directory)?.filter(|r| {
        r.backend == crate::optiscaler_nr::BACKEND
            && r.hashes
                .keys()
                .any(|k| k.starts_with("OptiScaler/streamline/"))
    });
    if legacy.is_none() && !read(target, KIND)?.is_some_and(|r| r.complete && r.entries.len() == 6)
    {
        return Ok(false);
    }
    if let Some(record) = &legacy {
        target.validate_record(record)?;
    }
    for asset in package()?
        .files
        .into_iter()
        .filter(|a| a.name.ends_with(".dll"))
    {
        let relative = format!("{DIRECTORY}/{}", asset.name);
        if let Some(r) = &legacy {
            ensure!(
                r.hashes.get(&relative) == Some(&asset.sha256),
                "Streamline 原安装记录不匹配"
            );
        }
        ensure!(
            matches(&target.directory.join(relative), &asset.sha256)?,
            "Streamline 文件已变化，请先检查现有安装"
        );
    }
    Ok(true)
}

pub struct Component(BTreeMap<String, PathBuf>);
impl Component {
    pub fn prepare(
        github: bool,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(transfer::Progress),
    ) -> Result<Self> {
        let mut sources = prepare_package(package()?, github, cancel, progress)?;
        sources.retain(|name, _| name.ends_with(".dll"));
        ensure!(sources.len() == 6, "Streamline 文件数量不符");
        #[cfg(windows)]
        for (name, path) in &sources {
            verify_signer(name, path)?;
        }
        Ok(Self(sources))
    }

    pub fn install(&self, target: &core::DeploymentTarget) -> Result<()> {
        let t = target.validate(true)?;
        let _lock = win::game_lock(&t.directory)?;
        let r = core::record(&t.directory)?.context("请先安装 OptiScaler")?;
        t.validate_record(&r)?;
        ensure!(
            r.backend == crate::optiscaler_nr::BACKEND && !r.cache_pending,
            "请先安装 OptiScaler"
        );
        if installed(&t)? {
            return Ok(());
        }
        let paths = self
            .0
            .keys()
            .map(|n| t.directory.join(DIRECTORY).join(n))
            .collect::<Vec<_>>();
        let members = package()?.files;
        install_locked_with(&t, KIND, &paths, &self.0, |source, dest| {
            let asset = members
                .iter()
                .find(|a| {
                    dest.file_name()
                        .is_some_and(|n| n.eq_ignore_ascii_case(&a.name))
                })
                .context("Streamline 文件名无效")?;
            let parent = core::no_links(dest.parent().context("Streamline 目录无效")?)?;
            fs::create_dir_all(&parent)?;
            let mut stage = tempfile::NamedTempFile::new_in(core::no_links(&parent)?)?;
            std::io::copy(
                &mut fs::File::open(core::no_links(source)?)?.take(asset.bytes + 1),
                &mut stage,
            )?;
            stage.as_file().sync_all()?;
            ensure!(
                matches(stage.path(), &asset.sha256)?,
                "Streamline 暂存文件校验失败"
            );
            stage.persist(core::no_links(dest)?)?;
            Ok(())
        })
    }
}
