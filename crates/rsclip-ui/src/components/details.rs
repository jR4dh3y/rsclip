use gtk::prelude::*;
use gtk4 as gtk;
use rsclip_core::format::{format_full_time, human_size};
use rsclip_core::models::{ClipboardEntry, EntryData, SecretEntry};

use crate::components::labels::muted_label;

/// Render the details rows for `entry`.
///
/// `shown_files` is the file preview's parsed list (count, truncated). The
/// returned label is the "Missing" value, which the preview fills in once
/// its off-thread existence check finishes.
pub(crate) fn render_details(
    container: &gtk::Box,
    entry: &ClipboardEntry,
    shown_files: Option<(usize, bool)>,
) -> Option<gtk::Label> {
    rsclip_core::profiler::begin_phase("render_details");
    let type_label = if entry.kind == rsclip_core::models::EntryKind::Text {
        let sample = entry
            .preview_text
            .as_deref()
            .or(entry.text_content.as_deref())
            .unwrap_or("");
        if let Some(lang) = rsclip_core::syntax::detect_code_language(sample) {
            format!("Code ({})", lang.display_name())
        } else {
            entry.kind.to_string()
        }
    } else {
        entry.kind.to_string()
    };
    let mut rows = vec![("Type", type_label)];

    let mut missing_row = None;
    if let EntryData::File { .. } = &entry.data {
        // Counts cover the shown subset, with "+" marking partial results.
        let (shown, truncated) = shown_files.unwrap_or((0, false));
        rows.push((
            "Files",
            if truncated {
                format!("{shown}+")
            } else {
                shown.to_string()
            },
        ));
        missing_row = Some(rows.len());
        rows.push(("Missing", if shown == 0 { "0" } else { "…" }.to_string()));
    }
    rows.extend([
        ("MIME", entry.mime_type.clone()),
        ("Size", human_size(entry.size_bytes)),
        ("First copied", format_full_time(entry.copied_at)),
    ]);
    let mut values = render_rows(container, &rows);
    rsclip_core::profiler::end_phase("render_details");
    missing_row
        .filter(|_| shown_files.is_some_and(|(shown, _)| shown > 0))
        .map(|index| values.swap_remove(index))
}

pub(crate) fn render_secret_details(container: &gtk::Box, secret: &SecretEntry) {
    let rows = [
        ("Type", "secret".to_string()),
        ("Updated", format_full_time(secret.updated_at)),
        ("Created", format_full_time(secret.created_at)),
        ("Copied", secret.use_count.to_string()),
    ];
    render_rows(container, &rows);
}

/// Append one row per `(key, value)`; returns the value labels in order.
fn render_rows(container: &gtk::Box, rows: &[(&str, String)]) -> Vec<gtk::Label> {
    let details = gtk::Box::new(gtk::Orientation::Vertical, 6);
    details.add_css_class("details-grid");
    details.set_hexpand(true);

    let mut values = Vec::with_capacity(rows.len());
    for (label, value) in rows {
        let (row, value) = detail_row(label, value);
        details.append(&row);
        values.push(value);
    }

    container.append(&details);
    values
}

fn detail_row(label: &str, value: &str) -> (gtk::Box, gtk::Label) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    row.add_css_class("details-row");
    row.set_hexpand(true);

    let key = muted_label(label);
    key.add_css_class("details-key");
    key.set_width_request(96);

    let value = gtk::Label::new(Some(value));
    value.add_css_class("details-value");
    value.set_hexpand(true);
    value.set_xalign(1.0);
    value.set_justify(gtk::Justification::Right);
    value.set_wrap(true);
    value.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    value.set_selectable(true);

    row.append(&key);
    row.append(&value);
    (row, value)
}
