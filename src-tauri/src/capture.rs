//! Liaison avec le device Max for Live « Ableton Manager Capture », posé sur
//! le master : OSC sur UDP, en local.
//!
//! Device → appli (port 47811) :
//! - `/am/hello` toutes les 5 s ; l'appli répond `/am/folder <dossier>` ;
//! - `/am/start n chemin tempo fin rendu` au lancement de la lecture ;
//! - `/am/pos n ms beats` toutes les 50 ms pendant la lecture ;
//! - `/am/stop n` (ou `/am/abort n`) quand le fichier est fermé ;
//! - `/am/error message` si un rendu ne peut pas démarrer.
//!
//! Appli → device (port 47812) : `/am/folder`, `/am/render`, `/am/cancel`.
//!
//! Sans réponse de l'appli, le device n'enregistre rien.

use std::net::UdpSocket;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rosc::{OscMessage, OscPacket, OscType};
use rusqlite::Connection;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::live_window::{self, OpenSet};
use crate::previews::{self, NewCapture};
use crate::vault::Vault;
use crate::AppState;

pub const APP_PORT: u16 = 47811;
pub const DEVICE_PORT: u16 = 47812;

/// Sans `/am/hello` depuis ce délai, le device est considéré absent.
const DEVICE_TIMEOUT: Duration = Duration::from_secs(12);
/// Écoute trop courte pour servir d'aperçu.
const MIN_SECONDS: f64 = 2.0;

pub const DEVICE_NAME: &str = "Ableton Manager Capture.amxd";
const DEVICE_PATCHER: &str = include_str!("../../device/am-capture.maxpat");
const DEVICE_SCRIPT: &str = include_str!("../../device/am-capture.js");

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

/// Écoute en cours d'enregistrement.
#[derive(Debug)]
struct Pending {
    n: i32,
    capture: NewCapture,
    /// Set ouvert dans Live au lancement de la lecture.
    set: Option<OpenSet>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RenderState {
    pub project_id: i64,
    /// Position atteinte et fin du morceau, en beats.
    pub beats: f64,
    pub end: f64,
}

#[derive(Default)]
struct State {
    last_hello: Option<Instant>,
    current: Option<Pending>,
    render: Option<RenderState>,
}

pub struct CaptureLink {
    socket: Option<UdpSocket>,
    folder: PathBuf,
    state: Mutex<State>,
}

#[derive(Clone, Serialize)]
pub struct CaptureStatus {
    /// L'appli écoute le device (port libre).
    pub listening: bool,
    /// Le device a donné signe de vie récemment.
    pub device: bool,
    pub recording: bool,
    /// Set ouvert dans Live, tel qu'affiché dans son titre.
    pub open_set: Option<String>,
    pub render: Option<RenderState>,
}

#[derive(Clone, Serialize)]
struct PreviewChanged {
    project_id: i64,
}

#[derive(Clone, Serialize)]
struct RenderDone {
    project_id: Option<i64>,
    error: Option<String>,
}

fn arg_f64(args: &[OscType], i: usize) -> Option<f64> {
    match args.get(i)? {
        OscType::Int(v) => Some(*v as f64),
        OscType::Float(v) => Some(*v as f64),
        OscType::Double(v) => Some(*v),
        OscType::Long(v) => Some(*v as f64),
        _ => None,
    }
}

fn arg_str(args: &[OscType], i: usize) -> Option<String> {
    match args.get(i)? {
        OscType::String(s) => Some(s.clone()),
        _ => None,
    }
}

/// Ce que l'appli doit faire après un message du device.
#[derive(Debug)]
enum Action {
    None,
    /// Répondre au device avec le dossier des captures.
    SendFolder,
    /// Écoute terminée : à ranger.
    Finished(Pending, Option<RenderState>),
    /// Écoute annulée : fichier à supprimer.
    Aborted(PathBuf, Option<RenderState>),
    RenderProgress(RenderState),
    RenderError(Option<RenderState>, String),
}

impl CaptureLink {
    fn new(folder: PathBuf) -> Self {
        let socket = UdpSocket::bind(("127.0.0.1", APP_PORT))
            .map_err(|e| eprintln!("pré-écoute : port {APP_PORT} indisponible ({e})"))
            .ok();
        Self { socket, folder, state: Mutex::default() }
    }

