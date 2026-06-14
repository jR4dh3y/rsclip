use anyhow::Result;
use chrono::Utc;
use rusqlite::{OptionalExtension, params};

use crate::models::SecretEntry;
use crate::secrets::normalize_secret_alias;

use super::{Database, rows::secret_from_row};

impl Database {
    pub fn list_secrets(&self, query: &str, limit: usize) -> Result<Vec<SecretEntry>> {
        self.list_secrets_page(query, limit, 0)
    }

    pub fn list_secrets_page(
        &self,
        query: &str,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<SecretEntry>> {
        let has_query = !query.trim().is_empty();
        let mut sql = String::from(
            r#"
            SELECT
              id, source_entry_id, alias, value, created_at, updated_at,
              last_used_at, use_count
            FROM secrets
            WHERE deleted = 0
            "#,
        );
        if has_query {
            sql.push_str(" AND alias LIKE ?1");
        }
        if has_query {
            sql.push_str(" ORDER BY updated_at DESC LIMIT ?2 OFFSET ?3");
        } else {
            sql.push_str(" ORDER BY updated_at DESC LIMIT ?1 OFFSET ?2");
        }

        let mut stmt = self.conn.prepare(&sql)?;
        let rows = if has_query {
            let pattern = format!("%{}%", query.trim());
            stmt.query_map(
                params![pattern, limit as i64, offset as i64],
                secret_from_row,
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?
        } else {
            stmt.query_map(params![limit as i64, offset as i64], secret_from_row)?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };
        Ok(rows)
    }

    pub fn count_secrets(&self, query: &str) -> Result<usize> {
        let has_query = !query.trim().is_empty();
        let mut sql = String::from("SELECT COUNT(*) FROM secrets WHERE deleted = 0");
        if has_query {
            sql.push_str(" AND alias LIKE ?1");
        }

        let count = if has_query {
            let pattern = format!("%{}%", query.trim());
            self.conn
                .query_row(&sql, params![pattern], |row| row.get::<_, i64>(0))?
        } else {
            self.conn.query_row(&sql, [], |row| row.get::<_, i64>(0))?
        };
        Ok(count.max(0) as usize)
    }

    pub fn save_secret(
        &self,
        source_entry_id: Option<i64>,
        alias: &str,
        value: &str,
    ) -> Result<i64> {
        let now = Utc::now().timestamp();
        let alias = normalize_secret_alias(alias);

        if let Some(source_entry_id) = source_entry_id {
            self.conn.execute(
                r#"
                INSERT INTO secrets (
                  source_entry_id, alias, value, created_at, updated_at, deleted
                )
                VALUES (?1, ?2, ?3, ?4, ?5, 0)
                ON CONFLICT(source_entry_id) DO UPDATE SET
                  alias=excluded.alias,
                  value=excluded.value,
                  updated_at=excluded.updated_at,
                  deleted=0
                "#,
                params![source_entry_id, alias, value, now, now],
            )?;
            let id = self.conn.query_row(
                "SELECT id FROM secrets WHERE source_entry_id = ?1",
                params![source_entry_id],
                |row| row.get::<_, i64>("id"),
            )?;
            return Ok(id);
        }

        self.conn.execute(
            r#"
            INSERT INTO secrets (source_entry_id, alias, value, created_at, updated_at, deleted)
            VALUES (NULL, ?1, ?2, ?3, ?4, 0)
            "#,
            params![alias, value, now, now],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn rename_secret(&self, id: i64, alias: &str) -> Result<()> {
        let alias = normalize_secret_alias(alias);
        self.conn.execute(
            "UPDATE secrets SET alias = ?2, updated_at = ?3 WHERE id = ?1",
            params![id, alias, Utc::now().timestamp()],
        )?;
        Ok(())
    }

    pub fn delete_secret(&self, id: i64) -> Result<()> {
        let source_entry_id = self
            .conn
            .query_row(
                "SELECT source_entry_id FROM secrets WHERE id = ?1",
                params![id],
                |row| row.get::<_, Option<i64>>("source_entry_id"),
            )
            .optional()?
            .flatten();

        self.conn.execute(
            "UPDATE secrets SET deleted = 1, updated_at = ?2 WHERE id = ?1",
            params![id, Utc::now().timestamp()],
        )?;

        if let Some(source_entry_id) = source_entry_id {
            self.conn.execute(
                "UPDATE entries SET deleted = 0 WHERE id = ?1",
                params![source_entry_id],
            )?;
        }

        Ok(())
    }

    pub fn touch_secret_used(&self, id: i64) -> Result<()> {
        let now = Utc::now().timestamp();
        self.conn.execute(
            "UPDATE secrets SET last_used_at = ?2, use_count = use_count + 1 WHERE id = ?1",
            params![id, now],
        )?;
        Ok(())
    }
}
