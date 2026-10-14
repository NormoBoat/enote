use super::{RenderedPage, render_size};
use cosmic_text::{Attrs, Buffer, Color, Family, FontSystem, Metrics, Shaping, SwashCache};
use std::{
    borrow::Cow,
    cmp::Ordering,
    io::{Cursor, Read},
    path::{Component, Path},
};

const ENTRY_LIMIT: u64 = 32 * 1024 * 1024;
const TEXT_LIMIT: usize = 8 * 1024 * 1024;
type Archive<'a> = zip::ZipArchive<Cursor<&'a [u8]>>;

fn entry(archive: &mut Archive<'_>, name: &str) -> Result<Vec<u8>, String> {
    let file = archive.by_name(name).map_err(|e| e.to_string())?;
    if file.size() > ENTRY_LIMIT {
        return Err("Book archive entry exceeds 32 MiB".into());
    }
    let mut bytes = Vec::new();
    file.take(ENTRY_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > ENTRY_LIMIT {
        return Err("Book archive entry exceeds 32 MiB".into());
    }
    Ok(bytes)
}

fn decode_xml(bytes: &[u8]) -> Result<Cow<'_, str>, String> {
    let (encoding, skip) = if let Some(bom) = encoding_rs::Encoding::for_bom(bytes) {
        bom
    } else {
        let prefix = String::from_utf8_lossy(&bytes[..bytes.len().min(512)]);
        let declaration = prefix
            .trim_start()
            .strip_prefix("<?xml")
            .and_then(|s| s.split_once("?>").map(|(s, _)| s));
        let label = declaration
            .and_then(|s| s.split_once("encoding"))
            .and_then(|(_, s)| s.trim_start().strip_prefix('='))
            .map(str::trim_start)
            .and_then(|s| {
                let quote = s.chars().next()?;
                if !matches!(quote, '\'' | '"') {
                    return None;
                }
                s[1..].split_once(quote).map(|(label, _)| label)
            });
        let encoding = match label {
            Some(label) => encoding_rs::Encoding::for_label(label.as_bytes())
                .ok_or("Unsupported book XML encoding")?,
            None => encoding_rs::UTF_8,
        };
        (encoding, 0)
    };
    let (text, errors) = encoding.decode_without_bom_handling(&bytes[skip..]);
    if errors {
        return Err("Invalid book XML encoding".into());
    }
    Ok(text)
}

fn xml(text: &str) -> Result<roxmltree::Document<'_>, String> {
    roxmltree::Document::parse(text).map_err(|e| e.to_string())
}

// Resolve EPUB references inside the ZIP without extracting files to disk.
fn archive_path(base: &str, href: &str) -> Result<String, String> {
    let mut parts: Vec<String> = Path::new(base)
        .parent()
        .unwrap_or(Path::new(""))
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    let href = href.split('#').next().unwrap_or_default();
    for part in Path::new(href).components() {
        match part {
            Component::Normal(name) => parts.push(name.to_string_lossy().into_owned()),
            Component::CurDir => {}
            Component::ParentDir if !parts.is_empty() => {
                parts.pop();
            }
            _ => return Err("Invalid EPUB archive path".into()),
        }
    }
    Ok(parts.join("/"))
}

fn collect_text(node: roxmltree::Node<'_, '_>, out: &mut String) -> Result<(), String> {
    if node.is_text() {
        for ch in node.text().unwrap_or_default().chars() {
            if ch.is_whitespace() {
                if !out.is_empty() && !out.ends_with([' ', '\n']) {
                    out.push(' ');
                }
            } else {
                out.push(ch);
            }
            if out.len() > TEXT_LIMIT {
                return Err("Book text exceeds 8 MiB".into());
            }
        }
    } else {
        let tag = node.tag_name().name();
        if matches!(tag, "script" | "style" | "binary" | "head") {
            return Ok(());
        }
        let block = matches!(
            tag,
            "p" | "div"
                | "section"
                | "title"
                | "subtitle"
                | "h1"
                | "h2"
                | "h3"
                | "h4"
                | "h5"
                | "h6"
                | "li"
                | "tr"
                | "br"
                | "empty-line"
        );
        if block && !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        for child in node.children() {
            collect_text(child, out)?;
        }
        if block && !out.ends_with('\n') {
            out.push('\n');
        }
    }
    if out.len() > TEXT_LIMIT {
        return Err("Book text exceeds 8 MiB".into());
    }
    Ok(())
}

