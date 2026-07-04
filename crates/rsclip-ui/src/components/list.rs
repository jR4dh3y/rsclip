use std::path::Path;

use gtk::prelude::*;
use gtk4 as gtk;
use rsclip_core::favicons::domain_cache_key;
use rsclip_core::files::parse_uri_list;
use rsclip_core::format::relative_time;
use rsclip_core::models::{ClipboardEntry, EntryData, EntryKind, SecretEntry};

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
    let icon = entry_icon(entry, favicon_icon_dir);
    outer.append(&icon);

    let text = gtk::Box::new(gtk::Orientation::Vertical, 3);
    text.set_hexpand(true);
    let title = gtk::Label::new(Some(&entry.title));
    title.add_css_class("entry-title");
    title.set_xalign(0.0);
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    text.append(&title);

    let subtitle = gtk::Label::new(Some(&subtitle(entry)));
    subtitle.add_css_class("entry-subtitle");
    subtitle.set_xalign(0.0);
    subtitle.set_ellipsize(gtk::pango::EllipsizeMode::End);
    text.append(&subtitle);
    outer.append(&text);

    if entry.pinned {
        let pinned = badge_icon("view-pin-symbolic", "Pinned");
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

fn row_icon(icon_name: &str, tooltip: &str) -> gtk::Image {
    let icon = gtk::Image::from_icon_name(icon_name);
    icon.add_css_class("entry-kind");
    icon.set_tooltip_text(Some(tooltip));
    icon.set_pixel_size(16);
    icon
}

fn badge_icon(icon_name: &str, tooltip: &str) -> gtk::Widget {
    let badge = gtk::CenterBox::new();
    badge.add_css_class("kind-badge");
    badge.set_tooltip_text(Some(tooltip));
    badge.set_width_request(28);
    badge.set_height_request(28);
    badge.set_halign(gtk::Align::Center);
    badge.set_valign(gtk::Align::Center);

    let icon = gtk::Image::from_icon_name(icon_name);
    icon.set_pixel_size(12);
    icon.set_halign(gtk::Align::Center);
    icon.set_valign(gtk::Align::Center);
    badge.set_center_widget(Some(&icon));
    badge.upcast()
}

fn entry_icon(entry: &ClipboardEntry, favicon_icon_dir: &Path) -> gtk::Widget {
    match &entry.data {
        EntryData::Link { domain, .. } => link_icon(favicon_icon_dir, domain),
        _ => row_icon(entry_icon_name(entry), entry_kind_label(entry)).upcast(),
    }
}

fn link_icon(favicon_icon_dir: &Path, domain: &str) -> gtk::Widget {
    let path = favicon_icon_dir.join(format!("{}.png", domain_cache_key(domain)));
    if path.exists() {
        let pixbuf =
            gdk_pixbuf::Pixbuf::from_file_at_scale(&path, FAVICON_SIZE, FAVICON_SIZE, true);
        if let Ok(pixbuf) = pixbuf {
            let icon = gtk::Image::from_pixbuf(Some(&pixbuf));
            icon.add_css_class("link-favicon");
            icon.set_width_request(FAVICON_SIZE);
            icon.set_height_request(FAVICON_SIZE);
            icon.set_halign(gtk::Align::Center);
            icon.set_valign(gtk::Align::Center);
            return favicon_slot(icon.upcast(), domain);
        }
    }

    let fallback = gtk::Label::new(Some(&domain_initial(domain)));
    fallback.add_css_class("link-favicon");
    fallback.add_css_class("favicon-fallback");
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

fn domain_initial(domain: &str) -> String {
    domain
        .split('.')
        .find_map(|label| label.chars().find(|ch| ch.is_ascii_alphanumeric()))
        .map(|ch| ch.to_ascii_uppercase().to_string())
        .unwrap_or_else(|| "?".to_string())
}

fn entry_icon_name(entry: &ClipboardEntry) -> &'static str {
    match &entry.data {
        EntryData::Link { .. } => unreachable!(),
        _ => match entry.kind {
            EntryKind::Text => "text-x-generic-symbolic",
            EntryKind::Image => "image-x-generic-symbolic",
            EntryKind::Color => "color-select-symbolic",
            EntryKind::File => "folder-symbolic",
            EntryKind::Unknown => "dialog-question-symbolic",
            EntryKind::Link => unreachable!(),
        },
    }
}

fn entry_kind_label(entry: &ClipboardEntry) -> &'static str {
    match entry.kind {
        EntryKind::Text => "Text",
        EntryKind::Image => "Image",
        EntryKind::Link => "Link",
        EntryKind::Color => "Color",
        EntryKind::File => "File",
        EntryKind::Unknown => "Unknown",
    }
}

fn subtitle(entry: &ClipboardEntry) -> String {
    if let EntryData::File { .. } = &entry.data
        && let Some(subtitle) = file_subtitle(entry)
    {
        return subtitle;
    }

    relative_time(entry.updated_at)
}

fn file_subtitle(entry: &ClipboardEntry) -> Option<String> {
    let files = parse_uri_list(entry.text_content.as_deref()?);
    if files.is_empty() {
        return None;
    }

    let missing = files.iter().filter(|file| !file.path.exists()).count();
    let mut subtitle = file_count_label(files.len());
    if missing > 0 {
        subtitle.push_str(&format!(", {missing} missing"));
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
