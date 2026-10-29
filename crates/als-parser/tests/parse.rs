use als_parser::{parse_xml, PluginFormat, TrackKind};

/// Extrait réduit d'un set Live 12 (structure relevée sur de vrais projets).
const LIVE12: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Ableton MajorVersion="5" MinorVersion="12.0_12203" Creator="Ableton Live 12.2.5">
  <LiveSet>
    <Tracks>
      <MidiTrack Id="1">
        <Name><EffectiveName Value="Kontakt 8" /><UserName Value="" /></Name>
        <DeviceChain><DeviceChain><Devices>
          <PluginDevice Id="0"><PluginDesc><Vst3PluginInfo Id="0">
            <Preset><Vst3Preset Id="5"><Name Value="" /></Vst3Preset></Preset>
            <Name Value="Kontakt 8" />
          </Vst3PluginInfo></PluginDesc></PluginDevice>
          <PluginDevice Id="1"><PluginDesc><VstPluginInfo Id="0">
            <PlugName Value="Serum" />
          </VstPluginInfo></PluginDesc></PluginDevice>
        </Devices></DeviceChain></DeviceChain>
      </MidiTrack>
      <AudioTrack Id="2">
        <Name><EffectiveName Value="Clap &amp; Snare" /></Name>
        <MainSequencer><Sample><ArrangerAutomation><Events>
          <AudioClip Id="0"><SampleRef><FileRef>
            <RelativePathType Value="3" />
            <RelativePath Value="../Pack/Clap.wav" />
            <Path Value="F:/musique/Pack/Clap.wav" />
          </FileRef></SampleRef></AudioClip>
          <AudioClip Id="1" Time="64"><CurrentStart Value="64" /><CurrentEnd Value="96" /><SampleRef><FileRef>
            <RelativePath Value="../Pack/Clap.wav" />
            <Path Value="F:/musique/Pack/Clap.wav" />
          </FileRef></SampleRef></AudioClip>
        </Events></ArrangerAutomation></Sample>
        <ClipSlotList><ClipSlot Id="0"><ClipSlot><Value>
          <AudioClip Id="2"><CurrentEnd Value="500" /></AudioClip>
        </Value></ClipSlot></ClipSlot></ClipSlotList></MainSequencer>
      </AudioTrack>
      <ReturnTrack Id="3"><Name><EffectiveName Value="A-Reverb" /></Name></ReturnTrack>
    </Tracks>
    <MainTrack>
      <Name><EffectiveName Value="Main" /></Name>
      <DeviceChain><Mixer>
        <Tempo><LomId Value="0" /><Manual Value="113.5" /></Tempo>
        <TimeSignature><Manual Value="302" /></TimeSignature>
      </Mixer></DeviceChain>
    </MainTrack>
    <Locators><Locators>
      <Locator Id="0"><Time Value="64" /><Name Value="Drop" /></Locator>
    </Locators></Locators>
  </LiveSet>
</Ableton>"#;

/// Ancien format (Live 9-10) : MasterTrack, chemins découpés en éléments.
const LIVE10: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Ableton MajorVersion="5" MinorVersion="10.0_377" Creator="Ableton Live 10.1.30">
  <LiveSet>
    <Tracks>
      <AudioTrack Id="0">
        <Name><EffectiveName Value="Kick" /></Name>
        <AudioClip><SampleRef><FileRef>
          <HasRelativePath Value="true" />
          <RelativePath>
            <RelativePathElement Id="0" Dir="Samples" />
            <RelativePathElement Id="1" Dir="Recorded" />
          </RelativePath>
          <Name Value="Kick.wav" />
        </FileRef></SampleRef></AudioClip>
      </AudioTrack>
    </Tracks>
    <MasterTrack><DeviceChain><Mixer>
      <Tempo><Manual Value="128" /></Tempo>
      <TimeSignature><Manual Value="201" /></TimeSignature>
    </Mixer></DeviceChain></MasterTrack>
  </LiveSet>
