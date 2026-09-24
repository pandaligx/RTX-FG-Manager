use crate::core;
use anyhow::{Context, Result, ensure};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
};
pub struct Embedded {
    pub name: &'static str,
    pub sha256: &'static str,
    pub size: usize,
    pub compressed: &'static [u8],
}
include!(concat!(env!("OUT_DIR"), "/embedded.rs"));
pub fn bytes(name: &str) -> Result<Vec<u8>> {
    let r = EMBEDDED
        .iter()
        .find(|r| r.name == name)
        .context("安装包文件缺失")?;
    let mut bytes = Vec::with_capacity(r.size);
    flate2::read::ZlibDecoder::new(r.compressed)
        .take(r.size as u64 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() == r.size && core::hash(&bytes) == r.sha256,
        "内置资源损坏"
    );
    Ok(bytes)
}
pub fn cache_root() -> Result<PathBuf> {
    Ok(
        PathBuf::from(std::env::var_os("LOCALAPPDATA").context("LOCALAPPDATA unavailable")?)
            .join("RTXFGManager")
            .join("runtime-rust"),
    )
}
fn write_resource(path: &Path, bytes: &[u8]) -> Result<()> {
    let path = core::no_links(path)?;
    let parent = path.parent().context("无效资源路径")?;
    // Closing during a startup download may terminate this worker. Only publish
    // the verified resource after its complete contents have reached the file;
    // a stopped extraction must never leave a partial executable at its name.
    let mut stage = tempfile::NamedTempFile::new_in(parent)?;
    stage.write_all(bytes)?;
    stage.as_file().sync_all()?;
    core::no_links(&path)?;
    stage.persist_noclobber(&path)?;
    Ok(())
}
pub fn tool() -> Result<PathBuf> {
    let root = cache_root()?.join(
        "aria2-".to_owned()
            + &EMBEDDED
                .iter()
                .find(|r| r.name == "app/tools/aria2c.exe")
                .unwrap()
                .sha256[..16],
    );
    core::no_links(&root)?;
    std::fs::create_dir_all(&root)?;
    let _lock = crate::win::resource_lock(&root)?;
    for name in ["aria2c.exe", "aria2.conf"] {
        let path = core::no_links(&root.join(name))?;
        let resource = format!("app/tools/{name}");
        let item = EMBEDDED.iter().find(|r| r.name == resource).unwrap();
        if path.exists() {
            ensure!(
                core::digest(&path)? == item.sha256,
                "下载工具已被修改，未执行"
            );
        } else {
            write_resource(&path, &bytes(&resource)?)?;
        }
    }
    Ok(root.join("aria2c.exe"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_commit_never_overwrites_existing_files_or_leaves_failed_stage() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("aria2c.exe");
        std::fs::write(&path, b"unknown existing file")?;
        assert!(write_resource(&path, b"replacement is forbidden").is_err());
        assert_eq!(std::fs::read(&path)?, b"unknown existing file");
        assert_eq!(std::fs::read_dir(directory.path())?.count(), 1);
        let new_path = directory.path().join("aria2.conf");
        write_resource(&new_path, b"complete verified resource")?;
        assert_eq!(std::fs::read(&new_path)?, b"complete verified resource");
        assert_eq!(std::fs::read_dir(directory.path())?.count(), 2);
        Ok(())
    }
}
