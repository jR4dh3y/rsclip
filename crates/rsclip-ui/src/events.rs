use std::cell::RefCell;
use std::rc::{Rc, Weak};
use std::sync::mpsc;
use std::time::Duration;

use anyhow::{Context, Result};
use gtk::gdk;
use gtk::prelude::*;
use gtk4 as gtk;
use rsclip_core::Database;
use rsclip_core::models::{EntryFilter, EntryKind};

use crate::actions::clipboard::{
    copy_entry_by_id, copy_text, finish_copy, queue_entry_copy, queue_secret_copy,
};
use crate::actions::ocr::run_ocr_for_entry;
use crate::actions::refresh::{
    apply_clipboard_search_results, apply_secret_search_results, refresh_entries,
    refresh_window_for_scroll, search_window_row_count,
};
use crate::actions::secrets::{
    delete_current, rename_current_secret_dialog, save_current_as_secret_dialog, toggle_pin,
};
use crate::actions::selection::{
    mark_selected_row, move_selection, schedule_preview_for_current_selection,
};
use crate::actions::{set_footer, update_mode_controls};
use crate::state::{
    AfterCopy, AppState, AppView, CopyRequest, CopyResponse, CopySource, ListRequest, ListResponse,
    ListResults, WorkerRequest, WorkerResponse, WriteOp, current_entry, current_secret,
    entry_at_row, secret_at_row,
};

/// Delay before applying search so fast typing renders only the final query.
/// Queries take a few milliseconds and the worker coalesces requests, so this
/// only needs to cover the gap between keystrokes of a quick burst.
const SEARCH_DEBOUNCE: Duration = Duration::from_millis(60);

thread_local! {
    /// GTK-thread state that list-worker wakeups deliver responses to.
    static LIST_RESPONSE_TARGET: RefCell<Weak<AppState>> = const { RefCell::new(Weak::new()) };
}

/// Channels to the background SQLite workers; both answer on `response_rx`.
pub(crate) struct Workers {
    pub(crate) list_request_tx: mpsc::Sender<WorkerRequest>,
    pub(crate) copy_request_tx: mpsc::Sender<CopyRequest>,
    pub(crate) response_rx: mpsc::Receiver<WorkerResponse>,
}

/// Start the list worker and the copy worker. Copies get their own thread and
/// connection so a payload-sized copy never delays a search.
pub(crate) fn start_workers(db_path: std::path::PathBuf) -> Result<Workers> {
    let (list_request_tx, list_request_rx) = mpsc::channel();
    let (copy_request_tx, copy_request_rx) = mpsc::channel();
    let (response_tx, response_rx) = mpsc::channel();
    let copy_response_tx = response_tx.clone();
    let copy_db_path = db_path.clone();
    std::thread::Builder::new()
        .name("rsclip-list".to_string())
        .spawn(move || list_worker(db_path, list_request_rx, response_tx))
        .context("starting list worker")?;
    std::thread::Builder::new()
        .name("rsclip-copy".to_string())
        .spawn(move || copy_worker(copy_db_path, copy_request_rx, copy_response_tx))
        .context("starting copy worker")?;
    Ok(Workers {
        list_request_tx,
        copy_request_tx,
        response_rx,
    })
}

pub(crate) fn connect(state: &Rc<AppState>, window: &gtk::ApplicationWindow) {
    LIST_RESPONSE_TARGET.with(|target| *target.borrow_mut() = Rc::downgrade(state));
    drain_list_responses();
    connect_mode_buttons(state);
    connect_ocr_button(state);
    connect_search(state);
    connect_filter(state);
    connect_lazy_scroll(state);
    connect_list_selection(state);
    connect_list_activation(state);
    connect_keyboard(state, window);
}

fn connect_lazy_scroll(state: &Rc<AppState>) {
    let adjustment = state.list_adjustment.clone();
    let state = Rc::clone(state);
    adjustment.connect_value_changed(move |_| {
        if let Err(err) = refresh_window_for_scroll(&state) {
            set_footer(&state, &format!("Scroll failed: {err:#}"));
        }
    });
}

