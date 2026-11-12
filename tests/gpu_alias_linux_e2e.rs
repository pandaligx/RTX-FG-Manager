//! End-to-end GPU display-name aliasing against a synthetic Wine prefix.
#![cfg(target_os = "linux")]
use rtx_fg_manager::{gpu_alias, wine_reg};

fn copy_prefix(source: &std::path::Path, dest: &std::path::Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(dest)?;
    std::fs::copy(source.join("system.reg"), dest.join("system.reg"))?;
    if source.join("user.reg").is_file() {
        std::fs::copy(source.join("user.reg"), dest.join("user.reg"))?;
    }
    std::fs::create_dir_all(dest.join("drive_c"))?;
    Ok(())
}

#[test]
fn rename_and_restore_cycle_updates_a_copied_prefix_registry() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let live = temp.path().join("original");
    std::fs::create_dir(&live)?;
    std::fs::write(
        live.join("system.reg"),
        r#"WINE REGISTRY Version 2

[System\\Select]
"Current"=dword:00000001

[System\\ControlSet001\\Enum\\PCI\\VEN_10DE&DEV_25A2\\00000000]
"ClassGUID"="{4D36E968-E325-11CE-BFC1-08002BE10318}"
"DeviceDesc"="NVIDIA GeForce RTX 3050 Laptop GPU"
"Driver"="{4d36e968-e325-11ce-bfc1-08002be10318}\\0000"

[System\\ControlSet001\\Control\\Class\\{4d36e968-e325-11ce-bfc1-08002be10318}\\0000]
"DriverDesc"="NVIDIA GeForce RTX 3050 Laptop GPU"
"HardwareInformation.AdapterString"="NVIDIA GeForce RTX 3050 Laptop GPU"
"DriverVersion"="test"
"InfPath"="test.inf"

[Software\\Unrelated]
"keep"="untouched"
"#,
    )?;
    let work = temp.path().join("working");
    copy_prefix(&live, &work)?;
    rtx_fg_manager::win::set_prefix(&work)?;

    let devices = gpu_alias::enumerate()?;
    assert_eq!(devices.len(), 1);
    let target = devices[0].clone();
    let original = target.name.clone();

    // Record the exact bytes so a failed cycle cannot silently corrupt the hive.
    let before = std::fs::read(wine_reg::hive_path(&work, wine_reg::Hive::Machine))?;

    let message = gpu_alias::apply(target.clone(), Some(gpu_alias::NAMES[0]))?;
    assert!(message.contains(gpu_alias::NAMES[0]));

    let renamed = gpu_alias::enumerate()?
        .into_iter()
        .find(|d| d.instance.eq_ignore_ascii_case(&target.instance))
        .expect("adapter still present");
    assert_eq!(
        renamed.name,
        gpu_alias::NAMES[0],
        "DeviceDesc must be renamed"
    );

    // A backup record must exist so restore is possible.
    gpu_alias::restore_all()?;
    let after = gpu_alias::enumerate()?
        .into_iter()
        .find(|d| d.instance.eq_ignore_ascii_case(&target.instance))
        .expect("adapter still present");
    assert_eq!(
        after.name, original,
        "restore must return the original name"
    );

    // The registry must still parse and differ only where intended.
    let final_bytes = std::fs::read(wine_reg::hive_path(&work, wine_reg::Hive::Machine))?;
    assert!(!final_bytes.is_empty());
    assert_ne!(before, final_bytes, "an edit must have occurred");
    wine_reg::RegFile::open(&wine_reg::hive_path(&work, wine_reg::Hive::Machine))
        .expect("registry remains parseable after the cycle");

    assert_eq!(std::fs::read(live.join("system.reg"))?, before);
    Ok(())
}
