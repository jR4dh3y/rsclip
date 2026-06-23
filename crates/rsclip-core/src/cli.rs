use std::io::{self, Write};

use anyhow::{Context, Result};

use crate::models::{ClipboardEntry, EntryFilter, SortMode};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ListEntriesArgs<'a> {
    pub query: &'a str,
    pub filter: EntryFilter,
    pub sort: SortMode,
    pub limit: usize,
    pub json: bool,
}

pub fn parse_list_entries_args(args: &[String]) -> ListEntriesArgs<'_> {
    ListEntriesArgs {
        query: option_value(args, "--query").unwrap_or(""),
        filter: EntryFilter::parse(option_value(args, "--filter").unwrap_or("all")),
        sort: SortMode::parse(option_value(args, "--sort").unwrap_or("default")),
        limit: option_value(args, "--limit")
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(100),
        json: flag(args, "--json"),
    }
}

pub fn option_value<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.windows(2)
        .find(|window| window[0] == name)
        .map(|window| window[1].as_str())
}

pub fn flag(args: &[String], name: &str) -> bool {
    args.iter().any(|arg| arg == name)
}

pub fn positional_i64(args: &[String], index: usize, label: &str) -> Result<i64> {
    args.iter()
        .filter(|arg| !arg.starts_with('-'))
        .nth(index)
        .with_context(|| format!("missing {label}"))?
        .parse::<i64>()
        .with_context(|| format!("invalid {label}"))
}

pub fn print_entries(entries: &[ClipboardEntry], json: bool) -> Result<()> {
    let stdout = io::stdout();
    let mut writer = stdout.lock();
    write_entries(&mut writer, entries, json)
}

pub fn write_entries(
    writer: &mut impl Write,
    entries: &[ClipboardEntry],
    json: bool,
) -> Result<()> {
    if json {
        writeln!(writer, "{}", serde_json::to_string_pretty(entries)?)?;
    } else {
        for entry in entries {
            writeln!(
                writer,
                "#{:<4} {:<6} {:<1} {}",
                entry.id,
                entry.kind,
                if entry.pinned { "P" } else { " " },
                entry.title
            )?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::models::{ClipboardEntry, EntryFilter, SortMode};

    use super::{flag, option_value, parse_list_entries_args, positional_i64, write_entries};

    fn make_args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    fn text_entry() -> ClipboardEntry {
        let mut entry = ClipboardEntry::test_text(7, "Title");
        entry.pinned = true;
        entry
    }

    #[test]
    fn parses_option_values_and_flags() {
        let args = make_args(&["--query", "needle", "--json"]);
        assert_eq!(option_value(&args, "--query"), Some("needle"));
        assert!(flag(&args, "--json"));
        assert!(!flag(&args, "--missing"));
    }

    #[test]
    fn parses_list_defaults() {
        let parsed = parse_list_entries_args(&[]);
        assert_eq!(parsed.query, "");
        assert_eq!(parsed.filter, EntryFilter::All);
        assert_eq!(parsed.sort, SortMode::Default);
        assert_eq!(parsed.limit, 100);
        assert!(!parsed.json);
    }

    #[test]
    fn parses_list_overrides() {
        let args = make_args(&[
            "--query",
            "needle",
            "--filter",
            "images",
            "--sort",
            "most-used",
            "--limit",
            "25",
            "--json",
        ]);
        let parsed = parse_list_entries_args(&args);
        assert_eq!(parsed.query, "needle");
        assert_eq!(parsed.filter, EntryFilter::Images);
        assert_eq!(parsed.sort, SortMode::MostUsed);
        assert_eq!(parsed.limit, 25);
        assert!(parsed.json);
    }

    #[test]
    fn parses_files_filter() {
        let args = make_args(&["--filter", "files"]);
        let parsed = parse_list_entries_args(&args);

        assert_eq!(parsed.filter, EntryFilter::Files);
    }

    #[test]
    fn parses_positional_ids() {
        let args = make_args(&["42", "--off"]);
        assert_eq!(positional_i64(&args, 0, "entry id").unwrap(), 42);
        assert!(positional_i64(&make_args(&["abc"]), 0, "entry id").is_err());
        assert!(positional_i64(&[], 0, "entry id").is_err());
    }

    #[test]
    fn writes_plain_entries() {
        let mut output = Vec::new();
        write_entries(&mut output, &[text_entry()], false).unwrap();
        assert_eq!(String::from_utf8(output).unwrap(), "#7    text P Title\n");
    }
}
