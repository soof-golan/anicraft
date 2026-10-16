//! What the application remembers between runs.
//!
//! Recent files and the autosave interval (`config.toml` in the
//! platform's config folder), and where templates and recovery files live (the platform's data folder).
//!
//! Both folders can be moved with `ANICRAFT_CONFIG_DIR` and `ANICRAFT_DATA_DIR`, and `--screenshot` runs keep them in
//! a temporary folder so a screenshot never reads or changes the person's own settings.

use std::path::{Path, PathBuf};
use std::time::Instant;

use anim_session::craft_settings::{self, Location, Settings, SettingsStore};
use anim_session::prefs::CommonSettings;
use serde::{Deserialize, Serialize};

/// The application id: the folder name under the config and data folders.
pub const APP_ID: &str = "anicraft";

/// Default autosave interval in minutes (Animate's default).
pub const DEFAULT_AUTOSAVE_MINUTES: u32 = 10;

/// The autosave intervals the File menu offers, in minutes (0 is off).
pub const AUTOSAVE_CHOICES: [u32; 6] = [0, 1, 5, 10, 15, 30];

/// The settings file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    pub common: CommonSettings,
}

impl Default for Prefs {
    fn default() -> Self {
        Self { common: CommonSettings { autosave_minutes: DEFAULT_AUTOSAVE_MINUTES, ..CommonSettings::default() } }
    }
}

impl Settings for Prefs {
    const APP_ID: &'static str = APP_ID;
    const SCHEMA_VERSION: u32 = 1;
}

/// The settings store and the folders next to it.
pub struct Store {
    settings: SettingsStore<Prefs>,
    data_dir: Option<PathBuf>,
}

impl Store {
    /// Opens the person's settings; `isolated` keeps everything in memory and under a temporary data folder.
    #[must_use]
    pub fn open(isolated: bool) -> Self {
        if isolated {
            return Self::in_memory();
        }
        let (settings, report) = SettingsStore::<Prefs>::open(Location::Auto);
        if report.has_problems() {
            eprintln!("anicraft: settings: {report:?}");
        }
        let data_dir = std::env::var_os("ANICRAFT_DATA_DIR").map(PathBuf::from).or_else(|| craft_settings::paths::data_dir(APP_ID));
        Self { settings, data_dir }
    }

    /// A store that reads and writes nothing; its data folder is a fresh temporary one.
    #[must_use]
    pub fn in_memory() -> Self {
        let (settings, _) = SettingsStore::<Prefs>::open(Location::Memory);
        let dir = std::env::temp_dir().join(format!("anicraft-isolated-{}", std::process::id()));
        Self { settings, data_dir: Some(dir) }
    }

    /// A store over a folder (tests).
    #[must_use]
    pub fn at(dir: &Path) -> Self {
        let (settings, _) = SettingsStore::<Prefs>::open_at(dir.join("config"));
        Self { settings, data_dir: Some(dir.join("data")) }
    }

    /// The current settings.
    #[must_use]
    pub fn get(&self) -> &Prefs {
        self.settings.get()
    }

    /// Changes the settings; they are written soon after.
    pub fn update(&mut self, f: impl FnOnce(&mut Prefs)) {
        self.settings.update(f);
    }

    /// Writes pending changes when their delay has passed; call every frame.
    pub fn tick(&mut self) {
        let _ = self.settings.tick(Instant::now());
    }

    /// Writes pending changes now (on exit).
    pub fn flush(&mut self) {
        let _ = self.settings.flush();
    }

    /// The folder for templates, recovery files and other state; `None` when the platform has none.
    #[must_use]
    pub fn data_dir(&self) -> Option<&Path> {
        self.data_dir.as_deref()
    }

    /// Where the person's own templates live.
    #[must_use]
    pub fn templates_dir(&self) -> Option<PathBuf> {
        self.data_dir.as_ref().map(|d| d.join("templates"))
    }

    /// Where saved commands live.
    #[must_use]
    pub fn commands_dir(&self) -> Option<PathBuf> {
        self.data_dir.as_ref().map(|d| d.join("commands"))
    }

    /// Where autosaved copies live.
    #[must_use]
    pub fn recovery_dir(&self) -> Option<PathBuf> {
        self.data_dir.as_ref().map(|d| d.join("recovery"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recent_files_and_autosave_persist() {
        let dir = tempfile::tempdir().unwrap();
        {
            let mut store = Store::at(dir.path());
            assert_eq!(store.get().common.autosave_minutes, DEFAULT_AUTOSAVE_MINUTES);
            store.update(|p| {
                p.common.recent.push(Path::new("/x/a.fla"));
                p.common.autosave_minutes = 5;
            });
            store.flush();
        }
        let again = Store::at(dir.path());
        assert_eq!(again.get().common.autosave_minutes, 5);
        assert_eq!(again.get().common.recent.to_vec(), [PathBuf::from("/x/a.fla")]);
        assert!(again.templates_dir().unwrap().ends_with("templates"));
    }
}
