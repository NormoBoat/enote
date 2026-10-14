use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone)]
pub struct Entry {
    pub path: PathBuf,
    pub name: String,
    pub directory: bool,
}

#[derive(Debug, Clone)]
pub struct Directory {
    pub path: PathBuf,
    pub entries: Vec<Entry>,
}

impl Directory {
    pub fn read(path: &Path) -> Result<Self, String> {
        let path = path.canonicalize().map_err(|e| e.to_string())?;
        let mut entries = Vec::new();
        for entry in fs::read_dir(&path).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            if entry.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            let kind = entry.file_type().map_err(|e| e.to_string())?;
            // Do not follow links into another volume or build cyclic trees.
            if kind.is_symlink() {
                continue;
            }
            if kind.is_dir() || kind.is_file() {
                entries.push(Entry {
                    path: entry.path(),
                    name: entry.file_name().to_string_lossy().into_owned(),
                    directory: kind.is_dir(),
                });
            }
        }
        entries.sort_by_key(|entry| (!entry.directory, entry.name.to_lowercase()));
        Ok(Self { path, entries })
    }
}

#[derive(Debug, Default)]
pub struct Explorer {
    pub root: Option<PathBuf>,
    pub directories: BTreeMap<PathBuf, Vec<Entry>>,
    pub expanded: BTreeSet<PathBuf>,
    pub loading: BTreeSet<PathBuf>,
}

impl Explorer {
    pub fn relocate(&mut self, change: &enote::file_ops::PathChange) {
        let rebase = |path: &Path| {
            path.strip_prefix(&change.from)
                .map(|suffix| change.to.join(suffix))
                .unwrap_or_else(|_| path.to_owned())
        };
        self.directories = std::mem::take(&mut self.directories)
            .into_iter()
            .map(|(path, mut entries)| {
                for entry in &mut entries {
                    entry.path = rebase(&entry.path);
                    entry.name = entry
                        .path
                        .file_name()
                        .unwrap()
                        .to_string_lossy()
                        .into_owned();
                }
                (rebase(&path), entries)
            })
            .collect();
        self.expanded = std::mem::take(&mut self.expanded)
            .into_iter()
            .map(|path| rebase(&path))
            .collect();
        self.loading.clear();
    }

    pub fn remove_subtree(&mut self, path: &Path) {
        self.directories
            .retain(|folder, _| !folder.starts_with(path));
        self.expanded.retain(|folder| !folder.starts_with(path));
        self.loading.clear();
    }
    pub fn set_root(&mut self, directory: Directory) {
        self.directories.clear();
        self.expanded.clear();
        self.loading.clear();
        self.root = Some(directory.path.clone());
        self.expanded.insert(directory.path.clone());
        self.directories.insert(directory.path, directory.entries);
    }

    pub fn visible(&self) -> Vec<(usize, &Entry)> {
        fn visit<'a>(
            explorer: &'a Explorer,
            path: &Path,
            depth: usize,
            output: &mut Vec<(usize, &'a Entry)>,
        ) {
            if !explorer.expanded.contains(path) {
                return;
            }
            if let Some(entries) = explorer.directories.get(path) {
                for entry in entries {
                    output.push((depth, entry));
                    if entry.directory {
                        visit(explorer, &entry.path, depth + 1, output);
                    }
                }
            }
        }
        let mut output = Vec::new();
        if let Some(root) = &self.root {
            visit(self, root, 0, &mut output);
        }
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remaps_expanded_subtree_after_move_without_losing_nested_state() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("A/deep")).unwrap();
        fs::create_dir(dir.path().join("B")).unwrap();
        fs::write(dir.path().join("A/deep/note.mde"), "note").unwrap();
        let root = dir.path().canonicalize().unwrap();
        let mut explorer = Explorer::default();
        explorer.set_root(Directory::read(&root).unwrap());
        for path in [root.join("A"), root.join("A/deep")] {
            let directory = Directory::read(&path).unwrap();
            explorer.expanded.insert(path.clone());
            explorer.directories.insert(path, directory.entries);
        }
        let change = enote::file_ops::PathChange {
            from: root.join("A"),
            to: root.join("B/A"),
        };
        fs::rename(&change.from, &change.to).unwrap();
        explorer.relocate(&change);
        for path in [root.clone(), root.join("B")] {
            let directory = Directory::read(&path).unwrap();
            explorer.expanded.insert(path.clone());
            explorer.directories.insert(path, directory.entries);
        }
        assert!(explorer.expanded.contains(&root.join("B/A/deep")));
        assert_eq!(explorer.visible().last().unwrap().0, 3);
        assert_eq!(
            explorer.visible().last().unwrap().1.path,
            root.join("B/A/deep/note.mde")
        );
        explorer.remove_subtree(&root.join("B/A"));
        assert!(!explorer.directories.contains_key(&root.join("B/A/deep")));
    }

    #[test]
    fn sorts_folders_first_and_expands_only_requested_directories() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("notes")).unwrap();
        fs::write(dir.path().join("notes/child.md"), "child").unwrap();
        fs::write(dir.path().join("a.md"), "a").unwrap();
        fs::write(dir.path().join(".DS_Store"), "hidden").unwrap();
        std::fs::create_dir(dir.path().join(".enote")).unwrap();
        let mut explorer = Explorer::default();
        explorer.set_root(Directory::read(dir.path()).unwrap());
        assert!(explorer.visible()[0].1.directory);
        assert_eq!(explorer.visible().len(), 2);
        let nested = Directory::read(&dir.path().join("notes")).unwrap();
        explorer.expanded.insert(nested.path.clone());
        explorer
            .directories
            .insert(nested.path.clone(), nested.entries);
        assert_eq!(explorer.visible()[1].0, 1);
        assert_eq!(explorer.visible()[1].1.name, "child.md");
        explorer.expanded.remove(&nested.path);
        assert_eq!(explorer.visible().len(), 2);
    }
}
