use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;

use gtk::prelude::*;
use gtk4 as gtk;
use rsclip_core::favicons::domain_cache_key;
use rsclip_core::files::parse_uri_list_bounded;
use rsclip_core::format::relative_time;
use rsclip_core::models::{ClipboardEntry, EntryData, EntryKind, SecretEntry};
use rsclip_core::syntax::CodeLanguage;

const FAVICON_SLOT_SIZE: i32 = 28;
const FAVICON_SIZE: i32 = 20;

pub(crate) struct ListPanel {
    pub(crate) scroller: gtk::ScrolledWindow,
    pub(crate) list: gtk::ListBox,
    pub(crate) adjustment: gtk::Adjustment,
}

pub(crate) fn build_panel() -> ListPanel {
    let scroller = gtk::ScrolledWindow::builder()
        .min_content_width(220)
        .vexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .build();
    scroller.add_css_class("sidebar");

    let list = gtk::ListBox::new();
    list.add_css_class("entry-list");
    list.set_selection_mode(gtk::SelectionMode::Single);
    scroller.set_child(Some(&list));

    let adjustment = scroller.vadjustment();
    list.set_adjustment(Some(&adjustment));

    ListPanel {
        scroller,
        list,
        adjustment,
    }
}

pub(crate) fn entry_row(entry: &ClipboardEntry, favicon_icon_dir: &Path) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::new();
    row.add_css_class("entry-row");

    let outer = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    outer.add_css_class("entry-row-content");
    outer.set_hexpand(true);
    let lang = detect_entry_language(entry);
    let icon = entry_icon(entry, favicon_icon_dir, lang);
    outer.append(&icon);

    let text = gtk::Box::new(gtk::Orientation::Vertical, 3);
    text.set_hexpand(true);
    let title = gtk::Label::new(Some(&entry.title));
    title.add_css_class("entry-title");
    title.set_xalign(0.0);
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    text.append(&title);

    let subtitle = gtk::Label::new(Some(&subtitle(entry, lang)));
    subtitle.add_css_class("entry-subtitle");
    subtitle.set_xalign(0.0);
    subtitle.set_ellipsize(gtk::pango::EllipsizeMode::End);
    text.append(&subtitle);
    outer.append(&text);

    if entry.pinned {
        let pinned = badge_icon("\u{f08d}", "Pinned");
        outer.append(&pinned);
    }

    row.set_child(Some(&outer));
    row
}

pub(crate) fn secret_row(secret: &SecretEntry) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::new();
    row.add_css_class("entry-row");

    let outer = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    outer.add_css_class("entry-row-content");
    outer.set_hexpand(true);

    let text = gtk::Box::new(gtk::Orientation::Vertical, 3);
    text.set_hexpand(true);

    let title = gtk::Label::new(Some(&secret.alias));
    title.add_css_class("entry-title");
    title.set_xalign(0.0);
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    text.append(&title);

    let subtitle = gtk::Label::new(Some(&relative_time(secret.updated_at)));
    subtitle.add_css_class("entry-subtitle");
    subtitle.set_xalign(0.0);
    subtitle.set_ellipsize(gtk::pango::EllipsizeMode::End);
    text.append(&subtitle);
    outer.append(&text);

    row.set_child(Some(&outer));
    row
}

fn nerd_icon(glyph: &str, color: Option<&str>, tooltip: &str) -> gtk::Widget {
    let label = gtk::Label::new(None);
    label.add_css_class("entry-kind-nerd");
    label.set_tooltip_text(Some(tooltip));
    label.set_width_request(28);
    label.set_halign(gtk::Align::Center);
    label.set_valign(gtk::Align::Center);

    if let Some(color) = color {
        label.set_markup(&format!("<span foreground=\"{color}\">{glyph}</span>"));
    } else {
        label.set_text(glyph);
    }
    label.upcast()
}

