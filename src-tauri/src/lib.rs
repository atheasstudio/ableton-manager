mod commands;
mod db;
mod scanner;

use std::sync::Mutex;

use tauri::Manager;

/// Connexion SQLite partagée par toutes les commandes.
pub struct AppState {
    pub db: Mutex<rusqlite::Connection>,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir)?;
            let conn = db::open(&dir.join("library.db"))?;
            app.manage(AppState { db: Mutex::new(conn) });
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
        ])
        .run(tauri::generate_context!())
        .expect("erreur au lancement de l'application");
}
