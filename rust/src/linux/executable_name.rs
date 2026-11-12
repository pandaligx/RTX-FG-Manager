//! Windows executable migration is deliberately unavailable in an AppImage.
use anyhow::Result;
use std::{path::Path, process::Command};
pub const FILE_NAME: &str = "RTXManager";
pub(crate) fn command(path: &Path) -> Command {
    let mut command = Command::new(path);
    command
        .env_remove("RTXFG_RENAME_READY")
        .env_remove("RTXFG_UPDATE_READY")
        .env_remove("RTXFG_RESET_READY");
    command
}
pub fn is_standard(_path: &Path) -> bool {
    true
}
pub fn signal_ready(_data: &Path) -> Result<()> {
    Ok(())
}
pub fn prepare(_data: &Path) -> Result<()> {
    anyhow::bail!("Linux AppImage 不需要统一 EXE 文件名")
}
pub fn apply(_path: &Path) -> Result<()> {
    anyhow::bail!("Linux AppImage 不支持 Windows 文件名迁移")
}
