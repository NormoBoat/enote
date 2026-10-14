use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::{
    env, fmt, fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

const MAX_FILE_SIZE: u64 = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    pub window: WindowSettings,
    pub editor: EditorSettings,
    pub workspace: WorkspaceSettings,
    pub viewer: ViewerSettings,
    pub shortcuts: crate::shortcuts::Shortcuts,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WindowSettings {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EditorSettings {
    pub font_size: u16,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceSettings {
    pub sidebar_visible: bool,
    pub sidebar_width: u16,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ViewerSettings {
    pub remember_position: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            window: WindowSettings {
                width: 900,
                height: 600,
            },
            editor: EditorSettings { font_size: 16 },
            workspace: WorkspaceSettings {
                sidebar_visible: true,
                sidebar_width: 250,
            },
            shortcuts: crate::shortcuts::Shortcuts::default(),
            viewer: ViewerSettings {
                remember_position: true,
            },
        }
    }
}

impl Settings {
    fn validate(&self) -> Result<(), String> {
        self.shortcuts.validate()?;
        if !(320..=7680).contains(&self.window.width) || !(240..=4320).contains(&self.window.height)
        {
            return Err("window dimensions must be within 320..7680 x 240..4320".into());
        }
        if !(8..=48).contains(&self.editor.font_size) {
            return Err("editor.font_size must be within 8..48".into());
        }
        if !(160..=480).contains(&self.workspace.sidebar_width) {
            return Err("workspace.sidebar_width must be within 160..480".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsPaths {
    pub global: PathBuf,
    pub user: PathBuf,
}

impl SettingsPaths {
    pub fn discover() -> Result<Self, SettingsError> {
        Self::from_environment(|name| env::var_os(name))
    }

    fn from_environment(
        mut get: impl FnMut(&str) -> Option<std::ffi::OsString>,
    ) -> Result<Self, SettingsError> {
        let absolute = |value: Option<std::ffi::OsString>| {
            value.map(PathBuf::from).filter(|path| path.is_absolute())
        };
        #[cfg(not(target_os = "windows"))]
        let (global, user) = {
            let base = absolute(get("XDG_CONFIG_HOME"))
                .or_else(|| absolute(get("HOME")).map(|home| home.join(".config")))
                .ok_or_else(|| {
                    SettingsError::new(
                        Path::new("settings.json"),
                        "HOME or an absolute XDG_CONFIG_HOME is required",
                    )
                })?;
            (
                PathBuf::from("/etc/enote/settings.json"),
                base.join("enote/settings.json"),
            )
        };
        #[cfg(target_os = "windows")]
        let (global, user) = {
            let global = absolute(get("PROGRAMDATA")).ok_or_else(|| {
                SettingsError::new(Path::new("settings.json"), "PROGRAMDATA is required")
            })?;
            let user = absolute(get("APPDATA"))
                .or_else(|| absolute(get("USERPROFILE")).map(|home| home.join(".config")))
                .ok_or_else(|| {
                    SettingsError::new(
                        Path::new("settings.json"),
                        "APPDATA or USERPROFILE is required",
                    )
                })?;
            (
                global.join("enote/settings.json"),
                user.join("enote/settings.json"),
            )
        };
        Ok(Self { global, user })
    }
}

#[derive(Debug, Clone)]
pub struct SettingsError {
    pub path: PathBuf,
    pub message: String,
}

impl SettingsError {
    fn new(path: &Path, message: impl ToString) -> Self {
        Self {
            path: path.to_owned(),
            message: message.to_string(),
        }
    }
}

impl fmt::Display for SettingsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.path.display(), self.message)
    }
}
impl std::error::Error for SettingsError {}

#[derive(Debug, Clone)]
pub struct LoadedSettings {
    pub settings: Settings,
    pub document: Value,
    pub warnings: Vec<SettingsError>,
}

impl LoadedSettings {
    pub fn from_settings(settings: Settings) -> Self {
        Self {
            document: serde_json::to_value(&settings).expect("settings serialize"),
            settings,
            warnings: Vec::new(),
        }
    }

    pub fn apply_file(&mut self, path: &Path) {
        let result = read_document(path).and_then(|overlay| {
            let mut candidate = self.document.clone();
            merge(&mut candidate, overlay);
            let settings = decode(&candidate, path)?;
            Ok((candidate, settings))
        });
        match result {
            Ok((document, settings)) => {
                self.document = document;
                self.settings = settings;
            }
            Err(error) => self.warnings.push(error),
        }
    }
}

#[derive(Debug)]
pub struct SettingsStore {
    pub paths: SettingsPaths,
}

impl SettingsStore {
    pub fn new(paths: SettingsPaths) -> Self {
        Self { paths }
    }

    pub fn load(&self) -> LoadedSettings {
        let mut loaded = LoadedSettings {
            settings: Settings::default(),
            document: serde_json::to_value(Settings::default())
                .expect("built-in settings serialize"),
            warnings: Vec::new(),
        };
        for path in [&self.paths.global, &self.paths.user] {
            loaded.apply_file(path);
        }
        loaded
    }

    /// Creates only the user layer. An empty object inherits all global defaults.
    pub fn ensure_user_file(&self) -> Result<(), SettingsError> {
        if self
            .paths
            .user
            .try_exists()
            .map_err(|e| SettingsError::new(&self.paths.user, e))?
        {
            return Ok(());
        }
        write_document(&self.paths.user, &Value::Object(Map::new()), false)
    }

    /// Merges a patch into the user layer while retaining unrelated and unknown keys.
    pub fn update_user(&self, patch: Value) -> Result<(), SettingsError> {
        if !patch.is_object() {
            return Err(SettingsError::new(
                &self.paths.user,
                "settings must be a JSON object",
            ));
        }
        let mut user = read_document(&self.paths.user)?;
        merge(&mut user, patch.clone());
        let loaded = self.load();
        // A malformed user file must never be replaced with defaults during a save.
        if let Some(error) = loaded
            .warnings
            .iter()
            .find(|error| error.path == self.paths.user)
        {
            return Err(error.clone());
        }
        let mut effective = loaded.document;
        merge(&mut effective, patch);
        decode(&effective, &self.paths.user)?;
        write_document(&self.paths.user, &user, true)
    }
}

fn decode(document: &Value, path: &Path) -> Result<Settings, SettingsError> {
    let settings: Settings =
        serde_json::from_value(document.clone()).map_err(|e| SettingsError::new(path, e))?;
    settings
        .validate()
        .map_err(|e| SettingsError::new(path, e))?;
    Ok(settings)
}

pub(crate) fn read_document(path: &Path) -> Result<Value, SettingsError> {
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Value::Object(Map::new()));
        }
        Err(error) => return Err(SettingsError::new(path, error)),
    };
    let mut bytes = Vec::new();
    file.take(MAX_FILE_SIZE + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| SettingsError::new(path, e))?;
    if bytes.len() as u64 > MAX_FILE_SIZE {
        return Err(SettingsError::new(path, "settings file exceeds 1 MiB"));
    }
    let document: Value =
        serde_json::from_slice(&bytes).map_err(|e| SettingsError::new(path, e))?;
    if !document.is_object() {
        return Err(SettingsError::new(path, "settings must be a JSON object"));
    }
    Ok(document)
}

