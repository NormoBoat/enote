use std::path::Path;

pub const DEFAULT_NOTE_EXTENSION: &str = "mde";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentFormat {
    Text,
    Markdown,
    EnoteMarkdown,
}

impl DocumentFormat {
    pub fn from_path(path: &Path) -> Self {
        match path
            .extension()
            .unwrap_or_default()
            .to_string_lossy()
            .to_ascii_lowercase()
            .as_str()
        {
            "mde" => Self::EnoteMarkdown,
            "md" | "markdown" => Self::Markdown,
            _ => Self::Text,
        }
    }
}