fn book_text(bytes: &[u8], extension: &str) -> Result<String, String> {
    let mut text = String::new();
    if extension.eq_ignore_ascii_case("fb2") {
        let decoded = decode_xml(bytes)?;
        let doc = xml(&decoded)?;
        if doc.root_element().tag_name().name() != "FictionBook" {
            return Err("Invalid FictionBook document".into());
        }
        for body in doc
            .root_element()
            .children()
            .filter(|n| n.has_tag_name("body"))
        {
            collect_text(body, &mut text)?;
        }
    } else {
        let mut archive = Archive::new(Cursor::new(bytes)).map_err(|e| e.to_string())?;
        let container = entry(&mut archive, "META-INF/container.xml")?;
        let container = decode_xml(&container)?;
        let container = xml(&container)?;
        let package_path = container
            .descendants()
            .find(|n| n.has_tag_name("rootfile"))
            .and_then(|n| n.attribute("full-path"))
            .ok_or("EPUB package is missing")?;
        let package = entry(&mut archive, package_path)?;
        let package = decode_xml(&package)?;
        let package = xml(&package)?;
        let manifest = package
            .descendants()
            .find(|n| n.has_tag_name("manifest"))
            .ok_or("EPUB manifest is missing")?;
        let spine = package
            .descendants()
            .find(|n| n.has_tag_name("spine"))
            .ok_or("EPUB reading order is missing")?;
        for item in spine.children().filter(|n| n.has_tag_name("itemref")) {
            if item.attribute("linear") == Some("no") {
                continue;
            }
            let id = item.attribute("idref").ok_or("Invalid EPUB spine item")?;
            let href = manifest
                .children()
                .find(|n| n.has_tag_name("item") && n.attribute("id") == Some(id))
                .and_then(|n| n.attribute("href"))
                .ok_or("EPUB chapter is missing")?;
            let path = archive_path(package_path, href)?;
            let chapter = entry(&mut archive, &path)?;
            let chapter = decode_xml(&chapter)?;
            let chapter = xml(&chapter)?;
            let body = chapter
                .descendants()
                .find(|n| n.has_tag_name("body"))
                .ok_or("EPUB chapter body is missing")?;
            collect_text(body, &mut text)?;
            text.push('\n');
        }
    }
    if text.trim().is_empty() {
        return Err("The book has no readable text".into());
    }
    Ok(text)
}

const LINES: usize = 29;

pub(super) struct PreparedBook {
    fonts: FontSystem,
    buffer: Buffer,
    cache: SwashCache,
    count: u32,
}

impl PreparedBook {
    pub(super) fn new(bytes: &[u8], extension: &str) -> Result<Self, String> {
        let text = book_text(bytes, extension)?;
        let mut fonts = FontSystem::new();
        if fonts.db().faces().next().is_none() {
            return Err("No fonts available for the book reader".into());
        }
        // Pagination uses a fixed logical page, so zoom does not change saved page numbers.
        let mut buffer = Buffer::new(&mut fonts, Metrics::new(16.0, 22.0));
        buffer.set_size(&mut fonts, Some(416.0), None);
        buffer.set_text(
            &mut fonts,
            &text,
            &Attrs::new().family(Family::SansSerif),
            Shaping::Advanced,
            None,
        );
        buffer.shape_until_scroll(&mut fonts, false);
        let count = u32::try_from(buffer.layout_runs().count().div_ceil(LINES))
            .map_err(|_| "Too many book pages")?;
        if count == 0 {
            return Err("The book has no pages".into());
        }
        Ok(Self {
            fonts,
            buffer,
            cache: SwashCache::new(),
            count,
        })
    }