fn connect_mode_buttons(state: &Rc<AppState>) {
    {
        let state = Rc::clone(state);
        let search = state.search_entry.clone();
        let button = state.history_button.clone();
        button.connect_clicked(move |_| {
            *state.view.borrow_mut() = AppView::Clipboard;
            *state.query.borrow_mut() = String::new();
            search.set_text("");
            let placeholder = crate::window::search_placeholder(state.as_ref(), AppView::Clipboard);
            search.set_placeholder_text(Some(&placeholder));
            update_mode_controls(&state);
            if let Err(err) = refresh_entries(&state) {
                set_footer(&state, &format!("Switch failed: {err:#}"));
            }
        });
    }

    {
        let state = Rc::clone(state);
        let search = state.search_entry.clone();
        let button = state.secrets_button.clone();
        button.connect_clicked(move |_| {
            *state.view.borrow_mut() = AppView::Secrets;
            *state.query.borrow_mut() = String::new();
            search.set_text("");
            let placeholder = crate::window::search_placeholder(state.as_ref(), AppView::Secrets);
            search.set_placeholder_text(Some(&placeholder));
            update_mode_controls(&state);
            if let Err(err) = refresh_entries(&state) {
                set_footer(&state, &format!("Switch failed: {err:#}"));
            }
        });
    }
}

fn switch_view(state: &Rc<AppState>, view: AppView) {
    if *state.view.borrow() == view {
        return;
    }

    *state.view.borrow_mut() = view;
    *state.query.borrow_mut() = String::new();
    state.search_entry.set_text("");
    let placeholder = crate::window::search_placeholder(state.as_ref(), view);
    state.search_entry.set_placeholder_text(Some(&placeholder));
    update_mode_controls(state);
    if let Err(err) = refresh_entries(state) {
        set_footer(state, &format!("Switch failed: {err:#}"));
    }
}

fn connect_ocr_button(state: &Rc<AppState>) {
    let button = state.ocr_button.clone();
    let state = Rc::clone(state);
    button.connect_clicked(move |_| {
        if let Some(entry) = current_entry(&state)
            && matches!(entry.kind, EntryKind::Image)
            && let Err(err) = run_ocr_for_entry(&state, entry.id)
        {
            set_footer(&state, &format!("OCR failed: {err:#}"));
        }
    });
}

/// Connect the debounced GTK search field to a persistent SQLite worker.
fn connect_search(state: &Rc<AppState>) {
    let search = state.search_entry.clone();
    let state = Rc::clone(state);
    let pending = Rc::new(RefCell::new(None::<gtk::glib::SourceId>));
    search.connect_search_changed(move |entry| {
        let text = entry.text().to_string();
        crate::state::advance_list_generation(&state);
        if let Some(source_id) = pending.borrow_mut().take() {
            source_id.remove();
        }
        // Programmatic resets (tab switch, etc.) set query before set_text; still cancel any
        // in-flight debounce so a mid-type timer cannot refresh after the reset.
        if *state.query.borrow() == text {
            return;
        }
        *state.query.borrow_mut() = text;

        let state = Rc::clone(&state);
        let pending_for_timeout = Rc::clone(&pending);
        let source_id = gtk::glib::timeout_add_local_once(SEARCH_DEBOUNCE, move || {
            let _ = pending_for_timeout.borrow_mut().take();
            let generation = crate::state::advance_list_generation(&state);
            let request = ListRequest {
                generation,
                view: *state.view.borrow(),
                query: state.query.borrow().clone(),
                filter: *state.filter.borrow(),
                sort: *state.sort.borrow(),
                row_limit: search_window_row_count(&state),
                requested_start: 0,
                selected_index: 0,
                preserve_scroll: false,
                known_total: None,
            };
            if queue_list(&state, request).is_err() {
                set_footer(&state, "List worker stopped");
            } else {
                set_footer(&state, "Searching…");
            }
        });
        *pending.borrow_mut() = Some(source_id);
    });
}

/// Queue one list query on the persistent worker; its response arrives via
/// [`drain_list_responses`].
pub(crate) fn queue_list(state: &Rc<AppState>, request: ListRequest) -> Result<()> {
    send_to_worker(state, WorkerRequest::List(request))
}

/// Queue a database write on the worker. It runs before any list request
/// queued after it, so a following refresh already sees the change.
pub(crate) fn queue_write(state: &Rc<AppState>, op: WriteOp) -> Result<()> {
    send_to_worker(state, WorkerRequest::Write(op))
}

