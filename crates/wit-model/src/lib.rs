//! Shared semantic model for Wit's diff engine.
//!
//! Zero I/O: `wit-model` defines the vocabulary every parser (`wit-als`,
//! `wit-logic`) targets and every consumer (`wit-diff`, `wit-share`, the
//! app's IPC layer, the golden tests) shares — one definition, not five
//! drifting ones. That also means it compiles to WASM later if the viewer
//! ever needs it.
//!
//! `BTreeMap` is used everywhere instead of `HashMap` so iteration order is
//! deterministic by construction — see the golden-parity guardrail in
//! wit-planning/PLAN.md (M1): Rust `HashMap` iteration order is randomized
//! per-process, which is exactly the nondeterminism bug being ported away
//! from (`PYTHONHASHSEED`) rather than reintroduced.
//!
//! This is the M0 skeleton: enough of the shape to prove the crate compiles,
//! tests deterministically, and gives `wit-diff` something real to depend
//! on. The full model (devices, clips, automation, units translation) lands
//! with the M1 Ableton port and the M2 Logic walker.

use std::collections::BTreeMap;

/// A project's structure at one point in time — one save, one backup, one
/// Alternative.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Model {
    /// Tempo in BPM, if the parser read one. `None` means "not read", never
    /// "zero" — a missing tempo must never render as a change to 0 BPM.
    pub tempo_bpm: Option<f64>,
    /// Track id -> track. Keyed and iterated in id order.
    pub tracks: BTreeMap<String, Track>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Track {
    pub id: String,
    pub name: String,
    pub kind: TrackKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackKind {
    Audio,
    Midi,
    Group,
    Return,
}

impl TrackKind {
    fn noun(self) -> &'static str {
        match self {
            TrackKind::Audio => "audio track",
            TrackKind::Midi => "MIDI track",
            TrackKind::Group => "group track",
            TrackKind::Return => "return track",
        }
    }
}

/// One line of a human-readable diff. Each variant renders to exactly one
/// plain sentence via [`render_text`] — never a raw field dump, never an
/// internal id (see PLAN.md "Vocabulary": banned words are product
/// decisions). The full vocabulary (14+ record kinds, device fingerprints,
/// EmptyVerdict) lands with the M1 Ableton port; this three-variant
/// skeleton is what M0 needs to prove the shape compiles and renders
/// deterministically.
#[derive(Debug, Clone, PartialEq)]
pub enum ChangeRecord {
    /// A track was added.
    TrackAdded { name: String, kind: TrackKind },
    /// A track was removed.
    TrackRemoved { name: String, kind: TrackKind },
    /// The project tempo changed.
    TempoChanged { from_bpm: f64, to_bpm: f64 },
}

/// Render change records as the plain sentences a musician reads — the
/// product's only vocabulary. Deterministic: calling this twice on the same
/// input always produces the same string, byte for byte.
pub fn render_text(records: &[ChangeRecord]) -> String {
    records
        .iter()
        .map(render_one)
        .collect::<Vec<_>>()
        .join("\n")
}

fn render_one(record: &ChangeRecord) -> String {
    match record {
        ChangeRecord::TrackAdded { name, kind } => {
            format!("added {} '{name}'", kind.noun())
        }
        ChangeRecord::TrackRemoved { name, kind } => {
            format!("removed {} '{name}'", kind.noun())
        }
        ChangeRecord::TempoChanged { from_bpm, to_bpm } => {
            format!("tempo {} -> {} BPM", fmt_num(*from_bpm), fmt_num(*to_bpm))
        }
    }
}

/// Format a float the way Python's `repr()` does: integral values keep a
/// trailing `.0` ("124.0", never "124"). Rust's `Display` for `f64` prints
/// `124.0` as `124`, so the M1 golden-parity port against
/// `tests/test_als_golden.py`'s `GOLDEN_DIFF` literals needs exactly this
/// behavior. Stubbed here as the shared home for it; round-half-even
/// quantization (Python's `round()` is banker's rounding, `f64::round` is
/// half-away-from-zero) is added when the golden port lands in M1 — see the
/// M1 guardrail in wit-planning/PLAN.md. Do not add rounding here without
/// that guardrail's test table; a plausible-looking half-away-from-zero
/// `round()` call is exactly the bug the guardrail exists to prevent.
pub fn fmt_num(x: f64) -> String {
    if x.is_finite() && x.fract() == 0.0 {
        format!("{x:.1}")
    } else {
        format!("{x}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn track_added_renders_one_sentence() {
        let records = vec![ChangeRecord::TrackAdded {
            name: "Lead Vox".to_string(),
            kind: TrackKind::Audio,
        }];
        assert_eq!(render_text(&records), "added audio track 'Lead Vox'");
    }

    #[test]
    fn track_removed_renders_one_sentence() {
        let records = vec![ChangeRecord::TrackRemoved {
            name: "Scratch".to_string(),
            kind: TrackKind::Midi,
        }];
        assert_eq!(render_text(&records), "removed MIDI track 'Scratch'");
    }

    #[test]
    fn tempo_changed_keeps_python_style_trailing_zero() {
        let records = vec![ChangeRecord::TempoChanged {
            from_bpm: 120.0,
            to_bpm: 124.0,
        }];
        assert_eq!(render_text(&records), "tempo 120.0 -> 124.0 BPM");
    }

    #[test]
    fn multiple_records_render_deterministically_across_runs() {
        let records = vec![
            ChangeRecord::TrackRemoved {
                name: "Scratch".to_string(),
                kind: TrackKind::Midi,
            },
            ChangeRecord::TrackAdded {
                name: "Bridge Gtr".to_string(),
                kind: TrackKind::Audio,
            },
        ];
        let first = render_text(&records);
        let second = render_text(&records);
        assert_eq!(first, second);
        assert_eq!(
            first,
            "removed MIDI track 'Scratch'\nadded audio track 'Bridge Gtr'"
        );
    }

    #[test]
    fn fmt_num_matches_python_repr_for_integral_and_fractional_values() {
        assert_eq!(fmt_num(124.0), "124.0");
        assert_eq!(fmt_num(0.0), "0.0");
        assert_eq!(fmt_num(0.794), "0.794");
        assert_eq!(fmt_num(-0.5), "-0.5");
    }
}
