//! Changements faits dans la base par un autre programme (la porte MCP) :
//! un fil lit `PRAGMA data_version` toutes les 2 s ; quand la base a bougé et
//! que `changements_externes` a une nouvelle ligne, il prévient l'interface.

use std::path::PathBuf;
use std::time::Duration;

use rusqlite::Connection;
use serde::Serialize;
use tauri::{AppHandle, Emitter};

/// Évènement Tauri : la bibliothèque a été changée hors de l'appli.
pub const EVENT: &str = "am:biblio-changee";
const TICK: Duration = Duration::from_secs(2);

#[derive(Clone, Serialize)]
struct BiblioChangee {
    /// Dernier id de `changements_externes`.
    dernier: i64,
}

/// État lu à un relevé : (data_version, dernier changement externe).
fn releve(conn: &Connection) -> rusqlite::Result<(i64, i64)> {
    let v = conn.query_row("PRAGMA data_version", [], |r| r.get(0))?;
    let d = conn.query_row("SELECT coalesce(max(id), 0) FROM changements_externes", [], |r| r.get(0))?;
    Ok((v, d))
}

/// Compare deux relevés : `Some(dernier)` s'il faut prévenir l'interface.
fn nouveau(avant: (i64, i64), apres: (i64, i64)) -> Option<i64> {
    (apres.0 != avant.0 && apres.1 > avant.1).then_some(apres.1)
}

pub fn start(app: AppHandle, db_path: PathBuf) {
    std::thread::spawn(move || {
        // Connexion à part : data_version ne change que pour les écritures
        // des autres connexions.
        let conn = match Connection::open(&db_path) {
            Ok(c) => c,
            Err(e) => return eprintln!("surveillance de la base : {e}"),
        };
        conn.busy_timeout(Duration::from_secs(5)).ok();
        let mut avant = match releve(&conn) {
            Ok(r) => r,
            Err(e) => return eprintln!("surveillance de la base : {e}"),
        };
        loop {
            std::thread::sleep(TICK);
            match releve(&conn) {
                Ok(apres) => {
                    if let Some(dernier) = nouveau(avant, apres) {
                        let _ = app.emit(EVENT, BiblioChangee { dernier });
                    }
                    avant = apres;
                }
                Err(e) => eprintln!("surveillance de la base : {e}"),
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn voit_une_ecriture_externe() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("library.db");
        let app = crate::db::open(&path).unwrap();
        app.execute("INSERT INTO projects (id, folder, als_path, name, modified_at, scanned_at) VALUES (1, '.', 'a.als', 'A', 0, 0)", []).unwrap();
        let guet = Connection::open(&path).unwrap();
        let r0 = releve(&guet).unwrap();

        // Écriture de l'appli elle-même (autre connexion) : pas d'évènement.
        biblio::add_tag(&app, 1, "club", biblio::SOURCE_APP).unwrap();
        let r1 = releve(&guet).unwrap();
        assert_ne!(r1.0, r0.0);
        assert_eq!(nouveau(r0, r1), None);

        // Écriture de la porte : évènement.
        let porte = Connection::open(&path).unwrap();
        biblio::set_status(&porte, 1, "fini", biblio::SOURCE_IA).unwrap();
        let r2 = releve(&guet).unwrap();
        assert_eq!(nouveau(r1, r2), Some(1));
        assert_eq!(nouveau(r2, releve(&guet).unwrap()), None);
    }
}
