use std::rc::Rc;

use gtk::gdk;
use gtk4 as gtk;
// sourceview5's prelude re-exports gtk4's, which covers gtk trait methods here.
use rsclip_core::colors::parse_color;
use rsclip_core::files::{URI_LIST_PREVIEW_MAX_FILES, parse_uri_list_bounded};
use rsclip_core::format::masked_secret;
use rsclip_core::models::{ClipboardEntry, EntryData, SecretEntry};
use rsclip_core::syntax::CodeLanguage;
use sourceview5::prelude::*;

use crate::components::details::{render_details, render_secret_details};
use crate::components::labels::{muted_label, section_label};
use crate::state::{AfterCopy, AppState, CopySource, PreviewRequest, PreviewResponse};

/// Bytes of an entry that previews read. The first 64 KiB of every text entry
/// sits in the `entry_text` side table, so reads within this bound stay cheap
/// even for legacy multi-megabyte rows. Text reads run on the preview worker.
pub(crate) const MAX_PREVIEW_READ_BYTES: usize = 64 * 1024;
/// Bounds for text shown in the preview `TextView`, which lays out and paints
/// on the GTK thread. Pango wraps a paragraph as one unit, so a single long
/// line is the worst case: a 64 KiB base64 line took ~1.3 s to paint and a
/// 64 KiB multi-line log ~40-70 ms. Past these bounds the preview truncates;
/// copying always delivers the full text.
pub(crate) const MAX_PREVIEW_BYTES: usize = 16 * 1024;
pub(crate) const MAX_PREVIEW_LINE_BYTES: usize = 1024;
pub(crate) const MAX_PREVIEW_LINES: usize = 400;
pub(crate) const PREVIEW_LINE_TRUNCATED_MARKER: &str = " …";
/// Payloads larger than this render as plain monospace (no syntax coloring).
/// gtksourceview's context parse costs roughly 1 ms per KiB, so a 64 KiB code
/// row cost ~57 ms per keypress; content beyond the bound is still shown and
/// copy always delivers the full text.
pub(crate) const MAX_HIGHLIGHT_BYTES: usize = 8 * 1024;
pub(crate) const FULL_PREVIEW_TRUNCATED_NOTICE: &str =
    "\n\n[Preview truncated — copy the entry for full content]";
pub(crate) const BINARY_PREVIEW_NOTICE: &str = "[Binary data — copy the entry for full content]";
/// How long a text selection waits for the preview worker before showing the
/// list summary instead. The read normally returns within a millisecond.
const SUMMARY_FALLBACK_DELAY: std::time::Duration = std::time::Duration::from_millis(40);

/// Text ready for the preview widgets: bounded, NUL-free, language known.
pub(crate) struct PreviewBody {
    pub(crate) text: String,
    pub(crate) lang: Option<CodeLanguage>,
}

/// What the preview worker read for an entry whose list summary lacks it.
pub(crate) enum PreviewLoad {
    /// Text and unknown entries: ready to display.
    Body(PreviewBody),
    /// Other kinds (file lists) parse their payload while rendering.
    Payload(String),
}

/// Detect the language of `raw` and bound it for display. Pure, so the
/// preview worker runs it off the GTK thread.
pub(crate) fn prepare_preview_body(raw: &str) -> PreviewBody {
    PreviewBody {
        lang: rsclip_core::syntax::detect_code_language(raw),
        text: sanitize_preview_text(raw).into_owned(),
    }
}

pub(crate) fn prepare_preview_load(entry: &ClipboardEntry, text: String) -> PreviewLoad {
    match entry.data {
        EntryData::Text | EntryData::Unknown => PreviewLoad::Body(prepare_preview_body(&text)),
        _ => PreviewLoad::Payload(text),
    }
}

pub(crate) struct PreviewPanel {
    pub(crate) shell: gtk::Box,
    pub(crate) preview: gtk::Box,
    pub(crate) details: gtk::Box,
    pub(crate) channels: crate::state::PreviewChannels,
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

