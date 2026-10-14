use crate::settings::{SettingsPaths, read_document, write_document};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct SessionStore {
    pub path: PathBuf,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct Session {
    last_volume: Option<PathBuf>,
}

impl SessionStore {
    pub fn new(paths: &SettingsPaths) -> Self {
        Self {
            path: paths.user.with_file_name("state.json"),
        }
    }

    pub fn last_volume(&self) -> Result<Option<PathBuf>, String> {
        let document = read_document(&self.path).map_err(|e| e.to_string())?;
        let session: Session = serde_json::from_value(document)
            .map_err(|e| format!("{}: {e}", self.path.display()))?;
        Ok(session.last_volume)
    }

    pub fn remember(&self, root: &Path) -> Result<(), String> {
        let root = root.canonicalize().map_err(|e| e.to_string())?;
        let mut document = read_document(&self.path).map_err(|e| e.to_string())?;
        document["last_volume"] = serde_json::to_value(root).map_err(|e| e.to_string())?;
        write_document(&self.path, &document, true).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persists_last_volume_separately_from_preferences() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("notes");
        std::fs::create_dir(&root).unwrap();
        let paths = SettingsPaths {
            global: dir.path().join("global.json"),
            user: dir.path().join("config/settings.json"),
        };
        let store = SessionStore::new(&paths);
        assert_eq!(store.last_volume().unwrap(), None);
        store.remember(&root).unwrap();
        assert_eq!(
            SessionStore::new(&paths).last_volume().unwrap(),
            Some(root.canonicalize().unwrap())
        );
        assert!(!paths.user.exists());
    }
}
