//! Golden-parity integration test: build a synthetic `.als` in Rust,
//! decompress and parse it through the *real* `wit-als` reader, diff the
//! two resulting models through the *real* `wit-diff::diff`, and render
//! through the *real* `wit-model::render_text` — then assert the result
//! matches `tests/test_als_golden.py`'s `GOLDEN_DIFF` byte-for-byte.
//!
//! This is the M1 exit criterion ("`wit diff-als a.als b.als` prints the
//! golden") exercised end to end, not just at the `ChangeRecord` level —
//! the XML builder below is a compact, purpose-built stand-in for
//! `tests/factories/als.py`; it does not attempt to reproduce that
//! factory's full realism (warp markers, LomId bookkeeping, view state),
//! only the element shape `wit-als`'s whitelist extraction actually reads.
//! `crates/wit-als/src/extract.rs`'s own unit tests separately prove that
//! noise fields like those are ignored.

use flate2::write::GzEncoder;
use flate2::Compression;
use std::io::Write;

// --------------------------------------------------------------------- //
// a minimal Live-set XML builder — just enough to drive wit-als
// --------------------------------------------------------------------- //

#[derive(Clone)]
struct ClipSpec {
    id: &'static str,
    name: &'static str,
    start: f64,
    end: f64,
    sample: &'static str,
    disabled: bool,
    notes: usize,
}

impl ClipSpec {
    fn audio(
        id: &'static str,
        name: &'static str,
        start: f64,
        end: f64,
        sample: &'static str,
    ) -> Self {
        ClipSpec {
            id,
            name,
            start,
            end,
            sample,
            disabled: false,
            notes: 0,
        }
    }
    fn midi(id: &'static str, name: &'static str, start: f64, end: f64, notes: usize) -> Self {
        ClipSpec {
            id,
            name,
            start,
            end,
            sample: "",
            disabled: false,
            notes,
        }
    }
    fn disabled(mut self) -> Self {
        self.disabled = true;
        self
    }
}

#[derive(Clone)]
struct TrackSpec {
    id: &'static str,
    name: &'static str,
    kind: &'static str,
    color: &'static str,
    volume: f64,
    pan: f64,
    speaker: bool,
    devices: Vec<&'static str>,
    clips: Vec<ClipSpec>,
    automation_lanes: usize,
}

impl TrackSpec {
    fn new(id: &'static str, name: &'static str) -> Self {
        TrackSpec {
            id,
            name,
            kind: "AudioTrack",
            color: "13",
            volume: 1.0,
            pan: 0.0,
            speaker: true,
            devices: vec![],
            clips: vec![],
            automation_lanes: 0,
        }
    }
}

struct LiveSetSpec {
    tempo: f64,
    tracks: Vec<TrackSpec>,
}

