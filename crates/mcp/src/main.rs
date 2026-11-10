//! Porte MCP (stdio) d'Ableton Manager : une IA lit la bibliothèque et peut
//! faire quelques écritures simples (tâches, statut, tag), toujours notées
//! `source = "ia"` via le crate partagé `biblio` (même code que l'appli).
//!
//! Base : `ABLETON_MANAGER_DB`, sinon `%APPDATA%\studio.atheas.ableton-manager\library.db`.
//! Réponses selon le contrat commun : refs `am:…`, dates ISO UTC en `Z`,
//! listes `{elements, total}`, `structuredContent` identique au texte.

use std::collections::BTreeMap;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use biblio::SOURCE_IA;
use rusqlite::{types::ValueRef, Connection, OpenFlags, OptionalExtension};
use serde_json::{json, Map, Value};

fn db_path() -> PathBuf {
    if let Ok(p) = std::env::var("ABLETON_MANAGER_DB") {
        return p.into();
    }
    let base = std::env::var("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".local/share"));
    base.join("studio.atheas.ableton-manager").join("library.db")
}

type R<T> = Result<T, String>;

/// Ouvre la base (sans jamais la créer) et vérifie qu'elle a déjà la
/// migration de la porte.
fn open(path: &Path, write: bool) -> R<Connection> {
    let flags = if write { OpenFlags::SQLITE_OPEN_READ_WRITE } else { OpenFlags::SQLITE_OPEN_READ_ONLY };
    let conn = Connection::open_with_flags(path, flags | OpenFlags::SQLITE_OPEN_NO_MUTEX).map_err(|e| {
        format!("base introuvable ({}) : {e}. Ouvre Ableton Manager une fois pour créer la base", path.display())
    })?;
    conn.busy_timeout(std::time::Duration::from_secs(5)).ok();
    biblio::verifier_schema(&conn).map_err(|e| e.to_string())?;
    Ok(conn)
}

fn sql(e: rusqlite::Error) -> String {
    format!("base de données : {e}")
}

/// Exécute une requête et rend chaque ligne en objet JSON (clés = noms des colonnes).
fn rows(conn: &Connection, q: &str, args: &[&dyn rusqlite::ToSql]) -> R<Vec<Value>> {
    let mut st = conn.prepare(q).map_err(sql)?;
    let cols: Vec<String> = st.column_names().iter().map(|s| s.to_string()).collect();
    let mut out = Vec::new();
    let mut res = st.query(args).map_err(sql)?;
    while let Some(r) = res.next().map_err(sql)? {
        let mut m = Map::new();
        for (i, c) in cols.iter().enumerate() {
            let v = match r.get_ref(i).map_err(sql)? {
                ValueRef::Null => Value::Null,
                ValueRef::Integer(n) => json!(n),
                ValueRef::Real(f) => json!(f),
                ValueRef::Text(t) => json!(String::from_utf8_lossy(t)),
                ValueRef::Blob(_) => continue,
            };
            m.insert(c.clone(), v);
        }
        out.push(Value::Object(m));
    }
    Ok(out)
}

/// Liste au format du contrat.
fn liste(elements: Vec<Value>, total: Option<i64>) -> Value {
    let total = total.unwrap_or(elements.len() as i64);
    json!({ "elements": elements, "total": total })
}

/// Colonnes SQLite rendues en booléens JSON.
fn en_bool(v: &mut Value, cles: &[&str]) {
    if let Some(o) = v.as_object_mut() {
        for k in cles {
            if let Some(n) = o.get(*k).and_then(Value::as_i64) {
                o.insert((*k).into(), json!(n != 0));
            }
        }
    }
}

/// Séparateur des tags dans `group_concat` (un tag peut contenir une virgule).
const SEP: &str = "\u{1f}";

fn tags_en_liste(v: &mut Value) {
    if let Some(o) = v.as_object_mut() {
        let tags: Vec<Value> = o
            .get("tags")
            .and_then(Value::as_str)
            .map(|s| s.split(SEP).filter(|t| !t.is_empty()).map(|t| json!(t)).collect())
            .unwrap_or_default();
        o.insert("tags".into(), Value::Array(tags));
    }
}

const ISO: &str = "'%Y-%m-%dT%H:%M:%SZ'";

fn proj_cols() -> String {
    format!(
        "'am:projet/' || p.id AS ref, p.id, p.name AS nom, p.als_path, p.folder AS dossier, p.tempo,
  p.time_sig AS signature, p.live_version AS version_live, p.status AS statut, p.favorite AS favori, p.note,
  strftime({ISO}, p.modified_at, 'unixepoch') AS modifie_le,
  (SELECT group_concat(t.name, char(31)) FROM project_tags pt JOIN tags t ON t.id = pt.tag_id WHERE pt.project_id = p.id) AS tags,
  (SELECT count(*) FROM todos d WHERE d.project_id = p.id AND d.done = 0) AS taches_ouvertes,
  (coalesce((SELECT sum(ended_at - started_at) FROM work_sessions w WHERE w.project_id = p.id), 0) + 30) / 60 AS minutes_travail"
    )
}

fn projet_fini(mut v: Value) -> Value {
    en_bool(&mut v, &["favori"]);
    tags_en_liste(&mut v);
    v
}

/// Ref et chemin d'un projet, ou « projet introuvable ».
fn projet_existant(c: &Connection, id: i64) -> R<String> {
    c.query_row("SELECT als_path FROM projects WHERE id = ?1", [id], |r| r.get(0))
        .optional()
        .map_err(sql)?
        .ok_or_else(|| "projet introuvable".to_string())
}

fn valider_statut(s: &str) -> R<()> {
    if biblio::STATUSES.contains(&s) {
        Ok(())
    } else {
        Err(format!("statut inconnu : {s} ({})", biblio::STATUSES.join(", ")))
    }
}

/// Instant ISO (ou date) → secondes UTC.
fn instant(c: &Connection, s: &str) -> R<i64> {
    c.query_row("SELECT CAST(strftime('%s', ?1) AS INTEGER)", [s], |r| r.get::<_, Option<i64>>(0))
        .map_err(sql)?
        .ok_or_else(|| format!("date invalide : {s} (format 2026-11-10T14:20:00Z)"))
}

/// Date calendaire locale « AAAA-MM-JJ » → secondes UTC de son début (minuit local).
fn minuit_local(c: &Connection, jour: &str, decalage_jours: i64) -> R<i64> {
    let ok: Option<String> = c.query_row("SELECT date(?1)", [jour], |r| r.get(0)).map_err(sql)?;
    if ok.as_deref() != Some(jour) {
        return Err(format!("date invalide : {jour} (format AAAA-MM-JJ)"));
    }
    c.query_row(
        "SELECT CAST(strftime('%s', ?1, ?2 || ' days', 'utc') AS INTEGER)",
        rusqlite::params![jour, decalage_jours],
        |r| r.get(0),
    )
    .map_err(sql)
}

/// Découpe [debut, fin[ (secondes UTC) en morceaux par jour local.
fn par_jour(c: &Connection, debut: i64, fin: i64) -> R<Vec<(String, i64)>> {
    let mut out = Vec::new();
    let mut t = debut;
    while t < fin {
        let (jour, minuit): (String, i64) = c
            .query_row(
                "SELECT date(?1, 'unixepoch', 'localtime'),
                        CAST(strftime('%s', date(?1, 'unixepoch', 'localtime'), '+1 day', 'utc') AS INTEGER)",
                [t],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(sql)?;
        let bout = fin.min(minuit.max(t + 1));
        out.push((jour, bout - t));
        t = bout;
    }
    Ok(out)
}

fn minutes(secondes: i64) -> i64 {
    (secondes + 30) / 60
}

fn temps_de_travail(c: &Connection, debut: &str, fin: &str, projet_id: Option<i64>) -> R<Value> {
    let de = minuit_local(c, debut, 0)?;
    let a = minuit_local(c, fin, 1)?;
    if a <= de {
        return Err("période vide : fin doit être le même jour que debut ou après".into());
    }
    if let Some(id) = projet_id {
        projet_existant(c, id)?;
    }
    let mut st = c
        .prepare(
            "SELECT project_id, max(started_at, ?1), min(ended_at, ?2) FROM work_sessions
             WHERE ended_at > ?1 AND started_at < ?2 AND (?3 IS NULL OR project_id = ?3)",
        )
        .map_err(sql)?;
    let sessions: Vec<(i64, i64, i64)> = st
        .query_map(rusqlite::params![de, a, projet_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .map_err(sql)?
        .collect::<rusqlite::Result<_>>()
        .map_err(sql)?;
    // jour → projet → secondes
    let mut jours: BTreeMap<String, BTreeMap<i64, i64>> = BTreeMap::new();
    for (pid, s, e) in sessions {
        for (jour, secs) in par_jour(c, s, e)? {
            *jours.entry(jour).or_default().entry(pid).or_default() += secs;
        }
    }
    let mut infos: BTreeMap<i64, (String, String)> = BTreeMap::new();
    let mut total = 0;
    let mut sortie = Vec::new();
    for (jour, projets) in jours {
        let mut liste_p = Vec::new();
        let mut min_jour = 0;
        for (pid, secs) in projets {
            let m = minutes(secs);
            if m == 0 {
                continue;
            }
            if !infos.contains_key(&pid) {
                let i: (String, String) = c
                    .query_row("SELECT name, als_path FROM projects WHERE id = ?1", [pid], |r| Ok((r.get(0)?, r.get(1)?)))
                    .map_err(sql)?;
                infos.insert(pid, i);
            }
            let (nom, als) = &infos[&pid];
            min_jour += m;
            liste_p.push(json!({"ref": format!("am:projet/{pid}"), "nom": nom, "als_path": als, "minutes": m}));
        }
        if min_jour == 0 {
            continue;
        }
        liste_p.sort_by_key(|p| -p["minutes"].as_i64().unwrap_or(0));
        total += min_jour;
        sortie.push(json!({"date": jour, "minutes": min_jour, "projets": liste_p}));
    }
    Ok(json!({"periode": {"debut": debut, "fin": fin}, "total_minutes": total, "jours": sortie}))
}

/// Description des outils. `lecture`, `modif` ou `destructif` → annotations du contrat.
fn tools() -> Value {
    let s = |props: Value, req: &[&str]| json!({"type":"object","properties":props,"required":req});
    let ann = |genre: &str| match genre {
        "lecture" => json!({"readOnlyHint": true, "destructiveHint": false}),
        "modif" => json!({"readOnlyHint": false, "destructiveHint": false}),
        _ => json!({"readOnlyHint": false, "destructiveHint": true}),
    };
    let statuts = json!(biblio::STATUSES);
    let outils = [
        ("lister_projets", "lecture", "Liste les projets Ableton (ref, nom, chemin du .als, dossier, tempo, statut, favori, tags, date de modif, tâches ouvertes, minutes de travail). Filtre plein texte optionnel (nom, note, plugins, samples, tags) et par statut.",
         s(json!({"recherche":{"type":"string"},"statut":{"type":"string","enum":statuts},"limite":{"type":"integer","description":"500 par défaut"}}), &[])),
        ("projet", "lecture", "Tout sur un projet : infos, pistes, plugins et samples (elements), tâches, versions, annotations, minutes de travail.",
         s(json!({"id":{"type":"integer"}}), &["id"])),
        ("taches", "lecture", "Tâches (to-do) de tous les projets. Par défaut seulement celles à faire.",
         s(json!({"toutes":{"type":"boolean"}}), &[])),
        ("plugins", "lecture", "Plugins utilisés, avec le nombre de projets qui les utilisent. Filtre optionnel sur le nom.",
         s(json!({"nom":{"type":"string"}}), &[])),
        ("dossiers_scannes", "lecture", "Dossiers racines scannés par l'appli.", s(json!({}), &[])),
        ("temps_de_travail", "lecture", "Minutes passées dans Live par jour local et par projet, du jour debut au jour fin inclus. Les jours sans travail sont omis.",
         s(json!({"debut":{"type":"string","description":"AAAA-MM-JJ"},"fin":{"type":"string","description":"AAAA-MM-JJ, inclus"},"projet_id":{"type":"integer"}}), &["debut","fin"])),
        ("versions_recentes", "lecture", "Dernières versions sauvegardées (toutes les branches), les plus récentes d'abord.",
         s(json!({"depuis":{"type":"string","description":"ISO UTC, ex. 2026-11-10T14:20:00Z"},"projet_id":{"type":"integer"},"limite":{"type":"integer","description":"50 par défaut"}}), &[])),
        ("ajouter_tache", "modif", "Ajoute une tâche à un projet (notée « par l'IA » dans l'appli).",
         s(json!({"projet_id":{"type":"integer"},"texte":{"type":"string"}}), &["projet_id","texte"])),
        ("cocher_tache", "modif", "Marque une tâche faite (ou à refaire avec fait=false).",
         s(json!({"id":{"type":"integer"},"fait":{"type":"boolean","description":"true par défaut"}}), &["id"])),
        ("changer_statut", "modif", "Change le statut d'un projet : idee, en_cours, fini ou abandonne.",
         s(json!({"projet_id":{"type":"integer"},"statut":{"type":"string","enum":statuts}}), &["projet_id","statut"])),
        ("ajouter_tag", "modif", "Ajoute un tag à un projet.",
         s(json!({"projet_id":{"type":"integer"},"tag":{"type":"string"}}), &["projet_id","tag"])),
        ("ouvrir_projet", "destructif", "Ouvre le .als du projet dans Ableton Live (comme un double-clic).",
         s(json!({"id":{"type":"integer"}}), &["id"])),
        ("apercu_audio", "destructif", "Copie le dernier aperçu audio (MP3) du projet dans un dossier temporaire et renvoie son chemin, pour l'écouter ou l'envoyer.",
         s(json!({"id":{"type":"integer"},"ouvrir":{"type":"boolean","description":"le lire tout de suite"}}), &["id"])),
    ];
    Value::Array(
        outils
            .into_iter()
            .map(|(name, genre, desc, schema)| json!({"name": name, "description": desc, "inputSchema": schema, "annotations": ann(genre)}))
            .collect(),
    )
}

/// Entier passé en nombre ou en texte.
fn entier(a: &Value, k: &str) -> Option<i64> {
    match a.get(k)? {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn call(db: &Path, name: &str, a: &Value) -> R<Value> {
    let s = |k: &str| a.get(k).and_then(Value::as_str).map(str::to_string).filter(|x| !x.trim().is_empty());
    let i = |k: &str| entier(a, k);
    let id_requis = |k: &str| i(k).ok_or_else(|| format!("{k} manquant"));
    match name {
        "lister_projets" => {
            let c = open(db, false)?;
            let lim = i("limite").unwrap_or(500).max(0);
            let statut = s("statut");
            if let Some(st) = &statut {
                valider_statut(st)?;
            }
            let (from, filtre, q) = match s("recherche") {
                Some(q) => {
                    let q = q.split_whitespace().map(|w| format!("\"{}\"*", w.replace('"', ""))).collect::<Vec<_>>().join(" ");
                    ("search JOIN projects p ON p.id = search.rowid", "search MATCH ?1", Some(q))
                }
                None => ("projects p", "?1 IS NULL", None),
            };
            let total: i64 = c
                .query_row(
                    &format!("SELECT count(*) FROM {from} WHERE {filtre} AND (?2 IS NULL OR p.status = ?2)"),
                    rusqlite::params![q, statut],
                    |r| r.get(0),
                )
                .map_err(sql)?;
            let el = rows(
                &c,
                &format!("SELECT {} FROM {from} WHERE {filtre} AND (?2 IS NULL OR p.status = ?2) ORDER BY p.modified_at DESC LIMIT ?3", proj_cols()),
                &[&q, &statut, &lim],
            )?;
            Ok(liste(el.into_iter().map(projet_fini).collect(), Some(total)))
        }
        "projet" => {
            let c = open(db, false)?;
            let id = id_requis("id")?;
            let p = rows(&c, &format!("SELECT {} FROM projects p WHERE p.id = ?1", proj_cols()), &[&id])?
                .pop()
                .ok_or("projet introuvable")?;
            let mut p = projet_fini(p);
            let o = p.as_object_mut().unwrap();
            o.insert("pistes".into(), json!(rows(&c, "SELECT name AS nom, kind AS type FROM tracks WHERE project_id = ?1 ORDER BY position", &[&id])?));
            o.insert("elements".into(), json!(rows(&c, "SELECT kind AS type, name AS nom, path AS chemin FROM project_items WHERE project_id = ?1 ORDER BY kind, name", &[&id])?));
            let taches = rows(
                &c,
                &format!("SELECT 'am:tache/' || id AS ref, id, text AS texte, done AS fait, source,
                    strftime({ISO}, created_at, 'unixepoch') AS cree_le FROM todos WHERE project_id = ?1 ORDER BY done, id"),
                &[&id],
            )?;
            o.insert("taches".into(), json!(taches.into_iter().map(|mut t| { en_bool(&mut t, &["fait"]); t }).collect::<Vec<_>>()));
            o.insert("versions".into(), json!(rows(
                &c,
                &format!("SELECT 'am:version/' || v.id AS ref, v.id, coalesce(v.name, v.auto_name) AS nom, b.name AS branche, v.source, v.tempo,
                    strftime({ISO}, v.created_at, 'unixepoch') AS le
                    FROM versions v JOIN branches b ON b.id = v.branch_id WHERE v.project_id = ?1 ORDER BY v.created_at DESC, v.id DESC"),
                &[&id],
            )?));
            o.insert("annotations".into(), json!(rows(&c, "SELECT id, beat AS position_beats, label AS texte FROM annotations WHERE project_id = ?1 ORDER BY beat", &[&id])?));
            Ok(p)
        }
        "taches" => {
            let c = open(db, false)?;
            let all = a.get("toutes").and_then(Value::as_bool).unwrap_or(false) as i64;
            let el = rows(
                &c,
                "SELECT 'am:tache/' || d.id AS ref, d.id, d.text AS texte, d.done AS fait, d.source,
                    'am:projet/' || p.id AS projet_ref, p.name AS projet, p.als_path
                 FROM todos d JOIN projects p ON p.id = d.project_id
                 WHERE ?1 = 1 OR d.done = 0 ORDER BY p.modified_at DESC, d.id",
                &[&all],
            )?;
            Ok(liste(el.into_iter().map(|mut t| { en_bool(&mut t, &["fait"]); t }).collect(), None))
        }
        "plugins" => {
            let c = open(db, false)?;
            let f = format!("%{}%", s("nom").unwrap_or_default());
            Ok(liste(rows(&c, "SELECT name AS nom, count(DISTINCT project_id) AS projets FROM project_items WHERE kind = 'plugin' AND name LIKE ?1
                GROUP BY name ORDER BY projets DESC, name", &[&f])?, None))
        }
        "dossiers_scannes" => Ok(liste(rows(&open(db, false)?, "SELECT id, path AS chemin FROM roots ORDER BY path", &[])?, None)),
        "temps_de_travail" => {
            let c = open(db, false)?;
            let debut = s("debut").ok_or("debut manquant (AAAA-MM-JJ)")?;
            let fin = s("fin").ok_or("fin manquant (AAAA-MM-JJ)")?;
            temps_de_travail(&c, debut.trim(), fin.trim(), i("projet_id"))
        }
        "versions_recentes" => {
            let c = open(db, false)?;
            let depuis = match s("depuis") {
                Some(d) => Some(instant(&c, d.trim())?),
                None => None,
            };
            let pid = i("projet_id");
            if let Some(p) = pid {
                projet_existant(&c, p)?;
            }
            let lim = i("limite").unwrap_or(50).max(0);
            let filtre = "(?1 IS NULL OR v.created_at >= ?1) AND (?2 IS NULL OR v.project_id = ?2)";
            let total: i64 = c
                .query_row(&format!("SELECT count(*) FROM versions v WHERE {filtre}"), rusqlite::params![depuis, pid], |r| r.get(0))
                .map_err(sql)?;
            let el = rows(
                &c,
                &format!("SELECT 'am:version/' || v.id AS ref, 'am:projet/' || p.id AS projet_ref, p.name AS projet, p.als_path,
                    coalesce(v.name, v.auto_name) AS nom, b.name AS branche, v.source, strftime({ISO}, v.created_at, 'unixepoch') AS le
                 FROM versions v JOIN projects p ON p.id = v.project_id JOIN branches b ON b.id = v.branch_id
                 WHERE {filtre} ORDER BY v.created_at DESC, v.id DESC LIMIT ?3"),
                &[&depuis, &pid, &lim],
            )?;
            Ok(liste(el, Some(total)))
        }
        "ajouter_tache" => {
            let c = open(db, true)?;
            let pid = id_requis("projet_id")?;
            let t = s("texte").ok_or("texte manquant")?;
            let als = projet_existant(&c, pid)?;
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
            let id = biblio::add_todo(&c, pid, &t, now, SOURCE_IA).map_err(|e| e.to_string())?;
            Ok(json!({"ref": format!("am:tache/{id}"), "projet_ref": format!("am:projet/{pid}"), "als_path": als,
                      "texte": t.trim(), "source": SOURCE_IA}))
        }
        "cocher_tache" => {
            let c = open(db, true)?;
            let id = id_requis("id")?;
            let fait = a.get("fait").and_then(Value::as_bool).unwrap_or(true);
            let existe: Option<i64> = c.query_row("SELECT id FROM todos WHERE id = ?1", [id], |r| r.get(0)).optional().map_err(sql)?;
            existe.ok_or("tâche introuvable")?;
            biblio::set_todo_done(&c, id, fait, SOURCE_IA).map_err(|e| e.to_string())?;
            Ok(json!({"ref": format!("am:tache/{id}"), "fait": fait, "source": SOURCE_IA}))
        }
        "changer_statut" => {
            let c = open(db, true)?;
            let pid = id_requis("projet_id")?;
            let statut = s("statut").ok_or("statut manquant")?;
            valider_statut(&statut)?;
            let als = projet_existant(&c, pid)?;
            biblio::set_status(&c, pid, &statut, SOURCE_IA).map_err(|e| e.to_string())?;
            Ok(json!({"ref": format!("am:projet/{pid}"), "als_path": als, "statut": statut, "source": SOURCE_IA}))
        }
        "ajouter_tag" => {
            let c = open(db, true)?;
            let pid = id_requis("projet_id")?;
            let tag = s("tag").ok_or("tag manquant")?;
            let als = projet_existant(&c, pid)?;
            biblio::add_tag(&c, pid, &tag, SOURCE_IA).map_err(|e| e.to_string())?;
            let mut st = c
                .prepare("SELECT t.name FROM project_tags pt JOIN tags t ON t.id = pt.tag_id WHERE pt.project_id = ?1 ORDER BY t.name COLLATE NOCASE")
                .map_err(sql)?;
            let tags: Vec<String> = st.query_map([pid], |r| r.get(0)).map_err(sql)?.collect::<rusqlite::Result<_>>().map_err(sql)?;
            Ok(json!({"ref": format!("am:projet/{pid}"), "als_path": als, "tags": tags, "source": SOURCE_IA}))
        }
        "ouvrir_projet" => {
            let c = open(db, false)?;
            let id = id_requis("id")?;
            let als = projet_existant(&c, id)?;
            open_file(&als)?;
            Ok(json!({"ref": format!("am:projet/{id}"), "als_path": als, "ouvert": als}))
        }
        "apercu_audio" => {
            let c = open(db, false)?;
            let id = id_requis("id")?;
            let (name, als, hash): (String, String, Option<String>) = c
                .query_row("SELECT name, als_path, preview_hash FROM projects WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
                .optional()
                .map_err(sql)?
                .ok_or("projet introuvable")?;
            let hash = hash.ok_or("pas encore d'aperçu pour ce projet")?;
            let mut vault: PathBuf = c
                .query_row("SELECT value FROM settings WHERE key = 'vault_dir'", [], |r| r.get::<_, String>(0))
                .map(PathBuf::from)
                .unwrap_or_else(|_| db.with_file_name("vault"));
            for _ in 0..8 {
                match std::fs::read_to_string(vault.join("moved-to.txt")) {
                    Ok(n) if !n.trim().is_empty() => vault = PathBuf::from(n.trim()),
                    _ => break,
                }
            }
            let src = vault.join("objects").join(&hash[..2]).join(&hash[2..]);
            let dir = std::env::temp_dir().join("ableton-manager-mcp");
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            let safe: String = name.chars().map(|ch| if r#"\/:*?"<>|"#.contains(ch) { '_' } else { ch }).collect();
            let dst = dir.join(format!("{safe}.mp3"));
            std::fs::copy(&src, &dst).map_err(|e| format!("aperçu introuvable dans le coffre ({}) : {e}", src.display()))?;
            if a.get("ouvrir").and_then(Value::as_bool).unwrap_or(false) {
                open_file(&dst.to_string_lossy())?;
            }
            Ok(json!({"ref": format!("am:projet/{id}"), "als_path": als, "fichier": dst}))
        }
        _ => Err(format!("outil inconnu : {name}")),
    }
}

/// Ouvre un fichier avec le programme associé (Live pour un .als).
fn open_file(path: &str) -> R<()> {
    #[cfg(windows)]
    let r = std::process::Command::new("cmd").args(["/C", "start", "", path]).spawn();
    #[cfg(not(windows))]
    let r = std::process::Command::new("xdg-open").arg(path).spawn();
    r.map(|_| ()).map_err(|e| e.to_string())
}

fn handle(db: &Path, msg: &Value) -> Option<Value> {
    let id = msg.get("id")?.clone();
    let p = msg.get("params").cloned().unwrap_or(Value::Null);
    let result = match msg.get("method").and_then(Value::as_str).unwrap_or("") {
        "initialize" => json!({
            "protocolVersion": p.get("protocolVersion").cloned().unwrap_or(json!("2024-11-05")),
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "ableton-manager", "version": env!("CARGO_PKG_VERSION")},
            "instructions": "Bibliothèque de projets Ableton d'ethaniel : où sont les projets, statuts, tâches, versions, plugins, temps de travail. Les refs am:projet/<id> vont toujours avec als_path."
        }),
        "ping" => json!({}),
        "tools/list" => json!({"tools": tools()}),
        "tools/call" => {
            let name = p.get("name").and_then(Value::as_str).unwrap_or("");
            let args = p.get("arguments").cloned().unwrap_or(json!({}));
            match call(db, name, &args) {
                Ok(v) => json!({"content":[{"type":"text","text": serde_json::to_string_pretty(&v).unwrap()}], "structuredContent": v}),
                Err(e) => json!({"content":[{"type":"text","text": e}], "isError": true}),
            }
        }
        m => return Some(json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":format!("méthode inconnue : {m}")}})),
    };
    Some(json!({"jsonrpc":"2.0","id":id,"result":result}))
}

fn main() {
    let db = db_path();
    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    for line in stdin.lock().lines().map_while(Result::ok) {
        let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
        if let Some(r) = handle(&db, &msg) {
            let _ = writeln!(out, "{r}");
            let _ = out.flush();
        }
    }
}

#[cfg(test)]
mod tests;
