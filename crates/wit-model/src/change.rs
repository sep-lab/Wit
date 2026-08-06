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
}
