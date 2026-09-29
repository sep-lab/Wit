//! Lookup tables that turn Ableton's internal names into what a musician
//! sees in Live's own UI: device class tags, root notes and scale names.
//! Every table names exactly where it came from, per AGENTS.md's rule that
//! a claim is measured, cited, or inferred — never blurred.

/// Live's device class tag -> the name Live's own device browser shows for
/// it. **\[measured\]**: for every entry, a factory preset shipped inside
/// `/Applications/Ableton Live 12 Suite.app/Contents/App-Resources/Core
/// Library/Devices/<category>/<display name>/` was decompressed (presets
/// are gzipped XML, exactly like a `.als`) and its root child element's tag
/// read directly — the folder name Live itself uses is the display name,
/// and the preset's own tag is the class Wit sees in a project. Checked on
/// this machine, Ableton Live 12 Suite, 2026-09-29.
///
/// A tag not in this table keeps rendering as its raw tag (whitelist, never
/// a guess) — `wit-story`'s Ableton sentences fall back to it, and the
/// golden CLI output (`render_text`) never consults this table at all, so
/// it stays byte-identical regardless of what's added here.
///
/// Two corrections to keep in mind if this table is extended: the modern
/// "Auto Filter" device (`AutoFilter2`) is a *different* tag from the
/// Legacy category's `AutoFilter`, and "Compressor" is `Compressor2`, not
/// `Compressor` — Live 12 has no bare `Compressor` tag among its factory
/// presets.
const DEVICE_DISPLAY_NAMES: &[(&str, &str)] = &[
    // Audio Effects
    ("Amp", "Amp"),
    ("AutoFilter2", "Auto Filter"),
    ("AutoPan2", "Auto Pan-Tremolo"),
    ("AutoShift", "Auto Shift"),
    ("BeatRepeat", "Beat Repeat"),
    ("Cabinet", "Cabinet"),
    ("ChannelEq", "Channel EQ"),
    ("Chorus2", "Chorus-Ensemble"),
    ("Compressor2", "Compressor"),
    ("Corpus", "Corpus"),
    ("Delay", "Delay"),
    ("DrumBuss", "Drum Buss"),
    ("Tube", "Dynamic Tube"),
    ("Eq8", "EQ Eight"),
    ("FilterEQ3", "EQ Three"),
    ("Echo", "Echo"),
    ("Erosion2", "Erosion"),
    ("FilterDelay", "Filter Delay"),
    ("Gate", "Gate"),
    ("GlueCompressor", "Glue Compressor"),
    ("GrainDelay", "Grain Delay"),
    ("Hybrid", "Hybrid Reverb"),
    ("Limiter", "Limiter"),
    ("Looper", "Looper"),
    ("MultibandDynamics", "Multiband Dynamics"),
    ("Overdrive", "Overdrive"),
    ("Pedal", "Pedal"),
    ("PhaserNew", "Phaser-Flanger"),
    ("Redux2", "Redux"),
    ("Resonator", "Resonators"),
    ("Reverb", "Reverb"),
    ("Roar", "Roar"),
    ("Saturator", "Saturator"),
    ("Shifter", "Shifter"),
    ("Transmute", "Spectral Resonator"),
    ("Spectral", "Spectral Time"),
    ("SpectrumAnalyzer", "Spectrum"),
    ("StereoGain", "Utility"),
    ("Vinyl", "Vinyl Distortion"),
    ("Vocoder", "Vocoder"),
    // Audio Effects — Legacy category (superseded by the tag above with the
    // same display name, but still opened from old projects).
    ("AutoFilter", "Auto Filter (Legacy)"),
    ("AutoPan", "Auto Pan (Legacy)"),
    ("Chorus", "Chorus (Legacy)"),
    ("Erosion", "Erosion (Legacy)"),
    ("Flanger", "Flanger (Legacy)"),
    ("FrequencyShifter", "Frequency Shifter (Legacy)"),
    ("Phaser", "Phaser (Legacy)"),
    ("Redux", "Redux (Legacy)"),
    // MIDI Effects
    ("MidiArpeggiator", "Arpeggiator"),
    ("MidiChord", "Chord"),
    ("MidiNoteLength", "Note Length"),
    ("MidiPitcher", "Pitch"),
    ("MidiRandom", "Random"),
    ("MidiScale", "Scale"),
    ("MidiVelocity", "Velocity"),
    // Instruments
    ("UltraAnalog", "Analog"),
    ("Collision", "Collision"),
    ("Drift", "Drift"),
    ("LoungeLizard", "Electric"),
    ("InstrumentImpulse", "Impulse"),
    ("InstrumentMeld", "Meld"),
    ("Operator", "Operator"),
    ("MultiSampler", "Sampler"),
    ("OriginalSimpler", "Simpler"),
    ("StringStudio", "Tension"),
    ("InstrumentVector", "Wavetable"),
    // Container/rack devices — **\[measured\]** on the real 29-file Backup
    // chain (`WIT_FIXTURES`): these are the tags Live 12 actually writes
    // for a rack in a project, not the `.adg` preset wrapper tag
    // (`GroupDevicePreset`), which is a different, preset-only shape.
    ("AudioEffectGroupDevice", "Audio Effect Rack"),
    ("InstrumentGroupDevice", "Instrument Rack"),
    ("DrumGroupDevice", "Drum Rack"),
    ("MidiEffectGroupDevice", "MIDI Effect Rack"),
];