    // Plain-text preview channel: one TextView whose buffer is re-filled per
    // selection instead of a fresh TextView per selection.
    let text_buffer = gtk::TextBuffer::new(None);
    let text_view = gtk::TextView::with_buffer(&text_buffer);
    text_view.add_css_class("preview-text");
    text_view.set_editable(false);
    text_view.set_cursor_visible(false);
    text_view.set_wrap_mode(gtk::WrapMode::WordChar);
    text_view.set_monospace(true);
    text_view.set_vexpand(true);
    text_view.set_left_margin(8);
    text_view.set_right_margin(8);
    let text_scroller = gtk::ScrolledWindow::builder()
        .min_content_height(80)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .propagate_natural_height(false)
        .child(&text_view)
        .build();

    PreviewPanel {
        shell,
        preview,
        details,
        channels: crate::state::PreviewChannels {
            text: text_scroller,
            text_buffer,
            code: std::cell::OnceCell::new(),
        },
    }
}

fn build_code_channel(language: CodeLanguage) -> crate::state::CodePreview {
    let code_buffer = crate::highlight::setup_source_buffer("", language);
    let code_view = crate::highlight::create_source_view(&code_buffer);
    let code_scroller = gtk::ScrolledWindow::builder()
        .min_content_height(80)
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vexpand(true)
        .propagate_natural_height(false)
        .child(&code_view)
        .build();

    crate::state::CodePreview {
        scroller: code_scroller,
        buffer: code_buffer,
    }
}