fn clip_xml(is_midi: bool, c: &ClipSpec) -> String {
    let tag = if is_midi { "MidiClip" } else { "AudioClip" };
    let sample_xml = if c.sample.is_empty() {
        String::new()
    } else {
        format!(
            r#"<SampleRef><FileRef><RelativePath Value="{}"/></FileRef></SampleRef>"#,
            c.sample
        )
    };
    let notes_xml = if is_midi {
        let events: String = (0..c.notes)
            .map(|i| format!(r#"<MidiNoteEvent Time="{i}.0" Duration="0.5" Velocity="100" IsEnabled="true"/>"#))
            .collect();
        format!(
            r#"<Notes><KeyTracks><KeyTrack Id="0"><Notes>{events}</Notes></KeyTrack></KeyTracks></Notes>"#
        )
    } else {
        String::new()
    };
    format!(
        r#"<{tag} Id="{id}" Time="{start}"><CurrentStart Value="{start}"/><CurrentEnd Value="{end}"/><Name Value="{name}"/><Disabled Value="{disabled}"/>{sample_xml}{notes_xml}</{tag}>"#,
        id = c.id,
        start = c.start,
        end = c.end,
        name = c.name,
        disabled = c.disabled,
    )
}

fn track_xml(t: &TrackSpec) -> String {
    let is_midi = t.kind == "MidiTrack";
    let clips_xml: String = t.clips.iter().map(|c| clip_xml(is_midi, c)).collect();
    let devices_xml: String = t
        .devices
        .iter()
        .enumerate()
        .map(|(i, d)| format!(r#"<{d} Id="{i}"><On><Manual Value="true"/></On></{d}>"#))
        .collect();
    let automation_xml: String = (0..t.automation_lanes)
        .map(|i| format!(r#"<AutomationEnvelope Id="{i}"/>"#))
        .collect();
    let sample_holder = if is_midi { "ClipTimeable" } else { "Sample" };

    format!(
        r#"<{kind} Id="{id}">
            <Name><EffectiveName Value="{name}"/></Name>
            <Color Value="{color}"/>
            <AutomationEnvelopes><Envelopes>{automation_xml}</Envelopes></AutomationEnvelopes>
            <DeviceChain>
                <Mixer>
                    <Volume><Manual Value="{volume}"/></Volume>
                    <Pan><Manual Value="{pan}"/></Pan>
                    <Speaker><Manual Value="{speaker}"/></Speaker>
                </Mixer>
                <DeviceChain><Devices>{devices_xml}</Devices></DeviceChain>
                <MainSequencer><{sample_holder}><ArrangerAutomation><Events>{clips_xml}</Events></ArrangerAutomation></{sample_holder}></MainSequencer>
            </DeviceChain>
        </{kind}>"#,
        kind = t.kind,
        id = t.id,
        name = t.name,
        color = t.color,
        volume = t.volume,
        pan = t.pan,
        speaker = t.speaker,
    )
}

fn live_set_xml(ls: &LiveSetSpec) -> String {
    let tracks_xml: String = ls.tracks.iter().map(track_xml).collect();
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
        <Ableton Creator="Ableton Live 12.0.5">
            <LiveSet>
                <Tracks>{tracks_xml}</Tracks>
                <MasterTrack>
                    <DeviceChain><Mixer><Tempo><Manual Value="{tempo}"/></Tempo></Mixer></DeviceChain>
                </MasterTrack>
            </LiveSet>
        </Ableton>"#,
        tempo = ls.tempo,
    )
}

fn als_bytes(ls: &LiveSetSpec) -> Vec<u8> {
    let xml = live_set_xml(ls);
    let mut enc = GzEncoder::new(Vec::new(), Compression::default());
    enc.write_all(xml.as_bytes()).unwrap();
    enc.finish().unwrap()
}

fn diff_of(a: &LiveSetSpec, b: &LiveSetSpec) -> Vec<String> {
    let model_a = wit_als::parse(&als_bytes(a)).expect("factory-built .als must parse");
    let model_b = wit_als::parse(&als_bytes(b)).expect("factory-built .als must parse");
    let records = wit_diff::diff(&model_a, &model_b);
    wit_model::render_text(&records)
        .lines()
        .map(str::to_string)
        .collect()
}

// --------------------------------------------------------------------- //
// tests/test_als_golden.py::kitchen_sink + GOLDEN_DIFF, ported
// --------------------------------------------------------------------- //

fn kitchen_sink() -> (LiveSetSpec, LiveSetSpec) {
    let mut drums = TrackSpec::new("100", "Drums");
    drums.devices = vec!["Eq8"];
    drums.clips = vec![
        ClipSpec::audio("10", "kick", 0.0, 16.0, "Samples/Imported/old kick.wav"),
        ClipSpec::audio("11", "hat", 16.0, 32.0, "Samples/Imported/hat.wav"),
    ];

    let mut bass = TrackSpec::new("101", "Bass");
    bass.color = "26";
    bass.volume = 0.794;
    bass.clips = vec![ClipSpec::audio(
        "20",
        "sub",
        0.0,
        32.0,
        "Samples/Recorded/sub.wav",
    )];

    let mut strings = TrackSpec::new("102", "Strings");
    strings.kind = "MidiTrack";
    strings.automation_lanes = 1;
    strings.clips = vec![ClipSpec::midi("30", "pad", 8.0, 24.0, 12)];

    let scratch = TrackSpec::new("103", "Scratch");

    let before = LiveSetSpec {
        tempo: 120.0,
        tracks: vec![drums.clone(), bass.clone(), strings.clone(), scratch],
    };

    // after: tempo 124, Scratch removed, Vox added, and the kitchen-sink
    // edits from test_als_golden.py::kitchen_sink applied.
    drums.clips[0].sample = "Samples/Imported/Kick 01.wav"; // a genuine rename
    drums.clips[1].start = 16.0;
    drums.clips[1].end = 30.0;
    drums.devices = vec!["Eq8", "Compressor2"];
    drums.pan = -0.15;

    bass.name = "Sub Bass";
    bass.volume = 0.525;
    bass.clips.push(ClipSpec::audio(
        "21",
        "",
        32.0,
        40.0,
        "Samples/Imported/drop.wav",
    ));

    strings.speaker = false;
    strings.color = "7";
    strings.automation_lanes = 3;
    strings.clips[0].notes = 20;
    strings.clips[0] = strings.clips[0].clone().disabled();

    let vox = TrackSpec::new("104", "Vox");

    let after = LiveSetSpec {
        tempo: 124.0,
        tracks: vec![drums, bass, strings, vox],
    };

    (before, after)
}

const GOLDEN_DIFF: &[&str] = &[
    "TEMPO   120.0 -> 124.0 BPM",
    "SAMPLE~ 'old kick.wav' -> 'Kick 01.wav'  (1 clip reference(s))",
    "TRACK+  added 'Vox' (AudioTrack)",
    "TRACK-  removed 'Scratch'",
    "MIX~    [Drums] pan: 0.0 -> -0.15",
    "FX+     [Drums] added: Compressor2",
    "CLIP~   [Drums] 'hat' 16.0-32.0 -> 16.0-30.0",
    "TRACK~  renamed 'Bass' -> 'Sub Bass'",
    "MIX~    [Sub Bass] volume: 0.794 -> 0.525",
    "CLIP+   [Sub Bass] added 'drop.wav' at bar 32.0",
    "MIX~    [Strings] output enabled: true -> false",
    "MIX~    [Strings] color: 13 -> 7",
    "AUTO~   [Strings] automation lanes 1 -> 3",
    "MIDI~   [Strings] note count 12 -> 20",
    "CLIP~   [Strings] 'pad' muted",
];

#[test]
fn golden_diff_matches_python_byte_for_byte() {
    let (before, after) = kitchen_sink();
    let lines = diff_of(&before, &after);
    assert_eq!(lines, GOLDEN_DIFF);
}

#[test]
fn golden_diff_is_deterministic_across_repeated_runs() {
    let (before, after) = kitchen_sink();
    let first = diff_of(&before, &after);
    for _ in 0..20 {
        assert_eq!(diff_of(&before, &after), first);
    }
}

#[test]
fn prefix_column_is_eight_characters_wide() {
    // tests/test_als_golden.py::test_prefix_column_is_eight_characters_wide
    for line in GOLDEN_DIFF {
        let bytes = line.as_bytes();
        assert_eq!(bytes[7], b' ', "column 8 must be a space in {line:?}");
        assert_ne!(bytes[8], b' ', "double gutter in {line:?}");
    }
}

#[test]
fn identical_sets_report_no_changes() {
    let (before, _after) = kitchen_sink();
    assert!(diff_of(&before, &before).is_empty());
}
