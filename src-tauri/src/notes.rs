//! Notes libres posées dans l'espace de l'arbre des versions.

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

pub type Result<T> = rusqlite::Result<T>;

#[derive(Debug, Serialize)]
pub struct TreeNote {
    pub id: i64,
    /// Version à laquelle la note est accrochée : elle la suit dans l'arbre.
    pub version_id: Option<i64>,
    /// Décalage par rapport à la version, ou position si la note est libre.
    pub x: f64,
    pub y: f64,
    pub text: String,
    pub created_at: i64,
}

fn row(r: &rusqlite::Row) -> rusqlite::Result<TreeNote> {
    Ok(TreeNote {
        id: r.get(0)?,
        version_id: r.get(1)?,
        x: r.get(2)?,
        y: r.get(3)?,
        text: r.get(4)?,
        created_at: r.get(5)?,
    })
}

const COLUMNS: &str = "id, version_id, x, y, text, created_at";

pub fn list(conn: &Connection, project_id: i64) -> Result<Vec<TreeNote>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM tree_notes WHERE project_id = ?1 ORDER BY created_at, id"
    ))?;
    let rows = stmt.query_map([project_id], row)?;
    rows.collect()
}

pub fn add(
    conn: &Connection,
    project_id: i64,
    version_id: Option<i64>,
    x: f64,
    y: f64,
    text: &str,
    now: i64,
) -> Result<TreeNote> {
    conn.execute(
        "INSERT INTO tree_notes (project_id, version_id, x, y, text, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![project_id, version_id, x, y, text, now],
    )?;
    get(conn, conn.last_insert_rowid())?.ok_or(rusqlite::Error::QueryReturnedNoRows)
}

fn get(conn: &Connection, id: i64) -> Result<Option<TreeNote>> {
    conn.query_row(&format!("SELECT {COLUMNS} FROM tree_notes WHERE id = ?1"), [id], row)
        .optional()
}

/// Déplace, réaccroche ou réécrit une note.
pub fn update(conn: &Connection, id: i64, version_id: Option<i64>, x: f64, y: f64, text: &str) -> Result<()> {
    conn.execute(
        "UPDATE tree_notes SET version_id = ?2, x = ?3, y = ?4, text = ?5 WHERE id = ?1",
        params![id, version_id, x, y, text],
    )?;
    Ok(())
}

pub fn delete(conn: &Connection, id: i64) -> Result<()> {
    conn.execute("DELETE FROM tree_notes WHERE id = ?1", [id])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    #[test]
    fn add_move_attach_delete() {
        let conn = db::open_in_memory().unwrap();
        conn.execute(
            "INSERT INTO projects (folder, als_path, name, modified_at, scanned_at)
             VALUES ('F:/p', 'F:/p/a.als', 'p', 0, 0)",
            [],
        )
        .unwrap();
        let note = add(&conn, 1, None, 120.0, -40.0, "drop trop long ?", 5).unwrap();
        assert_eq!((note.x, note.y, note.version_id), (120.0, -40.0, None));

        update(&conn, note.id, None, 10.0, 20.0, "drop à raccourcir").unwrap();
        let notes = list(&conn, 1).unwrap();
        assert_eq!(notes.len(), 1);
        assert_eq!((notes[0].x, notes[0].text.as_str()), (10.0, "drop à raccourcir"));

        delete(&conn, note.id).unwrap();
        assert!(list(&conn, 1).unwrap().is_empty());
    }
}
