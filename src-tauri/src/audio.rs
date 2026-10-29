//! Audio des aperçus : lecture des fichiers (WAV, AIFF, MP3, FLAC, OGG, M4A),
//! forme d'onde et encodage MP3.

use std::fs::File;
use std::path::Path;

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

/// Au-delà, l'aperçu est coupé : ça reste un aperçu, et la mémoire est limitée.
pub const MAX_SECONDS: u32 = 15 * 60;

/// Nombre de points de la forme d'onde, quelle que soit la durée.
pub const PEAKS: usize = 1600;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("fichier : {0}")]
    Io(#[from] std::io::Error),
    #[error("audio illisible : {0}")]
    Decode(String),
    #[error("encodage MP3 : {0}")]
    Encode(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Audio stéréo 16 bits, une piste par canal.
#[derive(Debug, Clone, Default)]
pub struct Pcm {
    pub rate: u32,
    pub left: Vec<i16>,
    pub right: Vec<i16>,
}

impl Pcm {
    pub fn silent(rate: u32, len: usize) -> Self {
        Self { rate, left: vec![0; len], right: vec![0; len] }
    }

    pub fn len(&self) -> usize {
        self.left.len()
    }

    pub fn seconds(&self) -> f64 {
        if self.rate == 0 {
            0.0
        } else {
            self.len() as f64 / self.rate as f64
        }
    }

    /// Copie `src[from..to]` à la position `at`, en agrandissant si besoin
    /// (jamais au-delà de `MAX_SECONDS`).
    pub fn paste(&mut self, src: &Pcm, from: usize, to: usize, at: usize) {
        let max = (MAX_SECONDS * self.rate) as usize;
        let to = to.min(src.len());
        if from >= to || at >= max {
            return;
        }
        let end = (at + to - from).min(max);
        if end > self.len() {
            self.left.resize(end, 0);
            self.right.resize(end, 0);
        }
        let n = end - at;
        self.left[at..end].copy_from_slice(&src.left[from..from + n]);
        self.right[at..end].copy_from_slice(&src.right[from..from + n]);
    }

    pub fn truncate(&mut self, len: usize) {
        self.left.truncate(len);
        self.right.truncate(len);
    }

    /// Retire le silence de fin (garde le début : il aligne l'aperçu sur la mesure 1).
    pub fn trim_end(&mut self) {
        let last = (0..self.len()).rev().find(|&i| self.left[i].abs() > 16 || self.right[i].abs() > 16);
        self.truncate(last.map_or(0, |i| i + 1));
    }
}

/// Lit un fichier audio. Le format est reconnu au contenu, pas à l'extension.
pub fn decode(path: &Path) -> Result<Pcm> {
    let file = File::open(path)?;
    let stream = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let bad = |e: SymphoniaError| Error::Decode(e.to_string());
    let probed = symphonia::default::get_probe()
        .format(&hint, stream, &FormatOptions::default(), &MetadataOptions::default())
        .map_err(bad)?;
    let mut format = probed.format;
    let track = format
        .default_track()
        .ok_or_else(|| Error::Decode("aucune piste audio".into()))?;
    let track_id = track.id;
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(bad)?;

    let mut pcm = Pcm { rate: track.codec_params.sample_rate.unwrap_or(0), ..Default::default() };
    let mut buf: Option<SampleBuffer<i16>> = None;
    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(SymphoniaError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            // Fichier coupé (capture interrompue) : on garde ce qui a été lu.
            Err(SymphoniaError::IoError(_)) if pcm.len() > 0 => break,
            Err(e) => return Err(bad(e)),
        };
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(e) => return Err(bad(e)),
        };
        let spec = *decoded.spec();
        pcm.rate = spec.rate;
        let channels = spec.channels.count().max(1);
        let b = buf.get_or_insert_with(|| SampleBuffer::new(decoded.capacity() as u64, spec));
        if b.capacity() < decoded.capacity() * channels {
            *b = SampleBuffer::new(decoded.capacity() as u64, spec);
        }
        b.copy_interleaved_ref(decoded);
        for frame in b.samples().chunks(channels) {
            pcm.left.push(frame[0]);
            pcm.right.push(frame[if channels > 1 { 1 } else { 0 }]);
        }
        if pcm.rate > 0 && pcm.len() >= (MAX_SECONDS * pcm.rate) as usize {
            break;
        }
    }
    if pcm.rate == 0 || pcm.len() == 0 {
        return Err(Error::Decode("fichier vide".into()));
    }
    Ok(pcm)
}

