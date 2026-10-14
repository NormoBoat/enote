use crate::settings::{SettingsPaths, read_document, write_document};
use serde::{Deserialize, Serialize};
use std::{
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex},
};

#[derive(Debug, Clone)]
pub struct SessionStore {
    pub path: PathBuf,
    io: Arc<Mutex<()>>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct Session {
    last_volume: Option<PathBuf>,
    window: Option<WindowPlacement>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WindowPlacement {
    pub width: f32,
    pub height: f32,
    pub position: Option<[f32; 2]>,
}

impl WindowPlacement {
    pub fn valid(&self) -> bool {
        (100.0..=16384.0).contains(&self.width)
            && (100.0..=16384.0).contains(&self.height)
            && self
                .position
                .is_none_or(|p| p.into_iter().all(f32::is_finite))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PaneLayout {
    Pane {
        tab: Option<usize>,
    },
    Split {
        vertical: bool,
        ratio: f32,
        a: Box<PaneLayout>,
        b: Box<PaneLayout>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WorkspaceSession {
    pub version: u32,
    pub tabs: Vec<PathBuf>,
    pub active: Option<usize>,
    pub layout: PaneLayout,
}

impl Default for WorkspaceSession {
    fn default() -> Self {
        Self {
            version: 1,
            tabs: Vec::new(),
            active: None,
            layout: PaneLayout::Pane { tab: None },
        }
    }
}

impl WorkspaceSession {
    fn valid(&self) -> bool {
        fn layout_valid(
            layout: &PaneLayout,
            tabs: usize,
            depth: usize,
            leaves: &mut usize,
        ) -> bool {
            if depth > 16 {
                return false;
            }
            match layout {
                PaneLayout::Pane { tab } => {
                    *leaves += 1;
                    tab.is_none_or(|n| n < tabs) && *leaves <= 64
                }
                PaneLayout::Split { ratio, a, b, .. } => {
                    (0.1..=0.9).contains(ratio)
                        && layout_valid(a, tabs, depth + 1, leaves)
                        && layout_valid(b, tabs, depth + 1, leaves)
                }
            }
        }
        self.version == 1
            && self.tabs.len() <= 256
            && self.tabs.iter().all(|p| {
                !p.as_os_str().is_empty()
                    && p.components()
                        .all(|c| matches!(c, Component::Normal(n) if n != ".enote"))
            })
            && self.active.is_none_or(|n| n < self.tabs.len())
            && layout_valid(&self.layout, self.tabs.len(), 0, &mut 0)
    }

    fn path(root: &Path) -> Result<PathBuf, String> {
        let technical = root.join(".enote");
        let metadata = std::fs::symlink_metadata(&technical).map_err(|e| e.to_string())?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err("Invalid volume technical directory".into());
        }
        let path = technical.join("workspace-state.json");
        if let Ok(metadata) = std::fs::symlink_metadata(&path)
            && (metadata.file_type().is_symlink() || !metadata.is_file())
        {
            return Err("Invalid workspace state file".into());
        }
        Ok(path)
    }

    pub fn load(root: &Path) -> Result<Self, String> {
        let document = read_document(&Self::path(root)?).map_err(|e| e.to_string())?;
        let state: Self = serde_json::from_value(document).map_err(|e| e.to_string())?;
        if !state.valid() {
            return Err("Invalid or unsupported workspace state".into());
        }
        Ok(state)
    }

    pub fn save(&self, root: &Path) -> Result<(), String> {
        if !self.valid() {
            return Err("Invalid workspace state".into());
        }
        let path = Self::path(root)?;
        // Preserve a corrupt or unsupported existing state instead of replacing it.
        Self::load(root)?;
        let mut document = read_document(&path).map_err(|e| e.to_string())?;
        let snapshot = serde_json::to_value(self).map_err(|e| e.to_string())?;
        document
            .as_object_mut()
            .ok_or("Invalid workspace state")?
            .extend(snapshot.as_object().unwrap().clone());
        write_document(&path, &document, true).map_err(|e| e.to_string())
    }
}

impl SessionStore {
    pub fn new(paths: &SettingsPaths) -> Self {
        Self {
            path: paths.user.with_file_name("state.json"),
            io: Arc::new(Mutex::new(())),
        }
    }

    pub fn last_volume(&self) -> Result<Option<PathBuf>, String> {
        let document = read_document(&self.path).map_err(|e| e.to_string())?;
        let session: Session = serde_json::from_value(document)
            .map_err(|e| format!("{}: {e}", self.path.display()))?;
        Ok(session.last_volume)
    }

    pub fn remember(&self, root: &Path) -> Result<(), String> {
        let _guard = self.io.lock().map_err(|_| "Session state lock poisoned")?;
        let root = root.canonicalize().map_err(|e| e.to_string())?;
        let mut document = read_document(&self.path).map_err(|e| e.to_string())?;
        document["last_volume"] = serde_json::to_value(root).map_err(|e| e.to_string())?;
        write_document(&self.path, &document, true).map_err(|e| e.to_string())
    }

    pub fn window(&self) -> Result<Option<WindowPlacement>, String> {
        let document = read_document(&self.path).map_err(|e| e.to_string())?;
        let session: Session = serde_json::from_value(document).map_err(|e| e.to_string())?;
        if session.window.as_ref().is_some_and(|w| !w.valid()) {
            return Err("Invalid saved window placement".into());
        }
        Ok(session.window)
    }

    pub fn remember_window(&self, window: &WindowPlacement) -> Result<(), String> {
        if !window.valid() {
            return Err("Invalid window placement".into());
        }
        let _guard = self.io.lock().map_err(|_| "Session state lock poisoned")?;
        let mut document = read_document(&self.path).map_err(|e| e.to_string())?;
        document["window"] = serde_json::to_value(window).map_err(|e| e.to_string())?;
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

    #[test]
    fn window_geometry_preserves_last_volume_and_preferences() {
        let dir = tempfile::tempdir().unwrap();
        let paths = SettingsPaths {
            global: dir.path().join("global.json"),
            user: dir.path().join("user/settings.json"),
        };
        let store = SessionStore::new(&paths);
        store.remember(dir.path()).unwrap();
        let window = WindowPlacement {
            width: 1234.0,
            height: 789.0,
            position: Some([-400.0, 120.0]),
        };
        store.remember_window(&window).unwrap();
        assert_eq!(SessionStore::new(&paths).window().unwrap(), Some(window));
        assert_eq!(
            store.last_volume().unwrap(),
            Some(dir.path().canonicalize().unwrap())
        );
        assert!(!paths.user.exists());
    }

    #[test]
    fn workspace_round_trips_relative_tabs_and_rejects_invalid_or_corrupt_state() {
        let dir = tempfile::tempdir().unwrap();
        let volume = crate::volume::VolumeStorage::open(dir.path()).unwrap();
        let workspace = WorkspaceSession {
            tabs: vec!["note.mde".into(), "Books/book.pdf".into()],
            active: Some(1),
            layout: PaneLayout::Split {
                vertical: true,
                ratio: 0.6,
                a: Box::new(PaneLayout::Pane { tab: Some(0) }),
                b: Box::new(PaneLayout::Pane { tab: Some(1) }),
            },
            ..Default::default()
        };
        workspace.save(&volume.root).unwrap();
        assert_eq!(WorkspaceSession::load(&volume.root).unwrap(), workspace);
        let path = volume.root.join(".enote/workspace-state.json");
        assert!(
            !std::fs::read_to_string(&path)
                .unwrap()
                .contains(volume.root.to_str().unwrap())
        );
        let mut invalid = workspace.clone();
        invalid.tabs[0] = "../outside.mde".into();
        assert!(invalid.save(&volume.root).is_err());
        invalid.tabs[0] = ".enote/settings.json".into();
        assert!(invalid.save(&volume.root).is_err());
        std::fs::write(&path, "broken").unwrap();
        assert!(workspace.save(&volume.root).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "broken");
    }
}
