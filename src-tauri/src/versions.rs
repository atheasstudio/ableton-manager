//! Versions d'un projet : instantanés (.als + audios) rangés dans le coffre,
//! restauration, branches et comparaison.
//!
//! Chaque sauvegarde détectée devient une version, enfant de la dernière
//! version de sa branche. Une branche est un dossier de travail complet,
//! recréé depuis n'importe quelle version.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::UNIX_EPOCH;

use als_parser::{LiveSet, Relocation};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::vault::{self, Vault};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("base de données : {0}")]
    Db(#[from] rusqlite::Error),
    #[error("fichier : {0}")]
    Io(#[from] std::io::Error),
    #[error("lecture du set : {0}")]
    Als(#[from] als_parser::Error),
    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, Error>;

fn other<T>(msg: impl Into<String>) -> Result<T> {
    Err(Error::Other(msg.into()))
}

/// Une seule opération sur le coffre à la fois : le nettoyage ne doit pas
/// effacer un fichier qu'un instantané vient d'y ranger.
static VAULT_LOCK: Mutex<()> = Mutex::new(());

pub(crate) fn lock() -> std::sync::MutexGuard<'static, ()> {
    VAULT_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

pub const MAIN_BRANCH: &str = "principale";

// ---------- Données ----------

#[derive(Debug, Clone, Serialize)]
pub struct Branch {
    pub id: i64,
    pub project_id: i64,
    pub name: String,
    pub folder: String,
    pub als_path: String,
    /// Version d'où part la branche.
    pub base_id: Option<i64>,
    /// Dernière version de la branche.
    pub head_id: Option<i64>,
    pub is_main: bool,
    pub created_at: i64,
}

const BRANCH_COLUMNS: &str =
    "id, project_id, name, folder, als_path, base_id, head_id, is_main, created_at";

fn branch_row(r: &rusqlite::Row) -> rusqlite::Result<Branch> {
    Ok(Branch {
        id: r.get(0)?,
        project_id: r.get(1)?,
        name: r.get(2)?,
        folder: r.get(3)?,
        als_path: r.get(4)?,
        base_id: r.get(5)?,
        head_id: r.get(6)?,
        is_main: r.get(7)?,
        created_at: r.get(8)?,
    })
}

pub fn branch(conn: &Connection, id: i64) -> Result<Branch> {
    conn.query_row(&format!("SELECT {BRANCH_COLUMNS} FROM branches WHERE id = ?1"), [id], branch_row)
        .optional()?
        .map_or_else(|| other("branche introuvable"), Ok)
}

pub fn branches(conn: &Connection, project_id: i64) -> Result<Vec<Branch>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {BRANCH_COLUMNS} FROM branches WHERE project_id = ?1 ORDER BY is_main DESC, created_at, id"
    ))?;
    let rows = stmt.query_map([project_id], branch_row)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Tous les .als suivis : (branche, chemin).
pub fn tracked(conn: &Connection) -> Result<Vec<(i64, String)>> {
    let mut stmt = conn.prepare("SELECT id, als_path FROM branches")?;
    let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrackInfo {
    pub name: String,
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LocatorInfo {
    pub time: f64,
    pub name: String,
}

/// Ce qu'on garde du contenu d'un set pour le comparer plus tard.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Summary {
    tracks: Vec<TrackInfo>,
    plugins: Vec<String>,
    locators: Vec<LocatorInfo>,
}

impl Summary {
    fn of(set: &LiveSet) -> Self {
        Self {
            tracks: set
                .tracks
                .iter()
                .map(|t| TrackInfo { name: t.name.clone(), kind: t.kind.as_str().into() })
                .collect(),
            plugins: set.plugins.iter().map(|p| p.name.clone()).collect(),
            locators: set
                .locators
                .iter()
                .map(|l| LocatorInfo { time: l.time, name: l.name.clone() })
                .collect(),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct VersionRow {
    pub id: i64,
    pub branch_id: i64,
    pub parent_id: Option<i64>,
    pub created_at: i64,
    pub source: String,
    /// Nom donné par l'utilisateur.
    pub name: Option<String>,
    /// Résumé des changements par rapport à la version parente.
    pub auto_name: String,
    pub als_name: String,
    pub tempo: Option<f64>,
    pub live_version: Option<String>,
    pub files: usize,
    /// Audios déjà introuvables au moment de la sauvegarde.
    pub missing: usize,
    /// Aperçu audio (fichier du coffre).
    pub preview_hash: Option<String>,
}

pub fn list(conn: &Connection, project_id: i64) -> Result<Vec<VersionRow>> {
    let mut stmt = conn.prepare(
        "SELECT v.id, v.branch_id, v.parent_id, v.created_at, v.source, v.name, v.auto_name,
             v.als_name, v.tempo, v.live_version, v.preview_hash,
             (SELECT count(*) FROM version_files f WHERE f.version_id = v.id),
             (SELECT count(*) FROM version_files f WHERE f.version_id = v.id AND f.hash IS NULL)
         FROM versions v WHERE v.project_id = ?1
         ORDER BY v.created_at DESC, v.id DESC",
    )?;
    let rows = stmt.query_map([project_id], |r| {
        Ok(VersionRow {
            id: r.get(0)?,
            branch_id: r.get(1)?,
            parent_id: r.get(2)?,
            created_at: r.get(3)?,
            source: r.get(4)?,
            name: r.get(5)?,
            auto_name: r.get(6)?,
            als_name: r.get(7)?,
            tempo: r.get(8)?,
            live_version: r.get(9)?,
            preview_hash: r.get(10)?,
            files: r.get::<_, i64>(11)? as usize,
            missing: r.get::<_, i64>(12)? as usize,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

struct Version {
    id: i64,
    project_id: i64,
    branch_id: i64,
    name: Option<String>,
    auto_name: String,
    als_name: String,
    als_hash: String,
}

fn version(conn: &Connection, id: i64) -> Result<Version> {
    conn.query_row(
        "SELECT id, project_id, branch_id, name, auto_name, als_name, als_hash
         FROM versions WHERE id = ?1",
        [id],
        |r| {
            Ok(Version {
                id: r.get(0)?,
                project_id: r.get(1)?,
                branch_id: r.get(2)?,
                name: r.get(3)?,
                auto_name: r.get(4)?,
                als_name: r.get(5)?,
                als_hash: r.get(6)?,
            })
        },
    )
    .optional()?
    .map_or_else(|| other("version introuvable"), Ok)
}

struct VersionFile {
    name: String,
    path: Option<String>,
    rel_path: Option<String>,
    hash: Option<String>,
}

fn version_files(conn: &Connection, id: i64) -> Result<Vec<VersionFile>> {
    let mut stmt =
        conn.prepare("SELECT name, path, rel_path, hash FROM version_files WHERE version_id = ?1")?;
    let rows = stmt.query_map([id], |r| {
        Ok(VersionFile { name: r.get(0)?, path: r.get(1)?, rel_path: r.get(2)?, hash: r.get(3)? })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

// ---------- Empreintes ----------

fn mtime(meta: &fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_millis() as i64)
}

/// Empreinte d'un fichier, recalculée seulement si sa taille ou sa date a
/// changé. Avec `store`, le fichier est aussi rangé dans le coffre.
fn file_hash(conn: &Connection, path: &Path, store: Option<&Vault>) -> Result<(String, u64)> {
    let meta = fs::metadata(path)?;
    let key = path.to_string_lossy();
    let (size, mtime) = (meta.len() as i64, mtime(&meta));
    let cached: Option<String> = conn
        .query_row(
            "SELECT hash FROM file_hashes WHERE path = ?1 AND size = ?2 AND mtime = ?3",
            params![key, size, mtime],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(hash) = cached {
        if store.is_none_or(|v| v.contains(&hash)) {
            return Ok((hash, size as u64));
        }
    }
    let (hash, len) = match store {
        Some(v) => v.store_file(path)?,
        None => vault::hash_file(path)?,
    };
    conn.execute(
        "INSERT INTO file_hashes (path, size, mtime, hash) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(path) DO UPDATE SET size = excluded.size, mtime = excluded.mtime, hash = excluded.hash",
        params![key, size, mtime, hash],
    )?;
    Ok((hash, len))
}

fn same_content(conn: &Connection, path: &Path, hash: &str) -> bool {
    path.is_file() && file_hash(conn, path, None).is_ok_and(|(h, _)| h == hash)
}

// ---------- Instantanés ----------

/// Active les versions d'un projet : crée la branche principale, importe au
/// besoin les sauvegardes du dossier `Backup/` de Live, puis l'état actuel.
pub fn enable(conn: &Connection, vault: &Vault, project_id: i64, import_backups: bool, now: i64) -> Result<usize> {
    let _guard = lock();
    if !branches(conn, project_id)?.is_empty() {
        return other("les versions sont déjà activées pour ce projet");
    }
    let (als, folder): (String, String) = conn
        .query_row("SELECT als_path, folder FROM projects WHERE id = ?1", [project_id], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .optional()?
        .map_or_else(|| other("projet introuvable"), Ok)?;
    conn.execute(
        "INSERT INTO branches (project_id, name, folder, als_path, is_main, created_at)
         VALUES (?1, ?2, ?3, ?4, 1, ?5)",
        params![project_id, MAIN_BRANCH, folder, als, now],
    )?;
    let branch_id = conn.last_insert_rowid();

    let mut created = 0;
    if import_backups {
        for (path, saved_at) in live_backups(Path::new(&als)) {
            let b = branch(conn, branch_id)?;
            // Une sauvegarde abîmée ne doit pas bloquer l'activation.
            if let Ok(Some(_)) = snapshot_file(conn, vault, &b, &path, saved_at, "backup") {
                created += 1;
            }
        }
    }
    if snapshot_unlocked(conn, vault, branch_id, "save")?.is_some() {
        created += 1;
    }
    Ok(created)
}

/// Sauvegardes automatiques de Live pour ce set (`Backup/<set> [date].als`),
/// de la plus ancienne à la plus récente.
fn live_backups(als: &Path) -> Vec<(PathBuf, i64)> {
    let (Some(dir), Some(stem)) = (als.parent(), als.file_stem()) else { return vec![] };
    let prefix = format!("{} [", stem.to_string_lossy());
    let Ok(entries) = fs::read_dir(dir.join("Backup")) else { return vec![] };
    let mut found: Vec<(PathBuf, i64)> = entries
        .flatten()
        .filter(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            name.starts_with(&prefix) && name.to_lowercase().ends_with(".als")
        })
        .filter_map(|e| Some((e.path(), mtime(&e.metadata().ok()?) / 1000)))
        .collect();
    // Le nom contient la date (« [2025-09-01 114124] ») : il départage les égalités.
    found.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
    found
}

/// Crée une version si le .als de la branche a changé depuis la dernière.
pub fn snapshot(conn: &Connection, vault: &Vault, branch_id: i64, source: &str) -> Result<Option<i64>> {
    let _guard = lock();
    snapshot_unlocked(conn, vault, branch_id, source)
}

fn snapshot_unlocked(conn: &Connection, vault: &Vault, branch_id: i64, source: &str) -> Result<Option<i64>> {
    let b = branch(conn, branch_id)?;
    let path = PathBuf::from(&b.als_path);
    let Ok(meta) = fs::metadata(&path) else { return Ok(None) };
    snapshot_file(conn, vault, &b, &path, mtime(&meta) / 1000, source)
}

fn head_hash(conn: &Connection, b: &Branch) -> Result<Option<String>> {
    let Some(head) = b.head_id else { return Ok(None) };
    Ok(conn
        .query_row("SELECT als_hash FROM versions WHERE id = ?1", [head], |r| r.get(0))
        .optional()?)
}

/// Range `als` et ses audios dans le coffre et l'ajoute à la branche.
/// Ne fait rien si le contenu est identique à la dernière version.
fn snapshot_file(
    conn: &Connection,
    vault: &Vault,
    b: &Branch,
    als: &Path,
    created_at: i64,
    source: &str,
) -> Result<Option<i64>> {
    let bytes = fs::read(als)?;
    let als_hash = vault::hash_bytes(&bytes);
    if head_hash(conn, b)?.as_deref() == Some(als_hash.as_str()) {
        return Ok(None);
    }
    // Live peut être en train d'écrire : on lit avant de ranger quoi que ce soit.
    let set = als_parser::parse_bytes(&bytes)?;
    vault.store_bytes(&bytes)?;

    let folder = Path::new(&b.folder);
    let mut files = vec![];
    for sample in set.samples.iter().filter(|s| !s.is_builtin()) {
        let name = sample.file_name().unwrap_or("?").to_owned();
        let rel = sample.relative_path.as_deref().map(|r| folder.join(r));
        let abs = sample.path.as_deref().map(PathBuf::from);
        // Live cherche d'abord dans le projet quand le chemin y est relatif.
        let candidates = if sample.is_project_relative() { [rel, abs] } else { [abs, rel] };
        let stored = candidates
            .into_iter()
            .flatten()
            .find(|p| p.is_file())
            .and_then(|p| file_hash(conn, &p, Some(vault)).ok());
        let rel_path = sample.relative_path.clone().filter(|_| sample.is_project_relative());
        files.push((name, sample.path.clone(), rel_path, stored));
    }

    let summary = Summary::of(&set);
    let content = Content {
        tempo: set.tempo,
        time_sig: set.time_signature.map(|(n, d)| format!("{n}/{d}")),
        live_version: set.live_version().map(str::to_owned),
        tracks: summary.tracks.clone(),
        plugins: summary.plugins.clone(),
        samples: files.iter().map(|f| f.0.clone()).collect(),
        locators: summary.locators.clone(),
    };
    let auto_name = match b.head_id {
        Some(parent) => describe(&diff_content(&load_content(conn, parent)?, &content)),
        None => "Première version".to_owned(),
    };
    let als_name = als_name_for(b, als);

    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "INSERT INTO versions (project_id, branch_id, parent_id, created_at, source, auto_name,
             als_name, als_hash, tempo, time_sig, live_version, summary)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            b.project_id,
            b.id,
            b.head_id,
            created_at,
            source,
            auto_name,
            als_name,
            als_hash,
            content.tempo,
            content.time_sig,
            content.live_version,
            serde_json::to_string(&summary).unwrap()
        ],
    )?;
    let id = tx.last_insert_rowid();
    {
        let mut insert = tx.prepare(
            "INSERT INTO version_files (version_id, name, path, rel_path, hash, size)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )?;
        for (name, path, rel_path, stored) in &files {
            let (hash, size) = stored.clone().unzip();
            insert.execute(params![id, name, path, rel_path, hash, size.map(|s| s as i64)])?;
        }
    }
    tx.execute("UPDATE branches SET head_id = ?2 WHERE id = ?1", params![b.id, id])?;
    tx.commit()?;
    Ok(Some(id))
}

/// Nom du fichier .als : celui de la branche, même pour une sauvegarde de `Backup/`.
fn als_name_for(b: &Branch, als: &Path) -> String {
    Path::new(&b.als_path)
        .file_name()
        .or_else(|| als.file_name())
        .map_or_else(|| "set.als".into(), |n| n.to_string_lossy().into_owned())
}

// ---------- Restauration et branches ----------

/// Chemin au format de Live (barres obliques).
fn live_path(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

/// Recrée la version `id` dans `folder` : audios du projet à leur place,
/// audios externes disparus ou modifiés copiés dans `Samples/Imported`, et le
/// .als (réécrit si des audios ont changé de place). Renvoie les audios
/// qui n'ont pas pu être recréés.
fn materialize(conn: &Connection, vault: &Vault, id: i64, folder: &Path, als_path: &Path) -> Result<Vec<String>> {
    let v = version(conn, id)?;
    let mut relocations = HashMap::new();
    let mut lost = vec![];
    for f in version_files(conn, id)? {
        let Some(hash) = f.hash else {
            lost.push(f.name);
            continue;
        };
        if let Some(rel) = &f.rel_path {
            // Live retrouve ces fichiers par le chemin relatif : il suffit de
            // les remettre en place.
            let dest = folder.join(rel);
            if !same_content(conn, &dest, &hash) {
                vault.restore_to(&hash, &dest)?;
            }
            continue;
        }
        let Some(original) = &f.path else { continue };
        if same_content(conn, Path::new(original), &hash) || relocations.contains_key(original) {
            continue;
        }
        let mut dest = folder.join("Samples/Imported").join(&f.name);
        if dest.exists() && !same_content(conn, &dest, &hash) {
            let stem = Path::new(&f.name).file_stem().map(|s| s.to_string_lossy().into_owned());
            let ext = Path::new(&f.name).extension().map(|e| e.to_string_lossy().into_owned());
            let unique = format!("{} ({})", stem.unwrap_or_default(), &hash[..8]);
            dest.set_file_name(match ext {
                Some(ext) => format!("{unique}.{ext}"),
                None => unique,
            });
        }
        if !dest.exists() {
            vault.restore_to(&hash, &dest)?;
        }
        let relative_path = live_path(dest.strip_prefix(folder).unwrap_or(&dest));
        relocations.insert(original.clone(), Relocation { path: live_path(&dest), relative_path });
    }

    let mut als = vault.read(&v.als_hash)?;
    if !relocations.is_empty() {
        als = als_parser::relocate_samples(&als, &relocations)?;
    }
    fs::create_dir_all(folder)?;
    let tmp = als_path.with_extension("als.am-tmp");
    fs::write(&tmp, &als)?;
    fs::rename(&tmp, als_path)?;
    Ok(lost)
}

#[derive(Debug, Serialize)]
pub struct RestoreReport {
    /// Version créée par la restauration (ou dernière version si rien n'a changé).
    pub version_id: Option<i64>,
    /// Audios introuvables dans le coffre.
    pub lost: Vec<String>,
}

/// Remet le dossier de travail de la branche dans l'état de la version `id`.
/// L'état actuel est d'abord sauvegardé, puis la restauration devient une
/// nouvelle version : rien n'est jamais perdu.
pub fn restore(conn: &Connection, vault: &Vault, id: i64, now: i64) -> Result<RestoreReport> {
    let _guard = lock();
    let v = version(conn, id)?;
    snapshot_unlocked(conn, vault, v.branch_id, "save")?;
    let b = branch(conn, v.branch_id)?;
    let lost = materialize(conn, vault, id, Path::new(&b.folder), Path::new(&b.als_path))?;
    let created = snapshot_file(conn, vault, &b, Path::new(&b.als_path), now, "restore")?;
    if let Some(new) = created {
        // Même contenu que la version restaurée : même aperçu.
        conn.execute(
            "UPDATE versions SET auto_name = ?2,
                 preview_hash = coalesce(preview_hash, (SELECT preview_hash FROM versions WHERE id = ?3))
             WHERE id = ?1",
            params![new, format!("Retour à « {} »", label(&v)), v.id],
        )?;
    }
    Ok(RestoreReport { version_id: created.or(branch(conn, b.id)?.head_id), lost })
}

fn label(v: &Version) -> String {
    v.name.clone().unwrap_or_else(|| v.auto_name.clone())
}

/// Caractères interdits dans un nom de dossier Windows.
fn safe_name(name: &str) -> String {
    name.chars()
        .map(|c| if "<>:\"/\\|?*".contains(c) || c.is_control() { '-' } else { c })
        .collect::<String>()
        .trim_matches([' ', '.'])
        .to_owned()
}

#[derive(Debug, Serialize)]
pub struct NewBranch {
    pub branch: Branch,
    pub lost: Vec<String>,
}

/// Crée une branche à partir de la version `id` : un nouveau dossier de projet
/// à côté de l'original (« <projet> - <branche> Project ») avec le .als et
/// ses audios. Les sauvegardes de ce dossier alimentent ensuite la branche.
pub fn create_branch(conn: &Connection, vault: &Vault, id: i64, name: &str, now: i64) -> Result<NewBranch> {
    let _guard = lock();
    let name = name.trim();
    let folder_part = safe_name(name);
    if folder_part.is_empty() {
        return other("donne un nom à la branche");
    }
    let v = version(conn, id)?;
    let all = branches(conn, v.project_id)?;
    if all.iter().any(|b| b.name.eq_ignore_ascii_case(name)) {
        return other(format!("la branche « {name} » existe déjà"));
    }
    let main = all.iter().find(|b| b.is_main).map_or_else(|| other("branche principale introuvable"), Ok)?;
    let main_folder = Path::new(&main.folder);
    let base = main_folder.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let base = base.strip_suffix(" Project").unwrap_or(&base);
    let folder = main_folder.with_file_name(format!("{base} - {folder_part} Project"));
    if folder.exists() {
        return other(format!("le dossier {} existe déjà", folder.display()));
    }
    let als_path = folder.join(&v.als_name);
    let lost = materialize(conn, vault, id, &folder, &als_path)?;
    conn.execute(
        "INSERT INTO branches (project_id, name, folder, als_path, base_id, head_id, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?5, ?6)",
        params![v.project_id, name, folder.to_string_lossy(), als_path.to_string_lossy(), v.id, now],
    )?;
    let branch = branch(conn, conn.last_insert_rowid())?;
    Ok(NewBranch { branch, lost })
}

pub fn rename(conn: &Connection, id: i64, name: &str) -> Result<()> {
    let name = Some(name.trim()).filter(|n| !n.is_empty());
    conn.execute("UPDATE versions SET name = ?2 WHERE id = ?1", params![id, name])?;
    Ok(())
}

/// Supprime une version : ses enfants sont rattachés à sa parente, et les
/// fichiers que plus aucune version n'utilise quittent le coffre.
/// Renvoie les octets libérés.
pub fn delete(conn: &Connection, vault: &Vault, id: i64) -> Result<u64> {
    let _guard = lock();
    let parent: Option<i64> = conn
        .query_row("SELECT parent_id FROM versions WHERE id = ?1", [id], |r| r.get(0))
        .optional()?
        .map_or_else(|| other("version introuvable"), Ok)?;
    let tx = conn.unchecked_transaction()?;
    tx.execute("UPDATE versions SET parent_id = ?2 WHERE parent_id = ?1", params![id, parent])?;
    tx.execute("UPDATE branches SET head_id = ?2 WHERE head_id = ?1", params![id, parent])?;
    tx.execute("UPDATE branches SET base_id = ?2 WHERE base_id = ?1", params![id, parent])?;
    tx.execute("DELETE FROM versions WHERE id = ?1", [id])?;
    tx.commit()?;
    Ok(collect_garbage(conn, vault)?.1)
}

pub(crate) fn collect_garbage(conn: &Connection, vault: &Vault) -> Result<(usize, u64)> {
    const PREVIEWS: &str = "SELECT preview_hash FROM versions WHERE preview_hash IS NOT NULL
                            UNION SELECT preview_hash FROM projects WHERE preview_hash IS NOT NULL";
    conn.execute(&format!("DELETE FROM previews WHERE hash NOT IN ({PREVIEWS})"), [])?;
    let mut stmt = conn.prepare(&format!(
        "SELECT als_hash FROM versions
         UNION {PREVIEWS}
         UNION SELECT hash FROM version_files WHERE hash IS NOT NULL"
    ))?;
    let used = stmt.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<HashSet<String>>>()?;
    Ok(vault.collect_garbage(&used)?)
}

/// Déplace le coffre (ex. vers un disque dédié).
pub fn move_vault(vault: &Vault, dest: &Path) -> Result<Vault> {
    let _guard = lock();
    Ok(vault.move_to(dest)?)
}

// ---------- Comparaison ----------

struct Content {
    tempo: Option<f64>,
    time_sig: Option<String>,
    live_version: Option<String>,
    tracks: Vec<TrackInfo>,
    plugins: Vec<String>,
    samples: Vec<String>,
    locators: Vec<LocatorInfo>,
}

fn load_content(conn: &Connection, id: i64) -> Result<Content> {
    let (tempo, time_sig, live_version, summary): (_, _, _, String) = conn.query_row(
        "SELECT tempo, time_sig, live_version, summary FROM versions WHERE id = ?1",
        [id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
    )?;
    let summary: Summary = serde_json::from_str(&summary).unwrap_or_default();
    let samples = version_files(conn, id)?.into_iter().map(|f| f.name).collect();
    Ok(Content {
        tempo,
        time_sig,
        live_version,
        tracks: summary.tracks,
        plugins: summary.plugins,
        samples,
        locators: summary.locators,
    })
}

/// Ce qui change d'une version A à une version B.
#[derive(Debug, Default, Serialize)]
pub struct Diff {
    /// (avant, après), seulement si différent.
    pub tempo: Option<(Option<f64>, Option<f64>)>,
    pub time_sig: Option<(Option<String>, Option<String>)>,
    pub live_version: Option<(Option<String>, Option<String>)>,
    pub tracks_added: Vec<TrackInfo>,
    pub tracks_removed: Vec<TrackInfo>,
    pub plugins_added: Vec<String>,
    pub plugins_removed: Vec<String>,
    pub samples_added: Vec<String>,
    pub samples_removed: Vec<String>,
    pub locators_added: Vec<String>,
    pub locators_removed: Vec<String>,
}

fn changed<T: PartialEq>(a: T, b: T) -> Option<(T, T)> {
    (a != b).then_some((a, b))
}

/// Éléments de `b` absents de `a`, en tenant compte des doublons (deux pistes
/// « Audio » d'un côté, trois de l'autre = une ajoutée).
fn added<T: PartialEq + Clone>(a: &[T], b: &[T]) -> Vec<T> {
    let mut left: Vec<&T> = a.iter().collect();
    b.iter()
        .filter(|x| match left.iter().position(|y| y == x) {
            Some(i) => {
                left.swap_remove(i);
                false
            }
            None => true,
        })
        .cloned()
        .collect()
}

/// Live numérote les pistes sans nom (« 2-Audio ») et renumérote tout quand
/// on en insère une : on compare sans ce numéro.
fn track_key(t: &TrackInfo) -> TrackInfo {
    let name = match t.name.split_once('-') {
        Some((n, rest)) if !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()) => rest,
        _ => &t.name,
    };
    TrackInfo { name: name.to_owned(), kind: t.kind.clone() }
}

fn diff_content(a: &Content, b: &Content) -> Diff {
    let loc = |c: &Content| c.locators.iter().map(|l| l.name.clone()).collect::<Vec<_>>();
    let tracks = |c: &Content| c.tracks.iter().map(track_key).collect::<Vec<_>>();
    Diff {
        tempo: changed(a.tempo, b.tempo),
        time_sig: changed(a.time_sig.clone(), b.time_sig.clone()),
        live_version: changed(a.live_version.clone(), b.live_version.clone()),
        tracks_added: added(&tracks(a), &tracks(b)),
        tracks_removed: added(&tracks(b), &tracks(a)),
        plugins_added: added(&a.plugins, &b.plugins),
        plugins_removed: added(&b.plugins, &a.plugins),
        samples_added: added(&a.samples, &b.samples),
        samples_removed: added(&b.samples, &a.samples),
        locators_added: added(&loc(a), &loc(b)),
        locators_removed: added(&loc(b), &loc(a)),
    }
}

pub fn compare(conn: &Connection, a: i64, b: i64) -> Result<Diff> {
    Ok(diff_content(&load_content(conn, a)?, &load_content(conn, b)?))
}

/// Nom automatique d'une version : ses changements principaux.
fn describe(d: &Diff) -> String {
    /// `prefix` distingue les plugins des pistes, qui portent souvent le même nom.
    fn names(sign: &str, prefix: &str, what: &str, items: &[String]) -> Option<String> {
        match items.len() {
            0 => None,
            1 | 2 => Some(format!("{sign} {prefix}{}", items.join(", "))),
            n => Some(format!("{sign}{n} {what}")),
        }
    }
    let track_names = |t: &[TrackInfo]| t.iter().map(|t| t.name.clone()).collect::<Vec<_>>();
    let mut parts: Vec<String> = [
        names("+", "", "pistes", &track_names(&d.tracks_added)),
        names("−", "", "pistes", &track_names(&d.tracks_removed)),
        names("+", "plugin ", "plugins", &d.plugins_added),
        names("−", "plugin ", "plugins", &d.plugins_removed),
    ]
    .into_iter()
    .flatten()
    .collect();
    if let Some((Some(a), Some(b))) = d.tempo {
        parts.push(format!("{a} → {b} BPM"));
    }
    if let Some((Some(a), Some(b))) = &d.time_sig {
        parts.push(format!("{a} → {b}"));
    }
    match (d.samples_added.len(), d.samples_removed.len()) {
        (0, 0) => {}
        (n, 0) => parts.push(format!("+{n} audio{}", if n > 1 { "s" } else { "" })),
        (0, n) => parts.push(format!("−{n} audio{}", if n > 1 { "s" } else { "" })),
        (a, r) => parts.push(format!("audios +{a} −{r}")),
    }
    if !d.locators_added.is_empty() || !d.locators_removed.is_empty() {
        parts.push("repères modifiés".into());
    }
    if parts.is_empty() {
        "Clips, réglages ou automations".into()
    } else {
        parts.join(" · ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    /// « Maintenant », après les dates des fichiers écrits par les tests.
    const NOW: i64 = 4_000_000_000;

    /// Écrit un petit set Live 12 avec ces pistes et ce tempo.
    fn write_set(path: &Path, tracks: &[&str], tempo: f64, samples: &[(&str, &str)]) {
        let tracks: String = tracks
            .iter()
            .map(|t| format!(r#"<AudioTrack><Name><EffectiveName Value="{t}" /></Name></AudioTrack>"#))
            .collect();
        let samples: String = samples
            .iter()
            .map(|(rel, abs)| {
                let kind = if rel.is_empty() { 0 } else { 3 };
                format!(
                    r#"<AudioClip><SampleRef><FileRef><RelativePathType Value="{kind}" /><RelativePath Value="{rel}" /><Path Value="{abs}" /></FileRef></SampleRef></AudioClip>"#
                )
            })
            .collect();
        let xml = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<Ableton Creator="Ableton Live 12.2.5"><LiveSet><Tracks>{tracks}<AudioTrack><Name><EffectiveName Value="Clips" /></Name>{samples}</AudioTrack></Tracks>
<MainTrack><DeviceChain><Mixer><Tempo><Manual Value="{tempo}" /></Tempo></Mixer></DeviceChain></MainTrack></LiveSet></Ableton>"#
        );
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, xml).unwrap();
    }

    struct Fixture {
        _dir: tempfile::TempDir,
        root: PathBuf,
        conn: Connection,
        vault: Vault,
        project: i64,
        als: PathBuf,
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_owned();
        let conn = db::open(&root.join("library.db")).unwrap();
        let vault = Vault::new(root.join("coffre"));
        let folder = root.join("Projets/woodkid Project");
        let als = folder.join("woodkid.als");
        conn.execute(
            "INSERT INTO projects (folder, als_path, name, modified_at, scanned_at)
             VALUES (?1, ?2, 'woodkid', 0, 0)",
            params![folder.to_string_lossy(), als.to_string_lossy()],
        )
        .unwrap();
        let project = conn.last_insert_rowid();
        Fixture { _dir: dir, root, conn, vault, project, als }
    }

    #[test]
    fn snapshots_dedupe_and_describe_changes() {
        let f = fixture();
        let folder = f.als.parent().unwrap();
        let pack = f.root.join("Pack/clap.wav");
        fs::create_dir_all(pack.parent().unwrap()).unwrap();
        fs::write(&pack, b"clap").unwrap();
        fs::create_dir_all(folder.join("Samples/Recorded")).unwrap();
        fs::write(folder.join("Samples/Recorded/voix.wav"), b"voix").unwrap();
        let samples = [("", live_path(&pack)), ("Samples/Recorded/voix.wav", "C:/vieux/voix.wav".into())];
        let samples: Vec<(&str, &str)> = samples.iter().map(|(a, b)| (*a, b.as_str())).collect();

        // Deux sauvegardes de Live dans Backup/, puis l'état actuel.
        write_set(&folder.join("Backup/woodkid [2025-09-01 114124].als"), &["Kick"], 120.0, &samples);
        write_set(&folder.join("Backup/woodkid [2025-09-01 122623].als"), &["Kick"], 120.0, &samples);
        write_set(&folder.join("Backup/autre set [2025-09-01 122623].als"), &["X"], 90.0, &[]);
        write_set(&f.als, &["Kick", "Basse"], 128.0, &samples);

        assert_eq!(enable(&f.conn, &f.vault, f.project, true, NOW).unwrap(), 2, "backup identique ignoré");
        let versions = list(&f.conn, f.project).unwrap();
        assert_eq!(versions.len(), 2);
        assert_eq!(versions[1].auto_name, "Première version");
        assert_eq!(versions[1].source, "backup");
        assert_eq!(versions[0].auto_name, "+ Basse · 120 → 128 BPM");
        assert_eq!(versions[0].parent_id, Some(versions[1].id));
        assert_eq!((versions[0].files, versions[0].missing), (2, 0));
        // 2 .als différents + 2 audios, rangés une seule fois.
        assert_eq!(f.vault.usage().unwrap().0, 4);

        let main = branches(&f.conn, f.project).unwrap()[0].id;
        assert_eq!(snapshot(&f.conn, &f.vault, main, "save").unwrap(), None, "rien n'a changé");

        let d = compare(&f.conn, versions[1].id, versions[0].id).unwrap();
        assert_eq!(d.tracks_added, [TrackInfo { name: "Basse".into(), kind: "audio".into() }]);
        assert_eq!(d.tempo, Some((Some(120.0), Some(128.0))));
        assert!(d.plugins_added.is_empty() && d.samples_removed.is_empty());
    }

    #[test]
    fn restore_branch_and_delete() {
        let f = fixture();
        let folder = f.als.parent().unwrap().to_owned();
        let pack = f.root.join("Pack/clap.wav");
        fs::create_dir_all(pack.parent().unwrap()).unwrap();
        fs::write(&pack, b"clap v1").unwrap();
        let voix = folder.join("Samples/Recorded/voix.wav");
        fs::create_dir_all(voix.parent().unwrap()).unwrap();
        fs::write(&voix, b"voix").unwrap();
        let pack_live = live_path(&pack);
        let samples = [("", pack_live.as_str()), ("Samples/Recorded/voix.wav", "C:/x/voix.wav")];

        write_set(&f.als, &["Kick"], 120.0, &samples);
        enable(&f.conn, &f.vault, f.project, false, NOW).unwrap();
        let v1 = list(&f.conn, f.project).unwrap()[0].id;
        let main = branches(&f.conn, f.project).unwrap()[0].id;

        // L'utilisateur continue : nouvelle piste, l'enregistrement est effacé,
        // le sample de la bibliothèque est modifié.
        write_set(&f.als, &["Kick", "Pad"], 120.0, &[]);
        fs::remove_file(&voix).unwrap();
        fs::write(&pack, b"clap v2 plus long").unwrap();

        let report = restore(&f.conn, &f.vault, v1, NOW).unwrap();
        assert!(report.lost.is_empty());
        let versions = list(&f.conn, f.project).unwrap();
        assert_eq!(versions.len(), 3, "état actuel sauvegardé + restauration");
        assert_eq!(versions[0].source, "restore");
        assert_eq!(versions[0].auto_name, "Retour à « Première version »");
        assert_eq!(fs::read(&voix).unwrap(), b"voix", "enregistrement remis en place");
        let imported = folder.join("Samples/Imported/clap.wav");
        assert_eq!(fs::read(&imported).unwrap(), b"clap v1", "ancien sample recopié dans le projet");
        let set = als_parser::parse_file(&f.als).unwrap();
        assert_eq!(set.tracks.len(), 2, "Kick + Clips");
        let clap = set.samples.iter().find(|s| s.file_name() == Some("clap.wav")).unwrap();
        assert_eq!(clap.path.as_deref(), Some(live_path(&imported).as_str()));
        assert_eq!(clap.relative_path.as_deref(), Some("Samples/Imported/clap.wav"));
        assert_eq!(fs::read(&pack).unwrap(), b"clap v2 plus long", "l'original n'est pas touché");

        // Branche depuis la version avec la piste Pad.
        let pad = versions[1].id;
        let new = create_branch(&f.conn, &f.vault, pad, "version club", NOW).unwrap();
        let b = new.branch;
        assert_eq!(b.base_id, Some(pad));
        let branch_als = f.root.join("Projets/woodkid - version club Project/woodkid.als");
        assert_eq!(PathBuf::from(&b.als_path), branch_als);
        assert_eq!(als_parser::parse_file(&branch_als).unwrap().tracks.len(), 3);
        assert!(create_branch(&f.conn, &f.vault, pad, "Version Club", NOW).is_err());
        assert!(db::is_branch_folder(&f.conn, &b.folder).unwrap());

        // Une sauvegarde dans le dossier de la branche part de la version d'origine.
        write_set(&branch_als, &["Kick", "Pad", "Sub"], 126.0, &[]);
        let on_branch = snapshot(&f.conn, &f.vault, b.id, "save").unwrap().unwrap();
        let versions = list(&f.conn, f.project).unwrap();
        let row = versions.iter().find(|v| v.id == on_branch).unwrap();
        assert_eq!((row.branch_id, row.parent_id), (b.id, Some(pad)));
        assert_eq!(row.auto_name, "+ Sub · 120 → 126 BPM");

        // Supprimer la version d'origine rattache la branche à la parente.
        let before = f.vault.usage().unwrap().0;
        delete(&f.conn, &f.vault, pad).unwrap();
        let versions = list(&f.conn, f.project).unwrap();
        assert_eq!(versions.iter().find(|v| v.id == on_branch).unwrap().parent_id, Some(v1));
        assert_eq!(branch(&f.conn, b.id).unwrap().base_id, Some(v1));
        assert!(f.vault.usage().unwrap().0 < before, "le .als supprimé quitte le coffre");
        let _ = main;
    }

    #[test]
    fn real_projects_roundtrip() {
        let Ok(dir) = std::env::var("ALS_FIXTURES") else { return };
        let f = fixture();
        for p in crate::scanner::find_projects(Path::new(&dir)) {
            f.conn
                .execute(
                    "INSERT INTO projects (folder, als_path, name, modified_at, scanned_at)
                     VALUES (?1, ?2, ?3, 0, 0)",
                    params![p.folder.to_string_lossy(), p.main_set.to_string_lossy(), p.name()],
                )
                .unwrap();
            let id = f.conn.last_insert_rowid();
            let n = enable(&f.conn, &f.vault, id, true, 1).unwrap();
            println!("{} : {n} versions", p.name());
            for v in list(&f.conn, id).unwrap().iter().rev() {
                println!("  {} · {} audios ({} introuvables) · {}", v.source, v.files, v.missing, v.auto_name);
            }
        }
        let (count, bytes) = f.vault.usage().unwrap();
        println!("coffre : {count} fichiers, {} Ko", bytes / 1024);
    }
}