pub(crate) fn render_secret_preview(state: &Rc<AppState>, secret: &SecretEntry) {
    if state.currently_previewed_secret_id.get() == Some(secret.id) {
        return;
    }
    state.currently_previewed_secret_id.set(Some(secret.id));
    state.currently_previewed_entry_id.set(None);

    crate::components::clear_box(&state.preview);
    crate::components::clear_box(&state.details);
    state.ocr_button.set_opacity(0.0);
    state.ocr_button.set_sensitive(false);

    let masked = bounded_full_preview(&masked_secret(&secret.value)).into_owned();
    state.channels.text_buffer.set_text(&masked);
    state.channels.text.vadjustment().set_value(0.0);
    state.preview.append(&state.channels.text);

    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);

    let copy_button = gtk::Button::with_label("Copy");
    {
        let state = Rc::clone(state);
        let secret = secret.clone();
        copy_button.connect_clicked(move |_| {
            if let Err(err) = crate::actions::clipboard::queue_secret_copy(
                &state,
                &secret,
                AfterCopy::REPORT_SECRET,
            ) {
                crate::actions::set_footer(&state, &format!("Copy failed: {err:#}"));
            }
        });
    }
    actions.append(&copy_button);

    let reveal_button = gtk::Button::with_label("Reveal");
    {
        let buffer = state.channels.text_buffer.clone();
        let value = bounded_full_preview(&secret.value).into_owned();
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

/// Show `entry` in the preview pane.
///
/// A text entry's list summary only carries a whitespace-collapsed excerpt,
/// so its real text comes from the preview worker. Rendering that excerpt
/// first and the text a millisecond later built every text preview twice,
/// and could compile highlighting for a language the excerpt alone suggested.
/// The pane now keeps its previous content until the worker answers, and
/// falls back to the excerpt only if the read is slow.
pub(crate) fn render_preview(state: &Rc<AppState>, entry: &ClipboardEntry) {
    if state.currently_previewed_entry_id.get() == Some(entry.id) {
        return;
    }
    state.currently_previewed_entry_id.set(Some(entry.id));
    if state.currently_previewed_secret_id.take().is_some() {
        // Never leave a secret's text or actions up while waiting.
        state.channels.text_buffer.set_text("");
        crate::components::clear_box(&state.preview);
        crate::components::clear_box(&state.details);
    }

    let generation = state.preview_generation.get().wrapping_add(1);
    state.preview_generation.set(generation);

    let needs_full_read = entry.text_content.is_none()
        && matches!(
            entry.data,
            EntryData::Text | EntryData::Unknown | EntryData::File { .. }
        );
    if needs_full_read {
        let request = PreviewRequest {
            entry: entry.clone(),
            generation,
        };
        if state.preview_request_tx.send(request).is_ok() {
            schedule_summary_fallback(state, entry.clone(), generation);
            return;
        }
    }
    render_preview_content(state, entry, None);
}

/// Whether `entry`'s request from `generation` is still the wanted preview.
fn preview_is_current(state: &AppState, entry_id: i64, generation: u64) -> bool {
    state.currently_previewed_entry_id.get() == Some(entry_id)
        && state.preview_generation.get() == generation
}

/// Show the list summary if the worker has not answered in time (cold
/// storage, a locked database). A later answer still replaces it.
fn schedule_summary_fallback(state: &Rc<AppState>, entry: ClipboardEntry, generation: u64) {
    let state = Rc::clone(state);
    gtk::glib::timeout_add_local_once(SUMMARY_FALLBACK_DELAY, move || {
        if preview_is_current(&state, entry.id, generation) {
            render_preview_content(&state, &entry, None);
        }
    });
}

/// Rebuild the preview and details panes for `entry`. `body` is the worker's
/// prepared text for a text entry, when it has one.
fn render_preview_content(state: &Rc<AppState>, entry: &ClipboardEntry, body: Option<PreviewBody>) {
    rsclip_core::profiler::begin_phase("render_preview");
    crate::components::clear_box(&state.preview);
    crate::components::clear_box(&state.details);
    let is_image = matches!(&entry.data, EntryData::Image { .. });
    let can_ocr = is_image && state.ocr_enabled.get();
    state
        .ocr_button
        .set_opacity(if can_ocr { 1.0 } else { 0.0 });
    state.ocr_button.set_sensitive(can_ocr);

    let mut file_list = None;
    match &entry.data {
        EntryData::Image { .. } => render_image_preview(state, entry),
        EntryData::Color { value, .. } => render_color_preview(state, value),
        EntryData::Link { url, .. } => {
            render_text_preview_state(state, Some(url));
        }
        EntryData::File { .. } => file_list = render_file_preview(state, entry),
        EntryData::Text | EntryData::Unknown => {
            let content = entry
                .text_content
                .as_deref()
                .or(entry.preview_text.as_deref());
            match body {
                Some(body) => render_text_or_code_preview(state, &body),
                None if content.is_none() && matches!(entry.data, EntryData::Unknown) => {
                    render_text_preview_state(state, Some(BINARY_PREVIEW_NOTICE));
                }
                None => {
                    render_text_or_code_preview(state, &prepare_preview_body(content.unwrap_or("")))
                }
            }
        }
    }

    if let EntryData::Image {
        ocr_text: Some(ocr),
        ..
    } = &entry.data
        && !ocr.is_empty()
    {
        render_ocr_header(state, ocr);
        render_text_preview_state(state, Some(ocr));
    }

    let shown_files = file_list
        .as_ref()
        .map(|files| (files.paths.len(), files.truncated));
    let missing_value = render_details(&state.details, entry, shown_files);
    if let Some(files) = file_list {
        count_missing_files(state, files, missing_value);
    }
    rsclip_core::profiler::end_phase("render_preview");
    if rsclip_core::profiler::enabled() {
        rsclip_core::profiler::print_report();
        rsclip_core::profiler::reset();
    }
}

pub(crate) fn clear_preview_state(state: &Rc<AppState>) {
    crate::state::invalidate_preview_cache(state);
    crate::components::clear_box(&state.preview);
    crate::components::clear_box(&state.details);
    state.channels.text_buffer.set_text("");
    clear_code_buffer(state);
    state.ocr_button.set_opacity(0.0);
    state.ocr_button.set_sensitive(false);
}

/// Render an image preview without touching the image file on the GTK thread.
///
/// Decoding a 1920x1080 PNG takes tens of milliseconds, and even reading the
/// header (`Pixbuf::file_info`) of a 4 MB screenshot cost ~50 ms cold, so
/// arrowing through screenshots stuttered. The frame appears immediately, the
/// header read and decode run on a GIO worker thread, and recently shown
/// images come from a small cache.
fn render_image_preview(state: &Rc<AppState>, entry: &ClipboardEntry) {
    rsclip_core::profiler::begin_phase("render_image_preview");
    let EntryData::Image { file_path, .. } = &entry.data else {
        state.preview.append(&muted_label("Image file is missing"));
        rsclip_core::profiler::end_phase("render_image_preview");
        return;
    };

    if let Some(texture) = cached_image(file_path) {
        let (frame, picture) = image_frame(texture.width(), texture.height());
        picture.set_paintable(Some(&texture));
        state.preview.append(&frame);
        rsclip_core::profiler::end_phase("render_image_preview");
        return;
    }

    // Sized 16:9 until the worker reports the real dimensions.
    let (frame, picture) = image_frame(16, 9);
    state.preview.append(&frame);

    let state = Rc::clone(state);
    let generation = state.preview_generation.get();
    let path = file_path.clone();
    gtk::glib::spawn_future_local(async move {
        let decode_path = path.clone();
        let texture = gio::spawn_blocking(move || decode_image_preview(&decode_path))
            .await
            .ok()
            .flatten();
        if let Some(texture) = &texture {
            cache_image(path, texture.clone());
        }
        if state.preview_generation.get() != generation {
            return;
        }
        match texture {
            Some(texture) => {
                frame.set_ratio(image_ratio(texture.width(), texture.height()));
                picture.set_paintable(Some(&texture));
            }
            None if frame.parent().is_some() => {
                state
                    .preview
                    .insert_child_after(&muted_label("Image preview is unavailable"), Some(&frame));
                state.preview.remove(&frame);
            }
            None => {}
        }
    });
    rsclip_core::profiler::end_phase("render_image_preview");
}

/// Longest decoded edge for image previews; larger images are scaled while
/// decoding so cached textures stay a few megabytes each.
const IMAGE_PREVIEW_MAX_EDGE: i32 = 1600;
const IMAGE_PREVIEW_CACHE_LEN: usize = 6;

thread_local! {
    /// Recently decoded image previews, most recently used last.
    static IMAGE_PREVIEW_CACHE: std::cell::RefCell<std::collections::VecDeque<(String, gdk::Texture)>> =
        const { std::cell::RefCell::new(std::collections::VecDeque::new()) };
}

fn cached_image(path: &str) -> Option<gdk::Texture> {
    IMAGE_PREVIEW_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        let index = cache.iter().position(|(cached, _)| cached == path)?;
        let item = cache.remove(index)?;
        let texture = item.1.clone();
        cache.push_back(item);
        Some(texture)
    })
}