fn badge_icon(glyph: &str, tooltip: &str) -> gtk::Widget {
    let badge = gtk::CenterBox::new();
    badge.add_css_class("kind-badge");
    badge.set_tooltip_text(Some(tooltip));
    badge.set_width_request(28);
    badge.set_height_request(28);
    badge.set_halign(gtk::Align::Center);
    badge.set_valign(gtk::Align::Center);

    let label = gtk::Label::new(Some(glyph));
    label.add_css_class("badge-nerd-icon");
    label.set_halign(gtk::Align::Center);
    label.set_valign(gtk::Align::Center);
    badge.set_center_widget(Some(&label));
    badge.upcast()
}

fn entry_icon(
    entry: &ClipboardEntry,
    favicon_icon_dir: &Path,
    lang: Option<CodeLanguage>,
) -> gtk::Widget {
    match &entry.data {
        EntryData::Link { domain, .. } => link_icon(favicon_icon_dir, domain),
        _ => {
            let (glyph, color, tooltip) = resolved_entry_nerd_icon_and_label(entry, lang);
            nerd_icon(glyph, color.as_deref(), &tooltip)
        }
    }
}

thread_local! {
    /// Decoded favicons by domain (`None` = no cached icon on disk). Every
    /// window shift rebuilds link rows; without this each one stat'ed and
    /// decoded its PNG on the GTK thread.
    static FAVICON_CACHE: RefCell<HashMap<String, Option<gdk_pixbuf::Pixbuf>>> =
        RefCell::new(HashMap::new());
}

/// Forget decoded favicons so newly fetched icons are picked up.
pub(crate) fn clear_favicon_cache() {
    FAVICON_CACHE.with(|cache| cache.borrow_mut().clear());
}

fn cached_favicon(favicon_icon_dir: &Path, domain: &str) -> Option<gdk_pixbuf::Pixbuf> {
    FAVICON_CACHE.with(|cache| {
        cache
            .borrow_mut()
            .entry(domain.to_string())
            .or_insert_with(|| {
                let path = favicon_icon_dir.join(format!("{}.png", domain_cache_key(domain)));
                gdk_pixbuf::Pixbuf::from_file_at_scale(&path, FAVICON_SIZE, FAVICON_SIZE, true).ok()
            })
            .clone()
    })
}

fn link_icon(favicon_icon_dir: &Path, domain: &str) -> gtk::Widget {
    if let Some(pixbuf) = cached_favicon(favicon_icon_dir, domain) {
        let icon = gtk::Image::from_pixbuf(Some(&pixbuf));
        icon.add_css_class("link-favicon");
        icon.set_width_request(FAVICON_SIZE);
        icon.set_height_request(FAVICON_SIZE);
        icon.set_halign(gtk::Align::Center);
        icon.set_valign(gtk::Align::Center);
        return favicon_slot(icon.upcast(), domain);
    }

    let fallback = gtk::Label::new(Some("\u{f0c1}"));
    fallback.add_css_class("link-favicon");
    fallback.add_css_class("entry-kind-nerd");
    fallback.set_width_request(FAVICON_SIZE);
    fallback.set_height_request(FAVICON_SIZE);
    fallback.set_halign(gtk::Align::Center);
    fallback.set_valign(gtk::Align::Center);
    fallback.set_xalign(0.5);
    fallback.set_yalign(0.5);
    favicon_slot(fallback.upcast(), domain)
}

fn favicon_slot(child: gtk::Widget, domain: &str) -> gtk::Widget {
    let slot = gtk::CenterBox::new();
    slot.add_css_class("link-favicon-slot");
    slot.set_tooltip_text(Some(domain_tooltip(domain)));
    slot.set_width_request(FAVICON_SLOT_SIZE);
    slot.set_height_request(FAVICON_SIZE);
    slot.set_halign(gtk::Align::Center);
    slot.set_valign(gtk::Align::Center);
    slot.set_center_widget(Some(&child));
    slot.upcast()
}

fn domain_tooltip(domain: &str) -> &str {
    if domain.is_empty() { "Link" } else { domain }
}

