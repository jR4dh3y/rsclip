use std::io::{self, Read};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use rsclip_core::cli::option_value;
use rsclip_core::favicons;
use rsclip_core::files::normalize_path_or_uri_list;
use rsclip_core::notify::notify_changed;
use rsclip_core::storage::{content_hash, store_image};
use rsclip_core::{AppConfig, Database, NewEntryData, RsclipPaths, classify_payload};
use tracing::{info, warn};

const FILE_TEXT_DUPLICATE_WINDOW_SECS: i64 = 5;

pub fn run(args: &[String]) -> Result<()> {
    let mime_type = option_value(args, "--mime").unwrap_or("text/plain");
    let paths = RsclipPaths::discover()?;
    paths.ensure()?;
    let config = AppConfig::load(&paths)?;
    let limit = payload_limit(mime_type, &config);
    let mut payload = Vec::new();
    let exceeded = read_payload(limit, &mut payload)?;

    if payload.is_empty() {
        return Ok(());
    }

    if exceeded && let Some(limit) = limit {
        info!(
            "skipping {mime_type} clipboard payload: {} bytes exceeds configured limit {limit}",
            payload.len()
        );
        return Ok(());
    }

    let db = Database::open(&paths.db_path)?;
    if should_skip_text_file_duplicate(mime_type, &payload, &db)? {
        info!("skipping text/plain duplicate for recent file clipboard entry");
        return Ok(());
    }

    let hash = content_hash(&payload);
    let entry_hash = if config.history.dedupe {
        hash.clone()
    } else {
        unique_entry_hash(&hash)
    };
    let mut entry = classify_payload(mime_type, entry_hash, &payload)?;
    if mime_type.starts_with("image/") {
        let path = store_image(&paths, &hash, mime_type, &payload)?;
        if let NewEntryData::Image {
            file_path,
            thumb_path: _,
            ocr_text: _,
        } = &mut entry.data
        {
            *file_path = Some(path.to_string_lossy().to_string());
        }
    }

    let id = db.upsert_entry(&entry)?;
    if config.ocr.enabled
        && config.ocr.auto_index
        && let NewEntryData::Image {
            file_path: Some(file_path),
            ..
        } = &entry.data
    {
        match rsclip_core::ocr::run_tesseract_with_options(
            file_path,
            &config.ocr.default_language,
            &config.ocr.command,
            config.ocr.timeout_seconds,
        ) {
            Ok(text) => db.save_ocr_result(id, &config.ocr.default_language, &text)?,
            Err(err) => warn!("auto OCR failed for entry {id}: {err:#}"),
        }
    }
    if config.history.cleanup_unpinned_after_days > 0 {
        let _ = db.delete_unpinned_older_than_days(config.history.cleanup_unpinned_after_days)?;
    }
    if config.links.favicon_cache
        && let NewEntryData::Link { domain, .. } = &entry.data
    {
        favicons::enqueue_domain(&paths, domain)?;
    }
    notify_changed(&paths);
    println!("{id}");
    Ok(())
}

fn should_skip_text_file_duplicate(mime_type: &str, payload: &[u8], db: &Database) -> Result<bool> {
    if mime_type != "text/plain" {
        return Ok(false);
    }

    let Ok(text) = std::str::from_utf8(payload) else {
        return Ok(false);
    };
    let Some(normalized_uri_list) = normalize_path_or_uri_list(text) else {
        return Ok(false);
    };

    db.has_recent_file_uri_list(&normalized_uri_list, FILE_TEXT_DUPLICATE_WINDOW_SECS)
}

fn payload_limit(mime_type: &str, config: &AppConfig) -> Option<usize> {
    let limit = if mime_type.starts_with("image/") {
        config.history.max_image_bytes
    } else {
        config.history.max_text_bytes
    };
    (limit > 0).then_some(limit)
}

fn read_payload(limit: Option<usize>, payload: &mut Vec<u8>) -> Result<bool> {
    let mut stdin = io::stdin();
    if let Some(limit) = limit {
        stdin
            .by_ref()
            .take(limit.saturating_add(1) as u64)
            .read_to_end(payload)
            .context("reading clipboard payload from stdin")?;
        Ok(payload.len() > limit)
    } else {
        stdin
            .read_to_end(payload)
            .context("reading clipboard payload from stdin")?;
        Ok(false)
    }
}

fn unique_entry_hash(hash: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    format!("{hash}-{}-{nanos}", std::process::id())
}
