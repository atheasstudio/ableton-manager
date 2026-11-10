//! Écritures sur la bibliothèque d'Ableton Manager, partagées par l'appli
//! (`src-tauri`) et sa porte MCP (`crates/mcp`) : même code, même validation.
//!
//! Chaque écriture prend une `source` : `"app"` quand ethaniel agit dans
//! l'appli, `"ia"` quand la porte agit pour une IA. Toute écriture dont la
//! source n'est pas `"app"` est aussi notée dans `changements_externes`, ce
//! qui permet à l'appli de voir qu'on a touché à sa base.

use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OptionalExtension};

/// Statuts possibles d'un projet.
pub const STATUSES: &[&str] = &["idee", "en_cours", "fini", "abandonne"];

/// Source d'une écriture faite dans l'appli.
pub const SOURCE_APP: &str = "app";
/// Source d'une écriture faite par la porte MCP.
pub const SOURCE_IA: &str = "ia";

/// Message quand la base n'a pas encore les colonnes de la porte.
pub const BASE_PAS_A_JOUR: &str =
    "la base n'est pas à jour : ouvre Ableton Manager une fois pour mettre la base à jour";

#[derive(Debug)]
pub enum Erreur {
    Base(rusqlite::Error),
    Message(String),
}

impl fmt::Display for Erreur {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Erreur::Base(e) => write!(f, "base de données : {e}"),
            Erreur::Message(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for Erreur {}

impl From<rusqlite::Error> for Erreur {
    fn from(e: rusqlite::Error) -> Self {
        Erreur::Base(e)
    }
}

pub type Result<T> = std::result::Result<T, Erreur>;

fn maintenant() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

/// Vérifie que la base a la colonne `todos.source` et la table
/// `changements_externes` (ajoutées par une migration de l'appli).
pub fn verifier_schema(conn: &Connection) -> Result<()> {
    let table: Option<String> = conn
        .query_row(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'changements_externes'",
            [],
            |r| r.get(0),
        )
        .optional()?;
    let colonne: i64 = conn.query_row(
        "SELECT count(*) FROM pragma_table_info('todos') WHERE name = 'source'",
        [],
        |r| r.get(0),
    )?;
    if table.is_none() || colonne == 0 {
        return Err(Erreur::Message(BASE_PAS_A_JOUR.into()));
    }
    Ok(())
}

/// Reconstruit la ligne de l'index plein texte d'un projet.
pub fn refresh_search(conn: &Connection, id: i64) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM search WHERE rowid = ?1", [id])?;
    conn.execute(
        "INSERT INTO search (rowid, name, note, plugins, samples, tags)
         SELECT p.id, p.name, p.note,
             (SELECT group_concat(name, ' ') FROM project_items WHERE project_id = p.id AND kind = 'plugin'),
             (SELECT group_concat(name, ' ') FROM project_items WHERE project_id = p.id AND kind = 'sample'),
             (SELECT group_concat(t.name, ' ') FROM project_tags pt JOIN tags t ON t.id = pt.tag_id
                 WHERE pt.project_id = p.id)
         FROM projects p WHERE p.id = ?1",
        [id],
    )?;
    Ok(())
}

/// Exécute `f` dans un point de sauvegarde : tout ou rien, même si
/// l'appelant a déjà ouvert une transaction.
fn tout_ou_rien<T>(conn: &Connection, f: impl FnOnce() -> Result<T>) -> Result<T> {
    conn.execute_batch("SAVEPOINT biblio")?;
    match f() {
        Ok(v) => {
            conn.execute_batch("RELEASE biblio")?;
            Ok(v)
        }
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK TO biblio; RELEASE biblio");
            Err(e)
        }
    }
}

/// Note un changement venu d'ailleurs que l'appli.
fn noter(conn: &Connection, source: &str, objet: &str, objet_id: i64, action: &str, valeur: Option<&str>, le: i64) -> Result<()> {
    if source == SOURCE_APP {
        return Ok(());
    }
    conn.execute(
        "INSERT INTO changements_externes (objet, objet_id, action, valeur, source, le)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![objet, objet_id, action, valeur, source, le],
    )?;
    Ok(())
}

/// Change le statut d'un projet. Refuse un statut inconnu.
pub fn set_status(conn: &Connection, id: i64, status: &str, source: &str) -> Result<()> {
    if !STATUSES.contains(&status) {
        return Err(Erreur::Message(format!("statut inconnu : {status} ({})", STATUSES.join(", "))));
    }
    tout_ou_rien(conn, || {
        conn.execute("UPDATE projects SET status = ?2 WHERE id = ?1", params![id, status])?;
        noter(conn, source, "projet", id, "statut", Some(status), maintenant())
    })
}

/// Ajoute un tag à un projet (créé au besoin) et met l'index à jour.
pub fn add_tag(conn: &Connection, id: i64, tag: &str, source: &str) -> Result<()> {
    let tag = tag.trim();
    if tag.is_empty() {
        return Ok(());
    }
    tout_ou_rien(conn, || {
        conn.execute("INSERT OR IGNORE INTO tags (name) VALUES (?1)", [tag])?;
        conn.execute(
            "INSERT OR IGNORE INTO project_tags (project_id, tag_id)
             SELECT ?1, id FROM tags WHERE name = ?2",
            params![id, tag],
        )?;
        refresh_search(conn, id)?;
        noter(conn, source, "projet", id, "tag", Some(tag), maintenant())
    })
}

