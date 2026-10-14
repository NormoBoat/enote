use crate::explorer::Directory;
use enote::{
    session::SessionStore,
    settings::{LoadedSettings, Settings, SettingsPaths, SettingsStore},
    volume::VolumeStorage,
};
use std::path::Path;

#[derive(Debug, Clone)]
pub struct OpenedVolume {
    pub storage: VolumeStorage,
    pub directory: Directory,
    pub settings: LoadedSettings,
    pub trash: Option<enote::file_ops::TrashRecord>,
}

impl OpenedVolume {
    pub fn open(root: &Path, base: &LoadedSettings) -> Result<Self, String> {
        let storage = VolumeStorage::open(root)?;
        Self::prepare(storage, base)
    }

    pub fn create(parent: &Path, name: &str, base: &LoadedSettings) -> Result<Self, String> {
        let storage = VolumeStorage::create(parent, name)?;
        Self::prepare(storage, base)
    }

    fn prepare(storage: VolumeStorage, base: &LoadedSettings) -> Result<Self, String> {
        let directory = Directory::read(&storage.root)?;
        let mut settings = base.clone();
        settings.apply_file(&storage.settings_path());
        let trash = match storage.latest_trash() {
            Ok(trash) => trash,
            Err(message) => {
                settings.warnings.push(enote::settings::SettingsError {
                    path: storage.root.join(".enote/trash"),
                    message,
                });
                None
            }
        };
        Ok(Self {
            storage,
            directory,
            settings,
            trash,
        })
    }
}

#[derive(Debug, Clone)]
pub struct Startup {
    pub base: LoadedSettings,
    pub session: Option<SessionStore>,
    pub volume: Option<OpenedVolume>,
    pub warnings: String,
}

impl Startup {
    pub fn discover() -> Self {
        match SettingsPaths::discover() {
            Ok(paths) => Self::load(paths),
            Err(error) => Self {
                base: LoadedSettings::from_settings(Settings::default()),
                session: None,
                volume: None,
                warnings: error.to_string(),
            },
        }
    }

    pub fn load(paths: SettingsPaths) -> Self {
        let store = SettingsStore::new(paths.clone());
        let initialization_error = store.ensure_user_file().err();
        let mut base = store.load();
        base.warnings.extend(initialization_error);
        let session = SessionStore::new(&paths);
        let restored = session.last_volume().and_then(|root| {
            root.map(|root| {
                OpenedVolume::open(&root, &base)
                    .map_err(|error| format!("Cannot restore {}: {error}", root.display()))
            })
            .transpose()
        });
        let (volume, restore_warning) = match restored {
            Ok(volume) => (volume, None),
            Err(error) => (None, Some(error)),
        };
        let loaded = volume
            .as_ref()
            .map(|volume| &volume.settings)
            .unwrap_or(&base);
        let mut warnings: Vec<_> = loaded.warnings.iter().map(ToString::to_string).collect();
        warnings.extend(restore_warning);
        Self {
            base,
            session: Some(session),
            volume,
            warnings: warnings.join("\n"),
        }
    }

    pub fn settings(&self) -> &Settings {
        &self
            .volume
            .as_ref()
            .map(|volume| &volume.settings)
            .unwrap_or(&self.base)
            .settings
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn paths(root: &Path) -> SettingsPaths {
        SettingsPaths {
            global: root.join("global/settings.json"),
            user: root.join("user/settings.json"),
        }
    }

    #[test]
    fn startup_restores_last_volume_and_applies_session_settings_only() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths(dir.path());
        let store = SettingsStore::new(paths.clone());
        store
            .update_user(serde_json::json!({"editor": {"font_size": 18}}))
            .unwrap();
        let user_before = fs::read(&paths.user).unwrap();
        let volume = VolumeStorage::create(dir.path(), "Notes").unwrap();
        fs::write(volume.root.join("note.md"), "note").unwrap();
        fs::write(
            volume.settings_path(),
            "{\"editor\":{\"font_size\":22},\"window\":{\"width\":1100}}",
        )
        .unwrap();
        SessionStore::new(&paths).remember(&volume.root).unwrap();
        let startup = Startup::load(paths.clone());
        assert_eq!(startup.settings().editor.font_size, 22);
        assert_eq!(startup.settings().window.width, 1100);
        assert_eq!(startup.base.settings.editor.font_size, 18);
        assert_eq!(startup.volume.as_ref().unwrap().directory.entries.len(), 1);
        assert_eq!(fs::read(&paths.user).unwrap(), user_before);
        let other = OpenedVolume::create(dir.path(), "Other", &startup.base).unwrap();
        assert_eq!(other.settings.settings.editor.font_size, 18);
        assert_eq!(other.settings.settings.window.width, 900);
    }

    #[test]
    fn first_start_or_unavailable_volume_returns_to_selector() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths(dir.path());
        assert!(Startup::load(paths.clone()).volume.is_none());
        let volume = VolumeStorage::create(dir.path(), "Notes").unwrap();
        SessionStore::new(&paths).remember(&volume.root).unwrap();
        fs::rename(&volume.root, dir.path().join("Moved")).unwrap();
        let startup = Startup::load(paths);
        assert!(startup.volume.is_none());
        assert!(startup.warnings.contains("Cannot restore"));
    }

    #[test]
    fn invalid_volume_settings_fall_back_without_corrupting_volume() {
        let dir = tempfile::tempdir().unwrap();
        let volume = VolumeStorage::create(dir.path(), "Notes").unwrap();
        fs::write(volume.settings_path(), "broken").unwrap();
        let base = LoadedSettings::from_settings(Settings::default());
        let opened = OpenedVolume::open(&volume.root, &base).unwrap();
        assert_eq!(opened.settings.settings, base.settings);
        assert_eq!(opened.settings.warnings.len(), 1);
        assert_eq!(
            fs::read_to_string(volume.settings_path()).unwrap(),
            "broken"
        );
    }
}
