//! Tests de la porte sur une base d'essai créée par les migrations de l'appli.

use super::*;

struct Essai {
    _dir: tempfile::TempDir,
    db: PathBuf,
}

/// Secondes UTC d'une heure locale « AAAA-MM-JJ HH:MM ».
fn local(c: &Connection, quand: &str) -> i64 {
    c.query_row("SELECT CAST(strftime('%s', ?1, 'utc') AS INTEGER)", [quand], |r| r.get(0)).unwrap()
}

fn essai() -> Essai {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("library.db");
    let c = ableton_manager_lib::ouvrir_base(&db).unwrap();
    c.execute_batch(
        "INSERT INTO roots (id, path) VALUES (1, 'C:\\Musique');
         INSERT INTO projects (id, root_id, folder, als_path, name, tempo, time_sig, live_version, modified_at, scanned_at, status, favorite, note)
           VALUES (1, 1, 'C:\\Musique\\Nuit Project', 'C:\\Musique\\Nuit Project\\Nuit.als', 'Nuit', 124, '4/4', '12.1', 1794321600, 0, 'en_cours', 1, 'à mixer'),
                  (2, 1, 'C:\\Musique\\Aube Project', 'C:\\Musique\\Aube Project\\Aube.als', 'Aube', 90, '4/4', '12.1', 1794300000, 0, 'idee', 0, '');
         INSERT INTO tracks (project_id, position, name, kind) VALUES (1, 0, 'Kick', 'audio'), (1, 1, 'Basse', 'midi');
         INSERT INTO project_items (project_id, kind, name, path) VALUES (1, 'plugin', 'Serum', NULL), (2, 'plugin', 'Serum', NULL),
           (1, 'sample', 'kick.wav', 'C:\\kick.wav');
         INSERT INTO annotations (project_id, beat, label, created_at) VALUES (1, 64, 'drop', 0);
         INSERT INTO branches (id, project_id, name, folder, als_path, is_main, created_at) VALUES (1, 1, 'principale', 'f', 'C:\\Musique\\Nuit Project\\Nuit.als', 1, 0);
         INSERT INTO versions (id, project_id, branch_id, created_at, source, name, auto_name, als_name, als_hash, summary)
           VALUES (1, 1, 1, 1794000000, 'save', NULL, 'Sauvegarde 1', 'Nuit.als', 'h1', '{}'),
                  (2, 1, 1, 1794321600, 'manual', 'Avant mix', 'Sauvegarde 2', 'Nuit.als', 'h2', '{}');
         INSERT INTO todos (id, project_id, text, done, created_at) VALUES (1, 1, 'refaire le kick', 0, 1794321600);",
    )
    .unwrap();
    biblio::refresh_search(&c, 2).unwrap();
    biblio::add_tag(&c, 1, "club", biblio::SOURCE_APP).unwrap();
    // Session à cheval sur minuit (local) : 30 min le 9, 45 min le 10.
    let (s, e) = (local(&c, "2026-11-09 23:30"), local(&c, "2026-11-10 00:45"));
    c.execute("INSERT INTO work_sessions (project_id, started_at, ended_at) VALUES (1, ?1, ?2)", [s, e]).unwrap();
    let (s, e) = (local(&c, "2026-11-10 10:00"), local(&c, "2026-11-10 10:20"));
    c.execute("INSERT INTO work_sessions (project_id, started_at, ended_at) VALUES (2, ?1, ?2)", [s, e]).unwrap();
    // Hors période.
    let (s, e) = (local(&c, "2026-10-01 10:00"), local(&c, "2026-10-01 11:00"));
    c.execute("INSERT INTO work_sessions (project_id, started_at, ended_at) VALUES (2, ?1, ?2)", [s, e]).unwrap();
    Essai { _dir: dir, db }
}

fn ok(e: &Essai, outil: &str, args: Value) -> Value {
    call(&e.db, outil, &args).unwrap_or_else(|err| panic!("{outil} : {err}"))
}

