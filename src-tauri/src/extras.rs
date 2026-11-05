//! Sauvegarde dans un dossier synchronisé (cloud), samples manquants avant
//! ouverture, exports du projet, plugins non installés et tâches par projet.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use walkdir::WalkDir;

use crate::db;

type Result<T> = std::result::Result<T, String>;

fn sql(e: rusqlite::Error) -> String {
    format!("base de données : {e}")
}

// ---------- Sauvegarde cloud ----------

const CLOUD_DIR: &str = "cloud_dir";

pub fn cloud_dir(conn: &Connection) -> Result<Option<String>> {
    db::get_setting(conn, CLOUD_DIR).map_err(sql).map(|v| v.filter(|s| !s.is_empty()))
}

pub fn set_cloud_dir(conn: &Connection, dir: &str) -> Result<()> {
    db::set_setting(conn, CLOUD_DIR, dir).map_err(sql)
}

pub fn last_backup(conn: &Connection, project_id: i64) -> Result<Option<i64>> {
    Ok(db::get_setting(conn, &format!("cloud_last:{project_id}"))
        .map_err(sql)?
        .and_then(|v| v.parse().ok()))
}

pub fn set_last_backup(conn: &Connection, project_id: i64, at: i64) -> Result<()> {
    db::set_setting(conn, &format!("cloud_last:{project_id}"), &at.to_string()).map_err(sql)
}

#[derive(Debug, Serialize, Default, PartialEq)]
pub struct BackupReport {
    pub dest: String,
    /// Fichiers copiés (nouveaux ou modifiés).
    pub copied: usize,
    /// Fichiers déjà à jour.
    pub unchanged: usize,
    pub bytes: u64,
}

/// Dossiers que Live recrée tout seul : inutile de les envoyer.
fn skipped(name: &str) -> bool {
    matches!(name, "Backup" | "Ableton Project Info")
}

/// Copie le dossier du projet dans `cloud/<nom du dossier>`, seulement les
/// fichiers nouveaux ou modifiés (taille ou date). Rien n'est supprimé côté cloud.
pub fn backup_folder(folder: &Path, cloud: &Path) -> Result<BackupReport> {
    let name = folder.file_name().ok_or("dossier du projet invalide")?;
    let dest = cloud.join(name);
    let mut report = BackupReport { dest: dest.to_string_lossy().into_owned(), ..Default::default() };
    let walker = WalkDir::new(folder).into_iter().filter_entry(|e| {
        e.depth() != 1 || !e.file_type().is_dir() || !skipped(&e.file_name().to_string_lossy())
    });
    for entry in walker {
        let entry = entry.map_err(|e| e.to_string())?;
        if !entry.file_type().is_file() {
            continue;
        }
        let rel = entry.path().strip_prefix(folder).map_err(|e| e.to_string())?;
        let target = dest.join(rel);
        let meta = entry.metadata().map_err(|e| e.to_string())?;
        let same = fs::metadata(&target).is_ok_and(|t| {
            t.len() == meta.len() && t.modified().ok() >= meta.modified().ok()
        });
        if same {
            report.unchanged += 1;
            continue;
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        fs::copy(entry.path(), &target).map_err(|e| format!("{}: {e}", target.display()))?;
        report.copied += 1;
        report.bytes += meta.len();
    }
    Ok(report)
}

// ---------- Samples manquants d'un projet ----------

/// Noms des samples introuvables du projet (même test que le docteur).
pub fn missing_samples(conn: &Connection, project_id: i64) -> Result<Vec<String>> {
    let folder: Option<String> = conn
        .query_row("SELECT folder FROM projects WHERE id = ?1", [project_id], |r| r.get(0))
        .optional()
        .map_err(sql)?;
    let Some(folder) = folder else { return Ok(vec![]) };
    let mut stmt = conn
        .prepare(
            "SELECT DISTINCT name, path, rel_path FROM project_items
             WHERE project_id = ?1 AND kind = 'sample' ORDER BY name",
        )
        .map_err(sql)?;
    let rows = stmt
        .query_map([project_id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, Option<String>>(2)?))
        })
        .map_err(sql)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(sql)?;
    Ok(rows
        .into_iter()
        .filter(|(_, p, r)| !db::sample_found(Path::new(&folder), p.as_deref(), r.as_deref()))
        .map(|(n, _, _)| n)
        .collect())
}

