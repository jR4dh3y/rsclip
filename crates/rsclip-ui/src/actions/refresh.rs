use std::rc::Rc;

use anyhow::Result;
use gtk4::prelude::*;

use crate::actions::set_footer;
use crate::components::labels::muted_label;
use crate::components::list::{entry_row, secret_row};
use crate::components::preview::{render_preview, render_secret_preview};
use crate::state::{
    AppState, AppView, current_entry_index, current_secret_index, row_index_for_entry,
    row_index_for_secret,
};

const ESTIMATED_ROW_HEIGHT: f64 = 48.0;
const MIN_VISIBLE_ROWS: usize = 20;
const WINDOW_PADDING_ROWS: usize = 50;

pub(crate) fn refresh_entries(state: &Rc<AppState>) -> Result<()> {
    load_window_from_start(state, 0, 0, false)
}

pub(crate) fn refresh_entries_if_changed(state: &Rc<AppState>) -> Result<()> {
    let selected_index = selected_index(state).unwrap_or(visible_first_index(state));
    load_window_around(state, selected_index, true)
}

pub(crate) fn rerender_current_list(state: &Rc<AppState>) {
    let selected_index = selected_index(state).unwrap_or(visible_first_index(state));
    match *state.view.borrow() {
        AppView::Clipboard => render_clipboard_window(state, Some(selected_index), true),
        AppView::Secrets => render_secrets_window(state, Some(selected_index), true),
    }
}

pub(crate) fn refresh_window_for_scroll(state: &Rc<AppState>) -> Result<()> {
    if state.virtual_list_update.get() {
        return Ok(());
    }

    let total = current_total(state);
    if total == 0 {
        return Ok(());
    }

    let first_visible = visible_first_index(state).min(total.saturating_sub(1));
    let visible_rows = visible_row_count(state);
    let current_start = current_start(state);
    let current_len = current_len(state);
    let desired_start = first_visible.saturating_sub(WINDOW_PADDING_ROWS);

    let needs_reload = current_len == 0
        || first_visible < current_start
        || first_visible.saturating_add(visible_rows) > current_start.saturating_add(current_len)
        || desired_start.abs_diff(current_start) >= WINDOW_PADDING_ROWS / 2;

    if !needs_reload {
        return Ok(());
    }

    let window_len = window_row_count(state, total);
    let normalized_start = desired_start.min(total.saturating_sub(window_len));
    let normalized_end = normalized_start.saturating_add(window_len);
    let selected_index = selected_index(state)
        .filter(|index| *index >= normalized_start && *index < normalized_end)
        .unwrap_or(first_visible);
    load_window_from_start(state, normalized_start, selected_index, true)
}

pub(crate) fn ensure_row_rendered(state: &Rc<AppState>, index: usize) -> Result<()> {
    let current_start = current_start(state);
    let current_len = current_len(state);
    if index >= current_start && index < current_start.saturating_add(current_len) {
        return Ok(());
    }

    load_window_around(state, index, true)
}

fn load_window_around(
    state: &Rc<AppState>,
    selected_index: usize,
    preserve_scroll: bool,
) -> Result<()> {
    let start = selected_index.saturating_sub(WINDOW_PADDING_ROWS);
    load_window_from_start(state, start, selected_index, preserve_scroll)
}

fn load_window_from_start(
    state: &Rc<AppState>,
    requested_start: usize,
    selected_index: usize,
    preserve_scroll: bool,
) -> Result<()> {
    match *state.view.borrow() {
        AppView::Clipboard => {
            load_clipboard_window(state, requested_start, selected_index, preserve_scroll)
        }
        AppView::Secrets => {
            load_secrets_window(state, requested_start, selected_index, preserve_scroll)
        }
    }
}

fn load_clipboard_window(
    state: &Rc<AppState>,
    requested_start: usize,
    selected_index: usize,
    preserve_scroll: bool,
) -> Result<()> {
    let query = state.query.borrow().clone();
    let filter = *state.filter.borrow();
    let sort = *state.sort.borrow();
    let total = state
        .db
        .count_entries(&query, filter)?
        .min(state.history_limit.get());
    let window_len = window_row_count(state, total);
    let start = requested_start.min(total.saturating_sub(window_len));
    let entries = if total == 0 {
        Vec::new()
    } else {
        state
            .db
            .list_entries_page(&query, filter, sort, window_len, start)?
    };

    state.secrets.borrow_mut().clear();
    state.secrets_start.set(0);
    state.secrets_total.set(0);
    *state.entries.borrow_mut() = entries;
    state.entries_start.set(start);
    state.entries_total.set(total);

    render_clipboard_window(state, Some(selected_index), preserve_scroll);
    Ok(())
}

