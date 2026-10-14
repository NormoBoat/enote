use crate::{
    settings::{read_document, write_document},
    volume::VolumeStorage,
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathChange {
    pub from: PathBuf,
    pub to: PathBuf,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, VolumeStorage) {
        let dir = tempfile::tempdir().unwrap();
        let volume = VolumeStorage::open(dir.path()).unwrap();
        fs::create_dir_all(volume.root.join("A/nested")).unwrap();
        fs::create_dir(volume.root.join("B")).unwrap();
        fs::write(volume.root.join("A/nested/note.mde"), "original").unwrap();
        (dir, volume)
    }

    #[test]
    fn imports_pdf_without_overwriting_or_removing_original() {
        let (_dir, volume) = fixture();
        let external = tempfile::tempdir().unwrap();
        let source = external.path().join("book.pdf");
        let bytes = include_bytes!("../tests/fixtures/reader.pdf");
        fs::write(&source, bytes).unwrap();
        let destination = volume.import_file(&source, &volume.root.join("B")).unwrap();
        assert_eq!(fs::read(&destination).unwrap(), bytes);
        assert!(source.exists());
        fs::write(&source, "different").unwrap();
        assert!(volume.import_file(&source, &volume.root.join("B")).is_err());
        assert_eq!(fs::read(&destination).unwrap(), bytes);
        assert!(
            volume
                .import_file(&source, &volume.root.join(".enote"))
                .is_err()
        );
    }

    #[test]
    fn moves_files_and_whole_folders_without_overwriting() {
        let (_dir, volume) = fixture();
        let root = &volume.root;
        let change = volume
            .move_entry(&root.join("A/nested/note.mde"), &root.join("B"))
            .unwrap();
        assert!(!change.from.exists());
        assert_eq!(fs::read_to_string(&change.to).unwrap(), "original");
        fs::write(root.join("A/note.mde"), "other").unwrap();
        assert!(
            volume
                .move_entry(&root.join("A/note.mde"), &root.join("B"))
                .is_err()
        );
        assert_eq!(
            fs::read_to_string(root.join("B/note.mde")).unwrap(),
            "original"
        );
        let change = volume.move_entry(&root.join("A"), &root.join("B")).unwrap();
        assert!(change.to.join("nested").is_dir());
        assert!(change.to.join("note.mde").is_file());
    }

    #[test]
    fn protects_root_technical_data_descendants_and_other_volumes() {
        let (_dir, volume) = fixture();
        let outside = tempfile::tempdir().unwrap();
        let root = &volume.root;
        for (source, destination) in [
            (root.clone(), root.join("B")),
            (root.join("A"), root.join("A/nested")),
            (root.join("A"), root.join(".enote")),
            (root.join("A"), outside.path().to_owned()),
            (root.join(".enote"), root.join("B")),
        ] {
            assert!(volume.move_entry(&source, &destination).is_err());
        }
        assert!(volume.trash_entry(root).is_err());
        assert!(volume.rename_entry(&root.join("A"), ".enote").is_err());
        assert!(root.join("A/nested/note.mde").exists());
    }

    #[test]
    fn renames_preserving_extension_and_duplicates_recursively() {
        let (_dir, volume) = fixture();
        let file = volume.root.join("A/nested/note.mde");
        let change = volume.rename_entry(&file, "Renamed").unwrap();
        assert!(change.to.ends_with("Renamed.mde"));
        let copy = volume.duplicate_entry(&change.to).unwrap();
        assert!(copy.ends_with("Renamed copy.mde"));
        assert!(
            volume
                .duplicate_entry(&change.to)
                .unwrap()
                .ends_with("Renamed copy 2.mde")
        );
        let folder = volume.duplicate_entry(&volume.root.join("A")).unwrap();
        assert_eq!(
            fs::read_to_string(folder.join("nested/Renamed.mde")).unwrap(),
            "original"
        );
        fs::write(volume.root.join("many.dots.mde"), "dots").unwrap();
        assert!(
            volume
                .duplicate_entry(&volume.root.join("many.dots.mde"))
                .unwrap()
                .ends_with("many.dots copy.mde")
        );
    }

    #[test]
    fn trash_survives_reopening_and_restore_refuses_collisions() {
        let (dir, volume) = fixture();
        let source = volume.root.join("A");
        let record = volume.trash_entry(&source).unwrap();
        assert!(!source.exists());
        let volume = VolumeStorage::open(dir.path()).unwrap();
        let latest = volume.latest_trash().unwrap().unwrap();
        assert_eq!(latest.id, record.id);
        assert_eq!(latest.original, vec!["A"]);
        fs::create_dir(&source).unwrap();
        assert!(volume.restore_entry(&latest).is_err());
        fs::remove_dir(&source).unwrap();
        assert_eq!(volume.restore_entry(&latest).unwrap(), source);
        assert_eq!(
            fs::read_to_string(source.join("nested/note.mde")).unwrap(),
            "original"
        );
        assert!(volume.latest_trash().unwrap().is_none());
    }

    #[cfg(unix)]
    #[test]
    fn refuses_linked_entries_and_linked_trash() {
        let (_dir, volume) = fixture();
        std::os::unix::fs::symlink(volume.root.join("A"), volume.root.join("linked")).unwrap();
        assert!(
            volume
                .move_entry(&volume.root.join("linked"), &volume.root.join("B"))
                .is_err()
        );
        assert!(volume.duplicate_entry(&volume.root.join("linked")).is_err());
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), volume.root.join(".enote/trash")).unwrap();
        assert!(volume.trash_entry(&volume.root.join("A")).is_err());
        assert!(volume.root.join("A").exists());
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrashRecord {
    pub id: Uuid,
    pub volume: Uuid,
    pub original: Vec<String>,
    pub deleted_at: u64,
}

