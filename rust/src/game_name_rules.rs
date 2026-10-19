//! Declarative, updateable identity rules. No scripts, regexes or absolute paths.
use crate::{core, game_names};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::path::{Component, Path};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    id: String,
    game: String,
    executables: Vec<String>,
    #[serde(default)]
    directories: Vec<String>,
    #[serde(default)]
    files: Vec<String>,
    #[serde(default)]
    products: Vec<String>,
    #[serde(default)]
    keys: Vec<String>,
}
pub(crate) fn relative(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 240
        && !path.contains([':', '\\', '*', '?'])
        && path
            .split('/')
            .all(|s| !s.is_empty() && s != "." && s != "..")
        && Path::new(path)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
}
impl Rule {
    pub(crate) fn id(&self) -> &str {
        &self.id
    }
    pub(crate) fn game(&self) -> &str {
        &self.game
    }
    pub(crate) fn validate(&self) -> Result<()> {
        ensure!(
            !self.id.is_empty()
                && self.id.len() <= 80
                && self
                    .id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b)),
            "名称规则 ID 无效"
        );
        ensure!(
            !self.executables.is_empty() && self.executables.len() <= 16,
            "名称规则缺少程序名"
        );
        ensure!(
            self.executables.iter().all(|s| relative(s)
                && !s.contains('/')
                && s.to_ascii_lowercase().ends_with(".exe")),
            "名称规则程序名无效"
        );
        ensure!(
            !self.directories.is_empty()
                || !self.files.is_empty()
                || !self.products.is_empty()
                || !self.keys.is_empty(),
            "名称规则不能仅依据 EXE"
        );
        for list in [&self.directories, &self.files, &self.products, &self.keys] {
            ensure!(list.len() <= 16, "名称规则条件过多");
        }
        ensure!(
            self.files.iter().all(|s| relative(s)),
            "名称规则文件路径无效"
        );
        ensure!(
            self.directories
                .iter()
                .all(|s| relative(s) && !s.contains('/')),
            "名称规则目录无效"
        );
        ensure!(
            self.products.iter().all(|s| game_names::valid_title(s)),
            "名称规则产品无效"
        );
        ensure!(
            self.keys.iter().all(|s| game_names::valid_key(s)),
            "名称规则平台 ID 无效"
        );
        Ok(())
    }
    pub(crate) fn matches(
        &self,
        exe: &Path,
        root: &Path,
        products: &[String],
        keys: &[String],
    ) -> bool {
        if !core::within(exe, root) {
            return false;
        }
        let name = exe.file_name().unwrap_or_default().to_string_lossy();
        self.executables
            .iter()
            .any(|s| s.eq_ignore_ascii_case(&name))
            && (self.directories.is_empty()
                || self.directories.iter().any(|s| {
                    exe.parent()
                        .into_iter()
                        .flat_map(|p| p.ancestors())
                        .take(8)
                        .filter_map(Path::file_name)
                        .any(|p| p.to_string_lossy().eq_ignore_ascii_case(s))
                }))
            && self.files.iter().all(|s| {
                let p = root.join(s);
                core::no_links(&p).is_ok() && p.exists()
            })
            && (self.products.is_empty()
                || self
                    .products
                    .iter()
                    .any(|s| products.iter().any(|v| v.eq_ignore_ascii_case(s))))
            && (self.keys.is_empty() || self.keys.iter().any(|s| keys.contains(s)))
    }
}
