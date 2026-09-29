//! Structural comparison between two [`Extracted`] readings — a list of
//! [`FlChange`], the FL equivalent of `wit-diff`'s `ChangeRecord` and
//! `wit-logic`'s name comparison.
//!
//! **Not wired into `wit_model::ChangeRecord` / the Story contract**
//! (`crates/wit-story`). `FlChange` is this crate's own small enum; mapping
//! it onto the Story contract is a separate piece of work.
//!
//! **No stable per-object identity exists at the whitelist-extraction
//! level** (unlike Ableton's track `Id`, which `wit-diff` keys on). FL's
//! event stream gives channel/pattern/plugin *names* with no id attached,
//! so a name that disappears and a different name that appears cannot be
//! told apart from "renamed" versus "one removed, an unrelated one added"
//! in general. This module only calls it a rename in the one case that
//! can't be confused with anything else: **exactly one name removed and
//! exactly one name added**. Anything less clean-cut is reported as plain
//! added/removed lines instead of guessing.
//!
//! **A new channel's generator is part of that channel, not a separate
//! plugin change.** Adding an FPC channel reports one line, "channel added:
//! 'Drums'" carrying `generator: Some("FPC")` — not that plus "plugin
//! added: 'FPC'". A generator change is only reported on its own when no
//! added/removed channel accounts for it (e.g. a channel's plugin was
//! replaced while its name stayed).

use crate::extract::{Extracted, MixerEffect, RackChannel, Tempo};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub enum FlChange {
    /// A channel appeared. `generator` is the plugin it plays, when every
    /// channel of that name in the newer file plays the same one (`None`
    /// for a plain Sampler/audio channel, or when that is ambiguous).
    ChannelAdded {
        name: String,
        generator: Option<String>,
    },
    ChannelRemoved {
        name: String,
        generator: Option<String>,
    },
    ChannelRenamed {
        old: String,
        new: String,
    },
    PatternAdded {
        name: String,
    },
    PatternRemoved {
        name: String,
    },
    PatternRenamed {
        old: String,
        new: String,
    },
    /// A channel's generator plugin appeared that no added channel accounts
    /// for (e.g. an existing channel's plugin was replaced).
    GeneratorAdded {
        name: String,
    },
    GeneratorRemoved {
        name: String,
    },
    /// An effect plugin appeared in the mixer. `insert` is as in
    /// [`MixerEffect::insert`] (0 = Master, `None` = not labelled). An
    /// effect moved to another insert reads as removed + added.
    EffectAdded {
        name: String,
        insert: Option<u16>,
    },
    EffectRemoved {
        name: String,
        insert: Option<u16>,
    },
    MixerInsertAdded {
        name: String,
    },
    MixerInsertRemoved {
        name: String,
    },
    MixerInsertRenamed {
        old: String,
        new: String,
    },
    ArrangementAdded {
        name: String,
    },
    ArrangementRemoved {
        name: String,
    },
    ArrangementRenamed {
        old: String,
        new: String,
    },
    TempoChanged {
        from_bpm: f64,
        to_bpm: f64,
    },
    /// The two files are not byte-identical, but nothing on this crate's
    /// whitelist changed. Never constructed by [`compare`] itself — see
    /// [`compare_with_bytes`], which is the only thing that knows whether
    /// the raw bytes actually differ.
    BytesChangedNothingReadable,
}

/// Compare two extractions, in a fixed, deterministic order: tempo, then
/// channels (with their generators), patterns, mixer effects, mixer
/// inserts, and arrangements. Never looks at raw bytes — see
/// [`compare_with_bytes`] for the one fact
/// ([`FlChange::BytesChangedNothingReadable`]) that requires them.
pub fn compare(old: &Extracted, new: &Extracted) -> Vec<FlChange> {
    let mut changes = Vec::new();

    if let (Tempo::Known(from_bpm), Tempo::Known(to_bpm)) = (old.tempo, new.tempo) {
        if from_bpm != to_bpm {
            changes.push(FlChange::TempoChanged { from_bpm, to_bpm });
        }
    }

    compare_channels(old, new, &mut changes);
    diff_names(
        &old.pattern_names,
        &new.pattern_names,
        &mut changes,
        |name| FlChange::PatternAdded { name },
        |name| FlChange::PatternRemoved { name },
        Some(|old, new| FlChange::PatternRenamed { old, new }),
    );
    compare_effects(&old.mixer_effects, &new.mixer_effects, &mut changes);
    diff_names(
        &old.mixer_insert_names,
        &new.mixer_insert_names,
        &mut changes,
        |name| FlChange::MixerInsertAdded { name },
        |name| FlChange::MixerInsertRemoved { name },
        Some(|old, new| FlChange::MixerInsertRenamed { old, new }),
    );
    diff_names(
        &old.arrangement_names,
        &new.arrangement_names,
        &mut changes,
        |name| FlChange::ArrangementAdded { name },
        |name| FlChange::ArrangementRemoved { name },
        Some(|old, new| FlChange::ArrangementRenamed { old, new }),
    );

    changes
}

