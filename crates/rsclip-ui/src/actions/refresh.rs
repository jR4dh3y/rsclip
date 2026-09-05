use std::rc::Rc;

use anyhow::Result;
use gtk4::prelude::*;
use rsclip_core::models::{ClipboardEntry, SecretEntry};

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
// Keep one fallback viewport on each side without rebuilding 100 off-screen rows.
const WINDOW_PADDING_ROWS: usize = MIN_VISIBLE_ROWS;

pub(crate) fn refresh_entries(state: &Rc<AppState>) -> Result<()> {
    let generation = crate::state::advance_list_generation(state);
    queue_window(state, generation, 0, 0, false)
}

pub(crate) fn refresh_entries_if_changed(state: &Rc<AppState>) -> Result<()> {
    let selected_index = selected_index(state).unwrap_or(visible_first_index(state));
    let generation = crate::state::advance_list_generation(state);
    queue_window(
        state,
        generation,
        selected_index.saturating_sub(WINDOW_PADDING_ROWS),
        selected_index,
        true,
    )
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
    let normalized_start = normalized_window_start(desired_start, total, window_len);
    let normalized_end = normalized_start.saturating_add(window_len);
    let selected_index = selected_index(state)
        .filter(|index| *index >= normalized_start && *index < normalized_end)
        .unwrap_or(first_visible);
    let generation = crate::state::advance_list_generation(state);
    queue_window(state, generation, normalized_start, selected_index, true)
}

pub(crate) fn ensure_row_rendered(state: &Rc<AppState>, index: usize) -> Result<()> {
    let current_start = current_start(state);
    let current_len = current_len(state);
    if index >= current_start && index < current_start.saturating_add(current_len) {
        return Ok(());
    }

    let generation = crate::state::advance_list_generation(state);
    queue_window(
        state,
        generation,
        index.saturating_sub(WINDOW_PADDING_ROWS),
        index,
        true,
    )
}

fn queue_window(
    state: &Rc<AppState>,
    generation: u64,
    requested_start: usize,
    selected_index: usize,
    preserve_scroll: bool,
) -> Result<()> {
    let query = state.query.borrow().clone();
    let filter = *state.filter.borrow();
    let sort = *state.sort.borrow();
    let requested_start = if current_total(state) == 0 {
        requested_start
    } else {
        normalized_window_start(
            requested_start,
            current_total(state),
            window_row_count(state, current_total(state)),
        )
    };
    crate::events::queue_list(
        state,
        crate::state::ListRequest {
            generation,
            view: *state.view.borrow(),
            query,
            filter,
            sort,
            history_limit: state.history_limit.get(),
            row_limit: search_window_row_count(state),
            requested_start,
            selected_index,
            preserve_scroll,
        },
    )
}

/// Replace the visible clipboard window with a completed worker result.
pub(crate) fn apply_clipboard_search_results(
    state: &Rc<AppState>,
    request: &crate::state::ListRequest,
    total: usize,
    start: usize,
    entries: Vec<ClipboardEntry>,
) {
    state.secrets.borrow_mut().clear();
    state.secrets_start.set(0);
    state.secrets_total.set(0);
    *state.entries.borrow_mut() = entries;
    state.entries_start.set(start);
    state.entries_total.set(total);
    render_clipboard_window(state, Some(request.selected_index), request.preserve_scroll);
}

/// Replace the visible secrets window with a completed worker result.
pub(crate) fn apply_secret_search_results(
    state: &Rc<AppState>,
    request: &crate::state::ListRequest,
    total: usize,
    start: usize,
    secrets: Vec<SecretEntry>,
) {
    state.entries.borrow_mut().clear();
    state.entries_start.set(0);
    state.entries_total.set(0);
    *state.secrets.borrow_mut() = secrets;
    state.secrets_start.set(start);
    state.secrets_total.set(total);
    render_secrets_window(state, Some(request.selected_index), request.preserve_scroll);
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
    restore_scroll_position(
        state,
        scroll_value,
        clamped_window_index(state, selected_index),
        preserve_scroll,
    );
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
    restore_scroll_position(
        state,
        scroll_value,
        clamped_window_index(state, selected_index),
        preserve_scroll,
    );
    state.virtual_list_update.set(false);
}

/// Clamp a requested index into the currently rendered window.
fn clamped_window_index(state: &Rc<AppState>, selected_index: Option<usize>) -> Option<usize> {
    clamp_window_index(
        current_start(state),
        current_len(state),
        current_total(state),
        selected_index,
    )
}

