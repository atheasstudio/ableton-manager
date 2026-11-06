//! Serveur MCP en stdio : un agent (IA locale) lit la base
//! d'Ableton Manager. Lecture seule, sauf ajouter / cocher une tâche.
//!
//! Base : `ABLETON_MANAGER_DB`, sinon `%APPDATA%\studio.atheas.ableton-manager\library.db`.

use std::io::{BufRead, Write};
use std::path::PathBuf;

use rusqlite::{params, types::ValueRef, Connection, OpenFlags};
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

fn open(write: bool) -> Result<Connection, String> {
    let flags = if write {
        OpenFlags::SQLITE_OPEN_READ_WRITE
    } else {
        OpenFlags::SQLITE_OPEN_READ_ONLY
    };
    let path = db_path();
    let conn = Connection::open_with_flags(&path, flags | OpenFlags::SQLITE_OPEN_NO_MUTEX)
        .map_err(|e| format!("base introuvable ({}) : {e}", path.display()))?;
    conn.busy_timeout(std::time::Duration::from_secs(5)).ok();
    Ok(conn)
}

/// Exécute une requête et rend chaque ligne en objet JSON.
fn rows(conn: &Connection, sql: &str, args: &[&dyn rusqlite::ToSql]) -> Result<Value, String> {
    let mut st = conn.prepare(sql).map_err(|e| e.to_string())?;
    let cols: Vec<String> = st.column_names().iter().map(|s| s.to_string()).collect();
    let mut out = Vec::new();
    let mut q = st.query(args).map_err(|e| e.to_string())?;
    while let Some(r) = q.next().map_err(|e| e.to_string())? {
        let mut m = Map::new();
        for (i, c) in cols.iter().enumerate() {
            let v = match r.get_ref(i).map_err(|e| e.to_string())? {
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
    Ok(Value::Array(out))
}

fn tools() -> Value {
    let s = |props: Value, req: &[&str]| json!({"type":"object","properties":props,"required":req});
    json!([
      {"name":"lister_projets","description":"Liste les projets Ableton (nom, chemin du .als, dossier, tempo, statut, favori, tags, date de modif, tâches ouvertes, temps passé). Filtre plein texte optionnel (nom, note, plugins, samples, tags) et par statut.",
       "inputSchema": s(json!({"recherche":{"type":"string"},"statut":{"type":"string"},"limite":{"type":"integer"}}), &[])},
      {"name":"projet","description":"Tout sur un projet : infos, pistes, plugins, samples, tags, tâches, versions, annotations, temps de travail.",
       "inputSchema": s(json!({"id":{"type":"integer"}}), &["id"])},
      {"name":"taches","description":"Tâches (to-do) de tous les projets. Par défaut seulement celles à faire.",
       "inputSchema": s(json!({"toutes":{"type":"boolean"}}), &[])},
      {"name":"plugins","description":"Plugins utilisés, avec le nombre de projets qui les utilisent. Filtre optionnel sur le nom.",
       "inputSchema": s(json!({"nom":{"type":"string"}}), &[])},
      {"name":"dossiers_scannes","description":"Dossiers racines scannés par l'appli.","inputSchema": s(json!({}), &[])},
      {"name":"ouvrir_projet","description":"Ouvre le .als du projet dans Ableton Live (comme un double-clic).",
       "inputSchema": s(json!({"id":{"type":"integer"}}), &["id"])},
      {"name":"apercu_audio","description":"Copie le dernier aperçu audio (MP3) du projet dans un dossier temporaire et renvoie son chemin, pour l'écouter ou l'envoyer.",
       "inputSchema": s(json!({"id":{"type":"integer"},"ouvrir":{"type":"boolean","description":"le lire tout de suite"}}), &["id"])},
      {"name":"ajouter_tache","description":"Ajoute une tâche à un projet.",
       "inputSchema": s(json!({"projet_id":{"type":"integer"},"texte":{"type":"string"}}), &["projet_id","texte"])},
      {"name":"cocher_tache","description":"Marque une tâche faite (ou à refaire avec fait=false).",
       "inputSchema": s(json!({"id":{"type":"integer"},"fait":{"type":"boolean"}}), &["id"])}
    ])
}

const PROJ_COLS: &str = "p.id, p.name, p.als_path, p.folder, p.tempo, p.time_sig, p.live_version, p.status, p.favorite, p.note,
  datetime(p.modified_at,'unixepoch') AS modifie_le,
  (SELECT group_concat(t.name, ', ') FROM project_tags pt JOIN tags t ON t.id=pt.tag_id WHERE pt.project_id=p.id) AS tags,
  (SELECT count(*) FROM todos d WHERE d.project_id=p.id AND d.done=0) AS taches_ouvertes,
  (SELECT sum(ended_at-started_at)/60 FROM work_sessions w WHERE w.project_id=p.id) AS minutes_travail";

fn call(name: &str, a: &Value) -> Result<Value, String> {
    let s = |k: &str| a.get(k).and_then(Value::as_str).map(str::to_string);
    let i = |k: &str| a.get(k).and_then(Value::as_i64);
    match name {
        "lister_projets" => {
            let c = open(false)?;
            let lim = i("limite").unwrap_or(500);
            let statut = s("statut");
            match s("recherche").filter(|x| !x.trim().is_empty()) {
                Some(q) => {
                    let q = q.split_whitespace().map(|w| format!("\"{}\"*", w.replace('"', ""))).collect::<Vec<_>>().join(" ");
                    rows(&c, &format!("SELECT {PROJ_COLS} FROM search JOIN projects p ON p.id=search.rowid
                        WHERE search MATCH ?1 AND (?2 IS NULL OR p.status=?2) ORDER BY p.modified_at DESC LIMIT ?3"),
                        &[&q, &statut, &lim])
                }
                None => rows(&c, &format!("SELECT {PROJ_COLS} FROM projects p WHERE (?1 IS NULL OR p.status=?1)
                    ORDER BY p.modified_at DESC LIMIT ?2"), &[&statut, &lim]),
            }
        }
        "projet" => {
            let c = open(false)?;
            let id = i("id").ok_or("id manquant")?;
            let mut p = rows(&c, &format!("SELECT {PROJ_COLS} FROM projects p WHERE p.id=?1"), &[&id])?;
            let mut p = p.as_array_mut().and_then(|v| v.pop()).ok_or("projet introuvable")?;
            let o = p.as_object_mut().unwrap();
            o.insert("pistes".into(), rows(&c, "SELECT name, kind FROM tracks WHERE project_id=?1 ORDER BY position", &[&id])?);
            o.insert("elements".into(), rows(&c, "SELECT kind, name, path FROM project_items WHERE project_id=?1", &[&id])?);
            o.insert("taches".into(), rows(&c, "SELECT id, text, done, datetime(created_at,'unixepoch') AS cree_le FROM todos WHERE project_id=?1 ORDER BY done, id", &[&id])?);
            o.insert("versions".into(), rows(&c, "SELECT v.id, b.name AS branche, coalesce(v.name, v.auto_name) AS nom, v.source, v.tempo, datetime(v.created_at,'unixepoch') AS le
                FROM versions v JOIN branches b ON b.id=v.branch_id WHERE v.project_id=?1 ORDER BY v.created_at DESC", &[&id])?);
            o.insert("annotations".into(), rows(&c, "SELECT beat, label FROM annotations WHERE project_id=?1 ORDER BY beat", &[&id])?);
            Ok(p)
        }
        "taches" => {
            let c = open(false)?;
            let all = a.get("toutes").and_then(Value::as_bool).unwrap_or(false) as i64;
            rows(&c, "SELECT d.id, d.text, d.done, p.id AS projet_id, p.name AS projet, p.als_path FROM todos d JOIN projects p ON p.id=d.project_id
                WHERE ?1=1 OR d.done=0 ORDER BY p.modified_at DESC, d.id", &[&all])
        }
        "plugins" => {
            let c = open(false)?;
            let f = format!("%{}%", s("nom").unwrap_or_default());
            rows(&c, "SELECT name, count(DISTINCT project_id) AS projets FROM project_items WHERE kind='plugin' AND name LIKE ?1
                GROUP BY name ORDER BY projets DESC, name", &[&f])
        }
        "dossiers_scannes" => rows(&open(false)?, "SELECT id, path FROM roots", &[]),
        "ouvrir_projet" => {
            let c = open(false)?;
            let als: String = c.query_row("SELECT als_path FROM projects WHERE id=?1", [i("id").ok_or("id manquant")?], |r| r.get(0))
                .map_err(|_| "projet introuvable".to_string())?;
            open_file(&als)?;
            Ok(json!({"ouvert": als}))
        }
        "apercu_audio" => {
            let c = open(false)?;
            let id = i("id").ok_or("id manquant")?;
            let (name, hash): (String, Option<String>) = c.query_row("SELECT name, preview_hash FROM projects WHERE id=?1", [id], |r| Ok((r.get(0)?, r.get(1)?)))
                .map_err(|_| "projet introuvable".to_string())?;
            let hash = hash.ok_or("pas encore d'aperçu pour ce projet")?;
            let mut vault: PathBuf = c.query_row("SELECT value FROM settings WHERE key='vault_dir'", [], |r| r.get::<_, String>(0))
                .map(PathBuf::from).unwrap_or_else(|_| db_path().with_file_name("vault"));
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
            if a.get("ouvrir").and_then(Value::as_bool).unwrap_or(false) { open_file(&dst.to_string_lossy())?; }
            Ok(json!({"fichier": dst}))
        }
        "ajouter_tache" => {
            let c = open(true)?;
            let pid = i("projet_id").ok_or("projet_id manquant")?;
            let t = s("texte").filter(|t| !t.trim().is_empty()).ok_or("texte manquant")?;
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
            c.execute("INSERT INTO todos (project_id, text, created_at) VALUES (?1, ?2, ?3)", params![pid, t.trim(), now])
                .map_err(|e| e.to_string())?;
            Ok(json!({"id": c.last_insert_rowid()}))
        }
        "cocher_tache" => {
            let c = open(true)?;
            let fait = a.get("fait").and_then(Value::as_bool).unwrap_or(true);
            let n = c.execute("UPDATE todos SET done=?1 WHERE id=?2", params![fait, i("id").ok_or("id manquant")?])
                .map_err(|e| e.to_string())?;
            if n == 0 { return Err("tâche introuvable".into()); }
            Ok(json!({"ok": true}))
        }
        _ => Err(format!("outil inconnu : {name}")),
    }
}

/// Ouvre un fichier avec le programme associé (Live pour un .als).
fn open_file(path: &str) -> Result<(), String> {
    #[cfg(windows)]
    let r = std::process::Command::new("cmd").args(["/C", "start", "", path]).spawn();
    #[cfg(not(windows))]
    let r = std::process::Command::new("xdg-open").arg(path).spawn();
    r.map(|_| ()).map_err(|e| e.to_string())
}

fn handle(msg: &Value) -> Option<Value> {
    let id = msg.get("id")?.clone();
    let p = msg.get("params").cloned().unwrap_or(Value::Null);
    let result = match msg.get("method").and_then(Value::as_str).unwrap_or("") {
        "initialize" => json!({
            "protocolVersion": p.get("protocolVersion").cloned().unwrap_or(json!("2024-11-05")),
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "ableton-manager", "version": env!("CARGO_PKG_VERSION")},
            "instructions": "Bibliothèque de projets Ableton d'ethaniel : où sont les projets, statuts, tâches, versions, plugins."
        }),
        "ping" => json!({}),
        "tools/list" => json!({"tools": tools()}),
        "tools/call" => {
            let name = p.get("name").and_then(Value::as_str).unwrap_or("");
            let args = p.get("arguments").cloned().unwrap_or(json!({}));
            match call(name, &args) {
                Ok(v) => json!({"content":[{"type":"text","text": serde_json::to_string_pretty(&v).unwrap()}]}),
                Err(e) => json!({"content":[{"type":"text","text": e}], "isError": true}),
            }
        }
        m => return Some(json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":format!("méthode inconnue : {m}")}})),
    };
    Some(json!({"jsonrpc":"2.0","id":id,"result":result}))
}

fn main() {
    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    for line in stdin.lock().lines().map_while(Result::ok) {
        let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
        if let Some(r) = handle(&msg) {
            let _ = writeln!(out, "{r}");
            let _ = out.flush();
        }
    }
}