fn cache_image(path: String, texture: gdk::Texture) {
    IMAGE_PREVIEW_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        cache.retain(|(cached, _)| *cached != path);
        cache.push_back((path, texture));
        while cache.len() > IMAGE_PREVIEW_CACHE_LEN {
            cache.pop_front();
        }
    });
}

/// Decode an image for previewing. Runs on a GIO worker thread.
fn decode_image_preview(path: &str) -> Option<gdk::Texture> {
    let (_, width, height) = gdk_pixbuf::Pixbuf::file_info(path)?;
    if width <= IMAGE_PREVIEW_MAX_EDGE && height <= IMAGE_PREVIEW_MAX_EDGE {
        return gdk::Texture::from_filename(path).ok();
    }

    let pixbuf = gdk_pixbuf::Pixbuf::from_file_at_scale(
        path,
        IMAGE_PREVIEW_MAX_EDGE,
        IMAGE_PREVIEW_MAX_EDGE,
        true,
    )
    .ok()?;
    let format = if pixbuf.has_alpha() {
        gdk::MemoryFormat::R8g8b8a8
    } else {
        gdk::MemoryFormat::R8g8b8
    };
    let texture = gdk::MemoryTexture::new(
        pixbuf.width(),
        pixbuf.height(),
        format,
        &pixbuf.read_pixel_bytes(),
        pixbuf.rowstride() as usize,
    );
    Some(texture.upcast())
}

fn image_ratio(width: i32, height: i32) -> f32 {
    (width as f32 / height.max(1) as f32).clamp(0.2, 8.0)
}