fn est_iso_z(v: &Value) -> bool {
    let s = v.as_str().unwrap_or("");
    s.len() == 20 && s.ends_with('Z') && &s[10..11] == "T"
}

#[test]
fn lister_projets_refs_dates_et_filtres() {
    let e = essai();
    let r = ok(&e, "lister_projets", json!({}));
    assert_eq!(r["total"], 2);
    let p = &r["elements"][0];
    assert_eq!(p["ref"], "am:projet/1");
    assert_eq!(p["als_path"], "C:\\Musique\\Nuit Project\\Nuit.als");
    assert_eq!(p["nom"], "Nuit");
    assert_eq!(p["dossier"], "C:\\Musique\\Nuit Project");
    assert_eq!(p["signature"], "4/4");
    assert_eq!(p["version_live"], "12.1");
    assert_eq!(p["statut"], "en_cours");
    assert_eq!(p["favori"], true);
    assert_eq!(p["modifie_le"], "2026-11-10T14:40:00Z");
    assert_eq!(p["tags"], json!(["club"]));
    assert_eq!(p["taches_ouvertes"], 1);
    assert_eq!(p["minutes_travail"], 75);

    let r = ok(&e, "lister_projets", json!({"recherche": "serum", "limite": 1}));
    assert_eq!((r["total"].as_i64(), r["elements"].as_array().unwrap().len()), (Some(2), 1));
    let r = ok(&e, "lister_projets", json!({"recherche": "club"}));
    assert_eq!(r["total"], 1);
    let r = ok(&e, "lister_projets", json!({"statut": "idee"}));
    assert_eq!(r["elements"][0]["ref"], "am:projet/2");
    assert_eq!(r["elements"][0]["tags"], json!([]));
    assert_eq!(call(&e.db, "lister_projets", &json!({"statut": "x"})).unwrap_err(), "statut inconnu : x (idee, en_cours, fini, abandonne)");
}

#[test]
fn projet_complet() {
    let e = essai();
    let p = ok(&e, "projet", json!({"id": 1}));
    assert_eq!(p["ref"], "am:projet/1");
    assert_eq!(p["pistes"][1], json!({"nom": "Basse", "type": "midi"}));
    assert_eq!(p["elements"].as_array().unwrap().len(), 2);
    let t = &p["taches"][0];
    assert_eq!(t["ref"], "am:tache/1");
    assert_eq!(t["fait"], false);
    assert_eq!(t["source"], "app");
    assert!(est_iso_z(&t["cree_le"]));
    let v = &p["versions"][0];
    assert_eq!(v["ref"], "am:version/2");
    assert_eq!(v["nom"], "Avant mix");
    assert_eq!(v["branche"], "principale");
    assert_eq!(v["le"], "2026-11-10T14:40:00Z");
    assert_eq!(p["annotations"][0]["texte"], "drop");
    assert_eq!(p["minutes_travail"], 75);
    assert_eq!(call(&e.db, "projet", &json!({"id": 99})).unwrap_err(), "projet introuvable");
}

#[test]
fn listes_simples() {
    let e = essai();
    let t = ok(&e, "taches", json!({}));
    assert_eq!(t["total"], 1);
    assert_eq!(
        t["elements"][0],
        json!({"ref": "am:tache/1", "id": 1, "texte": "refaire le kick", "fait": false, "source": "app",
               "projet_ref": "am:projet/1", "projet": "Nuit", "als_path": "C:\\Musique\\Nuit Project\\Nuit.als"})
    );
    let p = ok(&e, "plugins", json!({"nom": "ser"}));
    assert_eq!(p, json!({"elements": [{"nom": "Serum", "projets": 2}], "total": 1}));
    let d = ok(&e, "dossiers_scannes", json!({}));
    assert_eq!(d, json!({"elements": [{"id": 1, "chemin": "C:\\Musique"}], "total": 1}));
}

