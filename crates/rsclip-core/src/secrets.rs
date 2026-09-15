use crate::models::{ClipboardEntry, EntryData};

/// Extracts a text-like value from a clipboard entry suitable for storage as a secret.
pub fn secret_value_from_entry(entry: &ClipboardEntry) -> Option<String> {
    match &entry.data {
        EntryData::Image { ocr_text, .. } => ocr_text
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .map(ToOwned::to_owned),
        EntryData::Color { .. } | EntryData::File { .. } => None,
        EntryData::Link { url, .. } => Some(url.clone()),
        EntryData::Text | EntryData::Unknown => entry
            .text_content
            .as_deref()
            .or(entry.preview_text.as_deref())
            .filter(|value| !value.trim().is_empty())
            .map(ToOwned::to_owned),
    }
}

/// Generates a sensible default alias when promoting a clipboard entry to a secret.
pub fn default_secret_alias(entry: &ClipboardEntry) -> String {
    if matches!(
        &entry.data,
        EntryData::Image {
            ocr_text: Some(text),
            ..
        } if !text.trim().is_empty()
    ) {
        "OCR text".to_string()
    } else if entry.title.trim().is_empty() {
        "Untitled secret".to_string()
    } else {
        entry.title.clone()
    }
}

/// Trims an alias and falls back to "Untitled secret" if empty.
pub fn normalize_secret_alias(alias: &str) -> &str {
    let normalized = alias.trim();
    if normalized.is_empty() {
        "Untitled secret"
    } else {
        normalized
    }
}

#[cfg(test)]
mod tests {
    use crate::models::{ClipboardEntry, EntryData};

    use super::{default_secret_alias, normalize_secret_alias, secret_value_from_entry};

    #[test]
    fn extracts_text_like_secret_values() {
        let mut text = ClipboardEntry::test_text(1, "Title");
        text.text_content = Some("secret".to_string());
        assert_eq!(secret_value_from_entry(&text).as_deref(), Some("secret"));

        let link = ClipboardEntry::test_link(1, "https://example.com", "example.com");
        assert_eq!(
            secret_value_from_entry(&link).as_deref(),
            Some("https://example.com")
        );
    }

    #[test]
    fn extracts_image_ocr_but_rejects_non_text_entries() {
        let mut image = ClipboardEntry::test_image(1, "/tmp/test.png");
        assert_eq!(secret_value_from_entry(&image), None);
        image.data = EntryData::Image {
            file_path: String::new(),
            thumb_path: None,
            ocr_text: Some("image text".to_string()),
        };
        assert_eq!(
            secret_value_from_entry(&image).as_deref(),
            Some("image text")
        );

        let color = ClipboardEntry::test_color(1, "#ff0000", "hex");
        assert_eq!(secret_value_from_entry(&color), None);
    }

    #[test]
    fn chooses_default_secret_alias() {
        let mut image = ClipboardEntry::test_image(1, "/tmp/test.png");
        image.data = EntryData::Image {
            file_path: String::new(),
            thumb_path: None,
            ocr_text: Some("image text".to_string()),
        };
        assert_eq!(default_secret_alias(&image), "OCR text");

        let mut untitled = ClipboardEntry::test_text(1, "");
        untitled.title = "  ".to_string();
        assert_eq!(default_secret_alias(&untitled), "Untitled secret");
    }

    #[test]
    fn normalizes_secret_alias() {
        assert_eq!(normalize_secret_alias("  Named  "), "Named");
        assert_eq!(normalize_secret_alias("  "), "Untitled secret");
    }
}