/// Aspect-correct frame for an image whose pixels may arrive later.
fn image_frame(width: i32, height: i32) -> (gtk::AspectFrame, gtk::Picture) {
    let frame = gtk::AspectFrame::new(0.5, 0.5, image_ratio(width, height), false);
    frame.set_hexpand(true);
    frame.set_vexpand(true);

    let picture = gtk::Picture::new();
    picture.set_content_fit(gtk::ContentFit::Contain);
    picture.set_can_shrink(true);
    picture.set_hexpand(true);
    picture.set_vexpand(true);
    frame.set_child(Some(&picture));
    (frame, picture)
}

/// A rendered file list whose missing-file count is still being checked.
struct FileListPreview {
    paths: Vec<std::path::PathBuf>,
    truncated: bool,
    status: gtk::Label,
    status_text: String,
}

fn render_file_preview(state: &Rc<AppState>, entry: &ClipboardEntry) -> Option<FileListPreview> {
    rsclip_core::profiler::begin_phase("render_file_preview");
    let Some(payload) = entry.text_content.as_deref() else {
        state
            .preview
            .append(&muted_label("File list is unavailable"));
        rsclip_core::profiler::end_phase("render_file_preview");
        return None;
    };
    // Bound parsing and allocation before touching the payload: legacy rows
    // can hold thousands of URIs and this runs on the GTK thread.
    let bounded =
        parse_uri_list_bounded(payload, URI_LIST_PREVIEW_MAX_FILES, MAX_PREVIEW_READ_BYTES);
    if bounded.files.is_empty() {
        render_text_preview_state(state, Some(payload));
        rsclip_core::profiler::end_phase("render_file_preview");
        return None;
    }

    let files = &bounded.files;
    let paths = files
        .iter()
        .map(|file| file.path.to_string_lossy().to_string())
        .collect::<Vec<_>>()
        .join("\n");

    let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    header.set_hexpand(true);

    let title = section_label("Files");
    title.set_hexpand(true);
    header.append(&title);

    let status_text = if bounded.truncated {
        format!("{}+ files", files.len())
    } else {
        file_count_label(files.len())
    };
    let status = muted_label(&status_text);
    header.append(&status);

    let copy = gtk::Button::with_label("Copy paths");
    copy.add_css_class("primary-button");
    {
        let state = Rc::clone(state);
        let paths = paths.clone();
        copy.connect_clicked(move |_| {
            let then = AfterCopy::Report {
                done: "Copied paths",
                failed: "Copy paths failed",
            };
            let source = CopySource::Text(paths.clone());
            if let Err(err) = crate::actions::clipboard::queue_copy(&state, source, then) {
                crate::actions::set_footer(&state, &format!("Copy paths failed: {err:#}"));
            }
        });
    }
    header.append(&copy);
    state.preview.append(&header);

    // "Copy paths" only copies the shown subset, so label partial results.
    if bounded.truncated {
        state.preview.append(&muted_label(
            "[File list truncated — copy the entry for the full list]",
        ));
    }

    render_text_preview_state(state, Some(&paths));
    rsclip_core::profiler::end_phase("render_file_preview");
    Some(FileListPreview {
        paths: bounded.files.into_iter().map(|file| file.path).collect(),
        truncated: bounded.truncated,
        status,
        status_text,
    })
}

/// Check which listed files still exist, off the GTK thread.
///
/// Up to 500 `stat` calls ran on GTK for every selection of a file entry,
/// once for the header and again for the details pane; a single path on an
/// unresponsive mount froze the overlay. One pass on a worker now fills in
/// both places when it finishes.
fn count_missing_files(
    state: &Rc<AppState>,
    files: FileListPreview,
    missing_value: Option<gtk::Label>,
) {
    let generation = state.preview_generation.get();
    let state = Rc::clone(state);
    gtk::glib::spawn_future_local(async move {
        let FileListPreview {
            paths,
            truncated,
            status,
            status_text,
        } = files;
        let Ok(missing) =
            gio::spawn_blocking(move || paths.iter().filter(|path| !path.exists()).count()).await
        else {
            return;
        };
        if state.preview_generation.get() != generation {
            return;
        }
        if missing > 0 {
            status.set_text(&format!("{status_text}, {missing} missing"));
        }
        if let Some(value) = missing_value {
            value.set_text(&if truncated {
                format!("{missing}+")
            } else {
                missing.to_string()
            });
        }
    });
}

