use std::rc::Rc;
use std::sync::atomic::Ordering;

use anyhow::Result;
use gtk4::prelude::*;
use rsclip_core::models::{ClipboardEntry, EntryData, SecretEntry};

use crate::actions::selection::schedule_preview_for_current_selection;
use crate::actions::set_footer;
use crate::components::labels::muted_label;
use crate::components::list::{entry_row, secret_row};
use crate::components::preview::clear_preview_state;
use crate::state::{
    AppState, AppView, RowKey, current_entry_index, current_secret_index, row_index_for_entry,
    row_index_for_secret,
};

const ESTIMATED_ROW_HEIGHT: f64 = 48.0;
const MIN_VISIBLE_ROWS: usize = 20;
// Keep two full viewports on each side so fast arrow navigation stays within the rendered window.
const WINDOW_PADDING_ROWS: usize = 50;

/// Queue a fresh list window and discard any selection from the previous query.
pub(crate) fn refresh_entries(state: &Rc<AppState>) -> Result<()> {
    state.pending_selection.set(None);
    refresh_entries_with_pending_selection(state)
}

/// Queue the first result window while retaining a pending selection from an overlay reopen.
pub(crate) fn refresh_entries_with_pending_selection(state: &Rc<AppState>) -> Result<()> {
    let generation = crate::state::advance_list_generation(state);
    queue_window(state, generation, 0, 0, false, None)
}

/// Put the currently cached list back at its first row when the overlay is
/// shown again. Returns `true` when the first row is not cached and a refresh
/// is needed to materialize it.
pub(crate) fn reset_list_position(state: &Rc<AppState>) -> bool {
    state
        .list_adjustment
        .set_value(state.list_adjustment.lower());

    let total = current_total(state);
    if total == 0 {
        state.pending_selection.set(None);
        return false;
    }

    let start = current_start(state);
    let len = current_len(state);
    if start > 0 || len == 0 {
        state.pending_selection.set(Some(0));
        return true;
    }

    state.pending_selection.set(None);
    let row_index = match *state.view.borrow() {
        AppView::Clipboard => row_index_for_entry(state, 0),
        AppView::Secrets => row_index_for_secret(state, 0),
    };
    if let Some(row) = row_index.and_then(|index| state.list.row_at_index(index)) {
        state.list.select_row(Some(&row));
        schedule_preview_for_current_selection(state);
    }
    false
}

/// Refresh the current list window without moving the selected global index.
pub(crate) fn refresh_entries_preserving_selection(state: &Rc<AppState>) -> Result<()> {
    let selected_index = selected_index(state).unwrap_or(visible_first_index(state));
    refresh_entries_at_index(state, selected_index)
}

/// Load a window centered on `target_index` and preserve its requested selection.
pub(crate) fn refresh_entries_at_index(state: &Rc<AppState>, target_index: usize) -> Result<()> {
    state.pending_selection.set(None);
    let generation = crate::state::advance_list_generation(state);
    queue_window(
        state,
        generation,
        target_index.saturating_sub(WINDOW_PADDING_ROWS),
        target_index,
        true,
        None,
    )
}

pub(crate) use refresh_entries_preserving_selection as refresh_entries_if_changed;

pub(crate) fn current_selected_index(state: &Rc<AppState>) -> usize {
    selected_index(state).unwrap_or_else(|| visible_first_index(state))
}

pub(crate) fn rerender_current_list(state: &Rc<AppState>) {
    // Row content itself changed (favicons, config): rebuild every row.
    state.rendered_rows.borrow_mut().clear();
    let selected_index = selected_index(state).unwrap_or(visible_first_index(state));
    match *state.view.borrow() {
        AppView::Clipboard => render_clipboard_window(state, Some(selected_index), true),
        AppView::Secrets => render_secrets_window(state, Some(selected_index), true),
    }
}

