use anyhow::{Result, bail};
use rsclip_core::notify::notify_favicons_changed;
use rsclip_core::{Database, RsclipPaths, favicons};

pub fn run(args: &[String]) -> Result<()> {
    match args.first().map(String::as_str) {
        Some("clear") => clear(),
        Some("refresh") => refresh(),
        Some(command) => bail!("unknown favicons command: {command}"),
        None => bail!("missing favicons command"),
    }
}

fn clear() -> Result<()> {
    let paths = RsclipPaths::discover()?;
    paths.ensure()?;
    clear_paths(&paths)?;
    notify_favicons_changed(&paths);
    println!("cleared favicon cache");
    Ok(())
}

fn clear_paths(paths: &RsclipPaths) -> Result<()> {
    favicons::clear_cache(paths)?;
    Ok(())
}

fn refresh() -> Result<()> {
    let paths = RsclipPaths::discover()?;
    paths.ensure()?;
    let count = refresh_paths(&paths)?;
    notify_favicons_changed(&paths);
    println!("queued favicon refresh for {count} domains");
    Ok(())
}

fn refresh_paths(paths: &RsclipPaths) -> Result<usize> {
    let db = Database::open(&paths.db_path)?;
    let domains = db.list_link_domains()?;
    favicons::clear_cache(paths)?;
    for domain in &domains {
        favicons::enqueue_domain(paths, domain)?;
    }
    Ok(domains.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_paths(name: &str) -> RsclipPaths {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after Unix epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "rsclip-favicons-command-test-{name}-{}-{unique}",
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
    fn clear_paths_clears_temp_favicon_directory() {
        let paths = test_paths("clear");
        paths.ensure().unwrap();
        fs::write(paths.favicon_icon_dir.join("icon.png"), b"png").unwrap();
        fs::write(paths.favicon_miss_dir.join("domain.miss"), b"miss").unwrap();
        fs::write(paths.favicon_queue_dir.join("domain.json"), b"{}").unwrap();

        clear_paths(&paths).unwrap();

        assert!(
            fs::read_dir(&paths.favicon_icon_dir)
                .unwrap()
                .next()
                .is_none()
        );
        assert!(
            fs::read_dir(&paths.favicon_miss_dir)
                .unwrap()
                .next()
                .is_none()
        );
        assert!(
            fs::read_dir(&paths.favicon_queue_dir)
                .unwrap()
                .next()
                .is_none()
        );
    }

    #[test]
    fn refresh_paths_queues_stored_link_domains() {
        let paths = test_paths("refresh");
        paths.ensure().unwrap();
        let db = Database::open(&paths.db_path).unwrap();

        let mut first = rsclip_core::NewEntry::new(
            "hash-1".to_string(),
            "text/plain".to_string(),
            "github.com".to_string(),
        );
        first.data = rsclip_core::NewEntryData::Link {
            url: "https://github.com/openai".to_string(),
            domain: "github.com".to_string(),
            icon: "github".to_string(),
        };
        db.upsert_entry(&first).unwrap();

        let mut duplicate = rsclip_core::NewEntry::new(
            "hash-2".to_string(),
            "text/plain".to_string(),
            "github.com".to_string(),
        );
        duplicate.data = rsclip_core::NewEntryData::Link {
            url: "https://github.com/rust-lang".to_string(),
            domain: "github.com".to_string(),
            icon: "github".to_string(),
        };
        db.upsert_entry(&duplicate).unwrap();

        let mut second = rsclip_core::NewEntry::new(
            "hash-3".to_string(),
            "text/plain".to_string(),
            "docs.rs".to_string(),
        );
        second.data = rsclip_core::NewEntryData::Link {
            url: "https://docs.rs/anyhow".to_string(),
            domain: "docs.rs".to_string(),
            icon: "rust".to_string(),
        };
        db.upsert_entry(&second).unwrap();

        fs::write(paths.favicon_icon_dir.join("stale.png"), b"png").unwrap();
        fs::write(paths.favicon_miss_dir.join("stale.miss"), b"miss").unwrap();

        let count = refresh_paths(&paths).unwrap();

        assert_eq!(count, 2);
        assert!(favicons::queue_path(&paths, "github.com").exists());
        assert!(favicons::queue_path(&paths, "docs.rs").exists());
        assert!(!paths.favicon_icon_dir.join("stale.png").exists());
        assert!(!paths.favicon_miss_dir.join("stale.miss").exists());
    }
}