    pub(super) fn render(&mut self, page: u32, pixel_width: f32) -> Result<RenderedPage, String> {
        let page = page.min(self.count - 1);
        let (width, height) = render_size(480.0, 720.0, pixel_width)?;
        let scale = width as f32 / 480.0;
        let mut rgba = vec![255; (width * height * 4) as usize];
        for (line, run) in self
            .buffer
            .layout_runs()
            .skip(page as usize * LINES)
            .take(LINES)
            .enumerate()
        {
            let baseline = 32.0 + line as f32 * 22.0 + (run.line_y - run.line_top);
            for glyph in run.glyphs {
                let glyph = glyph.physical((32.0 * scale, 0.0), scale);
                self.cache.with_pixels(
                    &mut self.fonts,
                    glyph.cache_key,
                    Color::rgb(24, 24, 24),
                    |x, y, color| {
                        let x = glyph.x + x;
                        let y = glyph.y + y + (baseline * scale) as i32;
                        if x < 0 || y < 0 || x >= width as i32 || y >= height as i32 {
                            return;
                        }
                        let i = (y as usize * width as usize + x as usize) * 4;
                        let [r, g, b, a] = color.as_rgba();
                        for (dst, src) in rgba[i..i + 3].iter_mut().zip([r, g, b]) {
                            *dst = ((src as u32 * a as u32 + *dst as u32 * (255 - a as u32)) / 255)
                                as u8;
                        }
                    },
                );
            }
        }
        Ok(RenderedPage {
            page,
            count: self.count,
            width,
            height,
            rgba: rgba.into(),
        })
    }
}

pub(super) fn render_book(
    bytes: &[u8],
    extension: &str,
    page: u32,
    pixel_width: f32,
) -> Result<RenderedPage, String> {
    PreparedBook::new(bytes, extension)?.render(page, pixel_width)
}

// Natural order: page2 precedes page10, even without zero padding.
fn natural_order(a: &str, b: &str) -> Ordering {
    let (mut a, mut b) = (a.as_bytes(), b.as_bytes());
    while !a.is_empty() && !b.is_empty() {
        let order = if a[0].is_ascii_digit() && b[0].is_ascii_digit() {
            let an = a.iter().take_while(|c| c.is_ascii_digit()).count();
            let bn = b.iter().take_while(|c| c.is_ascii_digit()).count();
            let ax = &a[..an];
            let bx = &b[..bn];
            let ax = &ax[ax.iter().take_while(|c| **c == b'0').count()..];
            let bx = &bx[bx.iter().take_while(|c| **c == b'0').count()..];
            let order = ax.len().cmp(&bx.len()).then_with(|| ax.cmp(bx));
            a = &a[an..];
            b = &b[bn..];
            order
        } else {
            let order = a[0].to_ascii_lowercase().cmp(&b[0].to_ascii_lowercase());
            a = &a[1..];
            b = &b[1..];
            order
        };
        if order != Ordering::Equal {
            return order;
        }
    }
    a.len().cmp(&b.len())
}

