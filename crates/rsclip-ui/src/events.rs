use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc;
use std::time::Duration;

use anyhow::{Context, Result};
use gtk::gdk;
use gtk::prelude::*;
use gtk4 as gtk;
use rsclip_core::Database;
use rsclip_core::models::{EntryFilter, EntryKind};

use crate::actions::clipboard::{copy_secret, copy_selected_entry};
use crate::actions::ocr::run_ocr_for_entry;
use crate::actions::refresh::{
    apply_clipboard_search_results, apply_secret_search_results, refresh_entries,
    refresh_window_for_scroll, search_window_row_count,
};
use crate::actions::secrets::{
    delete_current, rename_current_secret_dialog, save_current_as_secret_dialog, toggle_pin,
};
use crate::actions::selection::{mark_selected_row, move_selection};
use crate::actions::{set_footer, update_mode_controls};
use crate::components::preview::{render_preview, render_secret_preview};
use crate::state::{
    AppState, AppView, ListRequest, ListResponse, ListResults, current_entry, current_full_entry,
    current_secret, entry_at_row, full_entry_at_row, secret_at_row,
};

/// Delay before applying search so typing does not block the UI on every keystroke.
const SEARCH_DEBOUNCE: Duration = Duration::from_millis(120);
const LIST_RESULT_POLL: Duration = Duration::from_millis(16);

