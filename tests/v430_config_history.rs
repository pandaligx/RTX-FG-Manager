//! End-to-end synthetic managed configurations; subprocess isolates LOCALAPPDATA.
use anyhow::Result;
use rtx_fg_manager::{
    config_store as store, core, encore, presets, rtxmfg, rtxmfg_upgrade, upstream040,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
};

#[test]
fn three_protocols_named_export_selected_import_and_conflict_aware_undo() -> Result<()> {
    if std::env::var_os("RTXFG_HISTORY_TEST_CHILD").is_none() {
        let temp = tempfile::tempdir()?;
        let result = std::process::Command::new(std::env::current_exe()?)
            .args([
                "--exact",
                "three_protocols_named_export_selected_import_and_conflict_aware_undo",
                "--nocapture",
            ])
            .env("LOCALAPPDATA", temp.path().join("local-data"))
            .env("RTXFG_HISTORY_TEST_CHILD", "1")
            .output()?;
        anyhow::ensure!(
            result.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        return Ok(());
    }
    let mut exe = vec![0u8; 512];
    exe[..2].copy_from_slice(b"MZ");
    exe[60..64].copy_from_slice(&128u32.to_le_bytes());
    exe[128..134].copy_from_slice(b"PE\0\0\x64\x86");
    exe[148..150].copy_from_slice(&240u16.to_le_bytes());
    exe[150..152].copy_from_slice(&2u16.to_le_bytes());
    exe[152..154].copy_from_slice(&0x20bu16.to_le_bytes());
    for (backend,scheme,protocol,raw,field,value) in [
        (upstream040::BACKEND,upstream040::SCHEME,upstream040::PROFILE,b"\xef\xbb\xbf; original\r\n[DLSSG]\r\nOptimized=1\r\n[Private]\r\nMine=42\r\n".as_slice(),"optimized","3"),
        (rtxmfg::BACKEND,rtxmfg_upgrade::SCHEME,rtxmfg::PROFILE,b"\xef\xbb\xbf{\r\n \"followGame\":true,\"mode\":\"follow\",\"multiplier\":2,\"menuHotkey\":121,\"custom\":42\r\n}\r\n".as_slice(),"rtx_mode","3"),
        (encore::BACKEND,encore::SCHEME,encore::PROFILE,b"\xef\xbb\xbf{\r\n // original\r\n \"frameGeneration\":{\"mode\":\"game\",\"multiplier\":4},\"unknown\":123\r\n}\r\n".as_slice(),"tf_mode","3"),
    ] {
        let temp=tempfile::tempdir()?;
        let game=temp.path().join("synthetic.exe");fs::write(&game,&exe)?;
        let target=core::DeploymentTarget::for_game(&game).validate(true)?;
        let operation=rtx_fg_manager::operation_history::Entry {
            created:chrono::Local::now().to_rfc3339(), action:"安装".into(),scheme:scheme.into(),
            version:"synthetic".into(),complete:true,detail:"Synthetic operation; no DLL loaded".into()};
        rtx_fg_manager::operation_history::append(&target,&operation)?;
        assert_eq!(rtx_fg_manager::operation_history::read(&target)?.len(),1);
        let other=core::DeploymentTarget::for_game(&temp.path().join("other.exe"));
        assert!(rtx_fg_manager::operation_history::read(&other)?.is_empty());
        fs::create_dir(target.directory.join(core::OWN))?;
        let names=core::deployment_names(backend,&["version.dll".into()])?;
        let mut hashes=BTreeMap::new();
        for name in names {
            let path=target.directory.join(&name);fs::create_dir_all(path.parent().unwrap())?;
            let data=if name==core::config_name(backend){raw}else{b"synthetic owned file; never loaded"};
            fs::write(path,data)?;hashes.insert(name,core::hash(data));
        }
        let record=core::Record {schema:3,backend:backend.into(),game_exe:target.record_exe(),payload_version:Some("1.0.0".into()),upstream_version:None,
            proxy:"version.dll".into(),proxies:vec!["version.dll".into()],hashes,cleanup_dirs:BTreeMap::new(),scheme_id:Some(scheme.into()),
            delta_cache_ids:vec![],delta_legacy_cache:false,cache_pending:false,cleanup_pending_reasons:vec![]};
        core::write_new(&target.directory.join(core::OWN).join(core::MARKER),&serde_json::to_vec(&record)?)?;
        let context=presets::Context {scheme:scheme.into(),profile:protocol.into(),delta:false,delta_capable:false};
        let snapshot=store::snapshot(&target,&context,"steam:1234","Quality: original")?;
        store::save_named(&target,&snapshot)?;
        assert_eq!(store::named(&target,"steam:1234",scheme,protocol)?.len(),1);
        let export=temp.path().join("export.json");store::save_envelope(&export,&snapshot)?;
        assert!(store::read_envelope(&export,"steam:5678",scheme,protocol).is_err());
        assert_eq!(store::read_envelope(&export,"steam:1234",scheme,protocol)?.settings,snapshot.settings);
        let values=presets::Values::from([(field.into(),value.into()),("logDirectory".into(),"C:\\private".into()),("menuHotkey".into(),"120".into())]);
        let preview=store::preview(&target,&context,&values)?;
        assert_eq!(preview.rejected.len(),2);
        let preview=store::select_preview(&preview,&BTreeSet::from([field.into()]))?;
        let id=store::apply(&target,&preview,"Imported: three frames")?;
        let path=target.directory.join(core::config_name(backend));
        let after=fs::read(&path)?;assert_ne!(after,raw);
        assert_eq!(store::history(&target)?.len(),1);
        let mut changed=after.clone();changed.extend(b" \r\n");fs::write(&path,&changed)?;
        assert!(store::undo(&target,&id).is_err());assert_eq!(fs::read(&path)?,changed);
        fs::write(&path,&after)?;store::undo(&target,&id)?;assert_eq!(fs::read(&path)?,raw);
        assert_eq!(store::history(&target)?[0].state,"undone");
    }
    Ok(())
}