fn detect_entry_language(entry: &ClipboardEntry) -> Option<CodeLanguage> {
    if entry.kind != EntryKind::Text {
        return None;
    }
    let sample = entry
        .preview_text
        .as_deref()
        .or(entry.text_content.as_deref())
        .unwrap_or("");
    if sample.is_empty() {
        return None;
    }
    // Bound the sample scanned to at most 4KB so detection never stalls on huge text
    let bounded = if sample.len() > 4096 {
        let mut end = 4096;
        while !sample.is_char_boundary(end) {
            end -= 1;
        }
        &sample[..end]
    } else {
        sample
    };
    rsclip_core::syntax::detect_code_language(bounded)
}

fn resolved_entry_nerd_icon_and_label(
    entry: &ClipboardEntry,
    lang: Option<CodeLanguage>,
) -> (&'static str, Option<String>, String) {
    match &entry.data {
        EntryData::Color { value, .. } => ("\u{f53f}", Some(value.clone()), "Color".to_string()),
        EntryData::File { .. } => ("\u{f07b}", Some("#79b8ff".to_string()), "File".to_string()),
        EntryData::Image { .. } => ("\u{f03e}", Some("#85e89d".to_string()), "Image".to_string()),
        EntryData::Text | EntryData::Unknown => {
            if let Some(lang) = lang {
                (
                    lang.nerd_icon(),
                    Some(lang.nerd_color().to_string()),
                    lang.display_name().to_string(),
                )
            } else if matches!(entry.data, EntryData::Unknown) {
                ("\u{f059}", None, "Unknown".to_string())
            } else {
                ("\u{f0219}", None, "Text".to_string())
            }
        }
        EntryData::Link { .. } => unreachable!(),
    }
}

fn subtitle(entry: &ClipboardEntry, lang: Option<CodeLanguage>) -> String {
    if let EntryData::File { .. } = &entry.data
        && let Some(subtitle) = file_subtitle(entry)
    {
        return subtitle;
    }

    let time = relative_time(entry.updated_at);
    if entry.kind == EntryKind::Text
        && let Some(lang) = lang
    {
        return format!("{time} • {}", lang.display_name());
    }

    time
}

fn file_subtitle(entry: &ClipboardEntry) -> Option<String> {
    let payload = entry
        .text_content
        .as_deref()
        .or(entry.preview_text.as_deref())?;
    let bounded = parse_uri_list_bounded(payload, 50, 4096);
    if bounded.files.is_empty() {
        return None;
    }

    let mut subtitle = file_count_label(bounded.files.len());
    if bounded.truncated {
        subtitle.push('+');
    }
    Some(subtitle)
}

fn file_count_label(count: usize) -> String {
    if count == 1 {
        "1 file".to_string()
    } else {
        format!("{count} files")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_entry(id: i64, text: &str) -> ClipboardEntry {
        ClipboardEntry {
            id,
            content_hash: format!("hash-{id}"),
            kind: EntryKind::Text,
            mime_type: "text/plain".to_string(),
            title: text.to_string(),
            preview_text: Some(text.to_string()),
            text_content: Some(text.to_string()),
            pinned: false,
            copied_at: 1700000000,
            updated_at: 1700000000,
            last_used_at: None,
            use_count: 0,
            size_bytes: 100,
            data: EntryData::Text,
        }
    }

    #[test]
    fn text_entry_with_code_resolves_language_icon_and_label() {
        let entry = test_entry(1, "fn main() {\n    println!(\"hi\");\n}");
        let lang = detect_entry_language(&entry);
        let (glyph, color, label) = resolved_entry_nerd_icon_and_label(&entry, lang);
        assert_eq!(label, "Rust");
        assert_eq!(glyph, "\u{e7a8}");
        assert!(color.is_some());

        let sub = subtitle(&entry, lang);
        assert!(sub.contains("Rust"));
    }

    #[test]
    fn plain_text_entry_resolves_text_icon_and_label() {
        let entry = test_entry(2, "Meeting at 3pm today with Alice");
        let lang = detect_entry_language(&entry);
        let (glyph, _color, label) = resolved_entry_nerd_icon_and_label(&entry, lang);
        assert_eq!(label, "Text");
        assert_eq!(glyph, "\u{f0219}");

        let sub = subtitle(&entry, lang);
        assert!(!sub.contains("•"));
    }
}
