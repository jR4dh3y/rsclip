use anyhow::Result;
use rusqlite::params;

use super::Database;

impl Database {
    pub fn migrate(&self) -> Result<()> {
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
            "#,
        )?;
        self.ensure_column("entries", "file_path", "TEXT")?;
        self.ensure_column("entries", "thumb_path", "TEXT")?;
        self.ensure_column("entries", "source_app", "TEXT")?;
        self.ensure_column("entries", "link_url", "TEXT")?;
        self.ensure_column("entries", "link_domain", "TEXT")?;
        self.ensure_column("entries", "link_icon", "TEXT")?;
        self.ensure_column("entries", "color_value", "TEXT")?;
        self.ensure_column("entries", "color_format", "TEXT")?;
        self.ensure_column("entries", "pinned", "INTEGER NOT NULL DEFAULT 0")?;
        self.ensure_column("entries", "copied_at", "INTEGER NOT NULL DEFAULT 0")?;
        self.ensure_column("entries", "updated_at", "INTEGER NOT NULL DEFAULT 0")?;
        self.ensure_column("entries", "last_used_at", "INTEGER")?;
        self.ensure_column("entries", "use_count", "INTEGER NOT NULL DEFAULT 0")?;
        self.ensure_column("entries", "size_bytes", "INTEGER NOT NULL DEFAULT 0")?;
        self.ensure_column("entries", "deleted", "INTEGER NOT NULL DEFAULT 0")?;
        self.ensure_column("secrets", "last_used_at", "INTEGER")?;
        self.ensure_column("secrets", "use_count", "INTEGER NOT NULL DEFAULT 0")?;
        self.ensure_column("secrets", "deleted", "INTEGER NOT NULL DEFAULT 0")?;
        self.conn.execute_batch(
            r#"
            CREATE UNIQUE INDEX IF NOT EXISTS idx_entries_hash ON entries(content_hash);
            CREATE INDEX IF NOT EXISTS idx_entries_copied_at ON entries(copied_at DESC);
            CREATE INDEX IF NOT EXISTS idx_entries_pinned ON entries(pinned DESC, updated_at DESC);
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

    fn ensure_column(&self, table: &str, column: &str, definition: &str) -> Result<()> {
        let mut stmt = self.conn.prepare(&format!("PRAGMA table_info({table})"))?;
        let exists = stmt
            .query_map([], |row| row.get::<_, String>("name"))?
            .collect::<rusqlite::Result<Vec<_>>>()?
            .iter()
            .any(|name| name == column);
        if !exists {
            self.conn.execute(
                &format!("ALTER TABLE {table} ADD COLUMN {column} {definition}"),
                params![],
            )?;
        }
        Ok(())
    }
}
