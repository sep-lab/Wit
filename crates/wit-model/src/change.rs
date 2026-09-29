//! One line of a human-readable diff. Each variant renders to exactly one
//! plain sentence via [`render_text`] — never a raw field dump, never an
//! internal id (PLAN.md "Vocabulary": banned words are product decisions).
//!
//! The full vocabulary and exact wording is a **characterization contract**
//! ported field-for-field from `tests/test_als_golden.py`'s `GOLDEN_DIFF` —
//! see that file's module docstring for why: "the diff output *is* the
//! product... docs/EXPERIMENTS.md quotes it verbatim... a musician reads
//! it." Changing a prefix, the gutter width, or the field order here is a
//! product decision, not a refactor.

use crate::fmt_num;

#[derive(Debug, Clone, PartialEq)]
pub enum ChangeRecord {
    TempoChanged {
        from_bpm: f64,
        to_bpm: f64,
    },
    /// A sample was renamed and every reference to it moved — the coalesced
    /// form of what would otherwise be one `ClipSampleReplaced` line per
    /// clip (`docs/EXPERIMENTS.md`: 425 raw clip changes collapse to 3
    /// semantic lines). See `wit-diff`'s rename-bijection guard (M1 named
    /// bug fix 3) for what makes a transition eligible to coalesce.
    SampleRenamed {
        old: String,
        new: String,
        count: usize,
    },
    TrackAdded {
        name: String,
        kind: crate::TrackKind,
    },
    TrackRemoved {
        name: String,
        kind: crate::TrackKind,
    },
    TrackRenamed {
        old_name: String,
        new_name: String,
    },
    MixChanged {
        track: String,
        field: MixField,
        from: MixValue,
        to: MixValue,
    },
    FxAdded {
        track: String,
        devices: Vec<String>,
    },
    FxRemoved {
        track: String,
        devices: Vec<String>,
    },
    FxReordered {
        track: String,
    },
    /// New in the Rust port (M1 named bug fix 3, the knob-turn false
    /// negative): the device chain's tags and order are unchanged, but this
    /// device's parameter fingerprint differs. See [`crate::Device`].
    FxSettingsChanged {
        track: String,
        device: String,
    },
    AutomationLanesChanged {
        track: String,
        from: usize,
        to: usize,
    },
    NoteCountChanged {
        track: String,
        from: usize,
        to: usize,
    },
    /// `start_bar` carries Ableton's `CurrentStart` verbatim, which is in
    /// **beats**, not bars. The golden text says "at bar" because the frozen
    /// prototype (`als_semantic_diff.py`) does, and the golden literals are
    /// the spec — so `render_text` keeps the word. Anything musician-facing
    /// (`wit-story`) converts beats to real bars with the time signature.
    ClipAdded {
        track: String,
        label: String,
        start_bar: f64,
    },
    ClipRemoved {
        track: String,
        label: String,
        start_bar: f64,
    },
    ClipRangeChanged {
        track: String,
        label: String,
        from_start: f64,
        from_end: f64,
        to_start: f64,
        to_end: f64,
    },
    ClipMuteChanged {
        track: String,
        label: String,
        muted: bool,
    },
    /// New in the Rust port (M1 named bug fix 3): the fallback for a sample
    /// transition that looked like a rename per-clip but failed the
    /// bijection guard — the old basename is still referenced somewhere in
    /// the new model, so it was not a rename (a swap, or a partial
    /// reassignment). `als_semantic_diff.py` has no line for this case at
    /// all; it just misreports the transition as `SAMPLE~`.
    ClipSampleReplaced {
        track: String,
        label: String,
        old: String,
        new: String,
    },

