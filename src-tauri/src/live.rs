//! Pont avec Ableton Live : messages OSC/UDP vers le Remote Script
//! `remote-script/AbletonManager`, qui écoute sur 127.0.0.1:11040.

use std::net::{SocketAddr, UdpSocket};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::live_window::OpenSet;

/// Port d'écoute du Remote Script (voir `server.py`).
pub const PORT: u16 = 11040;
/// Version des messages attendue (voir `handlers.py`).
pub const SCRIPT_VERSION: i32 = 1;

/// Fichiers du Remote Script, installés par l'appli dans la User Library.
pub const SCRIPT_FILES: &[(&str, &str)] = &[
    ("__init__.py", include_str!("../../remote-script/AbletonManager/__init__.py")),
    ("handlers.py", include_str!("../../remote-script/AbletonManager/handlers.py")),
    ("osc.py", include_str!("../../remote-script/AbletonManager/osc.py")),
    ("server.py", include_str!("../../remote-script/AbletonManager/server.py")),
];

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(
        "le script Ableton Manager ne répond pas : vérifie que Live est ouvert et que \
         « AbletonManager » est choisi comme surface de contrôle \
         (Préférences > Link, Tempo & MIDI)"
    )]
    NoAnswer,
    #[error("Live : {0}")]
    Script(String),
    #[error("réponse inattendue de Live")]
    BadReply,
    #[error("connexion à Live : {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Vrai si le set ouvert dans Live est l'un de ces .als (le projet ou une de
/// ses branches). Le dossier du projet n'est comparé que si le titre le donne :
/// beaucoup de sets s'appellent « Sans titre ».
pub fn is_open(open: &OpenSet, candidates: &[PathBuf]) -> bool {
    let same = |a: &str, b: &str| a.to_lowercase() == b.to_lowercase();
    candidates.iter().any(|als| {
        let stem = als.file_stem().map(|s| s.to_string_lossy()).unwrap_or_default();
        let folder = als.parent().and_then(|p| p.file_name()).map(|s| s.to_string_lossy());
        same(&open.set, &stem)
            && match (&open.project, folder) {
                (Some(project), Some(folder)) => same(project, &folder),
                _ => true,
            }
    })
}

// ---------- OSC ----------

#[derive(Debug, Clone, PartialEq)]
pub enum Arg {
    Int(i32),
    Float(f32),
    Double(f64),
    Str(String),
    Bool(bool),
    Nil,
}

impl Arg {
    fn as_f64(&self) -> Option<f64> {
        match *self {
            Arg::Int(i) => Some(i.into()),
            Arg::Float(f) => Some(f.into()),
            Arg::Double(d) => Some(d),
            _ => None,
        }
    }

    fn as_bool(&self) -> Option<bool> {
        match *self {
            Arg::Bool(b) => Some(b),
            Arg::Int(i) => Some(i != 0),
            _ => None,
        }
    }
}

/// Chaîne OSC : terminée par un octet nul, puis complétée à 4 octets.
fn push_str(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(s.as_bytes());
    out.push(0);
    while !out.len().is_multiple_of(4) {
        out.push(0);
    }
}

pub fn encode(address: &str, args: &[Arg]) -> Vec<u8> {
    let mut tags = String::from(",");
    let mut payload = Vec::new();
    for arg in args {
        match arg {
            Arg::Int(i) => {
                tags.push('i');
                payload.extend_from_slice(&i.to_be_bytes());
            }
            Arg::Float(f) => {
                tags.push('f');
                payload.extend_from_slice(&f.to_be_bytes());
            }
            Arg::Double(d) => {
                tags.push('d');
                payload.extend_from_slice(&d.to_be_bytes());
            }
            Arg::Str(s) => {
                tags.push('s');
                push_str(&mut payload, s);
            }
            Arg::Bool(b) => tags.push(if *b { 'T' } else { 'F' }),
            Arg::Nil => tags.push('N'),
        }
    }
    let mut out = Vec::new();
    push_str(&mut out, address);
    push_str(&mut out, &tags);
    out.extend(payload);
    out
}

struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl Cursor<'_> {
    fn string(&mut self) -> Option<String> {
        let rest = self.data.get(self.pos..)?;
        let len = rest.iter().position(|&b| b == 0)?;
        let s = String::from_utf8(rest[..len].to_vec()).ok()?;
        self.pos += (len + 4) & !3;
        Some(s)
    }

    fn bytes<const N: usize>(&mut self) -> Option<[u8; N]> {
        let b = self.data.get(self.pos..self.pos + N)?.try_into().ok()?;
        self.pos += N;
        Some(b)
    }
}

