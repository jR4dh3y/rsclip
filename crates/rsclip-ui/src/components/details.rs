use gtk::prelude::*;
use gtk4 as gtk;
use rsclip_core::format::{format_full_time, human_size};
use rsclip_core::models::{ClipboardEntry, SecretEntry};

use crate::components::labels::muted_label;

pub(crate) fn render_details(container: &gtk::Box, entry: &ClipboardEntry) {
    let rows = [
        ("Type", entry.kind.to_string()),
        ("MIME", entry.mime_type.clone()),
        ("Size", human_size(entry.size_bytes)),
        ("First copied", format_full_time(entry.copied_at)),
    ];
    render_rows(container, &rows);
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

fn render_rows(container: &gtk::Box, rows: &[(&str, String)]) {
    let details = gtk::Box::new(gtk::Orientation::Vertical, 6);
    details.add_css_class("details-grid");
    details.set_hexpand(true);

    for (label, value) in rows {
        details.append(&detail_row(label, value));
    }

    container.append(&details);
}

fn detail_row(label: &str, value: &str) -> gtk::Box {
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
    row
}
