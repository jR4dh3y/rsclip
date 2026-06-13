use anyhow::{Context, Result};
use rsclip_core::cli::{flag, option_value, positional_i64};
use rsclip_core::notify::notify_changed;
use rsclip_core::paste::paste_entry_with_method;
use rsclip_core::{AppConfig, Database, RsclipPaths};

pub fn run(args: &[String]) -> Result<()> {
    let id = positional_i64(args, 0, "entry id")?;
    let paths = RsclipPaths::discover()?;
    let config = AppConfig::load(&paths)?;
    let auto_paste = config.paste.auto_paste && !flag(args, "--copy-only");
    let delay_ms = option_value(args, "--delay-ms")
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(config.paste.paste_delay_ms);
    let db = Database::open(&paths.db_path)?;
    let entry = db
        .get_entry(id)?
        .with_context(|| format!("entry {id} not found"))?;
    paste_entry_with_method(&entry, auto_paste, delay_ms, &config.paste.method)?;
    db.touch_used(id)?;
    notify_changed(&paths);
    Ok(())
}
