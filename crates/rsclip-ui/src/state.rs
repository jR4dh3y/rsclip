use std::cell::{Cell, OnceCell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, mpsc};

use anyhow::Context;
use gtk::prelude::*;
use gtk4 as gtk;
use rsclip_core::Database;
use rsclip_core::format::{RelativeAge, relative_age};
use rsclip_core::models::{ClipboardEntry, EntryFilter, SecretEntry, SortMode};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum AppView {
    Clipboard,
    Secrets,
}

/// Identity of a rendered list row: equal keys render identical widgets, so
/// window shifts and new queries can keep those rows instead of rebuilding
/// them. `age` is the relative time the row shows, so a kept row never keeps
/// an outdated "5 min". Labels also distinguish edits within one timestamp tick.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct RowKey {
    view: AppView,
    id: i64,
    updated_at: i64,
    age: RelativeAge,
    pinned: bool,
    label: String,
}

impl RowKey {
    pub(crate) fn entry(entry: &ClipboardEntry) -> Self {
        Self {
            view: AppView::Clipboard,
            id: entry.id,
            updated_at: entry.updated_at,
            age: relative_age(entry.updated_at),
            pinned: entry.pinned,
            label: entry.title.clone(),
        }
    }

    pub(crate) fn secret(secret: &SecretEntry) -> Self {
        Self {
            view: AppView::Secrets,
            id: secret.id,
            updated_at: secret.updated_at,
            age: relative_age(secret.updated_at),
            pinned: false,
            label: secret.alias.clone(),
        }
    }

    #[cfg(test)]
    pub(crate) fn for_test(id: i64) -> Self {
        Self {
            view: AppView::Clipboard,
            id,
            updated_at: 0,
            age: RelativeAge::Now,
            pinned: false,
            label: String::new(),
        }
    }
}

/// Immutable list parameters sent from GTK to the persistent SQLite worker.
pub(crate) struct ListRequest {
    pub(crate) generation: u64,
    pub(crate) view: AppView,
    pub(crate) query: String,
    pub(crate) filter: EntryFilter,
    pub(crate) sort: SortMode,
    pub(crate) row_limit: usize,
    pub(crate) requested_start: usize,
    pub(crate) selected_index: usize,
    pub(crate) preserve_scroll: bool,
    pub(crate) known_total: Option<usize>,
}

/// Rows returned by the worker for the active application view.
pub(crate) enum ListResults {
    Clipboard {
        total: usize,
        start: usize,
        entries: Vec<ClipboardEntry>,
    },
    Secrets {
        total: usize,
        start: usize,
        secrets: Vec<SecretEntry>,
    },
}

/// Worker output paired with request metadata for stale-result validation.
pub(crate) struct ListResponse {
    pub(crate) request: ListRequest,
    pub(crate) result: Result<ListResults, String>,
}

/// A history/secret write, applied on the list worker's connection.
///
/// SQLite rewrites a whole row on UPDATE, so pinning, deleting, or pasting a
/// legacy 40 MB entry took ~200 ms, and any write could wait out the daemon's
/// lock (busy_timeout). Running writes on the worker keeps GTK responsive,
/// and queueing them ahead of the refresh keeps that refresh consistent.
pub(crate) enum WriteOp {
    SetPinned {
        id: i64,
        pinned: bool,
    },
    DeleteEntry(i64),
    DeleteSecret(i64),
    /// Save an entry's text as a secret. The worker reads the full payload,
    /// which for a legacy multi-megabyte row took 100+ ms on the GTK thread.
    SaveSecret {
        entry_id: i64,
        alias: String,
    },
    RenameSecret {
        id: i64,
        alias: String,
    },
    TouchEntry(i64),
    TouchSecret(i64),
}

impl WriteOp {
    pub(crate) fn apply(&self, db: &Database) -> anyhow::Result<()> {
        match self {
            Self::SetPinned { id, pinned } => db.set_pinned(*id, *pinned),
            Self::DeleteEntry(id) => db.delete_entry(*id),
            Self::DeleteSecret(id) => db.delete_secret(*id),
            Self::SaveSecret { entry_id, alias } => {
                let entry = db.get_entry(*entry_id)?.context("entry no longer exists")?;
                let value = rsclip_core::secrets::secret_value_from_entry(&entry)
                    .context("only text-like entries can be saved as secrets")?;
                db.transaction(|db| {
                    db.save_secret(Some(*entry_id), alias, &value)?;
                    db.delete_entry(*entry_id)
                })
            }
            Self::RenameSecret { id, alias } => db.rename_secret(*id, alias),
            Self::TouchEntry(id) => db.touch_used(*id),
            Self::TouchSecret(id) => db.touch_secret_used(*id),
        }
    }
}