#[test]
fn temps_de_travail_coupe_a_minuit() {
    let e = essai();
    let r = ok(&e, "temps_de_travail", json!({"debut": "2026-11-01", "fin": "2026-11-10"}));
    assert_eq!(r["periode"], json!({"debut": "2026-11-01", "fin": "2026-11-10"}));
    assert_eq!(r["total_minutes"], 95);
    let jours = r["jours"].as_array().unwrap();
    assert_eq!(jours.len(), 2, "jours sans travail omis : {r}");
    assert_eq!(jours[0]["date"], "2026-11-09");
    assert_eq!(jours[0]["minutes"], 30);
    assert_eq!(
        jours[0]["projets"],
        json!([{"ref": "am:projet/1", "nom": "Nuit", "als_path": "C:\\Musique\\Nuit Project\\Nuit.als", "minutes": 30}])
    );
    assert_eq!(jours[1]["date"], "2026-11-10");
    assert_eq!(jours[1]["minutes"], 65);
    assert_eq!(jours[1]["projets"][0]["minutes"], 45);
    assert_eq!(jours[1]["projets"][1]["ref"], "am:projet/2");

    // Un seul jour, un seul projet : la session est coupée au début du jour.
    let r = ok(&e, "temps_de_travail", json!({"debut": "2026-11-10", "fin": "2026-11-10", "projet_id": 1}));
    assert_eq!(r["total_minutes"], 45);
    assert!(call(&e.db, "temps_de_travail", &json!({"debut": "10/11/2026", "fin": "2026-11-10"})).unwrap_err().contains("date invalide"));
}

#[test]
fn versions_recentes_filtrees() {
    let e = essai();
    let r = ok(&e, "versions_recentes", json!({}));
    assert_eq!(r["total"], 2);
    assert_eq!(
        r["elements"][0],
        json!({"ref": "am:version/2", "projet_ref": "am:projet/1", "projet": "Nuit", "als_path": "C:\\Musique\\Nuit Project\\Nuit.als",
               "nom": "Avant mix", "branche": "principale", "source": "manual", "le": "2026-11-10T14:40:00Z"})
    );
    let r = ok(&e, "versions_recentes", json!({"depuis": "2026-11-10T00:00:00Z"}));
    assert_eq!(r["total"], 1);
    let r = ok(&e, "versions_recentes", json!({"projet_id": 2}));
    assert_eq!(r["total"], 0);
    let r = ok(&e, "versions_recentes", json!({"limite": 1}));
    assert_eq!((r["total"].as_i64(), r["elements"].as_array().unwrap().len()), (Some(2), 1));
}

fn changements(e: &Essai) -> Vec<(String, String, String)> {
    let c = Connection::open(&e.db).unwrap();
    let mut st = c.prepare("SELECT objet, action, source FROM changements_externes ORDER BY id").unwrap();
    st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap().map(Result::unwrap).collect()
}

#[test]
fn ecritures_notees_ia() {
    let e = essai();
    let r = ok(&e, "ajouter_tache", json!({"projet_id": 2, "texte": " poser la voix "}));
    assert_eq!(r["projet_ref"], "am:projet/2");
    assert_eq!(r["texte"], "poser la voix");
    assert_eq!(r["source"], "ia");
    let tref = r["ref"].as_str().unwrap().to_string();
    let tid: i64 = tref.strip_prefix("am:tache/").unwrap().parse().unwrap();

    let t = ok(&e, "taches", json!({}));
    let ia = t["elements"].as_array().unwrap().iter().find(|t| t["ref"] == tref.as_str()).unwrap().clone();
    assert_eq!(ia["source"], "ia");

    assert_eq!(ok(&e, "cocher_tache", json!({"id": tid})), json!({"ref": tref, "fait": true, "source": "ia"}));
    assert_eq!(ok(&e, "cocher_tache", json!({"id": tid, "fait": false}))["fait"], false);
    assert_eq!(call(&e.db, "cocher_tache", &json!({"id": 999})).unwrap_err(), "tâche introuvable");

    let r = ok(&e, "changer_statut", json!({"projet_id": 2, "statut": "fini"}));
    assert_eq!(r, json!({"ref": "am:projet/2", "als_path": "C:\\Musique\\Aube Project\\Aube.als", "statut": "fini", "source": "ia"}));
    assert_eq!(ok(&e, "projet", json!({"id": 2}))["statut"], "fini");
    assert_eq!(
        call(&e.db, "changer_statut", &json!({"projet_id": 2, "statut": "brouillon"})).unwrap_err(),
        "statut inconnu : brouillon (idee, en_cours, fini, abandonne)"
    );
    assert_eq!(call(&e.db, "changer_statut", &json!({"projet_id": 9, "statut": "fini"})).unwrap_err(), "projet introuvable");

    let r = ok(&e, "ajouter_tag", json!({"projet_id": 1, "tag": "techno"}));
    assert_eq!(r["tags"], json!(["club", "techno"]));
    assert_eq!(r["source"], "ia");
    assert_eq!(ok(&e, "lister_projets", json!({"recherche": "techno"}))["total"], 1);

    let ch = changements(&e);
    let actions: Vec<&str> = ch.iter().map(|c| c.1.as_str()).collect();
    assert_eq!(actions, ["ajout", "cocher", "decocher", "statut", "tag"]);
    assert!(ch.iter().all(|c| c.2 == "ia"));
}

