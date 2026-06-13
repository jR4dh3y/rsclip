use anyhow::Result;
use rsclip_core::cli::{flag, positional_i64};
use rsclip_core::notify::notify_changed;
use rsclip_core::{Database, RsclipPaths};

pub fn run(args: &[String]) -> Result<()> {
    let id = positional_i64(args, 0, "entry id")?;
    let pinned = !flag(args, "--off");
    let paths = RsclipPaths::discover()?;
    let db = Database::open(&paths.db_path)?;
    db.set_pinned(id, pinned)?;
    notify_changed(&paths);
    Ok(())
}
