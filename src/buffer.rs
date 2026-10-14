use crate::volume::{FileRef, Volume, VolumeId};
use std::{
    collections::{BTreeMap, VecDeque},
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct BufferId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    Image,
    Audio,
    Video,
    Binary,
    Document,
}

impl MediaKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Image => "Image",
            Self::Audio => "Audio",
            Self::Video => "Video",
            Self::Binary => "Binary",
            Self::Document => "Document",
        }
    }
}

#[derive(Debug, Clone)]
pub enum BufferContent {
    Text {
        text: String,
        saved: Vec<u8>,
    },
    Media {
        kind: MediaKind,
        bytes: Option<Arc<[u8]>>,
        size: u64,
    },
}

#[derive(Debug)]
pub struct Buffer {
    pub id: BufferId,
    pub source: Option<FileRef>,
    pub content: BufferContent,
    pub read_only: bool,
    pub changed_tick: u64,
    undo: VecDeque<String>,
    redo: VecDeque<String>,
}

impl Buffer {
    pub fn format(&self) -> crate::document::DocumentFormat {
        self.source
            .as_ref()
            .map(|source| crate::document::DocumentFormat::from_path(&source.path))
            .unwrap_or(crate::document::DocumentFormat::EnoteMarkdown)
    }
    pub fn text(&self) -> Option<&str> {
        match &self.content {
            BufferContent::Text { text, .. } => Some(text),
            _ => None,
        }
    }

    pub fn modified(&self) -> bool {
        matches!(&self.content, BufferContent::Text { text, saved } if text.as_bytes() != saved)
    }

    pub fn set_text(&mut self, value: String) {
        if !self.read_only
            && let BufferContent::Text { text, .. } = &mut self.content
            && *text != value
        {
            Self::remember(&mut self.undo, std::mem::replace(text, value));
            self.redo.clear();
            self.changed_tick += 1;
        }
    }

    fn remember(history: &mut VecDeque<String>, value: String) {
        history.push_back(value);
        let mut bytes: usize = history.iter().map(String::len).sum();
        while history.len() > 100 || bytes > 64 * 1024 * 1024 {
            bytes -= history.pop_front().unwrap().len();
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.read_only && !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.read_only && !self.redo.is_empty()
    }

    pub fn undo(&mut self) {
        self.restore(false);
    }
    pub fn redo(&mut self) {
        self.restore(true);
    }

    fn restore(&mut self, redo: bool) {
        if self.read_only {
            return;
        }
        let (from, to) = if redo {
            (&mut self.redo, &mut self.undo)
        } else {
            (&mut self.undo, &mut self.redo)
        };
        if let Some(previous) = from.pop_back()
            && let BufferContent::Text { text, .. } = &mut self.content
        {
            Self::remember(to, std::mem::replace(text, previous));
            self.changed_tick += 1;
        }
    }

    pub fn name(&self) -> String {
        self.source
            .as_ref()
            .and_then(|s| s.path.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| format!("Untitled {}", self.id.0))
    }
}

#[derive(Debug, Clone)]
pub struct LoadedFile {
    path: PathBuf,
    content: BufferContent,
    read_only: bool,
}

impl LoadedFile {
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn read(path: &Path) -> Result<Self, String> {
        let path = path.canonicalize().map_err(|e| e.to_string())?;
        let metadata = fs::metadata(&path).map_err(|e| e.to_string())?;
        if !metadata.is_file() {
            return Err("Select a regular file".into());
        }
        let extension = path
            .extension()
            .unwrap_or_default()
            .to_string_lossy()
            .to_ascii_lowercase();
        let kind = match extension.as_str() {
            "pdf" | "epub" | "fb2" | "mobi" | "cbz" | "xps" | "oxps" => Some(MediaKind::Document),
            "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "ico" | "tif" | "tiff" => {
                Some(MediaKind::Image)
            }
            "mp3" | "wav" | "flac" | "ogg" | "m4a" | "aac" => Some(MediaKind::Audio),
            "mp4" | "mov" | "webm" | "mkv" | "avi" => Some(MediaKind::Video),
            _ => None,
        };
        let content = if kind == Some(MediaKind::Document) {
            if metadata.len() > 256 * 1024 * 1024 {
                return Err("Documents are limited to 256 MiB".into());
            }
            BufferContent::Media {
                kind: MediaKind::Document,
                bytes: Some(fs::read(&path).map_err(|e| e.to_string())?.into()),
                size: metadata.len(),
            }
        } else if matches!(kind, Some(MediaKind::Audio | MediaKind::Video)) {
            BufferContent::Media {
                kind: kind.unwrap(),
                bytes: None,
                size: metadata.len(),
            }
        } else {
            if metadata.len() > 32 * 1024 * 1024 {
                return Err("Text and image buffers are limited to 32 MiB".into());
            }
            let bytes = fs::read(&path).map_err(|e| e.to_string())?;
            if let Some(kind) = kind {
                let mut reader = image::ImageReader::new(std::io::Cursor::new(&bytes))
                    .with_guessed_format()
                    .map_err(|e| e.to_string())?;
                let mut limits = image::Limits::default();
                limits.max_alloc = Some(64 * 1024 * 1024);
                reader.limits(limits);
                reader
                    .decode()
                    .map_err(|e| format!("Cannot decode image: {e}"))?;
                BufferContent::Media {
                    kind,
                    size: bytes.len() as u64,
                    bytes: Some(bytes.into()),
                }
            } else if let Ok(text) = std::str::from_utf8(&bytes)
                && !text.contains('\0')
            {
                BufferContent::Text {
                    text: text.to_owned(),
                    saved: bytes,
                }
            } else {
                BufferContent::Media {
                    kind: MediaKind::Binary,
                    bytes: None,
                    size: bytes.len() as u64,
                }
            }
        };
        let read_only =
            metadata.permissions().readonly() || matches!(content, BufferContent::Media { .. });
        Ok(Self {
            path,
            content,
            read_only,
        })
    }
}