/// Renvoie (adresse, arguments), ou `None` si le message est invalide.
pub fn decode(data: &[u8]) -> Option<(String, Vec<Arg>)> {
    let mut c = Cursor { data, pos: 0 };
    let address = c.string().filter(|a| a.starts_with('/'))?;
    if c.pos >= data.len() {
        return Some((address, vec![]));
    }
    let tags = c.string()?;
    let mut args = Vec::new();
    for tag in tags.strip_prefix(',')?.chars() {
        args.push(match tag {
            'i' => Arg::Int(i32::from_be_bytes(c.bytes()?)),
            'f' => Arg::Float(f32::from_be_bytes(c.bytes()?)),
            'd' => Arg::Double(f64::from_be_bytes(c.bytes()?)),
            's' => Arg::Str(c.string()?),
            'T' => Arg::Bool(true),
            'F' => Arg::Bool(false),
            'N' => Arg::Nil,
            _ => return None,
        });
    }
    Some((address, args))
}

// ---------- Client ----------

/// État de Live renvoyé par `/manager/status`.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Transport {
    pub playing: bool,
    /// Position de la tête de lecture, en beats.
    pub beat: f64,
    pub tempo: f64,
}

pub struct Live {
    target: SocketAddr,
    /// Une requête à la fois : la réponse attendue est la prochaine reçue.
    socket: Mutex<Option<UdpSocket>>,
}

impl Live {
    pub fn new() -> Self {
        Self::with_port(PORT)
    }

    pub fn with_port(port: u16) -> Self {
        Self { target: SocketAddr::from(([127, 0, 0, 1], port)), socket: Mutex::new(None) }
    }

