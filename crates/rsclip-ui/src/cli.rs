use anyhow::{Result, bail};
use rsclip_core::cli::{parse_list_entries_args, print_entries};
use rsclip_core::{Database, RsclipPaths};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum UiCommand {
    Show,
    Toggle,
    Preload,
    QuitUi,
    List(Vec<String>),
    Help,
}

pub(crate) fn parse_args(args: &[String]) -> Result<UiCommand> {
    match args.first().map(String::as_str) {
        None | Some("show") => Ok(UiCommand::Show),
        Some("toggle") => Ok(UiCommand::Toggle),
        Some("preload") => Ok(UiCommand::Preload),
        Some("quit-ui") => Ok(UiCommand::QuitUi),
        Some("list") => Ok(UiCommand::List(args[1..].to_vec())),
        Some("help" | "--help" | "-h") => Ok(UiCommand::Help),
        Some(command) => bail!("unknown command: {command}"),
    }
}

pub(crate) fn print_help() {
    println!(
        r#"rsclip

Commands:
  rsclip                  Show the resident clipboard UI
  rsclip show             Show the resident clipboard UI
  rsclip toggle           Toggle the resident clipboard UI
  rsclip preload          Start and warm the resident UI without showing it
  rsclip quit-ui          Quit the resident UI process
  rsclip list [options]   List clipboard history

List options:
  --json                  Print JSON
  --query <query>         Filter by query
  --filter <filter>       all, text, images, files, links, colors, pinned
  --sort <sort>           default, recent/newest, oldest, most-used
  --limit <limit>         Maximum entries to print
"#
    );
}

pub(crate) fn cmd_list(args: &[String]) -> Result<()> {
    let list_args = parse_list_entries_args(args);
    let paths = RsclipPaths::discover()?;
    let db = Database::open(&paths.db_path)?;
    let entries = db.list_entries(
        list_args.query,
        list_args.filter,
        list_args.sort,
        list_args.limit,
    )?;
    print_entries(&entries, list_args.json)
}

#[cfg(test)]
mod tests {
    use super::{UiCommand, parse_args};

    fn make_args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn parses_empty_args_as_show() {
        assert_eq!(parse_args(&[]).unwrap(), UiCommand::Show);
    }

    #[test]
    fn parses_show() {
        assert_eq!(parse_args(&make_args(&["show"])).unwrap(), UiCommand::Show);
    }

    #[test]
    fn parses_toggle() {
        assert_eq!(
            parse_args(&make_args(&["toggle"])).unwrap(),
            UiCommand::Toggle
        );
    }

    #[test]
    fn parses_preload() {
        assert_eq!(
            parse_args(&make_args(&["preload"])).unwrap(),
            UiCommand::Preload
        );
    }

    #[test]
    fn parses_quit_ui() {
        assert_eq!(
            parse_args(&make_args(&["quit-ui"])).unwrap(),
            UiCommand::QuitUi
        );
    }

    #[test]
    fn parses_list_passthrough() {
        assert_eq!(
            parse_args(&make_args(&["list", "--json", "--query", "foo"])).unwrap(),
            UiCommand::List(make_args(&["--json", "--query", "foo"]))
        );
    }

    #[test]
    fn parses_help_aliases() {
        for alias in ["help", "--help", "-h"] {
            assert_eq!(parse_args(&make_args(&[alias])).unwrap(), UiCommand::Help);
        }
    }

    #[test]
    fn rejects_unknown_command() {
        assert!(parse_args(&make_args(&["missing"])).is_err());
    }
}