fn merge(base: &mut Value, overlay: Value) {
    match (base, overlay) {
        (Value::Object(base), Value::Object(overlay)) => {
            for (key, value) in overlay {
                merge(base.entry(key).or_insert(Value::Null), value);
            }
        }
        (base, overlay) => *base = overlay,
    }
}

pub(crate) fn write_document(
    path: &Path,
    document: &Value,
    replace: bool,
) -> Result<(), SettingsError> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or_else(|| SettingsError::new(path, "settings path requires a parent directory"))?;
    fs::create_dir_all(parent).map_err(|e| SettingsError::new(path, e))?;
    let mut temp =
        tempfile::NamedTempFile::new_in(parent).map_err(|e| SettingsError::new(path, e))?;
    if replace && let Ok(metadata) = fs::metadata(path) {
        temp.as_file()
            .set_permissions(metadata.permissions())
            .map_err(|e| SettingsError::new(path, e))?;
    }
    let mut bytes = serde_json::to_vec_pretty(document).map_err(|e| SettingsError::new(path, e))?;
    bytes.push(b'\n');
    if bytes.len() as u64 > MAX_FILE_SIZE {
        return Err(SettingsError::new(path, "settings file exceeds 1 MiB"));
    }
    temp.write_all(&bytes)
        .map_err(|e| SettingsError::new(path, e))?;
    temp.as_file()
        .sync_all()
        .map_err(|e| SettingsError::new(path, e))?;
    if replace {
        temp.persist(path)
            .map_err(|e| SettingsError::new(path, e))?;
    } else if let Err(error) = temp.persist_noclobber(path)
        && error.error.kind() != std::io::ErrorKind::AlreadyExists
    {
        return Err(SettingsError::new(path, error));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn shortcut_overrides_merge_in_all_layers_and_disabled_bindings_stay_disabled() {
        use crate::shortcuts::ShortcutAction;
        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path());
        store
            .update_user(json!({"shortcuts": {"save": ["Alt+S"], "redo": []}}))
            .unwrap();
        let mut loaded = store.load();
        assert_eq!(
            loaded.settings.shortcuts.0[&ShortcutAction::Save],
            vec!["Alt+S"]
        );
        assert!(loaded.settings.shortcuts.0[&ShortcutAction::Redo].is_empty());
        assert!(!loaded.settings.shortcuts.0[&ShortcutAction::Open].is_empty());
        let override_path = dir.path().join("volume-settings.json");
        write_document(
            &override_path,
            &json!({"shortcuts": {"save": ["Alt+P"]}}),
            false,
        )
        .unwrap();
        loaded.apply_file(&override_path);
        assert_eq!(
            loaded.settings.shortcuts.0[&ShortcutAction::Save],
            vec!["Alt+P"]
        );
        assert!(
            store
                .update_user(json!({"shortcuts": {"save": ["Command+N"]}}))
                .is_err()
        );
    }

    #[test]
    fn example_matches_builtin_defaults() {
        let example: Settings =
            serde_json::from_str(include_str!("../settings.example.json")).unwrap();
        assert_eq!(example, Settings::default());
    }

    fn store(dir: &Path) -> SettingsStore {
        SettingsStore::new(SettingsPaths {
            global: dir.join("global/settings.json"),
            user: dir.join("user/settings.json"),
        })
    }

    #[test]
    fn missing_files_use_defaults_and_initialization_creates_only_user_layer() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path());
        assert_eq!(store.load().settings, Settings::default());
        store.ensure_user_file().unwrap();
        assert_eq!(read_document(&store.paths.user).unwrap(), json!({}));
        assert!(!store.paths.global.exists());
    }

    #[test]
    fn layers_merge_recursively_and_user_values_win() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path());
        write_document(
            &store.paths.global,
            &json!({"window": {"width": 1000}, "editor": {"font_size": 18}, "plugin": {"a": true}}),
            false,
        )
        .unwrap();
        write_document(
            &store.paths.user,
            &json!({"window": {"height": 700}, "editor": {"font_size": 20}, "plugin": {"b": true}}),
            false,
        )
        .unwrap();
        let loaded = store.load();
        assert!(loaded.warnings.is_empty());
        assert_eq!(
            loaded.settings.window,
            WindowSettings {
                width: 1000,
                height: 700
            }
        );
        assert_eq!(loaded.settings.editor.font_size, 20);
        assert_eq!(loaded.document["plugin"], json!({"a": true, "b": true}));
    }

    #[test]
    fn saving_patch_preserves_unknown_keys_and_does_not_copy_global_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path());
        write_document(
            &store.paths.global,
            &json!({"editor": {"font_size": 18}}),
            false,
        )
        .unwrap();
        store
            .update_user(json!({"plugin": {"enabled": true}}))
            .unwrap();
        store
            .update_user(json!({"workspace": {"sidebar_visible": false}}))
            .unwrap();
        let user = read_document(&store.paths.user).unwrap();
        assert_eq!(user["plugin"]["enabled"], true);
        assert!(user.get("editor").is_none());
        assert!(!store.load().settings.workspace.sidebar_visible);
        assert_eq!(store.load().settings.editor.font_size, 18);
    }

    #[test]
    fn invalid_global_layer_does_not_prevent_valid_user_settings() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path());
        fs::create_dir_all(store.paths.global.parent().unwrap()).unwrap();
        fs::write(&store.paths.global, "{broken").unwrap();
        store
            .update_user(json!({"editor": {"font_size": 22}}))
            .unwrap();
        let loaded = store.load();
        assert_eq!(loaded.warnings.len(), 1);
        assert_eq!(loaded.settings.editor.font_size, 22);
    }

    #[test]
    fn initialization_and_saving_never_overwrite_corrupt_user_file() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path());
        fs::create_dir_all(store.paths.user.parent().unwrap()).unwrap();
        fs::write(&store.paths.user, "broken").unwrap();
        store.ensure_user_file().unwrap();
        assert!(
            store
                .update_user(json!({"editor": {"font_size": 20}}))
                .is_err()
        );
        assert_eq!(fs::read_to_string(&store.paths.user).unwrap(), "broken");
        assert_eq!(store.load().settings, Settings::default());
    }

    #[test]
    fn invalid_values_are_rejected_without_modifying_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path());
        store.ensure_user_file().unwrap();
        for patch in [
            json!([]),
            json!({"editor": {"font_size": 0}}),
            json!({"window": {"width": "wide"}}),
            json!({"workspace": null}),
        ] {
            assert!(store.update_user(patch).is_err());
            assert_eq!(read_document(&store.paths.user).unwrap(), json!({}));
        }
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn discovers_home_and_xdg_paths_without_changing_process_environment() {
        for (xdg, expected) in [
            (None, "/home/test/.config/enote/settings.json"),
            (Some("/custom/config"), "/custom/config/enote/settings.json"),
            (Some("relative"), "/home/test/.config/enote/settings.json"),
        ] {
            let paths = SettingsPaths::from_environment(|key| match key {
                "HOME" => Some("/home/test".into()),
                "XDG_CONFIG_HOME" => xdg.map(Into::into),
                _ => None,
            })
            .unwrap();
            assert_eq!(paths.global, PathBuf::from("/etc/enote/settings.json"));
            assert_eq!(paths.user, PathBuf::from(expected));
        }
        assert!(SettingsPaths::from_environment(|_| None).is_err());
    }
}
