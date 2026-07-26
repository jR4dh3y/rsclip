use std::rc::Rc;

use gtk::gdk;
use gtk::prelude::*;
use gtk4 as gtk;
use rsclip_core::colors::parse_color;
use rsclip_core::files::parse_uri_list;
use rsclip_core::format::masked_secret;
use rsclip_core::models::{ClipboardEntry, EntryData, SecretEntry};

use crate::components::details::{render_details, render_secret_details};
use crate::components::labels::{muted_label, section_label};
use crate::state::AppState;

const MAX_PREVIEW_BYTES: usize = 32 * 1024;
const PREVIEW_TRUNCATED_NOTICE: &str = "\n\n[Preview truncated — copy the entry for full content]";

pub(crate) struct PreviewPanel {
    pub(crate) shell: gtk::Box,
    pub(crate) preview: gtk::Box,
    pub(crate) details: gtk::Box,
}

pub(crate) fn build_panel() -> PreviewPanel {
    let shell = gtk::Box::new(gtk::Orientation::Vertical, 8);
    shell.set_vexpand(true);
    shell.add_css_class("preview-pane");

    let preview = gtk::Box::new(gtk::Orientation::Vertical, 8);
    preview.set_vexpand(true);
    let preview_scroller = gtk::ScrolledWindow::builder()
        .min_content_width(150)
        .min_content_height(64)
        .vexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&preview)
        .build();
    shell.append(&preview_scroller);

    let details = gtk::Box::new(gtk::Orientation::Vertical, 4);
    details.add_css_class("details-panel");
    details.set_hexpand(true);
    shell.append(&details);

    PreviewPanel {
        shell,
        preview,
        details,
    }
}

pub(crate) fn render_secret_preview(state: &Rc<AppState>, secret: &SecretEntry) {
    crate::components::clear_box(&state.preview);
    crate::components::clear_box(&state.details);
    state.ocr_button.set_opacity(0.0);
    state.ocr_button.set_sensitive(false);

    let buffer = gtk::TextBuffer::new(None);
    buffer.set_text(&masked_secret(&secret.value));
    let view = gtk::TextView::with_buffer(&buffer);
    view.add_css_class("preview-text");
    view.set_editable(false);
    view.set_cursor_visible(false);
    view.set_wrap_mode(gtk::WrapMode::WordChar);
    view.set_monospace(true);
    view.set_vexpand(true);

    let scroller = gtk::ScrolledWindow::builder()
        .min_content_height(80)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&view)
        .build();
    state.preview.append(&scroller);

    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);

    let copy_button = gtk::Button::with_label("Copy");
    {
        let state = Rc::clone(state);
        let secret = secret.clone();
        copy_button.connect_clicked(move |_| {
            if let Err(err) = crate::actions::clipboard::copy_secret(&state, &secret) {
                crate::actions::set_footer(&state, &format!("Copy failed: {err:#}"));
            } else {
                crate::actions::set_footer(&state, "Copied secret");
            }
        });
    }
    actions.append(&copy_button);

    let reveal_button = gtk::Button::with_label("Reveal");
    {
        let buffer = buffer.clone();
        let value = secret.value.clone();
        let masked = masked_secret(&secret.value);
        reveal_button.connect_clicked(move |button| {
            if button.label().as_deref() == Some("Reveal") {
                buffer.set_text(&value);
                button.set_label("Hide");
            } else {
                buffer.set_text(&masked);
                button.set_label("Reveal");
            }
        });
    }
    actions.append(&reveal_button);

    let rename_button = gtk::Button::with_label("Rename");
    {
        let state = Rc::clone(state);
        let window = state
            .list
            .root()
            .and_then(|root| root.downcast::<gtk::Window>().ok());
        rename_button.connect_clicked(move |_| {
            if let Some(window) = window.as_ref() {
                crate::actions::secrets::rename_current_secret_dialog(&state, window);
            }
        });
    }
    actions.append(&rename_button);

    let delete_button = gtk::Button::with_label("Delete");
    delete_button.add_css_class("destructive-button");
    {
        let state = Rc::clone(state);
        delete_button.connect_clicked(move |_| {
            if let Err(err) = crate::actions::secrets::delete_current(&state) {
                crate::actions::set_footer(&state, &format!("Delete failed: {err:#}"));
            } else {
                crate::actions::set_footer(&state, "Deleted secret");
            }
        });
    }
    actions.append(&delete_button);

    state.preview.append(&actions);
    render_secret_details(&state.details, secret);
}

