use std::path::{Path, PathBuf};

use url::Url;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileReference {
    pub uri: String,
    pub path: PathBuf,
}

/// Maximum file entries parsed for UI previews.
///
/// File previews run on the GTK thread, and a legacy row can hold thousands
/// of URIs where each entry costs URL parsing, allocation, and a filesystem
/// `exists()` check. Cap the work and report truncation instead.
pub const URI_LIST_PREVIEW_MAX_FILES: usize = 500;

/// File entries parsed for a preview, with whether the input was truncated.
pub struct BoundedUriList {
    pub files: Vec<FileReference>,
    pub truncated: bool,
}

/// Parse at most `max_files` file references from at most `max_bytes` of payload.
///
/// The byte bound is applied at a line boundary so truncation never creates a
/// partial URI, and parsing stops after `max_files` valid entries.
/// `truncated` reports either bound being hit so callers can label partial
/// results instead of silently showing or copying a subset.
pub fn parse_uri_list_bounded(payload: &str, max_files: usize, max_bytes: usize) -> BoundedUriList {
    let bounded = truncate_to_line_boundary(payload, max_bytes);
    let mut truncated = bounded.len() < payload.len();
    let mut files = Vec::new();
    for line in uri_list_lines(bounded) {
        if files.len() >= max_files {
            truncated = true;
            break;
        }
        if let Some(file) = parse_file_uri(line) {
            files.push(file);
        }
    }
    BoundedUriList { files, truncated }
}

/// Truncate to `max_bytes` without splitting a UTF-8 code point or a line.
fn truncate_to_line_boundary(payload: &str, max_bytes: usize) -> &str {
    if payload.len() <= max_bytes {
        return payload;
    }
    let mut end = max_bytes;
    while !payload.is_char_boundary(end) {
        end -= 1;
    }
    match payload[..end].rfind('\n') {
        Some(pos) => &payload[..pos + 1],
        None => &payload[..end],
    }
}

pub fn parse_uri_list(payload: &str) -> Vec<FileReference> {
    uri_list_lines(payload).filter_map(parse_file_uri).collect()
}

pub fn normalize_uri_list(payload: &str) -> String {
    normalize_file_references(parse_uri_list(payload))
}

pub fn uri_list_title(payload: &str) -> String {
    let files = parse_uri_list(payload);
    match files.as_slice() {
        [] => "Files".to_string(),
        [file] => file_title(file),
        [first, rest @ ..] => format!("{} + {} more", file_title(first), rest.len()),
    }
}

pub fn uri_list_preview(payload: &str) -> String {
    parse_uri_list(payload)
        .iter()
        .map(|file| file.path.to_string_lossy().to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn uri_list_missing_count(payload: &str) -> usize {
    parse_uri_list(payload)
        .iter()
        .filter(|file| !file.path.exists())
        .count()
}

pub fn normalize_path_or_uri_list(payload: &str) -> Option<String> {
    let mut files = Vec::new();

    for line in payload
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        if line.starts_with("file://") {
            files.push(parse_file_uri(line)?);
            continue;
        }

        let path = Path::new(line);
        if !path.is_absolute() {
            return None;
        }

        let uri = Url::from_file_path(path).ok()?.to_string();
        files.push(FileReference {
            uri,
            path: path.to_path_buf(),
        });
    }

    (!files.is_empty()).then(|| normalize_file_references(files))
}

fn uri_list_lines(payload: &str) -> impl Iterator<Item = &str> {
    payload.lines().filter_map(|line| {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            None
        } else {
            Some(line)
        }
    })
}

fn parse_file_uri(line: &str) -> Option<FileReference> {
    let url = Url::parse(line).ok()?;
    if url.scheme() != "file" {
        return None;
    }

    let path = url.to_file_path().ok()?;
    let uri = Url::from_file_path(&path).ok()?.to_string();
    Some(FileReference { uri, path })
}

fn normalize_file_references(files: Vec<FileReference>) -> String {
    if files.is_empty() {
        return String::new();
    }

    let mut normalized = files
        .iter()
        .map(|file| file.uri.as_str())
        .collect::<Vec<_>>()
        .join("\r\n");
    normalized.push_str("\r\n");
    normalized
}

