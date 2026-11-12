//! Windows installation evidence is not inferred from unrelated host files.
use crate::{game_installations::Install, scanner::Game};
use std::{collections::BTreeMap, path::Path, sync::atomic::AtomicBool};
pub(crate) fn registry(_cancel: &AtomicBool) -> Vec<Install> {
    Vec::new()
}
pub(crate) fn products(_exe: &Path) -> Vec<String> {
    Vec::new()
}
pub(crate) fn shortcuts(_games: &[Game], _cancel: &AtomicBool) -> BTreeMap<String, Vec<String>> {
    BTreeMap::new()
}
