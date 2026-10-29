mod audio;
mod capture;
mod commands;
mod db;
mod live_window;
mod previews;
mod scanner;
mod vault;
mod versions;
mod watcher;

use std::path::PathBuf;
use std::sync::Mutex;

use tauri::Manager;

/// Connexion SQLite partagée par toutes les commandes.
pub struct AppState {
    pub db: Mutex<rusqlite::Connection>,
    pub db_path: PathBuf,
    pub data_dir: PathBuf,
}

/// Réglage : dossier du coffre (par défaut dans les données de l'appli).
pub const VAULT_DIR: &str = "vault_dir";

impl AppState {
    /// Connexion à part pour les travaux longs (coffre, versions), qui ne
    /// doivent pas bloquer l'interface.
    pub fn open_db(&self) -> rusqlite::Result<rusqlite::Connection> {
        db::open(&self.db_path)
    }

    pub fn vault_dir(&self, conn: &rusqlite::Connection) -> rusqlite::Result<PathBuf> {
        Ok(db::get_setting(conn, VAULT_DIR)?
            .map(PathBuf::from)
            .unwrap_or_else(|| self.data_dir.join("vault")))
    }

    pub fn vault(&self, conn: &rusqlite::Connection) -> rusqlite::Result<vault::Vault> {
        Ok(vault::Vault::new(self.vault_dir(conn)?))
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir)?;
            let db_path = dir.join("library.db");
            let conn = db::open(&db_path)?;
            app.manage(AppState { db: Mutex::new(conn), db_path, data_dir: dir.clone() });
            app.manage(watcher::SaveWatcher::start(app.handle().clone()));
            app.manage(capture::CaptureLink::start(app.handle().clone(), dir.join("captures")));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_roots,
            commands::add_root,
            commands::remove_root,
            commands::scan,
            commands::search_projects,
            commands::get_project,
            commands::all_tags,
            commands::set_status,
            commands::set_favorite,
            commands::set_note,
            commands::add_tag,
            commands::remove_tag,
            commands::open_in_ableton,
            commands::open_folder,
            commands::get_versions,
            commands::enable_versions,
            commands::snapshot_now,
            commands::rename_version,
            commands::restore_version,
            commands::create_branch,
            commands::delete_version,
            commands::compare_versions,
            commands::open_branch,
            commands::open_branch_folder,
            commands::vault_info,
            commands::set_vault_dir,
            commands::get_preview,
            commands::preview_audio,
            commands::find_bounce,
            commands::import_preview,
            commands::capture_status,
            commands::render_preview,
            commands::cancel_render,
            commands::install_device,
        ])
        .run(tauri::generate_context!())
        .expect("erreur au lancement de l'application");
}