/// Live's device tag -> the name Live's own UI shows, or the tag itself
/// when it isn't in [`DEVICE_DISPLAY_NAMES`] — whitelist extraction never
/// blocks on an unknown tag (AGENTS.md).
pub fn device_display_name(tag: &str) -> &str {
    DEVICE_DISPLAY_NAMES
        .iter()
        .find(|(t, _)| *t == tag)
        .map(|(_, name)| *name)
        .unwrap_or(tag)
}

/// MIDI-style pitch-class order (0 = C, 1 = C#, ... 11 = B). **\[inferred\]**
/// from the near-universal chromatic-index convention, not independently
/// verified against a real file: every file in the real 29-file Backup
/// chain has the scale feature switched off (`InKey=false`), so `Root`
/// never varies from its default `0` and there is nothing in the corpus to
/// check this table against. A public, tested parser
/// (github.com/owenbush/ableton-inspector, MIT licensed) uses the same
/// ordering, which corroborates but does not independently confirm it.
const ROOT_NOTE_NAMES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

/// Live's `ScaleInformation/Name` enum, in index order. **\[cited\]**:
/// github.com/owenbush/ableton-inspector (MIT licensed; its README claims
/// testing against real Live 11 and 12 projects), `packages/core/src/
/// constants/scales.ts`, fetched 2026-09-29. Not independently verified
/// against Wit's own real material for the same reason as
/// [`ROOT_NOTE_NAMES`] — no file in the real chain has the scale feature on.
const SCALE_NAMES: [&str; 35] = [
    "Major",
    "Minor",
    "Dorian",
    "Mixolydian",
    "Lydian",
    "Phrygian",
    "Locrian",
    "Whole Tone",
    "Half-whole Dim.",
    "Whole-half Dim.",
    "Minor Blues",
    "Minor Pentatonic",
    "Major Pentatonic",
    "Harmonic Minor",
    "Harmonic Major",
    "Dorian #4",
    "Phrygian Dominant",
    "Melodic Minor",
    "Lydian Augmented",
    "Lydian Dominant",
    "Super Locrian",
    "8-Tone Spanish",
    "Bhairav",
    "Hungarian Minor",
    "Hirajoshi",
    "In-Sen",
    "Iwato",
    "Kumoi",
    "Pelog Selisir",
    "Pelog Tembung",
    "Messiaen 3",
    "Messiaen 4",
    "Messiaen 5",
    "Messiaen 6",
    "Messiaen 7",
];

/// Render a project key as "C minor" from Ableton's raw `Root`/`Name` enum
/// indices, or `None` if either index is out of the known table's range —
/// an index this table can't map is left unrendered rather than guessed
/// (whitelist, not blacklist).
pub fn key_label(root: u32, name: u32) -> Option<String> {
    let root_name = ROOT_NOTE_NAMES.get(root as usize)?;
    let scale_name = SCALE_NAMES.get(name as usize)?;
    Some(format!("{root_name} {}", scale_name.to_lowercase()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_tags_map_to_lives_own_display_names() {
        assert_eq!(device_display_name("Eq8"), "EQ Eight");
        assert_eq!(device_display_name("AutoFilter2"), "Auto Filter");
        assert_eq!(device_display_name("Compressor2"), "Compressor");
        assert_eq!(device_display_name("DrumGroupDevice"), "Drum Rack");
    }

    #[test]
    fn a_legacy_tag_is_distinct_from_its_modern_replacement() {
        assert_eq!(device_display_name("AutoFilter"), "Auto Filter (Legacy)");
        assert_eq!(device_display_name("AutoFilter2"), "Auto Filter");
    }

    #[test]
    fn an_unmapped_tag_keeps_its_own_name() {
        assert_eq!(device_display_name("SomeFutureDevice"), "SomeFutureDevice");
    }

    #[test]
    fn key_label_renders_root_and_scale() {
        assert_eq!(key_label(0, 1).as_deref(), Some("C minor"));
        assert_eq!(key_label(9, 0).as_deref(), Some("A major"));
    }

    #[test]
    fn key_label_is_none_past_the_known_range() {
        assert_eq!(key_label(0, 999), None);
        assert_eq!(key_label(999, 0), None);
    }
}
