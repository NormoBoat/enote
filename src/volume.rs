use crate::settings::{read_document, write_document};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Component, Path, PathBuf};
use uuid::Uuid;

pub const TECHNICAL_DIRECTORY: &str = ".enote";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    File,
    Folder,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VolumeManifest {
    pub version: u32,
    pub id: Uuid,
}

#[derive(Debug, Clone)]
pub struct VolumeStorage {
    pub root: PathBuf,
    pub manifest: VolumeManifest,
}

impl VolumeStorage {
    pub fn create_entry(
        &self,
        parent: &Path,
        name: &str,
        kind: EntryKind,
    ) -> Result<PathBuf, String> {
        let parent = parent.canonicalize().map_err(|e| e.to_string())?;
        if !parent.is_dir() {
            return Err("Parent must be a folder".into());
        }
        let relative = parent
            .strip_prefix(&self.root)
            .map_err(|_| "Folder is outside the volume")?;
        if relative
            .components()
            .any(|part| part.as_os_str() == TECHNICAL_DIRECTORY)
        {
            return Err("Cannot create documents inside .enote".into());
        }
        let name = name.trim();
        Self::validate_name(name)?;
        if name.starts_with('.') || name.ends_with('.') {
            return Err("Choose a visible file or folder name".into());
        }
        let mut path = parent.join(name);
        if kind == EntryKind::File && path.extension().is_none() {
            path.set_extension(crate::document::DEFAULT_NOTE_EXTENSION);
        }
        match kind {
            EntryKind::File => {
                fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&path)
                    .map_err(|e| format!("Cannot create file: {e}"))?;
            }
            EntryKind::Folder => {
                fs::create_dir(&path).map_err(|e| format!("Cannot create folder: {e}"))?
            }
        }
        Ok(path)
    }
    pub fn open(root: &Path) -> Result<Self, String> {
        let root = root.canonicalize().map_err(|e| e.to_string())?;
        if root
            .components()
            .any(|part| part.as_os_str() == TECHNICAL_DIRECTORY)
        {
            return Err("Technical directories cannot be opened as volumes".into());
        }
        if !root.is_dir() {
            return Err("Volume root must be a directory".into());
        }
        let technical = root.join(TECHNICAL_DIRECTORY);
        if let Ok(metadata) = fs::symlink_metadata(&technical) {
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(".enote must be a regular directory".into());
            }
        } else {
            fs::create_dir(&technical).map_err(|e| e.to_string())?;
        }
        for filename in ["volume.json", "settings.json"] {
            if let Ok(metadata) = fs::symlink_metadata(technical.join(filename))
                && (metadata.file_type().is_symlink() || !metadata.is_file())
            {
                return Err(format!(".enote/{filename} must be a regular file"));
            }
        }
        let manifest_path = technical.join("volume.json");
        if !manifest_path.exists() {
            let manifest = VolumeManifest {
                version: 1,
                id: Uuid::new_v4(),
            };
            write_document(
                &manifest_path,
                &serde_json::to_value(manifest).map_err(|e| e.to_string())?,
                false,
            )
            .map_err(|e| e.to_string())?;
        }
        let manifest: VolumeManifest =
            serde_json::from_value(read_document(&manifest_path).map_err(|e| e.to_string())?)
                .map_err(|e| format!("Invalid volume manifest: {e}"))?;
        if manifest.version != 1 {
            return Err(format!("Unsupported volume version: {}", manifest.version));
        }
        let settings_path = technical.join("settings.json");
        if !settings_path.exists() {
            write_document(&settings_path, &serde_json::json!({}), false)
                .map_err(|e| e.to_string())?;
        }
        Ok(Self { root, manifest })
    }

    pub fn create(parent: &Path, name: &str) -> Result<Self, String> {
        let name = name.trim();
        Self::validate_name(name)?;
        let parent = parent.canonicalize().map_err(|e| e.to_string())?;
        let root = parent.join(name);
        fs::create_dir(&root).map_err(|e| format!("Cannot create volume: {e}"))?;
        Self::open(&root)
    }

    pub fn validate_name(name: &str) -> Result<(), String> {
        let name = name.trim();
        let mut components = Path::new(name).components();
        if name.is_empty()
            || name.len() > 128
            || name.contains(['/', '\\', ':', '*', '?', '"', '<', '>', '|'])
            || !matches!(components.next(), Some(Component::Normal(_)))
            || components.next().is_some()
            || name == TECHNICAL_DIRECTORY
        {
            return Err("Choose a folder name without path separators".into());
        }
        Ok(())
    }

    pub fn settings_path(&self) -> PathBuf {
        self.root.join(TECHNICAL_DIRECTORY).join("settings.json")
    }

    pub fn document_path(&self, path: &Path) -> Result<PathBuf, String> {
        let resolved = if path.exists() {
            path.canonicalize()
        } else {
            path.parent()
                .ok_or("Missing parent directory")?
                .canonicalize()
                .map(|parent| parent.join(path.file_name().unwrap_or_default()))
        }
        .map_err(|e| e.to_string())?;
        let relative = resolved
            .strip_prefix(&self.root)
            .map_err(|_| "File is outside the volume")?;
        if relative.as_os_str().is_empty()
            || relative
                .components()
                .any(|part| part.as_os_str() == TECHNICAL_DIRECTORY)
        {
            return Err("Technical volume files are not documents".into());
        }
        Ok(resolved)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct VolumeId(pub u64);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRef {
    pub volume: VolumeId,
    pub path: PathBuf,
}

#[derive(Debug)]
pub struct Volume {
    pub id: VolumeId,
    root: PathBuf,
}

impl Volume {
    pub fn new(id: VolumeId, root: &Path) -> Result<Self, String> {
        let root = root.canonicalize().map_err(|e| e.to_string())?;
        if root
            .components()
            .any(|part| part.as_os_str() == TECHNICAL_DIRECTORY)
        {
            return Err("Technical directories cannot be mounted as volumes".into());
        }
        if !root.is_dir() {
            return Err("Volume root must be a directory".into());
        }
        Ok(Self { id, root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn resolve(&self, file: &FileRef) -> Result<PathBuf, String> {
        if file.volume != self.id
            || file.path.as_os_str().is_empty()
            || file
                .path
                .components()
                .any(|c| !matches!(c, Component::Normal(_)) || c.as_os_str() == TECHNICAL_DIRECTORY)
        {
            return Err("Invalid path in volume".into());
        }
        let path = self.root.join(&file.path);
        // Resolve symlinks too: a relative path alone does not guarantee containment.
        let resolved = if path.exists() {
            path.canonicalize()
        } else {
            path.parent()
                .ok_or("Missing parent directory")?
                .canonicalize()
                .map(|parent| parent.join(path.file_name().unwrap()))
        }
        .map_err(|e| e.to_string())?;
        if !resolved.starts_with(&self.root) {
            return Err("File is outside the volume".into());
        }
        if resolved
            .strip_prefix(&self.root)
            .unwrap()
            .components()
            .any(|part| part.as_os_str() == TECHNICAL_DIRECTORY)
        {
            return Err("Technical volume files are not documents".into());
        }
        Ok(resolved)
    }

    pub fn reference(&self, path: &Path) -> Option<FileRef> {
        path.strip_prefix(&self.root)
            .ok()
            .filter(|path| {
                !path
                    .components()
                    .any(|part| part.as_os_str() == TECHNICAL_DIRECTORY)
            })
            .map(|path| FileRef {
                volume: self.id,
                path: path.to_owned(),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_entries_without_overwrites_traversal_or_technical_access() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let volume = VolumeStorage::open(dir.path()).unwrap();
        let folder = volume
            .create_entry(&volume.root, "Research", EntryKind::Folder)
            .unwrap();
        let note = volume
            .create_entry(&folder, "Note", EntryKind::File)
            .unwrap();
        assert!(note.ends_with("Note.mde"));
        fs::write(&note, "existing").unwrap();
        assert!(
            volume
                .create_entry(&folder, "Note", EntryKind::File)
                .is_err()
        );
        assert_eq!(fs::read_to_string(&note).unwrap(), "existing");
        assert!(
            volume
                .create_entry(&volume.root, "Research", EntryKind::Folder)
                .is_err()
        );
        assert!(
            volume
                .create_entry(&folder, "../escape", EntryKind::File)
                .is_err()
        );
        assert!(
            volume
                .create_entry(&volume.root.join(".enote"), "note", EntryKind::File)
                .is_err()
        );
        assert!(
            volume
                .create_entry(outside.path(), "note", EntryKind::File)
                .is_err()
        );
        assert!(
            volume
                .create_entry(&folder, ".hidden", EntryKind::File)
                .is_err()
        );
        assert!(
            volume
                .create_entry(&folder, "ordinary.md", EntryKind::File)
                .unwrap()
                .ends_with("ordinary.md")
        );
    }

    #[test]
    fn creates_portable_technical_files_and_keeps_identity_when_reopened_or_moved() {
        let dir = tempfile::tempdir().unwrap();
        let volume = VolumeStorage::create(dir.path(), "Notes").unwrap();
        assert!(volume.root.join(".enote/volume.json").is_file());
        assert_eq!(
            read_document(&volume.settings_path()).unwrap(),
            serde_json::json!({})
        );
        fs::write(volume.root.join("note.md"), "note").unwrap();
        let reopened = VolumeStorage::open(&volume.root).unwrap();
        assert_eq!(volume.manifest.id, reopened.manifest.id);
        let moved = dir.path().join("Moved");
        fs::rename(&volume.root, &moved).unwrap();
        let reopened = VolumeStorage::open(&moved).unwrap();
        assert_eq!(volume.manifest.id, reopened.manifest.id);
        assert_eq!(fs::read_to_string(moved.join("note.md")).unwrap(), "note");
    }

    #[test]
    fn opens_existing_folder_without_overwriting_documents_or_settings() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("note.md"), "existing").unwrap();
        let volume = VolumeStorage::open(dir.path()).unwrap();
        fs::write(volume.settings_path(), "{\"editor\":{\"font_size\":20}}").unwrap();
        VolumeStorage::open(dir.path()).unwrap();
        assert_eq!(
            fs::read_to_string(volume.settings_path()).unwrap(),
            "{\"editor\":{\"font_size\":20}}"
        );
        assert_eq!(
            fs::read_to_string(dir.path().join("note.md")).unwrap(),
            "existing"
        );
        assert!(VolumeStorage::create(dir.path(), "../outside").is_err());
        assert!(VolumeStorage::create(dir.path(), "").is_err());
        assert!(VolumeStorage::create(dir.path(), ".enote").is_err());
        assert!(VolumeStorage::create(dir.path(), "note.md").is_err());
    }

    #[test]
    fn technical_files_and_outside_paths_cannot_be_documents() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let volume = VolumeStorage::open(dir.path()).unwrap();
        assert!(volume.document_path(&volume.settings_path()).is_err());
        assert!(VolumeStorage::open(&dir.path().join(".enote")).is_err());
        assert!(Volume::new(VolumeId(2), &dir.path().join(".enote")).is_err());
        assert!(
            volume
                .document_path(&outside.path().join("note.md"))
                .is_err()
        );
        assert!(volume.document_path(&dir.path().join("note.md")).is_ok());
        let mounted = Volume::new(VolumeId(1), dir.path()).unwrap();
        assert!(mounted.reference(&volume.settings_path()).is_none());
        assert!(
            mounted
                .resolve(&FileRef {
                    volume: mounted.id,
                    path: ".enote/settings.json".into()
                })
                .is_err()
        );
    }

    #[cfg(unix)]
    #[test]
    fn refuses_technical_directory_symlink() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), dir.path().join(".enote")).unwrap();
        assert!(VolumeStorage::open(dir.path()).is_err());
        assert!(!outside.path().join("volume.json").exists());
    }

    #[test]
    fn refuses_unsupported_format_without_overwriting_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let volume = VolumeStorage::open(dir.path()).unwrap();
        let path = volume.root.join(".enote/volume.json");
        let manifest = serde_json::json!({"version": 99, "id": volume.manifest.id});
        fs::write(&path, manifest.to_string()).unwrap();
        assert!(VolumeStorage::open(dir.path()).is_err());
        assert_eq!(read_document(&path).unwrap(), manifest);
    }

    #[test]
    fn rejects_traversal_and_wrong_volume() {
        let dir = tempfile::tempdir().unwrap();
        let volume = Volume::new(VolumeId(1), dir.path()).unwrap();
        for (id, path) in [(1, "../outside"), (1, "/outside"), (2, "note.md")] {
            assert!(
                volume
                    .resolve(&FileRef {
                        volume: VolumeId(id),
                        path: path.into()
                    })
                    .is_err()
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_outside_volume() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), dir.path().join("link")).unwrap();
        let volume = Volume::new(VolumeId(1), dir.path()).unwrap();
        assert!(
            volume
                .resolve(&FileRef {
                    volume: volume.id,
                    path: "link/note.md".into()
                })
                .is_err()
        );
    }
}
