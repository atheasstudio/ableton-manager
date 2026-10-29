//! Pré-écoute : aperçus audio des projets et de leurs versions.
//!
//! Trois sources :
//! - les écoutes dans Live, enregistrées sur le master par le device Max for
//!   Live avec la position dans le morceau, puis assemblées : chaque passage
//!   est placé à sa mesure, le plus récent l'emporte ;
//! - le rendu complet à la demande (lecture du début à la fin, même chemin) ;
//! - un bounce déjà présent dans le dossier du projet, ou un fichier choisi.
//!
//! Un aperçu est un MP3 du coffre aligné sur la mesure 1 :
//! seconde = beats × 60 / tempo.

use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

use crate::audio::{self, Pcm};
use crate::live_window::OpenSet;
use crate::vault::Vault;
use crate::versions;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("base de données : {0}")]
    Db(#[from] rusqlite::Error),
    #[error("fichier : {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Audio(#[from] audio::Error),
    #[error("{0}")]
    Versions(#[from] versions::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Intervalle entre deux positions envoyées par le device.
const POLL: f64 = 0.05;
/// Écart toléré entre la position attendue et la position lue : au-delà, la
/// lecture a sauté (boucle, clic dans la timeline).
const JUMP_BEATS: f64 = 0.25;
/// Queue gardée après le dernier évènement du morceau (réverbs, delays).
const TAIL_BEATS: f64 = 4.0;
/// Les écoutes en attente ne dépassent pas cette taille…
const MAX_CAPTURE_BYTES: u64 = 2 << 30;
/// … ni cet âge.
const MAX_CAPTURE_AGE_MS: i64 = 30 * 24 * 3600 * 1000;

// ---------- Aperçus ----------

#[derive(Debug, Serialize)]
pub struct PreviewInfo {
    pub hash: String,
    /// Secondes.
    pub duration: f64,
    /// Forme d'onde : niveau crête de chaque tranche, de 0 à 255.
    pub peaks: Vec<u8>,
    pub source: String,
    /// Tempo qui aligne l'aperçu sur les mesures.
    pub tempo: Option<f64>,
    pub created_at: i64,
}

pub fn info(conn: &Connection, hash: &str) -> Result<Option<PreviewInfo>> {
    Ok(conn
        .query_row(
            "SELECT hash, duration, peaks, source, tempo, created_at FROM previews WHERE hash = ?1",
            [hash],
            |r| {
                Ok(PreviewInfo {
                    hash: r.get(0)?,
                    duration: r.get(1)?,
                    peaks: r.get(2)?,
                    source: r.get(3)?,
                    tempo: r.get(4)?,
                    created_at: r.get(5)?,
                })
            },
        )
        .optional()?)
}

/// Encode `pcm` en MP3 et le range dans le coffre. Les mises à jour qui y
/// font référence passent par `link`, sous le verrou du coffre.
fn store(
    conn: &Connection,
    vault: &Vault,
    pcm: &Pcm,
    source: &str,
    tempo: Option<f64>,
    now: i64,
    link: impl FnOnce(&Connection, &str) -> Result<()>,
) -> Result<String> {
    let mp3 = audio::encode_mp3(pcm)?;
    let peaks = audio::peaks(pcm, audio::PEAKS);
    // Le nettoyage du coffre ne doit pas passer entre le rangement et le lien.
    let _guard = versions::lock();
    let hash = vault.store_bytes(&mp3)?;
    conn.execute(
        "INSERT OR IGNORE INTO previews (hash, duration, peaks, source, tempo, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![hash, pcm.seconds(), peaks, source, tempo, now],
    )?;
    link(conn, &hash)?;
    Ok(hash)
}

fn project_tempo(conn: &Connection, project_id: i64) -> Result<Option<f64>> {
    Ok(conn
        .query_row("SELECT tempo FROM projects WHERE id = ?1", [project_id], |r| r.get(0))
        .optional()?
        .flatten())
}

/// Prend un fichier audio (bounce, export) comme aperçu du projet.
pub fn import_file(conn: &Connection, vault: &Vault, project_id: i64, path: &Path, now: i64) -> Result<String> {
    let mut pcm = audio::decode(path)?;
    pcm.trim_end();
    let tempo = project_tempo(conn, project_id)?;
    store(conn, vault, &pcm, "file", tempo, now, |c, hash| {
        c.execute("UPDATE projects SET preview_hash = ?2 WHERE id = ?1", params![project_id, hash])?;
        Ok(())
    })
}

const AUDIO_EXTENSIONS: &[&str] = &["wav", "aif", "aiff", "mp3", "flac", "ogg", "m4a"];

/// Bounce le plus probable dans le dossier du projet (là où Live exporte par
/// défaut) : un fichier audio qui porte le nom du set de préférence, sinon le
/// plus récent. Les sous-dossiers (Samples, Backup…) sont ignorés.
pub fn find_bounce(folder: &Path, set_name: &str) -> Option<PathBuf> {
    let set_name = set_name.to_lowercase();
    fs::read_dir(folder)
        .ok()?
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            let ext = path.extension()?.to_str()?.to_lowercase();
            let meta = e.metadata().ok()?;
            if !meta.is_file() || meta.len() < 50_000 || !AUDIO_EXTENSIONS.contains(&ext.as_str()) {
                return None;
            }
            let named = path.file_stem()?.to_string_lossy().to_lowercase().contains(&set_name);
            Some((named, meta.modified().ok()?, path))
        })
        .max_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)))
        .map(|(_, _, p)| p)
}

