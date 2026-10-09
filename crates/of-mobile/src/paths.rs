//! Replaces the PC version's `paths.rs` (same functions): where settings,
//! data and logs live. On Android the system hands the app its folders, so
//! [`init`] is called with them before anything reads a path; on a desktop
//! host (tests) the PC version's own folders are used.

use std::path::PathBuf;
use std::sync::OnceLock;

struct Dirs {
    config: PathBuf,
    local: PathBuf,
}

static DIRS: OnceLock<Dirs> = OnceLock::new();

/// Sets the folders once per process; later calls (a recreated activity) are ignored.
pub fn init(config: PathBuf, local: PathBuf) {
    let _ = DIRS.set(Dirs { config, local });
}

/// Settings, drawings and the demo account (`settings.json`, `drawings.json`, `demo.json`).
pub fn config_dir() -> PathBuf {
    DIRS.get().map_or_else(of_storage::app_config_dir, |d| d.config.clone())
}

/// Large local data: recorded ticks and logs.
pub fn local_dir() -> PathBuf {
    DIRS.get().map_or_else(of_storage::app_local_dir, |d| d.local.clone())
}

pub fn default_data_dir() -> PathBuf {
    local_dir().join("data")
}

pub fn log_file() -> PathBuf {
    local_dir().join("logs").join("ofterm.log")
}
