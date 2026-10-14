use crate::{
    file_ops::PathChange,
    settings::{read_document, write_document},
    volume::VolumeStorage,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex},
};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ReadingPosition {
    pub page: u32,
    pub zoom: f32,
    pub x: f32,
    pub y: f32,
}

impl Default for ReadingPosition {
    fn default() -> Self {
        Self {
            page: 0,
            zoom: 1.0,
            x: 0.0,
            y: 0.0,
        }
    }
}

impl ReadingPosition {
    fn valid(&self) -> bool {
        (0.5..=3.0).contains(&self.zoom)
            && (0.0..=1.0).contains(&self.x)
            && (0.0..=1.0).contains(&self.y)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SavedPositions {
    version: u32,
    entries: BTreeMap<String, ReadingPosition>,
}

#[derive(Debug)]
struct State {
    root: PathBuf,
    positions: SavedPositions,
    baseline: serde_json::Value,
    dirty: bool,
}

#[derive(Debug, Clone)]
pub struct ReaderStore(Arc<Mutex<State>>);

fn technical_path(root: &Path) -> Result<PathBuf, String> {
    let technical = root.join(".enote");
    let metadata = fs::symlink_metadata(&technical).map_err(|e| e.to_string())?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("Invalid volume technical directory".into());
    }
    let path = technical.join("viewer-state.json");
    if let Ok(metadata) = fs::symlink_metadata(&path)
        && (metadata.file_type().is_symlink() || !metadata.is_file())
    {
        return Err("Invalid viewer state file".into());
    }
    Ok(path)
}

fn key(root: &Path, path: &Path) -> Result<String, String> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| "Document is outside the volume")?;
    let parts = relative
        .components()
        .map(|part| match part {
            Component::Normal(name) if name != ".enote" => name
                .to_str()
                .map(str::to_owned)
                .ok_or("Document names must be Unicode"),
            _ => Err("Invalid document path"),
        })
        .collect::<Result<Vec<_>, _>>()?;
    if parts.is_empty() {
        return Err("Document path is empty".into());
    }
    serde_json::to_string(&parts).map_err(|e| e.to_string())
}

impl ReaderStore {
    pub fn load(volume: &VolumeStorage) -> Result<Self, String> {
        let path = technical_path(&volume.root)?;
        let baseline = read_document(&path).map_err(|e| e.to_string())?;
        let positions = if baseline.as_object().is_some_and(|object| object.is_empty()) {
            SavedPositions {
                version: 1,
                entries: BTreeMap::new(),
            }
        } else {
            serde_json::from_value::<SavedPositions>(baseline.clone()).map_err(|e| e.to_string())?
        };
        if positions.version != 1 || positions.entries.values().any(|position| !position.valid()) {
            return Err("Unsupported or invalid viewer state".into());
        }
        for entry in positions.entries.keys() {
            let parts: Vec<String> = serde_json::from_str(entry).map_err(|e| e.to_string())?;
            let mut path = volume.root.clone();
            for part in parts {
                VolumeStorage::validate_name(&part)?;
                path.push(part);
            }
            key(&volume.root, &path)?;
        }
        Ok(Self(Arc::new(Mutex::new(State {
            root: volume.root.clone(),
            positions,
            baseline,
            dirty: false,
        }))))
    }

    pub fn position(&self, path: &Path) -> Result<ReadingPosition, String> {
        let state = self.0.lock().map_err(|_| "Viewer state lock poisoned")?;
        Ok(state
            .positions
            .entries
            .get(&key(&state.root, path)?)
            .copied()
            .unwrap_or_default())
    }

    pub fn update(&self, path: &Path, position: ReadingPosition) -> Result<(), String> {
        if !position.valid() {
            return Err("Invalid reading position".into());
        }
        let mut state = self.0.lock().map_err(|_| "Viewer state lock poisoned")?;
        let key = key(&state.root, path)?;
        if state.positions.entries.get(&key) != Some(&position) {
            state.positions.entries.insert(key, position);
            state.dirty = true;
        }
        Ok(())
    }

    pub fn relocate(&self, change: &PathChange) -> Result<(), String> {
        let mut state = self.0.lock().map_err(|_| "Viewer state lock poisoned")?;
        let mut relocated = BTreeMap::new();
        for (entry, position) in &state.positions.entries {
            let parts: Vec<String> = serde_json::from_str(entry).map_err(|e| e.to_string())?;
            let path = parts
                .iter()
                .fold(state.root.clone(), |path, part| path.join(part));
            let new_key = if let Ok(suffix) = path.strip_prefix(&change.from) {
                key(&state.root, &change.to.join(suffix))?
            } else {
                entry.clone()
            };
            relocated.insert(new_key, *position);
        }
        if relocated != state.positions.entries {
            state.positions.entries = relocated;
            state.dirty = true;
        }
        Ok(())
    }

