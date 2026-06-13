use std::rc::Rc;

use anyhow::Result;
use gtk4::prelude::*;
use rsclip_core::Database;

use crate::actions::set_footer;
use crate::components::labels::muted_label;
use crate::components::list::{entry_row, secret_row};
use crate::components::preview::{render_preview, render_secret_preview};
use crate::state::{AppState, AppView, current_entry, current_secret};

pub(crate) fn refresh_entries(state: &Rc<AppState>) -> Result<()> {
    let db = Database::open(&state.db_path)?;
    let limit = state.history_limit.get();

    match *state.view.borrow() {
        AppView::Clipboard => {
            let selected_id = current_entry(state).map(|entry| entry.id);
            let entries = db.list_entries(
                &state.query.borrow(),
                *state.filter.borrow(),
                *state.sort.borrow(),
                limit,
            )?;
            *state.entries.borrow_mut() = entries;
            render_clipboard_list(state, selected_id);
        }
        AppView::Secrets => {
            let selected_id = current_secret(state).map(|secret| secret.id);
            let secrets = db.list_secrets(&state.query.borrow(), limit)?;
            *state.secrets.borrow_mut() = secrets;
            render_secrets_list(state, selected_id);
        }
    }
    Ok(())
}

pub(crate) fn refresh_entries_if_changed(state: &Rc<AppState>) -> Result<()> {
    let db = Database::open(&state.db_path)?;
    let limit = state.history_limit.get();

    match *state.view.borrow() {
        AppView::Clipboard => {
            let entries = db.list_entries(
                &state.query.borrow(),
                *state.filter.borrow(),
                *state.sort.borrow(),
                limit,
            )?;
            if state.entries.borrow().as_slice() == entries.as_slice() {
                return Ok(());
            }
            let selected_id = current_entry(state).map(|entry| entry.id);
            *state.entries.borrow_mut() = entries;
            render_clipboard_list(state, selected_id);
        }
        AppView::Secrets => {
            let secrets = db.list_secrets(&state.query.borrow(), limit)?;
            if state.secrets.borrow().as_slice() == secrets.as_slice() {
                return Ok(());
            }
            let selected_id = current_secret(state).map(|secret| secret.id);
            *state.secrets.borrow_mut() = secrets;
            render_secrets_list(state, selected_id);
        }
    }
    Ok(())
}

pub(crate) fn rerender_current_list(state: &Rc<AppState>) {
    match *state.view.borrow() {
        AppView::Clipboard => {
            let selected_id = current_entry(state).map(|entry| entry.id);
            render_clipboard_list(state, selected_id);
        }
        AppView::Secrets => {
            let selected_id = current_secret(state).map(|secret| secret.id);
            render_secrets_list(state, selected_id);
        }
    }
}

fn render_clipboard_list(state: &Rc<AppState>, selected_id: Option<i64>) {
    state.secrets.borrow_mut().clear();
    crate::components::clear_list(&state.list);

    for entry in state.entries.borrow().iter() {
        state
            .list
            .append(&entry_row(entry, &state.favicon_icon_dir));
    }

    let selected_index = selected_id
        .and_then(|id| {
            state
                .entries
                .borrow()
                .iter()
                .position(|entry| entry.id == id)
        })
        .unwrap_or(0);

    if let Some(row) = state.list.row_at_index(selected_index as i32) {
        state.list.select_row(Some(&row));
        if let Some(entry) = state.entries.borrow().get(selected_index) {
            render_preview(state, entry);
        }
    } else {
        crate::components::clear_box(&state.preview);
        crate::components::clear_box(&state.details);
        state
            .preview
            .append(&muted_label("No clipboard entries yet"));
    }
    state
        .count_label
        .set_text(&format!("Entries {}", state.entries.borrow().len()));
    if state.show_footer_hints.get() {
        set_footer(
            state,
            "Tab: switch tab | Enter: paste | Ctrl+C: copy | Ctrl+S: secret | Ctrl+P: pin | Ctrl+D: delete | Esc: close",
        );
    } else {
        set_footer(state, "");
    }
}

fn render_secrets_list(state: &Rc<AppState>, selected_id: Option<i64>) {
    state.entries.borrow_mut().clear();
    crate::components::clear_list(&state.list);

    for secret in state.secrets.borrow().iter() {
        state.list.append(&secret_row(secret));
    }

    let selected_index = selected_id
        .and_then(|id| {
            state
                .secrets
                .borrow()
                .iter()
                .position(|secret| secret.id == id)
        })
        .unwrap_or(0);

    if let Some(row) = state.list.row_at_index(selected_index as i32) {
        state.list.select_row(Some(&row));
        if let Some(secret) = state.secrets.borrow().get(selected_index) {
            render_secret_preview(state, secret);
        }
    } else {
        crate::components::clear_box(&state.preview);
        crate::components::clear_box(&state.details);
        state.preview.append(&muted_label("No secrets saved yet"));
    }
    state
        .count_label
        .set_text(&format!("Secrets {}", state.secrets.borrow().len()));
    if state.show_footer_hints.get() {
        set_footer(
            state,
            "Tab: switch tab | Enter: copy | Ctrl+C: copy | Ctrl+E: rename | Ctrl+D: delete | Esc: close",
        );
    } else {
        set_footer(state, "");
    }
}
