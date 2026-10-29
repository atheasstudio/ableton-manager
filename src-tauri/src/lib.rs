mod annotations;
mod audio;
mod capture;
mod commands;
mod db;
mod doctor;
mod files;
mod live;
mod live_window;
mod notes;
mod organize;
mod previews;
mod scanner;
mod tray;
mod vault;
mod versions;
mod watcher;
mod worktime;

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
        // En premier : une deuxième instance (raccourci, démarrage) réaffiche
        // simplement la fenêtre de la première, cachée dans la zone de notification.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| tray::show(app)))
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec![tray::HIDDEN_ARG]),
        ))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir)?;
            let db_path = dir.join("library.db");
            let conn = db::open(&db_path)?;
            app.manage(AppState {
                db: Mutex::new(conn),
                db_path,
                data_dir: dir.clone(),
            });
            app.manage(watcher::SaveWatcher::start(app.handle().clone()));
            app.manage(capture::CaptureLink::start(
                app.handle().clone(),
                dir.join("captures"),
            ));
            app.manage(live::Live::new());
            worktime::start(app.handle().clone());
            tray::build(app.handle())?;
            if tray::started_hidden() {
                if let Some(w) = app.get_webview_window("main") {
                    w.hide()?;
                }
            }
            Ok(())
        })
        .on_window_event(tray::on_window_event)
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
            commands::move_to_own_folder,
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
            commands::get_timeline,
            commands::add_annotation,
            commands::update_annotation,
            commands::delete_annotation,
            commands::import_locators,
            commands::live_status,
            commands::live_jump,
            commands::install_remote_script,
            commands::list_tree_notes,
            commands::add_tree_note,
            commands::update_tree_note,
            commands::delete_tree_note,
            commands::set_hidden,
            commands::rename_path,
            commands::move_path,
            commands::delete_path,
            commands::create_dir,
            commands::delete_branch,
            commands::live_add_capture,
            commands::open_project_id,
            commands::project_time,
            commands::delete_work_session,
            commands::worktime_overview,
            commands::worktime_enabled,
            commands::set_worktime_enabled,
            commands::list_collections,
            commands::save_collections,
            commands::doctor_missing_samples,
            commands::doctor_find_candidates,
            commands::doctor_relink,
            commands::doctor_collect_samples,
            commands::doctor_plugins,
            commands::close_to_tray,
            commands::set_close_to_tray,
            commands::autostart_enabled,
            commands::set_autostart_enabled,
        ])
        .run(tauri::generate_context!())
        .expect("erreur au lancement de l'application");
}
