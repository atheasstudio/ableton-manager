//! Rangement : sortir un set d'un dossier qu'il partage avec d'autres
//! morceaux pour lui donner son propre dossier de projet.

use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection};
use serde::Serialize;

use crate::db;
use crate::versions::{Error, Result};

#[derive(Debug, Serialize)]
pub struct Moved {
    /// Nouveau dossier du projet.
    pub folder: String,
    /// Audios du projet copiés dans le nouveau dossier.
    pub copied: usize,
}

/// Déplace le .als du projet `id` dans « <set> Project », à côté de son
/// dossier actuel. Les audios du dossier qu'il utilise sont copiés (d'autres
/// sets peuvent s'en servir) et ses sauvegardes Backup le suivent.
pub fn move_to_own_folder(conn: &Connection, id: i64) -> Result<Moved> {
    let (als, folder) = db::als_path(conn, id)?.ok_or_else(|| Error::Other("projet introuvable".into()))?;
    let (als, folder) = (PathBuf::from(als), PathBuf::from(folder));
    let stem = als.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let dest = folder.with_file_name(format!("{stem} Project"));
    if dest.exists() {
        return Err(Error::Other(format!("le dossier {} existe déjà", dest.display())));
    }
    let set = als_parser::parse_file(&als)?;

    fs::create_dir_all(&dest)?;
    let mut copied = 0;
    for sample in &set.samples {
        let source = match (&sample.relative_path, &sample.path) {
            (Some(rel), _) if sample.is_project_relative() => folder.join(rel),
            (_, Some(abs)) => PathBuf::from(abs),
            _ => continue,
        };
        let Ok(rel) = source.strip_prefix(&folder) else { continue };
        let target = dest.join(rel);
        if source.is_file() && !target.exists() {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(&source, &target)?;
            copied += 1;
        }
    }
    move_backups(&folder, &dest, &stem)?;
    let new_als = dest.join(als.file_name().unwrap_or_default());
    fs::rename(&als, &new_als)?;

    let (new_als, dest_str) = (new_als.to_string_lossy(), dest.to_string_lossy());
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "UPDATE projects SET folder = ?2, als_path = ?3, name = ?4 WHERE id = ?1",
        params![id, dest_str, new_als, stem],
    )?;
    // Le set n'est plus « un autre set du dossier » pour ses anciens voisins.
    tx.execute("DELETE FROM project_items WHERE kind = 'set' AND (project_id = ?1 OR path = ?2)", params![
        id,
        als.to_string_lossy()
    ])?;
    // Versions activées : la branche principale suit le set.
    tx.execute(
        "UPDATE branches SET folder = ?2, als_path = ?3 WHERE project_id = ?1 AND is_main = 1",
        params![id, dest_str, new_als],
    )?;
    tx.commit()?;
    Ok(Moved { folder: dest_str.into_owned(), copied })
}

/// Live range ses sauvegardes en « Backup/<set> [date].als ».
fn move_backups(folder: &Path, dest: &Path, stem: &str) -> Result<()> {
    let Ok(entries) = fs::read_dir(folder.join("Backup")) else { return Ok(()) };
    let prefix = format!("{stem} [");
    for entry in entries.flatten() {
        if entry.file_name().to_string_lossy().starts_with(&prefix) {
            fs::create_dir_all(dest.join("Backup"))?;
            fs::rename(entry.path(), dest.join("Backup").join(entry.file_name()))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn moves_set_with_its_samples_and_backups() {
        let dir = tempfile::tempdir().unwrap();
        let conn = db::open(&dir.path().join("library.db")).unwrap();
        let folder = dir.path().join("Projets").join("beats");
        let als = folder.join("woodkid.als");
        fs::create_dir_all(folder.join("Samples/Recorded")).unwrap();
        fs::create_dir_all(folder.join("Backup")).unwrap();
        fs::write(folder.join("Samples/Recorded/voix.wav"), b"voix").unwrap();
        fs::write(folder.join("Samples/Recorded/autre.wav"), b"autre").unwrap();
        fs::write(folder.join("Backup/woodkid [2025-09-01 114124].als"), b"").unwrap();
        fs::write(folder.join("Backup/autre [2025-09-01 114124].als"), b"").unwrap();
        fs::write(folder.join("autre.als"), b"").unwrap();
        fs::write(
            &als,
            r#"<?xml version="1.0" encoding="UTF-8"?>
<Ableton Creator="Ableton Live 12.2.5"><LiveSet><Tracks><AudioTrack><Name><EffectiveName Value="Voix" /></Name>
<AudioClip><SampleRef><FileRef><RelativePathType Value="3" /><RelativePath Value="Samples/Recorded/voix.wav" /><Path Value="C:/x/voix.wav" /></FileRef></SampleRef></AudioClip>
</AudioTrack></Tracks></LiveSet></Ableton>"#,
        )
        .unwrap();
        for (path, name) in [(&als, "woodkid"), (&folder.join("autre.als"), "autre")] {
            conn.execute(
                "INSERT INTO projects (folder, als_path, name, modified_at, scanned_at) VALUES (?1, ?2, ?3, 0, 0)",
                params![folder.to_string_lossy(), path.to_string_lossy(), name],
            )
            .unwrap();
        }

        let moved = move_to_own_folder(&conn, 1).unwrap();
        let dest = dir.path().join("Projets").join("woodkid Project");
        assert_eq!(moved.folder, dest.to_string_lossy());
        assert_eq!(moved.copied, 1);
        assert!(dest.join("woodkid.als").is_file() && !als.exists());
        assert!(dest.join("Samples/Recorded/voix.wav").is_file());
        assert!(folder.join("Samples/Recorded/voix.wav").is_file(), "copié, pas déplacé");
        assert!(!dest.join("Samples/Recorded/autre.wav").exists());
        assert!(dest.join("Backup/woodkid [2025-09-01 114124].als").is_file());
        assert!(folder.join("Backup/autre [2025-09-01 114124].als").is_file());
        let (path, f) = db::als_path(&conn, 1).unwrap().unwrap();
        assert_eq!((path, f), (dest.join("woodkid.als").to_string_lossy().into(), dest.to_string_lossy().into()));
        assert!(move_to_own_folder(&conn, 1).is_err(), "le dossier existe déjà");
    }
}
