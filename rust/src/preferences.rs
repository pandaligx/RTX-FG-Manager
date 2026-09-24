use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    thread,
};
const MAX_SETTINGS_BYTES: u64 = 4 * 1024 * 1024;
const BACKUP: &str = "games.last-good.json";
const CORRUPT: &str = "games.corrupt.json";
fn read_bounded(path: &Path) -> Result<Vec<u8>> {
    crate::core::no_links(path)?;
    let file = fs::File::open(path)?;
    ensure!(
        file.metadata()?.len() <= MAX_SETTINGS_BYTES,
        "设置文件超过大小限制"
    );
    let mut bytes = Vec::new();
    file.take(MAX_SETTINGS_BYTES + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= MAX_SETTINGS_BYTES,
        "设置文件超过大小限制"
    );
    Ok(bytes)
}
pub fn directory() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("RTXFGManager")
}
pub fn load(dir: &Path) -> Result<Value> {
    let path = dir.join("games.json");
    crate::core::no_links(&path)?;
    if !path.exists() {
        return Ok(
            json!({"schema":3,"games":[],"roots":[],"language":"system","theme":"跟随系统","backend":"native30","proxies":["version.dll"],"welcome_pending":true}),
        );
    }
    let v = crate::core::read_json(&path, MAX_SETTINGS_BYTES)?;
    validate(&v)?;
    Ok(v)
}
fn validate(v: &Value) -> Result<()> {
    ensure!(
        v.is_object() && v["schema"] == 3,
        "游戏列表版本不匹配，已保留原文件"
    );
    let games = v["games"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("游戏列表结构无效"))?;
    ensure!(
        games.len() <= 10000
            && games.iter().all(|g| g["exe"].is_string())
            && v.get("roots").is_none_or(Value::is_array),
        "游戏列表结构无效，已保留原文件"
    );
    Ok(())
}

#[derive(Debug)]
pub struct LoadReport {
    pub value: Value,
    pub recovered: bool,
    pub warning: Option<String>,
}

/// Recover only an absent or syntactically damaged file. A newer schema,
/// inaccessible file, reparse point, oversize file or structurally unknown JSON
/// is never replaced with older settings.
pub fn load_with_recovery(dir: &Path) -> Result<LoadReport> {
    let path = crate::core::no_links(&dir.join("games.json"))?;
    let backup = crate::core::no_links(&dir.join(BACKUP))?;
    let present = path.try_exists()?;
    let loaded = load(dir);
    let recovery_needed = !present && backup.try_exists()?;
    if !recovery_needed {
        match loaded {
            Ok(value) => {
                return Ok(LoadReport {
                    value,
                    recovered: false,
                    warning: None,
                });
            }
            Err(ref e)
                if e.downcast_ref::<serde_json::Error>()
                    .is_some_and(|e| e.is_syntax() || e.is_eof()) => {}
            Err(e) => return Err(e),
        }
    }
    let value = crate::core::read_json(&backup, MAX_SETTINGS_BYTES)
        .context("设置读取失败，未找到可用的最近成功备份；原文件已保留")?;
    validate(&value).context("设置备份版本或结构不受支持；原文件已保留")?;
    if present {
        // Preserve exactly one damaged snapshot. Do not overwrite a different
        // previous snapshot or turn recovery into an unbounded backup archive.
        let bytes = read_bounded(&path)?;
        let damaged = crate::core::no_links(&dir.join(CORRUPT))?;
        if damaged.try_exists()? {
            ensure!(
                damaged.is_file() && damaged.metadata()?.len() <= MAX_SETTINGS_BYTES,
                "已有待处理的损坏设置，未自动覆盖"
            );
            ensure!(
                read_bounded(&damaged)? == bytes,
                "已有另一份损坏设置，未自动覆盖"
            );
        } else {
            crate::core::write_new(&damaged, &bytes)?;
        }
        ensure!(
            read_bounded(&path)? == bytes,
            "恢复期间设置发生变化，未覆盖"
        );
    } else {
        ensure!(!path.try_exists()?, "恢复期间设置已被创建，未覆盖");
    }
    crate::core::atomic_json(&path, &value)?;
    Ok(LoadReport {
        value,
        recovered: true,
        warning: Some(
            if present {
                "设置文件损坏，已恢复最近成功保存的设置；损坏原件已保留。"
            } else {
                "设置文件缺失，已恢复最近成功保存的设置。"
            }
            .into(),
        ),
    })
}