/// Refresh the favicons of rendered link rows; no other row shows one.
///
/// Favicon fetches land while the overlay is open, and rebuilding the whole
/// ~120-row window per fetch cost ~35 ms of GTK time. Swapping only the link
/// rows' content keeps every row, its selection, and the scroll position.
pub(crate) fn rerender_link_rows(state: &Rc<AppState>) {
    if *state.view.borrow() != AppView::Clipboard {
        return;
    }
    let entries = state.entries.borrow();
    let start = state.entries_start.get();
    for (index, entry) in entries.iter().enumerate() {
        if !matches!(entry.data, EntryData::Link { .. }) {
            continue;
        }
        let Some(row) = row_index_for_entry(state, start + index)
            .and_then(|row_index| state.list.row_at_index(row_index))
        else {
            continue;
        };
        let fresh = entry_row(entry, &state.favicon_icon_dir);
        let content = fresh.child();
        fresh.set_child(None::<&gtk4::Widget>);
        row.set_child(content.as_ref());
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
    let Some(normalized_start) = scroll_reload_start(
        first_visible,
        visible_row_count(state),
        current_start(state),
        current_len(state),
        total,
    ) else {
        return Ok(());
    };

    let normalized_end = normalized_start.saturating_add(window_row_count(state, total));
    let selected_index = selected_index(state)
        .filter(|index| *index >= normalized_start && *index < normalized_end)
        .unwrap_or(first_visible);
    // Crucial: do not advance generation for smooth scroll window shifts.
    queue_window(
        state,
        state.list_generation.load(Ordering::Relaxed),
        normalized_start,
        selected_index,
        true,
        Some(total),
    )
}

/// Rows between the viewport and the loaded window's edge that trigger a
/// reload, so the next window arrives before the viewport runs out of rows.
const RELOAD_EDGE_ROWS: usize = 15;

/// Start of the window to load for a scroll position, or `None` while the
/// viewport is comfortably inside the loaded window `current_start..+len`.
fn scroll_reload_start(
    first_visible: usize,
    visible_rows: usize,
    current_start: usize,
    current_len: usize,
    total: usize,
) -> Option<usize> {
    let current_end = current_start.saturating_add(current_len);
    let near_top =
        first_visible < current_start.saturating_add(RELOAD_EDGE_ROWS) && current_start > 0;
    let near_bottom = first_visible
        .saturating_add(visible_rows)
        .saturating_add(RELOAD_EDGE_ROWS)
        > current_end
        && current_end < total;
    if current_len != 0 && !near_top && !near_bottom {
        return None;
    }

    let window_len = window_row_count_for(visible_rows, total);
    let start = normalized_window_start(
        first_visible.saturating_sub(WINDOW_PADDING_ROWS),
        total,
        window_len,
    );
    let already_loaded = start == current_start
        && current_len >= window_len.min(total.saturating_sub(current_start));
    (!already_loaded).then_some(start)
}

pub(crate) fn ensure_row_rendered(state: &Rc<AppState>, index: usize) -> Result<()> {
    let current_start = current_start(state);
    let current_len = current_len(state);
    if index >= current_start && index < current_start.saturating_add(current_len) {
        return Ok(());
    }

    // Do not advance generation: navigating past window edge should not invalidate
    // or discard in-flight list responses.
    queue_window(
        state,
        state.list_generation.load(Ordering::Relaxed),
        index.saturating_sub(WINDOW_PADDING_ROWS),
        index,
        true,
        Some(current_total(state)),
    )
}

fn queue_window(
    state: &Rc<AppState>,
    generation: u64,
    requested_start: usize,
    selected_index: usize,
    preserve_scroll: bool,
    known_total: Option<usize>,
) -> Result<()> {
    let query = state.query.borrow().clone();
    let filter = *state.filter.borrow();
    let sort = *state.sort.borrow();
    let (requested_start, mut row_limit) = fetch_window(
        requested_start,
        current_total(state),
        visible_row_count(state),
    );
    // First presentation needs one viewport and a little lookahead. Scroll
    // and selection reloads still use the full window to absorb key repeat.
    if !preserve_scroll && requested_start == 0 && selected_index == 0 {
        row_limit = search_window_row_count(state);
    }
    crate::events::queue_list(
        state,
        crate::state::ListRequest {
            generation,
            view: *state.view.borrow(),
            query,
            filter,
            sort,
            row_limit,
            requested_start,
            selected_index,
            preserve_scroll,
            known_total,
        },
    )
}

pub(crate) fn resolve_window_selection(
    live_selection: Option<usize>,
    request_selection: usize,
    window_start: usize,
    window_len: usize,
) -> Option<usize> {
    let window_end = window_start.saturating_add(window_len);
    live_selection
        .filter(|idx| *idx >= window_start && *idx < window_end)
        .or(Some(request_selection))
}

fn preferred_live_selection(
    pending_selection: Option<usize>,
    current_selection: Option<usize>,
) -> Option<usize> {
    pending_selection.or(current_selection)
}

/// Replace the visible clipboard window with a completed worker result.
pub(crate) fn apply_clipboard_search_results(
    state: &Rc<AppState>,
    request: &crate::state::ListRequest,
    total: usize,
    start: usize,
    entries: Vec<ClipboardEntry>,
) {
    let live_selection =
        preferred_live_selection(state.pending_selection.take(), current_entry_index(state));
    state.secrets.borrow_mut().clear();
    state.secrets_start.set(0);
    state.secrets_total.set(0);
    *state.entries.borrow_mut() = entries;
    state.entries_start.set(start);
    state.entries_total.set(total);
    let selected_index = resolve_window_selection(
        live_selection,
        request.selected_index,
        start,
        state.entries.borrow().len(),
    );
    render_clipboard_window(state, selected_index, request.preserve_scroll);
}

/// Replace the visible secrets window with a completed worker result.
pub(crate) fn apply_secret_search_results(
    state: &Rc<AppState>,
    request: &crate::state::ListRequest,
    total: usize,
    start: usize,
    secrets: Vec<SecretEntry>,
) {
    let live_selection =
        preferred_live_selection(state.pending_selection.take(), current_secret_index(state));
    state.entries.borrow_mut().clear();
    state.entries_start.set(0);
    state.entries_total.set(0);
    *state.secrets.borrow_mut() = secrets;
    state.secrets_start.set(start);
    state.secrets_total.set(total);
    let selected_index = resolve_window_selection(
        live_selection,
        request.selected_index,
        start,
        state.secrets.borrow().len(),
    );
    render_secrets_window(state, selected_index, request.preserve_scroll);
}

fn render_clipboard_window(
    state: &Rc<AppState>,
    selected_index: Option<usize>,
    preserve_scroll: bool,
) {
    let scroll_value = state.list_adjustment.value();
    state.virtual_list_update.set(true);

    {
        let entries = state.entries.borrow();
        let keys = entries.iter().map(RowKey::entry).collect();
        render_rows(
            state,
            state.entries_start.get(),
            state.entries_total.get(),
            keys,
            |index| entry_row(&entries[index], &state.favicon_icon_dir),
        );
    }

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

    {
        let secrets = state.secrets.borrow();
        let keys = secrets.iter().map(RowKey::secret).collect();
        render_rows(
            state,
            state.secrets_start.get(),
            state.secrets_total.get(),
            keys,
            |index| secret_row(&secrets[index]),
        );
    }

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
        clear_preview_state(state);
        state
            .preview
            .append(&muted_label("No clipboard entries yet"));
        return;
    }
    if state.entries.borrow().is_empty() {
        clear_preview_state(state);
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
        schedule_preview_for_current_selection(state);
    }
}

