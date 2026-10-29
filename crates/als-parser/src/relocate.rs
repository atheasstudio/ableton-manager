//! Réécriture des chemins de samples dans un `.als`.
//!
//! Quand on recrée un set ailleurs (branche, restauration), certains fichiers
//! audio changent de place. On remplace alors leur `Path` (et on les rend
//! relatifs au projet) sans toucher au reste du document.

use std::collections::HashMap;
use std::io::Read;
#[cfg(test)]
use std::io::Write;

use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, Writer};

use crate::Error;

/// Nouvel emplacement d'un fichier audio.
#[derive(Debug, Clone)]
pub struct Relocation {
    /// Chemin absolu, avec des `/` comme Live.
    pub path: String,
    /// Chemin relatif au dossier du projet.
    pub relative_path: String,
}

/// Décompresse un `.als` (accepte aussi un XML non compressé).
pub fn read_xml(als: &[u8]) -> Result<Vec<u8>, Error> {
    if !als.starts_with(&[0x1f, 0x8b]) {
        return Ok(als.to_vec());
    }
    let mut xml = Vec::new();
    GzDecoder::new(als).read_to_end(&mut xml)?;
    Ok(xml)
}

/// Remplace, dans chaque `FileRef` dont le `Path` est une clé de `map`, le
/// chemin absolu et le chemin relatif. Renvoie le `.als` recompressé.
pub fn relocate_samples(als: &[u8], map: &HashMap<String, Relocation>) -> Result<Vec<u8>, Error> {
    let xml = read_xml(als)?;
    let mut reader = Reader::from_reader(&xml[..]);
    let mut writer = Writer::new(GzEncoder::new(Vec::new(), Compression::default()));
    let mut file_ref: Option<Vec<Event<'static>>> = None;
    let mut buf = Vec::new();
    loop {
        let event = reader.read_event_into(&mut buf)?.into_owned();
        buf.clear();
        if matches!(event, Event::Eof) {
            break;
        }
        match (&mut file_ref, &event) {
            (None, Event::Start(e)) if e.name().as_ref() == b"FileRef" => {
                file_ref = Some(vec![event]);
            }
            (Some(events), Event::End(e)) if e.name().as_ref() == b"FileRef" => {
                events.push(event);
                for e in rewrite_file_ref(file_ref.take().unwrap(), map) {
                    writer.write_event(e).map_err(io_error)?;
                }
            }
            (Some(events), _) => events.push(event),
            (None, _) => writer.write_event(event).map_err(io_error)?,
        }
    }
    Ok(writer.into_inner().finish()?)
}

fn io_error(e: impl std::fmt::Display) -> Error {
    Error::Io(std::io::Error::other(e.to_string()))
}

fn value(e: &BytesStart) -> Option<String> {
    e.try_get_attribute("Value")
        .ok()
        .flatten()
        .and_then(|a| a.unescape_value().ok().map(|v| v.into_owned()))
}

/// Les éléments `Path`, `RelativePath` et `RelativePathType` sont des enfants
/// directs du `FileRef`, de la forme `<Path Value="…" />`.
fn rewrite_file_ref(
    events: Vec<Event<'static>>,
    map: &HashMap<String, Relocation>,
) -> Vec<Event<'static>> {
    let target = events.iter().find_map(|e| match e {
        Event::Empty(e) if e.name().as_ref() == b"Path" => value(e).and_then(|v| map.get(&v)),
        _ => None,
    });
    let Some(target) = target else { return events };
    events
        .into_iter()
        .map(|e| match &e {
            Event::Empty(el) => {
                let new = match el.name().as_ref() {
                    b"Path" => target.path.as_str(),
                    b"RelativePath" => target.relative_path.as_str(),
                    b"RelativePathType" => "3",
                    _ => return e,
                };
                let name = String::from_utf8_lossy(el.name().as_ref()).into_owned();
                Event::Empty(BytesStart::new(name).with_attributes([("Value", new)]))
            }
            _ => e,
        })
        .collect()
}

#[cfg(test)]
fn compress(xml: &[u8]) -> Result<Vec<u8>, Error> {
    let mut gz = GzEncoder::new(Vec::new(), Compression::default());
    gz.write_all(xml)?;
    Ok(gz.finish()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SET: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Ableton Creator="Ableton Live 12.2.5">
	<LiveSet>
		<SampleRef>
			<FileRef>
				<RelativePathType Value="0" />
				<RelativePath Value="" />
				<Path Value="E:/Musique telecharger/vocals &amp; co.wav" />
				<Type Value="1" />
			</FileRef>
		</SampleRef>
		<SampleRef>
			<FileRef>
				<RelativePathType Value="3" />
				<RelativePath Value="Samples/Recorded/kick.wav" />
				<Path Value="F:/old/kick.wav" />
			</FileRef>
		</SampleRef>
	</LiveSet>
</Ableton>"#;

    #[test]
    fn untouched_without_map() {
        let out = relocate_samples(&compress(SET.as_bytes()).unwrap(), &HashMap::new()).unwrap();
        assert_eq!(String::from_utf8(read_xml(&out).unwrap()).unwrap(), SET);
    }

    #[test]
    fn rewrites_only_matching_file_ref() {
        let map = HashMap::from([(
            "E:/Musique telecharger/vocals & co.wav".to_owned(),
            Relocation {
                path: "F:/Projets/x Project/Samples/Imported/vocals & co.wav".into(),
                relative_path: "Samples/Imported/vocals & co.wav".into(),
            },
        )]);
        let out = relocate_samples(SET.as_bytes(), &map).unwrap();
        let xml = String::from_utf8(read_xml(&out).unwrap()).unwrap();
        assert!(xml.contains(r#"<Path Value="F:/Projets/x Project/Samples/Imported/vocals &amp; co.wav"/>"#));
        assert!(xml.contains(r#"<RelativePath Value="Samples/Imported/vocals &amp; co.wav"/>"#));
        assert_eq!(xml.matches(r#"<RelativePathType Value="3"#).count(), 2);
        assert!(xml.contains(r#"<Path Value="F:/old/kick.wav" />"#), "l'autre FileRef est intact");
        let set = crate::parse_xml(xml.as_bytes()).unwrap();
        assert_eq!(set.creator, "Ableton Live 12.2.5");
    }
}