/// [`compare`], plus the one fallback fact it cannot see on its own:
/// whether the underlying bytes actually differ. If they do and `compare`
/// found nothing, that fact is worth a line — "something changed, Wit
/// can't read what" — rather than silently reporting an empty list
/// indistinguishable from "these two files are identical."
pub fn compare_with_bytes(old: &Extracted, new: &Extracted, bytes_equal: bool) -> Vec<FlChange> {
    let changes = compare(old, new);
    if changes.is_empty() && !bytes_equal {
        vec![FlChange::BytesChangedNothingReadable]
    } else {
        changes
    }
}

/// Channel names (with the single-clean-swap rename), then generator
/// changes that no added/removed channel accounts for.
fn compare_channels(old: &Extracted, new: &Extracted, out: &mut Vec<FlChange>) {
    let (removed, added) = multiset_diff(&old.channel_names(), &new.channel_names());
    let (mut generators_removed, mut generators_added) =
        multiset_diff(&old.generator_names(), &new.generator_names());

    if let ([old_name], [new_name]) = (removed.as_slice(), added.as_slice()) {
        out.push(FlChange::ChannelRenamed {
            old: old_name.clone(),
            new: new_name.clone(),
        });
    } else {
        for name in added {
            let generator = sole_generator(&new.channel_rack, &name);
            if let Some(g) = &generator {
                take_one(&mut generators_added, g);
            }
            out.push(FlChange::ChannelAdded { name, generator });
        }
        for name in removed {
            let generator = sole_generator(&old.channel_rack, &name);
            if let Some(g) = &generator {
                take_one(&mut generators_removed, g);
            }
            out.push(FlChange::ChannelRemoved { name, generator });
        }
    }
    for name in generators_added {
        out.push(FlChange::GeneratorAdded { name });
    }
    for name in generators_removed {
        out.push(FlChange::GeneratorRemoved { name });
    }
}

/// The generator every channel named `name` plays, if they all play the
/// same one; `None` if none does or they disagree.
fn sole_generator(rack: &[RackChannel], name: &str) -> Option<String> {
    let mut generators = rack
        .iter()
        .filter(|c| c.name.as_deref() == Some(name))
        .map(|c| c.generator.as_deref());
    let first = generators.next()??;
    generators
        .all(|g| g == Some(first))
        .then(|| first.to_string())
}

fn take_one(from: &mut Vec<String>, name: &str) {
    if let Some(i) = from.iter().position(|n| n == name) {
        from.remove(i);
    }
}

/// Effects are compared as a multiset of (name, insert): no rename (the
/// whitelist asks for plugin added/removed only).
fn compare_effects(old: &[MixerEffect], new: &[MixerEffect], out: &mut Vec<FlChange>) {
    let (removed, added) = multiset_diff(old, new);
    for effect in added {
        out.push(FlChange::EffectAdded {
            name: effect.name,
            insert: effect.insert,
        });
    }
    for effect in removed {
        out.push(FlChange::EffectRemoved {
            name: effect.name,
            insert: effect.insert,
        });
    }
}

/// `(removed, added)` as multisets, each sorted — `BTreeMap` order, never a
/// hash-randomized one, so output is deterministic.
fn multiset_diff<T: Ord + Clone>(old: &[T], new: &[T]) -> (Vec<T>, Vec<T>) {
    let mut counts: BTreeMap<&T, (usize, usize)> = BTreeMap::new();
    for item in old {
        counts.entry(item).or_default().0 += 1;
    }
    for item in new {
        counts.entry(item).or_default().1 += 1;
    }
    let mut removed = Vec::new();
    let mut added = Vec::new();
    for (item, (in_old, in_new)) in counts {
        for _ in in_new..in_old {
            removed.push(item.clone());
        }
        for _ in in_old..in_new {
            added.push(item.clone());
        }
    }
    (removed, added)
}

