use anyhow::Result;
use rsclip_core::cli::parse_list_entries_args;
use rsclip_core::{Database, RsclipPaths};

pub fn run(args: &[String]) -> Result<()> {
    let list_args = parse_list_entries_args(args);
    let paths = RsclipPaths::discover()?;
    let db = Database::open(&paths.db_path)?;
    let entries = db.list_entries(
        list_args.query,
        list_args.filter,
        list_args.sort,
        list_args.limit,
    )?;
    crate::output::print_entries(&entries, list_args.json)
}