/// What the GTK thread does once the copy worker finished a copy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AfterCopy {
    /// Close the overlay and paste (Enter, row activation).
    #[allow(dead_code)]
    Paste,
    /// Close the overlay and always inject Ctrl+V (explicit Enter action).
    /// This remains distinct from [`Self::Paste`] so callers that use the
    /// configurable copy-only behavior can keep it for background actions.
    PasteNow,
    /// Close the overlay without injecting a paste (copy-only action).
    #[allow(dead_code)]
    Hide,
    /// Keep the overlay open and report the copy in the footer.
    Report {
        done: &'static str,
        failed: &'static str,
    },
}

impl AfterCopy {
    /// Ctrl+C on a history entry.
    pub(crate) const REPORT_ENTRY: Self = Self::Report {
        done: "Copied selected entry",
        failed: "Copy failed",
    };
    pub(crate) const REPORT_SECRET: Self = Self::Report {
        done: "Copied secret",
        failed: "Copy failed",
    };
}

/// What a [`CopyRequest`] puts on the clipboard.
pub(crate) enum CopySource {
    /// A history entry; the worker reads its full payload.
    Entry(i64),
    Secret {
        id: i64,
        value: String,
    },
    /// Preview text (file paths, OCR); nothing to mark as used.
    Text(String),
}

impl CopySource {
    /// The write that records this copy as a use, once it succeeded.
    pub(crate) fn touch(&self) -> Option<WriteOp> {
        match self {
            Self::Entry(id) => Some(WriteOp::TouchEntry(*id)),
            Self::Secret { id, .. } => Some(WriteOp::TouchSecret(*id)),
            Self::Text(_) => None,
        }
    }
}

/// Messages to the list worker, processed in order.
pub(crate) enum WorkerRequest {
    Write(WriteOp),
    List(ListRequest),
}

/// A clipboard write, run on the copy worker.
///
/// Every write goes through this one ordered worker, so a slow copy can
/// never overwrite a newer one. An entry copy scales with the payload:
/// reading a legacy 40 MB row took ~140 ms and piping it to `wl-copy` more,
/// all of it on the GTK thread. The worker has its own thread, so a large
/// copy never holds up a search either.
pub(crate) struct CopyRequest {
    pub(crate) source: CopySource,
    pub(crate) then: AfterCopy,
    /// [`AppState::clipboard_serial`] when the copy was queued.
    pub(crate) serial: u64,
}

/// Result of a [`CopyRequest`].
pub(crate) struct CopyResponse {
    /// Applied on success: see [`CopySource::touch`].
    pub(crate) touch: Option<WriteOp>,
    pub(crate) then: AfterCopy,
    pub(crate) serial: u64,
    pub(crate) result: Result<(), String>,
}

/// Bounded preview reads use their own worker so searches and copies cannot
/// delay the selected entry's text.
pub(crate) struct PreviewRequest {
    pub(crate) entry: ClipboardEntry,
    pub(crate) generation: u64,
}

pub(crate) struct PreviewResponse {
    pub(crate) request: PreviewRequest,
    pub(crate) load: Result<Option<crate::components::preview::PreviewLoad>, String>,
}

/// Worker output: the newest list result and/or failures of writes applied
/// before it (reported after the list renders so its footer cannot hide
/// them), a finished copy, or a bounded preview read.
pub(crate) struct WorkerResponse {
    pub(crate) list: Option<ListResponse>,
    pub(crate) copy: Option<CopyResponse>,
    pub(crate) preview: Option<PreviewResponse>,
    pub(crate) write_error: Option<String>,
}

/// Persistent preview widget channels. Content is re-filled per selection;
/// the widget trees themselves are never torn down, so keypresses avoid
/// widget-tree and Pango-layout rebuild costs.
pub(crate) struct PreviewChannels {
    /// Plain-text preview: one TextView whose buffer is re-filled per selection.
    pub(crate) text: gtk::ScrolledWindow,
    pub(crate) text_buffer: gtk::TextBuffer,
    /// Build GtkSourceView only when the user first selects code.
    pub(crate) code: OnceCell<CodePreview>,
}

