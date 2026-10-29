//! Base SQLite locale : projets, pistes, plugins, samples, tags.

use std::path::Path;

use als_parser::LiveSet;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

pub type Result<T> = rusqlite::Result<T>;

/// Migrations appliquées dans l'ordre ; l'index courant est stocké dans
/// `PRAGMA user_version`. Ne jamais modifier une migration déjà publiée.
const MIGRATIONS: &[&str] = &[r#"
CREATE TABLE roots (
    id   INTEGER PRIMARY KEY,
    path TEXT NOT NULL UNIQUE
);

CREATE TABLE projects (
    id           INTEGER PRIMARY KEY,
    root_id      INTEGER REFERENCES roots(id) ON DELETE SET NULL,
    folder       TEXT NOT NULL,
    als_path     TEXT NOT NULL UNIQUE,
    name         TEXT NOT NULL,
    tempo        REAL,
    time_sig     TEXT,
    live_version TEXT,
    modified_at  INTEGER NOT NULL,
    scanned_at   INTEGER NOT NULL,
    status       TEXT NOT NULL DEFAULT 'idee',
    favorite     INTEGER NOT NULL DEFAULT 0,
    note         TEXT NOT NULL DEFAULT ''
);

CREATE TABLE tracks (
    project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    position   INTEGER NOT NULL,
    name       TEXT NOT NULL,
    kind       TEXT NOT NULL
);

-- kind : 'plugin', 'sample' ou 'set' (autre .als du même dossier)
CREATE TABLE project_items (
    project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    kind       TEXT NOT NULL,
    name       TEXT NOT NULL,
    path       TEXT,
    -- samples : chemin relatif au dossier du projet, si Live l'a noté
    rel_path   TEXT
);

CREATE TABLE tags (
    id   INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE COLLATE NOCASE
);

CREATE TABLE project_tags (
    project_id INTEGER NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    tag_id     INTEGER NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
    PRIMARY KEY (project_id, tag_id)
);

CREATE INDEX tracks_project ON tracks(project_id);
CREATE INDEX items_project ON project_items(project_id);

-- Index plein texte, reconstruit par refresh_search() à chaque changement.
CREATE VIRTUAL TABLE search USING fts5(
    name, note, plugins, samples, tags,
    tokenize = 'unicode61 remove_diacritics 2'
);
"#];

pub fn open(path: &Path) -> Result<Connection> {
    let conn = Connection::open(path)?;
    init(&conn)?;
    Ok(conn)
}

#[cfg(test)]
pub fn open_in_memory() -> Result<Connection> {
    let conn = Connection::open_in_memory()?;
    init(&conn)?;
    Ok(conn)
}

fn init(conn: &Connection) -> Result<()> {
    conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL;")?;
    let version: usize = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(version) {
        conn.execute_batch(sql)?;
        conn.pragma_update(None, "user_version", i + 1)?;
    }
    Ok(())
}

// ---------- Dossiers racine ----------

#[derive(Debug, Serialize)]
pub struct Root {
    pub id: i64,
    pub path: String,
}

pub fn add_root(conn: &Connection, path: &str) -> Result<i64> {
    conn.execute("INSERT OR IGNORE INTO roots (path) VALUES (?1)", [path])?;
    conn.query_row("SELECT id FROM roots WHERE path = ?1", [path], |r| r.get(0))
}

pub fn remove_root(conn: &Connection, id: i64) -> Result<()> {
    conn.execute("DELETE FROM roots WHERE id = ?1", [id])?;
    Ok(())
}

pub fn list_roots(conn: &Connection) -> Result<Vec<Root>> {
    let mut stmt = conn.prepare("SELECT id, path FROM roots ORDER BY path")?;
    let rows = stmt.query_map([], |r| Ok(Root { id: r.get(0)?, path: r.get(1)? }))?;
    rows.collect()
}

// ---------- Indexation ----------

/// Ce qu'on sait d'un projet trouvé sur le disque.
pub struct ScannedProject<'a> {
    pub root_id: Option<i64>,
    pub folder: &'a str,
    pub als_path: &'a str,
    pub name: &'a str,
    pub modified_at: i64,
    pub set: &'a LiveSet,
    /// Autres .als du même dossier.
    pub other_sets: &'a [String],
}

