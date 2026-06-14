use anyhow::Result;

use crate::colors::{parse_color, rgb_text};
use crate::format::human_bytes;
use crate::links::detect_single_url;
use crate::mime::kind_from_mime;
use crate::models::{EntryKind, NewEntry, NewEntryData};

pub fn classify_payload(mime_type: &str, content_hash: String, payload: &[u8]) -> Result<NewEntry> {
    let size_bytes = i64::try_from(payload.len()).unwrap_or(i64::MAX);
    if mime_type == "text/uri-list" {
        let text = String::from_utf8_lossy(payload).to_string();
        let mut entry = NewEntry::new(content_hash, mime_type.to_string(), first_line_title(&text));
        entry.preview_text = Some(preview_text(&text));
        entry.text_content = Some(text);
        entry.size_bytes = size_bytes;
        entry.data = NewEntryData::File { source_app: None };
        return Ok(entry);
    }
    if mime_type.starts_with("text/") {
        let text = String::from_utf8_lossy(payload).to_string();
        Ok(classify_text(mime_type, content_hash, text, size_bytes))
    } else {
        let kind = kind_from_mime(mime_type);
        let data = match kind {
            EntryKind::Image => NewEntryData::Image {
                file_path: None,
                thumb_path: None,
                ocr_text: None,
            },
            EntryKind::File => NewEntryData::File { source_app: None },
            _ => NewEntryData::default(),
        };
        let mut entry = NewEntry::new(
            content_hash,
            mime_type.to_string(),
            title_for_binary(mime_type, size_bytes),
        );
        entry.size_bytes = size_bytes;
        entry.data = data;
        Ok(entry)
    }
}

pub fn classify_text(
    mime_type: &str,
    content_hash: String,
    text: String,
    size_bytes: i64,
) -> NewEntry {
    let trimmed = text.trim();

    if let Some(color) = parse_color(trimmed) {
        let mut entry = NewEntry::new(
            content_hash,
            mime_type.to_string(),
            color.normalized_hex.clone(),
        );
        entry.preview_text = Some(format!("{}  {}", color.normalized_hex, rgb_text(color.rgb)));
        entry.text_content = Some(text);
        entry.size_bytes = size_bytes;
        entry.data = NewEntryData::Color {
            value: color.normalized_hex,
            format: color.original_format,
        };
        return entry;
    }

    if let Some(link) = detect_single_url(trimmed) {
        let mut entry = NewEntry::new(content_hash, mime_type.to_string(), link.domain.clone());
        entry.preview_text = Some(link.url.clone());
        entry.text_content = Some(text);
        entry.size_bytes = size_bytes;
        entry.data = NewEntryData::Link {
            url: link.url,
            domain: link.domain,
            icon: link.icon,
        };
        return entry;
    }

    let mut entry = NewEntry::new(
        content_hash,
        mime_type.to_string(),
        first_line_title(trimmed),
    );
    entry.preview_text = Some(preview_text(trimmed));
    entry.text_content = Some(text);
    entry.size_bytes = size_bytes;
    entry
}

fn first_line_title(text: &str) -> String {
    let title = text.lines().next().unwrap_or("").trim();
    let title = if title.is_empty() {
        "Untitled text"
    } else {
        title
    };
    truncate(title, 96)
}

fn preview_text(text: &str) -> String {
    truncate(&text.split_whitespace().collect::<Vec<_>>().join(" "), 320)
}

fn truncate(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        value.to_string()
    } else {
        format!(
            "{}...",
            value
                .chars()
                .take(max_chars.saturating_sub(3))
                .collect::<String>()
        )
    }
}

fn title_for_binary(mime_type: &str, size_bytes: i64) -> String {
    if mime_type.starts_with("image/") {
        format!("Image ({})", human_bytes(size_bytes))
    } else {
        format!("{mime_type} ({})", human_bytes(size_bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_link() {
        let entry =
            classify_payload("text/plain", "hash".to_string(), b"https://youtu.be/abc").unwrap();
        assert!(matches!(entry.data, NewEntryData::Link { .. }));
        if let NewEntryData::Link { icon, .. } = entry.data {
            assert_eq!(icon, "youtube");
        }
    }

    #[test]
    fn classifies_bare_domain_as_text() {
        let entry = classify_payload(
            "text/plain",
            "hash".to_string(),
            b"fastblobstorage.vercel.app",
        )
        .unwrap();
        assert!(matches!(entry.data, NewEntryData::Text));
        assert_eq!(entry.title, "fastblobstorage.vercel.app");
    }

    #[test]
    fn classifies_token_shaped_https_value_as_text() {
        let entry = classify_payload(
            "text/plain",
            "hash".to_string(),
            b"https://fbsa_1f3a8e0389a8c0cbc656fca80307e478.fbs-admin-token-2026",
        )
        .unwrap();
        assert!(matches!(entry.data, NewEntryData::Text));
    }

    #[test]
    fn classifies_color() {
        let entry = classify_payload("text/plain", "hash".to_string(), b"#c59edc").unwrap();
        assert!(matches!(entry.data, NewEntryData::Color { .. }));
        if let NewEntryData::Color { value, .. } = entry.data {
            assert_eq!(value, "#c59edc");
        }
    }

    #[test]
    fn classifies_uri_list_as_file() {
        let entry =
            classify_payload("text/uri-list", "hash".to_string(), b"file:///tmp/a.txt").unwrap();

        assert!(matches!(entry.data, NewEntryData::File { .. }));
    }
}
