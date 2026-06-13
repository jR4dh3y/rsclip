use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;
use std::time::Duration;

use anyhow::{Context, Result};
use gio::prelude::*;
use gtk::prelude::*;
use gtk4 as gtk;
use rsclip_core::{AppConfig, RsclipPaths};

use crate::actions::refresh::{refresh_entries, rerender_current_list};
use crate::actions::set_footer;
use crate::state::AppState;

const CONFIG_RELOAD_DEBOUNCE: Duration = Duration::from_millis(120);

pub(crate) fn install_config_watcher(
    state: &Rc<AppState>,
    window: &gtk::ApplicationWindow,
    paths: &RsclipPaths,
) -> Result<gio::FileMonitor> {
    let config_dir = gio::File::for_path(&paths.config_dir);
    let monitor = config_dir
        .monitor_directory(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE)
        .with_context(|| format!("watching config directory {}", paths.config_dir.display()))?;
    monitor.set_rate_limit(100);

    let pending_reload = Rc::new(RefCell::new(None::<gtk::glib::SourceId>));
    let config_path = paths.config_path();
    let paths = paths.clone();
    let state = Rc::clone(state);
    let window = window.clone();

    monitor.connect_changed(move |_, file, other_file, event| {
        if !should_reload_config(file, other_file, event, &config_path) {
            return;
        }

        if let Some(source_id) = pending_reload.borrow_mut().take() {
            source_id.remove();
        }

        let state = Rc::clone(&state);
        let window = window.clone();
        let paths = paths.clone();
        let pending_reload_for_timeout = Rc::clone(&pending_reload);
        let source_id = gtk::glib::timeout_add_local_once(CONFIG_RELOAD_DEBOUNCE, move || {
            let _ = pending_reload_for_timeout.borrow_mut().take();
            if let Err(err) = reload_config(&state, &window, &paths) {
                let message = format!("Config reload failed: {err:#}");
                set_footer(&state, &message);
                tracing::warn!("{message}");
            }
        });
        *pending_reload.borrow_mut() = Some(source_id);
    });

    Ok(monitor)
}

fn reload_config(
    state: &Rc<AppState>,
    window: &gtk::ApplicationWindow,
    paths: &RsclipPaths,
) -> Result<()> {
    let config = AppConfig::load(paths)?;
    crate::style::load_css(&config)?;
    let outcome = apply_live_config(state, window, &config);

    crate::window::sync_topbar_from_state(state);
    if window.is_visible() {
        if outcome.refresh_entries {
            refresh_entries(state)?;
        } else if outcome.rerender_current_list {
            rerender_current_list(state);
        }
    } else if outcome.refresh_entries || outcome.rerender_current_list {
        *state.dirty.borrow_mut() = true;
    }

    Ok(())
}

fn apply_live_config(
    state: &Rc<AppState>,
    window: &gtk::ApplicationWindow,
    config: &AppConfig,
) -> ApplyOutcome {
    let mut outcome = ApplyOutcome::default();

    let old_history_limit = state.history_limit.get();
    state.history_limit.set(config.history.max_entries);
    outcome.refresh_entries |= old_history_limit != config.history.max_entries;

    state.auto_paste.set(config.paste.auto_paste);
    state.paste_delay_ms.set(config.paste.paste_delay_ms);
    *state.paste_method.borrow_mut() = config.paste.method.clone();

    let old_ocr_enabled = state.ocr_enabled.get();
    state.ocr_enabled.set(config.ocr.enabled);
    *state.ocr_command.borrow_mut() = config.ocr.command.clone();
    *state.ocr_language.borrow_mut() = config.ocr.default_language.clone();
    state.ocr_timeout_seconds.set(config.ocr.timeout_seconds);
    outcome.rerender_current_list |= old_ocr_enabled != config.ocr.enabled;

    let new_default_view = crate::window::config_view(&config.ui.start_view);
    let old_default_filter = state.default_filter.get();
    let new_default_filter = crate::window::config_filter(&config.ui.default_filter);
    let old_default_sort = state.default_sort.get();
    let new_default_sort = crate::window::config_sort(&config.ui.default_sort);

    state.default_view.set(new_default_view);
    state.default_filter.set(new_default_filter);
    state.default_sort.set(new_default_sort);
    state.reset_on_show.set(config.ui.reset_on_show);
    state.auto_focus_search.set(config.ui.auto_focus_search);

    if *state.filter.borrow() == old_default_filter {
        *state.filter.borrow_mut() = new_default_filter;
        outcome.refresh_entries |= old_default_filter != new_default_filter;
    }
    if *state.sort.borrow() == old_default_sort {
        *state.sort.borrow_mut() = new_default_sort;
        outcome.refresh_entries |= old_default_sort != new_default_sort;
    }

    let old_show_footer_hints = state.show_footer_hints.get();
    state.show_footer_hints.set(config.ui.show_footer_hints);
    outcome.rerender_current_list |= old_show_footer_hints != config.ui.show_footer_hints;

    *state.search_placeholder.borrow_mut() = config.ui.search_placeholder.clone();
    *state.secrets_search_placeholder.borrow_mut() = config.ui.secrets_search_placeholder.clone();

    let window_width = config.ui.window_width.clamp(320, 3840);
    let window_height = config.ui.window_height.clamp(240, 2160);
    window.set_default_size(window_width, window_height);
    window.set_resizable(config.ui.resizable);

    let sidebar_width = config
        .ui
        .sidebar_width
        .clamp(160, window_width.saturating_sub(160));
    state.paned.set_position(sidebar_width);

    match (config.ui.preview_default, state.paned.end_child().is_some()) {
        (true, false) => {
            state.paned.set_end_child(Some(&state.preview_shell));
            outcome.rerender_current_list = true;
        }
        (false, true) => {
            state.paned.set_end_child(None::<&gtk::Widget>);
            outcome.rerender_current_list = true;
        }
        _ => {}
    }

    outcome
}

#[derive(Default)]
struct ApplyOutcome {
    refresh_entries: bool,
    rerender_current_list: bool,
}

fn should_reload_config(
    file: &gio::File,
    other_file: Option<&gio::File>,
    event: gio::FileMonitorEvent,
    config_path: &Path,
) -> bool {
    let reload_event = matches!(
        event,
        gio::FileMonitorEvent::Changed
            | gio::FileMonitorEvent::ChangesDoneHint
            | gio::FileMonitorEvent::Created
            | gio::FileMonitorEvent::Deleted
            | gio::FileMonitorEvent::Moved
            | gio::FileMonitorEvent::Renamed
            | gio::FileMonitorEvent::MovedIn
            | gio::FileMonitorEvent::MovedOut
    );
    reload_event
        && (file_matches(file, config_path)
            || other_file.is_some_and(|file| file_matches(file, config_path)))
}

fn file_matches(file: &gio::File, config_path: &Path) -> bool {
    file.path()
        .is_some_and(|path| path.as_path() == config_path)
}