pub(super) fn render_comic(
    bytes: &[u8],
    page: u32,
    pixel_width: f32,
) -> Result<RenderedPage, String> {
    let mut archive = Archive::new(Cursor::new(bytes)).map_err(|e| e.to_string())?;
    let mut names: Vec<String> = archive
        .file_names()
        .filter(|name| {
            let path = Path::new(name);
            !name.starts_with("__MACOSX/")
                && path.extension().is_some_and(|ext| {
                    matches!(
                        ext.to_string_lossy().to_ascii_lowercase().as_str(),
                        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "tif" | "tiff"
                    )
                })
        })
        .map(str::to_owned)
        .collect();
    names.sort_by(|a, b| natural_order(a, b).then_with(|| a.cmp(b)));
    let count = u32::try_from(names.len()).map_err(|_| "Too many comic pages")?;
    if count == 0 {
        return Err("The comic has no image pages".into());
    }
    let page = page.min(count - 1);
    let bytes = entry(&mut archive, &names[page as usize])?;
    let mut reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    let image = reader.decode().map_err(|e| e.to_string())?;
    let (width, height) = render_size(image.width() as f32, image.height() as f32, pixel_width)?;
    let rgba = image
        .resize_exact(width, height, image::imageops::FilterType::Triangle)
        .into_rgba8()
        .into_raw();
    Ok(RenderedPage {
        page,
        count,
        width,
        height,
        rgba: rgba.into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn archive(files: &[(&str, Vec<u8>)]) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, bytes) in files {
            writer
                .start_file(*name, zip::write::FileOptions::default())
                .unwrap();
            writer.write_all(bytes).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    #[test]
    fn comic_pages_follow_numeric_names_and_clamp_the_position() {
        let png = |color| {
            let image = image::RgbaImage::from_pixel(8, 12, image::Rgba(color));
            let mut bytes = Cursor::new(Vec::new());
            image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
            bytes.into_inner()
        };
        let comic = archive(&[
            ("page10.png", png([0, 0, 255, 255])),
            ("page2.png", png([255, 0, 0, 255])),
            ("__MACOSX/extra.png", vec![0]),
            ("readme.txt", b"not a page".to_vec()),
        ]);
        let first = super::super::render(&comic, "CBZ", 0, 600.0).unwrap();
        assert_eq!(first.count, 2);
        assert_eq!(&first.rgba[..4], &[255, 0, 0, 255]);
        let last = render_comic(&comic, 99, 600.0).unwrap();
        assert_eq!(last.page, 1);
        assert_eq!(&last.rgba[..4], &[0, 0, 255, 255]);
        assert_eq!(last.rgba.len(), (last.width * last.height * 4) as usize);
        assert!(render_comic(b"invalid", 0, 600.0).is_err());
        assert!(render_comic(&archive(&[]), 0, 600.0).is_err());
    }

    #[test]
    fn epub_uses_spine_order_and_ignores_styles_and_non_linear_items() {
        let book = archive(&[
            ("META-INF/container.xml", b"<container><rootfiles><rootfile full-path='OPS/book.opf'/></rootfiles></container>".to_vec()),
            ("OPS/book.opf", b"<package><manifest><item id='b' href='b.xhtml'/><item id='a' href='text/a.xhtml'/></manifest><spine><itemref idref='a'/><itemref idref='b'/><itemref idref='missing' linear='no'/></spine></package>".to_vec()),
            ("OPS/text/a.xhtml", b"<html><head><title>Hidden</title></head><body><style>Hidden CSS</style><p>First <strong>chapter</strong>.</p></body></html>".to_vec()),
            ("OPS/b.xhtml", b"<html><body><p>Second chapter.</p></body></html>".to_vec()),
        ]);
        let text = book_text(&book, "epub").unwrap();
        assert!(text.contains("First chapter."));
        assert!(text.find("First").unwrap() < text.find("Second").unwrap());
        assert!(!text.contains("Hidden"));
        assert_eq!(
            archive_path("OPS/book.opf", "../chapter.xhtml#part").unwrap(),
            "chapter.xhtml"
        );
        assert!(archive_path("OPS/book.opf", "../../outside").is_err());
    }

    #[test]
    fn fictionbook_displays_unicode_text_and_zoom_keeps_page_numbers() {
        let book = format!(
            "<FictionBook><description><p>Hidden metadata</p></description><body><section>{}</section></body><binary>Hidden image</binary></FictionBook>",
            "<p>Привет, мир! Текст книги на Rust.</p>".repeat(100)
        );
        let text = book_text(book.as_bytes(), "fb2").unwrap();
        assert!(text.contains("Привет, мир!"));
        assert!(!text.contains("Hidden"));
        let page = render_book(book.as_bytes(), "fb2", 1, 480.0).unwrap();
        let zoomed = render_book(book.as_bytes(), "fb2", 1, 960.0).unwrap();
        assert_eq!(page.count, zoomed.count);
        assert_eq!(page.page, zoomed.page);
        assert_eq!(zoomed.width, page.width * 2);
        for rendered in [page, zoomed] {
            assert!(rendered.rgba.chunks_exact(4).any(|p| p[0] < 150));
            assert!(rendered.rgba.chunks_exact(4).all(|p| p[3] == 255));
        }
        assert!(render_book(b"<FictionBook><body/></FictionBook>", "fb2", 0, 480.0).is_err());
        assert!(render_book(b"invalid XML", "fb2", 0, 480.0).is_err());
    }

    #[test]
    fn fictionbook_decodes_declared_cyrillic_encoding_and_utf16_bom() {
        let xml = "<?xml version='1.0' encoding='windows-1251'?><FictionBook><body><p>Привет, мир!</p></body></FictionBook>";
        let (bytes, _, errors) = encoding_rs::WINDOWS_1251.encode(xml);
        assert!(!errors);
        assert!(book_text(&bytes, "fb2").unwrap().contains("Привет, мир!"));
        let xml = "<FictionBook><body><p>Привет</p></body></FictionBook>";
        let bytes: Vec<u8> = [0xff, 0xfe]
            .into_iter()
            .chain(xml.encode_utf16().flat_map(u16::to_le_bytes))
            .collect();
        assert!(book_text(&bytes, "fb2").unwrap().contains("Привет"));
        assert!(decode_xml(b"\xffinvalid UTF-8").is_err());
    }
}
