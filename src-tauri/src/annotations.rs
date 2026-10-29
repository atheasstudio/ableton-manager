//! Annotations posées sur la timeline d'un projet. Les positions sont en
//! beats (noires) depuis la mesure 1 : elles restent justes si le tempo change.

use rusqlite::{params, Connection};
use serde::Serialize;

use crate::db::Result;

#[derive(Debug, Clone, Serialize)]
pub struct Annotation {
    pub id: i64,
    pub beat: f64,
    pub label: String,
    pub created_at: i64,
}

pub fn list(conn: &Connection, project_id: i64) -> Result<Vec<Annotation>> {
    let mut stmt = conn.prepare(
        "SELECT id, beat, label, created_at FROM annotations
         WHERE project_id = ?1 ORDER BY beat, id",
    )?;
    let rows = stmt.query_map([project_id], |r| {
        Ok(Annotation { id: r.get(0)?, beat: r.get(1)?, label: r.get(2)?, created_at: r.get(3)? })
    })?;
    rows.collect()
}

pub fn add(
    conn: &Connection,
    project_id: i64,
    beat: f64,
    label: &str,
    now: i64,
) -> Result<Annotation> {
    let beat = beat.max(0.0);
    let label = label.trim();
    conn.execute(
        "INSERT INTO annotations (project_id, beat, label, created_at) VALUES (?1, ?2, ?3, ?4)",
        params![project_id, beat, label, now],
    )?;
    Ok(Annotation { id: conn.last_insert_rowid(), beat, label: label.to_owned(), created_at: now })
}

pub fn update(conn: &Connection, id: i64, beat: f64, label: &str) -> Result<()> {
    conn.execute(
        "UPDATE annotations SET beat = ?2, label = ?3 WHERE id = ?1",
        params![id, beat.max(0.0), label.trim()],
    )?;
    Ok(())
}

pub fn delete(conn: &Connection, id: i64) -> Result<()> {
    conn.execute("DELETE FROM annotations WHERE id = ?1", [id])?;
    Ok(())
}

/// Crée une annotation par locator du set, sauf s'il en existe déjà une au
/// même endroit avec le même nom. Renvoie le nombre d'annotations créées.
pub fn import_locators(
    conn: &mut Connection,
    project_id: i64,
    locators: &[als_parser::Locator],
    now: i64,
) -> Result<usize> {
    let tx = conn.transaction()?;
    let mut created = 0;
    for loc in locators {
        let exists: bool = tx.query_row(
            "SELECT EXISTS (SELECT 1 FROM annotations
                 WHERE project_id = ?1 AND abs(beat - ?2) < 1e-6 AND label = ?3)",
            params![project_id, loc.time, loc.name.trim()],
            |r| r.get(0),
        )?;
        if !exists {
            add(&tx, project_id, loc.time, &loc.name, now)?;
            created += 1;
        }
    }
    tx.commit()?;
    Ok(created)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    fn project(conn: &Connection) -> i64 {
        conn.execute(
            "INSERT INTO projects (folder, als_path, name, modified_at, scanned_at)
             VALUES ('F:/p', 'F:/p/a.als', 'a', 0, 0)",
            [],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    #[test]
    fn crud_sorted_by_position() {
        let conn = db::open_in_memory().unwrap();
        let p = project(&conn);
        let drop = add(&conn, p, 160.0, " Drop ", 1).unwrap();
        add(&conn, p, 32.0, "Couplet", 2).unwrap();
        assert_eq!(drop.label, "Drop");
        let labels =
            |c: &Connection| list(c, p).unwrap().into_iter().map(|a| a.label).collect::<Vec<_>>();
        assert_eq!(labels(&conn), ["Couplet", "Drop"]);

        update(&conn, drop.id, 16.0, "Intro").unwrap();
        assert_eq!(labels(&conn), ["Intro", "Couplet"]);
        delete(&conn, drop.id).unwrap();
        assert_eq!(labels(&conn), ["Couplet"]);

        conn.execute("DELETE FROM projects WHERE id = ?1", [p]).unwrap();
        assert!(list(&conn, p).unwrap().is_empty(), "supprimées avec le projet");
    }

    #[test]
    fn import_locators_once() {
        let mut conn = db::open_in_memory().unwrap();
        let p = project(&conn);
        let locs = vec![
            als_parser::Locator { time: 64.0, name: "Drop".into() },
            als_parser::Locator { time: 128.0, name: "Pont".into() },
        ];
        assert_eq!(import_locators(&mut conn, p, &locs, 1).unwrap(), 2);
        assert_eq!(import_locators(&mut conn, p, &locs, 2).unwrap(), 0, "pas de doublons");
        assert_eq!(list(&conn, p).unwrap().len(), 2);
    }
}
