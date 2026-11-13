//! A small JSON file in the app's data folder (the scope, the app's settings), read leniently and written atomically.
//!
//! A file that is missing or unreadable reads as the default: a damaged settings file never stops the app from opening.
//! A write goes to a temporary file in the same folder, then a rename, so a crash mid-write never leaves half a file.

use serde::de::DeserializeOwned;
use serde::Serialize;
use std::fmt;
use std::fs;
use std::io::Write;
use std::path::Path;

/// Why a settings file could not be written.
#[derive(Debug)]
pub enum SettingsError {
    /// The path has no folder to write in.
    NoFolder,
    /// The path does not end in a file name.
    NotAFileName,
    /// Creating the folder, writing, syncing or renaming failed.
    Io(std::io::Error),
    /// The value cannot be written as JSON.
    Encode(serde_json::Error),
}

impl SettingsError {
    /// The stable code the page reads.
    pub fn code(&self) -> &'static str {
        match self {
            SettingsError::NoFolder => "settings_no_folder",
            SettingsError::NotAFileName => "settings_not_a_file_name",
            SettingsError::Io(_) => "settings_io",
            SettingsError::Encode(_) => "settings_encode",
        }
    }
}

impl fmt::Display for SettingsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            SettingsError::NoFolder => "the settings path has no folder",
            SettingsError::NotAFileName => "the settings path is not a file name",
            SettingsError::Io(_) => "the settings file could not be written",
            SettingsError::Encode(_) => "the settings could not be encoded",
        })
    }
}

impl std::error::Error for SettingsError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            SettingsError::Io(e) => Some(e),
            SettingsError::Encode(e) => Some(e),
            SettingsError::NoFolder | SettingsError::NotAFileName => None,
        }
    }
}

impl From<std::io::Error> for SettingsError {
    fn from(error: std::io::Error) -> Self {
        SettingsError::Io(error)
    }
}

/// The value stored in `file`, or the default when the file is missing or is not that value.
pub fn load<T: DeserializeOwned + Default>(file: &Path) -> T {
    fs::read_to_string(file)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

/// Writes `value` as pretty JSON, atomically (temporary file, `fsync`, rename), creating the folder if needed.
pub fn save<T: Serialize>(file: &Path, value: &T) -> Result<(), SettingsError> {
    let dir = file
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .ok_or(SettingsError::NoFolder)?;
    fs::create_dir_all(dir)?;
    let json = serde_json::to_string_pretty(value).map_err(SettingsError::Encode)?;
    let name = file.file_name().ok_or(SettingsError::NotAFileName)?;
    let temporary = dir.join(format!(".{}.tmp", name.to_string_lossy()));
    {
        let mut out = fs::File::create(&temporary)?;
        out.write_all(json.as_bytes())?;
        out.sync_all()?;
    }
    fs::rename(&temporary, file).map_err(|e| {
        let _ = fs::remove_file(&temporary);
        SettingsError::Io(e)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use tempfile::tempdir;

    #[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
    struct Prefs {
        channel: String,
        count: u32,
    }

    #[test]
    fn a_saved_value_reads_back_and_leaves_no_temporary_file() {
        let d = tempdir().unwrap();
        let file = d.path().join("nested").join("settings.json");
        let prefs = Prefs {
            channel: "beta".into(),
            count: 3,
        };
        save(&file, &prefs).unwrap();
        assert_eq!(load::<Prefs>(&file), prefs);
        let names: Vec<_> = fs::read_dir(file.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, vec![std::ffi::OsString::from("settings.json")]);
    }

    #[test]
    fn a_missing_or_damaged_file_reads_as_the_default() {
        let d = tempdir().unwrap();
        assert_eq!(
            load::<Prefs>(&d.path().join("missing.json")),
            Prefs::default()
        );
        let damaged = d.path().join("damaged.json");
        fs::write(&damaged, "{ not json").unwrap();
        assert_eq!(load::<Prefs>(&damaged), Prefs::default());
    }

    #[test]
    fn a_path_without_a_folder_is_refused() {
        assert!(save(Path::new("settings.json"), &Prefs::default()).is_err());
    }
}