pub(crate) fn send_to_worker(state: &Rc<AppState>, request: WorkerRequest) -> Result<()> {
    state
        .list_request_tx
        .send(request)
        .map_err(|_| anyhow::anyhow!("list worker stopped"))
}

/// Send a worker response and wake the GTK main loop to apply it immediately.
///
/// A 16 ms poll timer used to add up to a frame of latency to every search
/// and scroll window; the wakeup runs as soon as the main loop is free.
fn send_worker_response(
    response_tx: &mpsc::Sender<WorkerResponse>,
    response: WorkerResponse,
) -> bool {
    if response_tx.send(response).is_err() {
        return false;
    }
    gtk::glib::MainContext::default()
        .invoke_with_priority(gtk::glib::Priority::DEFAULT, drain_list_responses);
    true
}

/// Apply every queued worker response on the GTK thread.
fn drain_list_responses() {
    let Some(state) = LIST_RESPONSE_TARGET.with(|target| target.borrow().upgrade()) else {
        return;
    };
    while let Ok(response) = state.list_response_rx.try_recv() {
        if let Some(list) = response.list {
            apply_list_response(&state, list);
        }
        if let Some(copy) = response.copy {
            finish_copy(&state, copy);
        }
        if let Some(err) = response.write_error {
            set_footer(&state, &format!("Update failed: {err}"));
        }
    }
}

/// Own a dedicated SQLite connection: apply queued writes in order and run
/// only the newest queued list request.
fn list_worker(
    db_path: std::path::PathBuf,
    request_rx: mpsc::Receiver<WorkerRequest>,
    response_tx: mpsc::Sender<WorkerResponse>,
) {
    let db = match Database::open(db_path) {
        Ok(db) => db,
        Err(err) => {
            let err = format!("{err:#}");
            for request in request_rx {
                let response = match request {
                    WorkerRequest::Write(_) => WorkerResponse {
                        list: None,
                        copy: None,
                        write_error: Some(err.clone()),
                    },
                    WorkerRequest::List(request) => WorkerResponse {
                        list: Some(ListResponse {
                            request,
                            result: Err(err.clone()),
                        }),
                        copy: None,
                        write_error: None,
                    },
                };
                if !send_worker_response(&response_tx, response) {
                    break;
                }
            }
            return;
        }
    };

    while let Ok(first) = request_rx.recv() {
        let mut list_request = None;
        let mut write_error = None;
        let mut next = Some(first);
        while let Some(message) = next {
            match message {
                WorkerRequest::Write(op) => {
                    if let Err(err) = op.apply(&db) {
                        write_error = Some(format!("{err:#}"));
                    }
                }
                // If typing produced several requests before SQLite became
                // available, only execute the newest one. An in-flight older
                // result is rejected in GTK.
                WorkerRequest::List(request) => list_request = Some(request),
            }
            next = request_rx.try_recv().ok();
        }

        let list = list_request.map(|request| {
            let result = load_list(&db, &request).map_err(|err| format!("{err:#}"));
            ListResponse { request, result }
        });
        if (list.is_some() || write_error.is_some())
            && !send_worker_response(
                &response_tx,
                WorkerResponse {
                    list,
                    copy: None,
                    write_error,
                },
            )
        {
            break;
        }
    }
}

/// Run clipboard writes in order. The connection opens on the first entry
/// copy, so an overlay that never copies one never pays for it.
fn copy_worker(
    db_path: std::path::PathBuf,
    request_rx: mpsc::Receiver<CopyRequest>,
    response_tx: mpsc::Sender<WorkerResponse>,
) {
    let mut db = None;
    for request in request_rx {
        let result = match &request.source {
            CopySource::Entry(id) => {
                match db.get_or_insert_with(|| {
                    Database::open(&db_path).map_err(|err| format!("{err:#}"))
                }) {
                    Ok(db) => copy_entry_by_id(db, *id).map_err(|err| format!("{err:#}")),
                    Err(err) => Err(err.clone()),
                }
            }
            CopySource::Secret { value, .. } | CopySource::Text(value) => {
                copy_text(value).map_err(|err| format!("{err:#}"))
            }
        };
        let response = WorkerResponse {
            list: None,
            copy: Some(CopyResponse {
                touch: request.source.touch(),
                then: request.then,
                serial: request.serial,
                result,
            }),
            write_error: None,
        };
        if !send_worker_response(&response_tx, response) {
            break;
        }
    }
}

