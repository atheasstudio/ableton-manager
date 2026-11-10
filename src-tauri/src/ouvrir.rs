//! `ableton-manager.exe --ouvrir am:projet/42` (contrat commun §1, v1.1) :
//! l'appli (déjà ouverte ou qui démarre) passe au premier plan et sélectionne
//! le projet de la ref. Ref inconnue ou projet absent : l'appli s'ouvre simplement.

use std::sync::Mutex;

use rusqlite::{Connection, OptionalExtension};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::{tray, AppState};

pub const ARG: &str = "--ouvrir";
/// Évènement vers l'interface : sélectionner ce projet.
pub const EVENT: &str = "am:ouvrir-projet";

/// Ref d'Ableton Manager comprise par `--ouvrir`.
#[derive(Debug, PartialEq, Eq)]
pub enum RefAm {
    Projet(i64),
    Tache(i64),
    Version(i64),
}

/// Décode `am:projet/42`, `am:tache/7`, `am:version/88`.
pub fn decoder(r: &str) -> Option<RefAm> {
    let (genre, id) = r.trim().strip_prefix("am:")?.split_once('/')?;
    let id: i64 = id.parse().ok().filter(|&n| n > 0)?;
    match genre {
        "projet" => Some(RefAm::Projet(id)),
        "tache" => Some(RefAm::Tache(id)),
        "version" => Some(RefAm::Version(id)),
        _ => None,
    }
}

/// Valeur de `--ouvrir <ref>` (ou `--ouvrir=<ref>`) dans la ligne de commande.
pub fn ref_dans_args<S: AsRef<str>>(args: &[S]) -> Option<String> {
    let mut it = args.iter().map(AsRef::as_ref);
    while let Some(a) = it.next() {
        if a == ARG {
            return it.next().map(str::to_string);
        }
        if let Some(v) = a.strip_prefix("--ouvrir=") {
            return Some(v.to_string());
        }
    }
    None
}

/// Projet existant que désigne la ref.
pub fn projet_de(conn: &Connection, r: &RefAm) -> rusqlite::Result<Option<i64>> {
    let q = match r {
        RefAm::Projet(_) => "SELECT id FROM projects WHERE id = ?1",
        RefAm::Tache(_) => "SELECT project_id FROM todos WHERE id = ?1",
        RefAm::Version(_) => "SELECT project_id FROM versions WHERE id = ?1",
    };
    let (RefAm::Projet(id) | RefAm::Tache(id) | RefAm::Version(id)) = r;
    conn.query_row(q, [id], |row| row.get(0)).optional()
}

/// Projet demandé au lancement, en attente que l'interface le prenne.
#[derive(Default)]
pub struct EnAttente(pub Mutex<Option<i64>>);

#[derive(Clone, Serialize)]
struct OuvrirProjet {
    project_id: i64,
}

fn projet_des_args(app: &AppHandle, args: &[String]) -> Option<i64> {
    let r = decoder(&ref_dans_args(args)?)?;
    let state = app.try_state::<AppState>()?;
    let conn = state.db.lock().ok()?;
    projet_de(&conn, &r).ok().flatten()
}

/// Démarrage : le projet est gardé pour l'interface, qui n'écoute pas encore.
pub fn au_demarrage(app: &AppHandle) {
    let args: Vec<String> = std::env::args().collect();
    if let Some(id) = projet_des_args(app, &args) {
        if let Some(att) = app.try_state::<EnAttente>() {
            *att.0.lock().unwrap() = Some(id);
        }
    }
}

/// Deuxième lancement : l'instance déjà ouverte passe devant et va au projet.
pub fn seconde_instance(app: &AppHandle, args: &[String]) {
    tray::show(app);
    if let Some(id) = projet_des_args(app, args) {
        let _ = app.emit(EVENT, OuvrirProjet { project_id: id });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_les_refs() {
        assert_eq!(decoder("am:projet/42"), Some(RefAm::Projet(42)));
        assert_eq!(decoder(" am:tache/7 "), Some(RefAm::Tache(7)));
        assert_eq!(decoder("am:version/88"), Some(RefAm::Version(88)));
        for mauvaise in ["lvl:joueur", "am:projet/", "am:projet/x", "am:projet/-3", "am:souhait/1", "projet/1", ""] {
            assert_eq!(decoder(mauvaise), None, "{mauvaise}");
        }
    }

    #[test]
    fn trouve_l_argument() {
        assert_eq!(ref_dans_args(&["exe", "--ouvrir", "am:projet/42"]).as_deref(), Some("am:projet/42"));
        assert_eq!(ref_dans_args(&["exe", "--hidden", "--ouvrir=am:tache/7"]).as_deref(), Some("am:tache/7"));
        assert_eq!(ref_dans_args(&["exe", "--ouvrir"]), None);
        assert_eq!(ref_dans_args(&["exe"]), None);
    }

    #[test]
    fn ref_vers_projet() {
        let c = crate::db::open_in_memory().unwrap();
        c.execute_batch(
            "INSERT INTO projects (id, folder, als_path, name, modified_at, scanned_at) VALUES (3, '.', 'a.als', 'A', 0, 0);
             INSERT INTO todos (id, project_id, text, created_at) VALUES (7, 3, 'x', 0);
             INSERT INTO branches (id, project_id, name, folder, als_path, created_at) VALUES (1, 3, 'b', '.', 'a.als', 0);
             INSERT INTO versions (id, project_id, branch_id, created_at, source, auto_name, als_name, als_hash, summary)
               VALUES (88, 3, 1, 0, 'save', 'v', 'a.als', 'h', '{}');",
        )
        .unwrap();
        assert_eq!(projet_de(&c, &RefAm::Projet(3)).unwrap(), Some(3));
        assert_eq!(projet_de(&c, &RefAm::Tache(7)).unwrap(), Some(3));
        assert_eq!(projet_de(&c, &RefAm::Version(88)).unwrap(), Some(3));
        assert_eq!(projet_de(&c, &RefAm::Projet(42)).unwrap(), None);
    }
}
