//! Where CutOut keeps its files. `CUTOUT_DATA_DIR` redirects everything (tests, portable use).

use std::path::PathBuf;

fn override_dir() -> Option<PathBuf> {
    std::env::var_os("CUTOUT_DATA_DIR").filter(|v| !v.is_empty()).map(PathBuf::from)
}

/// `%APPDATA%\CutOut` on Windows: small roaming settings.
pub fn config_dir() -> PathBuf {
    override_dir().map(|d| d.join("Config")).unwrap_or_else(|| dirs::config_dir().unwrap_or_else(std::env::temp_dir).join("CutOut"))
}

/// `%LOCALAPPDATA%\CutOut`: machine-local data (models, recovery).
pub fn data_dir() -> PathBuf {
    override_dir().map(|d| d.join("Data")).unwrap_or_else(|| dirs::data_local_dir().unwrap_or_else(std::env::temp_dir).join("CutOut"))
}

pub fn settings_file() -> PathBuf {
    config_dir().join("settings.json")
}

pub fn models_dir() -> PathBuf {
    data_dir().join("Models")
}

/// Models shipped by the installer, next to the executable (read-only).
pub fn bundled_models_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let d = exe.parent()?.join("Models");
    d.is_dir().then_some(d)
}

pub fn recovery_file() -> PathBuf {
    data_dir().join("Recovery").join("Last Session.cutout")
}