// ---------- Exports du projet ----------

const AUDIO: &[&str] = &["wav", "aif", "aiff", "mp3", "flac", "ogg", "m4a"];

#[derive(Debug, Serialize, PartialEq)]
pub struct ExportFile {
    pub path: String,
    pub name: String,
    pub size: u64,
    pub modified: i64,
}

/// Fichiers audio exportés : à la racine du dossier du projet et dans ses
/// sous-dossiers directs, sauf Samples, Backup et les infos de Live.
/// Les plus récents d'abord.
pub fn list_exports(folder: &Path) -> Vec<ExportFile> {
    let mut out: Vec<ExportFile> = WalkDir::new(folder)
        .max_depth(2)
        .into_iter()
        .filter_entry(|e| {
            e.depth() != 1
                || !e.file_type().is_dir()
                || !matches!(e.file_name().to_string_lossy().as_ref(), "Samples" | "Backup" | "Ableton Project Info")
        })
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            let ext = path.extension()?.to_str()?.to_lowercase();
            let meta = e.metadata().ok()?;
            if !meta.is_file() || meta.len() < 50_000 || !AUDIO.contains(&ext.as_str()) {
                return None;
            }
            let modified = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_secs() as i64;
            Some(ExportFile {
                path: path.to_string_lossy().into_owned(),
                name: e.file_name().to_string_lossy().into_owned(),
                size: meta.len(),
                modified,
            })
        })
        .collect();
    out.sort_by(|a, b| b.modified.cmp(&a.modified));
    out
}

// ---------- Plugins non installés ----------

/// Dossiers de plugins habituels sous Windows.
fn plugin_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    for var in ["CommonProgramFiles", "ProgramFiles", "ProgramFiles(x86)", "CommonProgramFiles(x86)"] {
        if let Ok(base) = std::env::var(var) {
            let base = PathBuf::from(base);
            for sub in ["VST3", "VST2", "VST", "VSTPlugins", "Steinberg/VstPlugins", "CLAP", "Common Files/VST3", "Common Files/VST2"] {
                dirs.push(base.join(sub));
            }
        }
    }
    dirs
}