fn select_secret_row(state: &Rc<AppState>, selected_index: Option<usize>) {
    let total = state.secrets_total.get();
    if total == 0 {
        clear_preview_state(state);
        state.preview.append(&muted_label("No secrets saved yet"));
        return;
    }
    if state.secrets.borrow().is_empty() {
        clear_preview_state(state);
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
        schedule_preview_for_current_selection(state);
    }
}

/// Rows to drop from and add to each end of the rendered window so it shows
/// the new window, keeping the overlapping rows untouched.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RowShift {
    drop_front: usize,
    drop_back: usize,
    add_front: usize,
    add_back: usize,
}

/// Plan an in-place update when `new` is `old` slid up or down (scrolling,
/// key navigation, or an entry inserted at the top). `None` means the windows
/// share no contiguous run and the list must be rebuilt.
fn plan_row_shift(old: &[RowKey], new: &[RowKey]) -> Option<RowShift> {
    let (first_old, first_new) = (old.first()?, new.first()?);

    if let Some(skip) = old.iter().position(|key| key == first_new) {
        let overlap = (old.len() - skip).min(new.len());
        return (old[skip..skip + overlap] == new[..overlap]).then_some(RowShift {
            drop_front: skip,
            drop_back: old.len() - skip - overlap,
            add_front: 0,
            add_back: new.len() - overlap,
        });
    }

    let skip = new.iter().position(|key| key == first_old)?;
    let overlap = old.len().min(new.len() - skip);
    (new[skip..skip + overlap] == old[..overlap]).then_some(RowShift {
        drop_front: 0,
        drop_back: old.len() - overlap,
        add_front: skip,
        add_back: new.len() - skip - overlap,
    })
}