/// Ajoute une tâche à un projet ; renvoie son id.
pub fn add_todo(conn: &Connection, project_id: i64, text: &str, now: i64, source: &str) -> Result<i64> {
    let text = text.trim();
    if text.is_empty() {
        return Err(Erreur::Message("tâche vide".into()));
    }
    tout_ou_rien(conn, || {
        conn.execute(
            "INSERT INTO todos (project_id, text, done, created_at, source) VALUES (?1, ?2, 0, ?3, ?4)",
            params![project_id, text, now, source],
        )?;
        let id = conn.last_insert_rowid();
        noter(conn, source, "tache", id, "ajout", Some(text), now)?;
        Ok(id)
    })
}

/// Coche (ou décoche) une tâche.
pub fn set_todo_done(conn: &Connection, id: i64, done: bool, source: &str) -> Result<()> {
    tout_ou_rien(conn, || {
        conn.execute("UPDATE todos SET done = ?2 WHERE id = ?1", params![id, done as i64])?;
        let action = if done { "cocher" } else { "decocher" };
        noter(conn, source, "tache", id, action, None, maintenant())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Schéma réduit aux tables que touche ce crate.
    fn base() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch(
            "CREATE TABLE projects (id INTEGER PRIMARY KEY, name TEXT NOT NULL, note TEXT NOT NULL DEFAULT '',
                 status TEXT NOT NULL DEFAULT 'idee');
             CREATE TABLE project_items (project_id INTEGER, kind TEXT, name TEXT);
             CREATE TABLE tags (id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE COLLATE NOCASE);
             CREATE TABLE project_tags (project_id INTEGER, tag_id INTEGER, PRIMARY KEY (project_id, tag_id));
             CREATE VIRTUAL TABLE search USING fts5(name, note, plugins, samples, tags);
             CREATE TABLE todos (id INTEGER PRIMARY KEY, project_id INTEGER NOT NULL, text TEXT NOT NULL,
                 done INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL);
             INSERT INTO projects (id, name) VALUES (1, 'Morceau');",
        )
        .unwrap();
        c
    }

    fn a_jour(c: &Connection) {
        c.execute_batch(
            "ALTER TABLE todos ADD COLUMN source TEXT NOT NULL DEFAULT 'app';
             CREATE TABLE changements_externes (id INTEGER PRIMARY KEY, objet TEXT NOT NULL,
                 objet_id INTEGER NOT NULL, action TEXT NOT NULL, valeur TEXT, source TEXT NOT NULL, le INTEGER NOT NULL);",
        )
        .unwrap();
    }

    fn changements(c: &Connection) -> Vec<(String, i64, String, Option<String>, String)> {
        let mut st = c
            .prepare("SELECT objet, objet_id, action, valeur, source FROM changements_externes ORDER BY id")
            .unwrap();
        st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    }

    #[test]
    fn schema_ancien_refuse_avec_message_clair() {
        let c = base();
        let e = verifier_schema(&c).unwrap_err().to_string();
        assert!(e.contains("ouvre Ableton Manager une fois"), "{e}");
        a_jour(&c);
        verifier_schema(&c).unwrap();
    }

    #[test]
    fn ecritures_app_non_notees_ecritures_ia_notees() {
        let c = base();
        a_jour(&c);
        set_status(&c, 1, "en_cours", SOURCE_APP).unwrap();
        add_tag(&c, 1, " club ", SOURCE_APP).unwrap();
        let t = add_todo(&c, 1, " mix ", 5, SOURCE_APP).unwrap();
        set_todo_done(&c, t, true, SOURCE_APP).unwrap();
        assert!(changements(&c).is_empty());

        set_status(&c, 1, "fini", SOURCE_IA).unwrap();
        add_tag(&c, 1, "techno", SOURCE_IA).unwrap();
        let t2 = add_todo(&c, 1, "master", 6, SOURCE_IA).unwrap();
        set_todo_done(&c, t2, true, SOURCE_IA).unwrap();
        let ch = changements(&c);
        assert_eq!(ch.len(), 4);
        assert_eq!(ch[0], ("projet".into(), 1, "statut".into(), Some("fini".into()), "ia".into()));
        assert_eq!(ch[1].2, "tag");
        assert_eq!(ch[2], ("tache".into(), t2, "ajout".into(), Some("master".into()), "ia".into()));
        assert_eq!(ch[3].2, "cocher");

        let src: Vec<String> = c
            .prepare("SELECT source FROM todos ORDER BY id")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(src, ["app", "ia"]);
        let statut: String = c.query_row("SELECT status FROM projects WHERE id = 1", [], |r| r.get(0)).unwrap();
        assert_eq!(statut, "fini");
        let trouve: i64 = c
            .query_row("SELECT count(*) FROM search WHERE search MATCH 'techno'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(trouve, 1);
    }

    #[test]
    fn statut_inconnu_refuse_sans_rien_ecrire() {
        let c = base();
        a_jour(&c);
        let e = set_status(&c, 1, "brouillon", SOURCE_IA).unwrap_err().to_string();
        assert_eq!(e, "statut inconnu : brouillon (idee, en_cours, fini, abandonne)");
        assert!(changements(&c).is_empty());
        assert_eq!(add_todo(&c, 1, "  ", 0, SOURCE_IA).unwrap_err().to_string(), "tâche vide");
    }
}
