//! Synthesise a `.als` (gzipped Live-set XML) in the shape `wit-als` reads.
//!
//! Same contract as [`crate::logic`]: this emits only the whitelisted
//! elements `wit-als::build_model` extracts, in the nesting real Live uses.
//! It is not a Live writer — a real Live set carries thousands of elements
//! this omits, and Live would not open one of these. It exists so the app's
//! Ableton path (M5's exit criterion names "one Ableton `.als` lineage") is
//! demoable without shipping someone's real project.

use flate2::write::GzEncoder;
use flate2::Compression;
use std::io::Write;

#[derive(Debug, Clone, PartialEq)]
pub struct ClipSpec {
    pub id: u32,
    pub name: String,
    pub start: f64,
    pub end: f64,
    /// Written as a `RelativePath`; `wit-als` reduces it to a basename.
    pub sample: String,
    pub disabled: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TrackSpec {
    pub id: u32,
    pub name: String,
    pub volume: f64,
    pub pan: f64,
    /// Device element tags, e.g. `Eq8`, `Compressor2`. Each gets a single
    /// `<On><Manual Value=..>>` so the parameter fingerprint has something
    /// to hash — a knob move is modelled by changing `device_knob`.
    pub devices: Vec<String>,
    pub device_knob: f64,
    pub clips: Vec<ClipSpec>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SetSpec {
    pub creator: String,
    pub tempo_bpm: f64,
    pub tracks: Vec<TrackSpec>,
}

/// XML-escape a value destined for a double-quoted attribute. Demo project
/// names are ours, but a track name is the kind of string a future caller
/// will inevitably make user-supplied, and an unescaped `&` would produce a
/// file the parser rejects — a confusing failure to debug from the app.
fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

/// Format a float the way Live does — enough places that `wit-model`'s
/// 3-place rounding is exercised rather than bypassed.
fn num(v: f64) -> String {
    format!("{v:.6}")
}

fn clip_xml(clip: &ClipSpec) -> String {
    format!(
        r#"<AudioClip Id="{id}">
              <Name Value="{name}"/>
              <CurrentStart Value="{start}"/>
              <CurrentEnd Value="{end}"/>
              <Disabled Value="{disabled}"/>
              <SampleRef><FileRef><RelativePath Value="Samples/Imported/{sample}"/></FileRef></SampleRef>
            </AudioClip>"#,
        id = clip.id,
        name = esc(&clip.name),
        start = num(clip.start),
        end = num(clip.end),
        disabled = clip.disabled,
        sample = esc(&clip.sample),
    )
}

fn track_xml(track: &TrackSpec) -> String {
    let devices: String = track
        .devices
        .iter()
        .map(|tag| {
            format!(
                r#"<{tag} Id="0"><On><Manual Value="{knob}"/></On></{tag}>"#,
                tag = tag,
                knob = num(track.device_knob)
            )
        })
        .collect();
    let clips: String = track.clips.iter().map(clip_xml).collect();

    format!(
        r#"<AudioTrack Id="{id}">
          <Name><EffectiveName Value="{name}"/></Name>
          <Color Value="14"/>
          <DeviceChain>
            <Mixer>
              <Volume><Manual Value="{volume}"/></Volume>
              <Pan><Manual Value="{pan}"/></Pan>
              <Speaker><Manual Value="true"/></Speaker>
            </Mixer>
            <DeviceChain><Devices>{devices}</Devices></DeviceChain>
            <MainSequencer><Sample><ArrangerAutomation><Events>{clips}</Events></ArrangerAutomation></Sample></MainSequencer>
          </DeviceChain>
        </AudioTrack>"#,
        id = track.id,
        name = esc(&track.name),
        volume = num(track.volume),
        pan = num(track.pan),
    )
}

/// Render the uncompressed Live-set XML. Exposed for tests and for anyone
/// wanting to eyeball what the demo generator actually writes.
pub fn build_als_xml(spec: &SetSpec) -> String {
    let tracks: String = spec.tracks.iter().map(track_xml).collect();
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<Ableton MajorVersion="5" MinorVersion="12.0_12120" Creator="{creator}">
  <LiveSet>
    <Tracks>{tracks}</Tracks>
    <MainTrack>
      <DeviceChain><Mixer>
        <Tempo><Manual Value="{tempo}"/></Tempo>
      </Mixer></DeviceChain>
    </MainTrack>
  </LiveSet>
</Ableton>"#,
        creator = esc(&spec.creator),
        tempo = num(spec.tempo_bpm),
    )
}

/// Render and gzip — the on-disk `.als` form.
///
/// Compression level is pinned rather than left at default so the output is
/// byte-reproducible across flate2 versions that might change the default;
/// the demo library's determinism is a property tests rely on.
pub fn build_als(spec: &SetSpec) -> std::io::Result<Vec<u8>> {
    let xml = build_als_xml(spec);
    let mut encoder = GzEncoder::new(Vec::new(), Compression::new(6));
    encoder.write_all(xml.as_bytes())?;
    encoder.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn spec() -> SetSpec {
        SetSpec {
            creator: "Ableton Live 12.4.2".into(),
            tempo_bpm: 120.0,
            tracks: vec![TrackSpec {
                id: 8,
                name: "Rhodes".into(),
                volume: 0.7943282127,
                pan: 0.0,
                devices: vec!["Eq8".into()],
                device_knob: 1.0,
                clips: vec![ClipSpec {
                    id: 3,
                    name: "verse rhodes".into(),
                    start: 0.0,
                    end: 16.0,
                    sample: "rhodes take 3.wav".into(),
                    disabled: false,
                }],
            }],
        }
    }

    fn model_of(spec: &SetSpec) -> wit_model::Model {
        let gz = build_als(spec).unwrap();
        // Decompress independently first, so a gzip framing bug is
        // distinguishable from a parse bug when this fails.
        let mut xml = Vec::new();
        flate2::read::GzDecoder::new(&gz[..])
            .read_to_end(&mut xml)
            .expect("output must be valid gzip");
        assert!(xml.starts_with(b"<?xml"));
        wit_als::parse(&gz).expect("wit-als must parse what we generate")
    }

    #[test]
    fn every_whitelisted_field_survives_a_round_trip_through_wit_als() {
        let spec = spec();
        let model = model_of(&spec);
        assert_eq!(model.creator.as_deref(), Some("Ableton Live 12.4.2"));
        assert_eq!(model.tempo_bpm, Some(120.0));
        assert_eq!(model.tracks.len(), 1);
        let track = model.tracks.values().next().unwrap();
        assert_eq!(track.name, "Rhodes");
        // 0.7943282127 rounded to wit-model's 3 places.
        assert_eq!(track.volume, Some(0.794));
        assert_eq!(track.device_tags(), vec!["Eq8"]);
        assert_eq!(track.clips.len(), 1);
        // The path is written in full and reduced to a basename on read.
        assert_eq!(track.clips["3"].sample, "rhodes take 3.wav");
        assert_eq!(track.clips["3"].name, "verse rhodes");
    }

    #[test]
    fn an_identical_spec_produces_no_semantic_change() {
        let a = model_of(&spec());
        let b = model_of(&spec());
        assert!(wit_diff::diff(&a, &b).is_empty());
    }

    #[test]
    fn a_volume_move_is_a_semantic_change() {
        let mut quieter = spec();
        quieter.tracks[0].volume = 0.5;
        let records = wit_diff::diff(&model_of(&spec()), &model_of(&quieter));
        assert_eq!(records.len(), 1, "expected exactly one change: {records:?}");
    }

    #[test]
    fn a_knob_turn_inside_a_device_is_a_semantic_change() {
        // The M1 named bug fix 3 case — same device chain shape, different
        // parameter value. If the demo library could not express this, the
        // app's most interesting Ableton sentence would be undemoable.
        let mut turned = spec();
        turned.tracks[0].device_knob = 0.25;
        assert!(!wit_diff::diff(&model_of(&spec()), &model_of(&turned)).is_empty());
    }

    #[test]
    fn a_name_needing_xml_escaping_round_trips_intact() {
        let mut hostile = spec();
        hostile.tracks[0].name = r#"Bass & <Drums> "wide""#.into();
        let model = model_of(&hostile);
        assert_eq!(
            model.tracks.values().next().unwrap().name,
            r#"Bass & <Drums> "wide""#
        );
    }

    #[test]
    fn generation_is_deterministic() {
        assert_eq!(build_als(&spec()).unwrap(), build_als(&spec()).unwrap());
    }
}