/// For each key in `new`, the index of the `old` row showing the same key.
fn plan_row_reuse(old: &[RowKey], new: &[RowKey]) -> Vec<Option<usize>> {
    let mut old_index: std::collections::HashMap<RowKey, usize> = old
        .iter()
        .enumerate()
        .map(|(index, key)| (*key, index))
        .collect();
    new.iter().map(|key| old_index.remove(key)).collect()
}

/// Show the rows for `keys` (window `start` of `total`) between virtual spacers.
///
/// A window shift used to destroy and rebuild all ~120 rows on the GTK
/// thread; rows still inside the new window are now kept, so a scroll reload
/// only builds the rows that entered it.
fn render_rows(
    state: &Rc<AppState>,
    start: usize,
    total: usize,
    keys: Vec<RowKey>,
    make_row: impl Fn(usize) -> gtk4::ListBoxRow,
) {
    let list = &state.list;
    let old = state.rendered_rows.replace(Vec::new());
    let is_spacer = |child: &gtk4::Widget| child.has_css_class("virtual-spacer-row");
    if let Some(child) = list.first_child().filter(is_spacer) {
        list.remove(&child);
    }
    if let Some(child) = list.last_child().filter(is_spacer) {
        list.remove(&child);
    }

    let shift = plan_row_shift(&old, &keys).filter(|_| list_len(list) == old.len());
    match shift {
        Some(shift) => {
            for _ in 0..shift.drop_front {
                if let Some(child) = list.first_child() {
                    list.remove(&child);
                }
            }
            for _ in 0..shift.drop_back {
                if let Some(child) = list.last_child() {
                    list.remove(&child);
                }
            }
            for index in (0..shift.add_front).rev() {
                list.prepend(&make_row(index));
            }
            for index in keys.len() - shift.add_back..keys.len() {
                list.append(&make_row(index));
            }
        }
        None => {
            // A new query rarely lines up with the old window, but typing
            // "ht" -> "htt" keeps most rows: reuse any row whose key is still
            // listed and build only the new ones.
            let mut old_rows = Vec::with_capacity(old.len());
            if list_len(list) == old.len() {
                let mut child = list.first_child();
                while let Some(widget) = child {
                    child = widget.next_sibling();
                    old_rows.push(widget.downcast::<gtk4::ListBoxRow>().ok());
                }
            }
            // Clear the selection first: a detached row keeps its selected
            // state flag and would render as selected when re-added.
            list.unselect_all();
            crate::components::clear_list(list);
            for (index, reuse) in plan_row_reuse(&old, &keys).into_iter().enumerate() {
                let row = reuse
                    .and_then(|old_index| old_rows.get_mut(old_index)?.take())
                    .unwrap_or_else(|| make_row(index));
                list.append(&row);
            }
        }
    }

    if start > 0 {
        list.prepend(&spacer_row(start));
    }
    let remaining = total.saturating_sub(start.saturating_add(keys.len()));
    if remaining > 0 {
        list.append(&spacer_row(remaining));
    }
    *state.rendered_rows.borrow_mut() = keys;
}

