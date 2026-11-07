use std::rc::Rc;

use anyhow::{Context, Result};
use gtk4::prelude::*;
use rsclip_core::Database;
use rsclip_core::models::SecretEntry;
use rsclip_core::paste::{copy_entry, write_clipboard};

use crate::events::queue_write;
use crate::state::{
    AfterCopy, AppState, CopyRequest, CopyResponse, CopySource, advance_clipboard_serial,
};

/// Queue a clipboard write on the copy worker; [`finish_copy`] runs when it is done.
pub(crate) fn queue_copy(state: &Rc<AppState>, source: CopySource, then: AfterCopy) -> Result<()> {
    let serial = advance_clipboard_serial(state);
    state
        .copy_request_tx
        .send(CopyRequest {
            source,
            then,
            serial,
        })
        .map_err(|_| anyhow::anyhow!("copy worker stopped"))?;
    state.pending_copies.set(state.pending_copies.get() + 1);
    Ok(())
}

pub(crate) fn queue_entry_copy(state: &Rc<AppState>, id: i64, then: AfterCopy) -> Result<()> {
    queue_copy(state, CopySource::Entry(id), then)
}

pub(crate) fn queue_secret_copy(
    state: &Rc<AppState>,
    secret: &SecretEntry,
    then: AfterCopy,
) -> Result<()> {
    let source = CopySource::Secret {
        id: secret.id,
        value: secret.value.clone(),
    };
    queue_copy(state, source, then)
}

/// Copy one entry to the clipboard. Runs on the copy worker.
pub(crate) fn copy_entry_by_id(db: &Database, id: i64) -> Result<()> {
    let entry = db.get_entry(id)?.context("entry no longer exists")?;
    copy_entry(&entry)
}

pub(crate) fn copy_text(text: &str) -> Result<()> {
    write_clipboard("text/plain", text.as_bytes())
}

/// Apply a finished worker copy on the GTK thread.
pub(crate) fn finish_copy(state: &Rc<AppState>, copy: CopyResponse) {
    state
        .pending_copies
        .set(state.pending_copies.get().saturating_sub(1));
    if copy.result.is_ok()
        && let Some(touch) = copy.touch
        && let Err(err) = queue_write(state, touch)
    {
        crate::actions::set_footer(state, &format!("Update failed: {err:#}"));
    }
    // A later clipboard write or closing the overlay superseded this request:
    // its paste would insert another value or land in a session the user
    // already left.
    if copy.serial != state.clipboard_serial.get() {
        return;
    }
    let window = || {
        state
            .list
            .root()
            .and_then(|root| root.downcast::<gtk4::ApplicationWindow>().ok())
    };
    match (copy.result, copy.then) {
        (Err(err), AfterCopy::Paste | AfterCopy::PasteNow) => {
            crate::actions::set_footer(state, &format!("Paste failed: {err}"))
        }
        (Err(err), AfterCopy::Hide) => {
            crate::actions::set_footer(state, &format!("Copy failed: {err}"))
        }
        (Err(err), AfterCopy::Report { failed, .. }) => {
            crate::actions::set_footer(state, &format!("{failed}: {err}"))
        }
        (Ok(()), AfterCopy::Report { done, .. }) => crate::actions::set_footer(state, done),
        (Ok(()), AfterCopy::Paste) => {
            if let Some(window) = window() {
                crate::window::close_overlay_and_paste(state, &window, false);
            }
        }
        (Ok(()), AfterCopy::PasteNow) => {
            if let Some(window) = window() {
                crate::window::close_overlay_and_paste(state, &window, true);
            }
        }
        (Ok(()), AfterCopy::Hide) => {
            if let Some(window) = window() {
                crate::window::hide_overlay(state, &window);
            }
        }
    }
}
