//! `diff(Model, Model) -> Vec<ChangeRecord>` — the full M1 port of
//! `experiments/als_semantic_diff.py`'s `diff_models()`.
//!
//! Depends only on `wit-model`, so diff logic is testable without any
//! parser (`wit-als`, later `wit-logic`) in the loop.
//!
//! Two of the three M1 named bug fixes that are this crate's responsibility
//! (the third, tempo, belongs to the parser — see `wit-als`):
//!
//! - **Deterministic ordering** (bug 2): `BTreeMap`/`BTreeSet` iteration is
//!   sorted by construction, and the one place Python's original groups by
//!   a derived key — sample renames, grouped by `(old, new)` — is sorted
//!   explicitly by `(-count, old, new)` rather than relying on a stable
//!   sort over a hash-randomized iteration order.
//! - **Rename bijection guard** (bug 3): a sample transition only
//!   coalesces into one `SAMPLE~` line when the old basename is completely
//!   absent from the new model (see [`Model::sample_index`]) — otherwise
//!   it is a swap or a partial reassignment, not a rename, and is reported
//!   per clip as [`ChangeRecord::ClipSampleReplaced`] instead. See
//!   `tests/test_als_rename_coalescing.py`'s two `xfail` tests for the bug
//!   this fixes.

use std::collections::{BTreeMap, BTreeSet};
use wit_model::{ChangeRecord, MixField, MixValue, Model, Track, TrackId};

/// Compare two models and return the ordered list of human-facing change
/// records, in the same block order as `als_semantic_diff.py`'s
/// `diff_models()`: tempo, sample renames, tracks added, tracks removed,
/// then per common track (sorted by [`TrackId`]) — rename, mix fields,
/// device chain, automation, notes, clips added/removed/changed.
pub fn diff(old: &Model, new: &Model) -> Vec<ChangeRecord> {
    let mut records = Vec::new();

    if let (Some(from_bpm), Some(to_bpm)) = (old.tempo_bpm, new.tempo_bpm) {
        if from_bpm != to_bpm {
            records.push(ChangeRecord::TempoChanged { from_bpm, to_bpm });
        }
    }

    let renames = analyze_sample_renames(old, new);
    records.extend(renames.renamed);

    let ka: BTreeSet<&TrackId> = old.tracks.keys().collect();
    let kb: BTreeSet<&TrackId> = new.tracks.keys().collect();

    for id in kb.difference(&ka) {
        let t = &new.tracks[*id];
        records.push(ChangeRecord::TrackAdded {
            name: t.name.clone(),
            kind: t.kind,
        });
    }
    for id in ka.difference(&kb) {
        let t = &old.tracks[*id];
        records.push(ChangeRecord::TrackRemoved {
            name: t.name.clone(),
            kind: t.kind,
        });
    }

    for id in ka.intersection(&kb) {
        let ta = &old.tracks[*id];
        let tb = &new.tracks[*id];
        diff_track(&mut records, ta, tb, &renames.replaced_clips, id);
    }

    // ---- PLAN-V2 Story contract (2026-09-29): DAW-neutral records, always
    // appended after the block above so the M1 golden output — which never
    // exercises any of these — stays byte-identical.

    if let (Some(from), Some(to)) = (old.time_signature, new.time_signature) {
        if from != to {
            records.push(ChangeRecord::TimeSignatureChanged { from, to });
        }
    }

    if old.key != new.key {
        records.push(ChangeRecord::KeyChanged {
            from: old.key.clone(),
            to: new.key.clone(),
        });
    }

    records.extend(diff_markers(old, new));

    for id in ka.intersection(&kb) {
        let tb = &new.tracks[*id];
        records.extend(diff_plugin_names(
            Some(tb.name.as_str()),
            &old.tracks[*id].devices,
            &tb.devices,
        ));
    }

    // The master bus: named "Main" (Live 12.3+) or "Master" (older),
    // whichever side actually found the host element — a save can't change
    // which tag Live wrote without also changing the Live version, and
    // either side is the honest label for that state.
    if let Some(label) = new
        .master_track_label
        .as_deref()
        .or(old.master_track_label.as_deref())
    {
        diff_devices(
            &mut records,
            label,
            &old.master_devices,
            &new.master_devices,
        );
        records.extend(diff_plugin_names(
            Some(label),
            &old.master_devices,
            &new.master_devices,
        ));
    }

    records
}

