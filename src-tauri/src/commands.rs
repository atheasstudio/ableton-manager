//! Commandes appelées par l'interface (via `invoke`).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_opener::OpenerExt;

use crate::annotations::{self, Annotation};
use crate::capture::{self, CaptureLink, CaptureStatus};
use crate::db::{self, Filter, ProjectDetail, ProjectSummary, Root};
use crate::live::{self, Live, Transport};
use crate::live_window;
use crate::previews::{self, PreviewInfo};
use crate::versions::{self, Branch, Diff, NewBranch, RestoreReport, VersionRow};
use crate::watcher::SaveWatcher;
use crate::{notes, organize, scanner, worktime, AppState, VAULT_DIR};

/// Erreur renvoyée à l'interface sous forme de texte.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("base de données : {0}")]
    Db(#[from] rusqlite::Error),
    #[error("{0}")]
    Versions(#[from] versions::Error),
    #[error("{0}")]
    Previews(#[from] previews::Error),
    #[error("{0}")]
    Live(#[from] live::Error),
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
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
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
    let mut report = ScanReport {
        found: 0,
        updated: 0,
        removed: 0,
        errors: vec![],
    };

    for root in roots {
        let started = now();
        let projects = scanner::find_projects(Path::new(&root.path));
        report.found += projects.len();

        for (i, p) in projects.iter().enumerate() {
            let als = p.main_set.to_string_lossy().into_owned();
            let _ = app.emit(
                "scan-progress",
                ScanProgress {
                    root: root.path.clone(),
                    done: i,
                    total: projects.len(),
                    current: p.name(),
                },
            );

            let others: Vec<String> = p
                .other_sets
                .iter()
                .map(|o| o.to_string_lossy().into_owned())
                .collect();
            let name = p.name();
            let mut conn = state.db.lock().unwrap();
            // Dossier d'une branche : il appartient au projet d'origine.
            if db::is_branch_folder(&conn, &p.folder.to_string_lossy())? {
                continue;
            }
            if db::indexed_mtime(&conn, &als)? == Some(p.modified_at) {
                // Inchangé : on marque juste le projet comme revu.
                db::mark_seen(&mut conn, &als, &name, &others, started)?;
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
    work: impl FnOnce(&rusqlite::Connection, &crate::vault::Vault) -> versions::Result<T>
        + Send
        + 'static,
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
    in_background(app, move |c, v| {
        let id = versions::snapshot(c, v, branch_id, "manual")?;
        if let Some(id) = id {
            if let Err(e) = previews::on_new_version(c, v, id, now()) {
                eprintln!("aperçu de la version : {e}");
            }
        }
        Ok(id)
    })
    .await
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
    let created = in_background(app.clone(), move |c, v| {
        versions::create_branch(c, v, version_id, &name, now())
    })
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
    app.opener()
        .open_path(b.als_path, None::<&str>)
        .map_err(|e| Error::Other(e.to_string()))
}

#[tauri::command]
pub fn open_branch_folder(app: AppHandle, state: State<AppState>, branch_id: i64) -> Result<()> {
    let b = versions::branch(&state.db.lock().unwrap(), branch_id)?;
    app.opener()
        .reveal_item_in_dir(b.als_path)
        .map_err(|e| Error::Other(e.to_string()))
}

/// Donne au set son propre dossier de projet (il partageait le sien).
#[tauri::command]
pub fn move_to_own_folder(
    app: AppHandle,
    state: State<AppState>,
    id: i64,
) -> Result<organize::Moved> {
    let moved = organize::move_to_own_folder(&state.db.lock().unwrap(), id)?;
    app.state::<Arc<SaveWatcher>>().sync(&app);
    Ok(moved)
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
        Ok(VaultInfo {
            path: v.root().to_string_lossy().into_owned(),
            files,
            bytes,
        })
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

// ---------- Pré-écoute ----------

#[tauri::command]
pub fn get_preview(state: State<AppState>, hash: String) -> Result<Option<PreviewInfo>> {
    Ok(previews::info(&state.db.lock().unwrap(), &hash)?)
}

/// Le MP3 de l'aperçu, en binaire.
#[tauri::command]
pub async fn preview_audio(app: AppHandle, hash: String) -> Result<tauri::ipc::Response> {
    let bytes = in_background(app, move |_, v| Ok(v.read(&hash)?)).await?;
    Ok(tauri::ipc::Response::new(bytes))
}

/// Bounce trouvé dans le dossier du projet, s'il y en a un.
#[tauri::command]
pub fn find_bounce(state: State<AppState>, project_id: i64) -> Result<Option<String>> {
    let (als, folder) = paths(&state, project_id)?;
    let set = Path::new(&als)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    Ok(previews::find_bounce(Path::new(&folder), &set).map(|p| p.to_string_lossy().into_owned()))
}

/// Prend un fichier audio comme aperçu du projet. Renvoie l'aperçu.
#[tauri::command]
pub async fn import_preview(app: AppHandle, project_id: i64, path: String) -> Result<String> {
    in_background(app, move |c, v| {
        previews::import_file(c, v, project_id, Path::new(&path), now())
            .map_err(|e| versions::Error::Other(e.to_string()))
    })
    .await
}

#[tauri::command]
pub fn capture_status(link: State<Arc<CaptureLink>>) -> CaptureStatus {
    link.status()
}

/// Rendu complet à la demande : ouvre le set dans Live au besoin, puis le
/// device le joue du début à la fin. La fin arrive par l'évènement `render-done`.
#[tauri::command]
pub async fn render_preview(app: AppHandle, project_id: i64) -> Result<()> {
    let (als, _) = paths(&app.state::<AppState>(), project_id)?;
    tauri::async_runtime::spawn_blocking(move || {
        let link = app.state::<Arc<CaptureLink>>();
        capture::wait_for_set(&link, &als, || {
            app.opener()
                .open_path(als.clone(), None::<&str>)
                .map_err(|e| e.to_string())
        })
        .map_err(Error::Other)?;
        link.render(project_id);
        Ok(())
    })
    .await
    .map_err(|e| Error::Other(e.to_string()))?
}

#[tauri::command]
pub fn cancel_render(link: State<Arc<CaptureLink>>) {
    link.cancel_render();
}

/// Installe le device dans la bibliothèque utilisateur de Live (par défaut
/// `Documents\Ableton\User Library`). Renvoie le chemin du device.
#[tauri::command]
pub fn install_device(app: AppHandle, user_library: Option<String>) -> Result<String> {
    let library = match user_library {
        Some(dir) => PathBuf::from(dir),
        None => {
            let docs = app
                .path()
                .document_dir()
                .map_err(|e| Error::Other(e.to_string()))?;
            let default = docs.join("Ableton").join("User Library");
            if !default.is_dir() {
                return Err(Error::Other("NO_USER_LIBRARY".into()));
            }
            default
        }
    };
    let device = capture::install_device(&capture::device_dir(&library))
        .map_err(|e| Error::Other(format!("installation du device : {e}")))?;
    Ok(device.to_string_lossy().into_owned())
}

// ---------- Timeline et pont Ableton ----------

/// Lance un travail bloquant (lecture d'un .als, attente de Live) hors du fil
/// de l'interface.
async fn blocking<T: Send + 'static>(
    app: AppHandle,
    work: impl FnOnce(&AppHandle) -> Result<T> + Send + 'static,
) -> Result<T> {
    tauri::async_runtime::spawn_blocking(move || work(&app))
        .await
        .map_err(|e| Error::Other(e.to_string()))?
}

#[derive(Serialize)]
pub struct Timeline {
    pub tempo: Option<f64>,
    /// (numérateur, dénominateur)
    pub time_sig: (u32, u32),
    /// Fin du dernier clip de l'arrangement, en beats.
    pub length: Option<f64>,
    /// Locators du set, tels que sauvegardés.
    pub locators: Vec<als_parser::Locator>,
    pub annotations: Vec<Annotation>,
}

fn parse_time_sig(s: &str) -> Option<(u32, u32)> {
    let (n, d) = s.split_once('/')?;
    Some((n.trim().parse().ok()?, d.trim().parse().ok()?))
}

/// Tempo, signature, durée et locators sont relus dans le .als (le set a pu
/// changer depuis le dernier scan) ; à défaut, on prend ceux de la base.
#[tauri::command]
pub async fn get_timeline(app: AppHandle, project_id: i64) -> Result<Timeline> {
    blocking(app, move |app| {
        let state = app.state::<AppState>();
        let (project, annotations) = {
            let conn = state.db.lock().unwrap();
            (
                db::get_project(&conn, project_id)?,
                annotations::list(&conn, project_id)?,
            )
        };
        let project = project.ok_or_else(|| Error::Other("projet introuvable".into()))?;
        let set = als_parser::parse_file(&project.als_path).ok();
        Ok(Timeline {
            tempo: set.as_ref().and_then(|s| s.tempo).or(project.summary.tempo),
            time_sig: set
                .as_ref()
                .and_then(|s| s.time_signature)
                .or_else(|| project.time_sig.as_deref().and_then(parse_time_sig))
                .unwrap_or((4, 4)),
            length: set.as_ref().and_then(|s| s.arrangement_end),
            locators: set.map(|s| s.locators).unwrap_or_default(),
            annotations,
        })
    })
    .await
}

#[tauri::command]
pub fn add_annotation(
    state: State<AppState>,
    project_id: i64,
    beat: f64,
    label: String,
) -> Result<Annotation> {
    Ok(annotations::add(
        &state.db.lock().unwrap(),
        project_id,
        beat,
        &label,
        now(),
    )?)
}

#[tauri::command]
pub fn update_annotation(state: State<AppState>, id: i64, beat: f64, label: String) -> Result<()> {
    Ok(annotations::update(
        &state.db.lock().unwrap(),
        id,
        beat,
        &label,
    )?)
}

#[tauri::command]
pub fn delete_annotation(state: State<AppState>, id: i64) -> Result<()> {
    Ok(annotations::delete(&state.db.lock().unwrap(), id)?)
}

/// Crée une annotation par locator du set. Renvoie le nombre créé.
#[tauri::command]
pub async fn import_locators(app: AppHandle, project_id: i64) -> Result<usize> {
    blocking(app, move |app| {
        let state = app.state::<AppState>();
        let (als, _) = paths(&state, project_id)?;
        let set = als_parser::parse_file(&als).map_err(|e| Error::Other(e.to_string()))?;
        let mut conn = state.db.lock().unwrap();
        let created = annotations::import_locators(&mut conn, project_id, &set.locators, now())?;
        Ok(created)
    })
    .await
}

/// .als dont l'ouverture dans Live compte comme « le bon projet » : le projet
/// et ses branches. Le premier est celui qu'on ouvre.
fn project_sets(state: &State<AppState>, project_id: i64) -> Result<Vec<PathBuf>> {
    let (als, _) = paths(state, project_id)?;
    let conn = state.db.lock().unwrap();
    let mut sets = vec![PathBuf::from(als)];
    let branches = versions::branches(&conn, project_id)?;
    sets.extend(branches.into_iter().map(|b| PathBuf::from(b.als_path)));
    Ok(sets)
}

#[derive(Serialize)]
pub struct LiveStatus {
    /// Version du script, `None` s'il ne répond pas.
    pub script: Option<i32>,
    /// Version que l'appli sait installer.
    pub expected_script: i32,
    /// Set ouvert dans Live, d'après le titre de sa fenêtre.
    pub open_set: Option<String>,
    /// Le set ouvert est-il ce projet ? `None` si on ne peut pas le savoir.
    pub is_open: Option<bool>,
    pub transport: Option<Transport>,
}

#[tauri::command]
pub async fn live_status(app: AppHandle, project_id: i64) -> Result<LiveStatus> {
    blocking(app, move |app| {
        let sets = project_sets(&app.state::<AppState>(), project_id)?;
        let live = app.state::<Live>();
        let open = live_window::current_set();
        let script = live.script_version();
        Ok(LiveStatus {
            script,
            expected_script: live::SCRIPT_VERSION,
            is_open: open.as_ref().map(|o| live::is_open(o, &sets)),
            open_set: open.map(|o| o.set),
            transport: script.and_then(|_| live.transport().ok()),
        })
    })
    .await
}

/// Temps laissé à Live pour ouvrir le set (il peut d'abord demander s'il faut
/// enregistrer le set en cours, ou devoir démarrer).
const OPEN_TIMEOUT: Duration = Duration::from_secs(180);

/// Envoie Live à une position (en beats). Si un autre set est ouvert, ou si
/// Live est fermé, ouvre d'abord le projet et attend qu'il soit chargé.
/// Renvoie vrai si le set a dû être ouvert.
#[tauri::command]
pub async fn live_jump(app: AppHandle, project_id: i64, beat: f64, play: bool) -> Result<bool> {
    blocking(app, move |app| {
        let sets = project_sets(&app.state::<AppState>(), project_id)?;
        let live = app.state::<Live>();
        let ready = || {
            live.script_version().is_some()
                // Titre illisible : on fait confiance au script.
                && live_window::current_set().is_none_or(|o| live::is_open(&o, &sets))
        };

        let opened = !ready();
        if opened {
            app.opener()
                .open_path(sets[0].to_string_lossy(), None::<&str>)
                .map_err(|e| Error::Other(e.to_string()))?;
            let started = Instant::now();
            while !ready() {
                if started.elapsed() > OPEN_TIMEOUT {
                    return Err(Error::Other("Live n'a pas ouvert le set à temps".into()));
                }
                std::thread::sleep(Duration::from_millis(500));
            }
            // Le script redémarre avec chaque set : on laisse Live finir de charger.
            std::thread::sleep(Duration::from_secs(1));
        }
        live.jump(beat, play)?;
        Ok(opened)
    })
    .await
}

/// Copie le Remote Script dans la User Library de Live. Sans dossier donné,
/// prend celle des Documents ; renvoie `None` si elle n'y est pas.
#[tauri::command]
pub fn install_remote_script(
    app: AppHandle,
    user_library: Option<String>,
) -> Result<Option<String>> {
    let dir = match user_library {
        Some(chosen) => {
            let chosen = PathBuf::from(chosen);
            if chosen.file_name().is_some_and(|n| n == "Remote Scripts") {
                chosen
            } else if chosen.join("User Library").is_dir() {
                chosen.join("User Library").join("Remote Scripts")
            } else {
                chosen.join("Remote Scripts")
            }
        }
        None => {
            let docs = app
                .path()
                .document_dir()
                .map_err(|e| Error::Other(e.to_string()))?;
            let library = docs.join("Ableton").join("User Library");
            if !library.is_dir() {
                return Ok(None);
            }
            library.join("Remote Scripts")
        }
    }
    .join("AbletonManager");

    let write = || -> std::io::Result<()> {
        std::fs::create_dir_all(&dir)?;
        for (name, content) in live::SCRIPT_FILES {
            std::fs::write(dir.join(name), content)?;
        }
        Ok(())
    };
    write().map_err(|e| Error::Other(format!("installation du script : {e}")))?;
    Ok(Some(dir.to_string_lossy().into_owned()))
}

// ---------- Notes dans l'arbre des versions ----------

#[tauri::command]
pub fn list_tree_notes(state: State<AppState>, project_id: i64) -> Result<Vec<notes::TreeNote>> {
    Ok(notes::list(&state.db.lock().unwrap(), project_id)?)
}

#[tauri::command]
pub fn add_tree_note(
    state: State<AppState>,
    project_id: i64,
    version_id: Option<i64>,
    x: f64,
    y: f64,
    text: String,
) -> Result<notes::TreeNote> {
    Ok(notes::add(
        &state.db.lock().unwrap(),
        project_id,
        version_id,
        x,
        y,
        &text,
        now(),
    )?)
}

#[tauri::command]
pub fn update_tree_note(
    state: State<AppState>,
    id: i64,
    version_id: Option<i64>,
    x: f64,
    y: f64,
    text: String,
) -> Result<()> {
    Ok(notes::update(
        &state.db.lock().unwrap(),
        id,
        version_id,
        x,
        y,
        &text,
    )?)
}

#[tauri::command]
pub fn delete_tree_note(state: State<AppState>, id: i64) -> Result<()> {
    Ok(notes::delete(&state.db.lock().unwrap(), id)?)
}

// ---------- Fichiers (explorateur) ----------

#[tauri::command]
pub fn set_hidden(state: State<AppState>, path: String, hidden: bool) -> Result<()> {
    Ok(crate::files::set_hidden(
        &state.db.lock().unwrap(),
        &path,
        hidden,
    )?)
}

#[tauri::command]
pub fn rename_path(
    app: AppHandle,
    state: State<AppState>,
    path: String,
    name: String,
) -> Result<String> {
    let new = crate::files::rename(&state.db.lock().unwrap(), &path, &name)?;
    app.state::<Arc<SaveWatcher>>().sync(&app);
    Ok(new)
}

#[tauri::command]
pub fn move_path(
    app: AppHandle,
    state: State<AppState>,
    path: String,
    dest_dir: String,
) -> Result<String> {
    let new = crate::files::move_into(&state.db.lock().unwrap(), &path, &dest_dir)?;
    app.state::<Arc<SaveWatcher>>().sync(&app);
    Ok(new)
}

#[tauri::command]
pub fn delete_path(app: AppHandle, state: State<AppState>, path: String) -> Result<usize> {
    let n = crate::files::delete(&state.db.lock().unwrap(), &path)?;
    app.state::<Arc<SaveWatcher>>().sync(&app);
    Ok(n)
}

#[tauri::command]
pub fn create_dir(parent: String, name: String) -> Result<String> {
    Ok(crate::files::create_dir(&parent, &name)?)
}

#[tauri::command]
pub async fn delete_branch(
    app: AppHandle,
    watcher: State<'_, Arc<SaveWatcher>>,
    branch_id: i64,
) -> Result<u64> {
    let freed = in_background(app.clone(), move |c, v| {
        versions::delete_branch(c, v, branch_id)
    })
    .await?;
    watcher.sync(&app);
    Ok(freed)
}

/// Ajoute le device de capture au set ouvert dans Live (sur le Master).
#[tauri::command]
pub async fn live_add_capture(app: AppHandle) -> Result<()> {
    blocking(app, move |app| {
        app.state::<Live>()
            .request("/manager/load_capture", &[], Duration::from_secs(5))?;
        Ok(())
    })
    .await
}

/// Projet dont le set est ouvert dans Live (d'après le titre de sa fenêtre),
/// branches comprises. Le plus récemment modifié gagne en cas d'ambiguïté.
#[tauri::command]
pub fn open_project_id(state: State<AppState>) -> Result<Option<i64>> {
    let Some(open) = live_window::current_set() else { return Ok(None) };
    let conn = state.db.lock().unwrap();
    let mut stmt = conn.prepare(
        "SELECT p.id, s.als_path FROM projects p
         JOIN (SELECT id AS pid, als_path FROM projects
               UNION ALL SELECT project_id, als_path FROM branches) s ON s.pid = p.id
         ORDER BY p.modified_at DESC",
    )?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows.into_iter().find(|(_, als)| live::is_open(&open, &[PathBuf::from(als)])).map(|(id, _)| id))
}

// ---------- Collections ----------

#[tauri::command]
pub fn list_collections(state: State<AppState>) -> Result<Vec<db::Collection>> {
    Ok(db::list_collections(&state.db.lock().unwrap())?)
}

#[tauri::command]
pub fn save_collections(state: State<AppState>, collections: Vec<db::Collection>) -> Result<()> {
    Ok(db::save_collections(&state.db.lock().unwrap(), &collections)?)
}

// ---------- Temps passé ----------

#[tauri::command]
pub fn project_time(state: State<AppState>, project_id: i64) -> Result<worktime::ProjectTime> {
    Ok(worktime::project_time(&state.db.lock().unwrap(), project_id, now())?)
}

#[tauri::command]
pub fn delete_work_session(app: AppHandle, state: State<AppState>, id: i64) -> Result<()> {
    if let Some(project_id) = worktime::delete_session(&state.db.lock().unwrap(), id)? {
        worktime::notify(&app, project_id);
    }
    Ok(())
}

#[tauri::command]
pub fn worktime_overview(state: State<AppState>) -> Result<worktime::Overview> {
    Ok(worktime::overview(&state.db.lock().unwrap(), now())?)
}

#[tauri::command]
pub fn worktime_enabled(state: State<AppState>) -> Result<bool> {
    Ok(worktime::enabled(&state.db.lock().unwrap())?)
}

#[tauri::command]
pub fn set_worktime_enabled(state: State<AppState>, enabled: bool) -> Result<()> {
    Ok(worktime::set_enabled(&state.db.lock().unwrap(), enabled)?)
}
