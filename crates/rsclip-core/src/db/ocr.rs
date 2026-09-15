use anyhow::Result;
use chrono::Utc;
use rusqlite::params;

use super::Database;

impl Database {
    /// Save or update an OCR recognition result for an entry in `ocr_results`.
    pub fn save_ocr_result(&self, entry_id: i64, language: &str, text: &str) -> Result<()> {
        let now = Utc::now().timestamp();
        self.conn.execute(
            r#"
            INSERT INTO ocr_results (entry_id, status, text, language, created_at, updated_at)
            VALUES (?1, 'done', ?2, ?3, ?4, ?5)
            ON CONFLICT(entry_id) DO UPDATE SET
              status='done',
              text=excluded.text,
              language=excluded.language,
              updated_at=excluded.updated_at
            "#,
            params![entry_id, text, language, now, now],
        )?;
        Ok(())
    }
}