fn file_count_label(count: usize) -> String {
    if count == 1 {
        "1 file".to_string()
    } else {
        format!("{count} files")
    }
}

fn render_color_preview(state: &Rc<AppState>, hex: &str) {
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
        if width <= 0 || height <= 0 {
            return;
        }
        cr.set_source_rgb(color.0, color.1, color.2);
        cr.rectangle(0.0, 0.0, f64::from(width), f64::from(height));
        let _ = cr.fill();
    });
    frame.set_child(Some(&swatch));
    state.preview.append(&frame);
    render_text_preview_state(state, Some(hex));
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
            let then = AfterCopy::Report {
                done: "Copied OCR text",
                failed: "Copy OCR failed",
            };
            let source = CopySource::Text(ocr.clone());
            if let Err(err) = crate::actions::clipboard::queue_copy(&state, source, then) {
                crate::actions::set_footer(&state, &format!("Copy OCR failed: {err:#}"));
            }
        });
    }
    header.append(&copy);
    state.preview.append(&header);
}

fn is_binary_payload(text: &str) -> bool {
    text.as_bytes().contains(&0)
}

fn sanitize_preview_text(text: &str) -> std::borrow::Cow<'_, str> {
    let preview = bounded_full_preview(text);
    if preview.contains('\0') {
        std::borrow::Cow::Owned(preview.replace('\0', " "))
    } else {
        preview
    }
}

/// A background read may finish after selection, view, or content changed.
pub(crate) fn apply_preview_response(state: &Rc<AppState>, response: PreviewResponse) {
    let PreviewResponse { request, load } = response;
    if !preview_is_current(state, request.entry.id, request.generation) {
        return;
    }
    // Later answers for this request are obsolete from here on.
    state
        .preview_generation
        .set(request.generation.wrapping_add(1));
    let mut entry = request.entry;
    match load {
        Ok(Some(PreviewLoad::Body(body))) => render_preview_content(state, &entry, Some(body)),
        Ok(Some(PreviewLoad::Payload(text))) => {
            entry.text_content = Some(text);
            render_preview_content(state, &entry, None);
        }
        // Nothing more to read (or the read failed): the summary is all there is.
        Ok(None) | Err(_) => render_preview_content(state, &entry, None),
    }
}

fn render_text_or_code_preview(state: &Rc<AppState>, body: &PreviewBody) {
    if let Some(lang) = body.lang {
        state.channels.text_buffer.set_text("");
        render_code_preview(state, &body.text, lang);
    } else {
        clear_code_buffer(state);
        rsclip_core::profiler::begin_phase("render_text_preview");
        state.channels.text_buffer.set_text(&body.text);
        state.channels.text.vadjustment().set_value(0.0);
        state.preview.append(&state.channels.text);
        rsclip_core::profiler::end_phase("render_text_preview");
    }
}

fn clear_code_buffer(state: &AppState) {
    if let Some(code) = state.channels.code.get() {
        code.buffer.set_text("");
    }
}

/// Show already bounded `sanitized` text in the source view.
fn render_code_preview(
    state: &Rc<AppState>,
    sanitized: &str,
    lang: rsclip_core::syntax::CodeLanguage,
) {
    rsclip_core::profiler::begin_phase("render_code_preview");

    let code = state.channels.code.get_or_init(|| build_code_channel(lang));
    let buffer = &code.buffer;
    // Bound the highlight work: only payloads within the bound get the
    // context-engine parse; larger ones stay plain monospace, so their
    // language definition is never loaded at all.
    let highlight = sanitized.len() <= MAX_HIGHLIGHT_BYTES;
    // Freeze highlighting during the swap so the re-highlight of a newly set
    // language runs as one batch after the text is in place.
    buffer.set_highlight_syntax(false);
    if highlight && let Some(language) = crate::highlight::cached_language_for(lang) {
        crate::highlight::retain_language_contexts(lang, &language);
        buffer.set_language(Some(&language));
    }
    buffer.set_text(sanitized);
    buffer.set_highlight_syntax(highlight);

    code.scroller.vadjustment().set_value(0.0);
    code.scroller.hadjustment().set_value(0.0);
    state.preview.append(&code.scroller);
    rsclip_core::profiler::end_phase("render_code_preview");
}