fn file_title(file: &FileReference) -> String {
    file.path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| {
            let path = file.path.to_string_lossy();
            if path.is_empty() {
                file.uri.clone()
            } else {
                path.to_string()
            }
        })
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use url::Url;

    use super::{
        normalize_path_or_uri_list, normalize_uri_list, parse_uri_list, parse_uri_list_bounded,
        uri_list_missing_count, uri_list_preview, uri_list_title,
    };

    fn file_uri(path: &std::path::Path) -> String {
        Url::from_file_path(path).unwrap().to_string()
    }

    fn unique_temp_path(name: &str) -> std::path::PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "rsclip-files-test-{}-{unique}-{name}",
            std::process::id()
        ))
    }

    #[test]
    fn parses_lf_and_crlf_uri_lists() {
        let payload = "file:///tmp/a.txt\nfile:///tmp/b.txt\r\nfile:///tmp/c.txt\r\n";
        let files = parse_uri_list(payload);

        assert_eq!(files.len(), 3);
        assert_eq!(files[0].path, std::path::PathBuf::from("/tmp/a.txt"));
        assert_eq!(files[1].path, std::path::PathBuf::from("/tmp/b.txt"));
        assert_eq!(files[2].path, std::path::PathBuf::from("/tmp/c.txt"));
    }

    #[test]
    fn ignores_comments_and_blank_lines() {
        let payload = "\n# copied files\nfile:///tmp/a.txt\n\n# trailing\n";
        let files = parse_uri_list(payload);

        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, std::path::PathBuf::from("/tmp/a.txt"));
    }

    #[test]
    fn percent_decodes_paths() {
        let payload = "file:///tmp/a%20b%23c.txt\r\n";
        let files = parse_uri_list(payload);

        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, std::path::PathBuf::from("/tmp/a b#c.txt"));
    }

    #[test]
    fn rejects_non_file_uris() {
        let payload = "https://example.com/a.txt\nfile:///tmp/a.txt\ntrash:///tmp/b.txt\n";
        let files = parse_uri_list(payload);

        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, std::path::PathBuf::from("/tmp/a.txt"));
    }

    #[test]
    fn normalizes_to_crlf_file_uri_lines() {
        let payload = "# comment\nfile:///tmp/a%20b.txt\nfile:///tmp/c.txt";

        assert_eq!(
            normalize_uri_list(payload),
            "file:///tmp/a%20b.txt\r\nfile:///tmp/c.txt\r\n"
        );
    }

    #[test]
    fn produces_single_and_multi_file_titles() {
        assert_eq!(uri_list_title("file:///tmp/a.txt"), "a.txt");
        assert_eq!(
            uri_list_title("file:///tmp/a.txt\nfile:///tmp/b.txt\nfile:///tmp/c.txt"),
            "a.txt + 2 more"
        );
    }

    #[test]
    fn previews_newline_separated_paths() {
        assert_eq!(
            uri_list_preview("file:///tmp/a.txt\r\nfile:///tmp/b.txt\r\n"),
            "/tmp/a.txt\n/tmp/b.txt"
        );
    }

    #[test]
    fn counts_missing_paths() {
        let existing_dir = unique_temp_path("dir");
        let existing_file = existing_dir.join("present.txt");
        let missing_file = existing_dir.join("missing.txt");
        fs::create_dir_all(&existing_dir).unwrap();
        fs::write(&existing_file, "present").unwrap();

        let payload = format!(
            "{}\r\n{}\r\n",
            file_uri(&existing_file),
            file_uri(&missing_file)
        );

        assert_eq!(uri_list_missing_count(&payload), 1);

        let _ = fs::remove_file(&existing_file);
        let _ = fs::remove_dir(&existing_dir);
    }

    #[test]
    fn normalizes_plain_absolute_paths_or_file_uris() {
        assert_eq!(
            normalize_path_or_uri_list("/tmp/a b.txt\nfile:///tmp/c.txt").unwrap(),
            "file:///tmp/a%20b.txt\r\nfile:///tmp/c.txt\r\n"
        );
        assert_eq!(normalize_path_or_uri_list("not a path"), None);
    }

    #[test]
    fn bounded_parse_returns_all_when_within_bounds() {
        let payload = "file:///tmp/a.txt\nfile:///tmp/b.txt\n";
        let bounded = parse_uri_list_bounded(payload, 500, 1024 * 1024);

        assert_eq!(bounded.files.len(), 2);
        assert!(!bounded.truncated);
    }

    #[test]
    fn bounded_parse_caps_file_count_and_reports_truncation() {
        let payload = (0..600)
            .map(|index| format!("file:///tmp/f{index}.txt"))
            .collect::<Vec<_>>()
            .join("\n");
        let bounded = parse_uri_list_bounded(&payload, 500, 1024 * 1024);

        assert_eq!(bounded.files.len(), 500);
        assert_eq!(
            bounded.files[0].path,
            std::path::PathBuf::from("/tmp/f0.txt")
        );
        assert!(bounded.truncated);
    }

    #[test]
    fn bounded_parse_truncates_bytes_at_line_boundary() {
        let payload = "file:///tmp/a.txt\nfile:///tmp/b.txt\nfile:///tmp/c.txt\n";
        // 20 bytes would split the second line, so parsing sees one line.
        let bounded = parse_uri_list_bounded(payload, 500, 20);

        assert!(bounded.truncated);
        assert_eq!(bounded.files.len(), 1);
        assert_eq!(
            bounded.files[0].path,
            std::path::PathBuf::from("/tmp/a.txt")
        );
    }

    #[test]
    fn bounded_parse_handles_empty_payload() {
        let bounded = parse_uri_list_bounded("", 500, 1024 * 1024);

        assert!(bounded.files.is_empty());
        assert!(!bounded.truncated);
    }
}