    /// Envoie un message et attend la réponse portant la même adresse.
    pub fn request(&self, address: &str, args: &[Arg], timeout: Duration) -> Result<Vec<Arg>> {
        let mut guard = self.socket.lock().unwrap();
        if guard.is_none() {
            *guard = Some(UdpSocket::bind("127.0.0.1:0")?);
        }
        let socket = guard.as_ref().unwrap();

        // Réponses en retard d'une requête précédente : on les jette.
        socket.set_nonblocking(true)?;
        let mut buf = [0u8; 4096];
        while socket.recv_from(&mut buf).is_ok() {}
        socket.set_nonblocking(false)?;

        socket.send_to(&encode(address, args), self.target)?;
        let deadline = Instant::now() + timeout;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(Error::NoAnswer);
            }
            socket.set_read_timeout(Some(left))?;
            match socket.recv_from(&mut buf) {
                Ok((n, from)) if from == self.target => match decode(&buf[..n]) {
                    Some((a, args)) if a == address => return Ok(args),
                    Some((a, args)) if a == "/live/error" => {
                        let msg = match args.first() {
                            Some(Arg::Str(s)) => s.clone(),
                            _ => "erreur inconnue".into(),
                        };
                        return Err(Error::Script(msg));
                    }
                    _ => continue,
                },
                Ok(_) => continue,
                // Sous Windows, personne n'écoute sur le port → ConnectionReset.
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock
                            | std::io::ErrorKind::TimedOut
                            | std::io::ErrorKind::ConnectionReset
                            | std::io::ErrorKind::ConnectionRefused
                    ) =>
                {
                    return Err(Error::NoAnswer)
                }
                Err(e) => return Err(e.into()),
            }
        }
    }

    /// Version des messages du script, ou `None` s'il ne répond pas.
    pub fn script_version(&self) -> Option<i32> {
        match self.request("/manager/version", &[], Duration::from_millis(300)) {
            Ok(args) => match args.first() {
                Some(Arg::Int(v)) => Some(*v),
                _ => Some(0),
            },
            // Ancienne version du script, sans ce message.
            Err(Error::Script(_)) => Some(0),
            Err(_) => None,
        }
    }

    pub fn transport(&self) -> Result<Transport> {
        let args = self.request("/manager/status", &[], Duration::from_millis(500))?;
        match args.as_slice() {
            [playing, beat, tempo] => Ok(Transport {
                playing: playing.as_bool().ok_or(Error::BadReply)?,
                beat: beat.as_f64().ok_or(Error::BadReply)?,
                tempo: tempo.as_f64().ok_or(Error::BadReply)?,
            }),
            _ => Err(Error::BadReply),
        }
    }

    /// Place la tête de lecture (en beats) et lance la lecture si demandé.
    pub fn jump(&self, beat: f64, play: bool) -> Result<()> {
        self.request(
            "/manager/jump",
            &[Arg::Float(beat as f32), Arg::Bool(play)],
            Duration::from_secs(1),
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_set_matching() {
        let main = PathBuf::from("F:/musique/Projets/Sans titre Project/Sans titre.als");
        let branch = PathBuf::from("F:/musique/Projets/Sans titre - club Project/Sans titre.als");
        let open = |set: &str, project: Option<&str>| OpenSet {
            set: set.into(),
            project: project.map(Into::into),
            modified: false,
        };
        assert!(is_open(&open("Sans titre", Some("Sans titre Project")), std::slice::from_ref(&main)));
        assert!(is_open(&open("sans titre", None), std::slice::from_ref(&main)), "casse ignorée");
        assert!(
            is_open(
                &open("Sans titre", Some("Sans titre - club Project")),
                &[main.clone(), branch]
            ),
            "une branche du projet compte"
        );
        assert!(!is_open(&open("Sans titre", Some("Autre Project")), std::slice::from_ref(&main)));
        assert!(!is_open(&open("Autre", Some("Sans titre Project")), &[main]));
    }

    #[test]
    fn osc_round_trip() {
        let args = vec![
            Arg::Float(160.5),
            Arg::Bool(true),
            Arg::Int(-3),
            Arg::Str("Refrain é".into()),
            Arg::Double(1.25),
            Arg::Nil,
        ];
        let data = encode("/manager/jump", &args);
        assert_eq!(data.len() % 4, 0);
        assert_eq!(decode(&data), Some(("/manager/jump".into(), args)));
    }

    #[test]
    fn osc_spec_example() {
        let data = encode("/oscillator/4/frequency", &[Arg::Float(440.0)]);
        assert_eq!(data, b"/oscillator/4/frequency\0,f\0\0\x43\xdc\x00\x00");
    }

    #[test]
    fn osc_invalid() {
        assert_eq!(decode(b"pas de l'osc"), None);
        let data = encode("/x", &[Arg::Int(1)]);
        assert_eq!(decode(&data[..data.len() - 2]), None);
    }

    /// Faux script : répond à chaque message par `reply`.
    fn fake_script(
        reply: impl Fn(&str, &[Arg]) -> Option<(String, Vec<Arg>)> + Send + 'static,
    ) -> Live {
        let server = UdpSocket::bind("127.0.0.1:0").unwrap();
        let port = server.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let mut buf = [0u8; 1024];
            while let Ok((n, from)) = server.recv_from(&mut buf) {
                let (addr, args) = decode(&buf[..n]).unwrap();
                if let Some((a, r)) = reply(&addr, &args) {
                    server.send_to(&encode(&a, &r), from).unwrap();
                }
            }
        });
        Live::with_port(port)
    }

    #[test]
    fn client_requests() {
        let live = fake_script(|addr, args| match addr {
            "/manager/status" => {
                Some((addr.into(), vec![Arg::Bool(true), Arg::Float(64.0), Arg::Float(120.0)]))
            }
            "/manager/jump" => {
                assert_eq!(args, [Arg::Float(160.0), Arg::Bool(false)]);
                // Une réponse parasite d'abord : elle doit être ignorée.
                Some((addr.into(), vec![args[0].clone()]))
            }
            "/manager/version" => Some((addr.into(), vec![Arg::Int(SCRIPT_VERSION)])),
            _ => Some(("/live/error".into(), vec![Arg::Str(format!("adresse inconnue : {addr}"))])),
        });
        let t = live.transport().unwrap();
        assert!(t.playing);
        assert_eq!((t.beat, t.tempo), (64.0, 120.0));
        live.jump(160.0, false).unwrap();
        assert_eq!(live.script_version(), Some(SCRIPT_VERSION));
        assert!(matches!(
            live.request("/nope", &[], Duration::from_secs(1)),
            Err(Error::Script(m)) if m.contains("/nope")
        ));
    }

    #[test]
    fn no_script() {
        // Port lié puis libéré : personne n'y écoute.
        let port = UdpSocket::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let live = Live::with_port(port);
        assert!(matches!(live.transport(), Err(Error::NoAnswer)));
        assert_eq!(live.script_version(), None);
    }
}
