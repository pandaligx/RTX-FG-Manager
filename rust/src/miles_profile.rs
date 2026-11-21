//! Driver profile compatibility for older custom Miles Morales patches.
//! New patches read their own INI. Only a pre-existing owned driver setting
//! needs to be restored when moving to them.
use crate::{cloud, core, game_identity::SteamArtworkIndex, upstream040};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

mod driver;
type State = Option<u32>; // explicit per-title count; None inherits the driver's default
trait Settings {
    fn read(&mut self) -> Result<State>;
    fn replace(&mut self, expected: State, next: State) -> Result<()>;
}

fn has_title_name(path: &Path) -> bool {
    path.file_name()
        .is_some_and(|n| n.eq_ignore_ascii_case("MilesMorales.exe"))
}
fn is_reporter(path: &Path) -> bool {
    path.file_name()
        .is_some_and(|n| n.eq_ignore_ascii_case("crs-handler.exe"))
}
pub fn is_game(path: &Path) -> bool {
    (has_title_name(path)
        && SteamArtworkIndex::default()
            .lookup(path)
            .is_some_and(|(_, id)| id == "1817190"))
        || crate::scanner::miles_for_reporter(path).is_some()
}
fn needs_driver_profile(scheme: Option<&str>, version: Option<&str>) -> bool {
    scheme == Some(cloud::DELTA_041_SCHEME)
        && matches!(version, Some("0.4.2" | crate::delta::SIX_X_VERSION))
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    schema: u8,
    exe: String,
    directory: String,
    original: State,
    previous: State,
    applied: State,
}
fn validated(values: State) -> bool {
    values.is_none_or(|n| n <= 15)
}
fn receipt_path() -> Result<PathBuf> {
    let base = std::env::var_os("ProgramData").context("无法定位 NVIDIA 游戏配置的恢复记录")?;
    core::no_links(
        &PathBuf::from(base)
            .join("RTXFGManager")
            .join("driver-profiles")
            .join("miles-steam.json"),
    )
}
fn saved(path: &Path) -> Result<Option<Receipt>> {
    let path = core::no_links(path)?;
    let metadata = match fs::metadata(&path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    ensure!(
        metadata.is_file() && metadata.len() <= 32768,
        "游戏倍率恢复记录无效"
    );
    let r: Receipt = serde_json::from_slice(&fs::read(path)?)?;
    ensure!(
        r.schema == 1
            && has_title_name(Path::new(&r.exe))
            && Path::new(&r.exe).is_absolute()
            && Path::new(&r.directory).is_absolute()
            && [r.original, r.previous, r.applied]
                .into_iter()
                .all(validated),
        "游戏倍率恢复记录无效"
    );
    Ok(Some(r))
}

// DRS has a single profile even if two libraries contain the same title.
// Serialize separate manager processes, including different Windows sessions.
fn lock(path: &Path) -> Result<fs::File> {
    let lock = core::no_links(&path.with_extension("lock"))?;
    fs::create_dir_all(lock.parent().context("游戏倍率恢复目录无效")?)?;
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock)?;
    file.try_lock().context("游戏倍率正在由另一个管理器处理")?;
    Ok(file)
}
fn change(
    path: &Path,
    target: &core::DeploymentTarget,
    count: Option<u32>,
    driver: &mut impl Settings,
) -> Result<()> {
    // The driver always belongs to the renderer. Older installations may still
    // need their crash reporter identity for restoring pre-existing file backups.
    let owner = core::key(&target.game_exe.with_file_name("MilesMorales.exe"));
    let directory = core::key(&target.directory);
    let prior = saved(path)?;
    if let Some(prior) = &prior
        && (owner != prior.exe || directory != prior.directory)
    {
        ensure!(
            count.is_none(),
            "另一个《迈尔斯》安装目录正在管理 NVIDIA 倍率，请先在那里还原"
        );
        return Ok(());
    }
    if count.is_none() && prior.is_none() {
        return Ok(());
    }
    let now = driver.read()?;
    ensure!(
        validated(now),
        "当前 NVIDIA 游戏倍率超出支持范围，已保留原设置"
    );
    if let Some(count) = count {
        ensure!((1..=5).contains(&count), "无效的帧生成倍率");
        if let Some(prior) = &prior {
            ensure!(
                now == prior.applied || now == prior.previous,
                "NVIDIA 游戏设置已在其他程序中修改。请先选运行库默认，再设置倍率"
            );
        }
        // The multiplier is independent from the driver's FG enable switch.
        // Miles may update the latter on shutdown. Never take ownership of it.
        let next = Some(count);
        let original = prior.as_ref().map_or(now, |r| r.original);
        // No ownership to claim when the user's settings already implement this request.
        if now == next && prior.is_none() {
            return Ok(());
        }
        let mut receipt = Receipt {
            schema: 1,
            exe: owner,
            directory,
            original,
            previous: now,
            applied: next,
        };
        core::atomic_json(path, &receipt)?;
        if next != now {
            driver.replace(now, next)?;
        }
        // Once verified, an older value subsequently selected outside the manager
        // is a user edit, not an interrupted write.
        if receipt.previous != next {
            receipt.previous = next;
            core::atomic_json(path, &receipt)?;
        }
    } else if let Some(prior) = prior {
        if (now == prior.applied || now == prior.previous) && now != prior.original {
            driver.replace(now, prior.original)?;
        }
        fs::remove_file(path)?;
    }
    Ok(())
}
fn requested(ini: &[u8]) -> Result<Option<u32>> {
    let values = upstream040::read_041(ini)?;
    let n: u32 = values
        .get("max_generated_frames")
        .map_or("3", String::as_str)
        .parse()?;
    ensure!(
        n <= 5 && !(n > 3 && values.get("fg_engine").is_some_and(|v| v == "XeSS")),
        "此引擎不支持所选帧生成倍率"
    );
    Ok((n != 0).then_some(n))
}