/// Multiset added/removed, with the single-clean-swap rename exception
/// described in the module doc.
fn diff_names(
    old: &[String],
    new: &[String],
    out: &mut Vec<FlChange>,
    added: fn(String) -> FlChange,
    removed: fn(String) -> FlChange,
    renamed: Option<fn(String, String) -> FlChange>,
) {
    let (removed_names, added_names) = multiset_diff(old, new);
    if let (Some(renamed), [old_name], [new_name]) =
        (renamed, removed_names.as_slice(), added_names.as_slice())
    {
        out.push(renamed(old_name.clone(), new_name.clone()));
        return;
    }
    for name in added_names {
        out.push(added(name));
    }
    for name in removed_names {
        out.push(removed(name));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rack(channels: &[(&str, Option<&str>)]) -> Vec<RackChannel> {
        channels
            .iter()
            .map(|(name, generator)| RackChannel {
                name: Some(name.to_string()),
                generator: generator.map(str::to_string),
            })
            .collect()
    }

    fn with_channels(names: &[&str]) -> Extracted {
        Extracted {
            channel_rack: rack(&names.iter().map(|n| (*n, None)).collect::<Vec<_>>()),
            ..Extracted::default()
        }
    }

    fn with_rack(channels: &[(&str, Option<&str>)]) -> Extracted {
        Extracted {
            channel_rack: rack(channels),
            ..Extracted::default()
        }
    }

    fn with_effects(effects: &[(&str, Option<u16>)]) -> Extracted {
        Extracted {
            mixer_effects: effects
                .iter()
                .map(|(name, insert)| MixerEffect {
                    name: name.to_string(),
                    insert: *insert,
                })
                .collect(),
            ..Extracted::default()
        }
    }

    #[test]
    fn identical_extractions_compare_empty() {
        let e = with_channels(&["Kick", "Snare"]);
        assert_eq!(compare(&e, &e), vec![]);
    }

    #[test]
    fn a_new_channel_is_added() {
        let old = with_channels(&["Kick"]);
        let new = with_channels(&["Kick", "Snare"]);
        assert_eq!(
            compare(&old, &new),
            vec![FlChange::ChannelAdded {
                name: "Snare".to_string(),
                generator: None,
            }]
        );
    }

    #[test]
    fn a_new_generator_channel_is_one_change_not_a_channel_plus_a_plugin() {
        // The real FL 25 pair the review named: two channels added, each
        // playing a generator of the same name. That is two changes, each
        // carrying its generator — never four.
        let old = with_rack(&[("808 Kick", None)]);
        let new = with_rack(&[
            ("808 Kick", None),
            ("Drumpad", Some("Drumpad")),
            ("MIDI Out", Some("MIDI Out")),
        ]);
        assert_eq!(
            compare(&old, &new),
            vec![
                FlChange::ChannelAdded {
                    name: "Drumpad".to_string(),
                    generator: Some("Drumpad".to_string()),
                },
                FlChange::ChannelAdded {
                    name: "MIDI Out".to_string(),
                    generator: Some("MIDI Out".to_string()),
                },
            ]
        );
    }

    #[test]
    fn a_removed_generator_channel_carries_its_generator() {
        let old = with_rack(&[("Kick", None), ("Drums", Some("FPC"))]);
        let new = with_rack(&[("Kick", None)]);
        assert_eq!(
            compare(&old, &new),
            vec![FlChange::ChannelRemoved {
                name: "Drums".to_string(),
                generator: Some("FPC".to_string()),
            }]
        );
    }

    #[test]
    fn a_replaced_generator_on_a_kept_channel_is_reported_on_its_own() {
        let old = with_rack(&[("Lead", Some("Sytrus")), ("Kick", None)]);
        let new = with_rack(&[("Lead", Some("FLEX")), ("Kick", None)]);
        assert_eq!(
            compare(&old, &new),
            vec![
                FlChange::GeneratorAdded {
                    name: "FLEX".to_string()
                },
                FlChange::GeneratorRemoved {
                    name: "Sytrus".to_string()
                },
            ]
        );
    }

    #[test]
    fn an_ambiguous_generator_is_not_attributed_to_a_new_channel() {
        // Two channels named "Pad" play different generators: which one the
        // new "Pad" is can't be told, so no generator is claimed for it and
        // the net generator change is reported plainly.
        let old = with_rack(&[("Pad", Some("Sytrus"))]);
        let new = with_rack(&[("Pad", Some("Sytrus")), ("Pad", Some("Harmor"))]);
        assert_eq!(
            compare(&old, &new),
            vec![
                FlChange::ChannelAdded {
                    name: "Pad".to_string(),
                    generator: None,
                },
                FlChange::GeneratorAdded {
                    name: "Harmor".to_string()
                },
            ]
        );
    }

    #[test]
    fn a_missing_channel_is_removed() {
        let old = with_channels(&["Kick", "Snare"]);
        let new = with_channels(&["Kick"]);
        assert_eq!(
            compare(&old, &new),
            vec![FlChange::ChannelRemoved {
                name: "Snare".to_string(),
                generator: None,
            }]
        );
    }

    #[test]
    fn a_clean_one_for_one_swap_is_a_rename() {
        let old = with_channels(&["Kick"]);
        let new = with_channels(&["808 Kick"]);
        assert_eq!(
            compare(&old, &new),
            vec![FlChange::ChannelRenamed {
                old: "Kick".to_string(),
                new: "808 Kick".to_string(),
            }]
        );
    }

    #[test]
    fn ambiguous_multi_way_changes_are_never_guessed_as_a_rename() {
        let old = with_channels(&["A", "B"]);
        let new = with_channels(&["C", "D"]);
        let changes = compare(&old, &new);
        assert!(changes
            .iter()
            .all(|c| !matches!(c, FlChange::ChannelRenamed { .. })));
        assert_eq!(changes.len(), 4);
    }

    #[test]
    fn an_unnamed_channel_is_not_a_channel_name_change() {
        let mut old = with_channels(&["Kick"]);
        old.channel_rack.push(RackChannel::default());
        let new = with_channels(&["Kick"]);
        assert_eq!(compare(&old, &new), vec![]);
    }

    #[test]
    fn an_added_effect_is_reported_with_its_insert() {
        let old = with_effects(&[("Fruity Limiter", Some(0))]);
        let new = with_effects(&[("Fruity Limiter", Some(0)), ("Fruity Reeverb 2", Some(3))]);
        assert_eq!(
            compare(&old, &new),
            vec![FlChange::EffectAdded {
                name: "Fruity Reeverb 2".to_string(),
                insert: Some(3),
            }]
        );
    }

    #[test]
    fn an_effect_moved_to_another_insert_is_removed_and_added() {
        let old = with_effects(&[("Fruity Reeverb 2", Some(3))]);
        let new = with_effects(&[("Fruity Reeverb 2", Some(4))]);
        assert_eq!(
            compare(&old, &new),
            vec![
                FlChange::EffectAdded {
                    name: "Fruity Reeverb 2".to_string(),
                    insert: Some(4),
                },
                FlChange::EffectRemoved {
                    name: "Fruity Reeverb 2".to_string(),
                    insert: Some(3),
                },
            ]
        );
    }

    #[test]
    fn a_mixer_insert_rename_is_reported_as_a_rename_not_unreadable() {
        let old = Extracted {
            mixer_insert_names: vec!["Dream bell".to_string()],
            ..Extracted::default()
        };
        let new = Extracted {
            mixer_insert_names: vec!["Dream bell 2".to_string()],
            ..Extracted::default()
        };
        assert_eq!(
            compare(&old, &new),
            vec![FlChange::MixerInsertRenamed {
                old: "Dream bell".to_string(),
                new: "Dream bell 2".to_string(),
            }]
        );
    }

    #[test]
    fn an_arrangement_added_is_reported() {
        let old = Extracted::default();
        let new = Extracted {
            arrangement_names: vec!["Arrangement".to_string()],
            ..Extracted::default()
        };
        assert_eq!(
            compare(&old, &new),
            vec![FlChange::ArrangementAdded {
                name: "Arrangement".to_string()
            }]
        );
    }

    #[test]
    fn tempo_change_is_reported_only_when_both_sides_are_known() {
        let old = Extracted {
            tempo: Tempo::Known(120.0),
            ..Extracted::default()
        };
        let new = Extracted {
            tempo: Tempo::Known(128.0),
            ..Extracted::default()
        };
        assert_eq!(
            compare(&old, &new),
            vec![FlChange::TempoChanged {
                from_bpm: 120.0,
                to_bpm: 128.0
            }]
        );
    }

    #[test]
    fn tempo_is_never_compared_when_either_side_is_unknown_or_partial() {
        let known = Extracted {
            tempo: Tempo::Known(120.0),
            ..Extracted::default()
        };
        let unknown = Extracted {
            tempo: Tempo::Unknown,
            ..Extracted::default()
        };
        let partial = Extracted {
            tempo: Tempo::PartialV25ScalarsUnreadable,
            ..Extracted::default()
        };
        assert!(compare(&known, &unknown).is_empty());
        assert!(compare(&known, &partial).is_empty());
    }

    #[test]
    fn bytes_changed_but_nothing_readable_only_fires_when_bytes_actually_differ() {
        let e = Extracted::default();
        assert_eq!(compare_with_bytes(&e, &e, true), vec![]);
        assert_eq!(
            compare_with_bytes(&e, &e, false),
            vec![FlChange::BytesChangedNothingReadable]
        );
    }

    #[test]
    fn a_real_structural_change_takes_priority_over_the_bytes_fallback() {
        let old = with_channels(&["Kick"]);
        let new = with_channels(&["Kick", "Snare"]);
        assert_eq!(
            compare_with_bytes(&old, &new, false),
            vec![FlChange::ChannelAdded {
                name: "Snare".to_string(),
                generator: None,
            }]
        );
    }
}