fn list_len(list: &gtk4::ListBox) -> usize {
    let mut len = 0;
    let mut child = list.first_child();
    while let Some(widget) = child {
        len += 1;
        child = widget.next_sibling();
    }
    len
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
    let max_upper = state.list_adjustment.upper().max(content_height);
    let max_value = (max_upper - page_size).max(0.0);

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

    let value = if row_top < viewport_top {
        row_top
    } else if row_bottom > viewport_bottom {
        (row_bottom - page_size).max(0.0)
    } else {
        scroll_value
    };
    state.list_adjustment.set_value(value.clamp(0.0, max_value));
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

/// Rows requested for the first page of a new search. It always starts at row
/// 0, so it only has to cover the viewport; scrolling then loads full windows.
pub(crate) fn search_window_row_count(state: &Rc<AppState>) -> usize {
    visible_row_count(state).saturating_add(20)
}

/// Start and row limit of a browsing window fetched for `requested_start`.
///
/// Callers request `target - WINDOW_PADDING_ROWS`, so the limit must span the
/// padding plus the viewport: a shorter limit (the search page size) loads a
/// window that ends above the target row, pinning key navigation in place.
fn fetch_window(requested_start: usize, total: usize, visible_rows: usize) -> (usize, usize) {
    let row_limit = window_row_count_for(visible_rows, usize::MAX);
    if total == 0 {
        return (requested_start, row_limit);
    }
    let start = normalized_window_start(requested_start, total, row_limit.min(total));
    (start, row_limit)
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
            "Tab: switch tab | Enter: paste | Ctrl+C: copy | Ctrl+E: rename | Ctrl+D: delete | Esc: close",
        );
    } else {
        set_footer(state, "");
    }
}

#[cfg(test)]
mod tests {
    use super::{
        RowShift, WINDOW_PADDING_ROWS, clamp_window_index, fetch_window, normalized_window_start,
        plan_row_reuse, plan_row_shift, preferred_live_selection, resolve_window_selection,
        scroll_reload_start, window_row_count_for,
    };
    use crate::state::RowKey;

    fn keys(ids: impl IntoIterator<Item = i64>) -> Vec<RowKey> {
        ids.into_iter().map(RowKey::for_test).collect()
    }

    /// Apply a plan the way `render_rows` edits the ListBox.
    fn apply(old: &[RowKey], new: &[RowKey], shift: RowShift) -> Vec<RowKey> {
        let mut rows = old[shift.drop_front..old.len() - shift.drop_back].to_vec();
        for index in (0..shift.add_front).rev() {
            rows.insert(0, new[index]);
        }
        rows.extend_from_slice(&new[new.len() - shift.add_back..]);
        rows
    }

    #[test]
    fn row_shift_reuses_overlap_when_scrolling() {
        let cases = [
            (keys(0..120), keys(35..155)),  // scroll down one reload step
            (keys(35..155), keys(0..120)),  // scroll up
            (keys(0..120), keys(0..120)),   // same window re-applied
            (keys(0..120), keys(60..100)),  // filtered/shrunk tail
            (keys(0..40), keys(0..120)),    // search page grown to a full window
            (keys(0..120), keys(119..239)), // one-row overlap
        ];
        for (old, new) in cases {
            let shift = plan_row_shift(&old, &new).expect("windows overlap");
            assert_eq!(apply(&old, &new, shift), new);
        }

        let shift = plan_row_shift(&keys(0..120), &keys(35..155)).unwrap();
        assert_eq!(
            shift,
            RowShift {
                drop_front: 35,
                drop_back: 0,
                add_front: 0,
                add_back: 35
            }
        );
    }