/// Load one list window without accessing GTK-owned state.
fn load_list(db: &Database, request: &ListRequest) -> anyhow::Result<ListResults> {
    Ok(match request.view {
        AppView::Clipboard => {
            let start = request.requested_start;
            let window_len = request.row_limit;
            let entries = db.list_entry_summaries_page(
                &request.query,
                request.filter,
                request.sort,
                window_len,
                start,
            )?;
            let total = match request.known_total {
                Some(total) => total,
                None => {
                    if start == 0 && entries.len() < window_len {
                        entries.len()
                    } else {
                        db.count_entries(&request.query, request.filter)?
                    }
                }
            };
            ListResults::Clipboard {
                total,
                start,
                entries,
            }
        }
        AppView::Secrets => {
            let start = request.requested_start;
            let window_len = request.row_limit;
            let secrets = db.list_secrets_page(&request.query, window_len, start)?;
            let total = match request.known_total {
                Some(total) => total,
                None => {
                    if start == 0 && secrets.len() < window_len {
                        secrets.len()
                    } else {
                        db.count_secrets(&request.query)?
                    }
                }
            };
            ListResults::Secrets {
                total,
                start,
                secrets,
            }
        }
    })
}

/// Apply a response only when its complete list context is still current.
fn apply_list_response(state: &Rc<AppState>, response: ListResponse) {
    let request = &response.request;
    let is_current = request.generation == state.list_generation.get()
        && request.view == *state.view.borrow()
        && request.query == *state.query.borrow()
        && request.filter == *state.filter.borrow()
        && request.sort == *state.sort.borrow();
    if !is_current {
        return;
    }

    match response.result {
        Ok(ListResults::Clipboard {
            total,
            start,
            entries,
        }) => apply_clipboard_search_results(state, request, total, start, entries),
        Ok(ListResults::Secrets {
            total,
            start,
            secrets,
        }) => apply_secret_search_results(state, request, total, start, secrets),
        Err(err) => set_footer(state, &format!("List refresh failed: {err}")),
    }
}

fn connect_filter(state: &Rc<AppState>) {
    let filter = state.filter_select.clone();
    let state = Rc::clone(state);
    filter.connect_selected_notify(move |dropdown| {
        *state.filter.borrow_mut() = match dropdown.selected() {
            1 => EntryFilter::Code,
            2 => EntryFilter::Text,
            3 => EntryFilter::Images,
            4 => EntryFilter::Files,
            5 => EntryFilter::Links,
            6 => EntryFilter::Colors,
            7 => EntryFilter::Pinned,
            _ => EntryFilter::All,
        };
        if let Err(err) = refresh_entries(&state) {
            set_footer(&state, &format!("Filter failed: {err:#}"));
        }
    });
}

fn connect_list_selection(state: &Rc<AppState>) {
    let list = state.list.clone();
    let state = Rc::clone(state);
    list.connect_row_selected(move |_, row| {
        mark_selected_row(row);
        // Window rebuilds already render the preview directly via
        // select_*_row; skipping here avoids a second sync DB read and
        // TextView layout per rebuild.
        if state.virtual_list_update.get() {
            return;
        }
        // Coalesce user-driven selection changes (key-repeat, mouse) so at most
        // one preview rebuild happens per main-loop iteration.
        schedule_preview_for_current_selection(&state);
    });
}

/// Connect activation so both clipboard entries and secrets follow their
/// configured post-copy behavior.
fn connect_list_activation(state: &Rc<AppState>) {
    let list = state.list.clone();
    let state = Rc::clone(state);
    list.connect_row_activated(move |_, row| match *state.view.borrow() {
        AppView::Clipboard => {
            if let Some(entry) = entry_at_row(&state, row)
                && let Err(err) = queue_entry_copy(&state, entry.id, AfterCopy::PasteNow)
            {
                set_footer(&state, &format!("Paste failed: {err:#}"));
            }
        }
        AppView::Secrets => {
            if let Some(secret) = secret_at_row(&state, row)
                && let Err(err) = queue_secret_copy(&state, &secret, AfterCopy::PasteNow)
            {
                set_footer(&state, &format!("Copy failed: {err:#}"));
            }
        }
    });
}

