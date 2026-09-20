use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::error::AppResult;
use crate::platform::platform;
use crate::settings::AppSettings;

use super::model::{AppData, LegacyProfilesOnlyFile};

const LEGACY_PROFILES_FILE: &str = "profiles.json";
const LEGACY_SETTINGS_FILE: &str = "app_settings.json";

pub fn data_file_path() -> AppResult<PathBuf> {
    Ok(platform()
        .app_config_dir()?
        .join("data")
        .join("profiles.json"))
}

pub fn load_or_migrate() -> AppResult<AppData> {
    let path = data_file_path()?;
    if path.exists() {
        let raw = fs::read_to_string(&path)?;
        return parse_json_file(&path, &raw);
    }

    let app_root = platform().app_config_dir()?;
    let mut data = AppData::default();

    let legacy_profiles = app_root.join(LEGACY_PROFILES_FILE);
    if legacy_profiles.exists() {
        let raw = fs::read_to_string(&legacy_profiles)?;
        let file: LegacyProfilesOnlyFile = parse_json_file(&legacy_profiles, &raw)?;
        data.profiles = file.profiles;
    }

    let legacy_settings = app_root.join(LEGACY_SETTINGS_FILE);
    if legacy_settings.exists() {
        let raw = fs::read_to_string(&legacy_settings)?;
        let settings: AppSettings = parse_json_file(&legacy_settings, &raw)?;
        merge_settings(&mut data, settings);
    }

    Ok(data)
}

pub fn save(data: &AppData) -> AppResult<()> {
    let path = data_file_path()?;
    write_data(&path, data)
}

fn write_data(path: &Path, data: &AppData) -> AppResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(data)?;
    let parent = path
        .parent()
        .ok_or_else(|| crate::error::AppError::Message("configuration path has no parent".into()))?;
    let temp = parent.join(format!(".profiles-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| -> std::io::Result<()> {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp)?;
        file.write_all(format!("{text}\n").as_bytes())?;
        file.flush()?;
        file.sync_all()?;
        atomic_replace(&temp, path)?;
        if let Ok(directory) = File::open(parent) {
            let _ = directory.sync_all();
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result.map_err(Into::into)
}

fn parse_json_file<T: serde::de::DeserializeOwned>(path: &Path, raw: &str) -> AppResult<T> {
    serde_json::from_str(raw).map_err(|error| {
        crate::error::AppError::Message(format!(
            "configuration parse failed for {}: {error}",
            path.display()
        ))
    })
}

#[cfg(not(windows))]
fn atomic_replace(source: &Path, target: &Path) -> std::io::Result<()> {
    fs::rename(source, target)
}

#[cfg(windows)]
fn atomic_replace(source: &Path, target: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    let source = source
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let target = target
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    unsafe {
        MoveFileExW(
            PCWSTR(source.as_ptr()),
            PCWSTR(target.as_ptr()),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
        .map_err(|error| std::io::Error::other(error.to_string()))
    }
}

pub fn maybe_backup_legacy_files(path: &Path) -> AppResult<()> {
    if !path.exists() {
        return Ok(());
    }
    let app_root = platform().app_config_dir()?;
    for name in [LEGACY_PROFILES_FILE, LEGACY_SETTINGS_FILE] {
        let legacy = app_root.join(name);
        if legacy.exists() {
            let backup = app_root.join(format!("{name}.bak"));
            if !backup.exists() {
                let _ = fs::rename(&legacy, &backup);
            }
        }
    }
    Ok(())
}

fn merge_settings(data: &mut AppData, settings: AppSettings) {
    data.frp_profiles = settings.frp_profiles;
    data.last_workspace_id = settings.last_workspace_id;
    data.download = settings.download;
    data.proxy = settings.proxy;
    data.shared_secrets = settings.shared_secrets;
    data.workspace_secrets = settings.workspace_secrets;
    data.app_secrets = settings.app_secrets;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_json_is_reported_without_defaulting() {
        let path = Path::new("D:/isolated/profiles.json");
        let error = parse_json_file::<AppData>(path, "{\"download\":null")
            .expect_err("invalid JSON must fail");
        let message = error.to_string();
        assert!(message.contains("configuration parse failed"));
        assert!(message.contains("D:/isolated/profiles.json"));
    }

    #[test]
    fn write_data_replaces_existing_file_atomically() {
        let directory = tempfile::tempdir().expect("temporary config directory");
        let path = directory.path().join("profiles.json");
        fs::write(&path, b"{\"profiles\":[]}").expect("seed config");

        let data = AppData::default();
        write_data(&path, &data).expect("atomic save");

        let raw = fs::read_to_string(&path).expect("saved config");
        let parsed: AppData = serde_json::from_str(&raw).expect("saved JSON");
        assert!(parsed.profiles.is_empty());
        assert!(!directory
            .path()
            .read_dir()
            .expect("directory")
            .any(|entry| entry
                .ok()
                .map(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
                .unwrap_or(false)));
    }
}