pub(crate) struct CodePreview {
    pub(crate) scroller: gtk::ScrolledWindow,
    pub(crate) buffer: sourceview5::Buffer,
}

pub(crate) struct AppState {
    pub(crate) db_path: PathBuf,
    pub(crate) list_request_tx: mpsc::Sender<WorkerRequest>,
    pub(crate) list_response_rx: mpsc::Receiver<WorkerResponse>,
    pub(crate) list_generation: Arc<AtomicU64>,
    /// Identifies the currently visible overlay presentation.
    pub(crate) overlay_generation: Cell<u64>,
    pub(crate) copy_request_tx: mpsc::Sender<CopyRequest>,
    pub(crate) preview_request_tx: mpsc::Sender<PreviewRequest>,
    /// Bumped by every clipboard write and by hiding the overlay. A queued
    /// paste only fires while it still matches, so it can never paste a
    /// later copy's value or into a session the user already closed.
    pub(crate) clipboard_serial: Cell<u64>,
    pub(crate) paste_process: RefCell<Option<gio::Subprocess>>,
    /// Copies queued on the copy worker that GTK has not finished handling.
    pub(crate) pending_copies: Cell<usize>,
    pub(crate) favicon_icon_dir: PathBuf,
    /// Daemon capture budget (`[history] max_entries`). The daemon never
    /// prunes to it, so the list does not clamp counts by this value.
    pub(crate) history_limit: Cell<usize>,
    pub(crate) auto_paste: Cell<bool>,
    pub(crate) paste_delay_ms: Cell<u64>,
    pub(crate) paste_method: RefCell<String>,
    pub(crate) ocr_enabled: Cell<bool>,
    pub(crate) ocr_command: RefCell<String>,
    pub(crate) ocr_language: RefCell<String>,
    pub(crate) ocr_timeout_seconds: Cell<u64>,
    pub(crate) default_view: Cell<AppView>,
    pub(crate) default_filter: Cell<EntryFilter>,
    pub(crate) default_sort: Cell<SortMode>,
    pub(crate) reset_on_show: Cell<bool>,
    pub(crate) auto_focus_search: Cell<bool>,
    pub(crate) show_footer_hints: Cell<bool>,
    pub(crate) search_placeholder: RefCell<String>,
    pub(crate) secrets_search_placeholder: RefCell<String>,
    pub(crate) entries: RefCell<Vec<ClipboardEntry>>,
    pub(crate) secrets: RefCell<Vec<SecretEntry>>,
    pub(crate) entries_start: Cell<usize>,
    pub(crate) secrets_start: Cell<usize>,
    pub(crate) entries_total: Cell<usize>,
    pub(crate) secrets_total: Cell<usize>,
    pub(crate) pending_selection: Cell<Option<usize>>,
    /// Keys of the entry/secret rows currently in `list`, in order (spacers excluded).
    pub(crate) rendered_rows: RefCell<Vec<RowKey>>,
    pub(crate) row_cache: RefCell<std::collections::VecDeque<(RowKey, gtk::ListBoxRow)>>,
    pub(crate) virtual_list_update: Cell<bool>,
    pub(crate) query: RefCell<String>,
    pub(crate) filter: RefCell<EntryFilter>,
    pub(crate) sort: RefCell<SortMode>,
    pub(crate) view: RefCell<AppView>,
    pub(crate) prompt_active: RefCell<bool>,
    pub(crate) search_entry: gtk::SearchEntry,
    pub(crate) filter_select: gtk::DropDown,
    pub(crate) history_button: gtk::Button,
    pub(crate) secrets_button: gtk::Button,
    pub(crate) count_label: gtk::Label,
    pub(crate) paned: gtk::Paned,
    pub(crate) list: gtk::ListBox,
    pub(crate) list_adjustment: gtk::Adjustment,
    pub(crate) preview_shell: gtk::Box,
    pub(crate) preview: gtk::Box,
    pub(crate) details: gtk::Box,
    pub(crate) channels: PreviewChannels,
    /// Increments on every preview render; deferred upgrades validate against it.
    pub(crate) preview_generation: Cell<u64>,
    pub(crate) footer: gtk::Label,
    pub(crate) ocr_button: gtk::Button,
    pub(crate) currently_previewed_entry_id: Cell<Option<i64>>,
    pub(crate) currently_previewed_secret_id: Cell<Option<i64>>,
}