/// Date de modification déjà indexée pour ce .als, pour sauter les fichiers inchangés.
pub fn indexed_mtime(conn: &Connection, als_path: &str) -> Result<Option<i64>> {
    conn.query_row(
        "SELECT modified_at FROM projects WHERE als_path = ?1",
        [als_path],
        |r| r.get(0),
    )
    .optional()
}

/// Insère ou met à jour un projet ; tags, statut, favori et note sont conservés.
pub fn upsert_project(conn: &mut Connection, p: &ScannedProject, now: i64) -> Result<i64> {
    let tx = conn.transaction()?;
    let time_sig = p.set.time_signature.map(|(n, d)| format!("{n}/{d}"));
    let id: i64 = tx.query_row(
        "INSERT INTO projects
             (root_id, folder, als_path, name, tempo, time_sig, live_version, modified_at, scanned_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT(als_path) DO UPDATE SET
             root_id = excluded.root_id, folder = excluded.folder, name = excluded.name,
             tempo = excluded.tempo, time_sig = excluded.time_sig,
             live_version = excluded.live_version, modified_at = excluded.modified_at,
             scanned_at = excluded.scanned_at
         RETURNING id",
        params![
            p.root_id,
            p.folder,
            p.als_path,
            p.name,
            p.set.tempo,
            time_sig,
            p.set.live_version(),
            p.modified_at,
            now
        ],
        |r| r.get(0),
    )?;

    tx.execute("DELETE FROM tracks WHERE project_id = ?1", [id])?;
    tx.execute("DELETE FROM project_items WHERE project_id = ?1", [id])?;
    {
        let mut track = tx.prepare(
            "INSERT INTO tracks (project_id, position, name, kind) VALUES (?1, ?2, ?3, ?4)",
        )?;
        for (i, t) in p.set.tracks.iter().enumerate() {
            track.execute(params![id, i as i64, t.name, t.kind.as_str()])?;
        }
        let mut item = tx.prepare(
            "INSERT INTO project_items (project_id, kind, name, path, rel_path)
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )?;
        let none = Option::<String>::None;
        for plugin in &p.set.plugins {
            item.execute(params![id, "plugin", plugin.name, none, none])?;
        }
        for sample in &p.set.samples {
            let name = sample.file_name().unwrap_or("?");
            item.execute(params![id, "sample", name, sample.path, sample.relative_path])?;
        }
        for other in p.other_sets {
            let name = other.rsplit(['/', '\\']).next().unwrap_or(other);
            item.execute(params![id, "set", name, other, none])?;
        }
    }
    refresh_search(&tx, id)?;
    tx.commit()?;
    Ok(id)
}

/// Retire les projets d'une racine dont le .als n'a pas été revu au dernier scan.
pub fn remove_missing(conn: &Connection, root_id: i64, scanned_before: i64) -> Result<usize> {
    conn.execute(
        "DELETE FROM search WHERE rowid IN
             (SELECT id FROM projects WHERE root_id = ?1 AND scanned_at < ?2)",
        params![root_id, scanned_before],
    )?;
    conn.execute(
        "DELETE FROM projects WHERE root_id = ?1 AND scanned_at < ?2",
        params![root_id, scanned_before],
    )
}

fn refresh_search(conn: &Connection, id: i64) -> Result<()> {
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

// ---------- Lecture ----------

#[derive(Debug, Serialize)]
pub struct ProjectSummary {
    pub id: i64,
    pub name: String,
    pub folder: String,
    pub tempo: Option<f64>,
    pub live_version: Option<String>,
    pub modified_at: i64,
    pub status: String,
    pub favorite: bool,
    pub tags: Vec<String>,
}

#[derive(Debug, Default, serde::Deserialize)]
pub struct Filter {
    /// Texte libre : nom, note, plugin, sample ou tag.
    pub query: Option<String>,
    pub status: Option<String>,
    pub tag: Option<String>,
    #[serde(default)]
    pub favorites_only: bool,
}

pub fn search_projects(conn: &Connection, f: &Filter) -> Result<Vec<ProjectSummary>> {
    let fts = f.query.as_deref().and_then(fts_query);
    let mut stmt = conn.prepare(
        "SELECT p.id, p.name, p.folder, p.tempo, p.live_version, p.modified_at, p.status, p.favorite,
             (SELECT group_concat(t.name, char(31)) FROM project_tags pt JOIN tags t ON t.id = pt.tag_id
                 WHERE pt.project_id = p.id)
         FROM projects p
         WHERE (?1 IS NULL OR p.id IN (SELECT rowid FROM search WHERE search MATCH ?1))
           AND (?2 IS NULL OR p.status = ?2)
           AND (?3 IS NULL OR p.id IN (SELECT pt.project_id FROM project_tags pt
                                       JOIN tags t ON t.id = pt.tag_id WHERE t.name = ?3))
           AND (?4 = 0 OR p.favorite = 1)
         ORDER BY p.modified_at DESC",
    )?;
    let rows = stmt.query_map(params![fts, f.status, f.tag, f.favorites_only], |r| {
        let tags: Option<String> = r.get(8)?;
        Ok(ProjectSummary {
            id: r.get(0)?,
            name: r.get(1)?,
            folder: r.get(2)?,
            tempo: r.get(3)?,
            live_version: r.get(4)?,
            modified_at: r.get(5)?,
            status: r.get(6)?,
            favorite: r.get(7)?,
            tags: split_tags(tags),
        })
    })?;
    rows.collect()
}

/// Transforme une saisie libre en requête FTS5 : chaque mot est un préfixe,
/// tous doivent correspondre. Les guillemets protègent les caractères spéciaux.
fn fts_query(input: &str) -> Option<String> {
    let terms: Vec<String> = input
        .split_whitespace()
        .map(|w| format!("\"{}\"*", w.replace('"', "\"\"")))
        .collect();
    (!terms.is_empty()).then(|| terms.join(" "))
}

fn split_tags(tags: Option<String>) -> Vec<String> {
    tags.map(|t| t.split('\u{1f}').map(str::to_owned).collect())
        .unwrap_or_default()
}

#[derive(Debug, Serialize)]
pub struct TrackRow {
    pub name: String,
    pub kind: String,
}

#[derive(Debug, Serialize)]
pub struct ItemRow {
    pub name: String,
    pub path: Option<String>,
    /// Pour un sample : le fichier existe-t-il encore sur le disque ?
    pub missing: bool,
}

#[derive(Debug, Serialize)]
pub struct ProjectDetail {
    #[serde(flatten)]
    pub summary: ProjectSummary,
    pub als_path: String,
    pub time_sig: Option<String>,
    pub note: String,
    pub tracks: Vec<TrackRow>,
    pub plugins: Vec<ItemRow>,
    pub samples: Vec<ItemRow>,
    pub other_sets: Vec<ItemRow>,
}

pub fn get_project(conn: &Connection, id: i64) -> Result<Option<ProjectDetail>> {
    let row = conn
        .query_row(
            "SELECT p.id, p.name, p.folder, p.tempo, p.live_version, p.modified_at, p.status,
                 p.favorite, p.als_path, p.time_sig, p.note,
                 (SELECT group_concat(t.name, char(31)) FROM project_tags pt
                     JOIN tags t ON t.id = pt.tag_id WHERE pt.project_id = p.id)
             FROM projects p WHERE p.id = ?1",
            [id],
            |r| {
                Ok((
                    ProjectSummary {
                        id: r.get(0)?,
                        name: r.get(1)?,
                        folder: r.get(2)?,
                        tempo: r.get(3)?,
                        live_version: r.get(4)?,
                        modified_at: r.get(5)?,
                        status: r.get(6)?,
                        favorite: r.get(7)?,
                        tags: split_tags(r.get(11)?),
                    },
                    r.get::<_, String>(8)?,
                    r.get::<_, Option<String>>(9)?,
                    r.get::<_, String>(10)?,
                ))
            },
        )
        .optional()?;
    let Some((summary, als_path, time_sig, note)) = row else { return Ok(None) };

    let mut stmt =
        conn.prepare("SELECT name, kind FROM tracks WHERE project_id = ?1 ORDER BY position")?;
    let tracks = stmt
        .query_map([id], |r| Ok(TrackRow { name: r.get(0)?, kind: r.get(1)? }))?
        .collect::<Result<_>>()?;

    let items = |kind: &str| -> Result<Vec<ItemRow>> {
        let mut stmt = conn.prepare(
            "SELECT name, path, rel_path FROM project_items
             WHERE project_id = ?1 AND kind = ?2 ORDER BY name",
        )?;
        let folder = Path::new(&summary.folder);
        let rows = stmt.query_map(params![id, kind], |r| {
            let path: Option<String> = r.get(1)?;
            let rel_path: Option<String> = r.get(2)?;
            // Le projet a pu être déplacé : le chemin relatif peut marcher
            // alors que l'absolu ne marche plus, et inversement.
            let found = path.as_deref().is_some_and(|p| Path::new(p).exists())
                || rel_path.as_deref().is_some_and(|p| folder.join(p).exists());
            let missing = kind == "sample" && !found;
            Ok(ItemRow { name: r.get(0)?, path: path.or(rel_path), missing })
        })?;
        rows.collect()
    };

    Ok(Some(ProjectDetail {
        plugins: items("plugin")?,
        samples: items("sample")?,
        other_sets: items("set")?,
        summary,
        als_path,
        time_sig,
        note,
        tracks,
    }))
}

pub fn als_path(conn: &Connection, id: i64) -> Result<Option<(String, String)>> {
    conn.query_row(
        "SELECT als_path, folder FROM projects WHERE id = ?1",
        [id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .optional()
}

pub fn all_tags(conn: &Connection) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT name FROM tags WHERE id IN (SELECT tag_id FROM project_tags) ORDER BY name",
    )?;
    let rows = stmt.query_map([], |r| r.get(0))?;
    rows.collect()
}

// ---------- Modifications par l'utilisateur ----------

pub const STATUSES: &[&str] = &["idee", "en_cours", "fini", "abandonne"];

pub fn set_status(conn: &Connection, id: i64, status: &str) -> Result<()> {
    conn.execute("UPDATE projects SET status = ?2 WHERE id = ?1", params![id, status])?;
    Ok(())
}

pub fn set_favorite(conn: &Connection, id: i64, favorite: bool) -> Result<()> {
    conn.execute("UPDATE projects SET favorite = ?2 WHERE id = ?1", params![id, favorite])?;
    Ok(())
}

pub fn set_note(conn: &Connection, id: i64, note: &str) -> Result<()> {
    conn.execute("UPDATE projects SET note = ?2 WHERE id = ?1", params![id, note])?;
    refresh_search(conn, id)
}

pub fn add_tag(conn: &Connection, id: i64, tag: &str) -> Result<()> {
    let tag = tag.trim();
    if tag.is_empty() {
        return Ok(());
    }
    conn.execute("INSERT OR IGNORE INTO tags (name) VALUES (?1)", [tag])?;
    conn.execute(
        "INSERT OR IGNORE INTO project_tags (project_id, tag_id)
         SELECT ?1, id FROM tags WHERE name = ?2",
        params![id, tag],
    )?;
    refresh_search(conn, id)
}

pub fn remove_tag(conn: &Connection, id: i64, tag: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM project_tags WHERE project_id = ?1
             AND tag_id = (SELECT id FROM tags WHERE name = ?2)",
        params![id, tag],
    )?;
    refresh_search(conn, id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use als_parser::{Plugin, PluginFormat, Sample, Track, TrackKind};

    fn sample_set() -> LiveSet {
        LiveSet {
            creator: "Ableton Live 12.2.5".into(),
            tempo: Some(117.0),
            time_signature: Some((4, 4)),
            tracks: vec![Track { name: "Kontakt 8".into(), kind: TrackKind::Midi }],
            plugins: vec![Plugin { name: "Serum 2".into(), format: PluginFormat::Vst3 }],
            samples: vec![Sample {
                path: Some("F:/musique/Pack/Aubit - Awake Clap 6.wav".into()),
                ..Default::default()
            }],
            locators: vec![],
        }
    }

    fn insert(conn: &mut Connection, als: &str, name: &str, mtime: i64) -> i64 {
        let set = sample_set();
        let p = ScannedProject {
            root_id: None,
            folder: "F:/musique/Projets/x Project",
            als_path: als,
            name,
            modified_at: mtime,
            set: &set,
            other_sets: &[],
        };
        upsert_project(conn, &p, 100).unwrap()
    }

    #[test]
    fn search_by_name_plugin_sample_and_tag() {
        let mut conn = open_in_memory().unwrap();
        let id = insert(&mut conn, "a.als", "Woodkid perc", 10);
        insert(&mut conn, "b.als", "Autre", 20);
        add_tag(&conn, id, "cinématique").unwrap();

        let find = |q: &str| -> Vec<String> {
            let f = Filter { query: Some(q.into()), ..Default::default() };
            search_projects(&conn, &f).unwrap().into_iter().map(|p| p.name).collect()
        };
        assert_eq!(find("wood"), ["Woodkid perc"]);
        assert_eq!(find("serum").len(), 2);
        assert_eq!(find("awake clap").len(), 2);
        assert_eq!(find("cinematique"), ["Woodkid perc"], "accents ignorés");
        assert!(find("introuvable").is_empty());
        assert_eq!(find("\"wood"), ["Woodkid perc"], "un guillemet ne casse pas la recherche");
    }

    #[test]
    fn rescan_keeps_user_data() {
        let mut conn = open_in_memory().unwrap();
        let id = insert(&mut conn, "a.als", "Projet", 10);
        set_note(&conn, id, "garder le drop").unwrap();
        set_favorite(&conn, id, true).unwrap();
        add_tag(&conn, id, "club").unwrap();

        assert_eq!(insert(&mut conn, "a.als", "Projet", 50), id);
        let p = get_project(&conn, id).unwrap().unwrap();
        assert_eq!(p.note, "garder le drop");
        assert!(p.summary.favorite);
        assert_eq!(p.summary.tags, ["club"]);
        assert_eq!(p.summary.modified_at, 50);
        assert_eq!(p.tracks.len(), 1);
        assert_eq!(p.samples[0].name, "Aubit - Awake Clap 6.wav");
        assert!(p.samples[0].missing);
    }

    /// Scan complet (scanner + parseur + base) sur de vrais projets si
    /// `ALS_FIXTURES` pointe vers un dossier.
    #[test]
    fn real_library() {
        let Ok(dir) = std::env::var("ALS_FIXTURES") else { return };
        let mut conn = open_in_memory().unwrap();
        for p in crate::scanner::find_projects(Path::new(&dir)) {
            let set = als_parser::parse_file(&p.main_set).unwrap();
            let others: Vec<String> =
                p.other_sets.iter().map(|o| o.to_string_lossy().into_owned()).collect();
            let (name, folder, als) =
                (p.name(), p.folder.to_string_lossy(), p.main_set.to_string_lossy());
            let scanned = ScannedProject {
                root_id: None,
                folder: &folder,
                als_path: &als,
                name: &name,
                modified_at: p.modified_at,
                set: &set,
                other_sets: &others,
            };
            upsert_project(&mut conn, &scanned, 1).unwrap();
        }
        for p in search_projects(&conn, &Filter::default()).unwrap() {
            let d = get_project(&conn, p.id).unwrap().unwrap();
            let missing = d.samples.iter().filter(|s| s.missing).count();
            println!(
                "{} | {:?} BPM | {} pistes | plugins: {} | {} samples ({missing} introuvables) | autres sets: {}",
                p.name,
                p.tempo,
                d.tracks.len(),
                d.plugins.iter().map(|i| i.name.as_str()).collect::<Vec<_>>().join(", "),
                d.samples.len(),
                d.other_sets.len()
            );
        }
        let f = Filter { query: Some("kontakt".into()), ..Default::default() };
        println!("recherche « kontakt » : {} projets", search_projects(&conn, &f).unwrap().len());
    }

    #[test]
    fn newest_first() {
        let mut conn = open_in_memory().unwrap();
        insert(&mut conn, "a.als", "Vieux", 10);
        insert(&mut conn, "b.als", "Récent", 20);
        let names: Vec<_> = search_projects(&conn, &Filter::default())
            .unwrap()
            .into_iter()
            .map(|p| p.name)
            .collect();
        assert_eq!(names, ["Récent", "Vieux"]);
    }
}