#[test]
fn base_pas_migree_message_clair() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("vieille.db");
    let c = Connection::open(&db).unwrap();
    c.execute_batch("CREATE TABLE projects (id INTEGER PRIMARY KEY); CREATE TABLE todos (id INTEGER PRIMARY KEY);").unwrap();
    drop(c);
    for outil in ["lister_projets", "ajouter_tache"] {
        let err = call(&db, outil, &json!({"projet_id": 1, "texte": "x"})).unwrap_err();
        assert!(err.contains("ouvre Ableton Manager une fois pour mettre la base à jour"), "{err}");
    }
    let absente = dir.path().join("absente.db");
    assert!(call(&absente, "taches", &json!({})).unwrap_err().contains("base introuvable"));
    assert!(!absente.exists(), "la porte ne crée jamais la base");
}

#[test]
fn protocole_annotations_et_structured_content() {
    let e = essai();
    let r = handle(&e.db, &json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}})).unwrap();
    assert_eq!(r["result"]["serverInfo"]["name"], "ableton-manager");

    let r = handle(&e.db, &json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"})).unwrap();
    let outils = r["result"]["tools"].as_array().unwrap();
    let noms: Vec<&str> = outils.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(
        noms,
        ["lister_projets", "projet", "taches", "plugins", "dossiers_scannes", "temps_de_travail", "versions_recentes",
         "ajouter_tache", "cocher_tache", "changer_statut", "ajouter_tag", "ouvrir_projet", "apercu_audio"]
    );
    let ann = |n: &str| outils.iter().find(|t| t["name"] == n).unwrap()["annotations"].clone();
    for n in &noms[..7] {
        assert_eq!(ann(n), json!({"readOnlyHint": true, "destructiveHint": false}), "{n}");
    }
    for n in &noms[7..11] {
        assert_eq!(ann(n), json!({"readOnlyHint": false, "destructiveHint": false}), "{n}");
    }
    for n in &noms[11..] {
        assert_eq!(ann(n), json!({"readOnlyHint": false, "destructiveHint": true}), "{n}");
    }

    let r = handle(&e.db, &json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call",
        "params": {"name": "dossiers_scannes", "arguments": {}}})).unwrap();
    let res = &r["result"];
    let texte: Value = serde_json::from_str(res["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(res["content"][0]["type"], "text");
    assert_eq!(texte, res["structuredContent"]);
    assert!(res.get("isError").is_none());

    let r = handle(&e.db, &json!({"jsonrpc": "2.0", "id": 4, "method": "tools/call",
        "params": {"name": "projet", "arguments": {"id": 42}}})).unwrap();
    assert_eq!(r["result"]["isError"], true);
    assert_eq!(r["result"]["content"][0]["text"], "projet introuvable");
}