    // ---- PLAN-V2 Story contract (2026-09-29): DAW-neutral structure ----
    //
    // Added for Logic, GarageBand and FL, which have no Ableton-shaped
    // `Model`, and for Ableton fields the M1 port didn't model. None of these
    // is in `tests/test_als_golden.py`'s GOLDEN_DIFF, so their `render_text`
    // wording below is new, not ported. A parser emits one only when it read
    // the value from the file; it never guesses.
    /// The project's key or scale. `None` means "not set in the project",
    /// never "could not read" (a parser that can't read it emits nothing).
    KeyChanged {
        from: Option<String>,
        to: Option<String>,
    },
    TimeSignatureChanged {
        from: TimeSignature,
        to: TimeSignature,
    },
    /// The track count **as the DAW itself states it** — Logic's
    /// `MetaData.plist` `NumberOfTracks`. Never a container census count:
    /// ADR-0006 bans rendering a record count with a musician noun (a real
    /// project with 35 tracks has ~250 `Trak` records). A parser that can't
    /// read the DAW's own count must not emit this.
    TrackCountChanged {
        from: usize,
        to: usize,
    },
    /// A region (Logic) or clip-like part (FL pattern placement) appeared.
    /// `track` is `None` when the parser can't yet tie it to a track.
    RegionAdded {
        track: Option<String>,
        name: String,
        at: Option<BarPos>,
    },
    RegionRemoved {
        track: Option<String>,
        name: String,
        at: Option<BarPos>,
    },
    RegionMoved {
        track: Option<String>,
        name: String,
        from: BarPos,
        to: BarPos,
    },
    /// Length changed, in bars (the region kept its identity — same Logic
    /// region UUID, same Ableton clip id).
    RegionTrimmed {
        track: Option<String>,
        name: String,
        from_bars: f64,
        to_bars: f64,
    },
    /// A new region that is a copy of an existing one (same source, new
    /// identity).
    RegionDuplicated {
        track: Option<String>,
        name: String,
        at: Option<BarPos>,
    },
    /// An audio file joined the project's file list (Logic `AuFl`,
    /// basename only — never a path).
    AudioFileAdded {
        name: String,
    },
    AudioFileRemoved {
        name: String,
    },
    /// A marker (Logic) or locator (Ableton) — the song's section names.
    MarkerAdded {
        name: String,
        at: Option<BarPos>,
    },
    MarkerRemoved {
        name: String,
    },
    MarkerRenamed {
        old: String,
        new: String,
    },
    /// A plugin (instrument or effect) was inserted. Distinct from
    /// [`ChangeRecord::FxAdded`], which is Ableton's device-chain line and
    /// part of the golden vocabulary. Plugin state stays opaque (ADR-0003):
    /// this names the plugin, never its settings.
    PluginAdded {
        track: Option<String>,
        plugin: String,
    },
    PluginRemoved {
        track: Option<String>,
        plugin: String,
    },
}

/// A position in the song, in 1-based bars. `approximate` is true when the
/// parser had to assume something to get from its native unit to bars — a
/// mid-song tempo or meter change, or a time signature it couldn't read —
/// and the Story then says "about bar N".
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BarPos {
    pub bar: f64,
    pub approximate: bool,
}

impl BarPos {
    pub fn exact(bar: f64) -> Self {
        BarPos {
            bar,
            approximate: false,
        }
    }

    pub fn about(bar: f64) -> Self {
        BarPos {
            bar,
            approximate: true,
        }
    }