fn connect_keyboard(state: &Rc<AppState>, window: &gtk::ApplicationWindow) {
    let controller = gtk::EventControllerKey::new();
    controller.set_propagation_phase(gtk::PropagationPhase::Capture);
    {
        let state = Rc::clone(state);
        let window = window.clone();
        controller.connect_key_pressed(move |_, key, _, modifiers| {
            if *state.prompt_active.borrow() {
                return gtk::glib::Propagation::Proceed;
            }

            let ctrl = modifiers.contains(gdk::ModifierType::CONTROL_MASK);
            match (key, ctrl) {
                (gdk::Key::Tab, false) => {
                    let next_view = match *state.view.borrow() {
                        AppView::Clipboard => AppView::Secrets,
                        AppView::Secrets => AppView::Clipboard,
                    };
                    switch_view(&state, next_view);
                    gtk::glib::Propagation::Stop
                }
                (gdk::Key::Down, false) => {
                    move_selection(&state, 1);
                    gtk::glib::Propagation::Stop
                }
                (gdk::Key::Up, false) => {
                    move_selection(&state, -1);
                    gtk::glib::Propagation::Stop
                }
                (gdk::Key::Escape, _) => {
                    crate::window::hide_overlay(&state, &window);
                    gtk::glib::Propagation::Stop
                }
                (gdk::Key::Return | gdk::Key::KP_Enter, false) => {
                    handle_enter(&state);
                    gtk::glib::Propagation::Stop
                }
                (gdk::Key::Return | gdk::Key::KP_Enter, true) => {
                    handle_copy(&state);
                    gtk::glib::Propagation::Stop
                }
                (gdk::Key::s | gdk::Key::S, true) => {
                    match *state.view.borrow() {
                        AppView::Clipboard => {
                            save_current_as_secret_dialog(&state, window.upcast_ref())
                        }
                        AppView::Secrets => {
                            if let Some(secret) = current_secret(&state)
                                && let Err(err) =
                                    queue_secret_copy(&state, &secret, AfterCopy::REPORT_SECRET)
                            {
                                set_footer(&state, &format!("Copy failed: {err:#}"));
                            }
                        }
                    }
                    gtk::glib::Propagation::Stop
                }
                (gdk::Key::p | gdk::Key::P, true) => {
                    if *state.view.borrow() == AppView::Clipboard
                        && let Err(err) = toggle_pin(&state)
                    {
                        set_footer(&state, &format!("Pin failed: {err:#}"));
                    }
                    gtk::glib::Propagation::Stop
                }
                (gdk::Key::d | gdk::Key::D, true) => {
                    if let Err(err) = delete_current(&state) {
                        set_footer(&state, &format!("Delete failed: {err:#}"));
                    }
                    gtk::glib::Propagation::Stop
                }
                (gdk::Key::e | gdk::Key::E, true) => {
                    if *state.view.borrow() == AppView::Secrets {
                        rename_current_secret_dialog(&state, window.upcast_ref());
                    }
                    gtk::glib::Propagation::Stop
                }
                (gdk::Key::r | gdk::Key::R, true) => {
                    if let Err(err) = refresh_entries(&state) {
                        set_footer(&state, &format!("Refresh failed: {err:#}"));
                    }
                    gtk::glib::Propagation::Stop
                }
                (gdk::Key::i | gdk::Key::I, true) => {
                    if *state.view.borrow() == AppView::Clipboard {
                        set_filter(&state, EntryFilter::Images);
                    }
                    gtk::glib::Propagation::Stop
                }
                (gdk::Key::l | gdk::Key::L, true) => {
                    if *state.view.borrow() == AppView::Clipboard {
                        set_filter(&state, EntryFilter::Links);
                    }
                    gtk::glib::Propagation::Stop
                }
                (gdk::Key::c | gdk::Key::C, true) => {
                    handle_copy(&state);
                    gtk::glib::Propagation::Stop
                }
                _ => gtk::glib::Propagation::Proceed,
            }
        });
    }
    window.add_controller(controller);
}

