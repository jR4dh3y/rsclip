use anyhow::Result;
use chrono::Utc;
use rusqlite::{OptionalExtension, params};

use crate::models::{ClipboardEntry, EntryFilter, NewEntry, NewEntryData, SortMode};

use super::{Database, rows::entry_from_row};

impl Database {
    pub fn upsert_entry(&self, entry: &NewEntry) -> Result<i64> {
        let now = Utc::now().timestamp();
        let kind = entry.data.kind();

        let (
            file_path,
            thumb_path,
            source_app,
            link_url,
            link_domain,
            link_icon,
            color_value,
            color_format,
        ) = match &entry.data {
            NewEntryData::Text => (None, None, None, None, None, None, None, None),
            NewEntryData::Image {
                file_path,
                thumb_path,
                ocr_text: _,
            } => (
                file_path.clone(),
                thumb_path.clone(),
                None,
                None,
                None,
                None,
                None,
                None,
            ),
            NewEntryData::Link { url, domain, icon } => (
                None,
                None,
                None,
                Some(url.clone()),
                Some(domain.clone()),
                Some(icon.clone()),
                None,
                None,
            ),
            NewEntryData::Color { value, format } => (
                None,
                None,
                None,
                None,
                None,
                None,
                Some(value.clone()),
                Some(format.clone()),
            ),
            NewEntryData::File { source_app } => {
                (None, None, source_app.clone(), None, None, None, None, None)
            }
            NewEntryData::Unknown => (None, None, None, None, None, None, None, None),
        };

        self.conn.execute(
            r#"
            INSERT INTO entries (
              content_hash, kind, mime_type, title, preview_text, text_content,
              file_path, thumb_path, source_app, link_url, link_domain, link_icon,
              color_value, color_format, copied_at, updated_at, size_bytes, deleted
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, 0)
            ON CONFLICT(content_hash) DO UPDATE SET
              kind=excluded.kind,
              mime_type=excluded.mime_type,
              title=excluded.title,
              preview_text=excluded.preview_text,
              text_content=excluded.text_content,
              file_path=COALESCE(excluded.file_path, entries.file_path),
              thumb_path=COALESCE(excluded.thumb_path, entries.thumb_path),
              source_app=excluded.source_app,
              link_url=excluded.link_url,
              link_domain=excluded.link_domain,
              link_icon=excluded.link_icon,
              color_value=excluded.color_value,
              color_format=excluded.color_format,
              updated_at=excluded.updated_at,
              size_bytes=excluded.size_bytes,
              deleted=0
            "#,
            params![
                entry.content_hash,
                kind.as_str(),
                entry.mime_type,
                entry.title,
                entry.preview_text,
                entry.text_content,
                file_path,
                thumb_path,
                source_app,
                link_url,
                link_domain,
                link_icon,
                color_value,
                color_format,
                now,
                now,
                entry.size_bytes,
            ],
        )?;
        let id = self.conn.query_row(
            "SELECT id FROM entries WHERE content_hash = ?1",
            params![entry.content_hash],
            |row| row.get::<_, i64>("id"),
        )?;
        Ok(id)
    }

