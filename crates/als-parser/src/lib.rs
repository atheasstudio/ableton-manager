//! Lecture des fichiers `.als` d'Ableton Live.
//!
//! Un `.als` est un document XML compressé en gzip. On le lit en flux (sans
//! construire d'arbre) et on ignore tout ce qu'on ne connaît pas, pour rester
//! tolérant aux différences entre versions de Live.

use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;

use flate2::read::GzDecoder;
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;
use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("lecture du fichier : {0}")]
    Io(#[from] std::io::Error),
    #[error("XML invalide : {0}")]
    Xml(#[from] quick_xml::Error),
    #[error("ce fichier n'est pas un set Ableton")]
    NotAbletonSet,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TrackKind {
    Audio,
    Midi,
    Group,
    Return,
}

impl TrackKind {
    fn from_tag(tag: &str) -> Option<Self> {
        match tag {
            "AudioTrack" => Some(Self::Audio),
            "MidiTrack" => Some(Self::Midi),
            "GroupTrack" => Some(Self::Group),
            "ReturnTrack" => Some(Self::Return),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Audio => "audio",
            Self::Midi => "midi",
            Self::Group => "group",
            Self::Return => "return",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Track {
    pub name: String,
    pub kind: TrackKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PluginFormat {
    Vst2,
    Vst3,
    Au,
}

#[derive(Debug, Clone, Serialize)]
pub struct Plugin {
    pub name: String,
    pub format: PluginFormat,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Sample {
    /// Chemin absolu tel qu'enregistré par Live (Live 11+), s'il existe.
    pub path: Option<String>,
    /// Chemin relatif au dossier du projet.
    pub relative_path: Option<String>,
    /// Nom du fichier (toujours présent dans les vieux formats).
    pub name: Option<String>,
}

impl Sample {
    /// Nom de fichier lisible, quelle que soit la version du format.
    pub fn file_name(&self) -> Option<&str> {
        if let Some(n) = self.name.as_deref().filter(|n| !n.is_empty()) {
            return Some(n);
        }
        let p = self.path.as_deref().or(self.relative_path.as_deref())?;
        p.rsplit(['/', '\\']).next()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Locator {
    /// Position en temps (beats).
    pub time: f64,
    pub name: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct LiveSet {
    /// Ex. « Ableton Live 12.2.5 ».
    pub creator: String,
    pub tempo: Option<f64>,
    /// (numérateur, dénominateur)
    pub time_signature: Option<(u32, u32)>,
    pub tracks: Vec<Track>,
    /// Plugins tiers, sans doublons, dans l'ordre d'apparition.
    pub plugins: Vec<Plugin>,
    /// Fichiers audio référencés, sans doublons.
    pub samples: Vec<Sample>,
    pub locators: Vec<Locator>,
}

impl LiveSet {
    /// Version de Live, ex. « 12.2.5 ».
    pub fn live_version(&self) -> Option<&str> {
        self.creator.strip_prefix("Ableton Live ")
    }
}

/// Lit un fichier `.als` (compressé ou non).
pub fn parse_file(path: impl AsRef<Path>) -> Result<LiveSet, Error> {
    let mut file = BufReader::new(File::open(path)?);
    let gzip = file.fill_buf()?.starts_with(&[0x1f, 0x8b]);
    if gzip {
        parse_xml(BufReader::new(GzDecoder::new(file)))
    } else {
        parse_xml(file)
    }
}

/// Lit le XML d'un set, déjà décompressé.
pub fn parse_xml<R: BufRead>(input: R) -> Result<LiveSet, Error> {
    let mut reader = Reader::from_reader(input);
    let mut parser = Parser::default();
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Start(e) => {
                parser.element(&e);
                parser.stack.push(tag_name(&e));
            }
            Event::Empty(e) => parser.element(&e),
            Event::End(_) => {
                let tag = parser.stack.pop().unwrap_or_default();
                parser.close(&tag);
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    if !parser.is_ableton {
        return Err(Error::NotAbletonSet);
    }
    Ok(parser.set)
}

/// Lit un `.als` depuis n'importe quelle source (utile pour les tests).
pub fn parse_reader<R: Read>(input: R) -> Result<LiveSet, Error> {
    parse_xml(BufReader::new(GzDecoder::new(input)))
}

fn tag_name(e: &BytesStart) -> String {
    String::from_utf8_lossy(e.name().as_ref()).into_owned()
}

fn attr(e: &BytesStart, key: &str) -> Option<String> {
    e.try_get_attribute(key)
        .ok()
        .flatten()
        .and_then(|a| a.unescape_value().ok().map(|v| v.into_owned()))
}

#[derive(Default)]
struct Parser {
    stack: Vec<String>,
    is_ableton: bool,
    set: LiveSet,
    sample: Option<SampleBuilder>,
    locator: Option<(Option<f64>, String)>,
}

#[derive(Default)]
struct SampleBuilder {
    sample: Sample,
    /// Dossiers des vieux formats (Live 9-10), un élément par niveau.
    dirs: Vec<String>,
}

impl Parser {
    /// Vrai si les ancêtres de l'élément courant se terminent par `suffix`.
    fn under(&self, suffix: &[&str]) -> bool {
        self.stack.len() >= suffix.len()
            && self.stack[self.stack.len() - suffix.len()..]
                .iter()
                .zip(suffix)
                .all(|(a, b)| a == b)
    }

    fn parent(&self) -> Option<&str> {
        self.stack.last().map(String::as_str)
    }

    fn under_main_mixer(&self, param: &str) -> bool {
        // « MasterTrack » jusqu'à Live 11, « MainTrack » depuis Live 12.
        ["MainTrack", "MasterTrack"]
            .iter()
            .any(|main| self.under(&["LiveSet", main, "DeviceChain", "Mixer", param]))
    }

    fn element(&mut self, e: &BytesStart) {
        let tag = tag_name(e);
        let value = || attr(e, "Value");
        match tag.as_str() {
            "Ableton" if self.stack.is_empty() => {
                self.is_ableton = true;
                self.set.creator = attr(e, "Creator").unwrap_or_default();
            }
            "Manual" if self.under_main_mixer("Tempo") => {
                self.set.tempo = value().and_then(|v| v.parse().ok());
            }
            "Manual" if self.under_main_mixer("TimeSignature") => {
                self.set.time_signature =
                    value().and_then(|v| v.parse().ok()).map(decode_time_signature);
            }
            t if self.under(&["LiveSet", "Tracks"]) => {
                if let Some(kind) = TrackKind::from_tag(t) {
                    self.set.tracks.push(Track { name: String::new(), kind });
                }
            }
            "EffectiveName"
                if self.stack.len() >= 4
                    && self.under(&["Name"])
                    && TrackKind::from_tag(&self.stack[self.stack.len() - 2]).is_some()
                    && self.stack[self.stack.len() - 3] == "Tracks" =>
            {
                if let Some(track) = self.set.tracks.last_mut() {
                    track.name = value().unwrap_or_default();
                }
            }
            "Name" if self.parent() == Some("Vst3PluginInfo") => {
                self.add_plugin(value(), PluginFormat::Vst3)
            }
            "PlugName" if self.parent() == Some("VstPluginInfo") => {
                self.add_plugin(value(), PluginFormat::Vst2)
            }
            "Name" if self.parent() == Some("AuPluginInfo") => {
                self.add_plugin(value(), PluginFormat::Au)
            }
            "FileRef" if self.parent() == Some("SampleRef") => {
                self.sample = Some(SampleBuilder::default());
            }
            _ => {}
        }

        let parent = self.stack.last().cloned();
        let parent = parent.as_deref();
        let in_file_ref = self.under(&["SampleRef", "FileRef"]);
        if let Some(b) = self.sample.as_mut() {
            let non_empty = || value().filter(|v| !v.is_empty());
            match tag.as_str() {
                "Path" if in_file_ref => b.sample.path = non_empty(),
                "Name" if in_file_ref => b.sample.name = non_empty(),
                "RelativePath" if in_file_ref => b.sample.relative_path = non_empty(),
                "RelativePathElement" if parent == Some("RelativePath") => {
                    if let Some(dir) = attr(e, "Dir") {
                        b.dirs.push(dir);
                    }
                }
                _ => {}
            }
        }

        if tag == "Locator" && self.under(&["Locators", "Locators"]) {
            self.locator = Some((None, String::new()));
        }
        if let Some(loc) = self.locator.as_mut() {
            if parent == Some("Locator") {
                match tag.as_str() {
                    "Time" => loc.0 = value().and_then(|v| v.parse().ok()),
                    "Name" => loc.1 = value().unwrap_or_default(),
                    _ => {}
                }
            }
        }
    }

    fn close(&mut self, tag: &str) {
        match tag {
            "FileRef" if self.parent() == Some("SampleRef") => {
                if let Some(b) = self.sample.take() {
                    self.add_sample(b);
                }
            }
            "Locator" => {
                if let Some((Some(time), name)) = self.locator.take() {
                    self.set.locators.push(Locator { time, name });
                }
            }
            _ => {}
        }
    }

    fn add_plugin(&mut self, name: Option<String>, format: PluginFormat) {
        let Some(name) = name.filter(|n| !n.is_empty()) else { return };
        if !self.set.plugins.iter().any(|p| p.name == name && p.format == format) {
            self.set.plugins.push(Plugin { name, format });
        }
    }

    fn add_sample(&mut self, b: SampleBuilder) {
        let mut sample = b.sample;
        if sample.relative_path.is_none() && !b.dirs.is_empty() {
            let mut parts = b.dirs;
            parts.extend(sample.name.clone());
            sample.relative_path = Some(parts.join("/"));
        }
        if sample.path.is_none() && sample.relative_path.is_none() && sample.name.is_none() {
            return;
        }
        // Un même fichier peut être noté avec ou sans chemin relatif selon le clip.
        let key = |s: &Sample| s.path.clone().or(s.relative_path.clone()).or(s.name.clone());
        match self.set.samples.iter_mut().find(|s| key(s) == key(&sample)) {
            Some(existing) => {
                if existing.relative_path.is_none() {
                    existing.relative_path = sample.relative_path;
                }
            }
            None => self.set.samples.push(sample),
        }
    }
}

/// Live encode la signature en un seul nombre :
/// `(numérateur - 1) + 99 × log2(dénominateur)`. Ex. 201 = 4/4.
fn decode_time_signature(code: u32) -> (u32, u32) {
    let numerator = code % 99 + 1;
    let denominator = 1 << (code / 99);
    (numerator, denominator)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_signature_codes() {
        assert_eq!(decode_time_signature(201), (4, 4));
        assert_eq!(decode_time_signature(200), (3, 4));
        assert_eq!(decode_time_signature(302), (6, 8));
    }
}