#[derive(Debug)]
pub struct SaveResult {
    pub revision: u64,
    pub result: std::result::Result<(), String>,
    pub backup_warning: Option<String>,
}
struct SaveRequest {
    revision: u64,
    value: Value,
}
fn write_settings(dir: &Path, value: &Value) -> Result<Option<String>> {
    validate(value)?;
    ensure!(
        serde_json::to_vec_pretty(value)?.len() as u64 <= MAX_SETTINGS_BYTES,
        "设置文件超过大小限制"
    );
    let path = crate::core::no_links(&dir.join("games.json"))?;
    // A runtime external edit to a future/unknown schema must not be destroyed.
    if path.try_exists()? {
        let existing = crate::core::read_json(&path, MAX_SETTINGS_BYTES)?;
        validate(&existing)?;
    }
    crate::core::atomic_json(&path, value)?;
    let backup_result = (|| -> Result<()> {
        let backup = crate::core::no_links(&dir.join(BACKUP))?;
        if backup.try_exists()? {
            let existing = crate::core::read_json(&backup, MAX_SETTINGS_BYTES)?;
            validate(&existing)?;
        }
        crate::core::atomic_json(&backup, value)
    })();
    Ok(backup_result
        .err()
        .map(|e| format!("设置已保存，但最近成功备份未更新：{e:#}")))
}
pub struct Store {
    tx: mpsc::Sender<Option<SaveRequest>>,
    results: mpsc::Receiver<SaveResult>,
    revision: AtomicU64,
    worker: Option<thread::JoinHandle<Result<()>>>,
}
impl Store {
    pub fn new(dir: PathBuf) -> Self {
        let (tx, rx) = mpsc::channel::<Option<SaveRequest>>();
        let (result_tx, results) = mpsc::channel();
        let worker = thread::spawn(move || {
            let mut last_error = None;
            while let Ok(Some(mut request)) = rx.recv() {
                let mut finish = false;
                while let Ok(next) = rx.try_recv() {
                    match next {
                        Some(v) => request = v,
                        None => {
                            finish = true;
                            break;
                        }
                    }
                }
                let (result, backup_warning) = match write_settings(&dir, &request.value) {
                    Ok(warning) => {
                        last_error = None;
                        (Ok(()), warning)
                    }
                    Err(e) => {
                        let message = format!("{e:#}");
                        last_error = Some(e);
                        (Err(message), None)
                    }
                };
                let _ = result_tx.send(SaveResult {
                    revision: request.revision,
                    result,
                    backup_warning,
                });
                if finish {
                    break;
                }
            }
            if let Some(e) = last_error {
                Err(e)
            } else {
                Ok(())
            }
        });
        Self {
            tx,
            results,
            revision: AtomicU64::new(0),
            worker: Some(worker),
        }
    }
    pub fn save(&self, v: Value) -> Result<()> {
        self.save_tracked(v).map(|_| ())
    }
    pub fn save_tracked(&self, value: Value) -> Result<u64> {
        let revision = self.revision.fetch_add(1, Ordering::Relaxed) + 1;
        self.tx.send(Some(SaveRequest { revision, value }))?;
        Ok(revision)
    }
    pub fn try_result(&self) -> Option<SaveResult> {
        self.results.try_recv().ok()
    }
    pub fn finish(mut self) -> Result<()> {
        let _ = self.tx.send(None);
        self.worker
            .take()
            .unwrap()
            .join()
            .map_err(|_| anyhow::anyhow!("设置保存线程异常"))?
    }
}
