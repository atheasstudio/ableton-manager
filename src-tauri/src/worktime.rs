//! Temps passé sur chaque projet : un fil regarde régulièrement quel set est
//! ouvert dans Live (titre de sa fenêtre) et prolonge la session de travail
//! du projet. Le temps ne compte que si quelqu'un est là : clavier ou souris
//! utilisés récemment, ou Live en train de jouer.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::live::Live;
use crate::{db, live_window, previews, AppState};

/// Intervalle entre deux relevés.
const TICK: Duration = Duration::from_secs(20);
/// Au-delà de cet écart (secondes) sans relevé, une nouvelle session commence.
pub const GAP: i64 = 180;
/// Sans clavier ni souris depuis ce délai (secondes), on considère que
/// personne ne travaille, sauf si Live joue.
const IDLE: u64 = 300;
/// Réglage : "0" coupe le suivi.
pub const ENABLED: &str = "worktime_enabled";

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

#[derive(Clone, Serialize)]
struct WorktimeChanged {
    project_id: i64,
    /// Une session vient de commencer (la liste des projets peut changer d'ordre).
    new_session: bool,
}

/// Lance le suivi.
pub fn start(app: AppHandle) {
    std::thread::spawn(move || loop {
        std::thread::sleep(TICK);
        if let Err(e) = tick(&app) {
            eprintln!("suivi du temps : {e}");
        }
    });
}

fn tick(app: &AppHandle) -> Result<(), String> {
    let Some(open) = live_window::current_set() else { return Ok(()) };
    let state = app.state::<AppState>();
    let conn = state.open_db().map_err(|e| e.to_string())?;
    if !enabled(&conn).map_err(|e| e.to_string())? {
        return Ok(());
    }
    if idle_seconds() >= IDLE && !live_playing(app) {
        return Ok(());
    }
    let Some(set) = previews::resolve(&conn, &open).map_err(|e| e.to_string())? else {
        return Ok(());
    };
    let new_session = record(&conn, set.project_id, set.branch_id, now()).map_err(|e| e.to_string())?;
    let _ = app.emit("worktime-changed", WorktimeChanged { project_id: set.project_id, new_session });
    Ok(())
}

fn live_playing(app: &AppHandle) -> bool {
    let live = app.state::<Live>();
    live.script_version().is_some() && live.transport().is_ok_and(|t| t.playing)
}

pub fn enabled(conn: &Connection) -> db::Result<bool> {
    Ok(db::get_setting(conn, ENABLED)?.as_deref() != Some("0"))
}

pub fn set_enabled(conn: &Connection, on: bool) -> db::Result<()> {
    db::set_setting(conn, ENABLED, if on { "1" } else { "0" })
}

/// Note que le projet est travaillé à `now` : prolonge sa dernière session si
/// elle est récente, sinon en ouvre une. Renvoie vrai si une session commence.
pub fn record(conn: &Connection, project_id: i64, branch_id: Option<i64>, now: i64) -> db::Result<bool> {
    let last: Option<i64> = conn
        .query_row(
            "SELECT id FROM work_sessions WHERE project_id = ?1 AND ended_at >= ?2
             ORDER BY ended_at DESC LIMIT 1",
            params![project_id, now - GAP],
            |r| r.get(0),
        )
        .optional()?;
    match last {
        Some(id) => {
            conn.execute(
                "UPDATE work_sessions SET ended_at = max(ended_at, ?2) WHERE id = ?1",
                params![id, now],
            )?;
            Ok(false)
        }
        None => {
            conn.execute(
                "INSERT INTO work_sessions (project_id, branch_id, started_at, ended_at)
                 VALUES (?1, ?2, ?3, ?3)",
                params![project_id, branch_id, now],
            )?;
            Ok(true)
        }
    }
}