fn render_text_preview_state(state: &Rc<AppState>, text: Option<&str>) {
    rsclip_core::profiler::begin_phase("render_text_preview");
    fill_text_preview(state, text);
    state.channels.text.vadjustment().set_value(0.0);
    state.preview.append(&state.channels.text);
    rsclip_core::profiler::end_phase("render_text_preview");
}

/// Fill the persistent plain-text buffer with the bounded preview payload.
fn fill_text_preview(state: &Rc<AppState>, text: Option<&str>) {
    let sanitized = sanitize_preview_text(text.unwrap_or(""));
    state.channels.text_buffer.set_text(&sanitized);
}

/// Bound preview text so the `TextView` lays it out in a few milliseconds.
///
/// Keeps at most [`MAX_PREVIEW_LINES`] lines and [`MAX_PREVIEW_BYTES`] bytes,
/// cuts lines longer than [`MAX_PREVIEW_LINE_BYTES`] (marking the cut), and
/// never splits a UTF-8 code point. Payloads containing interior null bytes
/// are treated as binary and replaced with an explanatory notice instead of
/// crashing GTK FFI.
pub(crate) fn bounded_full_preview(text: &str) -> std::borrow::Cow<'_, str> {
    if is_binary_payload(text) {
        return std::borrow::Cow::Borrowed(BINARY_PREVIEW_NOTICE);
    }

    let within_bounds = text.len() <= MAX_PREVIEW_BYTES
        && text
            .split('\n')
            .enumerate()
            .all(|(index, line)| index < MAX_PREVIEW_LINES && line.len() <= MAX_PREVIEW_LINE_BYTES);
    if within_bounds {
        return std::borrow::Cow::Borrowed(text);
    }

    let mut preview = String::with_capacity(
        MAX_PREVIEW_BYTES.min(text.len()) + FULL_PREVIEW_TRUNCATED_NOTICE.len(),
    );
    let mut truncated = false;
    for (index, line) in text.split('\n').enumerate() {
        // Stop before the separator once the budget is spent: empty lines
        // would otherwise keep adding one byte each past it.
        if index == MAX_PREVIEW_LINES || (index > 0 && preview.len() >= MAX_PREVIEW_BYTES) {
            truncated = true;
            break;
        }
        if index > 0 {
            preview.push('\n');
        }
        let budget = MAX_PREVIEW_BYTES.saturating_sub(preview.len());
        let limit = MAX_PREVIEW_LINE_BYTES.min(budget);
        if line.len() <= limit {
            preview.push_str(line);
            continue;
        }
        preview.push_str(&line[..floor_char_boundary(line, limit)]);
        truncated = true;
        if limit == budget {
            break;
        }
        preview.push_str(PREVIEW_LINE_TRUNCATED_MARKER);
    }
    if truncated {
        preview.push_str(FULL_PREVIEW_TRUNCATED_NOTICE);
    }
    std::borrow::Cow::Owned(preview)
}

