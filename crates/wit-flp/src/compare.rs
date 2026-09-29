//! Structural comparison between two [`Extracted`] snapshots — a DAW-
//! neutral list of [`FlChange`], the FL equivalent of `wit-diff`'s
//! `ChangeRecord` and `wit-logic`'s `change_count`/name-diffing.
//!
//! **Not wired into `wit_model::ChangeRecord`.** The integrator's Story
//! contract (`crates/wit-story`) owns that enum and was landing in
//! parallel with this lane; `origin/main` did not have `crates/wit-story`
//! at the time this was written (checked via `git ls-tree -r origin/main`
//! before starting). `FlChange` is this crate's own small enum instead —
//! see this repo's PR body / `## Needs from other lanes` for the mapping
//! this should get once the Story contract lands.
//!
//! **No stable per-object identity exists at the whitelist-extraction
//! level** (unlike Ableton's track `Id`, which `wit-diff` keys on). FL's
//! event stream gives channel/pattern/plugin *names* with no id attached,
//! so a name that disappears and a different name that appears cannot be
//! distinguished from "renamed" versus "one removed, an unrelated one
//! added" in general. This module only ever calls it a rename in the one
//! case that can't be confused with anything else: **exactly one name
//! removed and exactly one name added**, mirroring the spirit of
//! `wit-diff`'s rename-bijection guard (a transition is only trusted when
//! there is no ambiguity about what it could otherwise be). Anything less
//! clean-cut is reported as plain added/removed lines instead of guessing.

use crate::extract::{Extracted, Tempo};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub enum FlChange {
    ChannelAdded {
        name: String,
    },
    ChannelRemoved {
        name: String,
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
    PluginAdded {
        name: String,
    },
    PluginRemoved {
        name: String,
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
/// channels, patterns, plugins, mixer inserts, and arrangements. Never
/// looks at raw bytes — see [`compare_with_bytes`] for the one fact
/// ([`FlChange::BytesChangedNothingReadable`]) that requires them.
pub fn compare(old: &Extracted, new: &Extracted) -> Vec<FlChange> {
    let mut changes = Vec::new();

    if let (Tempo::Known(from_bpm), Tempo::Known(to_bpm)) = (old.tempo, new.tempo) {
        if from_bpm != to_bpm {
            changes.push(FlChange::TempoChanged { from_bpm, to_bpm });
        }
    }

    diff_names(
        &old.channel_names,
        &new.channel_names,
        &mut changes,
        |name| FlChange::ChannelAdded { name },
        |name| FlChange::ChannelRemoved { name },
        Some(|old, new| FlChange::ChannelRenamed { old, new }),
    );
    diff_names(
        &old.pattern_names,
        &new.pattern_names,
        &mut changes,
        |name| FlChange::PatternAdded { name },
        |name| FlChange::PatternRemoved { name },
        Some(|old, new| FlChange::PatternRenamed { old, new }),
    );
    diff_names(
        &old.plugin_names,
        &new.plugin_names,
        &mut changes,
        |name| FlChange::PluginAdded { name },
        |name| FlChange::PluginRemoved { name },
        None, // the whitelist asks for plugin added/removed only, no renamed
    );
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

/// Multiset added/removed, with the single-clean-swap rename exception
/// described in the module doc. `added`/`removed` (when reported
/// individually) are sorted for determinism — `BTreeMap` iteration order,
/// not a hash-randomized one (AGENTS.md / M1 named bug fix 2's standing
/// rule elsewhere in this workspace).
fn diff_names(
    old: &[String],
    new: &[String],
    out: &mut Vec<FlChange>,
    added: fn(String) -> FlChange,
    removed: fn(String) -> FlChange,
    renamed: Option<fn(String, String) -> FlChange>,
) {
    let mut old_counts: BTreeMap<&str, usize> = BTreeMap::new();
    for n in old {
        *old_counts.entry(n.as_str()).or_default() += 1;
    }
    let mut new_counts: BTreeMap<&str, usize> = BTreeMap::new();
    for n in new {
        *new_counts.entry(n.as_str()).or_default() += 1;
    }

    let mut removed_names: Vec<String> = Vec::new();
    let mut added_names: Vec<String> = Vec::new();
    let all: std::collections::BTreeSet<&str> = old_counts
        .keys()
        .chain(new_counts.keys())
        .copied()
        .collect();
    for name in all {
        let oc = old_counts.get(name).copied().unwrap_or(0);
        let nc = new_counts.get(name).copied().unwrap_or(0);
        for _ in nc..oc {
            removed_names.push(name.to_string());
        }
        for _ in oc..nc {
            added_names.push(name.to_string());
        }
    }
    removed_names.sort();
    added_names.sort();

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

    fn with_channels(names: &[&str]) -> Extracted {
        Extracted {
            channel_names: names.iter().map(|s| s.to_string()).collect(),
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
                name: "Snare".to_string()
            }]
        );
    }

    #[test]
    fn a_missing_channel_is_removed() {
        let old = with_channels(&["Kick", "Snare"]);
        let new = with_channels(&["Kick"]);
        assert_eq!(
            compare(&old, &new),
            vec![FlChange::ChannelRemoved {
                name: "Snare".to_string()
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
        // Two removed, two added -- could be two renames, a swap, or
        // unrelated churn. Never guess; report plainly.
        let old = with_channels(&["A", "B"]);
        let new = with_channels(&["C", "D"]);
        let changes = compare(&old, &new);
        assert!(changes
            .iter()
            .all(|c| !matches!(c, FlChange::ChannelRenamed { .. })));
        assert_eq!(changes.len(), 4);
    }

    #[test]
    fn plugin_changes_never_report_a_rename() {
        let old = Extracted {
            plugin_names: vec!["Sytrus".to_string()],
            ..Extracted::default()
        };
        let new = Extracted {
            plugin_names: vec!["FLEX".to_string()],
            ..Extracted::default()
        };
        let changes = compare(&old, &new);
        assert_eq!(
            changes,
            vec![
                FlChange::PluginAdded {
                    name: "FLEX".to_string()
                },
                FlChange::PluginRemoved {
                    name: "Sytrus".to_string()
                },
            ]
        );
    }

    #[test]
    fn a_mixer_insert_rename_is_reported_as_a_rename_not_unreadable() {
        // The bug this closes: mixer insert and arrangement names were
        // never compared at all, so a rename there fell through to
        // compare_with_bytes's "something changed Wit can't read yet"
        // fallback even though the name change was perfectly readable.
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
                name: "Snare".to_string()
            }]
        );
    }
}
