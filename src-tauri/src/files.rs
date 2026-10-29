//! Gestion des fichiers depuis l'explorateur : masquer, renommer, déplacer,
//! supprimer (vers la corbeille) des dossiers ou des .als.

use std::fs;
use std::path::Path;

use rusqlite::{params, Connection};

use crate::versions::{Error, Result};

fn other<T>(msg: impl Into<String>) -> Result<T> {
    Err(Error::Other(msg.into()))
}

pub fn set_hidden(conn: &Connection, path: &str, hidden: bool) -> Result<()> {
    if hidden {
        conn.execute("INSERT OR IGNORE INTO hidden (path) VALUES (?1)", [path])?;
    } else {
        conn.execute("DELETE FROM hidden WHERE path = ?1", [path])?;
    }
    Ok(())
}

/// Remplace le préfixe `old` par `new` dans tous les chemins connus.
fn repath(conn: &Connection, old: &str, new: &str) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    for (table, col) in [
        ("projects", "folder"),
        ("projects", "als_path"),
        ("branches", "folder"),
        ("branches", "als_path"),
        ("captures", "als_path"),
        ("hidden", "path"),
    ] {
        tx.execute(
            &format!(
                "UPDATE {table} SET {col} = ?2 || substr({col}, length(?1) + 1)
                 WHERE {col} = ?1 OR substr({col}, 1, length(?1) + 1) IN (?1 || '\\', ?1 || '/')"
            ),
            params![old, new],
        )?;
    }
    tx.commit()?;
    Ok(())
}

/// Renomme un dossier (ou un .als) sur place. Renvoie le nouveau chemin.
pub fn rename(conn: &Connection, path: &str, name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() || name.contains(['\\', '/', ':', '*', '?', '"', '<', '>', '|']) {
        return other("nom invalide");
    }
    let src = Path::new(path);
    let mut name = name.to_owned();
    if src.is_file() && !name.to_lowercase().ends_with(".als") {
        name.push_str(".als");
    }
    let dest = src.with_file_name(&name);
    if dest.exists() {
        return other(format!("{} existe déjà", dest.display()));
    }
    fs::rename(src, &dest)?;
    let dest = dest.to_string_lossy().into_owned();
    repath(conn, path, &dest)?;
    if src
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("als"))
    {
        let stem = Path::new(&dest)
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        conn.execute(
            "UPDATE projects SET name = ?2 WHERE als_path = ?1",
            params![dest, stem],
        )?;
    }
    Ok(dest)
}

/// Déplace un dossier ou un .als dans `dest_dir`. Renvoie le nouveau chemin.
pub fn move_into(conn: &Connection, path: &str, dest_dir: &str) -> Result<String> {
    let src = Path::new(path);
    let dest = Path::new(dest_dir).join(src.file_name().unwrap_or_default());
    if dest.exists() {
        return other(format!("{} existe déjà", dest.display()));
    }
    if Path::new(dest_dir).starts_with(src) {
        return other("impossible de déplacer un dossier dans lui-même");
    }
    if fs::rename(src, &dest).is_err() {
        // Autre disque : copie puis corbeille.
        copy_all(src, &dest)?;
        trash::delete(src).map_err(|e| Error::Other(e.to_string()))?;
    }
    let dest = dest.to_string_lossy().into_owned();
    repath(conn, path, &dest)?;
    if src.is_file() || dest.to_lowercase().ends_with(".als") {
        // Un .als déplacé seul change de dossier de projet.
        conn.execute(
            "UPDATE projects SET folder = ?2 WHERE als_path = ?1",
            params![dest, dest_dir],
        )?;
    }
    Ok(dest)
}

fn copy_all(src: &Path, dest: &Path) -> std::io::Result<()> {
    if src.is_file() {
        fs::copy(src, dest)?;
        return Ok(());
    }
    fs::create_dir_all(dest)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        copy_all(&entry.path(), &dest.join(entry.file_name()))?;
    }
    Ok(())
}

/// Envoie un dossier ou un .als à la corbeille et oublie les projets dedans,
/// sauf les projets versionnés : leur historique est dans le coffre et doit
/// survivre à une sortie de la corbeille (comme pour un disque débranché).
pub fn delete(conn: &Connection, path: &str) -> Result<usize> {
    trash::delete(path).map_err(|e| Error::Other(format!("corbeille : {e}")))?;
    forget(conn, path)
}

fn forget(conn: &Connection, path: &str) -> Result<usize> {
    Ok(conn.execute(
        "DELETE FROM projects WHERE (als_path = ?1
            OR substr(als_path, 1, length(?1) + 1) IN (?1 || '\\', ?1 || '/'))
            AND id NOT IN (SELECT project_id FROM branches)",
        [path],
    )?)
}

pub fn create_dir(parent: &str, name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() || name.contains(['\\', '/', ':', '*', '?', '"', '<', '>', '|']) {
        return other("nom invalide");
    }
    let dir = Path::new(parent).join(name);
    fs::create_dir(&dir)?;
    Ok(dir.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rename_folder_updates_paths() {
        let tmp = tempfile::tempdir().unwrap();
        let old = tmp.path().join("A Project");
        fs::create_dir(&old).unwrap();
        fs::write(old.join("A.als"), b"x").unwrap();
        let conn = crate::db::open_in_memory().unwrap();
        let (o, als) = (
            old.to_string_lossy().into_owned(),
            old.join("A.als").to_string_lossy().into_owned(),
        );
        conn.execute(
            "INSERT INTO projects (folder, als_path, name, modified_at, scanned_at) VALUES (?1, ?2, 'A', 0, 0)",
            params![o, als],
        )
        .unwrap();
        let new = rename(&conn, &o, "B Project").unwrap();
        let (f, a): (String, String) = conn
            .query_row("SELECT folder, als_path FROM projects", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!(f, new);
        assert_eq!(a, Path::new(&new).join("A.als").to_string_lossy());
        assert!(Path::new(&a).is_file());
    }

    #[test]
    fn delete_keeps_versioned_projects() {
        let tmp = tempfile::tempdir().unwrap();
        let conn = crate::db::open_in_memory().unwrap();
        let mut ids = vec![];
        for name in ["A", "B"] {
            let folder = tmp.path().join(format!("{name} Project"));
            fs::create_dir(&folder).unwrap();
            let als = folder.join(format!("{name}.als"));
            fs::write(&als, b"x").unwrap();
            conn.execute(
                "INSERT INTO projects (folder, als_path, name, modified_at, scanned_at) VALUES (?1, ?2, ?3, 0, 0)",
                params![folder.to_string_lossy(), als.to_string_lossy(), name],
            )
            .unwrap();
            ids.push((conn.last_insert_rowid(), folder.to_string_lossy().into_owned()));
        }
        // B est versionné.
        conn.execute(
            "INSERT INTO branches (project_id, name, folder, als_path, is_main, created_at)
             VALUES (?1, 'principale', ?2, ?2, 1, 0)",
            params![ids[1].0, ids[1].1],
        )
        .unwrap();

        // `forget` : la partie base de `delete`, sans remplir la vraie corbeille.
        assert_eq!(forget(&conn, &ids[0].1).unwrap(), 1);
        assert_eq!(forget(&conn, &ids[1].1).unwrap(), 0, "versions gardées");
        let left: Vec<i64> = conn
            .prepare("SELECT id FROM projects")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(left, [ids[1].0]);
    }
}
