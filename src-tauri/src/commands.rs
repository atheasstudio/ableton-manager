//! Commandes appelées par l'interface (via `invoke`).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_opener::OpenerExt;

use crate::db::{self, Filter, ProjectDetail, ProjectSummary, Root};
use crate::versions::{self, Branch, Diff, NewBranch, RestoreReport, VersionRow};
use crate::watcher::SaveWatcher;
use crate::{scanner, AppState, VAULT_DIR};

/// Erreur renvoyée à l'interface sous forme de texte.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("base de données : {0}")]
    Db(#[from] rusqlite::Error),
    #[error("{0}")]
    Versions(#[from] versions::Error),
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
            // Dossier d'une branche : il appartient au projet d'origine.
            if db::is_branch_folder(&conn, &p.folder.to_string_lossy())? {
                continue;
            }
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

// ---------- Versions et branches ----------

/// Lance un travail long sur le coffre hors du fil de l'interface, avec sa
/// propre connexion à la base.
async fn in_background<T: Send + 'static>(
    app: AppHandle,
    work: impl FnOnce(&rusqlite::Connection, &crate::vault::Vault) -> versions::Result<T> + Send + 'static,
) -> Result<T> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let conn = state.open_db()?;
        let vault = state.vault(&conn)?;
        Ok(work(&conn, &vault)?)
    })
    .await
    .map_err(|e| Error::Other(e.to_string()))?
}

#[derive(Serialize)]
pub struct VersionsView {
    /// Vide si les versions ne sont pas activées pour ce projet.
    pub branches: Vec<Branch>,
    /// De la plus récente à la plus ancienne.
    pub versions: Vec<VersionRow>,
}

#[tauri::command]
pub fn get_versions(state: State<AppState>, project_id: i64) -> Result<VersionsView> {
    let conn = state.db.lock().unwrap();
    Ok(VersionsView {
        branches: versions::branches(&conn, project_id)?,
        versions: versions::list(&conn, project_id)?,
    })
}

/// Active les versions d'un projet. Renvoie le nombre de versions créées.
#[tauri::command]
pub async fn enable_versions(
    app: AppHandle,
    watcher: State<'_, Arc<SaveWatcher>>,
    project_id: i64,
    import_backups: bool,
) -> Result<usize> {
    let created = in_background(app.clone(), move |c, v| {
        versions::enable(c, v, project_id, import_backups, now())
    })
    .await?;
    watcher.sync(&app);
    Ok(created)
}

/// Crée une version tout de suite (sans attendre une sauvegarde).
#[tauri::command]
pub async fn snapshot_now(app: AppHandle, branch_id: i64) -> Result<Option<i64>> {
    in_background(app, move |c, v| versions::snapshot(c, v, branch_id, "manual")).await
}

#[tauri::command]
pub fn rename_version(state: State<AppState>, id: i64, name: String) -> Result<()> {
    Ok(versions::rename(&state.db.lock().unwrap(), id, &name)?)
}

#[tauri::command]
pub async fn restore_version(app: AppHandle, id: i64) -> Result<RestoreReport> {
    in_background(app, move |c, v| versions::restore(c, v, id, now())).await
}

#[tauri::command]
pub async fn create_branch(
    app: AppHandle,
    watcher: State<'_, Arc<SaveWatcher>>,
    version_id: i64,
    name: String,
) -> Result<NewBranch> {
    let created =
        in_background(app.clone(), move |c, v| versions::create_branch(c, v, version_id, &name, now()))
            .await?;
    watcher.sync(&app);
    Ok(created)
}

/// Renvoie le nombre d'octets libérés dans le coffre.
#[tauri::command]
pub async fn delete_version(app: AppHandle, id: i64) -> Result<u64> {
    in_background(app, move |c, v| versions::delete(c, v, id)).await
}

#[tauri::command]
pub fn compare_versions(state: State<AppState>, a: i64, b: i64) -> Result<Diff> {
    Ok(versions::compare(&state.db.lock().unwrap(), a, b)?)
}

#[tauri::command]
pub fn open_branch(app: AppHandle, state: State<AppState>, branch_id: i64) -> Result<()> {
    let b = versions::branch(&state.db.lock().unwrap(), branch_id)?;
    app.opener().open_path(b.als_path, None::<&str>).map_err(|e| Error::Other(e.to_string()))
}

#[tauri::command]
pub fn open_branch_folder(app: AppHandle, state: State<AppState>, branch_id: i64) -> Result<()> {
    let b = versions::branch(&state.db.lock().unwrap(), branch_id)?;
    app.opener().reveal_item_in_dir(b.als_path).map_err(|e| Error::Other(e.to_string()))
}

#[derive(Serialize)]
pub struct VaultInfo {
    pub path: String,
    pub files: usize,
    pub bytes: u64,
}

#[tauri::command]
pub async fn vault_info(app: AppHandle) -> Result<VaultInfo> {
    in_background(app, |_, v| {
        let (files, bytes) = v.usage()?;
        Ok(VaultInfo { path: v.root().to_string_lossy().into_owned(), files, bytes })
    })
    .await
}

/// Change le dossier du coffre et y déplace les fichiers déjà rangés.
#[tauri::command]
pub async fn set_vault_dir(app: AppHandle, path: String) -> Result<()> {
    in_background(app, move |c, v| {
        versions::move_vault(v, &PathBuf::from(&path))?;
        db::set_setting(c, VAULT_DIR, &path)?;
        Ok(())
    })
    .await
}