    #[test]
    fn row_shift_handles_entry_inserted_at_top() {
        let old = keys(1..121);
        let new: Vec<_> = keys([999]).into_iter().chain(keys(1..120)).collect();
        let shift = plan_row_shift(&old, &new).unwrap();
        assert_eq!((shift.add_front, shift.drop_back), (1, 1));
        assert_eq!(apply(&old, &new, shift), new);
    }

    #[test]
    fn rebuild_reuses_rows_still_listed_by_a_new_query() {
        // "ht" -> "htt": some results drop out, the rest reorder, one is new.
        let old = keys([10, 11, 12, 13]);
        let new = keys([12, 10, 99]);
        assert_eq!(plan_row_reuse(&old, &new), vec![Some(2), Some(0), None]);
        // A key is reused at most once.
        assert_eq!(
            plan_row_reuse(&keys([1]), &keys([1, 1])),
            vec![Some(0), None]
        );
        assert_eq!(plan_row_reuse(&[], &keys([1])), vec![None]);
    }

    #[test]
    fn row_shift_rebuilds_when_windows_do_not_line_up() {
        assert_eq!(plan_row_shift(&keys(0..120), &keys(500..620)), None);
        assert_eq!(plan_row_shift(&keys([1, 2, 3, 4]), &keys([1, 3, 4])), None);
        assert_eq!(plan_row_shift(&[], &keys(0..10)), None);
        assert_eq!(plan_row_shift(&keys(0..10), &[]), None);
    }

    /// Every navigation target must land inside the window fetched for it, or
    /// the selection clamps back to the old edge and key-repeat gets stuck.
    #[test]
    fn fetched_window_always_contains_navigation_target() {
        for visible_rows in [20_usize, 27, 45] {
            for total in [1_usize, 19, 40, 41, 120, 121, 500, 5_368] {
                for target in 0..total {
                    let (start, limit) = fetch_window(
                        target.saturating_sub(WINDOW_PADDING_ROWS),
                        total,
                        visible_rows,
                    );
                    let end = start + limit.min(total - start);
                    assert!(
                        start <= target && target < end,
                        "target {target} outside fetched window {start}..{end} (total {total}, visible {visible_rows})"
                    );
                }
            }
        }
    }

    /// A scroll reload must cover the whole viewport, not just its first row.
    #[test]
    fn fetched_window_covers_scrolled_viewport() {
        let visible_rows: usize = 20;
        let total: usize = 5_368;
        for first_visible in (0..total - visible_rows).step_by(7) {
            let (start, limit) = fetch_window(
                first_visible.saturating_sub(WINDOW_PADDING_ROWS),
                total,
                visible_rows,
            );
            let end = start + limit.min(total - start);
            assert!(start <= first_visible && first_visible + visible_rows <= end);
        }
    }

    #[test]
    fn window_selection_preserves_live_selection_within_window() {
        // User was at row 36 when async reload was dispatched.
        // During reload fetch, user arrowed down to row 38.
        // The newly rendered window covers rows 0..100.
        // Live selection 38 must be preserved instead of reverting to 36.
        assert_eq!(resolve_window_selection(Some(38), 36, 0, 100), Some(38));
        // If live selection fell outside the window bounds, fall back to request_selection.
        assert_eq!(resolve_window_selection(Some(150), 36, 0, 100), Some(36));
        // If there was no live selection, fall back to request_selection.
        assert_eq!(resolve_window_selection(None, 36, 0, 100), Some(36));
    }

    #[test]
    fn pending_reopen_selection_wins_over_stale_live_selection() {
        assert_eq!(preferred_live_selection(Some(0), Some(90)), Some(0));
        assert_eq!(preferred_live_selection(None, Some(90)), Some(90));
    }

