use crate::core;
use anyhow::{Context, Result, ensure};
use std::{io::Read, path::PathBuf};
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
