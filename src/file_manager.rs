use crate::explorer::Directory;
use enote::{
    file_ops::{PathChange, TrashRecord},
    volume::{EntryKind, VolumeStorage},
};
use iced::Point;
use std::{collections::BTreeSet, path::PathBuf};

#[derive(Debug, Clone)]
pub struct Target {
    pub path: PathBuf,
    pub kind: EntryKind,
}

#[derive(Debug, Clone)]
pub struct ContextMenu {
    pub target: Target,
    pub position: Point,
}

#[derive(Debug, Clone)]
pub struct Drag {
    pub source: Target,
    pub start: Point,
    pub moving: bool,
}

impl Drag {
    pub fn accepts(&self, target: &Target, root: &std::path::Path) -> bool {
        self.source.path != root
            && target.kind == EntryKind::Folder
            && target.path.starts_with(root)
            && !target.path.starts_with(&self.source.path)
            && self.source.path.parent() != Some(target.path.as_path())
    }
}

#[derive(Debug, Clone, Copy)]
pub enum ContextAction {
    Open,
    NewFile,
    NewFolder,
    Rename,
    Move,
    Duplicate,
    CopyPath,
    Delete,
    Restore,
    Refresh,
    CollapseAll,
}

#[derive(Debug, Clone)]
pub enum FilePrompt {
    Move {
        source: PathBuf,
        destination: PathBuf,
        directory: Directory,
    },
    Delete {
        source: PathBuf,
        modified: bool,
    },
}

#[derive(Debug, Clone)]
pub enum FileOperation {
    Import {
        source: PathBuf,
        destination: PathBuf,
    },
    Move {
        source: PathBuf,
        destination: PathBuf,
    },
    Rename {
        source: PathBuf,
        name: String,
    },
    Duplicate(PathBuf),
    Trash(PathBuf),
    Restore(TrashRecord),
}

#[derive(Debug, Clone)]
pub struct FileOutcome {
    pub change: Option<PathChange>,
    pub removed: Option<PathBuf>,
    pub selected: PathBuf,
    pub refreshed: Vec<Result<Directory, String>>,
    pub trash: Result<Option<TrashRecord>, String>,
}

impl FileOperation {
    pub fn run(self, volume: &VolumeStorage) -> Result<FileOutcome, String> {
        let mut parents = BTreeSet::new();
        let (change, removed, selected) = match self {
            Self::Import {
                source,
                destination,
            } => {
                let selected = volume.import_file(&source, &destination)?;
                parents.insert(destination);
                (None, None, selected)
            }
            Self::Move {
                source,
                destination,
            } => {
                let change = volume.move_entry(&source, &destination)?;
                parents.insert(change.from.parent().unwrap().to_owned());
                parents.insert(change.to.parent().unwrap().to_owned());
                let selected = change.to.clone();
                (Some(change), None, selected)
            }
            Self::Rename { source, name } => {
                let change = volume.rename_entry(&source, &name)?;
                parents.insert(change.to.parent().unwrap().to_owned());
                let selected = change.to.clone();
                (Some(change), None, selected)
            }
            Self::Duplicate(source) => {
                let selected = volume.duplicate_entry(&source)?;
                parents.insert(selected.parent().unwrap().to_owned());
                (None, None, selected)
            }
            Self::Trash(source) => {
                let source = volume.document_path(&source)?;
                volume.trash_entry(&source)?;
                let selected = source.parent().unwrap().to_owned();
                parents.insert(selected.clone());
                (None, Some(source), selected)
            }
            Self::Restore(record) => {
                let selected = volume.restore_entry(&record)?;
                parents.insert(selected.parent().unwrap().to_owned());
                (None, None, selected)
            }
        };
        let mut ancestor = selected.parent();
        while let Some(path) = ancestor.filter(|path| path.starts_with(&volume.root)) {
            parents.insert(path.to_owned());
            ancestor = path.parent();
        }
        // Keep successful mutations even if a subsequent directory refresh fails.
        let refreshed = parents
            .iter()
            .map(|parent| Directory::read(parent))
            .collect();
        Ok(FileOutcome {
            change,
            removed,
            selected,
            refreshed,
            trash: volume.latest_trash(),
        })
    }
}