/// Nom réduit aux lettres et chiffres, en minuscules, pour comparer.
pub fn normalize(name: &str) -> String {
    name.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// Noms (réduits) des plugins trouvés dans les dossiers donnés.
pub fn installed_plugins(dirs: &[PathBuf]) -> HashSet<String> {
    let mut out = HashSet::new();
    for dir in dirs {
        for e in WalkDir::new(dir).max_depth(6).into_iter().flatten() {
            let path = e.path();
            let Some(ext) = path.extension().and_then(|x| x.to_str()) else { continue };
            if matches!(ext.to_lowercase().as_str(), "vst3" | "dll" | "clap") {
                if let Some(stem) = path.file_stem() {
                    out.insert(normalize(&stem.to_string_lossy()));
                }
            }
        }
    }
    out
}

/// Vrai si un fichier installé ressemble au nom du plugin noté dans le set.
pub fn is_installed(name: &str, installed: &HashSet<String>) -> bool {
    let n = normalize(name);
    if n.is_empty() {
        return true;
    }
    installed.iter().any(|i| i == &n || (i.len() >= 4 && n.contains(i.as_str())) || (n.len() >= 4 && i.contains(n.as_str())))
}

/// Plugins de la bibliothèque introuvables sur ce PC. Vide si aucun dossier
/// de plugins n'existe (on ne sait pas juger).
pub fn missing_plugins(conn: &Connection) -> Result<Vec<String>> {
    let dirs: Vec<PathBuf> = plugin_dirs().into_iter().filter(|d| d.is_dir()).collect();
    if dirs.is_empty() {
        return Ok(vec![]);
    }
    let installed = installed_plugins(&dirs);
    Ok(db::plugin_inventory(conn)
        .map_err(sql)?
        .into_iter()
        .map(|p| p.name)
        .filter(|n| !is_installed(n, &installed))
        .collect())
}

// ---------- Tâches ----------

#[derive(Debug, Serialize, PartialEq)]
pub struct Todo {
    pub id: i64,
    pub project_id: i64,
    pub project_name: String,
    pub text: String,
    pub done: bool,
    pub created_at: i64,
}

const TODO_SELECT: &str = "SELECT t.id, t.project_id, p.name, t.text, t.done, t.created_at
     FROM todos t JOIN projects p ON p.id = t.project_id";

fn todo_row(r: &rusqlite::Row) -> rusqlite::Result<Todo> {
    Ok(Todo {
        id: r.get(0)?,
        project_id: r.get(1)?,
        project_name: r.get(2)?,
        text: r.get(3)?,
        done: r.get::<_, i64>(4)? != 0,
        created_at: r.get(5)?,
    })
}

/// Tâches d'un projet, ou toutes les tâches à faire si `project_id` est vide.
pub fn list_todos(conn: &Connection, project_id: Option<i64>) -> Result<Vec<Todo>> {
    let (q, arg) = match project_id {
        Some(id) => (format!("{TODO_SELECT} WHERE t.project_id = ?1 ORDER BY t.done, t.id"), id),
        None => (format!("{TODO_SELECT} WHERE t.done = 0 AND ?1 = ?1 ORDER BY p.name COLLATE NOCASE, t.id"), 0),
    };
    let mut stmt = conn.prepare(&q).map_err(sql)?;
    let rows = stmt.query_map([arg], todo_row).map_err(sql)?;
    rows.collect::<rusqlite::Result<_>>().map_err(sql)
}

pub fn add_todo(conn: &Connection, project_id: i64, text: &str, now: i64) -> Result<i64> {
    let text = text.trim();
    if text.is_empty() {
        return Err("tâche vide".into());
    }
    conn.execute(
        "INSERT INTO todos (project_id, text, done, created_at) VALUES (?1, ?2, 0, ?3)",
        params![project_id, text, now],
    )
    .map_err(sql)?;
    Ok(conn.last_insert_rowid())
}

pub fn set_todo_done(conn: &Connection, id: i64, done: bool) -> Result<()> {
    conn.execute("UPDATE todos SET done = ?2 WHERE id = ?1", params![id, done as i64]).map_err(sql)?;
    Ok(())
}

pub fn delete_todo(conn: &Connection, id: i64) -> Result<()> {
    conn.execute("DELETE FROM todos WHERE id = ?1", [id]).map_err(sql)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backup_copies_only_changes_and_skips_backup() {
        let src = tempfile::tempdir().unwrap();
        let cloud = tempfile::tempdir().unwrap();
        let folder = src.path().join("Morceau Project");
        fs::create_dir_all(folder.join("Backup")).unwrap();
        fs::create_dir_all(folder.join("Samples")).unwrap();
        fs::write(folder.join("Morceau.als"), b"als").unwrap();
        fs::write(folder.join("Samples/kick.wav"), b"wav").unwrap();
        fs::write(folder.join("Backup/old.als"), b"old").unwrap();

        let r = backup_folder(&folder, cloud.path()).unwrap();
        assert_eq!((r.copied, r.unchanged), (2, 0));
        assert!(cloud.path().join("Morceau Project/Samples/kick.wav").exists());
        assert!(!cloud.path().join("Morceau Project/Backup").exists());

        let r = backup_folder(&folder, cloud.path()).unwrap();
        assert_eq!((r.copied, r.unchanged), (0, 2));
    }

    #[test]
    fn plugin_matching() {
        let installed: HashSet<String> = ["serum2", "fabfilterproq3"].iter().map(|s| s.to_string()).collect();
        assert!(is_installed("Serum 2", &installed));
        assert!(is_installed("FabFilter Pro-Q 3", &installed));
        assert!(!is_installed("Vital", &installed));
    }

    #[test]
    fn todos_roundtrip() {
        let conn = db::open_in_memory().unwrap();
        conn.execute(
            "INSERT INTO projects (id, folder, als_path, name, modified_at, scanned_at) VALUES (1, '.', 'a.als', 'A', 0, 0)",
            [],
        )
        .unwrap();
        let id = add_todo(&conn, 1, "  mix du kick ", 10).unwrap();
        assert_eq!(list_todos(&conn, None).unwrap()[0].text, "mix du kick");
        set_todo_done(&conn, id, true).unwrap();
        assert!(list_todos(&conn, None).unwrap().is_empty());
        assert_eq!(list_todos(&conn, Some(1)).unwrap().len(), 1);
        delete_todo(&conn, id).unwrap();
        assert!(list_todos(&conn, Some(1)).unwrap().is_empty());
    }
}
