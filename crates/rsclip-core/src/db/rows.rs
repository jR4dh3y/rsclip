use std::str::FromStr;

use rusqlite::Row;

use crate::models::{ClipboardEntry, EntryData, EntryKind, SecretEntry};

pub(super) fn entry_from_row(row: &Row<'_>) -> rusqlite::Result<ClipboardEntry> {
    let kind: String = row.get("kind")?;
    let kind = EntryKind::from_str(&kind).unwrap_or(EntryKind::Unknown);

    let (kind, data) = match kind {
        EntryKind::Text => (EntryKind::Text, EntryData::Text),
        EntryKind::Image => {
            let file_path: Option<String> = row.get("file_path")?;

            match file_path {
                Some(file_path) => (
                    EntryKind::Image,
                    EntryData::Image {
                        file_path,
                        thumb_path: row.get("thumb_path")?,
                        ocr_text: row.get("ocr_text")?,
                    },
                ),
                None => (EntryKind::Unknown, EntryData::Unknown),
            }
        }
        EntryKind::Link => {
            let url: Option<String> = row.get("link_url")?;
            let domain: Option<String> = row.get("link_domain")?;
            let icon: Option<String> = row.get("link_icon")?;

            match (url, domain) {
                (Some(url), Some(domain)) => (
                    EntryKind::Link,
                    EntryData::Link {
                        url,
                        domain,
                        icon: icon.unwrap_or_else(|| "globe".to_string()),
                    },
                ),
                _ => (EntryKind::Unknown, EntryData::Unknown),
            }
        }
        EntryKind::Color => {
            let value: Option<String> = row.get("color_value")?;
            let format: Option<String> = row.get("color_format")?;

            match (value, format) {
                (Some(value), Some(format)) => {
                    (EntryKind::Color, EntryData::Color { value, format })
                }
                _ => (EntryKind::Unknown, EntryData::Unknown),
            }
        }
        EntryKind::File => (
            EntryKind::File,
            EntryData::File {
                source_app: row.get("source_app")?,
            },
        ),
        EntryKind::Unknown => (EntryKind::Unknown, EntryData::Unknown),
    };

    Ok(ClipboardEntry {
        id: row.get("id")?,
        content_hash: row.get("content_hash")?,
        kind,
        mime_type: row.get("mime_type")?,
        title: row.get("title")?,
        preview_text: row.get("preview_text")?,
        text_content: row.get("text_content")?,
        pinned: row.get::<_, i64>("pinned")? != 0,
        copied_at: row.get("copied_at")?,
        updated_at: row.get("updated_at")?,
        last_used_at: row.get("last_used_at")?,
        use_count: row.get("use_count")?,
        size_bytes: row.get("size_bytes")?,
        data,
    })
}

pub(super) fn secret_from_row(row: &Row<'_>) -> rusqlite::Result<SecretEntry> {
    Ok(SecretEntry {
        id: row.get("id")?,
        source_entry_id: row.get("source_entry_id")?,
        alias: row.get("alias")?,
        value: row.get("value")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
        last_used_at: row.get("last_used_at")?,
        use_count: row.get("use_count")?,
    })
}