fn clamp_window_index(
    start: usize,
    len: usize,
    total: usize,
    selected_index: Option<usize>,
) -> Option<usize> {
    if total == 0 || len == 0 {
        return None;
    }

    // Intersect the rendered window with [0, total-1]; a stale start beyond
    // total (e.g. after a filter shrink) falls back to the last row instead
    // of panicking on an empty clamp range.
    let last = total.saturating_sub(1);
    let end = start.saturating_add(len);
    let lo = start.min(last);
    let hi = end.saturating_sub(1).min(last);
    let (lo, hi) = if lo <= hi { (lo, hi) } else { (last, last) };
    Some(selected_index.unwrap_or(lo).min(last).clamp(lo, hi))
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
    let len = state.entries.borrow().len();
    let end = start.saturating_add(len);
    let last = total.saturating_sub(1);
    let lo = start.min(last);
    let hi = end.saturating_sub(1).min(last);
    let (lo, hi) = if lo <= hi { (lo, hi) } else { (last, last) };
    let selected_index = selected_index.unwrap_or(lo).min(last).clamp(lo, hi);
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
    let len = state.secrets.borrow().len();
    let end = start.saturating_add(len);
    let last = total.saturating_sub(1);
    let lo = start.min(last);
    let hi = end.saturating_sub(1).min(last);
    let (lo, hi) = if lo <= hi { (lo, hi) } else { (last, last) };
    let selected_index = selected_index.unwrap_or(lo).min(last).clamp(lo, hi);
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
    // Spacers can span the whole viewport; keep pointer/focus state from
    // painting row styles (hover background) across the list.
    row.set_can_target(false);
    row.set_focusable(false);

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

/// Restore the scroll position after a window rebuild.
///
/// Spacers are sized from [`ESTIMATED_ROW_HEIGHT`] while real rows differ,
/// so a restored raw offset drifts with window depth and can land inside a
/// spacer, leaving the viewport empty. Instead, check the selected row's
/// estimated position against the restored viewport and re-anchor the scroll
/// to that row when it falls outside, clamped against the freshly computed
/// content size rather than the not-yet-updated adjustment.
fn restore_scroll_position(
    state: &Rc<AppState>,
    scroll_value: f64,
    selected_index: Option<usize>,
    preserve_scroll: bool,
) {
    if !preserve_scroll {
        state.list_adjustment.set_value(0.0);
        return;
    }

    let start = current_start(state);
    let len = current_len(state);
    let total = current_total(state);
    let page_size = state.list_adjustment.page_size();

    let content_height = f64::from(spacer_height(start))
        + len as f64 * ESTIMATED_ROW_HEIGHT
        + f64::from(spacer_height(
            total.saturating_sub(start.saturating_add(len)),
        ));
    let max_value = (content_height - page_size).max(0.0);

    let Some(index) = selected_index else {
        state
            .list_adjustment
            .set_value(scroll_value.clamp(0.0, max_value));
        return;
    };

    let row_top =
        f64::from(spacer_height(start)) + index.saturating_sub(start) as f64 * ESTIMATED_ROW_HEIGHT;
    let row_bottom = row_top + ESTIMATED_ROW_HEIGHT;
    let viewport_top = scroll_value;
    let viewport_bottom = scroll_value + page_size;

    let value = if row_top >= viewport_top && row_bottom <= viewport_bottom {
        scroll_value
    } else {
        (row_top - page_size * 0.25).max(0.0)
    };
    state.list_adjustment.set_value(value.min(max_value));
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

    window_row_count_for(visible_row_count(state), total)
}

/// Calculate the number of rows requested by the background list worker.
pub(crate) fn search_window_row_count(state: &Rc<AppState>) -> usize {
    window_row_count_for(visible_row_count(state), usize::MAX)
}

fn window_row_count_for(visible_rows: usize, total: usize) -> usize {
    visible_rows
        .saturating_add(WINDOW_PADDING_ROWS * 2)
        .min(total)
}

fn normalized_window_start(requested_start: usize, total: usize, window_len: usize) -> usize {
    requested_start.min(total.saturating_sub(window_len))
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

#[cfg(test)]
mod tests {
    use super::{clamp_window_index, normalized_window_start, window_row_count_for};

    #[test]
    fn result_window_is_bounded_for_large_history() {
        assert_eq!(window_row_count_for(20, usize::MAX), 60);
    }

    #[test]
    fn result_window_never_exceeds_total() {
        assert_eq!(window_row_count_for(20, 12), 12);
        assert_eq!(window_row_count_for(20, 0), 0);
    }

    #[test]
    fn requested_start_is_clamped_to_last_full_window() {
        assert_eq!(normalized_window_start(900, 1_000, 60), 900);
        assert_eq!(normalized_window_start(999, 1_000, 60), 940);
        assert_eq!(normalized_window_start(1_975, 2_000, 60), 1_940);
        assert_eq!(normalized_window_start(900, 12, 12), 0);
    }

    #[test]
    fn window_index_clamps_into_rendered_window() {
        assert_eq!(clamp_window_index(100, 60, 1_000, Some(120)), Some(120));
        assert_eq!(clamp_window_index(100, 60, 1_000, Some(0)), Some(100));
        assert_eq!(clamp_window_index(100, 60, 1_000, Some(999)), Some(159));
        assert_eq!(clamp_window_index(100, 60, 1_000, None), Some(100));
        assert_eq!(clamp_window_index(0, 0, 1_000, Some(5)), None);
        assert_eq!(clamp_window_index(0, 60, 0, Some(5)), None);
    }

    #[test]
    fn window_index_handles_saturating_edge() {
        assert_eq!(
            clamp_window_index(usize::MAX, 10, usize::MAX, Some(usize::MAX)),
            Some(usize::MAX.saturating_sub(1))
        );
    }
}
