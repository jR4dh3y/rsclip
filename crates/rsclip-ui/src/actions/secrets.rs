use std::rc::Rc;

use anyhow::{Context, Result};
use gtk::prelude::*;
use gtk4 as gtk;
use rsclip_core::Database;
use rsclip_core::secrets::{default_secret_alias, secret_value_from_entry};

use crate::actions::refresh::refresh_entries;
use crate::actions::{set_footer, update_mode_controls};
use crate::dialogs::secret_alias::prompt_secret_alias;
use crate::state::{AppState, AppView, current_entry, current_secret};

pub(crate) fn save_current_as_secret_dialog(state: &Rc<AppState>, parent: &gtk::Window) {
    let Some(entry) = current_entry(state) else {
        set_footer(state, "No selected entry to save");
        return;
    };
    let Some(value) = secret_value_from_entry(&entry) else {
        set_footer(state, "Only text-like entries can be saved as secrets");
        return;
    };
    let default_alias = default_secret_alias(&entry);

    prompt_secret_alias(
        state,
        parent,
        "Save Secret",
        &default_alias,
        move |state, alias| {
            let db = Database::open(&state.db_path)?;
            db.save_secret(Some(entry.id), &alias, &value)?;
            db.delete_entry(entry.id)?;
            *state.view.borrow_mut() = AppView::Secrets;
            *state.query.borrow_mut() = String::new();
            state.search_entry.set_text("");
            let placeholder = crate::window::search_placeholder(state.as_ref(), AppView::Secrets);
            state.search_entry.set_placeholder_text(Some(&placeholder));
            update_mode_controls(state);
            refresh_entries(state)?;
            set_footer(state, "Saved secret");
            Ok(())
        },
    );
}

pub(crate) fn rename_current_secret_dialog(state: &Rc<AppState>, parent: &gtk::Window) {
    let Some(secret) = current_secret(state) else {
        set_footer(state, "No selected secret to rename");
        return;
    };

    prompt_secret_alias(
        state,
        parent,
        "Rename Secret",
        &secret.alias,
        move |state, alias| {
            let db = Database::open(&state.db_path)?;
            db.rename_secret(secret.id, &alias)?;
            refresh_entries(state)?;
            set_footer(state, "Renamed secret");
            Ok(())
        },
    );
}

pub(crate) fn toggle_pin(state: &Rc<AppState>) -> Result<()> {
    let entry = current_entry(state).context("no selected entry")?;
    let db = Database::open(&state.db_path)?;
    db.set_pinned(entry.id, !entry.pinned)?;
    refresh_entries(state)
}

pub(crate) fn delete_current(state: &Rc<AppState>) -> Result<()> {
    let db = Database::open(&state.db_path)?;
    let view = *state.view.borrow();
    match view {
        AppView::Clipboard => {
            let entry = current_entry(state).context("no selected entry")?;
            db.delete_entry(entry.id)?;
        }
        AppView::Secrets => {
            let secret = current_secret(state).context("no selected secret")?;
            let restore_clipboard = secret.source_entry_id.is_some();
            db.delete_secret(secret.id)?;
            if restore_clipboard {
                *state.view.borrow_mut() = AppView::Clipboard;
                *state.query.borrow_mut() = String::new();
                state.search_entry.set_text("");
                let placeholder =
                    crate::window::search_placeholder(state.as_ref(), AppView::Clipboard);
                state.search_entry.set_placeholder_text(Some(&placeholder));
                update_mode_controls(state);
            }
        }
    }
    refresh_entries(state)
}
