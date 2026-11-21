use super::*;

#[derive(Default)]
struct Fake {
    value: State,
    writes: usize,
    reads: usize,
    fail_before: bool,
    fail_after: bool,
}
impl Settings for Fake {
    fn read(&mut self) -> Result<State> {
        self.reads += 1;
        Ok(self.value)
    }
    fn replace(&mut self, expected: State, next: State) -> Result<()> {
        ensure!(expected == self.value && !self.fail_before, "write refused");
        self.value = next;
        self.writes += 1;
        ensure!(!self.fail_after, "readback failed");
        Ok(())
    }
}

#[test]
fn source_and_title_both_have_to_match() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let apps = temp.path().join("steamapps");
    let root = apps.join("common/Miles");
    fs::create_dir_all(&root)?;
    let exe = root.join("MilesMorales.exe");
    fs::write(&exe, [])?;
    fs::write(
        apps.join("appmanifest_1817190.acf"),
        "\"appid\" \"1817190\"\n\"installdir\" \"Miles\"",
    )?;
    assert!(is_game(&exe));
    assert!(!is_game(&root.join("DeltaForceClient-Win64-Shipping.exe")));
    assert!(!is_game(&temp.path().join("MilesMorales.exe")));
    fs::write(
        apps.join("appmanifest_42.acf"),
        "\"appid\" \"42\"\n\"installdir\" \"Miles\"",
    )?;
    assert!(!is_game(&exe));
    assert!(supported(Some(cloud::DELTA_041_SCHEME), Some("0.4.2")));
    for (scheme, version) in [
        (upstream040::SCHEME, "0.4.2"),
        ("pipo", "0.4.2"),
        (cloud::DELTA_041_SCHEME, "0.4.1"),
    ] {
        assert!(!supported(Some(scheme), Some(version)));
    }
    Ok(())
}

#[test]
fn counts_follow_the_actual_engine_contract() -> Result<()> {
    let ini = |engine: &str, count: u8| {
        format!("[FrameGeneration]\nEngine={engine}\nMaxGeneratedFrames={count}\n")
    };
    assert_eq!(requested(b"[FrameGeneration]\nEngine=DLSS\n")?, Some(3));
    for count in 0..=5 {
        assert_eq!(
            requested(ini("DLSS", count).as_bytes())?,
            (count != 0).then_some(count.into())
        );
        if count <= 3 {
            assert_eq!(
                requested(ini("XeSS", count).as_bytes())?,
                (count != 0).then_some(count.into())
            );
        } else {
            assert!(requested(ini("XeSS", count).as_bytes()).is_err());
        }
    }
    Ok(())
}

#[test]
fn install_reapply_update_and_restore_preserve_the_original() -> Result<()> {
    for original in [None, Some(0), Some(2), Some(12)] {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("miles.json");
        let target = core::DeploymentTarget::for_game(&temp.path().join("MilesMorales.exe"));
        let mut driver = Fake {
            value: original,
            ..Fake::default()
        };
        change(&path, &target, None, &mut driver)?;
        assert_eq!(driver.reads, 0);
        change(&path, &target, Some(3), &mut driver)?;
        change(&path, &target, Some(3), &mut driver)?;
        change(&path, &target, Some(5), &mut driver)?;
        assert_eq!(driver.writes, 2);
        change(&path, &target, None, &mut driver)?;
        assert_eq!(driver.value, original);
        assert!(!path.exists());
    }
    Ok(())
}

#[test]
fn another_copy_and_external_changes_are_preserved() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("miles.json");
    let target = core::DeploymentTarget::for_game(&temp.path().join("MilesMorales.exe"));
    let second = core::DeploymentTarget::for_game(&temp.path().join("other/MilesMorales.exe"));
    let mut driver = Fake::default();
    change(&path, &target, Some(3), &mut driver)?;
    assert!(change(&path, &second, Some(5), &mut driver).is_err());
    change(&path, &second, None, &mut driver)?;
    assert!(path.exists());
    driver.value = Some(2);
    assert!(change(&path, &target, Some(5), &mut driver).is_err());
    change(&path, &target, None, &mut driver)?;
    assert_eq!(driver.value, Some(2));
    assert!(!path.exists());
    change(&path, &target, Some(2), &mut driver)?;
    assert!(!path.exists());
    Ok(())
}

#[test]
fn interrupted_apply_and_restore_remain_recoverable() -> Result<()> {
    for committed in [false, true] {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("miles.json");
        let target = core::DeploymentTarget::for_game(&temp.path().join("MilesMorales.exe"));
        let mut driver = Fake {
            fail_before: !committed,
            fail_after: committed,
            ..Fake::default()
        };
        assert!(change(&path, &target, Some(3), &mut driver).is_err());
        assert!(path.exists());
        driver.fail_before = false;
        driver.fail_after = false;
        change(&path, &target, Some(3), &mut driver)?;
        driver.fail_after = true;
        assert!(change(&path, &target, None, &mut driver).is_err());
        assert!(path.exists());
        driver.fail_after = false;
        change(&path, &target, None, &mut driver)?;
        assert!(!path.exists());
        assert_eq!(driver.value, None);
    }
    Ok(())
}
