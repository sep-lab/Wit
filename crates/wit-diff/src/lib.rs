//! `diff(Model, Model) -> Vec<ChangeRecord>`.
//!
//! Depends only on `wit-model`, so diff logic is testable without any parser
//! (`wit-als`, `wit-logic`) in the loop — see wit-planning/PLAN.md
//! workspace_layout. This is the M0 skeleton (tracks + tempo only); rename
//! coalescing, device fingerprints, `aggregate()` for Since-ranges, and
//! `EmptyVerdict` land with the M1 Ableton port and M2 Logic walker.

use wit_model::{ChangeRecord, Model};

/// Compare two models and return the ordered list of human-facing change
/// records. Removed tracks are reported before added tracks, matching the
/// prototype's `TRACK-` / `TRACK+` ordering in `experiments/als_semantic_diff.py`.
pub fn diff(old: &Model, new: &Model) -> Vec<ChangeRecord> {
    let mut records = Vec::new();

    if let (Some(from_bpm), Some(to_bpm)) = (old.tempo_bpm, new.tempo_bpm) {
        if from_bpm != to_bpm {
            records.push(ChangeRecord::TempoChanged { from_bpm, to_bpm });
        }
    }

    // BTreeMap iteration is already id-ordered, so this stays deterministic
    // without an explicit sort key. TrackId ordering becomes a deliberate,
    // documented divergence from Python's lexicographic `sorted()` once real
    // ids land in M1 — see the M1 guardrail in wit-planning/PLAN.md.
    for (id, track) in &old.tracks {
        if !new.tracks.contains_key(id) {
            records.push(ChangeRecord::TrackRemoved {
                name: track.name.clone(),
                kind: track.kind,
            });
        }
    }
    for (id, track) in &new.tracks {
        if !old.tracks.contains_key(id) {
            records.push(ChangeRecord::TrackAdded {
                name: track.name.clone(),
                kind: track.kind,
            });
        }
    }

    records
}

#[cfg(test)]
mod tests {
    use super::*;
    use wit_model::{Track, TrackKind};

    fn with_track(model: &mut Model, id: &str, name: &str, kind: TrackKind) {
        model.tracks.insert(
            id.to_string(),
            Track {
                id: id.to_string(),
                name: name.to_string(),
                kind,
            },
        );
    }

    #[test]
    fn diff_of_identical_models_is_empty() {
        let mut model = Model::default();
        with_track(&mut model, "1", "Lead Vox", TrackKind::Audio);
        assert!(diff(&model, &model).is_empty());
    }

    #[test]
    fn diff_detects_added_and_removed_tracks_in_order() {
        let mut old = Model::default();
        with_track(&mut old, "1", "Scratch", TrackKind::Midi);

        let mut new = Model::default();
        with_track(&mut new, "2", "Bridge Gtr", TrackKind::Audio);

        let records = diff(&old, &new);
        assert_eq!(
            records,
            vec![
                ChangeRecord::TrackRemoved {
                    name: "Scratch".to_string(),
                    kind: TrackKind::Midi,
                },
                ChangeRecord::TrackAdded {
                    name: "Bridge Gtr".to_string(),
                    kind: TrackKind::Audio,
                },
            ]
        );
    }

    #[test]
    fn diff_detects_tempo_change() {
        let old = Model {
            tempo_bpm: Some(120.0),
            ..Model::default()
        };
        let new = Model {
            tempo_bpm: Some(124.0),
            ..Model::default()
        };
        assert_eq!(
            diff(&old, &new),
            vec![ChangeRecord::TempoChanged {
                from_bpm: 120.0,
                to_bpm: 124.0,
            }]
        );
    }

    #[test]
    fn missing_tempo_on_either_side_never_reports_a_change() {
        // A parser that didn't read tempo must report nothing, never a
        // change to/from 0 BPM — the honesty-tier doctrine applies to the
        // skeleton too.
        let old = Model::default();
        let new = Model {
            tempo_bpm: Some(124.0),
            ..Model::default()
        };
        assert!(diff(&old, &new).is_empty());
    }
}
