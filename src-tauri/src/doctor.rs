//! Docteur de projets : samples introuvables (recherche du même fichier dans
//! les dossiers scannés, puis réécriture du .als) et inventaire des plugins.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::UNIX_EPOCH;

use als_parser::Relocation;
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::live_window::OpenSet;
use crate::vault::Vault;
use crate::{db, previews, versions};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("base de données : {0}")]
    Db(#[from] rusqlite::Error),
    #[error("fichier : {0}")]
    Io(#[from] std::io::Error),
    #[error("set : {0}")]
    Als(#[from] als_parser::Error),
    #[error("{0}")]
    Versions(#[from] versions::Error),
    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Nombre maximal de fichiers candidats proposés pour un sample.
const MAX_CANDIDATES: usize = 20;

#[derive(Debug, Serialize)]
pub struct MissingSample {
    /// Nom du fichier.
    pub name: String,
    /// Chemin absolu noté dans le set.
    pub path: Option<String>,
    pub rel_path: Option<String>,
    /// Sans chemin absolu dans le set (vieux formats), on ne sait pas le réécrire.
    pub relinkable: bool,
}

#[derive(Debug, Serialize)]
pub struct MissingProject {
    pub project_id: i64,
    pub name: String,
    pub folder: String,
    pub samples: Vec<MissingSample>,
}

/// Projets (non masqués) dont au moins un sample est introuvable, même test
/// que la fiche du projet. `progress(fait, total)` après chaque projet.
pub fn missing_samples(conn: &Connection, mut progress: impl FnMut(usize, usize)) -> Result<Vec<MissingProject>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT p.id, p.name, p.folder FROM projects p WHERE NOT {} ORDER BY p.name COLLATE NOCASE",
        db::HIDDEN_SQL
    ))?;
    let projects: Vec<(i64, String, String)> =
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<rusqlite::Result<_>>()?;
    let mut items = conn.prepare(
        "SELECT DISTINCT name, path, rel_path FROM project_items
         WHERE project_id = ?1 AND kind = 'sample' ORDER BY name",
    )?;
    let mut out = Vec::new();
    for (i, (id, name, folder)) in projects.iter().enumerate() {
        progress(i, projects.len());
        let samples: Vec<MissingSample> = items
            .query_map([id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, Option<String>>(2)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?
            .into_iter()
            .filter(|(_, path, rel)| !db::sample_found(Path::new(folder), path.as_deref(), rel.as_deref()))
            .map(|(name, path, rel_path)| MissingSample { relinkable: path.is_some(), name, path, rel_path })
            .collect();
        if !samples.is_empty() {
            out.push(MissingProject { project_id: *id, name: name.clone(), folder: folder.clone(), samples });
        }
    }
    progress(projects.len(), projects.len());
    Ok(out)
}

/// Cherche dans `roots` les fichiers qui portent l'un de ces noms (sans tenir
/// compte de la casse). Renvoie, pour chaque nom demandé, les chemins trouvés.
/// `progress(fichiers parcourus)` de temps en temps.
pub fn find_candidates(
    roots: &[PathBuf],
    names: &[String],
    mut progress: impl FnMut(usize),
) -> HashMap<String, Vec<String>> {
    let wanted: HashMap<String, &String> = names.iter().map(|n| (n.to_lowercase(), n)).collect();
    let mut found: HashMap<String, Vec<String>> = HashMap::new();
    let mut seen = HashSet::new();
    let mut count = 0;
    for root in roots {
        for entry in walkdir::WalkDir::new(root).into_iter().filter_map(|e| e.ok()) {
            count += 1;
            if count % 500 == 0 {
                progress(count);
            }
            if !entry.file_type().is_file() {
                continue;
            }
            let Some(name) = entry.file_name().to_str() else { continue };
            let Some(original) = wanted.get(&name.to_lowercase()) else { continue };
            let path = entry.path().to_string_lossy().replace('\\', "/");
            // Racines imbriquées : un fichier n'est compté qu'une fois.
            if !seen.insert(path.to_lowercase()) {
                continue;
            }
            let list = found.entry((*original).clone()).or_default();
            if list.len() < MAX_CANDIDATES {
                list.push(path);
            }
        }
    }
    progress(count);
    found
}

#[derive(Debug, Clone, Deserialize)]
pub struct Relink {
    /// Chemin absolu noté dans le set.
    pub old_path: String,
    /// Fichier choisi pour le remplacer.
    pub new_path: String,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct RelinkReport {
    pub relinked: usize,
    /// Copie de sécurité du .als, quand le projet n'est pas versionné.
    pub backup: Option<String>,
    /// Version créée avant la réécriture (projet versionné, set modifié depuis
    /// la dernière version).
    pub version_id: Option<i64>,
}

/// Chemin relatif de `target` depuis `base`, avec des `/` (« ../Samples/x.wav »).
/// Rien si les deux ne sont pas sur le même disque.
fn relative_to(base: &Path, target: &Path) -> Option<String> {
    let key = |c: &Component| c.as_os_str().to_string_lossy().to_lowercase();
    let b: Vec<Component> = base.components().collect();
    let t: Vec<Component> = target.components().collect();
    if b.first().map(key) != t.first().map(key) {
        return None;
    }
    let common = b.iter().zip(&t).take_while(|(x, y)| key(x) == key(y)).count();
    let mut parts: Vec<String> = vec!["..".into(); b.len() - common];
    parts.extend(t[common..].iter().map(|c| c.as_os_str().to_string_lossy().into_owned()));
    Some(parts.join("/"))
}

/// Premier nom libre parmi « x.als.bak », « x.als.bak2 »…
fn backup_path(als: &Path) -> PathBuf {
    let base = als.as_os_str().to_string_lossy().into_owned();
    (1..)
        .map(|i| PathBuf::from(if i == 1 { format!("{base}.bak") } else { format!("{base}.bak{i}") }))
        .find(|p| !p.exists())
        .expect("suite infinie")
}

fn mtime_secs(path: &Path) -> i64 {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs() as i64)
}

/// Fait pointer les samples du projet vers les fichiers choisis : sauvegarde
/// d'abord (version si le projet est versionné, sinon copie `.bak`), réécrit le
/// .als puis le relit pour la bibliothèque. `open` : set ouvert dans Live, qui
/// ne doit pas être celui-ci (Live écraserait la réécriture).
pub fn relink(
    conn: &mut Connection,
    vault: &Vault,
    project_id: i64,
    relinks: &[Relink],
    open: Option<&OpenSet>,
    now: i64,
) -> Result<RelinkReport> {
    let Some((als, folder)) = db::als_path(conn, project_id)? else {
        return Err(Error::Other("projet introuvable".into()));
    };
    if open.is_some_and(|o| previews::matches(&als, o)) {
        return Err(Error::Other("ferme d'abord ce set dans Live".into()));
    }
    let map: HashMap<String, Relocation> = relinks
        .iter()
        .filter(|r| Path::new(&r.new_path).is_file())
        .map(|r| {
            let path = r.new_path.replace('\\', "/");
            let relative_path = relative_to(Path::new(&folder), Path::new(&path)).unwrap_or_default();
            (r.old_path.clone(), Relocation { path, relative_path })
        })
        .collect();
    if map.is_empty() {
        return Err(Error::Other("aucun fichier de remplacement valable".into()));
    }

    let als_file = PathBuf::from(&als);
    let bytes = fs::read(&als_file)?;
    let new_bytes = als_parser::relocate_samples(&bytes, &map)?;
    // On vérifie que le résultat se relit avant de toucher au fichier.
    let set = als_parser::parse_bytes(&new_bytes)?;

    let main_branch: Option<i64> = conn
        .query_row(
            "SELECT id FROM branches WHERE project_id = ?1 AND is_main = 1",
            [project_id],
            |r| r.get(0),
        )
        .optional()?;
    let (mut backup, mut version_id) = (None, None);
    match main_branch {
        Some(branch) => {
            version_id = versions::snapshot(conn, vault, branch, "manual")?;
            if let Some(id) = version_id {
                if let Err(e) = previews::on_new_version(conn, vault, id, now) {
                    eprintln!("aperçu de la version : {e}");
                }
            }
        }
        None => {
            let bak = backup_path(&als_file);
            fs::copy(&als_file, &bak)?;
            backup = Some(bak.to_string_lossy().into_owned());
        }
    }

    let tmp = als_file.with_extension("als.tmp");
    fs::write(&tmp, &new_bytes)?;
    fs::rename(&tmp, &als_file)?;

    // Relecture pour la bibliothèque : les samples ne sont plus manquants.
    let (root_id, name): (Option<i64>, String) =
        conn.query_row("SELECT root_id, name FROM projects WHERE id = ?1", [project_id], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })?;
    let mut stmt = conn.prepare("SELECT path FROM project_items WHERE project_id = ?1 AND kind = 'set'")?;
    let others: Vec<String> = stmt
        .query_map([project_id], |r| r.get::<_, Option<String>>(0))?
        .filter_map(|r| r.transpose())
        .collect::<rusqlite::Result<_>>()?;
    drop(stmt);
    db::upsert_project(
        conn,
        &db::ScannedProject {
            root_id,
            folder: &folder,
            als_path: &als,
            name: &name,
            modified_at: mtime_secs(&als_file),
            set: &set,
            other_sets: &others,
        },
        now,
    )?;
    Ok(RelinkReport { relinked: map.len(), backup, version_id })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set_xml(sample: &str) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<Ableton Creator="Ableton Live 12.2.5">
	<LiveSet>
		<SampleRef>
			<FileRef>
				<RelativePathType Value="0" />
				<RelativePath Value="" />
				<Path Value="{sample}" />
				<Type Value="1" />
			</FileRef>
		</SampleRef>
	</LiveSet>
</Ableton>"#
        )
    }

    /// Projet indexé dont le sample pointe vers un disque absent.
    fn setup(dir: &Path) -> (Connection, i64, PathBuf, PathBuf) {
        let folder = dir.join("x Project");
        let lib = dir.join("Samples");
        fs::create_dir_all(&folder).unwrap();
        fs::create_dir_all(&lib).unwrap();
        let wav = lib.join("Kick 01.wav");
        fs::write(&wav, b"RIFF").unwrap();
        let als = folder.join("x.als");
        fs::write(&als, set_xml("Z:/disque absent/Kick 01.wav")).unwrap();

        let mut conn = db::open_in_memory().unwrap();
        let set = als_parser::parse_file(&als).unwrap();
        let (folder_s, als_s) = (folder.to_string_lossy().into_owned(), als.to_string_lossy().into_owned());
        let id = db::upsert_project(
            &mut conn,
            &db::ScannedProject {
                root_id: None,
                folder: &folder_s,
                als_path: &als_s,
                name: "x",
                modified_at: 1,
                set: &set,
                other_sets: &[],
            },
            1,
        )
        .unwrap();
        (conn, id, als, wav)
    }

    #[test]
    fn finds_and_relinks_missing_sample() {
        let dir = tempfile::tempdir().unwrap();
        let (mut conn, id, als, wav) = setup(dir.path());

        let missing = missing_samples(&conn, |_, _| {}).unwrap();
        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0].project_id, id);
        let s = &missing[0].samples[0];
        assert_eq!((s.name.as_str(), s.relinkable), ("Kick 01.wav", true));

        let found = find_candidates(&[dir.path().to_path_buf()], &["kick 01.WAV".into()], |_| {});
        let candidates = &found["kick 01.WAV"];
        assert_eq!(candidates.len(), 1);

        let open = OpenSet { set: "x".into(), project: None, modified: false };
        let relinks = [Relink { old_path: s.path.clone().unwrap(), new_path: candidates[0].clone() }];
        let vault = Vault::new(dir.path().join("vault"));
        assert!(relink(&mut conn, &vault, id, &relinks, Some(&open), 2).is_err(), "set ouvert dans Live");

        let report = relink(&mut conn, &vault, id, &relinks, None, 2).unwrap();
        assert_eq!(report.relinked, 1);
        assert_eq!(report.version_id, None);
        let bak = PathBuf::from(report.backup.unwrap());
        assert!(fs::read_to_string(&bak).unwrap().contains("Z:/disque absent"), "copie de l'original");

        let xml = String::from_utf8(als_parser::read_xml(&fs::read(&als).unwrap()).unwrap()).unwrap();
        assert!(xml.contains(&wav.to_string_lossy().replace('\\', "/")), "{xml}");
        assert!(xml.contains(r#"<RelativePath Value="../Samples/Kick 01.wav"/>"#), "{xml}");
        assert!(missing_samples(&conn, |_, _| {}).unwrap().is_empty(), "bibliothèque relue");

        // Deuxième passage : la copie précédente n'est pas écrasée.
        assert_ne!(backup_path(&als), bak);
    }

    #[test]
    fn relative_paths() {
        let rel = |a: &str, b: &str| relative_to(Path::new(a), Path::new(b));
        assert_eq!(rel("/m/p/x Project", "/m/p/x Project/Samples/a.wav").as_deref(), Some("Samples/a.wav"));
        assert_eq!(rel("/m/p/x Project", "/m/lib/a.wav").as_deref(), Some("../../lib/a.wav"));
    }
}