fn floor_char_boundary(text: &str, mut index: usize) -> usize {
    while !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

#[cfg(test)]
mod tests {
    use super::{
        BINARY_PREVIEW_NOTICE, FULL_PREVIEW_TRUNCATED_NOTICE, MAX_PREVIEW_BYTES,
        MAX_PREVIEW_LINE_BYTES, MAX_PREVIEW_LINES, PREVIEW_LINE_TRUNCATED_MARKER,
        bounded_full_preview,
    };

    /// Shapes that once froze the GTK thread when previewed unbounded.
    fn pathological_payloads() -> Vec<(&'static str, String)> {
        vec![
            ("64 KiB single-line base64", "QUJD".repeat(16 * 1024)),
            ("1 MiB single line", "x".repeat(1024 * 1024)),
            ("multi-byte single line", "🦀".repeat(MAX_PREVIEW_BYTES)),
            (
                "64 KiB JSON",
                "{\"key\": \"value\", \"n\": 12345},\n".repeat(2_000),
            ),
            ("100k empty lines", "\n".repeat(100_000)),
            (
                "exactly full budget, then empty lines",
                format!("{}\n", "z".repeat(MAX_PREVIEW_LINE_BYTES - 1)).repeat(15)
                    + &"z".repeat(MAX_PREVIEW_LINE_BYTES)
                    + &"\n".repeat(300),
            ),
            (
                "long lines between short ones",
                format!("head\n{}\ntail\n", "y".repeat(10_000)).repeat(20),
            ),
            (
                "CRLF log",
                "2026-09-30 INFO request served\r\n".repeat(5_000),
            ),
        ]
    }

    /// Every preview must stay within the layout bounds, whatever the payload.
    #[test]
    fn perf_preview_text_stays_within_layout_bounds() {
        let max_body = MAX_PREVIEW_BYTES + PREVIEW_LINE_TRUNCATED_MARKER.len();
        let max_line = MAX_PREVIEW_LINE_BYTES + PREVIEW_LINE_TRUNCATED_MARKER.len();
        for (name, payload) in pathological_payloads() {
            let preview = bounded_full_preview(&payload);
            let body = preview
                .strip_suffix(FULL_PREVIEW_TRUNCATED_NOTICE)
                .unwrap_or(&preview);
            assert!(
                body.len() <= max_body,
                "{name}: preview body is {} bytes",
                body.len()
            );
            assert!(
                body.split('\n').count() <= MAX_PREVIEW_LINES,
                "{name}: preview keeps {} lines",
                body.split('\n').count()
            );
            let longest = body.split('\n').map(str::len).max().unwrap_or(0);
            assert!(
                longest <= max_line,
                "{name}: preview line is {longest} bytes"
            );
            assert!(
                preview.ends_with(FULL_PREVIEW_TRUNCATED_NOTICE),
                "{name}: truncation must be announced"
            );
        }
    }

    /// Measured on the preview `TextView`: one wrapped 8 KiB line paints in
    /// ~18 ms, 64 KiB in ~1.3 s, so the bounds must stay near these values.
    /// The headless UI test (tests/headless_ui_perf.rs) times the real paint.
    #[test]
    fn perf_preview_bounds_stay_small() {
        const { assert!(MAX_PREVIEW_LINE_BYTES <= 2 * 1024) };
        const { assert!(MAX_PREVIEW_BYTES <= 32 * 1024) };
        const { assert!(MAX_PREVIEW_LINES <= 1_000) };
    }

    #[test]
    fn preview_is_bounded_on_utf8_boundary() {
        let text = "🦀".repeat(MAX_PREVIEW_BYTES);
        let preview = bounded_full_preview(&text);

        assert!(preview.ends_with(FULL_PREVIEW_TRUNCATED_NOTICE));
        assert!(std::str::from_utf8(preview.as_bytes()).is_ok());
    }

    #[test]
    fn long_line_is_cut_and_marked_but_following_lines_survive() {
        let text = format!("{}\nsecond line", "a".repeat(MAX_PREVIEW_LINE_BYTES * 3));
        let preview = bounded_full_preview(&text);

        let mut lines = preview.split('\n');
        let first = lines.next().unwrap();
        assert!(first.ends_with(PREVIEW_LINE_TRUNCATED_MARKER));
        assert_eq!(
            first.len(),
            MAX_PREVIEW_LINE_BYTES + PREVIEW_LINE_TRUNCATED_MARKER.len()
        );
        assert_eq!(
            lines.next().map(|line| line.starts_with("second line")),
            Some(true)
        );
        assert!(preview.ends_with(FULL_PREVIEW_TRUNCATED_NOTICE));
    }

    #[test]
    fn preview_within_bounds_is_borrowed_unchanged() {
        for text in [
            "small clipboard entry",
            "line one\r\nline two\n",
            &"z".repeat(MAX_PREVIEW_LINE_BYTES),
        ] {
            let preview = bounded_full_preview(text);
            assert!(matches!(preview, std::borrow::Cow::Borrowed(_)));
            assert_eq!(preview, text);
        }
    }

    #[test]
    fn binary_payload_shows_binary_notice() {
        let binary = "some\0binary\0data";
        assert_eq!(bounded_full_preview(binary), BINARY_PREVIEW_NOTICE);
    }
}
