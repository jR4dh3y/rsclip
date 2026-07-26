use std::cell::{Cell, RefCell};
use std::rc::Rc;

use anyhow::Result;
use gio::prelude::*;
use gtk::prelude::*;
use gtk4 as gtk;
use rsclip_core::models::{EntryFilter, SortMode};
use rsclip_core::{AppConfig, Database, RsclipPaths};

use crate::actions::refresh::refresh_entries;
use crate::actions::update_mode_controls;
use crate::components::{footer, list, preview, topbar};
use crate::state::{AppState, AppView};

pub(crate) struct UiRuntime {
    pub(crate) state: Rc<AppState>,
    pub(crate) window: gtk::ApplicationWindow,
    _config_monitor: gio::FileMonitor,
    _hold: gio::ApplicationHoldGuard,
}

impl UiRuntime {
    pub(crate) fn preload(&self) -> Result<()> {
        sync_topbar_from_state(&self.state);
        update_mode_controls(&self.state);
        refresh_entries(&self.state)
    }

    pub(crate) fn show_reset(&self) -> Result<()> {
        use gtk4_layer_shell::{KeyboardMode, LayerShell};

        let reset_on_show = self.state.reset_on_show.get();
        if self.state.reset_on_show.get() {
            *self.state.view.borrow_mut() = self.state.default_view.get();
            *self.state.query.borrow_mut() = String::new();
            *self.state.filter.borrow_mut() = self.state.default_filter.get();
            *self.state.sort.borrow_mut() = self.state.default_sort.get();
            self.state.search_entry.set_text("");
        }

        sync_topbar_from_state(&self.state);
        update_mode_controls(&self.state);

        self.window.set_keyboard_mode(KeyboardMode::Exclusive);
        self.window.set_visible(true);
        self.window.present();
        if self.state.auto_focus_search.get() {
            self.state.search_entry.grab_focus();
        }
        refresh_entries_after_present(&self.state, reset_on_show);
        Ok(())
    }

    pub(crate) fn toggle(&self) -> Result<()> {
        if self.window.is_visible() {
            self.hide();
            Ok(())
        } else {
            self.show_reset()
        }
    }

    pub(crate) fn hide(&self) {
        hide_overlay(&self.state, &self.window);
    }
}

pub(crate) fn build_ui(app: &gtk::Application) -> Result<UiRuntime> {
    let paths = RsclipPaths::discover()?;
    paths.ensure()?;
    let config = AppConfig::load(&paths)?;
    let db = Database::open(&paths.db_path)?;

    crate::style::load_css(&config)?;

    let default_view = config_view(&config.ui.start_view);
    let default_filter = config_filter(&config.ui.default_filter);
    let default_sort = config_sort(&config.ui.default_sort);
    let window_width = config.ui.window_width.clamp(320, 3840);
    let window_height = config.ui.window_height.clamp(240, 2160);
    let sidebar_width = config
        .ui
        .sidebar_width
        .clamp(160, window_width.saturating_sub(160));
    let initial_placeholder = match default_view {
        AppView::Clipboard => config.ui.search_placeholder.as_str(),
        AppView::Secrets => config.ui.secrets_search_placeholder.as_str(),
    };

    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title("rsclip")
        .default_width(window_width)
        .default_height(window_height)
        .resizable(config.ui.resizable)
        .build();
    window.add_css_class("rsclip-window");
    configure_overlay_window(&window, &config);

    let root = gtk::Overlay::new();
    window.set_child(Some(&root));

    let shell = gtk::Box::new(gtk::Orientation::Vertical, 0);
    shell.add_css_class("app-shell");
    root.set_child(Some(&shell));

    let topbar = topbar::build(initial_placeholder);
    topbar.filter.set_selected(filter_index(default_filter));
    shell.append(&topbar.container);

    let paned = gtk::Paned::new(gtk::Orientation::Horizontal);
    paned.set_wide_handle(true);
    paned.set_vexpand(true);
    shell.append(&paned);

    let list_panel = list::build_panel();
    paned.set_start_child(Some(&list_panel.scroller));

    let preview_panel = preview::build_panel();
    if config.ui.preview_default {
        paned.set_end_child(Some(&preview_panel.shell));
    }
    paned.set_position(sidebar_width);

    let footer_bar = footer::build(config.ui.show_footer_hints);
    shell.append(&footer_bar.container);

    let state = Rc::new(AppState {
        db,
        db_path: paths.db_path.clone(),
        favicon_icon_dir: paths.favicon_icon_dir.clone(),
        history_limit: Cell::new(config.history.max_entries),
        auto_paste: Cell::new(config.paste.auto_paste),
        paste_delay_ms: Cell::new(config.paste.paste_delay_ms),
        paste_method: RefCell::new(config.paste.method.clone()),
        ocr_enabled: Cell::new(config.ocr.enabled),
        ocr_command: RefCell::new(config.ocr.command.clone()),
        ocr_language: RefCell::new(config.ocr.default_language.clone()),
        ocr_timeout_seconds: Cell::new(config.ocr.timeout_seconds),
        default_view: Cell::new(default_view),
        default_filter: Cell::new(default_filter),
        default_sort: Cell::new(default_sort),
        reset_on_show: Cell::new(config.ui.reset_on_show),
        auto_focus_search: Cell::new(config.ui.auto_focus_search),
        show_footer_hints: Cell::new(config.ui.show_footer_hints),
        search_placeholder: RefCell::new(config.ui.search_placeholder.clone()),
        secrets_search_placeholder: RefCell::new(config.ui.secrets_search_placeholder.clone()),
        entries: RefCell::new(Vec::new()),
        secrets: RefCell::new(Vec::new()),
        entries_start: Cell::new(0),
        secrets_start: Cell::new(0),
        entries_total: Cell::new(0),
        secrets_total: Cell::new(0),
        virtual_list_update: Cell::new(false),
        query: RefCell::new(String::new()),
        filter: RefCell::new(default_filter),
        sort: RefCell::new(default_sort),
        view: RefCell::new(default_view),
        prompt_active: RefCell::new(false),
        search_entry: topbar.search.clone(),
        filter_select: topbar.filter.clone(),
        history_button: topbar.history_button.clone(),
        secrets_button: topbar.secrets_button.clone(),
        count_label: topbar.count.clone(),
        paned: paned.clone(),
        list: list_panel.list.clone(),
        list_adjustment: list_panel.adjustment,
        preview_shell: preview_panel.shell.clone(),
        preview: preview_panel.preview.clone(),
        details: preview_panel.details.clone(),
        footer: footer_bar.footer.clone(),
        ocr_button: footer_bar.ocr_button.clone(),
    });
    update_mode_controls(&state);

    crate::notify::install_change_listener(&state, &window, &paths.socket_path)?;
    let config_monitor = crate::config_reload::install_config_watcher(&state, &window, &paths)?;
    crate::events::connect(&state, &window);

    Ok(UiRuntime {
        state,
        window,
        _config_monitor: config_monitor,
        _hold: app.hold(),
    })
}

