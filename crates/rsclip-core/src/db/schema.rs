use anyhow::Result;
use rusqlite::{OptionalExtension, params};

use crate::syntax::detect_code_language;

use super::{Database, ENTRY_TEXT_PREFIX_CHARS};

/// Highest applied schema version. Add a new `if current < N` step when changing schema.
const SCHEMA_USER_VERSION: i32 = 6;

impl Database {
    pub fn migrate(&self) -> Result<()> {
        let current: i32 = self
            .conn
            .pragma_query_value(None, "user_version", |row| row.get(0))?;
        if current >= SCHEMA_USER_VERSION {
            return Ok(());
        }

        // Apply pending steps in order. New schema work becomes `if current < 2 { ... }`, etc.
        if current < 1 {
            self.migrate_v1()?;
        }
        if current < 2 {
            self.migrate_v2()?;
        }
        if current < 3 {
            self.migrate_v3()?;
        }
        if current < 4 {
            self.migrate_v4()?;
        }
        if current < 5 {
            self.migrate_v5()?;
        }
        if current < 6 {
            self.migrate_v6()?;
        }

        self.conn
            .pragma_update(None, "user_version", SCHEMA_USER_VERSION)?;
        Ok(())
    }

    /// Bounded side table for text entries: search, the Code/Text filters, and
    /// previews read this narrow row instead of `entries.text_content`.
    ///
    /// SQLite materializes a whole TEXT value before `substr`/`LIKE` run, so a
    /// single legacy 40 MB payload made every search scan and every preview of
    /// that row cost 10+ ms. Triggers keep the table in sync for every writer.
    fn migrate_v4(&self) -> Result<()> {
        self.conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS entry_text (
              entry_id INTEGER PRIMARY KEY,
              code_lang TEXT,
              body TEXT NOT NULL
            );
            "#,
        )?;
        self.create_entry_text_triggers()?;
        self.conn.execute(
            r#"
            INSERT OR IGNORE INTO entry_text (entry_id, body)
            SELECT id, substr(text_content, 1, ?1)
              FROM entries
             WHERE kind = 'text' AND text_content IS NOT NULL
            "#,
            params![ENTRY_TEXT_PREFIX_CHARS as i64],
        )?;
        self.backfill_code_languages()
    }

    fn create_entry_text_triggers(&self) -> Result<()> {
        let prefix = ENTRY_TEXT_PREFIX_CHARS;
        self.conn.execute_batch(&format!(
            r#"
            CREATE TRIGGER IF NOT EXISTS entry_text_after_insert
            AFTER INSERT ON entries
            WHEN NEW.kind = 'text' AND NEW.text_content IS NOT NULL
            BEGIN
              INSERT OR REPLACE INTO entry_text (entry_id, body)
              VALUES (NEW.id, substr(NEW.text_content, 1, {prefix}));
            END;

            CREATE TRIGGER IF NOT EXISTS entry_text_after_update
            AFTER UPDATE OF kind, text_content ON entries
            BEGIN
              DELETE FROM entry_text WHERE entry_id = OLD.id;
              INSERT INTO entry_text (entry_id, body)
              SELECT NEW.id, substr(NEW.text_content, 1, {prefix})
               WHERE NEW.kind = 'text' AND NEW.text_content IS NOT NULL;
            END;

            CREATE TRIGGER IF NOT EXISTS entry_text_after_delete
            AFTER DELETE ON entries
            BEGIN
              DELETE FROM entry_text WHERE entry_id = OLD.id;
            END;
            "#
        ))?;
        Ok(())
    }

    /// Rebuild `entries` with its payload column last.
    ///
    /// SQLite stores a row's columns in declaration order, and reading a
    /// column stored after a multi-megabyte `text_content` walks that value's
    /// whole overflow chain (~12 ms for a 40 MB row). The original layout put
    /// `pinned`, `updated_at`, `size_bytes`, etc. after the payload, so every
    /// list window containing a huge entry paid that cost. Small fixed-size
    /// columns now come first, then short strings, then the payload.
    fn migrate_v5(&self) -> Result<()> {
        if self.entries_payload_is_last()? {
            return Ok(());
        }
        // Dropping the old table must not cascade into ocr_results/secrets,
        // and foreign_keys cannot change inside a transaction.
        self.conn.pragma_update(None, "foreign_keys", false)?;
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let result = self.rebuild_entries_table();
        let finish = if result.is_ok() { "COMMIT" } else { "ROLLBACK" };
        let finished = self.conn.execute_batch(finish);
        self.conn.pragma_update(None, "foreign_keys", true)?;
        result?;
        finished?;
        // The old table's pages (the whole history) are now free; reclaim
        // them so the file does not double in size. Best effort: another
        // process holding the database makes VACUUM fail, and SQLite reuses
        // free pages for new entries anyway.
        let _ = self.conn.execute_batch("VACUUM");
        Ok(())
    }

    /// Add the exact-value index used to discard saved-secret clipboard echoes.
    fn migrate_v6(&self) -> Result<()> {
        self.conn
            .execute_batch("CREATE INDEX IF NOT EXISTS idx_secrets_value ON secrets(value);")?;
        Ok(())
    }

    fn entries_payload_is_last(&self) -> Result<bool> {
        let mut stmt = self
            .conn
            .prepare("SELECT name FROM pragma_table_info('entries')")?;
        let columns = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(columns.last().map(String::as_str) == Some("text_content"))
    }

    fn rebuild_entries_table(&self) -> Result<()> {
        // Another process may have rebuilt the table while we waited for the lock.
        if self.entries_payload_is_last()? {
            return Ok(());
        }
        let sequence: Option<i64> = self
            .conn
            .query_row(
                "SELECT seq FROM sqlite_sequence WHERE name = 'entries'",
                [],
                |row| row.get(0),
            )
            .optional()?;

        let columns = "id, content_hash, kind, mime_type, pinned, copied_at, updated_at, \
            last_used_at, use_count, size_bytes, deleted, title, link_domain, link_icon, \
            color_value, color_format, source_app, file_path, thumb_path, link_url, \
            preview_text, text_content";
        self.conn.execute_batch(&format!(
            r#"
            DROP TABLE IF EXISTS entries_rebuild;
            CREATE TABLE entries_rebuild (
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              content_hash TEXT NOT NULL,
              kind TEXT NOT NULL,
              mime_type TEXT NOT NULL,
              pinned INTEGER NOT NULL DEFAULT 0,
              copied_at INTEGER NOT NULL DEFAULT 0,
              updated_at INTEGER NOT NULL DEFAULT 0,
              last_used_at INTEGER,
              use_count INTEGER NOT NULL DEFAULT 0,
              size_bytes INTEGER NOT NULL DEFAULT 0,
              deleted INTEGER NOT NULL DEFAULT 0,
              title TEXT NOT NULL,
              link_domain TEXT,
              link_icon TEXT,
              color_value TEXT,
              color_format TEXT,
              source_app TEXT,
              file_path TEXT,
              thumb_path TEXT,
              link_url TEXT,
              preview_text TEXT,
              -- Keep last: see migrate_v5.
              text_content TEXT
            );
            INSERT INTO entries_rebuild ({columns}) SELECT {columns} FROM entries;
            DROP TABLE entries;
            ALTER TABLE entries_rebuild RENAME TO entries;
            "#
        ))?;
        if let Some(sequence) = sequence {
            self.conn.execute(
                "UPDATE sqlite_sequence SET seq = max(seq, ?1) WHERE name = 'entries'",
                params![sequence],
            )?;
        }
        self.create_entry_indexes()?;
        self.create_entry_text_triggers()
    }

    fn create_entry_indexes(&self) -> Result<()> {
        self.conn.execute_batch(
            r#"
            CREATE UNIQUE INDEX IF NOT EXISTS idx_entries_hash ON entries(content_hash);
            CREATE INDEX IF NOT EXISTS idx_entries_copied_at ON entries(copied_at DESC);
            CREATE INDEX IF NOT EXISTS idx_entries_pinned ON entries(pinned DESC, updated_at DESC);
            CREATE INDEX IF NOT EXISTS idx_entries_updated_at ON entries(deleted, updated_at DESC);
            CREATE INDEX IF NOT EXISTS idx_entries_pinned_updated ON entries(deleted, pinned DESC, updated_at DESC);
            CREATE INDEX IF NOT EXISTS idx_entries_kind_pinned_updated ON entries(deleted, kind, pinned DESC, updated_at DESC);
            CREATE INDEX IF NOT EXISTS idx_entries_type_sort ON entries(deleted, kind ASC, updated_at DESC);
            CREATE INDEX IF NOT EXISTS idx_entries_most_used ON entries(deleted, use_count DESC, updated_at DESC);
            CREATE INDEX IF NOT EXISTS idx_entries_kind_used ON entries(deleted, kind, use_count DESC, updated_at DESC);
            CREATE INDEX IF NOT EXISTS idx_entries_kind ON entries(kind);
            CREATE INDEX IF NOT EXISTS idx_entries_domain ON entries(link_domain);
            "#,
        )?;
        Ok(())
    }

    /// Classify existing text rows once so the Code/Text filters become a
    /// column check instead of running the language detector per row per query.
    fn backfill_code_languages(&self) -> Result<()> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT t.entry_id, COALESCE(e.preview_text, t.body, e.title)
              FROM entry_text t
              JOIN entries e ON e.id = t.entry_id
             WHERE t.code_lang IS NULL
            "#,
        )?;
        let detected = stmt
            .query_map([], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?))
            })?
            .filter_map(|row| match row {
                Ok((id, Some(sample))) => detect_code_language(&sample).map(|lang| Ok((id, lang))),
                Ok((_, None)) => None,
                Err(err) => Some(Err(err)),
            })
            .collect::<rusqlite::Result<Vec<_>>>()?;

        let mut update = self
            .conn
            .prepare("UPDATE entry_text SET code_lang = ?2 WHERE entry_id = ?1")?;
        for (id, lang) in detected {
            update.execute(params![id, lang.sourceview_id()])?;
        }
        Ok(())
    }

    /// Performance covering indexes for all SortMode variants (Type, MostUsed) and kind filtering.
    fn migrate_v3(&self) -> Result<()> {
        self.conn.execute_batch(
            r#"
            CREATE INDEX IF NOT EXISTS idx_entries_kind_pinned_updated ON entries(deleted, kind, pinned DESC, updated_at DESC);
            CREATE INDEX IF NOT EXISTS idx_entries_type_sort ON entries(deleted, kind ASC, updated_at DESC);
            CREATE INDEX IF NOT EXISTS idx_entries_most_used ON entries(deleted, use_count DESC, updated_at DESC);
            CREATE INDEX IF NOT EXISTS idx_entries_kind_used ON entries(deleted, kind, use_count DESC, updated_at DESC);
            "#,
        )?;
        Ok(())
    }

    /// Performance indexes for recent and pinned sorting without table scans.
    fn migrate_v2(&self) -> Result<()> {
        self.conn.execute_batch(
            r#"
            CREATE INDEX IF NOT EXISTS idx_entries_updated_at ON entries(deleted, updated_at DESC);
            CREATE INDEX IF NOT EXISTS idx_entries_pinned_updated ON entries(deleted, pinned DESC, updated_at DESC);
            CREATE INDEX IF NOT EXISTS idx_secrets_updated_at ON secrets(deleted, updated_at DESC);
            "#,
        )?;
        Ok(())
    }

    /// Baseline schema: tables, columns, indexes, and one-time link repair.
    fn migrate_v1(&self) -> Result<()> {
        self.conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS entries (
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              content_hash TEXT NOT NULL,
              kind TEXT NOT NULL,
              mime_type TEXT NOT NULL,
              title TEXT NOT NULL,
              preview_text TEXT,
              text_content TEXT,
              file_path TEXT,
              thumb_path TEXT,
              source_app TEXT,
              link_url TEXT,
              link_domain TEXT,
              link_icon TEXT,
              color_value TEXT,
              color_format TEXT,
              pinned INTEGER NOT NULL DEFAULT 0,
              copied_at INTEGER NOT NULL,
              updated_at INTEGER NOT NULL,
              last_used_at INTEGER,
              use_count INTEGER NOT NULL DEFAULT 0,
              size_bytes INTEGER NOT NULL DEFAULT 0,
              deleted INTEGER NOT NULL DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS ocr_results (
              entry_id INTEGER PRIMARY KEY,
              status TEXT NOT NULL,
              text TEXT,
              language TEXT,
              created_at INTEGER NOT NULL,
              updated_at INTEGER NOT NULL,
              FOREIGN KEY(entry_id) REFERENCES entries(id)
            );

            CREATE TABLE IF NOT EXISTS secrets (
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              source_entry_id INTEGER UNIQUE,
              alias TEXT NOT NULL,
              value TEXT NOT NULL,
              created_at INTEGER NOT NULL,
              updated_at INTEGER NOT NULL,
              last_used_at INTEGER,
              use_count INTEGER NOT NULL DEFAULT 0,
              deleted INTEGER NOT NULL DEFAULT 0,
              FOREIGN KEY(source_entry_id) REFERENCES entries(id)
            );

            CREATE INDEX IF NOT EXISTS idx_secrets_updated_at ON secrets(updated_at DESC);
            CREATE INDEX IF NOT EXISTS idx_secrets_alias ON secrets(alias);
            CREATE INDEX IF NOT EXISTS idx_secrets_value ON secrets(value);
            "#,
        )?;
        self.ensure_columns(
            "entries",
            &[
                ("file_path", "TEXT"),
                ("thumb_path", "TEXT"),
                ("source_app", "TEXT"),
                ("link_url", "TEXT"),
                ("link_domain", "TEXT"),
                ("link_icon", "TEXT"),
                ("color_value", "TEXT"),
                ("color_format", "TEXT"),
                ("pinned", "INTEGER NOT NULL DEFAULT 0"),
                ("copied_at", "INTEGER NOT NULL DEFAULT 0"),
                ("updated_at", "INTEGER NOT NULL DEFAULT 0"),
                ("last_used_at", "INTEGER"),
                ("use_count", "INTEGER NOT NULL DEFAULT 0"),
                ("size_bytes", "INTEGER NOT NULL DEFAULT 0"),
                ("deleted", "INTEGER NOT NULL DEFAULT 0"),
            ],
        )?;
        self.ensure_columns(
            "secrets",
            &[
                ("last_used_at", "INTEGER"),
                ("use_count", "INTEGER NOT NULL DEFAULT 0"),
                ("deleted", "INTEGER NOT NULL DEFAULT 0"),
            ],
        )?;
        self.conn.execute_batch(
            r#"
            CREATE UNIQUE INDEX IF NOT EXISTS idx_entries_hash ON entries(content_hash);
            CREATE INDEX IF NOT EXISTS idx_entries_copied_at ON entries(copied_at DESC);
            CREATE INDEX IF NOT EXISTS idx_entries_pinned ON entries(pinned DESC, updated_at DESC);
            CREATE INDEX IF NOT EXISTS idx_entries_updated_at ON entries(deleted, updated_at DESC);
            CREATE INDEX IF NOT EXISTS idx_entries_pinned_updated ON entries(deleted, pinned DESC, updated_at DESC);
            CREATE INDEX IF NOT EXISTS idx_entries_kind_pinned_updated ON entries(deleted, kind, pinned DESC, updated_at DESC);
            CREATE INDEX IF NOT EXISTS idx_entries_type_sort ON entries(deleted, kind ASC, updated_at DESC);
            CREATE INDEX IF NOT EXISTS idx_entries_most_used ON entries(deleted, use_count DESC, updated_at DESC);
            CREATE INDEX IF NOT EXISTS idx_entries_kind_used ON entries(deleted, kind, use_count DESC, updated_at DESC);
            CREATE INDEX IF NOT EXISTS idx_secrets_updated_at ON secrets(deleted, updated_at DESC);
            CREATE INDEX IF NOT EXISTS idx_entries_kind ON entries(kind);
            CREATE INDEX IF NOT EXISTS idx_entries_domain ON entries(link_domain);

            UPDATE entries
               SET kind = 'text',
                   title = substr(text_content, 1, 96),
                   preview_text = text_content,
                   link_url = NULL,
                   link_domain = NULL,
                   link_icon = NULL
             WHERE kind = 'link'
               AND text_content IS NOT NULL
               AND trim(text_content) != ''
               AND lower(trim(text_content)) NOT LIKE 'http://%'
               AND lower(trim(text_content)) NOT LIKE 'https://%';
            "#,
        )?;
        Ok(())
    }

    fn ensure_columns(&self, table: &str, columns: &[(&str, &str)]) -> Result<()> {
        let mut stmt = self.conn.prepare(&format!("PRAGMA table_info({table})"))?;
        let existing: std::collections::HashSet<String> = stmt
            .query_map([], |row| row.get::<_, String>("name"))?
            .collect::<rusqlite::Result<_>>()?;
        for (column, definition) in columns {
            if !existing.contains(*column) {
                self.conn.execute(
                    &format!("ALTER TABLE {table} ADD COLUMN {column} {definition}"),
                    params![],
                )?;
            }
        }
        Ok(())
    }
}