pub(crate) fn start_list_worker(
    db_path: std::path::PathBuf,
) -> Result<(mpsc::Sender<ListRequest>, mpsc::Receiver<ListResponse>)> {
    let (request_tx, request_rx) = mpsc::channel();
    let (response_tx, response_rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("rsclip-list".to_string())
        .spawn(move || list_worker(db_path, request_rx, response_tx))
        .context("starting list worker")?;
    Ok((request_tx, response_rx))
}

pub(crate) fn connect(state: &Rc<AppState>, window: &gtk::ApplicationWindow) {
    connect_mode_buttons(state);
    connect_ocr_button(state);
    connect_search(state);
    connect_filter(state);
    connect_lazy_scroll(state);
    connect_list_selection(state);
    connect_list_activation(state, window);
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
                history_limit: state.history_limit.get(),
                row_limit: search_window_row_count(&state),
                requested_start: 0,
                selected_index: 0,
                preserve_scroll: false,
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

/// Queue one list query on the persistent worker and watch for its response.
pub(crate) fn queue_list(state: &Rc<AppState>, request: ListRequest) -> Result<()> {
    let generation = request.generation;
    state
        .list_request_tx
        .send(request)
        .map_err(|_| anyhow::anyhow!("list worker stopped"))?;
    start_list_response_poll(state, generation);
    Ok(())
}

/// Poll for one expected worker response only while a list query is in flight.
fn start_list_response_poll(state: &Rc<AppState>, expected_generation: u64) {
    if let Some(source_id) = state.list_response_poll.borrow_mut().take() {
        source_id.remove();
    }

    let state = Rc::clone(state);
    let poll_state = Rc::clone(&state);
    let source_id = gtk::glib::timeout_add_local(LIST_RESULT_POLL, move || {
        let mut received_expected = false;
        while let Ok(response) = poll_state.list_response_rx.try_recv() {
            received_expected |= response.request.generation == expected_generation;
            apply_list_response(&poll_state, response);
        }

        if received_expected {
            let _ = poll_state.list_response_poll.borrow_mut().take();
            gtk::glib::ControlFlow::Break
        } else {
            gtk::glib::ControlFlow::Continue
        }
    });
    *state.list_response_poll.borrow_mut() = Some(source_id);
}

/// Own a dedicated SQLite connection and coalesce queued list requests.
fn list_worker(
    db_path: std::path::PathBuf,
    request_rx: mpsc::Receiver<ListRequest>,
    response_tx: mpsc::Sender<ListResponse>,
) {
    let db = match Database::open(db_path) {
        Ok(db) => db,
        Err(err) => {
            for request in request_rx {
                if response_tx
                    .send(ListResponse {
                        request,
                        result: Err(format!("{err:#}")),
                    })
                    .is_err()
                {
                    break;
                }
            }
            return;
        }
    };

    while let Ok(mut request) = request_rx.recv() {
        // If typing produced several requests before SQLite became available, only
        // execute the newest one. An in-flight older result is rejected in GTK.
        while let Ok(newer) = request_rx.try_recv() {
            request = newer;
        }
        let result = load_list(&db, &request).map_err(|err| format!("{err:#}"));
        if response_tx.send(ListResponse { request, result }).is_err() {
            break;
        }
    }
}

/// Load one list window without accessing GTK-owned state.
fn load_list(db: &Database, request: &ListRequest) -> anyhow::Result<ListResults> {
    Ok(match request.view {
        AppView::Clipboard => {
            let total = db
                .count_entries(&request.query, request.filter)?
                .min(request.history_limit);
            let window_len = request.row_limit.min(total);
            let start = request
                .requested_start
                .min(total.saturating_sub(window_len));
            let entries = if total == 0 || window_len == 0 {
                Vec::new()
            } else {
                db.list_entry_summaries_page(
                    &request.query,
                    request.filter,
                    request.sort,
                    window_len,
                    start,
                )?
            };
            ListResults::Clipboard {
                total,
                start,
                entries,
            }
        }
        AppView::Secrets => {
            let total = db.count_secrets(&request.query)?.min(request.history_limit);
            let window_len = request.row_limit.min(total);
            let start = request
                .requested_start
                .min(total.saturating_sub(window_len));
            let secrets = if total == 0 || window_len == 0 {
                Vec::new()
            } else {
                db.list_secrets_page(&request.query, window_len, start)?
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
            1 => EntryFilter::Text,
            2 => EntryFilter::Images,
            3 => EntryFilter::Files,
            4 => EntryFilter::Links,
            5 => EntryFilter::Colors,
            6 => EntryFilter::Pinned,
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
    list.connect_row_selected(move |list, row| {
        mark_selected_row(list, row);
        if let Some(row) = row {
            let index = row.index();
            if index >= 0 {
                match *state.view.borrow() {
                    AppView::Clipboard => {
                        if let Some(entry) = entry_at_row(&state, row) {
                            render_preview(&state, &entry);
                        }
                    }
                    AppView::Secrets => {
                        if let Some(secret) = secret_at_row(&state, row) {
                            render_secret_preview(&state, &secret);
                        }
                    }
                }
            }
        }
    });
}

fn connect_list_activation(state: &Rc<AppState>, window: &gtk::ApplicationWindow) {
    let list = state.list.clone();
    let state = Rc::clone(state);
    let window = window.clone();
    list.connect_row_activated(move |_, row| match *state.view.borrow() {
        AppView::Clipboard => {
            if let Some(entry) = full_entry_at_row(&state, row) {
                if let Err(err) = copy_selected_entry(&state, &entry) {
                    set_footer(&state, &format!("Paste failed: {err:#}"));
                    return;
                }
                crate::window::close_overlay_and_paste(&state, &window);
            }
        }
        AppView::Secrets => {
            if let Some(secret) = secret_at_row(&state, row) {
                if let Err(err) = copy_secret(&state, &secret) {
                    set_footer(&state, &format!("Copy failed: {err:#}"));
                    return;
                }
                crate::window::hide_overlay(&state, &window);
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
                    handle_enter(&state, &window);
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
                            if let Some(secret) = current_secret(&state) {
                                if let Err(err) = copy_secret(&state, &secret) {
                                    set_footer(&state, &format!("Copy failed: {err:#}"));
                                } else {
                                    set_footer(&state, "Copied secret");
                                }
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

fn handle_enter(state: &Rc<AppState>, window: &gtk::ApplicationWindow) {
    match *state.view.borrow() {
        AppView::Clipboard => {
            if let Some(entry) = current_full_entry(state) {
                if let Err(err) = copy_selected_entry(state, &entry) {
                    set_footer(state, &format!("Paste failed: {err:#}"));
                } else {
                    crate::window::close_overlay_and_paste(state, window);
                }
            }
        }
        AppView::Secrets => {
            if let Some(secret) = current_secret(state) {
                if let Err(err) = copy_secret(state, &secret) {
                    set_footer(state, &format!("Copy failed: {err:#}"));
                } else {
                    crate::window::hide_overlay(state, window);
                }
            }
        }
    }
}

fn handle_copy(state: &Rc<AppState>) {
    match *state.view.borrow() {
        AppView::Clipboard => {
            if let Some(entry) = current_full_entry(state) {
                if let Err(err) = copy_selected_entry(state, &entry) {
                    set_footer(state, &format!("Copy failed: {err:#}"));
                } else {
                    set_footer(state, "Copied selected entry");
                }
            }
        }
        AppView::Secrets => {
            if let Some(secret) = current_secret(state) {
                if let Err(err) = copy_secret(state, &secret) {
                    set_footer(state, &format!("Copy failed: {err:#}"));
                } else {
                    set_footer(state, "Copied secret");
                }
            }
        }
    }
}