    pub fn flush(&self) -> Result<(), String> {
        let mut state = self.0.lock().map_err(|_| "Viewer state lock poisoned")?;
        if !state.dirty {
            return Ok(());
        }
        let path = technical_path(&state.root)?;
        if read_document(&path).map_err(|e| e.to_string())? != state.baseline {
            return Err("Viewer state changed outside eNote; not overwriting it".into());
        }
        let document = serde_json::to_value(&state.positions).map_err(|e| e.to_string())?;
        write_document(&path, &document, true).map_err(|e| e.to_string())?;
        state.baseline = document;
        state.dirty = false;
        Ok(())
    }
}

pub fn set_remember_position(volume: &VolumeStorage, enabled: bool) -> Result<(), String> {
    let path = volume.settings_path();
    technical_path(&volume.root)?;
    if fs::symlink_metadata(&path)
        .map_err(|e| e.to_string())?
        .file_type()
        .is_symlink()
    {
        return Err("Linked volume settings are not supported".into());
    }
    let mut settings = read_document(&path).map_err(|e| e.to_string())?;
    let viewer = settings
        .as_object_mut()
        .unwrap()
        .entry("viewer")
        .or_insert_with(|| serde_json::json!({}));
    viewer
        .as_object_mut()
        .ok_or("Invalid viewer settings")?
        .insert("remember_position".into(), enabled.into());
    write_document(&path, &settings, true).map_err(|e| e.to_string())
}

#[derive(Debug, Clone)]
pub struct RenderedPage {
    pub page: u32,
    pub count: u32,
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<[u8]>,
}

pub fn render(
    bytes: &[u8],
    extension: &str,
    page: u32,
    pixel_width: f32,
) -> Result<RenderedPage, String> {
    if !pixel_width.is_finite() {
        return Err("Invalid render size".into());
    }
    match extension.to_ascii_lowercase().as_str() {
        "pdf" => render_pdf(bytes, page, pixel_width),
        "cbz" => book::render_comic(bytes, page, pixel_width),
        "epub" | "fb2" => book::render_book(bytes, extension, page, pixel_width),
        _ => Err(format!("Unsupported document format: {extension}")),
    }
}

mod book;
mod worker;
pub use worker::RenderWorker;

// Bound both the area and individual dimensions before allocating a bitmap.
fn render_size(width: f32, height: f32, pixel_width: f32) -> Result<(u32, u32), String> {
    if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
        return Err("Invalid page dimensions".into());
    }
    let scale = (pixel_width.clamp(256.0, 2400.0) / width)
        .min((12_000_000.0 / (width as f64 * height as f64)).sqrt() as f32)
        .min(65_000.0 / width.max(height));
    let w = (width * scale).floor() as u32;
    let h = (height * scale).floor() as u32;
    if w == 0 || h == 0 {
        return Err("Invalid rendered page dimensions".into());
    }
    Ok((w, h))
}

fn render_pdf(bytes: &[u8], page: u32, pixel_width: f32) -> Result<RenderedPage, String> {
    use hayro::{RenderCache, hayro_syntax::Pdf};
    let document = Pdf::new(bytes.to_vec()).map_err(|e| format!("Cannot open PDF: {e:?}"))?;
    render_pdf_page(&document, &RenderCache::new(), page, pixel_width)
}