pub(crate) fn restore_locked(target: &core::DeploymentTarget) -> Result<()> {
    if !has_title_name(&target.game_exe) && !is_reporter(&target.game_exe) {
        return Ok(());
    }
    let path = receipt_path()?;
    if !path.try_exists()? {
        return Ok(());
    }
    let _lock = lock(&path)?;
    change(&path, target, None, &mut driver::Driver)
}
/// Called with the game's deployment lock. Re-check current disk ownership
/// and use the committed INI, including on upgrade or community preset import.
fn sync_locked(target: &core::DeploymentTarget) -> Result<()> {
    let Some(record) = core::record(&target.directory)? else {
        return restore_locked(target);
    };
    target.validate_record(&record)?;
    if record.backend != upstream040::BACKEND
        || !needs_driver_profile(
            record.scheme_id.as_deref(),
            record
                .upstream_version
                .as_deref()
                .or(record.payload_version.as_deref()),
        )
        || !is_game(&target.game_exe)
    {
        return restore_locked(target);
    }
    let ini = core::no_links(&target.directory.join(core::INI))?;
    ensure!(ini.metadata()?.len() <= 1024 * 1024, "INI 文件过大");
    match requested(&fs::read(ini)?)? {
        None => restore_locked(target),
        count => {
            let path = receipt_path()?;
            let _lock = lock(&path)?;
            change(&path, target, count, &mut driver::Driver)
        }
    }
}
pub(crate) fn after_install(target: &core::DeploymentTarget) -> Result<()> {
    if !has_title_name(&target.game_exe) && !is_reporter(&target.game_exe) {
        return Ok(());
    }
    let target = target.validate(true)?;
    let _lock = crate::win::game_lock(&target.directory)?;
    core::assert_target_stopped(&target)?;
    sync_locked(&target).context("补丁已安装，NVIDIA 倍率未同步。请退出游戏后重新应用预设")
}
pub(crate) fn after_preset_locked(target: &core::DeploymentTarget) -> Result<()> {
    if !has_title_name(&target.game_exe) && !is_reporter(&target.game_exe) {
        return Ok(());
    }
    sync_locked(target).context("预设已写入，NVIDIA 倍率未同步。请退出游戏后重新应用方案")
}

#[cfg(test)]
mod tests;