fn diff_track(
    records: &mut Vec<ChangeRecord>,
    ta: &Track,
    tb: &Track,
    replaced_clips: &BTreeSet<(TrackId, String)>,
    id: &TrackId,
) {
    let label = tb.name.clone();

    if ta.name != tb.name {
        records.push(ChangeRecord::TrackRenamed {
            old_name: ta.name.clone(),
            new_name: tb.name.clone(),
        });
    }

    mix_num(records, &label, MixField::Volume, ta.volume, tb.volume);
    mix_num(records, &label, MixField::Pan, ta.pan, tb.pan);
    mix_text(
        records,
        &label,
        MixField::OutputEnabled,
        ta.speaker.as_deref(),
        tb.speaker.as_deref(),
    );
    mix_text(
        records,
        &label,
        MixField::Color,
        ta.color.as_deref(),
        tb.color.as_deref(),
    );

    diff_devices(records, &label, &ta.devices, &tb.devices);

    if ta.automation_lanes != tb.automation_lanes {
        records.push(ChangeRecord::AutomationLanesChanged {
            track: label.clone(),
            from: ta.automation_lanes,
            to: tb.automation_lanes,
        });
    }
    if ta.notes != tb.notes {
        records.push(ChangeRecord::NoteCountChanged {
            track: label.clone(),
            from: ta.notes,
            to: tb.notes,
        });
    }

    diff_clips(records, &label, ta, tb, replaced_clips, id);
}

fn mix_num(
    records: &mut Vec<ChangeRecord>,
    label: &str,
    field: MixField,
    from: Option<f64>,
    to: Option<f64>,
) {
    if let (Some(a), Some(b)) = (from, to) {
        if a != b {
            records.push(ChangeRecord::MixChanged {
                track: label.to_string(),
                field,
                from: MixValue::Num(a),
                to: MixValue::Num(b),
            });
        }
    }
}

fn mix_text(
    records: &mut Vec<ChangeRecord>,
    label: &str,
    field: MixField,
    from: Option<&str>,
    to: Option<&str>,
) {
    if let (Some(a), Some(b)) = (from, to) {
        if a != b {
            records.push(ChangeRecord::MixChanged {
                track: label.to_string(),
                field,
                from: MixValue::Text(a.to_string()),
                to: MixValue::Text(b.to_string()),
            });
        }
    }
}

/// Device-chain add/remove/reorder/settings-changed, by tag and fingerprint
/// — the M1 Ableton device diff. Takes plain slices, not a [`Track`], so
/// the same logic serves a regular track's chain and the master bus's own
/// chain ([`Model::master_devices`]), which is not itself a `Track`.
fn diff_devices(
    records: &mut Vec<ChangeRecord>,
    label: &str,
    ta: &[wit_model::Device],
    tb: &[wit_model::Device],
) {
    let ta_tags: Vec<&str> = ta.iter().map(|d| d.tag.as_str()).collect();
    let tb_tags: Vec<&str> = tb.iter().map(|d| d.tag.as_str()).collect();
    if ta_tags != tb_tags {
        let added: Vec<String> = tb_tags
            .iter()
            .filter(|d| !ta_tags.contains(d))
            .map(|s| s.to_string())
            .collect();
        let removed: Vec<String> = ta_tags
            .iter()
            .filter(|d| !tb_tags.contains(d))
            .map(|s| s.to_string())
            .collect();
        let (added_empty, removed_empty) = (added.is_empty(), removed.is_empty());
        if !added_empty {
            records.push(ChangeRecord::FxAdded {
                track: label.to_string(),
                devices: added,
            });
        }
        if !removed_empty {
            records.push(ChangeRecord::FxRemoved {
                track: label.to_string(),
                devices: removed,
            });
        }
        if added_empty && removed_empty {
            records.push(ChangeRecord::FxReordered {
                track: label.to_string(),
            });
        }
    } else {
        // Tags and order are identical, so positions align 1:1 — the only
        // way the two chains can still differ is a parameter fingerprint.
        // New in the Rust port (M1 named bug fix 3): what would otherwise
        // be a silent "no musical change" for a same-shape knob turn.
        for (da, db) in ta.iter().zip(tb.iter()) {
            if da.fingerprint != db.fingerprint {
                records.push(ChangeRecord::FxSettingsChanged {
                    track: label.to_string(),
                    device: db.tag.clone(),
                });
            }
        }
    }
}

