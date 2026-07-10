use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use gtk::prelude::*;
use gtk4 as gtk;
use rsclip_core::Database;
use rsclip_core::models::{ClipboardEntry, EntryFilter, SecretEntry, SortMode};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AppView {
    Clipboard,
    Secrets,
}

pub(crate) struct AppState {
    /// Long-lived connection so search/list never re-open + re-migrate on the UI thread.
    pub(crate) db: RefCell<Database>,
    pub(crate) favicon_icon_dir: PathBuf,
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
    pub(crate) footer: gtk::Label,
    pub(crate) ocr_button: gtk::Button,
}

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
