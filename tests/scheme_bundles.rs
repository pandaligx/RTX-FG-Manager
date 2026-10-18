use rtx_fg_manager::{cloud, core};
use std::{
    collections::BTreeSet,
    io::{Cursor, Write},
};

#[test]
fn each_available_scheme_downloads_one_archive_with_gitcode_first() {
    let catalog = cloud::bundled();
    catalog.validate().unwrap();
    for scheme in catalog.schemes() {
        let files = catalog
            .packages
            .iter()
            .filter(|p| p.scheme_id == scheme.scheme_id)
            .map(|p| &p.archive)
            .collect::<BTreeSet<_>>();
        assert_eq!(files.len(), 1, "{}", scheme.scheme_id);
        for package in catalog
            .packages
            .iter()
            .filter(|p| p.scheme_id == scheme.scheme_id)
        {
            assert!(package.urls["domestic"].starts_with("https://gitcode.com/"));
            assert!(
                package
                    .urls
                    .values()
                    .all(|url| url.contains("/releases/download/payloads/"))
            );
        }
    }
}

fn archive(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, bytes) in entries {
        zip.start_file(*name, zip::write::FileOptions::default())
            .unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

#[test]
fn complete_archive_extracts_only_the_selected_logical_entry() {
    let mut package = cloud::bundled().packages[0].clone();
    package.archive_prefix = "chosen".into();
    package.files = vec![cloud::File {
        name: "settings.ini".into(),
        bytes: 3,
        sha256: core::hash(b"one"),
    }];
    let bytes = archive(&[
        ("chosen/settings.ini", b"one"),
        ("other/settings.ini", b"two"),
    ]);
    package.bytes = bytes.len() as u64;
    package.sha256 = core::hash(&bytes);
    let files = cloud::unpack(&package, &bytes).unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files["settings.ini"], b"one");
    for invalid in [
        "../settings.ini",
        "/settings.ini",
        "other/../settings.ini",
        "chosen/SETTINGS.ini",
    ] {
        let bytes = archive(&[("chosen/settings.ini", b"one"), (invalid, b"two")]);
        package.bytes = bytes.len() as u64;
        package.sha256 = core::hash(&bytes);
        assert!(cloud::unpack(&package, &bytes).is_err(), "{invalid}");
    }
}
