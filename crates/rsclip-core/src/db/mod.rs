mod entries;
mod ocr;
mod rows;
mod schema;
mod secrets;

use anyhow::{Context, Result};
use rusqlite::Connection;

pub struct Database {
    conn: Connection,
}

impl Database {
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self> {
        if let Some(parent) = path.as_ref().parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating database directory {}", parent.display()))?;
        }
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let db = Self { conn };
        db.migrate()?;
        Ok(db)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::models::{EntryFilter, NewEntry, NewEntryData, SortMode};

    use super::Database;

    fn temp_db_path() -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "rsclip-core-db-test-{}-{unique}.sqlite",
            std::process::id()
        ))
    }

    fn text_entry(hash: &str, title: &str) -> NewEntry {
        let mut entry = NewEntry::new(
            hash.to_string(),
            "text/plain".to_string(),
            title.to_string(),
        );
        entry.preview_text = Some(title.to_string());
        entry.text_content = Some(title.to_string());
        entry.size_bytes = title.len() as i64;
        entry
    }

    fn image_entry(hash: &str, title: &str) -> NewEntry {
        let mut entry = NewEntry::new(hash.to_string(), "image/png".to_string(), title.to_string());
        entry.preview_text = Some(title.to_string());
        entry.text_content = Some(title.to_string());
        entry.size_bytes = title.len() as i64;
        entry.data = NewEntryData::Image {
            file_path: Some("/tmp/test.png".to_string()),
            thumb_path: None,
            ocr_text: None,
        };
        entry
    }

    #[test]
    fn database_entry_secret_and_ocr_smoke_test() {
        let path = temp_db_path();
        let db = Database::open(&path).unwrap();

        let entry_id = db
            .upsert_entry(&text_entry("hash-1", "secret text"))
            .unwrap();
        let entries = db
            .list_entries("", EntryFilter::All, SortMode::Default, 100)
            .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, entry_id);

        let entry = db.get_entry(entry_id).unwrap().unwrap();
        assert_eq!(entry.title, "secret text");

        let image_id = db
            .upsert_entry(&image_entry("hash-2", "ocr image"))
            .unwrap();
        db.save_ocr_result(image_id, "eng", "ocr body").unwrap();
        let entry = db.get_entry(image_id).unwrap().unwrap();
        if let crate::models::EntryData::Image { ocr_text, .. } = entry.data {
            assert_eq!(ocr_text.as_deref(), Some("ocr body"));
        } else {
            panic!("expected Image entry data");
        }

        let secret_id = db
            .save_secret(Some(entry_id), "Alias", "secret value")
            .unwrap();
        let secrets = db.list_secrets("", 100).unwrap();
        assert_eq!(secrets.len(), 1);
        assert_eq!(secrets[0].id, secret_id);
        assert_eq!(db.count_secrets("").unwrap(), 1);
        assert_eq!(db.list_secrets_page("", 1, 0).unwrap()[0].id, secret_id);

        db.rename_secret(secret_id, "Renamed").unwrap();
        let secret = db.list_secrets("Renamed", 100).unwrap().remove(0);
        assert_eq!(secret.alias, "Renamed");

        db.delete_secret(secret_id).unwrap();
        assert!(db.list_secrets("", 100).unwrap().is_empty());

        drop(db);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
        let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
    }

    #[test]
    fn list_entries_honors_large_requested_limit() {
        let path = temp_db_path();
        let db = Database::open(&path).unwrap();

        for index in 0..1005 {
            let title = format!("entry-{index}");
            db.conn
                .execute(
                    r#"
                    INSERT INTO entries (
                      content_hash, kind, mime_type, title, preview_text, text_content,
                      copied_at, updated_at, size_bytes
                    )
                    VALUES (?1, 'text', 'text/plain', ?2, ?2, ?2, ?3, ?3, ?4)
                    "#,
                    rusqlite::params![
                        format!("large-limit-hash-{index}"),
                        title,
                        index as i64,
                        index as i64,
                    ],
                )
                .unwrap();
        }

        let entries = db
            .list_entries("", EntryFilter::All, SortMode::Default, 1005)
            .unwrap();

        assert_eq!(entries.len(), 1005);
        assert_eq!(db.count_entries("", EntryFilter::All).unwrap(), 1005);

        let page = db
            .list_entries_page("", EntryFilter::All, SortMode::Default, 25, 200)
            .unwrap();
        assert_eq!(page.len(), 25);

        drop(db);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
        let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
    }

    #[test]
    fn cleanup_soft_deletes_only_old_unpinned_entries() {
        let path = temp_db_path();
        let db = Database::open(&path).unwrap();
        let now = chrono::Utc::now().timestamp();
        let old = now - 3 * 86_400;

        for (hash, title, updated_at, pinned) in [
            ("old-unpinned", "old unpinned", old, 0),
            ("old-pinned", "old pinned", old, 1),
            ("recent-unpinned", "recent unpinned", now, 0),
        ] {
            db.conn
                .execute(
                    r#"
                    INSERT INTO entries (
                      content_hash, kind, mime_type, title, preview_text, text_content,
                      pinned, copied_at, updated_at, size_bytes
                    )
                    VALUES (?1, 'text', 'text/plain', ?2, ?2, ?2, ?3, ?4, ?4, ?5)
                    "#,
                    rusqlite::params![hash, title, pinned, updated_at, title.len() as i64],
                )
                .unwrap();
        }

        let deleted = db.delete_unpinned_older_than_days(1).unwrap();
        let entries = db
            .list_entries(
                "",
                crate::models::EntryFilter::All,
                crate::models::SortMode::Default,
                10,
            )
            .unwrap();

        assert_eq!(deleted, 1);
        assert_eq!(entries.len(), 2);
        assert!(entries.iter().any(|entry| entry.title == "old pinned"));
        assert!(entries.iter().any(|entry| entry.title == "recent unpinned"));
        assert!(!entries.iter().any(|entry| entry.title == "old unpinned"));

        drop(db);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
        let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
    }

    #[test]
    fn migration_repairs_generated_https_links_from_bare_text() {
        let path = temp_db_path();
        let db = Database::open(&path).unwrap();

        db.conn
            .execute(
                r#"
                INSERT INTO entries (
                  content_hash, kind, mime_type, title, preview_text, text_content,
                  link_url, link_domain, link_icon, copied_at, updated_at, size_bytes
                )
                VALUES (?1, 'link', 'text/plain', ?2, ?3, ?4, ?3, ?2, 'globe', 1, 1, ?5)
                "#,
                rusqlite::params![
                    "hash-old-generated-link",
                    "fbs-admin-token-2026",
                    "https://fbsa_1f3a8e0389a8c0cbc656fca80307e478.fbs-admin-token-2026",
                    "fbsa_1f3a8e0389a8c0cbc656fca80307e478.fbs-admin-token-2026",
                    68_i64,
                ],
            )
            .unwrap();

        db.migrate().unwrap();

        let entries = db
            .list_entries("", EntryFilter::All, SortMode::Default, 100)
            .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0].title,
            "fbsa_1f3a8e0389a8c0cbc656fca80307e478.fbs-admin-token-2026"
        );
        assert!(matches!(entries[0].data, crate::models::EntryData::Text));

        drop(db);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
        let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
    }

    #[test]
    fn migration_adds_columns_to_existing_database() {
        let path = temp_db_path();
        {
            let conn = rusqlite::Connection::open(&path).unwrap();
            conn.execute_batch(
                r#"
                CREATE TABLE entries (
                  id INTEGER PRIMARY KEY AUTOINCREMENT,
                  content_hash TEXT NOT NULL UNIQUE,
                  kind TEXT NOT NULL,
                  mime_type TEXT NOT NULL,
                  title TEXT NOT NULL,
                  preview_text TEXT,
                  text_content TEXT,
                  pinned INTEGER NOT NULL DEFAULT 0,
                  copied_at INTEGER NOT NULL,
                  updated_at INTEGER NOT NULL
                );
                INSERT INTO entries (
                  content_hash, kind, mime_type, title, preview_text, text_content,
                  copied_at, updated_at
                )
                VALUES ('old-hash', 'text', 'text/plain', 'old', 'old', 'old', 1, 1);

                CREATE TABLE secrets (
                  id INTEGER PRIMARY KEY AUTOINCREMENT,
                  source_entry_id INTEGER UNIQUE,
                  alias TEXT NOT NULL,
                  value TEXT NOT NULL,
                  created_at INTEGER NOT NULL,
                  updated_at INTEGER NOT NULL
                );
                "#,
            )
            .unwrap();
        }

        let db = Database::open(&path).unwrap();
        let entries = db
            .list_entries("", EntryFilter::All, SortMode::Default, 10)
            .unwrap();

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].title, "old");

        drop(db);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
        let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
    }
}