// ---------- Set ouvert dans Live ----------

/// Set suivi par l'appli : celui d'un projet, ou d'une branche.
#[derive(Debug, Clone, PartialEq)]
pub struct SetRef {
    pub als_path: String,
    pub project_id: i64,
    pub branch_id: Option<i64>,
}

pub fn matches(als: &str, open: &OpenSet) -> bool {
    let p = Path::new(als);
    let same = |a: Option<&std::ffi::OsStr>, b: &str| a.is_some_and(|a| a.to_string_lossy().eq_ignore_ascii_case(b));
    same(p.file_stem(), &open.set)
        && open.project.as_deref().is_none_or(|f| same(p.parent().and_then(Path::file_name), f))
}

/// Retrouve le .als du set ouvert dans Live (nom du set + dossier du projet).
pub fn resolve(conn: &Connection, open: &OpenSet) -> Result<Option<SetRef>> {
    let mut stmt = conn.prepare("SELECT id, project_id, als_path FROM branches")?;
    let branches: Vec<(i64, i64, String)> =
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<rusqlite::Result<_>>()?;
    if let Some((id, project_id, als)) = branches.into_iter().find(|b| matches(&b.2, open)) {
        return Ok(Some(SetRef { als_path: als, project_id, branch_id: Some(id) }));
    }
    let mut stmt = conn.prepare("SELECT id, als_path FROM projects ORDER BY modified_at DESC")?;
    let projects: Vec<(i64, String)> =
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
    let found: Vec<_> = projects.into_iter().filter(|p| matches(&p.1, open)).collect();
    // Sans dossier de projet dans le titre, un nom seul ne suffit que s'il est unique.
    if open.project.is_none() && found.len() != 1 {
        return Ok(None);
    }
    Ok(found
        .into_iter()
        .next()
        .map(|(project_id, als_path)| SetRef { als_path, project_id, branch_id: None }))
}

// ---------- Écoutes captées dans Live ----------

/// Une écoute terminée, telle que le device l'a envoyée.
#[derive(Debug, Clone)]
pub struct NewCapture {
    pub path: PathBuf,
    /// Millisecondes depuis 1970.
    pub started_at: i64,
    pub tempo: f64,
    /// Fin de l'arrangement, en beats.
    pub song_end: f64,
    /// (ms depuis le début de l'enregistrement, position en beats)
    pub positions: Vec<(f64, f64)>,
    /// Rendu complet allé jusqu'au bout.
    pub render: bool,
    /// Le set avait des modifications non sauvegardées.
    pub modified: bool,
}