#[derive(Debug, Default)]
pub struct Buffers {
    pub entries: BTreeMap<BufferId, Buffer>,
    volumes: BTreeMap<VolumeId, Volume>,
    next_id: u64,
}

impl Buffers {
    pub fn affected(&self, path: &Path) -> Vec<BufferId> {
        self.entries
            .values()
            .filter_map(|buffer| {
                let source = buffer.source.as_ref()?;
                let mounted = self.volumes.get(&source.volume)?;
                mounted
                    .root()
                    .join(&source.path)
                    .starts_with(path)
                    .then_some(buffer.id)
            })
            .collect()
    }

    pub fn relocate(&mut self, change: &crate::file_ops::PathChange) -> Result<(), String> {
        let updates: Vec<_> = self
            .entries
            .values()
            .filter_map(|buffer| {
                let source = buffer.source.as_ref()?;
                let path = self.volumes.get(&source.volume)?.root().join(&source.path);
                path.strip_prefix(&change.from)
                    .ok()
                    .map(|suffix| (buffer.id, change.to.join(suffix)))
            })
            .collect();
        for (id, path) in updates {
            let source = self.reference(&path)?;
            self.entries.get_mut(&id).unwrap().source = Some(source);
        }
        Ok(())
    }
    fn allocate(&mut self) -> BufferId {
        self.next_id += 1;
        BufferId(self.next_id)
    }

    pub fn scratch(&mut self) -> BufferId {
        let id = self.allocate();
        self.entries.insert(
            id,
            Buffer {
                id,
                source: None,
                read_only: false,
                changed_tick: 0,
                undo: VecDeque::new(),
                redo: VecDeque::new(),
                content: BufferContent::Text {
                    text: String::new(),
                    saved: Vec::new(),
                },
            },
        );
        id
    }

    pub fn mount(&mut self, root: &Path) -> Result<VolumeId, String> {
        let root = root.canonicalize().map_err(|e| e.to_string())?;
        if let Some(volume) = self.volumes.values().find(|v| v.root() == root) {
            return Ok(volume.id);
        }
        let id = VolumeId(self.volumes.len() as u64 + 1);
        self.volumes.insert(id, Volume::new(id, &root)?);
        Ok(id)
    }

    fn reference(&mut self, path: &Path) -> Result<FileRef, String> {
        if let Some(reference) = self.volumes.values().find_map(|v| v.reference(path)) {
            return Ok(reference);
        }
        let id = self.mount(path.parent().ok_or("Missing parent directory")?)?;
        self.volumes[&id]
            .reference(path)
            .ok_or("File is outside the volume".into())
    }

    pub fn path(&self, source: &FileRef) -> Result<PathBuf, String> {
        self.volumes
            .get(&source.volume)
            .ok_or("Volume is not mounted")?
            .resolve(source)
    }

    pub fn insert(&mut self, loaded: LoadedFile) -> Result<BufferId, String> {
        for buffer in self.entries.values() {
            if let Some(source) = &buffer.source
                && self.path(source)? == loaded.path
            {
                return Ok(buffer.id);
            }
        }
        let source = self.reference(&loaded.path)?;
        let id = self.allocate();
        self.entries.insert(
            id,
            Buffer {
                id,
                source: Some(source),
                content: loaded.content,
                read_only: loaded.read_only,
                changed_tick: 0,
                undo: VecDeque::new(),
                redo: VecDeque::new(),
            },
        );
        Ok(id)
    }

