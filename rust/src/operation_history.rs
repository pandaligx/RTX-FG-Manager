//! Local per-game operation history; never sent to the community service.
use crate::{config_store, core};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::fs;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub created: String,
    pub action: String,
    pub scheme: String,
    pub version: String,
    pub complete: bool,
    pub detail: String,
}
pub fn append(target: &core::DeploymentTarget, entry: &Entry) -> Result<()> {
    ensure!(entry.detail.len() <= 16384, "操作记录过长");
    let root = core::no_links(&config_store::history_root(target)?.join("operations"))?;
    fs::create_dir_all(&root)?;
    let path = root.join(format!("{}.json", uuid::Uuid::new_v4()));
    core::write_new(&path, &serde_json::to_vec(entry)?)
}
pub fn read(target: &core::DeploymentTarget) -> Result<Vec<Entry>> {
    let root = core::no_links(&config_store::history_root(target)?.join("operations"))?;
    if !root.is_dir() {
        return Ok(Vec::new());
    }
    let mut result = Vec::new();
    for item in fs::read_dir(root)? {
        let path = core::no_links(&item?.path())?;
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let entry: Entry = serde_json::from_value(core::read_json(&path, 32768)?)?;
        result.push(entry);
    }
    result.sort_by(|a, b| b.created.cmp(&a.created));
    result.truncate(200);
    Ok(result)
}