    fn render(self) -> String {
        if self.approximate {
            format!("~{}", fmt_num(self.bar))
        } else {
            fmt_num(self.bar)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeSignature {
    pub numerator: u16,
    pub denominator: u16,
}

impl std::fmt::Display for TimeSignature {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.numerator, self.denominator)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MixField {
    Volume,
    Pan,
    OutputEnabled,
    Color,
}

impl MixField {
    fn label(self) -> &'static str {
        match self {
            MixField::Volume => "volume",
            MixField::Pan => "pan",
            MixField::OutputEnabled => "output enabled",
            MixField::Color => "color",
        }
    }
}

/// A `MIX~` field's before/after value. `Num` renders through [`fmt_num`]
/// (volume, pan); `Text` renders verbatim — Ableton's raw XML string, not a
/// re-parsed type (`speaker`'s "true"/"false", `color`'s swatch index "13").
#[derive(Debug, Clone, PartialEq)]
pub enum MixValue {
    Num(f64),
    Text(String),
}

impl MixValue {
    fn render(&self) -> String {
        match self {
            MixValue::Num(x) => fmt_num(*x),
            MixValue::Text(s) => s.clone(),
        }
    }
}

/// Render change records as the plain sentences a musician reads — the
/// product's only vocabulary. Deterministic: calling this twice on the same
/// input always produces the same string, byte for byte (M1 named bug fix
/// 2 — see `wit-diff` for where the ordering itself is made deterministic;
/// this function only renders the order it is given).
pub fn render_text(records: &[ChangeRecord]) -> String {
    records
        .iter()
        .map(render_one)
        .collect::<Vec<_>>()
        .join("\n")
}

/// Every prefix is left-padded to an 8-column gutter
/// (`tests/test_als_golden.py::test_prefix_column_is_eight_characters_wide`).
/// `"SAMPLE~"` is 7 characters and gets exactly one padding space; everything
/// else is shorter and gets more — no special-casing needed, `{:<8}` does
/// the right thing for both.
fn line(prefix: &str, rest: impl AsRef<str>) -> String {
    format!("{prefix:<8}{}", rest.as_ref())
}

fn render_one(record: &ChangeRecord) -> String {
    match record {
        ChangeRecord::TempoChanged { from_bpm, to_bpm } => line(
            "TEMPO",
            format!("{} -> {} BPM", fmt_num(*from_bpm), fmt_num(*to_bpm)),
        ),
        ChangeRecord::SampleRenamed { old, new, count } => line(
            "SAMPLE~",
            format!("'{old}' -> '{new}'  ({count} clip reference(s))"),
        ),
        ChangeRecord::TrackAdded { name, kind } => {
            line("TRACK+", format!("added '{name}' ({})", kind.xml_tag()))
        }
        ChangeRecord::TrackRemoved { name, .. } => line("TRACK-", format!("removed '{name}'")),
        ChangeRecord::TrackRenamed { old_name, new_name } => {
            line("TRACK~", format!("renamed '{old_name}' -> '{new_name}'"))
        }
        ChangeRecord::MixChanged {
            track,
            field,
            from,
            to,
        } => line(
            "MIX~",
            format!(
                "[{track}] {}: {} -> {}",
                field.label(),
                from.render(),
                to.render()
            ),
        ),
        ChangeRecord::FxAdded { track, devices } => {
            line("FX+", format!("[{track}] added: {}", devices.join(", ")))
        }
        ChangeRecord::FxRemoved { track, devices } => {
            line("FX-", format!("[{track}] removed: {}", devices.join(", ")))
        }
        ChangeRecord::FxReordered { track } => {
            line("FX~", format!("[{track}] device chain reordered"))
        }
        ChangeRecord::FxSettingsChanged { track, device } => {
            line("FX~", format!("[{track}] {device} settings changed"))
        }
        ChangeRecord::AutomationLanesChanged { track, from, to } => line(
            "AUTO~",
            format!("[{track}] automation lanes {from} -> {to}"),
        ),
        ChangeRecord::NoteCountChanged { track, from, to } => {
            line("MIDI~", format!("[{track}] note count {from} -> {to}"))
        }
        ChangeRecord::ClipAdded {
            track,
            label,
            start_bar,
        } => line(
            "CLIP+",
            format!("[{track}] added '{label}' at bar {}", fmt_num(*start_bar)),
        ),
        ChangeRecord::ClipRemoved {
            track,
            label,
            start_bar,
        } => line(
            "CLIP-",
            format!("[{track}] removed '{label}' at bar {}", fmt_num(*start_bar)),
        ),
        ChangeRecord::ClipRangeChanged {
            track,
            label,
            from_start,
            from_end,
            to_start,
            to_end,
        } => line(
            "CLIP~",
            format!(
                "[{track}] '{label}' {}-{} -> {}-{}",
                fmt_num(*from_start),
                fmt_num(*from_end),
                fmt_num(*to_start),
                fmt_num(*to_end)
            ),
        ),
        ChangeRecord::ClipMuteChanged {
            track,
            label,
            muted,
        } => line(
            "CLIP~",
            format!(
                "[{track}] '{label}' {}",
                if *muted { "muted" } else { "unmuted" }
            ),
        ),
        ChangeRecord::ClipSampleReplaced {
            track,
            label,
            old,
            new,
        } => line(
            "CLIP~",
            format!("[{track}] '{label}' sample replaced: '{old}' -> '{new}'"),
        ),
        ChangeRecord::KeyChanged { from, to } => line(
            "KEY",
            format!(
                "{} -> {}",
                quoted_or_none(from.as_deref()),
                quoted_or_none(to.as_deref())
            ),
        ),
        ChangeRecord::TimeSignatureChanged { from, to } => line("METER", format!("{from} -> {to}")),
        ChangeRecord::TrackCountChanged { from, to } => line("TRACKS", format!("{from} -> {to}")),
        ChangeRecord::RegionAdded { track, name, at } => line(
            "REGION+",
            format!("{}added '{name}'{}", on_track(track), at_bar(at)),
        ),
        ChangeRecord::RegionRemoved { track, name, at } => line(
            "REGION-",
            format!("{}removed '{name}'{}", on_track(track), at_bar(at)),
        ),
        ChangeRecord::RegionMoved {
            track,
            name,
            from,
            to,
        } => line(
            "REGION~",
            format!(
                "{}'{name}' moved bar {} -> {}",
                on_track(track),
                from.render(),
                to.render()
            ),
        ),
        ChangeRecord::RegionTrimmed {
            track,
            name,
            from_bars,
            to_bars,
        } => line(
            "REGION~",
            format!(
                "{}'{name}' length {} -> {} bars",
                on_track(track),
                fmt_num(*from_bars),
                fmt_num(*to_bars)
            ),
        ),
        ChangeRecord::RegionDuplicated { track, name, at } => line(
            "REGION+",
            format!("{}'{name}' duplicated{}", on_track(track), at_bar(at)),
        ),
        ChangeRecord::AudioFileAdded { name } => line("AUDIO+", format!("added '{name}'")),
        ChangeRecord::AudioFileRemoved { name } => line("AUDIO-", format!("removed '{name}'")),
        ChangeRecord::MarkerAdded { name, at } => {
            line("MARKER+", format!("'{name}'{}", at_bar(at)))
        }
        ChangeRecord::MarkerRemoved { name } => line("MARKER-", format!("'{name}'")),
        ChangeRecord::MarkerRenamed { old, new } => line("MARKER~", format!("'{old}' -> '{new}'")),
        ChangeRecord::PluginAdded { track, plugin } => {
            line("PLUGIN+", format!("{}added '{plugin}'", on_track(track)))
        }
        ChangeRecord::PluginRemoved { track, plugin } => {
            line("PLUGIN-", format!("{}removed '{plugin}'", on_track(track)))
        }
    }
}

fn quoted_or_none(value: Option<&str>) -> String {
    match value {
        Some(v) => format!("'{v}'"),
        None => "none".to_string(),
    }
}

fn on_track(track: &Option<String>) -> String {
    match track {
        Some(t) => format!("[{t}] "),
        None => String::new(),
    }
}

fn at_bar(at: &Option<BarPos>) -> String {
    match at {
        Some(p) => format!(" at bar {}", p.render()),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TrackKind;

    #[test]
    fn every_prefix_is_padded_to_an_eight_column_gutter() {
        let cases = vec![
            (
                ChangeRecord::TempoChanged {
                    from_bpm: 120.0,
                    to_bpm: 124.0,
                },
                "TEMPO   120.0 -> 124.0 BPM",
            ),
            (
                ChangeRecord::SampleRenamed {
                    old: "old kick.wav".into(),
                    new: "Kick 01.wav".into(),
                    count: 1,
                },
                "SAMPLE~ 'old kick.wav' -> 'Kick 01.wav'  (1 clip reference(s))",
            ),
            (
                ChangeRecord::TrackAdded {
                    name: "Vox".into(),
                    kind: TrackKind::Audio,
                },
                "TRACK+  added 'Vox' (AudioTrack)",
            ),
            (
                ChangeRecord::TrackRemoved {
                    name: "Scratch".into(),
                    kind: TrackKind::Audio,
                },
                "TRACK-  removed 'Scratch'",
            ),
        ];
        for (record, expected) in cases {
            assert_eq!(render_text(&[record]), expected);
        }
    }

    #[test]
    fn story_contract_variants_render_in_the_same_gutter() {
        let cases = vec![
            (
                ChangeRecord::KeyChanged {
                    from: None,
                    to: Some("C minor".into()),
                },
                "KEY     none -> 'C minor'",
            ),
            (
                ChangeRecord::TimeSignatureChanged {
                    from: TimeSignature {
                        numerator: 4,
                        denominator: 4,
                    },
                    to: TimeSignature {
                        numerator: 3,
                        denominator: 4,
                    },
                },
                "METER   4/4 -> 3/4",
            ),
            (
                ChangeRecord::TrackCountChanged { from: 26, to: 27 },
                "TRACKS  26 -> 27",
            ),
            (
                ChangeRecord::RegionAdded {
                    track: None,
                    name: "Chorus Rhodes".into(),
                    at: None,
                },
                "REGION+ added 'Chorus Rhodes'",
            ),
            (
                ChangeRecord::RegionMoved {
                    track: Some("Bass".into()),
                    name: "havoc bass".into(),
                    from: BarPos::exact(7.0),
                    to: BarPos::about(9.0),
                },
                "REGION~ [Bass] 'havoc bass' moved bar 7.0 -> ~9.0",
            ),
            (
                ChangeRecord::RegionTrimmed {
                    track: None,
                    name: "Beat 01".into(),
                    from_bars: 16.0,
                    to_bars: 12.0,
                },
                "REGION~ 'Beat 01' length 16.0 -> 12.0 bars",
            ),
            (
                ChangeRecord::AudioFileAdded {
                    name: "Brushed Kit 124.caf".into(),
                },
                "AUDIO+  added 'Brushed Kit 124.caf'",
            ),
            (
                ChangeRecord::MarkerAdded {
                    name: "Chorus".into(),
                    at: Some(BarPos::exact(17.0)),
                },
                "MARKER+ 'Chorus' at bar 17.0",
            ),
            (
                ChangeRecord::PluginAdded {
                    track: Some("Vox".into()),
                    plugin: "Channel EQ".into(),
                },
                "PLUGIN+ [Vox] added 'Channel EQ'",
            ),
        ];
        for (record, expected) in cases {
            assert_eq!(render_text(&[record]), expected);
        }
    }
}