    #[test]
    fn result_window_is_bounded_for_large_history() {
        assert_eq!(window_row_count_for(20, usize::MAX), 120);
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

    #[test]
    fn window_index_clamps_after_deleting_last_entry() {
        // Suppose total was 10, user was at index 9. One entry deleted -> total is 9.
        // Rendered window starts at 0 with length 9.
        assert_eq!(clamp_window_index(0, 9, 9, Some(9)), Some(8));
        // Deleting middle item at index 5 when total was 10 -> total is 9.
        assert_eq!(clamp_window_index(0, 9, 9, Some(5)), Some(5));
        // Deleting the only remaining item -> total is 0.
        assert_eq!(clamp_window_index(0, 0, 0, Some(0)), None);
    }

    #[test]
    fn scrolling_inside_the_loaded_window_does_not_reload() {
        let (visible, total) = (20, 5_000);
        for first_visible in 0..=85 {
            assert_eq!(
                scroll_reload_start(first_visible, visible, 0, 120, total),
                None,
                "unnecessary reload at first_visible {first_visible}"
            );
        }
        assert!(scroll_reload_start(86, visible, 0, 120, total).is_some());
        assert!(scroll_reload_start(0, visible, 0, 0, total).is_some());
    }

    /// Hold an arrow key through a long history using the real windowing
    /// functions, as the GTK code drives them.
    ///
    /// Returns (rows built, reloads). Panics if the selection ever falls
    /// outside the loaded window, which is how v0.1.22 got stuck at row 39.
    fn simulate_key_repeat(total: usize, visible: usize, down: bool) -> (usize, usize) {
        let first = if down { 0 } else { total - 1 };
        let (start, limit) =
            fetch_window(first.saturating_sub(WINDOW_PADDING_ROWS), total, visible);
        let window_for = |start: usize, limit: usize| {
            keys(start as i64..(start + limit.min(total - start)) as i64)
        };
        let mut window_start = start;
        let mut window = window_for(start, limit);
        let (mut rows_built, mut reloads) = (window.len(), 0);

        let order: Box<dyn Iterator<Item = usize>> = if down {
            Box::new(0..total)
        } else {
            Box::new((0..total).rev())
        };
        for selected in order {
            let inside = |start: usize, len: usize| start <= selected && selected < start + len;
            let reload = if inside(window_start, window.len()) {
                // scroll_row_into_view keeps the selection on the viewport edge.
                let first_visible = if down {
                    (selected + 1).saturating_sub(visible)
                } else {
                    selected
                };
                scroll_reload_start(first_visible, visible, window_start, window.len(), total)
            } else {
                // ensure_row_rendered: the selection outran the scroll reloads.
                Some(selected.saturating_sub(WINDOW_PADDING_ROWS))
            };
            if let Some(requested) = reload {
                let (start, limit) = fetch_window(requested, total, visible);
                let new = window_for(start, limit);
                let shift = plan_row_shift(&window, &new).expect("consecutive windows overlap");
                rows_built += shift.add_front + shift.add_back;
                reloads += 1;
                window_start = start;
                window = new;
            }
            assert!(
                inside(window_start, window.len()),
                "selection {selected} outside loaded window {window_start}..{}",
                window_start + window.len()
            );
        }
        (rows_built, reloads)
    }

    /// Holding Down/Up through 5k entries must never strand the selection,
    /// must reload rarely, and (with row reuse) must build each row about
    /// once instead of rebuilding the whole ~120-row window per reload.
    #[test]
    fn perf_key_repeat_through_history_builds_each_row_about_once() {
        let total = 5_000;
        for visible in [20, 45] {
            for down in [true, false] {
                let (rows_built, reloads) = simulate_key_repeat(total, visible, down);
                let window = window_row_count_for(visible, total);
                assert!(
                    rows_built <= total + window,
                    "built {rows_built} rows for {total} entries (visible {visible}, down {down}); window rows are being rebuilt"
                );
                assert!(
                    reloads <= total / 25,
                    "{reloads} reloads for {total} entries (visible {visible}, down {down})"
                );
            }
        }
    }
}
