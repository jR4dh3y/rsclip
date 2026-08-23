use anyhow::Result;

use crate::colors::{parse_color, rgb_text};
use crate::files::{normalize_uri_list, parse_uri_list, uri_list_preview, uri_list_title};
use crate::format::human_bytes;
use crate::links::detect_single_url;
use crate::mime::kind_from_mime;
use crate::models::{EntryKind, NewEntry, NewEntryData};

pub fn classify_payload(mime_type: &str, content_hash: String, payload: &[u8]) -> Result<NewEntry> {
    let size_bytes = i64::try_from(payload.len()).unwrap_or(i64::MAX);
    if mime_type == "text/uri-list" {
        let text = String::from_utf8_lossy(payload).to_string();
        return Ok(
            classify_uri_list(content_hash.clone(), text.clone(), size_bytes)
                .unwrap_or_else(|| classify_text(mime_type, content_hash, text, size_bytes)),
        );
    }
    if mime_type == "x-special/gnome-copied-files" {
        let text = String::from_utf8_lossy(payload).to_string();
        return Ok(classify_gnome_copied_files(content_hash.clone(), &text)
            .unwrap_or_else(|| classify_text(mime_type, content_hash, text, size_bytes)));
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

fn classify_uri_list(
    content_hash: String,
    text: String,
    fallback_size_bytes: i64,
) -> Option<NewEntry> {
    if parse_uri_list(&text).is_empty() {
        return None;
    }

    let normalized = normalize_uri_list(&text);
    let mut entry = NewEntry::new(
        content_hash,
        "text/uri-list".to_string(),
        uri_list_title(&normalized),
    );
    entry.preview_text = Some(uri_list_preview(&normalized));
    entry.text_content = Some(normalized.clone());
    entry.size_bytes = i64::try_from(normalized.len()).unwrap_or(fallback_size_bytes);
    entry.data = NewEntryData::File { source_app: None };
    Some(entry)
}

fn classify_gnome_copied_files(content_hash: String, text: &str) -> Option<NewEntry> {
    let uri_list = strip_gnome_copy_action(text);
    classify_uri_list(
        content_hash,
        uri_list.to_string(),
        i64::try_from(text.len()).unwrap_or(i64::MAX),
    )
}

fn strip_gnome_copy_action(text: &str) -> &str {
    let Some((first, rest)) = text.split_once('\n') else {
        return text;
    };

    if matches!(first.trim_end_matches('\r'), "copy" | "cut") {
        rest
    } else {
        text
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
    const MAX_CHARS: usize = 320;

    let mut preview = String::with_capacity(MAX_CHARS);
    let mut normalized_chars = 0;
    let mut pending_space = false;

    for character in text.chars() {
        if character.is_whitespace() {
            if !preview.is_empty() {
                pending_space = true;
            }
            continue;
        }

        if pending_space {
            normalized_chars += 1;
            if normalized_chars > MAX_CHARS {
                return truncated_preview(&preview);
            }
            preview.push(' ');
            pending_space = false;
        }

        normalized_chars += 1;
        if normalized_chars > MAX_CHARS {
            return truncated_preview(&preview);
        }
        preview.push(character);
    }

    preview
}

fn truncated_preview(value: &str) -> String {
    const PREFIX_CHARS: usize = 317;
    let prefix = value
        .char_indices()
        .nth(PREFIX_CHARS)
        .map_or(value, |(index, _)| &value[..index]);
    format!("{prefix}...")
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
            classify_payload("text/uri-list", "hash".to_string(), b"file:///tmp/a.txt\n").unwrap();

        assert!(matches!(entry.data, NewEntryData::File { .. }));
        assert_eq!(entry.mime_type, "text/uri-list");
        assert_eq!(entry.title, "a.txt");
        assert_eq!(entry.preview_text.as_deref(), Some("/tmp/a.txt"));
        assert_eq!(entry.text_content.as_deref(), Some("file:///tmp/a.txt\r\n"));
        assert_eq!(entry.size_bytes, "file:///tmp/a.txt\r\n".len() as i64);
    }

    #[test]
    fn classifies_gnome_copy_payload_as_file() {
        let entry = classify_payload(
            "x-special/gnome-copied-files",
            "hash".to_string(),
            b"copy\nfile:///tmp/a.txt\n",
        )
        .unwrap();

        assert!(matches!(entry.data, NewEntryData::File { .. }));
        assert_eq!(entry.mime_type, "text/uri-list");
        assert_eq!(entry.text_content.as_deref(), Some("file:///tmp/a.txt\r\n"));
    }

    #[test]
    fn classifies_gnome_cut_payload_as_copyable_file() {
        let entry = classify_payload(
            "x-special/gnome-copied-files",
            "hash".to_string(),
            b"cut\nfile:///tmp/a.txt\n",
        )
        .unwrap();

        assert!(matches!(entry.data, NewEntryData::File { .. }));
        assert_eq!(entry.mime_type, "text/uri-list");
        assert_eq!(entry.text_content.as_deref(), Some("file:///tmp/a.txt\r\n"));
    }

    #[test]
    fn invalid_uri_list_falls_back_to_text_classification() {
        let entry = classify_payload("text/uri-list", "hash".to_string(), b"not a uri").unwrap();

        assert!(matches!(entry.data, NewEntryData::Text));
        assert_eq!(entry.mime_type, "text/uri-list");
        assert_eq!(entry.text_content.as_deref(), Some("not a uri"));
    }

    #[test]
    fn bounds_preview_for_large_multiline_text() {
        let text = (0..10_000)
            .map(|line| format!("line {line}\n"))
            .collect::<String>();

        let preview = preview_text(&text);

        assert_eq!(preview.chars().count(), 320);
        assert!(preview.ends_with("..."));
        assert!(preview.starts_with("line 0 line 1 line 2"));
        assert!(!preview.contains("line 9999"));
    }

    #[test]
    fn truncates_at_unicode_character_boundary() {
        let text = format!("{}🙂 trailing", "a".repeat(316));

        assert_eq!(preview_text(&text), format!("{}🙂...", "a".repeat(316)));
    }

    #[test]
    fn preview_normalizes_unicode_whitespace() {
        assert_eq!(preview_text("\u{2003}one\n\t two\u{00a0}"), "one two");
    }
}