fn load_secrets_window(
    state: &Rc<AppState>,
    requested_start: usize,
    selected_index: usize,
    preserve_scroll: bool,
) -> Result<()> {
    let query = state.query.borrow().clone();
    let total = state.db.count_secrets(&query)?.min(state.history_limit.get());
    let window_len = window_row_count(state, total);
    let start = requested_start.min(total.saturating_sub(window_len));
    let secrets = if total == 0 {
        Vec::new()
    } else {
        state.db.list_secrets_page(&query, window_len, start)?
    };

    state.entries.borrow_mut().clear();
    state.entries_start.set(0);
    state.entries_total.set(0);
    *state.secrets.borrow_mut() = secrets;
    state.secrets_start.set(start);
    state.secrets_total.set(total);

    render_secrets_window(state, Some(selected_index), preserve_scroll);
    Ok(())
}

fn render_clipboard_window(
    state: &Rc<AppState>,
    selected_index: Option<usize>,
    preserve_scroll: bool,
) {
    let scroll_value = state.list_adjustment.value();
    state.virtual_list_update.set(true);
    crate::components::clear_list(&state.list);

    let start = state.entries_start.get();
    let total = state.entries_total.get();
    append_top_spacer(state, start);

    for entry in state.entries.borrow().iter() {
        state
            .list
            .append(&entry_row(entry, &state.favicon_icon_dir));
    }

    let loaded_end = start.saturating_add(state.entries.borrow().len());
    append_bottom_spacer(state, total.saturating_sub(loaded_end));

    select_clipboard_row(state, selected_index);
    update_clipboard_count(state);
    update_clipboard_footer(state);
    restore_scroll_position(state, scroll_value, preserve_scroll);
    state.virtual_list_update.set(false);
}

fn render_secrets_window(
    state: &Rc<AppState>,
    selected_index: Option<usize>,
    preserve_scroll: bool,
) {
    let scroll_value = state.list_adjustment.value();
    state.virtual_list_update.set(true);
    crate::components::clear_list(&state.list);

    let start = state.secrets_start.get();
    let total = state.secrets_total.get();
    append_top_spacer(state, start);

    for secret in state.secrets.borrow().iter() {
        state.list.append(&secret_row(secret));
    }

    let loaded_end = start.saturating_add(state.secrets.borrow().len());
    append_bottom_spacer(state, total.saturating_sub(loaded_end));

    select_secret_row(state, selected_index);
    update_secret_count(state);
    update_secret_footer(state);
    restore_scroll_position(state, scroll_value, preserve_scroll);
    state.virtual_list_update.set(false);
}

fn select_clipboard_row(state: &Rc<AppState>, selected_index: Option<usize>) {
    let total = state.entries_total.get();
    if total == 0 {
        crate::components::clear_box(&state.preview);
        crate::components::clear_box(&state.details);
        state
            .preview
            .append(&muted_label("No clipboard entries yet"));
        return;
    }
    if state.entries.borrow().is_empty() {
        crate::components::clear_box(&state.preview);
        crate::components::clear_box(&state.details);
        return;
    }

    let start = state.entries_start.get();
    let end = start.saturating_add(state.entries.borrow().len());
    let selected_index = selected_index.unwrap_or(start).min(total.saturating_sub(1));
    let selected_index = selected_index.clamp(start, end.saturating_sub(1));
    if let Some(row_index) = row_index_for_entry(state, selected_index)
        && let Some(row) = state.list.row_at_index(row_index)
    {
        state.list.select_row(Some(&row));
        let relative_index = selected_index.saturating_sub(state.entries_start.get());
        if let Some(entry) = state.entries.borrow().get(relative_index) {
            render_preview(state, entry);
        }
    }
}

fn select_secret_row(state: &Rc<AppState>, selected_index: Option<usize>) {
    let total = state.secrets_total.get();
    if total == 0 {
        crate::components::clear_box(&state.preview);
        crate::components::clear_box(&state.details);
        state.preview.append(&muted_label("No secrets saved yet"));
        return;
    }
    if state.secrets.borrow().is_empty() {
        crate::components::clear_box(&state.preview);
        crate::components::clear_box(&state.details);
        return;
    }

    let start = state.secrets_start.get();
    let end = start.saturating_add(state.secrets.borrow().len());
    let selected_index = selected_index.unwrap_or(start).min(total.saturating_sub(1));
    let selected_index = selected_index.clamp(start, end.saturating_sub(1));
    if let Some(row_index) = row_index_for_secret(state, selected_index)
        && let Some(row) = state.list.row_at_index(row_index)
    {
        state.list.select_row(Some(&row));
        let relative_index = selected_index.saturating_sub(state.secrets_start.get());
        if let Some(secret) = state.secrets.borrow().get(relative_index) {
            render_secret_preview(state, secret);
        }
    }
}

