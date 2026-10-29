//! Recherche des projets Ableton sur le disque.
//!
//! Un projet = un dossier qui contient au moins un `.als`. Le `.als` le plus
//! récent du dossier est le set principal ; les autres sont listés à côté.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use walkdir::WalkDir;

/// Dossiers jamais parcourus : sauvegardes automatiques de Live, dossiers
/// système de Windows, et contenus de bibliothèques qui ne sont pas des projets.
const SKIPPED_DIRS: &[&str] = &[
    "backup",
    "$recycle.bin",
    "system volume information",
    "windows",
    "program files",
    "program files (x86)",
    "programdata",
    "appdata",
    "node_modules",
    "ableton project info",
];

#[derive(Debug)]
pub struct FoundProject {
    pub folder: PathBuf,
    pub main_set: PathBuf,
    pub modified_at: i64,
    pub other_sets: Vec<PathBuf>,
}

impl FoundProject {
    /// Nom affiché : celui du dossier sans le suffixe « Project » ajouté par Live.
    pub fn name(&self) -> String {
        let folder = self.folder.file_name().map(|n| n.to_string_lossy().into_owned());
        let set = self.main_set.file_stem().map(|n| n.to_string_lossy().into_owned());
        match folder {
            Some(f) if f.ends_with(" Project") => f.trim_end_matches(" Project").to_owned(),
            _ => set.unwrap_or_default(),
        }
    }
}

fn skipped(entry: &walkdir::DirEntry) -> bool {
    if entry.depth() == 0 || !entry.file_type().is_dir() {
        return false;
    }
    let name = entry.file_name().to_string_lossy().to_lowercase();
    name.starts_with('.') || SKIPPED_DIRS.contains(&name.as_str())
}

fn mtime(path: &Path) -> i64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs() as i64)
}

/// Parcourt `root` et regroupe les `.als` par dossier. Les dossiers illisibles
/// (droits, disque débranché) sont ignorés sans arrêter le scan.
pub fn find_projects(root: &Path) -> Vec<FoundProject> {
    let mut by_folder: BTreeMap<PathBuf, Vec<(PathBuf, i64)>> = BTreeMap::new();
    let walker = WalkDir::new(root).into_iter().filter_entry(|e| !skipped(e));
    for entry in walker.flatten() {
        let path = entry.path();
        let is_als = entry.file_type().is_file()
            && path.extension().is_some_and(|e| e.eq_ignore_ascii_case("als"));
        if is_als {
            if let Some(folder) = path.parent() {
                by_folder.entry(folder.to_owned()).or_default().push((path.to_owned(), mtime(path)));
            }
        }
    }

    by_folder
        .into_iter()
        .map(|(folder, mut sets)| {
            sets.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
            let (main_set, modified_at) = sets.remove(0);
            FoundProject {
                folder,
                main_set,
                modified_at,
                other_sets: sets.into_iter().map(|(p, _)| p).collect(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn groups_by_folder_and_skips_backups() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let a = root.join("Projets/1woodkid perc Project");
        fs::create_dir_all(a.join("Backup")).unwrap();
        fs::write(a.join("Sans titre.als"), b"").unwrap();
        fs::write(a.join("Backup/Sans titre [2025-09-01 114124].als"), b"").unwrap();
        let b = root.join("ailleurs/dsd Project");
        fs::create_dir_all(&b).unwrap();
        fs::write(b.join("sdf.als"), b"").unwrap();
        fs::write(b.join("2gdfggdfgdf.als"), b"").unwrap();
        fs::create_dir_all(root.join("$RECYCLE.BIN/x")).unwrap();
        fs::write(root.join("$RECYCLE.BIN/x/old.als"), b"").unwrap();

        let found = find_projects(root);
        let names: Vec<_> = found.iter().map(|p| p.name()).collect();
        assert_eq!(names, ["1woodkid perc", "dsd"]);
        assert!(found[0].other_sets.is_empty(), "les Backup sont ignorés");
        assert_eq!(found[1].other_sets.len(), 1);
    }
}
