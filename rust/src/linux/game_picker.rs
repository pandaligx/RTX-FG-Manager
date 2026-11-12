//! Native portal selection does not execute the selected Windows program.
use anyhow::Result;
use std::path::PathBuf;
pub fn pick(title: &str, multiple: bool) -> Result<Option<Vec<PathBuf>>> {
    let dialog = rfd::FileDialog::new()
        .set_title(title)
        .add_filter("Windows game", &["exe", "EXE"]);
    Ok(if multiple {
        dialog.pick_files()
    } else {
        dialog.pick_file().map(|p| vec![p])
    })
}