/// Third-party plugin names added or removed on one track (or the master
/// bus), by set difference over [`wit_model::Device::plugin_name`] — a new
/// record ([`ChangeRecord::PluginAdded`]/[`PluginRemoved`]) alongside the
/// existing tag-based [`ChangeRecord::FxAdded`]/[`FxRemoved`] line
/// `diff_devices` already emits for the same change, since a raw device tag
/// (`"AuPluginDevice"`) names nothing a musician recognises.
fn diff_plugin_names(
    track: Option<&str>,
    ta: &[wit_model::Device],
    tb: &[wit_model::Device],
) -> Vec<ChangeRecord> {
    let mut records = Vec::new();
    let a: BTreeSet<&str> = ta.iter().filter_map(|d| d.plugin_name.as_deref()).collect();
    let b: BTreeSet<&str> = tb.iter().filter_map(|d| d.plugin_name.as_deref()).collect();
    for name in b.difference(&a) {
        records.push(ChangeRecord::PluginAdded {
            track: track.map(str::to_string),
            plugin: name.to_string(),
        });
    }
    for name in a.difference(&b) {
        records.push(ChangeRecord::PluginRemoved {
            track: track.map(str::to_string),
            plugin: name.to_string(),
        });
    }
    records
}

/// Convert an Ableton beat position to a 1-based bar position, using
/// whichever model actually carries a time signature — mirrors
/// `wit_model::beats_per_bar`'s formula, but this crate cannot depend on
/// `wit-story` for the shared `SentenceContext` version of the same
/// conversion (dependency runs the other way), so it is repeated here at
/// the single call site that needs it ([`diff_markers`]).
fn beats_to_bar(beats: f64, sig: Option<wit_model::TimeSignature>) -> wit_model::BarPos {
    match sig {
        Some(sig) if sig.numerator > 0 && sig.denominator > 0 => {
            let bpb = wit_model::beats_per_bar(sig);
            wit_model::BarPos::exact((beats / bpb).floor() + 1.0)
        }
        _ => wit_model::BarPos::about((beats / 4.0).floor() + 1.0),
    }
}

/// Locators added, removed or renamed between two models — Ableton's
/// section markers ("Verse", "Chorus"). Matched by beat time (rounded to 3
/// places, [`wit_model::round3`]'s noise gate), not by Ableton's `Locator
/// Id`: locator-id stability across saves has not been measured the way
/// track-id stability was (`wit-als/src/extract.rs`'s module doc), so
/// matching on the value a musician actually experiences — where the
/// marker sits — is the claim this crate can stand behind.
///
/// A rename is only reported when **exactly one** unchanged-time pair
/// changed name; multiple simultaneous name changes are ambiguous about
/// which old name became which new one, so each is reported independently
/// as a plain add/remove instead of a guessed pairing (the same discipline
/// [`analyze_sample_renames`] applies to sample renames).
fn diff_markers(old: &Model, new: &Model) -> Vec<ChangeRecord> {
    let mut records = Vec::new();
    let sig = new.time_signature.or(old.time_signature);

    // Keyed by a rounded, `Ord`/`Eq`-safe bucket (floats cannot be map keys
    // directly, and Ableton's own float serialisation can differ in its
    // last decimal between two saves of a value nobody touched); the value
    // keeps the original beat time for rendering, so converting to a bar
    // position never round-trips through the bucket.
    let old_by_time = locators_by_time(&old.locators);
    let new_by_time = locators_by_time(&new.locators);

    let old_keys: BTreeSet<i64> = old_by_time.keys().copied().collect();
    let new_keys: BTreeSet<i64> = new_by_time.keys().copied().collect();

    let renamed: Vec<i64> = old_keys
        .intersection(&new_keys)
        .copied()
        .filter(|k| old_by_time[k].1 != new_by_time[k].1)
        .collect();

    if renamed.len() == 1 {
        let k = renamed[0];
        records.push(ChangeRecord::MarkerRenamed {
            old: old_by_time[&k].1.to_string(),
            new: new_by_time[&k].1.to_string(),
        });
    } else {
        for k in &renamed {
            records.push(ChangeRecord::MarkerRemoved {
                name: old_by_time[k].1.to_string(),
            });
        }
        for k in &renamed {
            let (beats, name) = new_by_time[k];
            records.push(ChangeRecord::MarkerAdded {
                name: name.to_string(),
                at: Some(beats_to_bar(beats, sig)),
            });
        }
    }

    for k in new_keys.difference(&old_keys) {
        let (beats, name) = new_by_time[k];
        records.push(ChangeRecord::MarkerAdded {
            name: name.to_string(),
            at: Some(beats_to_bar(beats, sig)),
        });
    }
    for k in old_keys.difference(&new_keys) {
        records.push(ChangeRecord::MarkerRemoved {
            name: old_by_time[k].1.to_string(),
        });
    }

    records
}

