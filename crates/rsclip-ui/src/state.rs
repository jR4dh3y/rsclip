use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use gtk::prelude::*;
use gtk4 as gtk;
use rsclip_core::models::{ClipboardEntry, EntryFilter, SecretEntry, SortMode};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AppView {
    Clipboard,
    Secrets,
}

pub(crate) struct AppState {
    pub(crate) db_path: PathBuf,
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
    pub(crate) query: RefCell<String>,
    pub(crate) filter: RefCell<EntryFilter>,
    pub(crate) sort: RefCell<SortMode>,
    pub(crate) view: RefCell<AppView>,
    pub(crate) dirty: RefCell<bool>,
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
    let index = row.index();
    if index < 0 {
        return None;
    }
    state.entries.borrow().get(index as usize).cloned()
}

pub(crate) fn current_secret(state: &Rc<AppState>) -> Option<SecretEntry> {
    let row = state.list.selected_row()?;
    let index = row.index();
    if index < 0 {
        return None;
    }
    state.secrets.borrow().get(index as usize).cloned()
}