struct Capture {
    id: i64,
    path: String,
    started_at: i64,
    tempo: f64,
    song_end: f64,
    positions: Vec<(f64, f64)>,
    render: bool,
}

fn captures(conn: &Connection, als_path: &str, until_ms: Option<i64>) -> Result<Vec<Capture>> {
    let mut stmt = conn.prepare(
        "SELECT id, path, started_at, tempo, song_end, positions, render FROM captures
         WHERE als_path = ?1 AND (?2 IS NULL OR started_at <= ?2) ORDER BY started_at, id",
    )?;
    let rows = stmt.query_map(params![als_path, until_ms], |r| {
        let positions: String = r.get(5)?;
        Ok(Capture {
            id: r.get(0)?,
            path: r.get(1)?,
            started_at: r.get(2)?,
            tempo: r.get(3)?,
            song_end: r.get(4)?,
            positions: serde_json::from_str(&positions).unwrap_or_default(),
            render: r.get(6)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

fn delete_capture(conn: &Connection, id: i64, path: &str) -> Result<()> {
    let _ = fs::remove_file(path);
    conn.execute("DELETE FROM captures WHERE id = ?1", [id])?;
    Ok(())
}

/// Passage continu d'une écoute : de `from` à `to` (secondes dans
/// l'enregistrement), placé à `at` secondes dans le morceau.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Run {
    from: f64,
    to: f64,
    at: f64,
}

/// Découpe une écoute en passages continus. Une boucle ou un saut dans la
/// timeline commence un nouveau passage. Chaque passage est placé avec la
/// moyenne des positions lues, ce qui gomme l'imprécision de chaque mesure.
fn runs(positions: &[(f64, f64)], tempo: f64) -> Vec<Run> {
    let mut out = vec![];
    let mut start = 0;
    for i in 0..positions.len() {
        let next = positions.get(i + 1);
        let broken = next.is_none_or(|&(ms, beats)| {
            let (prev_ms, prev_beats) = positions[i];
            let dt = (ms - prev_ms) / 1000.0;
            dt <= 0.0 || dt > 1.0 || (beats - prev_beats - dt * tempo / 60.0).abs() > JUMP_BEATS
        });
        if !broken {
            continue;
        }
        let run = &positions[start..=i];
        let offset = run.iter().map(|&(ms, beats)| beats * 60.0 / tempo - ms / 1000.0).sum::<f64>()
            / run.len() as f64;
        let from = run[0].0 / 1000.0;
        // Le saut a eu lieu entre deux lectures : on s'arrête au plus tard à la suivante.
        let to = (positions[i].0 / 1000.0 + POLL).min(next.map_or(f64::MAX, |n| n.0 / 1000.0));
        out.push(Run { from, to, at: from + offset });
        start = i + 1;
    }
    out
}

/// Longueur de [a, b) que `covered` (intervalles triés, disjoints) ne couvre pas,
/// puis ajoute [a, b) à `covered`.
fn uncover(covered: &mut Vec<(f64, f64)>, a: f64, b: f64) -> f64 {
    let mut free = b - a;
    for &(x, y) in covered.iter() {
        free -= (y.min(b) - x.max(a)).max(0.0);
    }
    covered.push((a, b));
    covered.sort_by(|p, q| p.0.total_cmp(&q.0));
    let mut merged: Vec<(f64, f64)> = vec![];
    for &(x, y) in covered.iter() {
        match merged.last_mut() {
            Some(last) if x <= last.1 => last.1 = last.1.max(y),
            _ => merged.push((x, y)),
        }
    }
    *covered = merged;
    free.max(0.0)
}

struct Assembly {
    pcm: Pcm,
    tempo: f64,
    /// Le plus récent est un rendu complet.
    render: bool,
    /// Écoutes entièrement recouvertes par de plus récentes (ou à un autre tempo).
    useless: Vec<(i64, String)>,
}

/// Assemble les écoutes d'un set en un aperçu aligné sur la mesure 1.
fn assemble(list: &[Capture]) -> Result<Option<Assembly>> {
    let Some(newest) = list.last() else { return Ok(None) };
    let tempo = newest.tempo;
    let mut useless = vec![];
    // Un tempo différent décale toutes les mesures : les vieilles écoutes ne collent plus.
    let (usable, other_tempo): (Vec<&Capture>, Vec<&Capture>) =
        list.iter().partition(|c| (c.tempo - tempo).abs() < 0.01);
    useless.extend(other_tempo.iter().map(|c| (c.id, c.path.clone())));

    let song_end = usable.iter().rev().map(|c| c.song_end).find(|&e| e > 0.0);
    let limit = song_end.map(|e| (e + TAIL_BEATS) * 60.0 / tempo);

    // Du plus récent au plus ancien : on calcule ce que chaque écoute apporte
    // encore, puis on colle du plus ancien au plus récent.
    let mut covered = vec![];
    let mut keep = vec![];
    for c in usable.iter().rev() {
        let mut useful = 0.0;
        let runs = runs(&c.positions, tempo);
        for r in &runs {
            let a = r.at.max(0.0);
            let b = (r.at + r.to - r.from).min(limit.unwrap_or(f64::MAX));
            if b > a {
                useful += uncover(&mut covered, a, b);
            }
        }
        if useful < 0.05 {
            useless.push((c.id, c.path.clone()));
        } else {
            keep.push((*c, runs));
        }
    }

    let mut out: Option<Pcm> = None;
    for (c, runs) in keep.iter().rev() {
        let pcm = match audio::decode(Path::new(&c.path)) {
            Ok(p) => p,
            // Fichier disparu ou illisible : il ne servira plus.
            Err(_) => {
                useless.push((c.id, c.path.clone()));
                continue;
            }
        };
        let out = out.get_or_insert_with(|| Pcm::silent(pcm.rate, 0));
        if pcm.rate != out.rate {
            useless.push((c.id, c.path.clone()));
            continue;
        }
        let rate = pcm.rate as f64;
        for r in runs {
            // Avant la mesure 1 (décompte, pré-roll) : coupé.
            let skip = (-r.at).max(0.0);
            let from = ((r.from + skip) * rate).round() as usize;
            let to = (r.to * rate).round() as usize;
            out.paste(&pcm, from, to, ((r.at + skip) * rate).round() as usize);
        }
    }
    let Some(mut pcm) = out else { return Ok(None) };
    if let Some(limit) = limit {
        pcm.truncate((limit * pcm.rate as f64) as usize);
    }
    pcm.trim_end();
    if pcm.len() == 0 {
        return Ok(None);
    }
    Ok(Some(Assembly { pcm, tempo, render: newest.render, useless }))
}

/// Supprime les écoutes en attente trop vieilles ou en trop (les plus anciennes d'abord).
fn prune_old(conn: &Connection, now_ms: i64) -> Result<()> {
    let mut stmt = conn.prepare("SELECT id, path, started_at FROM captures ORDER BY started_at DESC")?;
    let rows: Vec<(i64, String, i64)> =
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<rusqlite::Result<_>>()?;
    let mut total = 0;
    for (id, path, started_at) in rows {
        total += fs::metadata(&path).map_or(0, |m| m.len());
        if total > MAX_CAPTURE_BYTES || now_ms - started_at > MAX_CAPTURE_AGE_MS {
            delete_capture(conn, id, &path)?;
        }
    }
    Ok(())
}

/// Range une écoute terminée et met à jour l'aperçu du projet. Si le set
/// était sauvegardé (pas de « * »), l'écoute est aussi celle de la dernière
/// version de sa branche. Renvoie l'aperçu créé.
pub fn add_capture(conn: &Connection, vault: &Vault, set: &SetRef, c: &NewCapture, now: i64) -> Result<Option<String>> {
    conn.execute(
        "INSERT INTO captures (als_path, path, started_at, tempo, song_end, positions, render, modified)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            set.als_path,
            c.path.to_string_lossy(),
            c.started_at,
            c.tempo,
            c.song_end,
            serde_json::to_string(&c.positions).unwrap(),
            c.render,
            c.modified
        ],
    )?;
    prune_old(conn, now * 1000)?;

    let Some(a) = assemble(&captures(conn, &set.als_path, None)?)? else { return Ok(None) };
    for (id, path) in &a.useless {
        delete_capture(conn, *id, path)?;
    }
    let head = match (set.branch_id, c.modified) {
        (Some(branch), false) => versions::branch(conn, branch)?
            .head_id
            .map(|id| -> Result<_> {
                let saved: i64 = conn.query_row("SELECT created_at FROM versions WHERE id = ?1", [id], |r| r.get(0))?;
                Ok((id, saved))
            })
            .transpose()?
            // L'écoute doit avoir commencé après la sauvegarde.
            .filter(|&(_, saved)| saved * 1000 <= c.started_at)
            .map(|(id, _)| id),
        _ => None,
    };
    let source = if a.render { "render" } else { "capture" };
    let hash = store(conn, vault, &a.pcm, source, Some(a.tempo), now, |conn, hash| {
        conn.execute("UPDATE projects SET preview_hash = ?2 WHERE id = ?1", params![set.project_id, hash])?;
        if let Some(head) = head {
            conn.execute("UPDATE versions SET preview_hash = ?2 WHERE id = ?1", params![head, hash])?;
        }
        Ok(())
    })?;
    Ok(Some(hash))
}

/// Donne un aperçu à une version qui vient d'être créée, si le set a été
/// écouté depuis la version précédente : les écoutes faites jusqu'à la
/// sauvegarde, assemblées. Renvoie vrai si un aperçu a été attaché.
pub fn on_new_version(conn: &Connection, vault: &Vault, version_id: i64, now: i64) -> Result<bool> {
    let (als_path, saved, parent_saved): (String, i64, Option<i64>) = conn.query_row(
        "SELECT b.als_path, v.created_at, (SELECT p.created_at FROM versions p WHERE p.id = v.parent_id)
         FROM versions v JOIN branches b ON b.id = v.branch_id WHERE v.id = ?1",
        [version_id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    // Date de sauvegarde à la seconde près : on prend toute la seconde.
    let list = captures(conn, &als_path, Some(saved * 1000 + 999))?;
    if !list.iter().any(|c| parent_saved.is_none_or(|p| c.started_at >= p * 1000)) {
        return Ok(false);
    }
    let Some(a) = assemble(&list)? else { return Ok(false) };
    let source = if a.render { "render" } else { "capture" };
    store(conn, vault, &a.pcm, source, Some(a.tempo), now, |conn, hash| {
        conn.execute("UPDATE versions SET preview_hash = ?2 WHERE id = ?1", params![version_id, hash])?;
        Ok(())
    })?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::tests::write_tone;
    use crate::db;

    /// Positions lues toutes les 50 ms pendant `seconds`, à partir de `beat`.
    fn positions(beat: f64, seconds: f64, tempo: f64) -> Vec<(f64, f64)> {
        (0..=(seconds / POLL) as usize)
            .map(|i| {
                let ms = i as f64 * POLL * 1000.0;
                (ms, beat + ms / 1000.0 * tempo / 60.0)
            })
            .collect()
    }

    #[test]
    fn runs_follow_loops_and_jumps() {
        // 120 BPM : 2 beats par seconde. Lecture depuis le beat 8 (4 s),
        // puis boucle qui revient au beat 8 au bout d'une seconde.
        let mut p = positions(8.0, 1.0, 120.0);
        let looped: Vec<_> = positions(8.0, 1.0, 120.0).into_iter().map(|(ms, b)| (ms + 1050.0, b)).collect();
        p.extend(looped);
        let r = runs(&p, 120.0);
        assert_eq!(r.len(), 2);
        assert!((r[0].at - 4.0).abs() < 1e-9 && r[0].from == 0.0);
        assert!((r[0].to - 1.05).abs() < 1e-9, "{:?}", r[0]);
        assert!((r[1].at - 4.0).abs() < 1e-9 && (r[1].from - 1.05).abs() < 1e-9);

        // Une position lue en retard de 20 ms ne casse rien et décale à peine.
        let mut p = positions(0.0, 2.0, 120.0);
        p[10].1 -= 0.04;
        let r = runs(&p, 120.0);
        assert_eq!(r.len(), 1);
        assert!(r[0].at.abs() < 0.001);
    }

    #[test]
    fn uncover_counts_new_parts() {
        let mut c = vec![];
        assert_eq!(uncover(&mut c, 0.0, 10.0), 10.0);
        assert_eq!(uncover(&mut c, 5.0, 15.0), 5.0);
        assert_eq!(uncover(&mut c, 2.0, 12.0), 0.0);
        assert_eq!(c, [(0.0, 15.0)]);
    }

    struct Fixture {
        dir: tempfile::TempDir,
        conn: Connection,
        vault: Vault,
        set: SetRef,
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let conn = db::open(&dir.path().join("library.db")).unwrap();
        let vault = Vault::new(dir.path().join("coffre"));
        let als = dir.path().join("woodkid Project/woodkid.als").to_string_lossy().into_owned();
        conn.execute(
            "INSERT INTO projects (folder, als_path, name, tempo, modified_at, scanned_at)
             VALUES ('x', ?1, 'woodkid', 120, 0, 0)",
            [&als],
        )
        .unwrap();
        let set = SetRef { als_path: als, project_id: conn.last_insert_rowid(), branch_id: None };
        Fixture { dir, conn, vault, set }
    }

    fn capture(f: &Fixture, name: &str, started_at: i64, beat: f64, seconds: f64, freq: f64) -> NewCapture {
        let path = f.dir.path().join(name);
        write_tone(&path, 8000, seconds, freq);
        NewCapture {
            path,
            started_at,
            tempo: 120.0,
            song_end: 32.0,
            positions: positions(beat, seconds - POLL, 120.0),
            render: false,
            modified: true,
        }
    }

    #[test]
    fn captures_are_assembled_by_position() {
        let f = fixture();
        // Écoute des mesures 1 à 4 (beats 0 à 16 = 8 s), puis du beat 8 au beat 12.
        let first = capture(&f, "a.wav", 1_000, 0.0, 8.0, 440.0);
        let h1 = add_capture(&f.conn, &f.vault, &f.set, &first, 10).unwrap().unwrap();
        let p1 = info(&f.conn, &h1).unwrap().unwrap();
        assert!((p1.duration - 8.0).abs() < 0.06, "{}", p1.duration);
        assert_eq!(p1.source, "capture");
        assert_eq!(p1.tempo, Some(120.0));

        let second = capture(&f, "b.wav", 2_000, 8.0, 2.0, 220.0);
        let h2 = add_capture(&f.conn, &f.vault, &f.set, &second, 11).unwrap().unwrap();
        assert_ne!(h1, h2);
        let project: String =
            f.conn.query_row("SELECT preview_hash FROM projects", [], |r| r.get(0)).unwrap();
        assert_eq!(project, h2);
        let p2 = info(&f.conn, &h2).unwrap().unwrap();
        assert!((p2.duration - 8.0).abs() < 0.06, "le morceau garde sa longueur");
        let n: i64 = f.conn.query_row("SELECT count(*) FROM captures", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 2, "la première écoute couvre encore le reste");

        // Une écoute complète plus récente rend les deux autres inutiles.
        let third = capture(&f, "c.wav", 3_000, 0.0, 8.0, 330.0);
        add_capture(&f.conn, &f.vault, &f.set, &third, 12).unwrap().unwrap();
        let paths: Vec<String> = f
            .conn
            .prepare("SELECT path FROM captures")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(paths.len(), 1);
        assert!(paths[0].ends_with("c.wav"));
        assert!(!f.dir.path().join("a.wav").exists(), "fichier supprimé");
    }

    #[test]
    fn assembly_places_audio_at_its_beat() {
        let f = fixture();
        // Le beat 8 à 120 BPM tombe à 4 s : 4 s de silence avant.
        let c = capture(&f, "a.wav", 1_000, 8.0, 2.0, 440.0);
        add_capture(&f.conn, &f.vault, &f.set, &c, 10).unwrap();
        let list = captures(&f.conn, &f.set.als_path, None).unwrap();
        let a = assemble(&list).unwrap().unwrap();
        assert_eq!(a.pcm.rate, 8000);
        assert!(a.pcm.left[..31_900].iter().all(|&s| s == 0));
        assert!(a.pcm.left[32_100..32_200].iter().any(|&s| s.abs() > 1000));
        assert!((a.pcm.seconds() - 6.0).abs() < 0.06);
    }

    #[test]
    fn versions_get_the_captures_made_before_their_save() {
        let f = fixture();
        let als = PathBuf::from(&f.set.als_path);
        fs::create_dir_all(als.parent().unwrap()).unwrap();
        fs::write(&als, br#"<?xml version="1.0" encoding="UTF-8"?><Ableton Creator="Ableton Live 12.2"><LiveSet /></Ableton>"#).unwrap();
        versions::enable(&f.conn, &f.vault, f.set.project_id, false, 4_000_000_000).unwrap();
        let (branch, v1): (i64, i64) =
            f.conn.query_row("SELECT id, head_id FROM branches", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        let saved: i64 = f.conn.query_row("SELECT created_at FROM versions", [], |r| r.get(0)).unwrap();
        let set = SetRef { branch_id: Some(branch), ..f.set.clone() };

        // Écoute avant la sauvegarde : rattachée à la version à sa création.
        let before = capture(&f, "a.wav", saved * 1000 - 60_000, 0.0, 4.0, 440.0);
        add_capture(&f.conn, &f.vault, &set, &before, 10).unwrap();
        let none: Option<String> =
            f.conn.query_row("SELECT preview_hash FROM versions", [], |r| r.get(0)).unwrap();
        assert_eq!(none, None, "écoute antérieure à la version : pas pour elle");
        assert!(on_new_version(&f.conn, &f.vault, v1, 10).unwrap());

        // Écoute du set sauvegardé (sans « * ») : elle remplace l'aperçu de la version.
        let after = NewCapture { modified: false, ..capture(&f, "b.wav", saved * 1000 + 5_000, 0.0, 4.0, 220.0) };
        let h = add_capture(&f.conn, &f.vault, &set, &after, 11).unwrap().unwrap();
        let v: String = f.conn.query_row("SELECT preview_hash FROM versions", [], |r| r.get(0)).unwrap();
        assert_eq!(v, h);
    }

    #[test]
    fn resolves_open_set_and_finds_bounce() {
        let f = fixture();
        let open = |set: &str, project: Option<&str>| OpenSet {
            set: set.into(),
            project: project.map(Into::into),
            modified: false,
        };
        assert_eq!(resolve(&f.conn, &open("Woodkid", Some("woodkid Project"))).unwrap(), Some(f.set.clone()));
        assert_eq!(resolve(&f.conn, &open("woodkid", None)).unwrap(), Some(f.set.clone()));
        assert_eq!(resolve(&f.conn, &open("woodkid", Some("autre Project"))).unwrap(), None);

        let folder = f.dir.path().join("bounces");
        fs::create_dir_all(folder.join("Samples")).unwrap();
        write_tone(&folder.join("export final.wav"), 8000, 4.0, 440.0);
        write_tone(&folder.join("Samples/woodkid.wav"), 8000, 4.0, 440.0);
        assert_eq!(find_bounce(&folder, "woodkid"), Some(folder.join("export final.wav")));
        write_tone(&folder.join("woodkid v2.wav"), 8000, 4.0, 440.0);
        assert_eq!(find_bounce(&folder, "Woodkid"), Some(folder.join("woodkid v2.wav")));

        let hash = import_file(&f.conn, &f.vault, f.set.project_id, &folder.join("woodkid v2.wav"), 5).unwrap();
        let p = info(&f.conn, &hash).unwrap().unwrap();
        assert_eq!((p.source.as_str(), p.tempo), ("file", Some(120.0)));
        assert!(f.vault.contains(&hash));
    }
}
