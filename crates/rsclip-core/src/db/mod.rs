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

    /// Run `f` inside a single SQLite transaction (rollback on error).
    pub fn transaction<T>(&self, f: impl FnOnce(&Self) -> Result<T>) -> Result<T> {
        let tx = self.conn.unchecked_transaction()?;
        let value = f(self)?;
        tx.commit()?;
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::models::{
        ClipboardEntry, EntryData, EntryFilter, EntryKind, NewEntry, NewEntryData, SortMode,
    };

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

    fn file_entry(hash: &str, title: &str, uri_list: &str) -> NewEntry {
        let mut entry = NewEntry::new(
            hash.to_string(),
            "text/uri-list".to_string(),
            title.to_string(),
        );
        entry.preview_text = Some("/tmp/test.txt".to_string());
        entry.text_content = Some(uri_list.to_string());
        entry.size_bytes = uri_list.len() as i64;
        entry.data = NewEntryData::File { source_app: None };
        entry
    }

    fn assert_bounded_summary(entry: &ClipboardEntry) {
        let limit = super::entries::SUMMARY_TEXT_LIMIT_CHARS;
        assert!(entry.content_hash.chars().count() <= limit);
        assert!(entry.mime_type.chars().count() <= limit);
        assert!(entry.title.chars().count() <= limit);
        for value in [entry.preview_text.as_deref(), entry.text_content.as_deref()]
            .into_iter()
            .flatten()
        {
            assert!(value.chars().count() <= limit);
        }

        match &entry.data {
            EntryData::Image {
                file_path,
                thumb_path,
                ocr_text,
            } => {
                assert!(file_path.chars().count() <= limit);
                for value in [thumb_path.as_deref(), ocr_text.as_deref()]
                    .into_iter()
                    .flatten()
                {
                    assert!(value.chars().count() <= limit);
                }
            }
            EntryData::Link { url, domain, icon } => {
                assert!(url.chars().count() <= limit);
                assert!(domain.chars().count() <= limit);
                assert!(icon.chars().count() <= limit);
            }
            EntryData::Color { value, format } => {
                assert!(value.chars().count() <= limit);
                assert!(format.chars().count() <= limit);
            }
            EntryData::File { source_app } => {
                assert!(
                    source_app
                        .as_deref()
                        .is_none_or(|value| value.chars().count() <= limit)
                );
            }
            EntryData::Text | EntryData::Unknown => {}
        }

        if entry.kind == EntryKind::Text {
            assert!(entry.text_content.is_none());
        }
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
    fn entry_summaries_omit_text_payload_but_remain_searchable() {
        let path = temp_db_path();
        let db = Database::open(&path).unwrap();
        let entry_id = db
            .upsert_entry(&text_entry(
                "large-text-hash",
                "needle in a potentially large payload",
            ))
            .unwrap();

        let summaries = db
            .list_entry_summaries_page("needle", EntryFilter::All, SortMode::Default, 10, 0)
            .unwrap();

        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].id, entry_id);
        assert!(summaries[0].text_content.is_none());
        assert!(summaries[0].preview_text.is_some());
        assert!(
            db.get_entry(entry_id)
                .unwrap()
                .unwrap()
                .text_content
                .is_some()
        );

        drop(db);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
        let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
    }

    #[test]
    fn entry_summaries_bound_legacy_payloads_without_changing_search_or_full_rows() {
        let path = temp_db_path();
        let db = Database::open(&path).unwrap();
        let legacy_payload = |marker: &str| format!("{}:{marker}", "x".repeat(64 * 1024));

        let mut text = text_entry("legacy-text", "legacy text");
        text.preview_text = Some(legacy_payload("shared-needle"));
        text.text_content = Some(legacy_payload("shared-needle-text"));
        let text_id = db.upsert_entry(&text).unwrap();

        let mut image = image_entry("legacy-image", "legacy image");
        image.preview_text = Some(legacy_payload("shared-needle"));
        image.text_content = Some(legacy_payload("shared-needle-image"));
        let image_id = db.upsert_entry(&image).unwrap();
        let ocr = legacy_payload("shared-needle-ocr");
        db.save_ocr_result(image_id, "eng", &ocr).unwrap();

        let mut file = file_entry("legacy-file", "legacy file", "legacy-file");
        file.preview_text = Some(legacy_payload("shared-needle"));
        file.text_content = Some(legacy_payload("shared-needle-file"));
        let file_id = db.upsert_entry(&file).unwrap();

        let mut link = NewEntry::new(
            "legacy-link".to_string(),
            "text/plain".to_string(),
            legacy_payload("shared-needle-link-title"),
        );
        let link_url = legacy_payload("shared-needle-link-url");
        let link_domain = legacy_payload("shared-needle-link-domain");
        let link_icon = legacy_payload("shared-needle-link-icon");
        link.preview_text = Some(legacy_payload("shared-needle-link-preview"));
        link.text_content = Some(legacy_payload("shared-needle-link-text"));
        link.data = NewEntryData::Link {
            url: link_url.clone(),
            domain: link_domain.clone(),
            icon: link_icon.clone(),
        };
        let link_id = db.upsert_entry(&link).unwrap();

        assert_eq!(
            db.count_entries("shared-needle", EntryFilter::All).unwrap(),
            4
        );
        assert_eq!(
            db.count_entries("shared-needle-ocr", EntryFilter::Images)
                .unwrap(),
            1
        );
        assert_eq!(
            db.count_entries("shared-needle-link-url", EntryFilter::Links)
                .unwrap(),
            1
        );

        let page = db
            .list_entry_summaries_page("shared-needle", EntryFilter::All, SortMode::Default, 10, 0)
            .unwrap();
        assert_eq!(page.len(), 4);
        for entry in &page {
            assert_bounded_summary(entry);
        }

        let link_page = db
            .list_entry_summaries_page(
                "shared-needle-link-url",
                EntryFilter::Links,
                SortMode::Default,
                10,
                0,
            )
            .unwrap();
        assert_eq!(link_page.len(), 1);
        assert_bounded_summary(&link_page[0]);

        assert_eq!(
            db.get_entry(text_id)
                .unwrap()
                .unwrap()
                .text_content
                .unwrap()
                .len(),
            64 * 1024 + 1 + "shared-needle-text".len()
        );
        assert_eq!(
            db.get_entry(image_id).unwrap().unwrap().data,
            EntryData::Image {
                file_path: "/tmp/test.png".to_string(),
                thumb_path: None,
                ocr_text: Some(ocr),
            }
        );
        assert_eq!(
            db.get_entry(file_id)
                .unwrap()
                .unwrap()
                .text_content
                .unwrap()
                .len(),
            64 * 1024 + 1 + "shared-needle-file".len()
        );
        match db.get_entry(link_id).unwrap().unwrap().data {
            EntryData::Link { url, domain, icon } => {
                assert_eq!(url, link_url);
                assert_eq!(domain, link_domain);
                assert_eq!(icon, link_icon);
            }
            _ => panic!("expected full link entry"),
        }

        drop(db);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
        let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
    }

    #[test]
    fn file_filter_returns_only_file_entries() {
        let path = temp_db_path();
        let db = Database::open(&path).unwrap();

        db.upsert_entry(&text_entry("hash-text", "plain text"))
            .unwrap();
        let file_uri_list = "file:///tmp/test.txt\r\n";
        db.upsert_entry(&file_entry("hash-file", "test.txt", file_uri_list))
            .unwrap();

        let entries = db
            .list_entries("", EntryFilter::Files, SortMode::Default, 10)
            .unwrap();

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].kind, crate::models::EntryKind::File);
        assert!(matches!(
            entries[0].data,
            crate::models::EntryData::File { .. }
        ));
        assert_eq!(entries[0].text_content.as_deref(), Some(file_uri_list));
        assert_eq!(db.count_entries("", EntryFilter::Files).unwrap(), 1);
        assert!(db.has_recent_file_uri_list(file_uri_list, 5).unwrap());

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

        // Force a re-migrate so the data repair runs after the bad row was inserted.
        db.conn.pragma_update(None, "user_version", 0).unwrap();
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
