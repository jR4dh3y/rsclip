mod delete;
mod favicons;
mod list;
mod ocr;
mod paste;
mod pin;
mod store;

use anyhow::{Result, bail};

pub fn run(args: &[String]) -> Result<()> {
    match args.first().map(String::as_str) {
        Some("store") => store::run(&args[1..]),
        Some("list") => list::run(&args[1..]),
        Some("watch") | None => crate::watcher::run_watchers(),
        Some("pin") => pin::run(&args[1..]),
        Some("delete") => delete::run(&args[1..]),
        Some("paste") => paste::run(&args[1..]),
        Some("ocr") => ocr::run(&args[1..]),
        Some("favicons") => favicons::run(&args[1..]),
        Some("help" | "--help" | "-h") => {
            print_help();
            Ok(())
        }
        Some(command) => bail!("unknown command: {command}"),
    }
}

fn print_help() {
    println!(
        r#"rsclipd

Commands:
  watch                              Start wl-paste watchers
  store --mime text/plain            Store stdin as a clipboard entry
  list [--json] [--query q]           List history
  pin <id> [--off]                   Pin or unpin an entry
  delete <id>                        Soft-delete an entry
  paste <id> [--copy-only] [--delay-ms <ms>]
                                      Restore an entry and optionally paste
  ocr <id> [--lang <lang>]           Run configured OCR for an image entry
  favicons clear                     Clear cached favicons and failed-domain records
  favicons refresh                   Clear and queue favicons for stored link domains
"#
    );
}
