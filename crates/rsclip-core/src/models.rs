use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum EntryKind {
    Text,
    Image,
    Link,
    Color,
    File,
    Unknown,
}

impl EntryKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Image => "image",
            Self::Link => "link",
            Self::Color => "color",
            Self::File => "file",
            Self::Unknown => "unknown",
        }
    }
}

impl fmt::Display for EntryKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for EntryKind {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Ok(match value {
            "text" => Self::Text,
            "image" => Self::Image,
            "link" => Self::Link,
            "color" => Self::Color,
            "file" => Self::File,
            _ => Self::Unknown,
        })
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub enum EntryData {
    #[default]
    Text,
    Image {
        file_path: String,
        thumb_path: Option<String>,
        ocr_text: Option<String>,
    },
    Link {
        url: String,
        domain: String,
        icon: String,
    },
    Color {
        value: String,
        format: String,
    },
    File {
        source_app: Option<String>,
    },
    Unknown,
}

impl EntryData {
    pub fn kind(&self) -> EntryKind {
        match self {
            Self::Text => EntryKind::Text,
            Self::Image { .. } => EntryKind::Image,
            Self::Link { .. } => EntryKind::Link,
            Self::Color { .. } => EntryKind::Color,
            Self::File { .. } => EntryKind::File,
            Self::Unknown => EntryKind::Unknown,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub enum NewEntryData {
    #[default]
    Text,
    Image {
        file_path: Option<String>,
        thumb_path: Option<String>,
        ocr_text: Option<String>,
    },
    Link {
        url: String,
        domain: String,
        icon: String,
    },
    Color {
        value: String,
        format: String,
    },
    File {
        source_app: Option<String>,
    },
    Unknown,
}

impl NewEntryData {
    pub fn kind(&self) -> EntryKind {
        match self {
            Self::Text => EntryKind::Text,
            Self::Image { .. } => EntryKind::Image,
            Self::Link { .. } => EntryKind::Link,
            Self::Color { .. } => EntryKind::Color,
            Self::File { .. } => EntryKind::File,
            Self::Unknown => EntryKind::Unknown,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ClipboardEntry {
    pub id: i64,
    pub content_hash: String,
    pub kind: EntryKind,
    pub mime_type: String,
    pub title: String,
    pub preview_text: Option<String>,
    pub text_content: Option<String>,
    pub pinned: bool,
    pub copied_at: i64,
    pub updated_at: i64,
    pub last_used_at: Option<i64>,
    pub use_count: i64,
    pub size_bytes: i64,
    pub data: EntryData,
}

impl ClipboardEntry {
    #[cfg(test)]
    pub fn test_text(id: i64, title: &str) -> Self {
        Self {
            id,
            content_hash: "hash".to_string(),
            kind: EntryKind::Text,
            mime_type: "text/plain".to_string(),
            title: title.to_string(),
            preview_text: None,
            text_content: None,
            pinned: false,
            copied_at: 0,
            updated_at: 0,
            last_used_at: None,
            use_count: 0,
            size_bytes: 0,
            data: EntryData::Text,
        }
    }

    #[cfg(test)]
    pub fn test_image(id: i64, file_path: &str) -> Self {
        Self {
            id,
            content_hash: "hash".to_string(),
            kind: EntryKind::Image,
            mime_type: "image/png".to_string(),
            title: "Image".to_string(),
            preview_text: None,
            text_content: None,
            pinned: false,
            copied_at: 0,
            updated_at: 0,
            last_used_at: None,
            use_count: 0,
            size_bytes: 0,
            data: EntryData::Image {
                file_path: file_path.to_string(),
                thumb_path: None,
                ocr_text: None,
            },
        }
    }

    #[cfg(test)]
    pub fn test_link(id: i64, url: &str, domain: &str) -> Self {
        Self {
            id,
            content_hash: "hash".to_string(),
            kind: EntryKind::Link,
            mime_type: "text/plain".to_string(),
            title: domain.to_string(),
            preview_text: None,
            text_content: None,
            pinned: false,
            copied_at: 0,
            updated_at: 0,
            last_used_at: None,
            use_count: 0,
            size_bytes: 0,
            data: EntryData::Link {
                url: url.to_string(),
                domain: domain.to_string(),
                icon: "globe".to_string(),
            },
        }
    }

    #[cfg(test)]
    pub fn test_color(id: i64, value: &str, format: &str) -> Self {
        Self {
            id,
            content_hash: "hash".to_string(),
            kind: EntryKind::Color,
            mime_type: "text/plain".to_string(),
            title: value.to_string(),
            preview_text: None,
            text_content: None,
            pinned: false,
            copied_at: 0,
            updated_at: 0,
            last_used_at: None,
            use_count: 0,
            size_bytes: 0,
            data: EntryData::Color {
                value: value.to_string(),
                format: format.to_string(),
            },
        }
    }

    #[cfg(test)]
    pub fn test_file(id: i64, title: &str, uri_list: Option<&str>) -> Self {
        Self {
            id,
            content_hash: "hash".to_string(),
            kind: EntryKind::File,
            mime_type: "text/uri-list".to_string(),
            title: title.to_string(),
            preview_text: None,
            text_content: uri_list.map(str::to_string),
            pinned: false,
            copied_at: 0,
            updated_at: 0,
            last_used_at: None,
            use_count: 0,
            size_bytes: uri_list.map(str::len).unwrap_or_default() as i64,
            data: EntryData::File { source_app: None },
        }
    }
}

#[derive(Clone, Debug)]
pub struct NewEntry {
    pub content_hash: String,
    pub mime_type: String,
    pub title: String,
    pub preview_text: Option<String>,
    pub text_content: Option<String>,
    pub size_bytes: i64,
    pub data: NewEntryData,
}

impl NewEntry {
    pub fn new(content_hash: String, mime_type: String, title: String) -> Self {
        Self {
            content_hash,
            mime_type,
            title,
            preview_text: None,
            text_content: None,
            size_bytes: 0,
            data: NewEntryData::default(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SecretEntry {
    pub id: i64,
    pub source_entry_id: Option<i64>,
    pub alias: String,
    pub value: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub last_used_at: Option<i64>,
    pub use_count: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EntryFilter {
    All,
    Text,
    Images,
    Files,
    Links,
    Colors,
    Pinned,
}

impl EntryFilter {
    pub fn parse(value: &str) -> Self {
        match value {
            "text" => Self::Text,
            "images" | "image" => Self::Images,
            "files" | "file" => Self::Files,
            "links" | "link" => Self::Links,
            "colors" | "color" => Self::Colors,
            "pinned" => Self::Pinned,
            _ => Self::All,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::EntryFilter;

    #[test]
    fn parses_file_filter_aliases() {
        assert_eq!(EntryFilter::parse("files"), EntryFilter::Files);
        assert_eq!(EntryFilter::parse("file"), EntryFilter::Files);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SortMode {
    Default,
    Recent,
    Oldest,
    Type,
    MostUsed,
}

impl SortMode {
    pub fn parse(value: &str) -> Self {
        match value {
            "recent" | "newest" => Self::Recent,
            "oldest" => Self::Oldest,
            "type" => Self::Type,
            "most-used" => Self::MostUsed,
            _ => Self::Default,
        }
    }
}