/// Group locators into a rounded-time -> (original beats, name) map — see
/// [`diff_markers`] for why the key is rounded and the value isn't.
fn locators_by_time(locators: &[wit_model::Locator]) -> BTreeMap<i64, (f64, &str)> {
    locators
        .iter()
        .map(|l| (round3_key(l.time_beats), (l.time_beats, l.name.as_str())))
        .collect()
}

/// A stable, ordered map key for a beat time, rounded to the same 3 places
/// every other mixer value is (`wit_model::round3`).
fn round3_key(beats: f64) -> i64 {
    (wit_model::round3(beats) * 1000.0).round() as i64
}

fn clip_label(clip: &wit_model::Clip) -> String {
    if !clip.name.is_empty() {
        clip.name.clone()
    } else {
        clip.sample.clone()
    }
}

fn diff_clips(
    records: &mut Vec<ChangeRecord>,
    label: &str,
    ta: &Track,
    tb: &Track,
    replaced_clips: &BTreeSet<(TrackId, String)>,
    id: &TrackId,
) {
    let ca: BTreeSet<&String> = ta.clips.keys().collect();
    let cb: BTreeSet<&String> = tb.clips.keys().collect();

    for cid in cb.difference(&ca) {
        let c = &tb.clips[*cid];
        records.push(ChangeRecord::ClipAdded {
            track: label.to_string(),
            label: clip_label(c),
            start_bar: c.start,
        });
    }
    for cid in ca.difference(&cb) {
        let c = &ta.clips[*cid];
        records.push(ChangeRecord::ClipRemoved {
            track: label.to_string(),
            label: clip_label(c),
            start_bar: c.start,
        });
    }
    for cid in ca.intersection(&cb) {
        let x = &ta.clips[*cid];
        let y = &tb.clips[*cid];
        let nm = clip_label(y);

        if replaced_clips.contains(&(id.clone(), (*cid).clone())) {
            records.push(ChangeRecord::ClipSampleReplaced {
                track: label.to_string(),
                label: nm.clone(),
                old: x.sample.clone(),
                new: y.sample.clone(),
            });
        }
        if (x.start, x.end) != (y.start, y.end) {
            records.push(ChangeRecord::ClipRangeChanged {
                track: label.to_string(),
                label: nm.clone(),
                from_start: x.start,
                from_end: x.end,
                to_start: y.start,
                to_end: y.end,
            });
        }
        if x.disabled != y.disabled {
            records.push(ChangeRecord::ClipMuteChanged {
                track: label.to_string(),
                label: nm,
                muted: y.disabled,
            });
        }
    }
}

struct RenameAnalysis {
    renamed: Vec<ChangeRecord>,
    /// `(track id, clip id)` pairs whose sample transition looked like a
    /// rename per-clip but failed the bijection guard — the old basename
    /// is still referenced somewhere in the new model. `diff_clips` reports
    /// these individually as [`ChangeRecord::ClipSampleReplaced`].
    replaced_clips: BTreeSet<(TrackId, String)>,
}

