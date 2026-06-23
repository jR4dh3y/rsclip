use std::fs;
use std::io::Write;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, bail};

use crate::models::{ClipboardEntry, EntryData};

pub fn copy_entry(entry: &ClipboardEntry) -> Result<()> {
    copy_entry_with_writer(entry, write_clipboard)
}

fn copy_entry_with_writer(
    entry: &ClipboardEntry,
    mut write: impl FnMut(&str, &[u8]) -> Result<()>,
) -> Result<()> {
    match &entry.data {
        EntryData::Image { file_path, .. } => {
            let bytes = fs::read(file_path).with_context(|| format!("reading {file_path}"))?;
            write(&entry.mime_type, &bytes)
        }
        EntryData::File { .. } => {
            let text = entry
                .text_content
                .as_deref()
                .context("file entry has no URI-list content")?;
            write("text/uri-list", text.as_bytes())
        }
        _ => {
            let text = entry
                .text_content
                .as_deref()
                .or(entry.preview_text.as_deref())
                .context("entry has no text content")?;
            write("text/plain", text.as_bytes())
        }
    }
}

pub fn paste_entry(entry: &ClipboardEntry, auto_paste: bool, delay_ms: u64) -> Result<()> {
    paste_entry_with_method(entry, auto_paste, delay_ms, "wtype")
}

pub fn paste_entry_with_method(
    entry: &ClipboardEntry,
    auto_paste: bool,
    delay_ms: u64,
    method: &str,
) -> Result<()> {
    copy_entry(entry)?;
    if auto_paste {
        thread::sleep(Duration::from_millis(delay_ms));
        trigger_paste_with_method(method)?;
    }
    Ok(())
}

pub fn write_clipboard(mime_type: &str, bytes: &[u8]) -> Result<()> {
    let mut child = Command::new("wl-copy")
        .arg("--type")
        .arg(mime_type)
        .stdin(Stdio::piped())
        .spawn()
        .context("spawning wl-copy")?;
    child
        .stdin
        .as_mut()
        .context("opening wl-copy stdin")?
        .write_all(bytes)
        .context("writing clipboard payload")?;
    let status = child.wait().context("waiting for wl-copy")?;
    if !status.success() {
        bail!("wl-copy exited with {status}");
    }
    Ok(())
}

pub fn trigger_paste() -> Result<()> {
    trigger_paste_with_method("wtype")
}

pub fn trigger_paste_with_method(method: &str) -> Result<()> {
    match method.trim() {
        "wtype" => trigger_paste_wtype(),
        "" => bail!("paste method is empty"),
        other => bail!("unsupported paste method '{other}'; supported method: wtype"),
    }
}

fn trigger_paste_wtype() -> Result<()> {
    let status = Command::new("wtype")
        .args(["-M", "ctrl", "v", "-m", "ctrl"])
        .status()
        .context("spawning wtype")?;
    if !status.success() {
        bail!("wtype exited with {status}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::copy_entry_with_writer;
    use crate::models::ClipboardEntry;

    #[test]
    fn file_entry_writes_uri_list_mime_and_bytes() {
        let entry = ClipboardEntry::test_file(1, "a.txt", Some("file:///tmp/a.txt\r\n"));
        let mut writes = Vec::new();

        copy_entry_with_writer(&entry, |mime, bytes| {
            writes.push((mime.to_string(), bytes.to_vec()));
            Ok(())
        })
        .unwrap();

        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0].0, "text/uri-list");
        assert_eq!(writes[0].1, b"file:///tmp/a.txt\r\n");
    }

    #[test]
    fn file_entry_without_text_content_returns_clear_error() {
        let entry = ClipboardEntry::test_file(1, "a.txt", None);
        let err = copy_entry_with_writer(&entry, |_, _| Ok(())).unwrap_err();

        assert!(
            err.to_string()
                .contains("file entry has no URI-list content")
        );
    }
}
