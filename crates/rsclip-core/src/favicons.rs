use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::Serialize;

use crate::config::RsclipPaths;

#[derive(Serialize)]
struct QueueJob<'a> {
    domain: &'a str,
}

/// Computes a stable hex BLAKE3 hash of the domain name for cache filenames.
pub fn domain_cache_key(domain: &str) -> String {
    blake3::hash(domain.as_bytes()).to_hex().to_string()
}

/// Returns the filesystem path for a cached PNG favicon.
pub fn icon_path(paths: &RsclipPaths, domain: &str) -> PathBuf {
    paths
        .favicon_icon_dir
        .join(format!("{}.png", domain_cache_key(domain)))
}

/// Returns the filesystem path for a recorded domain miss (failed fetch).
pub fn miss_path(paths: &RsclipPaths, domain: &str) -> PathBuf {
    paths
        .favicon_miss_dir
        .join(format!("{}.miss", domain_cache_key(domain)))
}

/// Returns the filesystem path for a queued domain fetch job.
pub fn queue_path(paths: &RsclipPaths, domain: &str) -> PathBuf {
    paths
        .favicon_queue_dir
        .join(format!("{}.json", domain_cache_key(domain)))
}

/// Returns the cached icon path if it currently exists on disk.
pub fn cached_icon_path(paths: &RsclipPaths, domain: &str) -> Option<PathBuf> {
    let path = icon_path(paths, domain);
    path.exists().then_some(path)
}

/// Returns true if this domain has neither an existing icon nor a recorded miss.
pub fn should_enqueue(paths: &RsclipPaths, domain: &str) -> bool {
    !icon_path(paths, domain).exists() && !miss_path(paths, domain).exists()
}

/// Adds a domain to the background favicon download queue if not already fetched or missed.
pub fn enqueue_domain(paths: &RsclipPaths, domain: &str) -> Result<()> {
    if !should_enqueue(paths, domain) {
        return Ok(());
    }

    fs::create_dir_all(&paths.favicon_queue_dir)
        .with_context(|| format!("creating {}", paths.favicon_queue_dir.display()))?;

    let path = queue_path(paths, domain);
    if path.exists() {
        return Ok(());
    }

    let tmp_path = path.with_extension(format!("json.tmp.{}", std::process::id()));
    let contents = serde_json::to_vec_pretty(&QueueJob { domain })?;
    fs::write(&tmp_path, contents).with_context(|| format!("writing {}", tmp_path.display()))?;
    match fs::rename(&tmp_path, &path) {
        Ok(()) => Ok(()),
        Err(_err) if path.exists() => {
            let _ = fs::remove_file(&tmp_path);
            Ok(())
        }
        Err(err) => Err(err).with_context(|| format!("renaming {}", path.display())),
    }
}

pub fn clear_cache(paths: &RsclipPaths) -> Result<()> {
    for dir in [
        &paths.favicon_icon_dir,
        &paths.favicon_miss_dir,
        &paths.favicon_queue_dir,
    ] {
        match fs::remove_dir_all(dir) {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(err).with_context(|| format!("removing {}", dir.display())),
        }
        fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_paths(name: &str) -> RsclipPaths {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after Unix epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "rsclip-favicon-test-{name}-{}-{unique}",
            std::process::id()
        ));
        RsclipPaths {
            config_dir: root.join("config"),
            state_dir: root.join("state"),
            data_dir: root.join("data"),
            db_path: root.join("state").join("rsclip.db"),
            image_dir: root.join("data").join("images"),
            thumb_dir: root.join("data").join("thumbs"),
            ocr_dir: root.join("data").join("ocr"),
            favicon_dir: root.join("data").join("favicons"),
            favicon_icon_dir: root.join("data").join("favicons").join("icons"),
            favicon_queue_dir: root.join("data").join("favicons").join("queue"),
            favicon_miss_dir: root.join("data").join("favicons").join("misses"),
            log_path: root.join("state").join("rsclip.log"),
            socket_path: root.join("rsclip.sock"),
        }
    }

    #[test]
    fn domain_cache_key_is_stable() {
        assert_eq!(
            domain_cache_key("github.com"),
            domain_cache_key("github.com")
        );
        assert_ne!(domain_cache_key("github.com"), domain_cache_key("docs.rs"));
    }

    #[test]
    fn cache_paths_point_under_favicon_dirs() {
        let paths = test_paths("paths");
        assert!(icon_path(&paths, "github.com").starts_with(&paths.favicon_icon_dir));
        assert!(miss_path(&paths, "github.com").starts_with(&paths.favicon_miss_dir));
        assert!(queue_path(&paths, "github.com").starts_with(&paths.favicon_queue_dir));
    }

    #[test]
    fn should_enqueue_is_false_when_icon_exists() {
        let paths = test_paths("icon-exists");
        fs::create_dir_all(&paths.favicon_icon_dir).unwrap();
        fs::write(icon_path(&paths, "github.com"), b"png").unwrap();

        assert!(!should_enqueue(&paths, "github.com"));
    }

    #[test]
    fn should_enqueue_is_false_when_miss_exists() {
        let paths = test_paths("miss-exists");
        fs::create_dir_all(&paths.favicon_miss_dir).unwrap();
        fs::write(miss_path(&paths, "github.com"), b"miss").unwrap();

        assert!(!should_enqueue(&paths, "github.com"));
    }

    #[test]
    fn enqueue_domain_creates_queue_json_and_is_idempotent() {
        let paths = test_paths("enqueue");

        enqueue_domain(&paths, "github.com").unwrap();
        enqueue_domain(&paths, "github.com").unwrap();

        let path = queue_path(&paths, "github.com");
        let contents = fs::read_to_string(path).unwrap();
        assert!(contents.contains(r#""domain": "github.com""#));
    }

    #[test]
    fn clear_cache_removes_icons_misses_and_queue_entries() {
        let paths = test_paths("clear");
        paths.ensure().unwrap();
        fs::write(icon_path(&paths, "github.com"), b"png").unwrap();
        fs::write(miss_path(&paths, "github.com"), b"miss").unwrap();
        fs::write(queue_path(&paths, "github.com"), b"{}").unwrap();

        clear_cache(&paths).unwrap();

        assert!(!icon_path(&paths, "github.com").exists());
        assert!(!miss_path(&paths, "github.com").exists());
        assert!(!queue_path(&paths, "github.com").exists());
        assert!(paths.favicon_icon_dir.is_dir());
        assert!(paths.favicon_miss_dir.is_dir());
        assert!(paths.favicon_queue_dir.is_dir());
    }
}