    /// Lance l'écoute du device.
    pub fn start(app: AppHandle, folder: PathBuf) -> Arc<Self> {
        let _ = std::fs::create_dir_all(&folder);
        let this = Arc::new(Self::new(folder));
        let Some(socket) = this.socket.as_ref().and_then(|s| s.try_clone().ok()) else { return this };
        let link = this.clone();
        std::thread::spawn(move || {
            let mut buf = vec![0u8; 65_536];
            loop {
                let Ok((n, _)) = socket.recv_from(&mut buf) else { continue };
                let Ok((_, packet)) = rosc::decoder::decode_udp(&buf[..n]) else { continue };
                for msg in flatten(packet) {
                    let action = link.handle(&msg, now_ms());
                    link.act(&app, action);
                }
            }
        });
        this
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn send(&self, addr: &str, args: Vec<OscType>) {
        let Some(socket) = &self.socket else { return };
        let packet = OscPacket::Message(OscMessage { addr: addr.into(), args });
        if let Ok(bytes) = rosc::encoder::encode(&packet) {
            let _ = socket.send_to(&bytes, ("127.0.0.1", DEVICE_PORT));
        }
    }

    /// Chemin au format de Max (barres obliques).
    fn folder_for_max(&self) -> String {
        self.folder.to_string_lossy().replace('\\', "/")
    }

    /// Vrai pour un fichier posé directement dans le dossier des captures :
    /// l'appli supprime ces fichiers, elle ne doit pas toucher à un autre
    /// chemin envoyé sur le port (n'importe quel programme du PC peut y écrire).
    fn in_folder(&self, path: &str) -> bool {
        let norm = |s: &str| s.replace('\\', "/").to_lowercase();
        let (folder, path) = (norm(&self.folder_for_max()), norm(path));
        path.strip_prefix(folder.trim_end_matches('/'))
            .and_then(|rest| rest.strip_prefix('/'))
            .is_some_and(|name| !name.is_empty() && !name.contains('/'))
    }

    fn handle(&self, msg: &OscMessage, now: i64) -> Action {
        let a = &msg.args;
        let mut s = self.lock();
        match msg.addr.as_str() {
            "/am/hello" => {
                s.last_hello = Some(Instant::now());
                Action::SendFolder
            }
            "/am/start" => {
                let (Some(n), Some(path), Some(tempo)) = (arg_f64(a, 0), arg_str(a, 1), arg_f64(a, 2)) else {
                    return Action::None;
                };
                if !self.in_folder(&path) {
                    eprintln!("pré-écoute : fichier hors du dossier des captures ignoré ({path})");
                    return Action::None;
                }
                let set = live_window::current_set();
                s.current = Some(Pending {
                    n: n as i32,
                    capture: NewCapture {
                        path: PathBuf::from(path),
                        started_at: now,
                        tempo,
                        song_end: arg_f64(a, 3).unwrap_or(0.0),
                        positions: vec![],
                        render: arg_f64(a, 4).unwrap_or(0.0) != 0.0,
                        modified: set.as_ref().is_some_and(|o| o.modified),
                    },
                    set,
                });
                if let (Some(r), Some(end)) = (s.render.as_mut(), arg_f64(a, 3)) {
                    r.end = end;
                }
                Action::None
            }
            "/am/pos" => {
                let (Some(n), Some(ms), Some(beats)) = (arg_f64(a, 0), arg_f64(a, 1), arg_f64(a, 2)) else {
                    return Action::None;
                };
                let Some(p) = s.current.as_mut().filter(|p| p.n == n as i32) else { return Action::None };
                p.capture.positions.push((ms, beats));
                let (render, count) = (p.capture.render, p.capture.positions.len());
                match s.render.as_mut() {
                    // Une mise à jour par demi-seconde suffit à la barre de progression.
                    Some(r) if render => {
                        r.beats = beats;
                        if count % 10 == 0 {
                            return Action::RenderProgress(r.clone());
                        }
                        Action::None
                    }
                    _ => Action::None,
                }
            }
            "/am/stop" | "/am/abort" => {
                let n = arg_f64(a, 0).map(|n| n as i32);
                let Some(p) = s.current.take_if(|p| Some(p.n) == n) else { return Action::None };
                let render = if p.capture.render { s.render.take() } else { None };
                if msg.addr == "/am/abort" {
                    Action::Aborted(p.capture.path, render)
                } else {
                    Action::Finished(p, render)
                }
            }
            "/am/error" => {
                let render = s.render.take();
                Action::RenderError(render, arg_str(a, 0).unwrap_or_default())
            }
            _ => Action::None,
        }
    }

    fn act(&self, app: &AppHandle, action: Action) {
        match action {
            Action::None => {}
            Action::SendFolder => self.send("/am/folder", vec![OscType::String(self.folder_for_max())]),
            Action::RenderProgress(r) => {
                let _ = app.emit("render-progress", r);
            }
            Action::RenderError(r, error) => {
                let _ = app.emit("render-done", RenderDone { project_id: r.map(|r| r.project_id), error: Some(error) });
            }
            Action::Aborted(path, render) => {
                wait_closed(&path);
                let _ = std::fs::remove_file(path);
                if let Some(r) = render {
                    let error = Some("Rendu annulé".into());
                    let _ = app.emit("render-done", RenderDone { project_id: Some(r.project_id), error });
                }
            }
            Action::Finished(p, render) => {
                // L'encodage prend quelques secondes : hors du fil réseau.
                let app = app.clone();
                std::thread::spawn(move || {
                    let render_project = render.map(|r| r.project_id);
                    let state = app.state::<AppState>();
                    let result = state
                        .open_db()
                        .map_err(|e| e.to_string())
                        .and_then(|conn| {
                            let vault = state.vault(&conn).map_err(|e| e.to_string())?;
                            finish(&conn, &vault, p, now_ms() / 1000).map_err(|e| e.to_string())
                        });
                    match &result {
                        Ok(Some(project_id)) => {
                            let payload = PreviewChanged { project_id: *project_id };
                            let _ = app.emit("preview-changed", payload.clone());
                            let _ = app.emit("versions-changed", payload);
                        }
                        Ok(None) => {}
                        Err(e) => eprintln!("pré-écoute : {e}"),
                    }
                    if render_project.is_some() {
                        let (project_id, error) = match result {
                            Ok(Some(id)) => (Some(id), None),
                            Ok(None) => (render_project, Some("Le set joué n'est pas dans la bibliothèque".into())),
                            Err(e) => (render_project, Some(e)),
                        };
                        let _ = app.emit("render-done", RenderDone { project_id, error });
                    }
                });
            }
        }
    }

    pub fn device_connected(&self) -> bool {
        self.lock().last_hello.is_some_and(|t| t.elapsed() < DEVICE_TIMEOUT)
    }

    fn hello_since(&self, since: Instant) -> bool {
        self.lock().last_hello.is_some_and(|t| t > since)
    }

    pub fn status(&self) -> CaptureStatus {
        let s = self.lock();
        let open = live_window::current_set();
        CaptureStatus {
            listening: self.socket.is_some(),
            device: s.last_hello.is_some_and(|t| t.elapsed() < DEVICE_TIMEOUT),
            recording: s.current.is_some(),
            open_set: open.map(|o| match o.project {
                Some(p) => format!("{} [{p}]", o.set),
                None => o.set,
            }),
            render: s.render.clone(),
        }
    }

    /// Demande au device un rendu complet du set ouvert : lecture du début
    /// à la fin de l'arrangement, enregistrée en temps réel.
    pub fn render(&self, project_id: i64) {
        self.lock().render = Some(RenderState { project_id, beats: 0.0, end: 0.0 });
        self.send("/am/render", vec![]);
    }

    pub fn cancel_render(&self) {
        self.send("/am/cancel", vec![]);
    }
}

fn flatten(packet: OscPacket) -> Vec<OscMessage> {
    match packet {
        OscPacket::Message(m) => vec![m],
        OscPacket::Bundle(b) => b.content.into_iter().flat_map(flatten).collect(),
    }
}

/// Attend que Max ait fini d'écrire le fichier (taille stable).
fn wait_closed(path: &Path) {
    let size = || std::fs::metadata(path).map(|m| m.len()).ok();
    let mut last = size();
    for _ in 0..20 {
        std::thread::sleep(Duration::from_millis(250));
        let now = size();
        if now.is_some() && now == last {
            return;
        }
        last = now;
    }
}

/// Range une écoute terminée. Renvoie le projet dont l'aperçu a changé.
fn finish(conn: &Connection, vault: &Vault, p: Pending, now: i64) -> previews::Result<Option<i64>> {
    let mut c = p.capture;
    wait_closed(&c.path);
    let span = match (c.positions.first(), c.positions.last()) {
        (Some(a), Some(b)) => (b.0 - a.0) / 1000.0,
        _ => 0.0,
    };
    let set = match p.set.or_else(live_window::current_set) {
        Some(open) if span >= MIN_SECONDS => previews::resolve(conn, &open)?,
        _ => None,
    };
    let Some(set) = set else {
        let _ = std::fs::remove_file(&c.path);
        return Ok(None);
    };
    // Rendu arrêté avant la fin : c'est une écoute comme une autre.
    let last_beat = c.positions.last().map_or(0.0, |p| p.1);
    c.render = c.render && c.song_end > 0.0 && last_beat >= c.song_end - 0.5;
    previews::add_capture(conn, vault, &set, &c, now)?;
    Ok(Some(set.project_id))
}

/// Attend que `als` soit le set ouvert dans Live et que le device de ce set
/// se soit annoncé. Ouvre le set au besoin.
pub fn wait_for_set(link: &CaptureLink, als: &str, open: impl FnOnce() -> Result<(), String>) -> Result<(), String> {
    let is_open = || live_window::current_set().is_some_and(|o| previews::matches(als, &o));
    if is_open() {
        return if link.device_connected() {
            Ok(())
        } else {
            Err(NO_DEVICE.into())
        };
    }
    let asked = Instant::now();
    open()?;
    while asked.elapsed() < Duration::from_secs(120) {
        std::thread::sleep(Duration::from_millis(500));
        if is_open() && link.hello_since(asked) {
            return Ok(());
        }
    }
    Err(if is_open() {
        NO_DEVICE.into()
    } else {
        "Le set ne s'est pas ouvert dans Live (une fenêtre de Live attend peut-être une réponse).".into()
    })
}

const NO_DEVICE: &str =
    "Le device « Ableton Manager Capture » ne répond pas : ajoute-le sur le master du set (onglet Pré-écoute des réglages).";

/// Fichier .amxd : en-tête Max for Live (effet audio) puis le patcher JSON.
pub fn amxd(patcher: &str) -> Vec<u8> {
    let mut ptch = patcher.as_bytes().to_vec();
    ptch.extend_from_slice(b"\n\0");
    let mut out = vec![];
    out.extend_from_slice(b"ampf");
    out.extend_from_slice(&4u32.to_le_bytes());
    out.extend_from_slice(b"aaaa");
    out.extend_from_slice(b"meta");
    out.extend_from_slice(&4u32.to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(b"ptch");
    out.extend_from_slice(&(ptch.len() as u32).to_le_bytes());
    out.extend_from_slice(&ptch);
    out
}

/// Installe le device (et son script) dans `dir`. Renvoie le chemin du device.
pub fn install_device(dir: &Path) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let device = dir.join(DEVICE_NAME);
    std::fs::write(&device, amxd(DEVICE_PATCHER))?;
    std::fs::write(dir.join("am-capture.js"), DEVICE_SCRIPT)?;
    Ok(device)
}

/// Dossier des effets Max for Live de la bibliothèque utilisateur de Live,
/// à partir du dossier « User Library ».
pub fn device_dir(user_library: &Path) -> PathBuf {
    user_library.join("Presets").join("Audio Effects").join("Max Audio Effect")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(addr: &str, args: Vec<OscType>) -> OscMessage {
        OscMessage { addr: addr.into(), args }
    }

    #[test]
    fn follows_a_capture() {
        let link = CaptureLink { socket: None, folder: "C:\\captures".into(), state: Mutex::default() };
        assert_eq!(link.folder_for_max(), "C:/captures");
        assert!(matches!(link.handle(&msg("/am/hello", vec![]), 0), Action::SendFolder));
        assert!(link.device_connected());

        link.render(7);
        let start = vec![
            OscType::Int(3),
            OscType::String("C:/captures/cap-1.wav".into()),
            OscType::Float(120.0),
            OscType::Float(64.0),
            OscType::Int(1),
        ];
        link.handle(&msg("/am/start", start), 1_000);
        for i in 0..10 {
            let pos = vec![OscType::Int(3), OscType::Int(i * 50), OscType::Float(i as f32 * 0.1)];
            let action = link.handle(&msg("/am/pos", pos), 0);
            assert_eq!(matches!(action, Action::RenderProgress(_)), i == 9);
        }
        // Une position d'une autre écoute est ignorée.
        link.handle(&msg("/am/pos", vec![OscType::Int(2), OscType::Int(0), OscType::Float(0.0)]), 0);
        assert!(link.status().recording);

        let Action::Finished(p, Some(r)) = link.handle(&msg("/am/stop", vec![OscType::Int(3)]), 0) else {
            panic!("écoute terminée attendue");
        };
        assert_eq!((r.project_id, r.end), (7, 64.0));
        assert_eq!(p.capture.positions.len(), 10);
        assert_eq!((p.capture.started_at, p.capture.tempo, p.capture.render), (1_000, 120.0, true));
        assert!(!link.status().recording);
    }

    #[test]
    fn ignores_files_outside_the_capture_folder() {
        let link = CaptureLink { socket: None, folder: "C:\\captures".into(), state: Mutex::default() };
        for path in ["C:/Users/moi/set.als", "C:/captures/../set.als", "C:/captures/sub/x.wav", "C:/capturesx/a.wav"] {
            let start = vec![OscType::Int(1), OscType::String(path.into()), OscType::Float(120.0)];
            link.handle(&msg("/am/start", start), 0);
            assert!(!link.status().recording, "{path}");
            assert!(matches!(link.handle(&msg("/am/abort", vec![OscType::Int(1)]), 0), Action::None));
        }
        assert!(link.in_folder("c:/Captures/cap-2.wav"), "casse et barres de Windows");
    }

    #[test]
    fn device_file() {
        let data = amxd(DEVICE_PATCHER);
        assert_eq!(&data[..4], b"ampf");
        assert_eq!(&data[8..12], b"aaaa");
        assert_eq!(&data[24..28], b"ptch");
        let len = u32::from_le_bytes(data[28..32].try_into().unwrap()) as usize;
        assert_eq!(data.len(), 32 + len);
        let json: serde_json::Value = serde_json::from_str(DEVICE_PATCHER).unwrap();
        let boxes = json["patcher"]["boxes"].as_array().unwrap();
        let texts: Vec<&str> = boxes.iter().filter_map(|b| b["box"]["text"].as_str()).collect();
        assert!(texts.contains(&"js am-capture.js"));
        assert!(texts.contains(&format!("udpreceive {DEVICE_PORT}").as_str()));
        assert!(texts.contains(&format!("udpsend 127.0.0.1 {APP_PORT}").as_str()));
        // Chaque connexion relie deux objets qui existent.
        let ids: Vec<&str> = boxes.iter().map(|b| b["box"]["id"].as_str().unwrap()).collect();
        for line in json["patcher"]["lines"].as_array().unwrap() {
            for end in ["source", "destination"] {
                assert!(ids.contains(&line["patchline"][end][0].as_str().unwrap()));
            }
        }
    }
}
