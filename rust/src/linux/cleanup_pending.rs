//! Keep unknown leftovers and recovery records when Win32 handle identity is unavailable.
use crate::core::DeploymentTarget;
use anyhow::Result;
use std::path::Path;
#[derive(Clone, Debug)]
pub struct Plan {
    target: DeploymentTarget,
}
impl Plan {
    pub fn target(&self) -> &DeploymentTarget {
        &self.target
    }
    pub fn paths(&self) -> impl Iterator<Item = &Path> {
        std::iter::empty()
    }
}
pub fn prepare(target: &DeploymentTarget) -> Result<Plan> {
    prepare_at(target, None)
}
pub fn prepare_at(_target: &DeploymentTarget, _cache_root: Option<&Path>) -> Result<Plan> {
    anyhow::bail!("Linux 暂不支持额外残留清理，已保留恢复记录；请关闭游戏后重试正常卸载")
}
pub fn delete_confirmed(_plan: &Plan) -> Result<()> {
    anyhow::bail!("Linux 暂不支持额外残留清理，未删除文件")
}