fn configure_overlay_window(window: &gtk::ApplicationWindow, config: &AppConfig) {
    use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};

    window.set_decorated(false);
    window.set_resizable(config.ui.resizable);

    window.init_layer_shell();
    window.set_namespace(Some("rsclip"));
    window.set_layer(Layer::Overlay);
    window.set_keyboard_mode(KeyboardMode::None);
    window.set_exclusive_zone(-1);

    window.set_anchor(Edge::Left, false);
    window.set_anchor(Edge::Right, false);
    window.set_anchor(Edge::Top, false);
    window.set_anchor(Edge::Bottom, false);
}

fn refresh_entries_after_present(state: &Rc<AppState>, reset_on_show: bool) {
    let state = Rc::clone(state);
    gtk::glib::idle_add_local_once(move || {
        if let Err(err) = refresh_entries(&state) {
            crate::actions::set_footer(&state, &format!("Refresh failed: {err:#}"));
            return;
        }

        if reset_on_show {
            state.list_adjustment.set_value(0.0);
            if let Some(row) = state.list.row_at_index(0) {
                state.list.select_row(Some(&row));
            }
        }
    });
}

pub(crate) fn hide_overlay(state: &Rc<AppState>, window: &gtk::ApplicationWindow) {
    use gtk4_layer_shell::{KeyboardMode, LayerShell};

    window.set_keyboard_mode(KeyboardMode::None);
    window.set_visible(false);
    preview::clear_preview_state(state);
    *state.prompt_active.borrow_mut() = false;
}

pub(crate) fn close_overlay_and_paste(state: &Rc<AppState>, window: &gtk::ApplicationWindow) {
    hide_overlay(state, window);

    if state.auto_paste.get() {
        let delay = std::time::Duration::from_millis(state.paste_delay_ms.get());
        let method = state.paste_method.borrow().clone();
        gtk::glib::timeout_add_local_once(delay, move || {
            if let Err(err) = rsclip_core::paste::trigger_paste_with_method(&method) {
                eprintln!("rsclip: Paste failed: {err:#}");
            }
        });
    }
}

pub(crate) fn sync_topbar_from_state(state: &Rc<AppState>) {
    let view = *state.view.borrow();
    let placeholder = search_placeholder(state, view);
    state.search_entry.set_placeholder_text(Some(&placeholder));
    state
        .filter_select
        .set_selected(filter_index(*state.filter.borrow()));
}

pub(crate) fn search_placeholder(state: &AppState, view: AppView) -> String {
    match view {
        AppView::Clipboard => state.search_placeholder.borrow().clone(),
        AppView::Secrets => state.secrets_search_placeholder.borrow().clone(),
    }
}

pub(crate) fn config_view(value: &str) -> AppView {
    match value.trim().to_ascii_lowercase().as_str() {
        "secrets" | "secret" => AppView::Secrets,
        _ => AppView::Clipboard,
    }
}

pub(crate) fn config_filter(value: &str) -> EntryFilter {
    EntryFilter::parse(&value.trim().to_ascii_lowercase())
}

pub(crate) fn config_sort(value: &str) -> SortMode {
    SortMode::parse(&value.trim().to_ascii_lowercase())
}

pub(crate) fn filter_index(filter: EntryFilter) -> u32 {
    match filter {
        EntryFilter::All => 0,
        EntryFilter::Text => 1,
        EntryFilter::Images => 2,
        EntryFilter::Files => 3,
        EntryFilter::Links => 4,
        EntryFilter::Colors => 5,
        EntryFilter::Pinned => 6,
    }
}