pub(crate) fn render_preview(state: &Rc<AppState>, entry: &ClipboardEntry) {
    crate::components::clear_box(&state.preview);
    crate::components::clear_box(&state.details);
    let is_image = matches!(&entry.data, EntryData::Image { .. });
    let can_ocr = is_image && state.ocr_enabled.get();
    state
        .ocr_button
        .set_opacity(if can_ocr { 1.0 } else { 0.0 });
    state.ocr_button.set_sensitive(can_ocr);

    match &entry.data {
        EntryData::Image { .. } => render_image_preview(&state.preview, entry),
        EntryData::Color { value, .. } => render_color_preview(&state.preview, value),
        EntryData::Link { url, .. } => {
            render_text_preview(&state.preview, Some(url));
        }
        EntryData::File { .. } => render_file_preview(state, entry),
        EntryData::Text | EntryData::Unknown => {
            render_text_preview(
                &state.preview,
                entry
                    .text_content
                    .as_deref()
                    .or(entry.preview_text.as_deref()),
            );
        }
    }

    if let EntryData::Image {
        ocr_text: Some(ocr),
        ..
    } = &entry.data
        && !ocr.is_empty()
    {
        render_ocr_header(state, ocr);
        render_text_preview(&state.preview, Some(ocr));
    }

    render_details(&state.details, entry);
}

pub(crate) fn clear_preview_state(state: &Rc<AppState>) {
    crate::components::clear_box(&state.preview);
    crate::components::clear_box(&state.details);
    state.ocr_button.set_opacity(0.0);
    state.ocr_button.set_sensitive(false);
}

fn render_image_preview(container: &gtk::Box, entry: &ClipboardEntry) {
    if let EntryData::Image { file_path, .. } = &entry.data {
        let file = gio::File::for_path(file_path);
        if let Ok(texture) = gdk::Texture::from_file(&file) {
            let ratio = (texture.width() as f32 / texture.height().max(1) as f32).clamp(0.2, 8.0);
            let frame = gtk::AspectFrame::new(0.5, 0.5, ratio, false);
            frame.set_hexpand(true);
            frame.set_vexpand(true);

            let picture = gtk::Picture::for_paintable(&texture);
            picture.set_content_fit(gtk::ContentFit::Contain);
            picture.set_can_shrink(true);
            picture.set_hexpand(true);
            picture.set_vexpand(true);
            frame.set_child(Some(&picture));
            container.append(&frame);
        } else {
            container.append(&muted_label("Image preview is unavailable"));
        }
    } else {
        container.append(&muted_label("Image file is missing"));
    }
}

