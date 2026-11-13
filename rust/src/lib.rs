pub mod assets;
pub mod cache;
pub mod cleanup;
#[cfg_attr(target_os = "linux", path = "linux/cleanup_pending.rs")]
pub mod cleanup_pending;
pub mod cloud;
pub mod config_store;
pub mod core;
pub mod delta;
pub mod deployment;
pub mod diagnostics;
#[cfg_attr(target_os = "linux", path = "elevation_linux.rs")]
pub mod elevation;
pub mod encore;
pub mod encore_components;
pub mod encore_install;
pub mod encore_upgrade;
#[cfg_attr(target_os = "linux", path = "linux/executable_name.rs")]
pub mod executable_name;
mod game_engines;
pub mod game_identity;
mod game_installations;
mod game_name_rules;
#[cfg_attr(target_os = "linux", path = "linux/game_names.rs")]
mod game_name_windows;
pub mod game_names;
#[cfg_attr(target_os = "linux", path = "linux/game_picker.rs")]
pub mod game_picker;
#[cfg_attr(target_os = "linux", path = "gpu_alias_linux.rs")]
pub mod gpu_alias;
#[cfg_attr(target_os = "linux", path = "hags_linux.rs")]
pub mod hags;
pub mod i18n;
pub mod jsonc;
#[cfg(windows)]
mod manager_shortcuts;
pub mod mfg_host;
pub mod mfg_plugin;
pub mod operation_history;
#[cfg(target_os = "linux")]
pub mod pe_version;
pub mod platform;
pub mod preferences;
pub mod presets;
pub mod recommendation_presets;
pub mod recommendation_runtimes;
pub mod recommendations;
pub mod release_notes;
pub mod reset;
pub mod rtxmfg;
pub mod rtxmfg_settings;
pub mod rtxmfg_upgrade;
pub mod scanner;
#[cfg_attr(target_os = "linux", path = "selftest_linux.rs")]
pub mod selftest;
pub mod task_progress;
pub mod transfer;
pub mod transfusion;
pub mod updater;
pub mod upstream040;
pub mod upstream040_upgrade;
#[cfg_attr(target_os = "linux", path = "win_linux.rs")]
pub mod win;
#[cfg(target_os = "linux")]
pub mod wine_reg;
pub mod xbox;
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub mod neural_runtime;
pub mod runtime_catalog;
pub mod runtime_files;
