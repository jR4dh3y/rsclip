use std::rc::Rc;

use gtk::gdk;
use gtk::prelude::*;
use gtk4 as gtk;
use rsclip_core::models::{EntryFilter, EntryKind};

use crate::actions::clipboard::{copy_secret, copy_selected_entry};
use crate::actions::ocr::run_ocr_for_entry;
use crate::actions::refresh::refresh_entries;
use crate::actions::secrets::{
    delete_current, rename_current_secret_dialog, save_current_as_secret_dialog, toggle_pin,
};
use crate::actions::selection::{mark_selected_row, move_selection};
use crate::actions::{set_footer, update_mode_controls};
use crate::components::preview::{render_preview, render_secret_preview};
use crate::state::{AppState, AppView, current_entry, current_secret};

pub(crate) fn connect(state: &Rc<AppState>, window: &gtk::ApplicationWindow) {
    connect_mode_buttons(state);
    connect_ocr_button(state);
    connect_search(state);
    connect_filter(state);
    connect_list_selection(state);
    connect_list_activation(state, window);
    connect_keyboard(state, window);
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
            search.set_placeholder_text(Some("Search clipboard..."));
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
            search.set_placeholder_text(Some("Search secrets by name..."));
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
    state.search_entry.set_placeholder_text(Some(match view {
        AppView::Clipboard => "Search clipboard...",
        AppView::Secrets => "Search secrets by name...",
    }));
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

fn connect_search(state: &Rc<AppState>) {
    let search = state.search_entry.clone();
    let state = Rc::clone(state);
    search.connect_search_changed(move |entry| {
        *state.query.borrow_mut() = entry.text().to_string();
        if let Err(err) = refresh_entries(&state) {
            set_footer(&state, &format!("Search failed: {err:#}"));
        }
    });
}

fn connect_filter(state: &Rc<AppState>) {
    let filter = state.filter_select.clone();
    let state = Rc::clone(state);
    filter.connect_selected_notify(move |dropdown| {
        *state.filter.borrow_mut() = match dropdown.selected() {
            1 => EntryFilter::Text,
            2 => EntryFilter::Images,
            3 => EntryFilter::Links,
            4 => EntryFilter::Colors,
            5 => EntryFilter::Pinned,
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
                        if let Some(entry) = state.entries.borrow().get(index as usize) {
                            render_preview(&state, entry);
                        }
                    }
                    AppView::Secrets => {
                        if let Some(secret) = state.secrets.borrow().get(index as usize) {
                            render_secret_preview(&state, secret);
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
            if let Some(entry) = state.entries.borrow().get(row.index() as usize).cloned() {
                if let Err(err) = copy_selected_entry(&state, &entry) {
                    set_footer(&state, &format!("Paste failed: {err:#}"));
                    return;
                }
                crate::window::close_overlay_and_paste(&state, &window);
            }
        }
        AppView::Secrets => {
            if let Some(secret) = state.secrets.borrow().get(row.index() as usize).cloned() {
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
                        *state.filter.borrow_mut() = EntryFilter::Images;
                        if let Err(err) = refresh_entries(&state) {
                            set_footer(&state, &format!("Filter failed: {err:#}"));
                        }
                    }
                    gtk::glib::Propagation::Stop
                }
                (gdk::Key::l | gdk::Key::L, true) => {
                    if *state.view.borrow() == AppView::Clipboard {
                        *state.filter.borrow_mut() = EntryFilter::Links;
                        if let Err(err) = refresh_entries(&state) {
                            set_footer(&state, &format!("Filter failed: {err:#}"));
                        }
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

fn handle_enter(state: &Rc<AppState>, window: &gtk::ApplicationWindow) {
    match *state.view.borrow() {
        AppView::Clipboard => {
            if let Some(entry) = current_entry(state) {
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
            if let Some(entry) = current_entry(state) {
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