/// Invalidate queued and in-flight list work before changing its context.
pub(crate) fn advance_list_generation(state: &AppState) -> u64 {
    state
        .list_generation
        .fetch_add(1, Ordering::Relaxed)
        .wrapping_add(1)
}

/// Invalidate any pending paste; returns the new serial.
pub(crate) fn advance_clipboard_serial(state: &AppState) -> u64 {
    if let Some(process) = state.paste_process.borrow_mut().take() {
        process.force_exit();
    }
    let serial = state.clipboard_serial.get().wrapping_add(1);
    state.clipboard_serial.set(serial);
    serial
}

/// Invalidate callbacks belonging to an earlier overlay presentation.
pub(crate) fn advance_overlay_generation(state: &AppState) -> u64 {
    state
        .overlay_generation
        .set(state.overlay_generation.get().wrapping_add(1));
    state.overlay_generation.get()
}

/// Return the selected list summary without reading its full payload from SQLite.
pub(crate) fn current_entry(state: &Rc<AppState>) -> Option<ClipboardEntry> {
    let row = state.list.selected_row()?;
    entry_at_row(state, &row)
}

pub(crate) fn current_secret(state: &Rc<AppState>) -> Option<SecretEntry> {
    let row = state.list.selected_row()?;
    secret_at_row(state, &row)
}

pub(crate) fn current_entry_index(state: &Rc<AppState>) -> Option<usize> {
    let row = state.list.selected_row()?;
    entry_index_at_row(state, &row)
}

pub(crate) fn current_secret_index(state: &Rc<AppState>) -> Option<usize> {
    let row = state.list.selected_row()?;
    secret_index_at_row(state, &row)
}

pub(crate) fn entry_at_row(state: &Rc<AppState>, row: &gtk::ListBoxRow) -> Option<ClipboardEntry> {
    let relative_index =
        row_relative_index(row, state.entries_start.get(), state.entries.borrow().len())?;
    state.entries.borrow().get(relative_index).cloned()
}

pub(crate) fn secret_at_row(state: &Rc<AppState>, row: &gtk::ListBoxRow) -> Option<SecretEntry> {
    let relative_index =
        row_relative_index(row, state.secrets_start.get(), state.secrets.borrow().len())?;
    state.secrets.borrow().get(relative_index).cloned()
}

pub(crate) fn entry_index_at_row(state: &Rc<AppState>, row: &gtk::ListBoxRow) -> Option<usize> {
    row_relative_index(row, state.entries_start.get(), state.entries.borrow().len())
        .map(|index| state.entries_start.get() + index)
}

pub(crate) fn secret_index_at_row(state: &Rc<AppState>, row: &gtk::ListBoxRow) -> Option<usize> {
    row_relative_index(row, state.secrets_start.get(), state.secrets.borrow().len())
        .map(|index| state.secrets_start.get() + index)
}

pub(crate) fn row_index_for_entry(state: &Rc<AppState>, absolute_index: usize) -> Option<i32> {
    row_index_for_absolute(
        absolute_index,
        state.entries_start.get(),
        state.entries.borrow().len(),
    )
}

pub(crate) fn row_index_for_secret(state: &Rc<AppState>, absolute_index: usize) -> Option<i32> {
    row_index_for_absolute(
        absolute_index,
        state.secrets_start.get(),
        state.secrets.borrow().len(),
    )
}

fn row_relative_index(
    row: &gtk::ListBoxRow,
    window_start: usize,
    window_len: usize,
) -> Option<usize> {
    let list_index = row.index();
    if list_index < 0 {
        return None;
    }

    let top_spacer_rows = i32::from(window_start > 0);
    let relative_index = list_index - top_spacer_rows;
    if relative_index < 0 {
        return None;
    }

    let relative_index = relative_index as usize;
    (relative_index < window_len).then_some(relative_index)
}

fn row_index_for_absolute(
    absolute_index: usize,
    window_start: usize,
    window_len: usize,
) -> Option<i32> {
    if absolute_index < window_start {
        return None;
    }

    let relative_index = absolute_index - window_start;
    if relative_index >= window_len {
        return None;
    }

    Some((relative_index + usize::from(window_start > 0)) as i32)
}

pub(crate) fn invalidate_preview_cache(state: &AppState) {
    state.currently_previewed_entry_id.set(None);
    state.currently_previewed_secret_id.set(None);
}
