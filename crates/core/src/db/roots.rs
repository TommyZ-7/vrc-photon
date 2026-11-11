use super::*;

impl Db {
    // ---------- library roots ----------

    pub fn add_root(&self, path: &str, kind: &str) -> Result<i64> {
        let id = self.conn.query_row(
            "INSERT INTO library_roots(path, kind, created_at) VALUES (?1, ?2, ?3) \
             ON CONFLICT(path) DO UPDATE SET kind = \
                CASE WHEN library_roots.kind = 'managed' THEN 'managed' ELSE excluded.kind END \
             RETURNING id",
            params![path, kind, now_string()],
            |row| row.get(0),
        )?;
        Ok(id)
    }

    pub fn roots(&self) -> Result<Vec<LibraryRoot>> {
        let mut stmt = self
            .conn
            .prepare_cached("SELECT id, path, kind FROM library_roots ORDER BY id")?;
        let rows = stmt.query_map([], |row| {
            Ok(LibraryRoot {
                id: row.get(0)?,
                path: row.get(1)?,
                kind: row.get(2)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
}
