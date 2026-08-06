//! Shared semantic model for Wit's diff engine.
//!
//! Zero I/O: `wit-model` defines the vocabulary every parser (`wit-als`,
//! later `wit-logic`) targets and every consumer (`wit-diff`, `wit-share`,
//! the app's IPC layer, the golden tests) shares — one definition, not five
//! drifting ones. That also means it compiles to WASM later if the viewer
//! ever needs it.
//!
//! `BTreeMap` is used everywhere instead of `HashMap` so iteration order is
//! deterministic by construction — see the golden-parity guardrail in
//! wit-planning/PLAN.md (M1): Rust `HashMap` iteration order is randomized
//! per-process, which is exactly the nondeterminism bug (M1 named bug fix 2,
//! `PYTHONHASHSEED`) being ported *away* from, not reintroduced.
//!
//! This is the full M1 model — see `model.rs` for the types, `change.rs`
//! for the diff vocabulary and renderer, and `fmt.rs` for the Python-parity
//! number formatting the golden tests depend on.

mod change;
mod fmt;
mod model;

pub use change::{render_text, ChangeRecord, MixField, MixValue};
pub use fmt::{fmt_num, round3, round_places};
pub use model::{Clip, Device, Fingerprint, Model, Track, TrackId, TrackKind};

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn with_track(model: &mut Model, id: &str, name: &str, kind: TrackKind) {
        model.tracks.insert(
            TrackId::from(id),
            Track {
                id: TrackId::from(id),
                name: name.to_string(),
                kind,
                color: None,
                volume: None,
                pan: None,
                speaker: None,
                devices: vec![],
                clips: BTreeMap::new(),
                automation_lanes: 0,
                notes: 0,
            },
        );
    }

    #[test]
    fn track_added_renders_one_sentence() {
        let mut model = Model::default();
        with_track(&mut model, "104", "Lead Vox", TrackKind::Audio);
        let records = vec![ChangeRecord::TrackAdded {
            name: model.tracks[&TrackId::from("104")].name.clone(),
            kind: TrackKind::Audio,
        }];
        assert_eq!(
            render_text(&records),
            "TRACK+  added 'Lead Vox' (AudioTrack)"
        );
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
            "TRACK-  removed 'Scratch'\nTRACK+  added 'Bridge Gtr' (AudioTrack)"
        );
    }
}