fn render_pdf_page<'a>(
    document: &'a hayro::hayro_syntax::Pdf,
    cache: &hayro::RenderCache<'a>,
    page: u32,
    pixel_width: f32,
) -> Result<RenderedPage, String> {
    use hayro::{PixmapSettings, RenderSettings};
    let pages = document.pages();
    let count = u32::try_from(pages.len()).map_err(|_| "Too many document pages")?;
    if count == 0 {
        return Err("The document has no pages".into());
    }
    let index = page.min(count - 1);
    let page = &pages[index as usize];
    let (width, height) = page.render_dimensions();
    let (w, h) = render_size(width, height, pixel_width)?;
    let pixmap = hayro::render(
        page,
        cache,
        &Default::default(),
        &RenderSettings::default(),
        &PixmapSettings {
            x_scale: w as f32 / width,
            y_scale: h as f32 / height,
            bg_color: hayro::vello_cpu::color::palette::css::WHITE,
        },
    );
    Ok(RenderedPage {
        page: index,
        count,
        width: pixmap.width() as u32,
        height: pixmap.height() as u32,
        rgba: pixmap.data_as_u8_slice().into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const PDF: &[u8] = include_bytes!("../tests/fixtures/reader.pdf");

    #[test]
    fn pdf_renders_three_pages_and_clamps_old_positions() {
        let rendered = render(PDF, "pdf", 99, 700.0).unwrap();
        assert_eq!(rendered.count, 3);
        assert_eq!(rendered.page, 2);
        assert_eq!(
            rendered.rgba.len(),
            (rendered.width * rendered.height * 4) as usize
        );
        assert!(
            rendered
                .rgba
                .chunks_exact(4)
                .any(|pixel| pixel[0] < 150 && pixel[1] < 150)
        );
        assert!(render(b"broken document", "pdf", 0, 700.0).is_err());
        assert!(render(PDF, "pdf", 0, f32::NAN).is_err());
    }

    fn epub() -> Vec<u8> {
        let mut archive = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let files = [
            ("mimetype", "application/epub+zip".to_string()),
            ("META-INF/container.xml", "<?xml version=\"1.0\"?><container version=\"1.0\" xmlns=\"urn:oasis:names:tc:opendocument:xmlns:container\"><rootfiles><rootfile full-path=\"book.opf\" media-type=\"application/oebps-package+xml\"/></rootfiles></container>".into()),
            ("book.opf", "<?xml version=\"1.0\"?><package version=\"2.0\" unique-identifier=\"id\" xmlns=\"http://www.idpf.org/2007/opf\"><metadata xmlns:dc=\"http://purl.org/dc/elements/1.1/\"><dc:title>Reader Test</dc:title><dc:identifier id=\"id\">enote-reader-test</dc:identifier><dc:language>en</dc:language></metadata><manifest><item id=\"chapter\" href=\"chapter.xhtml\" media-type=\"application/xhtml+xml\"/></manifest><spine><itemref idref=\"chapter\"/></spine></package>".into()),
            ("chapter.xhtml", format!("<html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title>Book</title></head><body><h1>EPUB Reader Test</h1>{}</body></html>", "<p>This electronic book is rendered by Rust inside an eNote buffer.</p>".repeat(200))),
        ];
        for (name, content) in files {
            archive
                .start_file(name, zip::write::FileOptions::default())
                .unwrap();
            archive.write_all(content.as_bytes()).unwrap();
        }
        archive.finish().unwrap().into_inner()
    }

    #[test]
    fn epub_and_fictionbook_are_paginated_by_rust() {
        let book = epub();
        let page = render(&book, "epub", 1, 600.0).unwrap();
        assert!(page.count > 2);
        assert_eq!(page.page, 1);
        let book = format!(
            "<?xml version=\"1.0\"?><FictionBook xmlns=\"http://www.gribuser.ru/xml/fictionbook/2.0\"><description><title-info><book-title>Reader Test</book-title><lang>en</lang></title-info></description><body><section><title><p>FictionBook Test</p></title>{}</section></body></FictionBook>",
            "<p>A paragraph in a FictionBook document, displayed in an eNote reader buffer.</p>"
                .repeat(200)
        );
        assert!(render(book.as_bytes(), "fb2", 1, 600.0).unwrap().count > 2);
    }

    #[test]
    fn remembers_page_zoom_and_scroll_after_restart_and_move() {
        let dir = tempfile::tempdir().unwrap();
        let volume = VolumeStorage::open(dir.path()).unwrap();
        fs::create_dir(volume.root.join("Books")).unwrap();
        let path = volume.root.join("Books/book.pdf");
        fs::write(&path, PDF).unwrap();
        let store = ReaderStore::load(&volume).unwrap();
        let position = ReadingPosition {
            page: 2,
            zoom: 1.5,
            x: 0.3,
            y: 0.8,
        };
        store.update(&path, position).unwrap();
        store.flush().unwrap();
        let store = ReaderStore::load(&volume).unwrap();
        assert_eq!(store.position(&path).unwrap(), position);
        let change = volume
            .move_entry(&volume.root.join("Books"), &volume.root)
            .unwrap_err();
        assert!(change.contains("already"));
        let change = volume
            .rename_entry(&volume.root.join("Books"), "Library")
            .unwrap();
        store.relocate(&change).unwrap();
        store.flush().unwrap();
        let store = ReaderStore::load(&volume).unwrap();
        assert_eq!(
            store
                .position(&volume.root.join("Library/book.pdf"))
                .unwrap(),
            position
        );
        let state = fs::read_to_string(volume.root.join(".enote/viewer-state.json")).unwrap();
        assert!(!state.contains(volume.root.to_str().unwrap()));
        assert_eq!(store.position(&path).unwrap(), ReadingPosition::default());
    }

    #[test]
    fn corrupt_or_externally_modified_state_is_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let volume = VolumeStorage::open(dir.path()).unwrap();
        let state = volume.root.join(".enote/viewer-state.json");
        let store = ReaderStore::load(&volume).unwrap();
        store
            .update(&volume.root.join("book.pdf"), ReadingPosition::default())
            .unwrap();
        fs::write(&state, "broken").unwrap();
        assert!(store.flush().is_err());
        assert!(ReaderStore::load(&volume).is_err());
        assert_eq!(fs::read_to_string(state).unwrap(), "broken");
        assert!(
            store
                .update(
                    &volume.root.join(".enote/secret.pdf"),
                    ReadingPosition::default()
                )
                .is_err()
        );
        assert!(
            store
                .update(
                    &volume.root.join("book.pdf"),
                    ReadingPosition {
                        zoom: f32::NAN,
                        ..Default::default()
                    }
                )
                .is_err()
        );
    }

    #[test]
    fn remember_preference_preserves_other_volume_settings() {
        let dir = tempfile::tempdir().unwrap();
        let volume = VolumeStorage::open(dir.path()).unwrap();
        fs::write(volume.settings_path(), "{\"plugin\":{\"enabled\":true}}").unwrap();
        set_remember_position(&volume, false).unwrap();
        let settings = read_document(&volume.settings_path()).unwrap();
        assert_eq!(settings["plugin"]["enabled"], true);
        assert_eq!(settings["viewer"]["remember_position"], false);
    }
}