/// Coalesce per-clip sample transitions into `SAMPLE~` renames, applying
/// the bijection guard: a candidate `(old, new)` transition only counts as
/// a genuine rename if `old` is completely absent from the new model's
/// [`Model::sample_index`]. If it is still referenced anywhere — a swap, or
/// a partial reassignment where some clips moved and others didn't — every
/// clip that proposed that transition is reported individually instead of
/// being folded into a rename nobody asked for.
fn analyze_sample_renames(old: &Model, new: &Model) -> RenameAnalysis {
    let mut candidates: BTreeMap<(String, String), Vec<(TrackId, String)>> = BTreeMap::new();

    for (tid, ta) in &old.tracks {
        let Some(tb) = new.tracks.get(tid) else {
            continue;
        };
        for (cid, ca) in &ta.clips {
            let Some(cb) = tb.clips.get(cid) else {
                continue;
            };
            if ca.sample != cb.sample && !ca.sample.is_empty() && !cb.sample.is_empty() {
                candidates
                    .entry((ca.sample.clone(), cb.sample.clone()))
                    .or_default()
                    .push((tid.clone(), cid.clone()));
            }
        }
    }

    let new_sample_index = new.sample_index();
    let mut renamed_tuples: Vec<(String, String, usize)> = Vec::new();
    let mut replaced_clips = BTreeSet::new();

    for ((old_name, new_name), occurrences) in &candidates {
        if !new_sample_index.contains_key(old_name) {
            renamed_tuples.push((old_name.clone(), new_name.clone(), occurrences.len()));
        } else {
            for (tid, cid) in occurrences {
                replaced_clips.insert((tid.clone(), cid.clone()));
            }
        }
    }

    // M1 named bug fix 2: sort by (-count, old, new) explicitly rather than
    // a stable sort over a hash-randomized candidate iteration order —
    // `tests/test_als_rename_coalescing.py::test_diff_output_order_is_stable_across_processes`
    // (xfail) measured 6 distinct output orderings from 6 PYTHONHASHSEED
    // values on the Python original.
    renamed_tuples.sort_by(|a, b| {
        b.2.cmp(&a.2)
            .then_with(|| a.0.cmp(&b.0))
            .then_with(|| a.1.cmp(&b.1))
    });

    let renamed = renamed_tuples
        .into_iter()
        .map(|(old, new, count)| ChangeRecord::SampleRenamed { old, new, count })
        .collect();

    RenameAnalysis {
        renamed,
        replaced_clips,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use wit_model::{Clip, Device, Fingerprint, TrackKind};

    fn track(id: &str, name: &str, kind: TrackKind) -> Track {
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
        }
    }

    fn with_track(model: &mut Model, t: Track) {
        model.tracks.insert(t.id.clone(), t);
    }

    #[test]
    fn diff_of_identical_models_is_empty() {
        let mut model = Model::default();
        with_track(&mut model, track("1", "Lead Vox", TrackKind::Audio));
        assert!(diff(&model, &model).is_empty());
    }

    #[test]
    fn diff_reports_added_tracks_before_removed_tracks() {
        // GOLDEN_DIFF order: TRACK+ lines precede TRACK- lines
        // (als_semantic_diff.py's diff_models loops `sorted(kb - ka)` before
        // `sorted(ka - kb)`).
        let mut old = Model::default();
        with_track(&mut old, track("1", "Scratch", TrackKind::Midi));

        let mut new = Model::default();
        with_track(&mut new, track("2", "Bridge Gtr", TrackKind::Audio));

        let records = diff(&old, &new);
        assert_eq!(
            records,
            vec![
                ChangeRecord::TrackAdded {
                    name: "Bridge Gtr".to_string(),
                    kind: TrackKind::Audio,
                },
                ChangeRecord::TrackRemoved {
                    name: "Scratch".to_string(),
                    kind: TrackKind::Midi,
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
        let old = Model::default();
        let new = Model {
            tempo_bpm: Some(124.0),
            ..Model::default()
        };
        assert!(diff(&old, &new).is_empty());
    }

    // ---- M1 named bug fix 3: rename bijection guard -----------------

    fn clip(id: &str, sample: &str) -> Clip {
        Clip {
            id: id.to_string(),
            name: String::new(),
            start: 0.0,
            end: 4.0,
            sample: sample.to_string(),
            disabled: false,
        }
    }

    fn track_with_clips(id: &str, clips: Vec<Clip>) -> Track {
        let mut t = track(id, &format!("Track {id}"), TrackKind::Audio);
        for c in clips {
            t.clips.insert(c.id.clone(), c);
        }
        t
    }

    #[test]
    fn a_real_rename_collapses_to_one_line() {
        let mut old = Model::default();
        with_track(
            &mut old,
            track_with_clips("100", vec![clip("1", "old.wav"), clip("2", "old.wav")]),
        );
        let mut new = Model::default();
        with_track(
            &mut new,
            track_with_clips("100", vec![clip("1", "new.wav"), clip("2", "new.wav")]),
        );
        assert_eq!(
            diff(&old, &new),
            vec![ChangeRecord::SampleRenamed {
                old: "old.wav".to_string(),
                new: "new.wav".to_string(),
                count: 2,
            }]
        );
    }

    #[test]
    fn swapping_two_samples_between_clips_is_not_a_rename() {
        // tests/test_als_rename_coalescing.py::test_swapping_two_samples_between_clips_is_not_a_rename
        // (xfail in Python; fixed in this port).
        let mut old = Model::default();
        with_track(
            &mut old,
            track_with_clips("100", vec![clip("1", "kick.wav"), clip("2", "snare.wav")]),
        );
        let mut new = Model::default();
        with_track(
            &mut new,
            track_with_clips("100", vec![clip("1", "snare.wav"), clip("2", "kick.wav")]),
        );
        let records = diff(&old, &new);
        assert!(
            records
                .iter()
                .all(|r| !matches!(r, ChangeRecord::SampleRenamed { .. })),
            "a swap must never be reported as a rename: {records:?}"
        );
        // Both sides still exist and are still used, so the fallback line
        // is a per-clip "sample replaced", not silence.
        assert_eq!(records.len(), 2);
        assert!(records
            .iter()
            .all(|r| matches!(r, ChangeRecord::ClipSampleReplaced { .. })));
    }

    #[test]
    fn a_partial_move_while_the_old_sample_is_still_in_use_is_not_a_rename() {
        let mut old = Model::default();
        with_track(
            &mut old,
            track_with_clips(
                "100",
                (1..=5).map(|i| clip(&i.to_string(), "old.wav")).collect(),
            ),
        );
        let mut new = Model::default();
        with_track(
            &mut new,
            track_with_clips(
                "100",
                vec![
                    clip("1", "new.wav"),
                    clip("2", "new.wav"),
                    clip("3", "new.wav"),
                    clip("4", "old.wav"),
                    clip("5", "old.wav"),
                ],
            ),
        );
        let records = diff(&old, &new);
        assert!(records
            .iter()
            .all(|r| !matches!(r, ChangeRecord::SampleRenamed { .. })));
    }

    #[test]
    fn samples_on_added_tracks_are_not_considered_but_still_allow_the_rename() {
        let mut old = Model::default();
        with_track(
            &mut old,
            track_with_clips("100", vec![clip("1", "old.wav")]),
        );

        let mut new = Model::default();
        with_track(
            &mut new,
            track_with_clips("100", vec![clip("1", "new.wav")]),
        );
        with_track(
            &mut new,
            track_with_clips("101", vec![clip("1", "new.wav")]),
        );

        let records = diff(&old, &new);
        let renames: Vec<_> = records
            .iter()
            .filter(|r| matches!(r, ChangeRecord::SampleRenamed { .. }))
            .collect();
        assert_eq!(
            renames,
            vec![&ChangeRecord::SampleRenamed {
                old: "old.wav".to_string(),
                new: "new.wav".to_string(),
                count: 1,
            }]
        );
    }

    // ---- M1 named bug fix 3 (new capability): device settings fingerprint

    fn device(tag: &str, fp_seed: u8) -> Device {
        Device {
            tag: tag.to_string(),
            fingerprint: Fingerprint([fp_seed; 32]),
            plugin_name: None,
        }
    }

    #[test]
    fn same_tag_same_position_different_fingerprint_is_a_settings_change() {
        let mut old = Model::default();
        let mut t_old = track("1", "Drums", TrackKind::Audio);
        t_old.devices = vec![device("Eq8", 1)];
        with_track(&mut old, t_old);

        let mut new = Model::default();
        let mut t_new = track("1", "Drums", TrackKind::Audio);
        t_new.devices = vec![device("Eq8", 2)];
        with_track(&mut new, t_new);

        assert_eq!(
            diff(&old, &new),
            vec![ChangeRecord::FxSettingsChanged {
                track: "Drums".to_string(),
                device: "Eq8".to_string(),
            }]
        );
    }

    #[test]
    fn identical_device_fingerprints_report_no_musical_change() {
        let mut old = Model::default();
        let mut t_old = track("1", "Drums", TrackKind::Audio);
        t_old.devices = vec![device("Eq8", 1)];
        with_track(&mut old, t_old);

        let mut new = Model::default();
        let mut t_new = track("1", "Drums", TrackKind::Audio);
        t_new.devices = vec![device("Eq8", 1)];
        with_track(&mut new, t_new);

        assert!(diff(&old, &new).is_empty());
    }

    // ---- PLAN-V2 Story contract: time signature, key, markers, plugins,
    // master bus ------------------------------------------------------------

    fn sig(numerator: u16, denominator: u16) -> wit_model::TimeSignature {
        wit_model::TimeSignature {
            numerator,
            denominator,
        }
    }

    #[test]
    fn time_signature_change_is_reported_when_both_sides_are_known() {
        let old = Model {
            time_signature: Some(sig(4, 4)),
            ..Model::default()
        };
        let new = Model {
            time_signature: Some(sig(3, 4)),
            ..Model::default()
        };
        assert_eq!(
            diff(&old, &new),
            vec![ChangeRecord::TimeSignatureChanged {
                from: sig(4, 4),
                to: sig(3, 4),
            }]
        );
    }

    #[test]
    fn a_time_signature_unknown_on_either_side_never_reports_a_change() {
        let old = Model::default();
        let new = Model {
            time_signature: Some(sig(3, 4)),
            ..Model::default()
        };
        assert!(diff(&old, &new).is_empty());
    }

    #[test]
    fn key_change_is_reported_even_from_none() {
        let old = Model::default();
        let new = Model {
            key: Some("C minor".to_string()),
            ..Model::default()
        };
        assert_eq!(
            diff(&old, &new),
            vec![ChangeRecord::KeyChanged {
                from: None,
                to: Some("C minor".to_string()),
            }]
        );
    }

    #[test]
    fn no_key_change_is_reported_when_the_key_is_unchanged() {
        let old = Model {
            key: Some("C minor".to_string()),
            ..Model::default()
        };
        let new = old.clone();
        assert!(diff(&old, &new).is_empty());
    }

    fn locator(name: &str, time_beats: f64) -> wit_model::Locator {
        wit_model::Locator {
            name: name.to_string(),
            time_beats,
        }
    }

    #[test]
    fn a_new_locator_is_a_marker_added_at_an_exact_bar() {
        let old = Model::default();
        let new = Model {
            time_signature: Some(sig(4, 4)),
            locators: vec![locator("Chorus", 16.0)],
            ..Model::default()
        };
        assert_eq!(
            diff(&old, &new),
            vec![ChangeRecord::MarkerAdded {
                name: "Chorus".to_string(),
                at: Some(wit_model::BarPos::exact(5.0)),
            }]
        );
    }

    #[test]
    fn a_marker_position_is_about_when_the_meter_is_unknown() {
        let old = Model::default();
        let new = Model {
            locators: vec![locator("Chorus", 16.0)],
            ..Model::default()
        };
        assert_eq!(
            diff(&old, &new),
            vec![ChangeRecord::MarkerAdded {
                name: "Chorus".to_string(),
                at: Some(wit_model::BarPos::about(5.0)),
            }]
        );
    }

    #[test]
    fn a_removed_locator_is_a_marker_removed() {
        let old = Model {
            locators: vec![locator("Bridge", 8.0)],
            ..Model::default()
        };
        let new = Model::default();
        assert_eq!(
            diff(&old, &new),
            vec![ChangeRecord::MarkerRemoved {
                name: "Bridge".to_string(),
            }]
        );
    }

    #[test]
    fn a_single_name_change_at_an_unchanged_time_is_a_marker_renamed() {
        let old = Model {
            locators: vec![locator("Verse", 8.0)],
            ..Model::default()
        };
        let new = Model {
            locators: vec![locator("Verse 2", 8.0)],
            ..Model::default()
        };
        assert_eq!(
            diff(&old, &new),
            vec![ChangeRecord::MarkerRenamed {
                old: "Verse".to_string(),
                new: "Verse 2".to_string(),
            }]
        );
    }

    #[test]
    fn two_simultaneous_name_changes_are_not_guessed_as_renames() {
        // Brief's own rule: a rename is only reported when exactly one
        // unchanged-time pair changed name.
        let old = Model {
            locators: vec![locator("A", 0.0), locator("B", 8.0)],
            ..Model::default()
        };
        let new = Model {
            locators: vec![locator("A2", 0.0), locator("B2", 8.0)],
            ..Model::default()
        };
        let records = diff(&old, &new);
        assert!(records
            .iter()
            .all(|r| !matches!(r, ChangeRecord::MarkerRenamed { .. })));
        assert_eq!(
            records
                .iter()
                .filter(|r| matches!(r, ChangeRecord::MarkerAdded { .. }))
                .count(),
            2
        );
        assert_eq!(
            records
                .iter()
                .filter(|r| matches!(r, ChangeRecord::MarkerRemoved { .. }))
                .count(),
            2
        );
    }

    fn device_with_plugin(tag: &str, plugin: Option<&str>) -> Device {
        Device {
            tag: tag.to_string(),
            fingerprint: Fingerprint([0; 32]),
            plugin_name: plugin.map(str::to_string),
        }
    }

    #[test]
    fn a_third_party_plugin_added_to_a_track_is_reported_by_name() {
        let mut old = Model::default();
        with_track(&mut old, track("1", "Vox", TrackKind::Audio));
        let mut new = Model::default();
        let mut t = track("1", "Vox", TrackKind::Audio);
        t.devices = vec![device_with_plugin("AuPluginDevice", Some("Pro-Q 3"))];
        with_track(&mut new, t);

        let records = diff(&old, &new);
        assert!(records.contains(&ChangeRecord::PluginAdded {
            track: Some("Vox".to_string()),
            plugin: "Pro-Q 3".to_string(),
        }));
        // The existing tag-based FX+ line still fires alongside it.
        assert!(records.contains(&ChangeRecord::FxAdded {
            track: "Vox".to_string(),
            devices: vec!["AuPluginDevice".to_string()],
        }));
    }

    #[test]
    fn a_third_party_plugin_removed_from_a_track_is_reported_by_name() {
        let mut old = Model::default();
        let mut t = track("1", "Vox", TrackKind::Audio);
        t.devices = vec![device_with_plugin("AuPluginDevice", Some("Pro-Q 3"))];
        with_track(&mut old, t);
        let mut new = Model::default();
        with_track(&mut new, track("1", "Vox", TrackKind::Audio));

        let records = diff(&old, &new);
        assert!(records.contains(&ChangeRecord::PluginRemoved {
            track: Some("Vox".to_string()),
            plugin: "Pro-Q 3".to_string(),
        }));
    }

    // ---- PLAN-V2: master bus devices — the real, reviewed bug this fixes:
    // a save that toggled a mastering plugin off on the master bus and Wit
    // said nothing.

    #[test]
    fn a_master_bus_plugin_toggled_off_is_a_settings_change_named_for_the_bus() {
        let old = Model {
            master_track_label: Some("Main".to_string()),
            master_devices: vec![device("AuPluginDevice", 1)],
            ..Model::default()
        };
        // On/Manual flipped.
        let new = Model {
            master_track_label: Some("Main".to_string()),
            master_devices: vec![device("AuPluginDevice", 2)],
            ..Model::default()
        };

        assert_eq!(
            diff(&old, &new),
            vec![ChangeRecord::FxSettingsChanged {
                track: "Main".to_string(),
                device: "AuPluginDevice".to_string(),
            }]
        );
    }

    #[test]
    fn a_plugin_added_to_the_master_bus_is_reported_by_name() {
        let old = Model {
            master_track_label: Some("Master".to_string()),
            ..Model::default()
        };
        let new = Model {
            master_track_label: Some("Master".to_string()),
            master_devices: vec![device_with_plugin("AuPluginDevice", Some("Ozone"))],
            ..Model::default()
        };

        let records = diff(&old, &new);
        assert!(records.contains(&ChangeRecord::PluginAdded {
            track: Some("Master".to_string()),
            plugin: "Ozone".to_string(),
        }));
        assert!(records.contains(&ChangeRecord::FxAdded {
            track: "Master".to_string(),
            devices: vec!["AuPluginDevice".to_string()],
        }));
    }

    #[test]
    fn no_master_track_found_on_either_side_reports_nothing_for_the_bus() {
        let old = Model::default();
        let new = Model::default();
        assert!(diff(&old, &new).is_empty());
    }
}
