//! `.le3` file persistence (SPEC §7).
//!
//! A `.le3` is a SQLite database. This early version implements only a small
//! part of the schema from the spec — enough to round-trip the rectangle list —
//! but keeps the pieces that matter for forward compatibility: a `meta` table
//! and a `schema_migrations` table, and rectangles stored as DAG nodes with a
//! JSON payload (SPEC §7.3). When real features arrive they slot into the same
//! `dag_nodes` shape.

use crate::model::{Document, Rect};
use rusqlite::Connection;

/// Bump when the on-disk schema changes; pair with a migration below.
const SCHEMA_VERSION: i64 = 1;
const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

pub type Result<T> = std::result::Result<T, String>;

/// Create the tables for a fresh database (idempotent).
fn init_schema(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS schema_migrations (
            id         INTEGER PRIMARY KEY,
            name       TEXT NOT NULL,
            applied_at TEXT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS meta (
            key   TEXT PRIMARY KEY,
            value TEXT
        );
        CREATE TABLE IF NOT EXISTS dag_nodes (
            id           INTEGER PRIMARY KEY,
            component_id INTEGER,
            kind         TEXT NOT NULL,
            payload      TEXT NOT NULL
        );
        ",
    )?;
    Ok(())
}

/// Save `doc` to `path`, overwriting any existing document content.
///
/// We rewrite the node table wholesale: at this scale it's simplest and keeps
/// the file an exact reflection of the in-memory document. The action DAG
/// (SPEC §4) will replace this with incremental, append-only history later.
pub fn save(path: &str, doc: &Document) -> Result<()> {
    let mut conn = Connection::open(path).map_err(|e| e.to_string())?;
    init_schema(&conn).map_err(|e| e.to_string())?;

    let tx = conn.transaction().map_err(|e| e.to_string())?;

    tx.execute(
        "INSERT OR REPLACE INTO schema_migrations (id, name, applied_at)
         VALUES (?1, 'initial', datetime('now'))",
        rusqlite::params![SCHEMA_VERSION],
    )
    .map_err(|e| e.to_string())?;

    tx.execute(
        "INSERT OR REPLACE INTO meta (key, value) VALUES ('app_version', ?1)",
        rusqlite::params![APP_VERSION],
    )
    .map_err(|e| e.to_string())?;

    tx.execute("DELETE FROM dag_nodes WHERE kind = 'rectangle'", [])
        .map_err(|e| e.to_string())?;

    for (i, rect) in doc.rects.iter().enumerate() {
        let payload = serde_json::to_string(rect).map_err(|e| e.to_string())?;
        tx.execute(
            "INSERT INTO dag_nodes (id, component_id, kind, payload)
             VALUES (?1, 0, 'rectangle', ?2)",
            rusqlite::params![i as i64, payload],
        )
        .map_err(|e| e.to_string())?;
    }

    tx.commit().map_err(|e| e.to_string())?;
    Ok(())
}

/// Open the document stored at `path`.
pub fn open(path: &str) -> Result<Document> {
    let conn = Connection::open(path).map_err(|e| e.to_string())?;

    let mut stmt = conn
        .prepare("SELECT payload FROM dag_nodes WHERE kind = 'rectangle' ORDER BY id")
        .map_err(|e| e.to_string())?;

    let rows = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| e.to_string())?;

    let mut rects = Vec::new();
    for row in rows {
        let payload = row.map_err(|e| e.to_string())?;
        let rect: Rect = serde_json::from_str(&payload).map_err(|e| e.to_string())?;
        rects.push(rect);
    }

    Ok(Document { rects })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_rectangles() {
        let dir = std::env::temp_dir();
        let path = dir.join("le3_roundtrip_test.le3");
        let path = path.to_string_lossy().to_string();
        let _ = std::fs::remove_file(&path);

        let doc = Document {
            rects: vec![
                Rect { x: 1.0, y: 2.0, w: 3.0, h: 4.0 },
                Rect { x: 10.0, y: 20.0, w: 30.0, h: 40.0 },
            ],
        };

        save(&path, &doc).unwrap();
        let loaded = open(&path).unwrap();

        assert_eq!(loaded.rects, doc.rects);

        // Saving again must not duplicate rows.
        save(&path, &doc).unwrap();
        let reloaded = open(&path).unwrap();
        assert_eq!(reloaded.rects.len(), 2);

        std::fs::remove_file(&path).unwrap();
    }
}