#[derive(Debug, Serialize)]
pub struct Session {
    pub id: i64,
    pub started_at: i64,
    pub ended_at: i64,
    /// Nom de la branche, si ce n'est pas la principale.
    pub branch: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ProjectTime {
    /// Secondes, depuis toujours.
    pub total: i64,
    /// Secondes sur les 7 et 30 derniers jours.
    pub week: i64,
    pub month: i64,
    pub sessions: i64,
    pub first: Option<i64>,
    pub last: Option<i64>,
    /// Secondes par jour (date locale « AAAA-MM-JJ »), sur les 12 dernières semaines.
    pub days: Vec<(String, i64)>,
    /// Les plus récentes d'abord.
    pub recent: Vec<Session>,
}

pub fn project_time(conn: &Connection, project_id: i64, now: i64) -> db::Result<ProjectTime> {
    let sum_since = |since: i64| -> db::Result<i64> {
        conn.query_row(
            "SELECT COALESCE(SUM(ended_at - max(started_at, ?2)), 0) FROM work_sessions
             WHERE project_id = ?1 AND ended_at > ?2",
            params![project_id, since],
            |r| r.get(0),
        )
    };
    let (sessions, first, last) = conn.query_row(
        "SELECT COUNT(*), MIN(started_at), MAX(ended_at) FROM work_sessions WHERE project_id = ?1",
        [project_id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;

    // Une session est rangée au jour où elle commence : assez juste pour un
    // aperçu, sans découper les sessions qui passent minuit.
    let mut stmt = conn.prepare(
        "SELECT date(started_at, 'unixepoch', 'localtime') AS day, SUM(ended_at - started_at)
         FROM work_sessions WHERE project_id = ?1 AND started_at >= ?2
         GROUP BY day ORDER BY day",
    )?;
    let days = stmt
        .query_map(params![project_id, now - 84 * 86_400], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<db::Result<_>>()?;

    let mut stmt = conn.prepare(
        "SELECT s.id, s.started_at, s.ended_at, CASE WHEN b.is_main = 0 THEN b.name END
         FROM work_sessions s LEFT JOIN branches b ON b.id = s.branch_id
         WHERE s.project_id = ?1 ORDER BY s.started_at DESC LIMIT 30",
    )?;
    let recent = stmt
        .query_map([project_id], |r| {
            Ok(Session { id: r.get(0)?, started_at: r.get(1)?, ended_at: r.get(2)?, branch: r.get(3)? })
        })?
        .collect::<db::Result<_>>()?;

    Ok(ProjectTime {
        total: sum_since(0)?,
        week: sum_since(now - 7 * 86_400)?,
        month: sum_since(now - 30 * 86_400)?,
        sessions,
        first,
        last,
        days,
        recent,
    })
}

pub fn delete_session(conn: &Connection, id: i64) -> db::Result<()> {
    conn.execute("DELETE FROM work_sessions WHERE id = ?1", [id])?;
    Ok(())
}

#[derive(Debug, Serialize)]
pub struct TopProject {
    pub id: i64,
    pub name: String,
    pub seconds: i64,
}

#[derive(Debug, Serialize)]
pub struct Overview {
    /// Secondes travaillées sur les 7 derniers jours, tous projets confondus.
    pub week: i64,
    /// Projets les plus travaillés sur 7 jours.
    pub top: Vec<TopProject>,
}

pub fn overview(conn: &Connection, now: i64) -> db::Result<Overview> {
    let since = now - 7 * 86_400;
    let week = conn.query_row(
        "SELECT COALESCE(SUM(ended_at - max(started_at, ?1)), 0) FROM work_sessions WHERE ended_at > ?1",
        [since],
        |r| r.get(0),
    )?;
    let mut stmt = conn.prepare(
        "SELECT p.id, p.name, SUM(s.ended_at - max(s.started_at, ?1)) AS t
         FROM work_sessions s JOIN projects p ON p.id = s.project_id
         WHERE s.ended_at > ?1 GROUP BY p.id ORDER BY t DESC LIMIT 5",
    )?;
    let top = stmt
        .query_map([since], |r| Ok(TopProject { id: r.get(0)?, name: r.get(1)?, seconds: r.get(2)? }))?
        .collect::<db::Result<_>>()?;
    Ok(Overview { week, top })
}

/// Secondes depuis la dernière action au clavier ou à la souris.
#[cfg(windows)]
fn idle_seconds() -> u64 {
    #[repr(C)]
    struct LastInputInfo {
        cb_size: u32,
        dw_time: u32,
    }
    #[link(name = "user32")]
    extern "system" {
        fn GetLastInputInfo(plii: *mut LastInputInfo) -> i32;
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetTickCount() -> u32;
    }
    let mut info = LastInputInfo { cb_size: std::mem::size_of::<LastInputInfo>() as u32, dw_time: 0 };
    // SAFETY : `info` est une structure LASTINPUTINFO valide, cbSize renseigné.
    unsafe {
        if GetLastInputInfo(&mut info) == 0 {
            return 0;
        }
        // Les deux compteurs repartent de zéro après 49 jours : wrapping_sub.
        u64::from(GetTickCount().wrapping_sub(info.dw_time)) / 1000
    }
}

#[cfg(not(windows))]
fn idle_seconds() -> u64 {
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(conn: &Connection, als: &str) -> i64 {
        conn.query_row(
            "INSERT INTO projects (folder, als_path, name, modified_at, scanned_at)
             VALUES ('F:/p', ?1, 'p', 0, 0) RETURNING id",
            [als],
            |r| r.get(0),
        )
        .unwrap()
    }

    #[test]
    fn sessions_extend_then_split() {
        let conn = db::open_in_memory().unwrap();
        let a = project(&conn, "a.als");
        let t0 = 1_000_000;
        assert!(record(&conn, a, None, t0).unwrap(), "première session");
        assert!(!record(&conn, a, None, t0 + 20).unwrap());
        assert!(!record(&conn, a, None, t0 + 40).unwrap());
        // Pause plus longue que GAP : nouvelle session.
        assert!(record(&conn, a, None, t0 + 40 + GAP + 1).unwrap());
        assert!(!record(&conn, a, None, t0 + 40 + GAP + 61).unwrap());

        let t = project_time(&conn, a, t0 + 1000).unwrap();
        assert_eq!(t.sessions, 2);
        assert_eq!(t.total, 40 + 60);
        assert_eq!(t.week, 100);
        assert_eq!(t.first, Some(t0));
        assert_eq!(t.recent.len(), 2);
        assert_eq!(t.recent[0].ended_at - t.recent[0].started_at, 60);
    }

    #[test]
    fn projects_are_separate_and_window_is_clipped() {
        let conn = db::open_in_memory().unwrap();
        let (a, b) = (project(&conn, "a.als"), project(&conn, "b.als"));
        let now = 100 * 86_400;
        // Session de a à cheval sur la limite des 7 jours : 100 s dedans.
        let edge = now - 7 * 86_400;
        record(&conn, a, None, edge - 50).unwrap();
        record(&conn, a, None, edge + 100).unwrap();
        record(&conn, b, None, now - 30).unwrap();
        record(&conn, b, None, now).unwrap();

        let ta = project_time(&conn, a, now).unwrap();
        assert_eq!((ta.total, ta.week), (150, 100));
        let o = overview(&conn, now).unwrap();
        assert_eq!(o.week, 130);
        assert_eq!(o.top[0].id, a);
        assert_eq!(o.top[1].seconds, 30);

        delete_session(&conn, ta.recent[0].id).unwrap();
        assert_eq!(project_time(&conn, a, now).unwrap().total, 0);
    }

    #[test]
    fn can_be_turned_off() {
        let conn = db::open_in_memory().unwrap();
        assert!(enabled(&conn).unwrap());
        set_enabled(&conn, false).unwrap();
        assert!(!enabled(&conn).unwrap());
    }
}