/// Forme d'onde : niveau crête de chaque tranche, de 0 à 255.
pub fn peaks(pcm: &Pcm, count: usize) -> Vec<u8> {
    let len = pcm.len();
    if len == 0 {
        return vec![];
    }
    let count = count.min(len);
    (0..count)
        .map(|i| {
            let (a, b) = (i * len / count, ((i + 1) * len / count).max(i * len / count + 1));
            let peak = pcm.left[a..b]
                .iter()
                .chain(&pcm.right[a..b])
                .map(|s| s.unsigned_abs())
                .max()
                .unwrap_or(0);
            (peak as u32 * 255 / 32768) as u8
        })
        .collect()
}

/// Encode en MP3 stéréo 192 kb/s (quelques Mo pour un morceau).
pub fn encode_mp3(pcm: &Pcm) -> Result<Vec<u8>> {
    use mp3lame_encoder::{Bitrate, Builder, DualPcm, FlushNoGap, Quality};
    let err = |e: &dyn std::fmt::Display| Error::Encode(e.to_string());
    let mut encoder = Builder::new()
        .ok_or_else(|| Error::Encode("LAME indisponible".into()))?
        .with_num_channels(2)
        .map_err(|e| err(&e))?
        .with_sample_rate(pcm.rate)
        .map_err(|e| err(&e))?
        .with_brate(Bitrate::Kbps192)
        .map_err(|e| err(&e))?
        .with_quality(Quality::Good)
        .map_err(|e| err(&e))?
        .build()
        .map_err(|e| err(&e))?;
    let mut out = vec![];
    const CHUNK: usize = 1 << 16;
    for (left, right) in pcm.left.chunks(CHUNK).zip(pcm.right.chunks(CHUNK)) {
        out.reserve(mp3lame_encoder::max_required_buffer_size(left.len()));
        encoder.encode_to_vec(DualPcm { left, right }, &mut out).map_err(|e| err(&e))?;
    }
    out.reserve(7200);
    encoder.flush_to_vec::<FlushNoGap>(&mut out).map_err(|e| err(&e))?;
    Ok(out)
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// Écrit un WAV stéréo : un sinus à `freq` Hz pendant `seconds`.
    pub fn write_tone(path: &Path, rate: u32, seconds: f64, freq: f64) {
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(path, spec).unwrap();
        for i in 0..(seconds * rate as f64) as usize {
            let s = ((i as f64 / rate as f64 * freq * std::f64::consts::TAU).sin() * 16000.0) as i16;
            w.write_sample(s).unwrap();
            w.write_sample(s).unwrap();
        }
        w.finalize().unwrap();
    }

    #[test]
    fn decode_peaks_and_mp3() {
        let dir = tempfile::tempdir().unwrap();
        let wav = dir.path().join("bounce.wav");
        write_tone(&wav, 44_100, 2.0, 440.0);
        let pcm = decode(&wav).unwrap();
        assert_eq!((pcm.rate, pcm.len()), (44_100, 88_200));

        let p = peaks(&pcm, 100);
        assert_eq!(p.len(), 100);
        assert!(p.iter().all(|&x| (120..=127).contains(&x)), "sinus à mi-niveau : {p:?}");

        let mp3 = encode_mp3(&pcm).unwrap();
        assert!(mp3.len() > 20_000 && mp3.len() < 80_000, "{} octets", mp3.len());
        let back = dir.path().join("apercu.mp3");
        std::fs::write(&back, &mp3).unwrap();
        let decoded = decode(&back).unwrap();
        assert!((decoded.seconds() - 2.0).abs() < 0.1, "{} s", decoded.seconds());
    }

    #[test]
    fn paste_grows_and_trims() {
        let src = Pcm { rate: 10, left: vec![1000; 10], right: vec![1000; 10] };
        let mut out = Pcm::silent(10, 0);
        out.paste(&src, 2, 6, 20);
        assert_eq!(out.len(), 24);
        assert_eq!(&out.left[18..24], &[0, 0, 1000, 1000, 1000, 1000]);
        out.left.resize(40, 0);
        out.right.resize(40, 0);
        out.trim_end();
        assert_eq!(out.len(), 24);
    }
}