fn render_file_preview(state: &Rc<AppState>, entry: &ClipboardEntry) {
    let Some(payload) = entry.text_content.as_deref() else {
        state
            .preview
            .append(&muted_label("File list is unavailable"));
        return;
    };
    let files = parse_uri_list(payload);
    if files.is_empty() {
        render_text_preview(&state.preview, Some(payload));
        return;
    }

    let paths = files
        .iter()
        .map(|file| file.path.to_string_lossy().to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let missing_count = files.iter().filter(|file| !file.path.exists()).count();

    let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    header.set_hexpand(true);

    let title = section_label("Files");
    title.set_hexpand(true);
    header.append(&title);

    let mut status = file_count_label(files.len());
    if missing_count > 0 {
        status.push_str(&format!(", {missing_count} missing"));
    }
    header.append(&muted_label(&status));

    let copy = gtk::Button::with_label("Copy paths");
    copy.add_css_class("primary-button");
    {
        let state = Rc::clone(state);
        let paths = paths.clone();
        copy.connect_clicked(move |_| {
            if let Err(err) = crate::actions::clipboard::copy_text(&paths) {
                crate::actions::set_footer(&state, &format!("Copy paths failed: {err:#}"));
            } else {
                crate::actions::set_footer(&state, "Copied paths");
            }
        });
    }
    header.append(&copy);
    state.preview.append(&header);

    render_text_preview(&state.preview, Some(&paths));
}

fn file_count_label(count: usize) -> String {
    if count == 1 {
        "1 file".to_string()
    } else {
        format!("{count} files")
    }
}

fn render_color_preview(container: &gtk::Box, hex: &str) {
    let frame = gtk::AspectFrame::new(0.5, 0.5, 16.0 / 9.0, false);
    frame.set_hexpand(true);
    frame.set_vexpand(true);

    let swatch = gtk::DrawingArea::new();
    swatch.add_css_class("color-swatch");
    swatch.set_hexpand(true);
    swatch.set_vexpand(true);
    let color = parse_color(hex)
        .map(|color| {
            (
                f64::from(color.rgb.0) / 255.0,
                f64::from(color.rgb.1) / 255.0,
                f64::from(color.rgb.2) / 255.0,
            )
        })
        .unwrap_or((0.2, 0.2, 0.2));
    swatch.set_draw_func(move |_, cr, width, height| {
        cr.set_source_rgb(color.0, color.1, color.2);
        cr.rectangle(0.0, 0.0, f64::from(width), f64::from(height));
        let _ = cr.fill();
    });
    frame.set_child(Some(&swatch));
    container.append(&frame);
    render_text_preview(container, Some(hex));
}

fn render_ocr_header(state: &Rc<AppState>, ocr: &str) {
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    header.set_hexpand(true);

    let title = section_label("OCR");
    title.set_hexpand(true);
    header.append(&title);

    let copy = gtk::Button::with_label("Copy OCR");
    copy.add_css_class("primary-button");
    {
        let state = Rc::clone(state);
        let ocr = ocr.to_string();
        copy.connect_clicked(move |_| {
            if let Err(err) = crate::actions::clipboard::copy_text(&ocr) {
                crate::actions::set_footer(&state, &format!("Copy OCR failed: {err:#}"));
            } else {
                crate::actions::set_footer(&state, "Copied OCR text");
            }
        });
    }
    header.append(&copy);
    state.preview.append(&header);
}

fn render_text_preview(container: &gtk::Box, text: Option<&str>) {
    let buffer = gtk::TextBuffer::new(None);
    let text = bounded_preview(text.unwrap_or(""));
    buffer.set_text(&text);
    let view = gtk::TextView::with_buffer(&buffer);
    view.add_css_class("preview-text");
    view.set_editable(false);
    view.set_cursor_visible(false);
    view.set_wrap_mode(gtk::WrapMode::WordChar);
    view.set_monospace(true);
    view.set_vexpand(true);

    let scroller = gtk::ScrolledWindow::builder()
        .min_content_height(80)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .child(&view)
        .build();
    container.append(&scroller);
}

/// Truncate preview text without splitting a UTF-8 code point.
fn bounded_preview(text: &str) -> std::borrow::Cow<'_, str> {
    if text.len() <= MAX_PREVIEW_BYTES {
        return std::borrow::Cow::Borrowed(text);
    }

    let mut end = MAX_PREVIEW_BYTES;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let mut preview = String::with_capacity(end + PREVIEW_TRUNCATED_NOTICE.len());
    preview.push_str(&text[..end]);
    preview.push_str(PREVIEW_TRUNCATED_NOTICE);
    std::borrow::Cow::Owned(preview)
}

#[cfg(test)]
mod tests {
    use super::{MAX_PREVIEW_BYTES, PREVIEW_TRUNCATED_NOTICE, bounded_preview};

    #[test]
    fn preview_is_bounded_on_utf8_boundary() {
        let text = "🦀".repeat(MAX_PREVIEW_BYTES);
        let preview = bounded_preview(&text);

        assert!(preview.ends_with(PREVIEW_TRUNCATED_NOTICE));
        assert!(preview.len() <= MAX_PREVIEW_BYTES + PREVIEW_TRUNCATED_NOTICE.len());
        assert!(std::str::from_utf8(preview.as_bytes()).is_ok());
    }

    #[test]
    fn small_preview_is_unchanged() {
        let text = "small clipboard entry";
        assert_eq!(bounded_preview(text), text);
    }
}