    pub fn list_entries(
        &self,
        query: &str,
        filter: EntryFilter,
        sort: SortMode,
        limit: usize,
    ) -> Result<Vec<ClipboardEntry>> {
        let mut sql = String::from(
            r#"
            SELECT
              e.id, e.content_hash, e.kind, e.mime_type, e.title, e.preview_text,
              e.text_content, e.file_path, e.thumb_path, e.source_app, e.link_url,
              e.link_domain, e.link_icon, e.color_value, e.color_format, e.pinned,
              e.favorite, e.copied_at, e.updated_at, e.last_used_at, e.use_count,
              e.size_bytes, o.text AS ocr_text
            FROM entries e
            LEFT JOIN ocr_results o ON o.entry_id = e.id
            WHERE e.deleted = 0
            "#,
        );

        sql.push_str(match filter {
            EntryFilter::All => "",
            EntryFilter::Text => " AND e.kind = 'text'",
            EntryFilter::Images => " AND e.kind = 'image'",
            EntryFilter::Links => " AND e.kind = 'link'",
            EntryFilter::Colors => " AND e.kind = 'color'",
            EntryFilter::Pinned => " AND e.pinned = 1",
        });

        let has_query = !query.trim().is_empty();
        if has_query {
            sql.push_str(
                r#"
                AND (
                  e.title LIKE ?1 OR e.preview_text LIKE ?1 OR e.text_content LIKE ?1
                  OR e.link_url LIKE ?1 OR e.link_domain LIKE ?1 OR e.color_value LIKE ?1
                  OR o.text LIKE ?1
                )
                "#,
            );
        }

        sql.push_str(match sort {
            SortMode::Default => " ORDER BY e.pinned DESC, e.updated_at DESC",
            SortMode::Recent => " ORDER BY e.updated_at DESC",
            SortMode::Oldest => " ORDER BY e.updated_at ASC",
            SortMode::Type => " ORDER BY e.kind ASC, e.updated_at DESC",
            SortMode::MostUsed => " ORDER BY e.use_count DESC, e.updated_at DESC",
        });
        sql.push_str(" LIMIT ");
        sql.push_str(&limit.to_string());

        let mut stmt = self.conn.prepare(&sql)?;
        let rows = if has_query {
            let pattern = format!("%{}%", query.trim());
            stmt.query_map(params![pattern], entry_from_row)?
                .collect::<rusqlite::Result<Vec<_>>>()?
        } else {
            stmt.query_map([], entry_from_row)?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };
        Ok(rows)
    }

    pub fn list_link_domains(&self) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT DISTINCT link_domain
            FROM entries
            WHERE deleted = 0
              AND kind = 'link'
              AND link_domain IS NOT NULL
              AND link_domain != ''
            ORDER BY link_domain ASC
            "#,
        )?;
        let domains = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(domains)
    }

    pub fn get_entry(&self, id: i64) -> Result<Option<ClipboardEntry>> {
        self.conn
            .query_row(
                r#"
                SELECT
                  e.id, e.content_hash, e.kind, e.mime_type, e.title, e.preview_text,
                  e.text_content, e.file_path, e.thumb_path, e.source_app, e.link_url,
                  e.link_domain, e.link_icon, e.color_value, e.color_format, e.pinned,
                  e.favorite, e.copied_at, e.updated_at, e.last_used_at, e.use_count,
                  e.size_bytes, o.text AS ocr_text
                FROM entries e
                LEFT JOIN ocr_results o ON o.entry_id = e.id
                WHERE e.id = ?1 AND e.deleted = 0
                "#,
                params![id],
                entry_from_row,
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn set_pinned(&self, id: i64, pinned: bool) -> Result<()> {
        self.conn.execute(
            "UPDATE entries SET pinned = ?2, updated_at = ?3 WHERE id = ?1",
            params![id, pinned, Utc::now().timestamp()],
        )?;
        Ok(())
    }

    pub fn delete_entry(&self, id: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE entries SET deleted = 1, updated_at = ?2 WHERE id = ?1",
            params![id, Utc::now().timestamp()],
        )?;
        Ok(())
    }

    pub fn touch_used(&self, id: i64) -> Result<()> {
        let now = Utc::now().timestamp();
        self.conn.execute(
            "UPDATE entries SET last_used_at = ?2, use_count = use_count + 1 WHERE id = ?1",
            params![id, now],
        )?;
        Ok(())
    }

    pub fn delete_unpinned_older_than_days(&self, days: u32) -> Result<usize> {
        if days == 0 {
            return Ok(0);
        }

        let cutoff = Utc::now().timestamp() - i64::from(days) * 86_400;
        let deleted = self.conn.execute(
            r#"
            UPDATE entries
               SET deleted = 1, updated_at = ?2
             WHERE deleted = 0
               AND pinned = 0
               AND updated_at < ?1
            "#,
            params![cutoff, Utc::now().timestamp()],
        )?;
        Ok(deleted)
    }
}
