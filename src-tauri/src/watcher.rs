//! Détection des sauvegardes : on surveille le dossier de chaque branche et,
//! quand Live a fini d'écrire, on crée une version si le .als a changé.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use tauri::{AppHandle, Emitter, Manager};

use crate::{previews, versions, AppState};

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

/// Live écrit le set en plusieurs fois : on attend ce délai sans activité.
const QUIET: Duration = Duration::from_secs(2);

pub struct SaveWatcher {
    watcher: Mutex<Option<RecommendedWatcher>>,
    watched: Mutex<HashSet<PathBuf>>,
}

#[derive(Clone, serde::Serialize)]
struct VersionsChanged {
    project_id: i64,
}

/// Comparaison de chemins insensible à la casse et aux séparateurs (Windows).
fn same_path(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| p.to_string_lossy().replace('/', "\\").to_lowercase();
    norm(a) == norm(b)
}

impl SaveWatcher {
    /// Lance la surveillance. Les sauvegardes faites pendant que l'appli était
    /// fermée sont rattrapées tout de suite.
    pub fn start(app: AppHandle) -> Arc<Self> {
        let (tx, rx) = channel::<PathBuf>();
        let watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if let Ok(event) = res {
                for path in event.paths {
                    let _ = tx.send(path);
                }
            }
        })
        .map_err(|e| eprintln!("surveillance des sauvegardes indisponible : {e}"))
        .ok();
        let this = Arc::new(Self { watcher: Mutex::new(watcher), watched: Mutex::default() });
        this.sync(&app);

        std::thread::spawn(move || {
            check(&app, None);
            // Dossier → dernier évènement reçu.
            let mut pending: HashMap<PathBuf, Instant> = HashMap::new();
            loop {
                match rx.recv_timeout(Duration::from_millis(500)) {
                    Ok(path) => {
                        if let Some(dir) = path.parent() {
                            pending.insert(dir.to_owned(), Instant::now());
                        }
                    }
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => break,
                }
                let ready: Vec<PathBuf> = pending
                    .iter()
                    .filter(|(_, at)| at.elapsed() >= QUIET)
                    .map(|(dir, _)| dir.clone())
                    .collect();
                for dir in ready {
                    pending.remove(&dir);
                    check(&app, Some(&dir));
                }
            }
        });
        this
    }

    /// Met la liste des dossiers surveillés à jour (après activation des
    /// versions ou création d'une branche).
    pub fn sync(&self, app: &AppHandle) {
        let state = app.state::<AppState>();
        let Ok(tracked) = state.open_db().map_err(|e| e.to_string()).and_then(|c| {
            versions::tracked(&c).map_err(|e| e.to_string())
        }) else {
            return;
        };
        let wanted: HashSet<PathBuf> = tracked
            .iter()
            .filter_map(|(_, als)| Path::new(als).parent().map(Path::to_owned))
            .collect();
        let mut guard = self.watcher.lock().unwrap();
        let Some(watcher) = guard.as_mut() else { return };
        let mut watched = self.watched.lock().unwrap();
        for dir in watched.difference(&wanted).cloned().collect::<Vec<_>>() {
            let _ = watcher.unwatch(&dir);
            watched.remove(&dir);
        }
        for dir in wanted {
            // Un disque débranché : on réessaiera à la prochaine synchronisation.
            if !watched.contains(&dir) && watcher.watch(&dir, RecursiveMode::NonRecursive).is_ok() {
                watched.insert(dir);
            }
        }
    }
}

/// Crée une version pour chaque branche dont le .als est dans `dir` (toutes
/// si `dir` est vide) et prévient l'interface.
fn check(app: &AppHandle, dir: Option<&Path>) {
    let state = app.state::<AppState>();
    let result = (|| -> Result<(), String> {
        let conn = state.open_db().map_err(|e| e.to_string())?;
        let vault = state.vault(&conn).map_err(|e| e.to_string())?;
        for (branch_id, als) in versions::tracked(&conn).map_err(|e| e.to_string())? {
            let als = PathBuf::from(als);
            if dir.is_some_and(|d| !als.parent().is_some_and(|p| same_path(p, d))) {
                continue;
            }
            match versions::snapshot(&conn, &vault, branch_id, "save") {
                Ok(Some(id)) => {
                    if let Err(e) = previews::on_new_version(&conn, &vault, id, now()) {
                        eprintln!("aperçu de la version : {e}");
                    }
                    let project_id = versions::branch(&conn, branch_id).map_err(|e| e.to_string())?.project_id;
                    let _ = app.emit("versions-changed", VersionsChanged { project_id });
                }
                Ok(None) => {}
                // Fichier en cours d'écriture : le prochain évènement relancera.
                Err(e) => eprintln!("{} : {e}", als.display()),
            }
        }
        Ok(())
    })();
    if let Err(e) = result {
        eprintln!("détection des sauvegardes : {e}");
    }
}
