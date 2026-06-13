use anyhow::Result;
use rsclip_core::cli::positional_i64;
use rsclip_core::notify::notify_changed;
use rsclip_core::{Database, RsclipPaths};

pub fn run(args: &[String]) -> Result<()> {
    let id = positional_i64(args, 0, "entry id")?;
    let paths = RsclipPaths::discover()?;
    let db = Database::open(&paths.db_path)?;
    db.delete_entry(id)?;
    notify_changed(&paths);
    Ok(())
}