    pub fn close(&mut self, id: BufferId, discard: bool) -> Result<(), String> {
        if self.entries.get(&id).is_some_and(|b| b.modified()) && !discard {
            return Err("Buffer has unsaved changes".into());
        }
        self.entries.remove(&id);
        Ok(())
    }

    pub fn save_job(&self, id: BufferId, target: Option<PathBuf>) -> Result<SaveJob, String> {
        let buffer = self.entries.get(&id).ok_or("Buffer no longer exists")?;
        if buffer.read_only {
            return Err("Buffer is read-only".into());
        }
        let BufferContent::Text { text, saved } = &buffer.content else {
            return Err("Media buffers cannot be saved as text".into());
        };
        let (path, expected) = if let Some(target) = target {
            let parent = target
                .parent()
                .ok_or("Missing parent directory")?
                .canonicalize()
                .map_err(|e| e.to_string())?;
            let path = parent.join(target.file_name().ok_or("Missing filename")?);
            let path = if path.exists() {
                path.canonicalize().map_err(|e| e.to_string())?
            } else {
                path
            };
            for other in self.entries.values() {
                if other.id != id
                    && let Some(source) = &other.source
                    && self.path(source)? == path
                {
                    return Err("This file is already open in another buffer".into());
                }
            }
            let same_file = buffer
                .source
                .as_ref()
                .map(|source| self.path(source))
                .transpose()?
                .is_some_and(|original| original == path);
            let expected = if same_file {
                Some(saved.clone())
            } else {
                match fs::read(&path) {
                    Ok(bytes) => Some(bytes),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                    Err(e) => return Err(e.to_string()),
                }
            };
            (path, expected)
        } else if let Some(source) = &buffer.source {
            (self.path(source)?, Some(saved.clone()))
        } else {
            return Err("Choose a path for the buffer".into());
        };
        Ok(SaveJob {
            id,
            path,
            text: text.clone(),
            expected,
        })
    }