fn append_top_spacer(state: &Rc<AppState>, rows: usize) {
    if rows > 0 {
        state.list.append(&spacer_row(rows));
    }
}

fn append_bottom_spacer(state: &Rc<AppState>, rows: usize) {
    if rows > 0 {
        state.list.append(&spacer_row(rows));
    }
}

fn spacer_row(rows: usize) -> gtk4::ListBoxRow {
    let row = gtk4::ListBoxRow::new();
    row.add_css_class("virtual-spacer-row");
    row.set_selectable(false);
    row.set_activatable(false);

    let spacer = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    spacer.set_height_request(spacer_height(rows));
    row.set_child(Some(&spacer));
    row
}

fn spacer_height(rows: usize) -> i32 {
    (rows as f64 * ESTIMATED_ROW_HEIGHT)
        .round()
        .clamp(0.0, i32::MAX as f64) as i32
}

fn restore_scroll_position(state: &Rc<AppState>, scroll_value: f64, preserve_scroll: bool) {
    if preserve_scroll {
        let max_value = (state.list_adjustment.upper() - state.list_adjustment.page_size())
            .max(state.list_adjustment.lower());
        state
            .list_adjustment
            .set_value(scroll_value.clamp(state.list_adjustment.lower(), max_value));
    } else {
        state.list_adjustment.set_value(0.0);
    }
}

fn visible_first_index(state: &Rc<AppState>) -> usize {
    (state.list_adjustment.value() / ESTIMATED_ROW_HEIGHT)
        .floor()
        .max(0.0) as usize
}

fn visible_row_count(state: &Rc<AppState>) -> usize {
    let page_size = state.list_adjustment.page_size();
    if page_size <= 0.0 {
        return MIN_VISIBLE_ROWS;
    }

    ((page_size / ESTIMATED_ROW_HEIGHT).ceil() as usize)
        .saturating_add(1)
        .max(MIN_VISIBLE_ROWS)
}

fn window_row_count(state: &Rc<AppState>, total: usize) -> usize {
    if total == 0 {
        return 0;
    }

    visible_row_count(state)
        .saturating_add(WINDOW_PADDING_ROWS * 2)
        .min(total)
}

fn selected_index(state: &Rc<AppState>) -> Option<usize> {
    match *state.view.borrow() {
        AppView::Clipboard => current_entry_index(state),
        AppView::Secrets => current_secret_index(state),
    }
}

fn current_total(state: &Rc<AppState>) -> usize {
    match *state.view.borrow() {
        AppView::Clipboard => state.entries_total.get(),
        AppView::Secrets => state.secrets_total.get(),
    }
}

fn current_start(state: &Rc<AppState>) -> usize {
    match *state.view.borrow() {
        AppView::Clipboard => state.entries_start.get(),
        AppView::Secrets => state.secrets_start.get(),
    }
}

fn current_len(state: &Rc<AppState>) -> usize {
    match *state.view.borrow() {
        AppView::Clipboard => state.entries.borrow().len(),
        AppView::Secrets => state.secrets.borrow().len(),
    }
}

fn update_clipboard_count(state: &Rc<AppState>) {
    state
        .count_label
        .set_text(&format!("Entries {}", state.entries_total.get()));
}

fn update_secret_count(state: &Rc<AppState>) {
    state
        .count_label
        .set_text(&format!("Secrets {}", state.secrets_total.get()));
}

fn update_clipboard_footer(state: &Rc<AppState>) {
    if state.show_footer_hints.get() {
        set_footer(
            state,
            "Tab: switch tab | Enter: paste | Ctrl+C: copy | Ctrl+S: secret | Ctrl+P: pin | Ctrl+D: delete | Esc: close",
        );
    } else {
        set_footer(state, "");
    }
}

fn update_secret_footer(state: &Rc<AppState>) {
    if state.show_footer_hints.get() {
        set_footer(
            state,
            "Tab: switch tab | Enter: copy | Ctrl+C: copy | Ctrl+E: rename | Ctrl+D: delete | Esc: close",
        );
    } else {
        set_footer(state, "");
    }
}