</Ableton>"#;

#[test]
fn live12_set() {
    let set = parse_xml(LIVE12.as_bytes()).unwrap();
    assert_eq!(set.live_version(), Some("12.2.5"));
    assert_eq!(set.tempo, Some(113.5));
    assert_eq!(set.time_signature, Some((6, 8)));

    let tracks: Vec<_> = set.tracks.iter().map(|t| (t.name.as_str(), t.kind)).collect();
    assert_eq!(
        tracks,
        [
            ("Kontakt 8", TrackKind::Midi),
            ("Clap & Snare", TrackKind::Audio),
            ("A-Reverb", TrackKind::Return),
        ]
    );

    let plugins: Vec<_> = set.plugins.iter().map(|p| (p.name.as_str(), p.format)).collect();
    assert_eq!(plugins, [("Kontakt 8", PluginFormat::Vst3), ("Serum", PluginFormat::Vst2)]);

    assert_eq!(set.samples.len(), 1, "le même sample ne doit apparaître qu'une fois");
    assert_eq!(set.samples[0].path.as_deref(), Some("F:/musique/Pack/Clap.wav"));
    assert_eq!(set.samples[0].file_name(), Some("Clap.wav"));

    assert_eq!(set.locators.len(), 1);
    assert_eq!(set.locators[0].time, 64.0);
    assert_eq!(set.locators[0].name, "Drop");
    assert_eq!(set.arrangement_end, Some(96.0), "les clips de la vue Session sont ignorés");
}

#[test]
fn live10_set() {
    let set = parse_xml(LIVE10.as_bytes()).unwrap();
    assert_eq!(set.live_version(), Some("10.1.30"));
    assert_eq!(set.tempo, Some(128.0));
    assert_eq!(set.time_signature, Some((4, 4)));
    assert_eq!(set.samples.len(), 1);
    assert_eq!(set.samples[0].relative_path.as_deref(), Some("Samples/Recorded/Kick.wav"));
    assert_eq!(set.samples[0].file_name(), Some("Kick.wav"));
}

#[test]
fn rejects_other_xml() {
    assert!(parse_xml(r#"<?xml version="1.0"?><Other />"#.as_bytes()).is_err());
}

/// Lance le parseur sur de vrais projets si `ALS_FIXTURES` pointe vers un dossier
/// (ils ne sont pas versionnés : ce sont des projets personnels).
#[test]
fn real_projects() {
    let Ok(dir) = std::env::var("ALS_FIXTURES") else { return };
    let mut count = 0;
    for entry in walk(std::path::Path::new(&dir)) {
        let set = als_parser::parse_file(&entry)
            .unwrap_or_else(|e| panic!("{}: {e}", entry.display()));
        assert!(set.tempo.is_some(), "{}: tempo manquant", entry.display());
        assert!(!set.tracks.is_empty(), "{}: aucune piste", entry.display());
        println!(
            "{} — Live {} — {} BPM — {} pistes, {} plugins, {} samples",
            entry.display(),
            set.live_version().unwrap_or("?"),
            set.tempo.unwrap(),
            set.tracks.len(),
            set.plugins.len(),
            set.samples.len()
        );
        // Sans relocalisation, la réécriture rend exactement le même XML.
        let bytes = std::fs::read(&entry).unwrap();
        let same = als_parser::relocate_samples(&bytes, &Default::default()).unwrap();
        assert!(
            als_parser::read_xml(&same).unwrap() == als_parser::read_xml(&bytes).unwrap(),
            "{}: réécriture non fidèle",
            entry.display()
        );
        count += 1;
    }
    assert!(count > 0, "aucun .als trouvé dans {dir}");
}

fn walk(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(walk(&path));
        } else if path.extension().is_some_and(|e| e == "als") {
            out.push(path);
        }
    }
    out
}
