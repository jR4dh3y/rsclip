use std::rc::Rc;

use anyhow::Result;
use rsclip_core::models::{ClipboardEntry, SecretEntry};
use rsclip_core::paste::{copy_entry, write_clipboard};

use crate::state::AppState;

pub(crate) fn copy_selected_entry(state: &Rc<AppState>, entry: &ClipboardEntry) -> Result<()> {
    copy_entry(entry)?;
    state.db.borrow().touch_used(entry.id)?;
    Ok(())
}

pub(crate) fn copy_secret(state: &Rc<AppState>, secret: &SecretEntry) -> Result<()> {
    write_clipboard("text/plain", secret.value.as_bytes())?;
    state.db.borrow().touch_secret_used(secret.id)?;
    Ok(())
}

pub(crate) fn copy_text(text: &str) -> Result<()> {
    write_clipboard("text/plain", text.as_bytes())
}