fn set_filter(state: &Rc<AppState>, filter: EntryFilter) {
    *state.filter.borrow_mut() = filter;
    let selected = crate::window::filter_index(filter);
    let dropdown_changed = state.filter_select.selected() != selected;
    state.filter_select.set_selected(selected);
    if !dropdown_changed && let Err(err) = refresh_entries(state) {
        set_footer(state, &format!("Filter failed: {err:#}"));
    }
}

/// Handle Enter by queueing a paste for clipboard entries and saved secrets.
fn handle_enter(state: &Rc<AppState>) {
    match *state.view.borrow() {
        AppView::Clipboard => {
            if let Some(entry) = current_entry(state)
                && let Err(err) = queue_entry_copy(state, entry.id, AfterCopy::PasteNow)
            {
                set_footer(state, &format!("Paste failed: {err:#}"));
            }
        }
        AppView::Secrets => {
            if let Some(secret) = current_secret(state)
                && let Err(err) = queue_secret_copy(state, &secret, AfterCopy::PasteNow)
            {
                set_footer(state, &format!("Copy failed: {err:#}"));
            }
        }
    }
}

fn handle_copy(state: &Rc<AppState>) {
    match *state.view.borrow() {
        AppView::Clipboard => {
            if let Some(entry) = current_entry(state)
                && let Err(err) = queue_entry_copy(state, entry.id, AfterCopy::REPORT_ENTRY)
            {
                set_footer(state, &format!("Copy failed: {err:#}"));
            }
        }
        AppView::Secrets => {
            if let Some(secret) = current_secret(state)
                && let Err(err) = queue_secret_copy(state, &secret, AfterCopy::REPORT_SECRET)
            {
                set_footer(state, &format!("Copy failed: {err:#}"));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use rsclip_core::Database;
    use rsclip_core::models::{EntryFilter, NewEntry, SortMode};

    use super::{SEARCH_DEBOUNCE, start_workers};
    use crate::components::topbar::SEARCH_ENTRY_DELAY_MS;
    use crate::state::{AppView, ListRequest, ListResults, WorkerRequest, WriteOp};

    fn list_request(filter: EntryFilter) -> ListRequest {
        ListRequest {
            generation: 1,
            view: AppView::Clipboard,
            query: String::new(),
            filter,
            sort: SortMode::Default,
            row_limit: 120,
            requested_start: 0,
            selected_index: 0,
            preserve_scroll: false,
            known_total: None,
        }
    }

    /// Writes moved off the GTK thread must still land before the refresh
    /// queued after them, or pin/delete would render stale lists.
    #[test]
    fn worker_applies_writes_before_following_list_request() {
        let path = std::env::temp_dir().join(format!(
            "rsclip-ui-worker-test-{}-{:?}.sqlite",
            std::process::id(),
            std::time::SystemTime::now(),
        ));
        let db = Database::open(&path).unwrap();
        let mut entry = NewEntry::new("worker-hash".into(), "text/plain".into(), "note".into());
        entry.text_content = Some("note".into());
        let id = db.upsert_entry(&entry).unwrap();
        drop(db);

        let workers = start_workers(path.clone()).unwrap();
        let (request_tx, response_rx) = (workers.list_request_tx, workers.response_rx);
        request_tx
            .send(WorkerRequest::Write(WriteOp::SetPinned {
                id,
                pinned: true,
            }))
            .unwrap();
        request_tx
            .send(WorkerRequest::List(list_request(EntryFilter::Pinned)))
            .unwrap();

        let response = response_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(response.write_error.is_none());
        match response.list.unwrap().result.unwrap() {
            ListResults::Clipboard { total, entries, .. } => {
                assert_eq!(total, 1);
                assert_eq!(entries[0].id, id);
                assert!(entries[0].pinned);
            }
            ListResults::Secrets { .. } => panic!("expected clipboard results"),
        }

        drop(request_tx);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
        let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
    }

    /// Keystroke-to-query delay: GtkSearchEntry's own delay plus our debounce.
    /// It once stacked to ~310 ms (150 + 160), which read as laggy search;
    /// queries themselves take a few milliseconds.
    #[test]
    fn perf_search_input_latency_budget() {
        let latency = Duration::from_millis(SEARCH_ENTRY_DELAY_MS.into()) + SEARCH_DEBOUNCE;
        assert!(
            latency <= Duration::from_millis(100),
            "search waits {latency:?} after the last keystroke"
        );
    }
}
