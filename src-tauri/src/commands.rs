//! Commandes appelées par l'interface (via `invoke`).

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_opener::OpenerExt;

use crate::db::{self, Filter, ProjectDetail, ProjectSummary, Root};
use crate::{scanner, AppState};

/// Erreur renvoyée à l'interface sous forme de texte.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("base de données : {0}")]
    Db(#[from] rusqlite::Error),
    #[error("{0}")]
    Other(String),
}

impl Serialize for Error {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

type Result<T> = std::result::Result<T, Error>;

fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

#[tauri::command]
pub fn list_roots(state: State<AppState>) -> Result<Vec<Root>> {
    Ok(db::list_roots(&state.db.lock().unwrap())?)
}

#[tauri::command]
pub fn add_root(state: State<AppState>, path: String) -> Result<i64> {
    Ok(db::add_root(&state.db.lock().unwrap(), &path)?)
}

#[tauri::command]
pub fn remove_root(state: State<AppState>, id: i64) -> Result<()> {
    Ok(db::remove_root(&state.db.lock().unwrap(), id)?)
}

#[derive(Clone, Serialize)]
struct ScanProgress {
    root: String,
    done: usize,
    total: usize,
    current: String,
}

#[derive(Serialize)]
pub struct ScanReport {
    pub found: usize,
    pub updated: usize,
    pub removed: usize,
    /// .als illisibles : (chemin, raison)
    pub errors: Vec<(String, String)>,
}

/// Scanne toutes les racines. Les .als inchangés depuis le dernier scan ne
/// sont pas relus. Envoie un évènement `scan-progress` après chaque projet.
#[tauri::command]
pub async fn scan(app: AppHandle) -> Result<ScanReport> {
    tauri::async_runtime::spawn_blocking(move || scan_blocking(&app))
        .await
        .map_err(|e| Error::Other(e.to_string()))?
}

fn scan_blocking(app: &AppHandle) -> Result<ScanReport> {
    let state = app.state::<AppState>();
    let roots = db::list_roots(&state.db.lock().unwrap())?;
    let mut report = ScanReport { found: 0, updated: 0, removed: 0, errors: vec![] };

    for root in roots {
        let started = now();
        let projects = scanner::find_projects(Path::new(&root.path));
        report.found += projects.len();

        for (i, p) in projects.iter().enumerate() {
            let als = p.main_set.to_string_lossy().into_owned();
            let _ = app.emit(
                "scan-progress",
                ScanProgress { root: root.path.clone(), done: i, total: projects.len(), current: p.name() },
            );

            let conn = state.db.lock().unwrap();
            if db::indexed_mtime(&conn, &als)? == Some(p.modified_at) {
                // Inchangé : on marque juste le projet comme revu.
                conn.execute("UPDATE projects SET scanned_at = ?2 WHERE als_path = ?1", (&als, started))?;
                continue;
            }
            drop(conn); // la lecture du .als peut être longue : on libère la base

            let set = match als_parser::parse_file(&p.main_set) {
                Ok(set) => set,
                Err(e) => {
                    report.errors.push((als, e.to_string()));
                    continue;
                }
            };
            let others: Vec<String> =
                p.other_sets.iter().map(|o| o.to_string_lossy().into_owned()).collect();
            let name = p.name();
            let folder = p.folder.to_string_lossy();
            let scanned = db::ScannedProject {
                root_id: Some(root.id),
                folder: &folder,
                als_path: &als,
                name: &name,
                modified_at: p.modified_at,
                set: &set,
                other_sets: &others,
            };
            db::upsert_project(&mut state.db.lock().unwrap(), &scanned, started)?;
            report.updated += 1;
        }
        report.removed += db::remove_missing(&state.db.lock().unwrap(), root.id, started)?;
    }
    let _ = app.emit("scan-done", ());
    Ok(report)
}

#[tauri::command]
pub fn search_projects(state: State<AppState>, filter: Filter) -> Result<Vec<ProjectSummary>> {
    Ok(db::search_projects(&state.db.lock().unwrap(), &filter)?)
}

#[tauri::command]
pub fn get_project(state: State<AppState>, id: i64) -> Result<Option<ProjectDetail>> {
    Ok(db::get_project(&state.db.lock().unwrap(), id)?)
}

#[tauri::command]
pub fn all_tags(state: State<AppState>) -> Result<Vec<String>> {
    Ok(db::all_tags(&state.db.lock().unwrap())?)
}

#[tauri::command]
pub fn set_status(state: State<AppState>, id: i64, status: String) -> Result<()> {
    if !db::STATUSES.contains(&status.as_str()) {
        return Err(Error::Other(format!("statut inconnu : {status}")));
    }
    Ok(db::set_status(&state.db.lock().unwrap(), id, &status)?)
}

#[tauri::command]
pub fn set_favorite(state: State<AppState>, id: i64, favorite: bool) -> Result<()> {
    Ok(db::set_favorite(&state.db.lock().unwrap(), id, favorite)?)
}

#[tauri::command]
pub fn set_note(state: State<AppState>, id: i64, note: String) -> Result<()> {
    Ok(db::set_note(&state.db.lock().unwrap(), id, &note)?)
}

#[tauri::command]
pub fn add_tag(state: State<AppState>, id: i64, tag: String) -> Result<()> {
    Ok(db::add_tag(&state.db.lock().unwrap(), id, &tag)?)
}

#[tauri::command]
pub fn remove_tag(state: State<AppState>, id: i64, tag: String) -> Result<()> {
    Ok(db::remove_tag(&state.db.lock().unwrap(), id, &tag)?)
}

fn paths(state: &State<AppState>, id: i64) -> Result<(String, String)> {
    db::als_path(&state.db.lock().unwrap(), id)?
        .ok_or_else(|| Error::Other("projet introuvable".into()))
}

/// Ouvre le .als avec l'application associée (Ableton Live).
#[tauri::command]
pub fn open_in_ableton(app: AppHandle, state: State<AppState>, id: i64) -> Result<()> {
    let (als, _) = paths(&state, id)?;
    app.opener()
        .open_path(als, None::<&str>)
        .map_err(|e| Error::Other(e.to_string()))
}

/// Ouvre l'explorateur sur le dossier du projet, .als sélectionné.
#[tauri::command]
pub fn open_folder(app: AppHandle, state: State<AppState>, id: i64) -> Result<()> {
    let (als, _) = paths(&state, id)?;
    app.opener()
        .reveal_item_in_dir(als)
        .map_err(|e| Error::Other(e.to_string()))
}
