use std::rc::Rc;

use anyhow::{Context, Result};
use rsclip_core::EntryData;
use rsclip_core::ocr::run_tesseract_with_options;

use crate::actions::set_footer;
use crate::components::preview::render_preview;
use crate::state::AppState;

pub(crate) fn run_ocr_for_entry(state: &Rc<AppState>, entry_id: i64) -> Result<()> {
    if !state.ocr_enabled.get() {
        anyhow::bail!("OCR is disabled in config");
    }

    set_footer(state, "Running OCR...");

    let entry = state
        .db
        .get_entry(entry_id)?
        .with_context(|| format!("entry {entry_id} not found"))?;
    let image_path = match &entry.data {
        EntryData::Image { file_path, .. } => file_path.clone(),
        _ => anyhow::bail!("entry is not an image"),
    };
    let language = state.ocr_language.borrow().clone();
    let command = state.ocr_command.borrow().clone();
    let timeout_seconds = state.ocr_timeout_seconds.get();
    let text = run_tesseract_with_options(&image_path, &language, &command, timeout_seconds)?;
    state.db.save_ocr_result(entry_id, &language, &text)?;
    let updated = state
        .db
        .get_entry(entry_id)?
        .with_context(|| format!("entry {entry_id} not found after OCR"))?;
    if let Some(slot) = state
        .entries
        .borrow_mut()
        .iter_mut()
        .find(|entry| entry.id == entry_id)
    {
        *slot = updated.clone();
    }
    render_preview(state, &updated);
    set_footer(state, "OCR complete");
    Ok(())
}