    pub fn saved(&mut self, saved: SavedFile) -> Result<(), String> {
        let source = self.reference(&saved.path)?;
        if let Some(buffer) = self.entries.get_mut(&saved.id) {
            buffer.source = Some(source);
            if let BufferContent::Text {
                saved: baseline, ..
            } = &mut buffer.content
            {
                *baseline = saved.text.into_bytes();
            }
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct SaveJob {
    id: BufferId,
    path: PathBuf,
    text: String,
    expected: Option<Vec<u8>>,
}
#[derive(Debug, Clone)]
pub struct SavedFile {
    pub id: BufferId,
    path: PathBuf,
    text: String,
}

impl SaveJob {
    pub fn write(self) -> Result<SavedFile, String> {
        let current = match fs::read(&self.path) {
            Ok(bytes) => Some(bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.to_string()),
        };
        if current != self.expected {
            return Err("File changed on disk. Reopen it before saving.".into());
        }
        let mut temp =
            tempfile::NamedTempFile::new_in(self.path.parent().ok_or("Missing parent directory")?)
                .map_err(|e| e.to_string())?;
        if let Ok(metadata) = fs::metadata(&self.path) {
            temp.as_file()
                .set_permissions(metadata.permissions())
                .map_err(|e| e.to_string())?;
        }
        temp.write_all(self.text.as_bytes())
            .map_err(|e| e.to_string())?;
        temp.as_file().sync_all().map_err(|e| e.to_string())?;
        if self.expected.is_none() {
            temp.persist_noclobber(&self.path)
                .map_err(|e| e.to_string())?;
        } else {
            temp.persist(&self.path).map_err(|e| e.to_string())?;
        }
        Ok(SavedFile {
            id: self.id,
            path: self.path,
            text: self.text,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_as_changes_source_preserves_original_and_rejects_same_file_conflicts() {
        let dir = tempfile::tempdir().unwrap();
        let original = dir.path().join("original.mde");
        let copy = dir.path().join("copy.mde");
        fs::write(&original, "original").unwrap();
        let mut buffers = Buffers::default();
        let id = buffers
            .insert(LoadedFile::read(&original).unwrap())
            .unwrap();
        buffers
            .entries
            .get_mut(&id)
            .unwrap()
            .set_text("copy".into());
        let saved = buffers
            .save_job(id, Some(copy.clone()))
            .unwrap()
            .write()
            .unwrap();
        buffers.saved(saved).unwrap();
        assert_eq!(fs::read_to_string(&original).unwrap(), "original");
        assert_eq!(fs::read_to_string(&copy).unwrap(), "copy");
        assert_eq!(
            buffers
                .path(buffers.entries[&id].source.as_ref().unwrap())
                .unwrap(),
            copy.canonicalize().unwrap()
        );
        assert!(!buffers.entries[&id].modified());
        fs::write(&copy, "external").unwrap();
        assert!(buffers.save_job(id, Some(copy)).unwrap().write().is_err());
    }

    #[test]
    fn valid_images_become_media_and_invalid_images_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("image.png");
        image::RgbaImage::new(2, 2).save(&path).unwrap();
        let loaded = LoadedFile::read(&path).unwrap();
        assert!(matches!(
            loaded.content,
            BufferContent::Media {
                kind: MediaKind::Image,
                bytes: Some(_),
                ..
            }
        ));
        assert!(loaded.read_only);
        fs::write(&path, "not an image").unwrap();
        assert!(LoadedFile::read(&path).is_err());
    }

    #[test]
    fn mounted_volume_is_used_for_nested_files() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("notes")).unwrap();
        let path = dir.path().join("notes/note.md");
        fs::write(&path, "note").unwrap();
        let mut buffers = Buffers::default();
        let volume = buffers.mount(dir.path()).unwrap();
        let id = buffers.insert(LoadedFile::read(&path).unwrap()).unwrap();
        let source = buffers.entries[&id].source.as_ref().unwrap();
        assert_eq!(source.volume, volume);
        assert_eq!(source.path, PathBuf::from("notes/note.md"));
    }

    #[test]
    fn scratch_dirty_state_and_guarded_close() {
        let mut buffers = Buffers::default();
        let id = buffers.scratch();
        assert!(!buffers.entries[&id].modified());
        buffers
            .entries
            .get_mut(&id)
            .unwrap()
            .set_text("note".into());
        assert!(buffers.close(id, false).is_err());
        buffers
            .entries
            .get_mut(&id)
            .unwrap()
            .set_text(String::new());
        assert!(!buffers.entries[&id].modified());
        buffers.close(id, false).unwrap();
    }

    #[test]
    fn reopening_preserves_unsaved_content() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("note.md");
        fs::write(&path, "original\r\n").unwrap();
        let mut buffers = Buffers::default();
        let id = buffers.insert(LoadedFile::read(&path).unwrap()).unwrap();
        buffers
            .entries
            .get_mut(&id)
            .unwrap()
            .set_text("changed".into());
        assert_eq!(
            id,
            buffers.insert(LoadedFile::read(&path).unwrap()).unwrap()
        );
        assert_eq!(buffers.entries[&id].text(), Some("changed"));
    }

    #[test]
    fn save_preserves_edits_made_while_writing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("note.md");
        let mut buffers = Buffers::default();
        let id = buffers.scratch();
        buffers
            .entries
            .get_mut(&id)
            .unwrap()
            .set_text("first\r\n".into());
        let job = buffers.save_job(id, Some(path.clone())).unwrap();
        buffers
            .entries
            .get_mut(&id)
            .unwrap()
            .set_text("second".into());
        buffers.saved(job.write().unwrap()).unwrap();
        assert_eq!(fs::read_to_string(path).unwrap(), "first\r\n");
        assert!(buffers.entries[&id].modified());
        let job = buffers.save_job(id, None).unwrap();
        buffers.saved(job.write().unwrap()).unwrap();
        assert!(!buffers.entries[&id].modified());
    }

    #[test]
    fn refuses_external_changes_and_duplicate_save_targets() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("note.md");
        fs::write(&path, "original").unwrap();
        let mut buffers = Buffers::default();
        let id = buffers.insert(LoadedFile::read(&path).unwrap()).unwrap();
        buffers
            .entries
            .get_mut(&id)
            .unwrap()
            .set_text("edited".into());
        fs::write(&path, "external").unwrap();
        assert!(buffers.save_job(id, None).unwrap().write().is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "external");
        let scratch = buffers.scratch();
        assert!(buffers.save_job(scratch, Some(path)).is_err());
    }

    #[test]
    fn binary_and_media_are_read_only() {
        let dir = tempfile::tempdir().unwrap();
        let mut buffers = Buffers::default();
        for (name, bytes, expected) in [
            ("data.bin", b"a\0b".as_slice(), MediaKind::Binary),
            ("clip.mp4", b"video".as_slice(), MediaKind::Video),
        ] {
            let path = dir.path().join(name);
            fs::write(&path, bytes).unwrap();
            let id = buffers.insert(LoadedFile::read(&path).unwrap()).unwrap();
            assert!(
                matches!(buffers.entries[&id].content, BufferContent::Media { kind, .. } if kind == expected)
            );
            assert!(buffers.save_job(id, None).is_err());
        }
    }
}
