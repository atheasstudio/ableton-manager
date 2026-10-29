//! Icône dans la zone de notification : menu Ouvrir / Scanner / Quitter, la
//! croix qui y réduit la fenêtre, et le lancement au démarrage de Windows.

use std::sync::atomic::{AtomicBool, Ordering};

use serde::Serialize;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, Window, WindowEvent};

use crate::{commands, db, AppState};

/// Réglage : "0" fait quitter l'appli à la croix au lieu de la réduire.
pub const CLOSE_TO_TRAY: &str = "close_to_tray";
/// Argument passé par le lancement au démarrage : la fenêtre reste cachée.
pub const HIDDEN_ARG: &str = "--hidden";

static SCANNING: AtomicBool = AtomicBool::new(false);

pub fn close_to_tray(conn: &rusqlite::Connection) -> db::Result<bool> {
    Ok(db::get_setting(conn, CLOSE_TO_TRAY)?.as_deref() != Some("0"))
}

pub fn set_close_to_tray(conn: &rusqlite::Connection, on: bool) -> db::Result<()> {
    db::set_setting(conn, CLOSE_TO_TRAY, if on { "1" } else { "0" })
}

/// Lancé avec `--hidden` (démarrage de Windows) ?
pub fn started_hidden() -> bool {
    std::env::args().any(|a| a == HIDDEN_ARG)
}

/// Remet la fenêtre principale au premier plan.
pub fn show(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

pub fn build(app: &AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Ouvrir", true, None::<&str>)?;
    let scan = MenuItem::with_id(app, "scan", "Scanner", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quitter", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &scan, &quit])?;
    let mut tray = TrayIconBuilder::with_id("main")
        .tooltip("Ableton Manager")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, e| match e.id().as_ref() {
            "open" => show(app),
            "scan" => scan_in_background(app.clone()),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, e| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
                show(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

/// Fin d'un scan lancé depuis l'icône : l'interface l'annonce (`tray-scan-done`).
#[derive(Clone, Serialize)]
struct TrayScanDone {
    message: String,
}

fn scan_in_background(app: AppHandle) {
    if SCANNING.swap(true, Ordering::SeqCst) {
        return; // déjà en cours
    }
    std::thread::spawn(move || {
        let message = match commands::scan_blocking(&app) {
            Ok(r) => {
                let mut parts = vec![format!("{} projets trouvés", r.found), format!("{} mis à jour", r.updated)];
                if r.removed > 0 {
                    parts.push(format!("{} disparus", r.removed));
                }
                if !r.errors.is_empty() {
                    parts.push(format!("{} illisibles", r.errors.len()));
                }
                format!("Scan terminé : {}", parts.join(" · "))
            }
            Err(e) => format!("Scan : {e}"),
        };
        SCANNING.store(false, Ordering::SeqCst);
        let _ = app.emit("tray-scan-done", TrayScanDone { message });
        // L'explorateur recharge sa liste sur cet évènement.
        let _ = app.emit("preview-changed", ());
    });
}

/// La croix de la fenêtre principale la cache au lieu de quitter, si le
/// réglage le demande.
pub fn on_window_event(window: &Window, event: &WindowEvent) {
    let WindowEvent::CloseRequested { api, .. } = event else { return };
    if window.label() != "main" {
        return;
    }
    let state = window.state::<AppState>();
    let hide = close_to_tray(&state.db.lock().unwrap()).unwrap_or(true);
    if hide {
        api.prevent_close();
        let _ = window.hide();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn close_to_tray_by_default() {
        let conn = db::open_in_memory().unwrap();
        assert!(close_to_tray(&conn).unwrap());
        set_close_to_tray(&conn, false).unwrap();
        assert!(!close_to_tray(&conn).unwrap());
    }
}