fn absent(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(_) => Err("An object with this name already exists".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

fn copy_tree(source: &Path, destination: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(source).map_err(|e| e.to_string())?;
    if metadata.file_type().is_symlink() {
        return Err("Linked objects cannot be duplicated".into());
    }
    if metadata.is_file() {
        fs::copy(source, destination).map_err(|e| e.to_string())?;
    } else if metadata.is_dir() {
        fs::create_dir(destination).map_err(|e| e.to_string())?;
        for child in fs::read_dir(source).map_err(|e| e.to_string())? {
            let child = child.map_err(|e| e.to_string())?;
            copy_tree(&child.path(), &destination.join(child.file_name()))?;
        }
    } else {
        return Err("Only regular files and folders are supported".into());
    }
    Ok(())
}

impl VolumeStorage {
    pub fn import_file(&self, source: &Path, parent: &Path) -> Result<PathBuf, String> {
        let parent = self.directory_path(parent)?;
        let name = source
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or("Invalid file name")?;
        Self::validate_name(name)?;
        let target = parent.join(name);
        self.document_path(&target)?;
        let metadata = fs::symlink_metadata(source).map_err(|e| e.to_string())?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err("Drop regular files to import them into the volume".into());
        }
        absent(&target)?;
        let mut input = fs::File::open(source).map_err(|e| e.to_string())?;
        let mut staged = tempfile::NamedTempFile::new_in(&parent).map_err(|e| e.to_string())?;
        std::io::copy(&mut input, &mut staged).map_err(|e| e.to_string())?;
        staged.as_file().sync_all().map_err(|e| e.to_string())?;
        staged
            .persist_noclobber(&target)
            .map_err(|e| e.to_string())?;
        Ok(target)
    }

    pub fn directory_path(&self, path: &Path) -> Result<PathBuf, String> {
        let resolved = path.canonicalize().map_err(|e| e.to_string())?;
        if resolved != self.root {
            self.document_path(&resolved)?;
        }
        if !resolved.is_dir() {
            return Err("Destination must be a folder".into());
        }
        Ok(resolved)
    }

    fn existing_entry(&self, path: &Path) -> Result<PathBuf, String> {
        let metadata = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
        if metadata.file_type().is_symlink() || !(metadata.is_file() || metadata.is_dir()) {
            return Err("Only regular files and folders are supported".into());
        }
        self.document_path(path)
    }

    fn relocate(&self, source: &Path, destination: PathBuf) -> Result<PathChange, String> {
        let from = self.existing_entry(source)?;
        if from == destination {
            return Err("Object is already at this location".into());
        }
        if from.is_dir() && destination.starts_with(&from) {
            return Err("A folder cannot be moved into itself".into());
        }
        self.document_path(&destination)?;
        absent(&destination)?;
        fs::rename(&from, &destination).map_err(|e| e.to_string())?;
        Ok(PathChange {
            from,
            to: destination,
        })
    }

    pub fn move_entry(
        &self,
        source: &Path,
        destination_folder: &Path,
    ) -> Result<PathChange, String> {
        let folder = self.directory_path(destination_folder)?;
        let name = source.file_name().ok_or("Missing object name")?;
        self.relocate(source, folder.join(name))
    }

    pub fn rename_entry(&self, source: &Path, name: &str) -> Result<PathChange, String> {
        let from = self.existing_entry(source)?;
        let name = name.trim();
        Self::validate_name(name)?;
        if name.starts_with('.') || name.ends_with('.') {
            return Err("Choose a visible file or folder name".into());
        }
        let mut target = from.parent().unwrap().join(name);
        if from.is_file()
            && target.extension().is_none()
            && let Some(extension) = from.extension()
        {
            target.set_extension(extension);
        }
        self.relocate(&from, target)
    }

    pub fn duplicate_entry(&self, source: &Path) -> Result<PathBuf, String> {
        let source = self.existing_entry(source)?;
        let parent = source.parent().unwrap();
        let stem = if source.is_file() {
            source.file_stem()
        } else {
            source.file_name()
        }
        .unwrap()
        .to_string_lossy();
        let extension = if source.is_file() {
            source.extension()
        } else {
            None
        };
        let mut number = 1;
        let target = loop {
            let suffix = if number == 1 {
                " copy".to_string()
            } else {
                format!(" copy {number}")
            };
            let mut name = std::ffi::OsString::from(format!("{stem}{suffix}"));
            if let Some(extension) = extension {
                name.push(".");
                name.push(extension);
            }
            let target = parent.join(name);
            match fs::symlink_metadata(&target) {
                Ok(_) => number += 1,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => break target,
                Err(error) => return Err(error.to_string()),
            }
        };
        let staging = tempfile::tempdir_in(parent).map_err(|e| e.to_string())?;
        let staged = staging.path().join("entry");
        copy_tree(&source, &staged)?;
        absent(&target)?;
        fs::rename(staged, &target).map_err(|e| e.to_string())?;
        Ok(target)
    }

    fn trash_root(&self) -> Result<PathBuf, String> {
        let technical = self.root.join(".enote");
        let metadata = fs::symlink_metadata(&technical).map_err(|e| e.to_string())?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err("Invalid technical directory".into());
        }
        let path = technical.join("trash");
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => (),
            Ok(_) => return Err("Invalid volume trash directory".into()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(&path).map_err(|e| e.to_string())?
            }
            Err(error) => return Err(error.to_string()),
        }
        Ok(path)
    }

    pub fn trash_entry(&self, source: &Path) -> Result<TrashRecord, String> {
        let source = self.existing_entry(source)?;
        let original = source
            .strip_prefix(&self.root)
            .unwrap()
            .components()
            .map(|part| {
                part.as_os_str()
                    .to_str()
                    .map(str::to_owned)
                    .ok_or("Trash requires a Unicode file name".to_string())
            })
            .collect::<Result<Vec<_>, _>>()?;
        let record = TrashRecord {
            id: Uuid::new_v4(),
            volume: self.manifest.id,
            original,
            deleted_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|e| e.to_string())?
                .as_millis() as u64,
        };
        let directory = self.trash_root()?.join(record.id.to_string());
        fs::create_dir(&directory).map_err(|e| e.to_string())?;
        write_document(
            &directory.join("record.json"),
            &serde_json::to_value(&record).map_err(|e| e.to_string())?,
            false,
        )
        .map_err(|e| e.to_string())?;
        if let Err(error) = fs::rename(&source, directory.join("entry")) {
            let _ = fs::remove_dir_all(&directory);
            return Err(error.to_string());
        }
        Ok(record)
    }

    pub fn latest_trash(&self) -> Result<Option<TrashRecord>, String> {
        let root = self.root.join(".enote/trash");
        if !root.try_exists().map_err(|e| e.to_string())? {
            return Ok(None);
        }
        let root = self.trash_root()?;
        let mut records = Vec::new();
        for entry in fs::read_dir(root).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            if !entry.file_type().map_err(|e| e.to_string())?.is_dir() {
                continue;
            }
            let Ok(document) = read_document(&entry.path().join("record.json")) else {
                continue;
            };
            let Ok(record) = serde_json::from_value::<TrashRecord>(document) else {
                continue;
            };
            if record.volume == self.manifest.id
                && entry.file_name() == record.id.to_string().as_str()
                && entry.path().join("entry").exists()
            {
                records.push(record);
            }
        }
        records.sort_by_key(|record| record.deleted_at);
        Ok(records.pop())
    }

    pub fn restore_entry(&self, record: &TrashRecord) -> Result<PathBuf, String> {
        if record.volume != self.manifest.id || record.original.is_empty() {
            return Err("Trash record belongs to another volume".into());
        }
        let directory = self.trash_root()?.join(record.id.to_string());
        let metadata = fs::symlink_metadata(&directory).map_err(|e| e.to_string())?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err("Invalid trash record directory".into());
        }
        let stored: TrashRecord = serde_json::from_value(
            read_document(&directory.join("record.json")).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        if stored.id != record.id
            || stored.volume != record.volume
            || stored.original != record.original
        {
            return Err("Trash record changed".into());
        }
        let mut destination = self.root.clone();
        for part in &record.original {
            Self::validate_name(part)?;
            destination.push(part);
        }
        self.document_path(&destination)?;
        absent(&destination)?;
        let payload = directory.join("entry");
        if fs::symlink_metadata(&payload)
            .map_err(|e| e.to_string())?
            .file_type()
            .is_symlink()
        {
            return Err("Invalid trash payload".into());
        }
        fs::rename(payload, &destination).map_err(|e| e.to_string())?;
        let _ = fs::remove_dir_all(directory);
        Ok(destination)
    }
}
