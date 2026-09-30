//! Structural comparison between two [`Extracted`] readings — a list of
//! [`FlChange`], the FL equivalent of `wit-diff`'s `ChangeRecord` and
//! `wit-logic`'s name comparison.
//!
//! **Not wired into `wit_model::ChangeRecord` / the Story contract**
//! (`crates/wit-story`). `FlChange` is this crate's own enum; mapping it
//! onto the Story contract is a separate piece of work.
//!
//! # Names belong to objects, and objects have positions
//!
//! FL saves most names only when the user set one: a pattern or mixer
//! insert with no saved name still exists (FL shows its default name), and
//! a channel's name can be missing too. So a name that disappears is
//! **not** an object that disappeared. Every name is compared against the
//! object it belongs to, identified the only ways the file allows:
//!
//! - **Channels** by their place in the Channel rack. The two racks are
//!   lined up first (identical channels anchor the alignment), so adding or
//!   deleting a channel anywhere does not shift every channel after it into
//!   a "rename". Between anchors, an old and a new channel are the *same*
//!   channel only if they still share their name or their generator — a
//!   channel whose name and generator both changed reads as one removed
//!   and one added, never as a rename.
//! - **Patterns** by the number FL saves with each one (see
//!   `extract.rs`'s `PAT_NEW`). Not compared at all on FL 25, where that
//!   number is scrambled.
//! - **Mixer inserts** by mixer position. On FL 8.5–20.8 the number of
//!   positions is fixed by the FL version (105 or 127), so an insert is
//!   never "added" or "removed" there — only named, renamed or cleared.
//!   Only between two FL 25 saves (18 positions measured, possibly
//!   variable, unverified) is a position appearing or disappearing
//!   reported as an insert added or removed.
//! - **Arrangements** by their order.
//!
//! Effects are compared as a multiset of (plugin, mixer position).

use crate::extract::{Extracted, FormatStatus, MixerEffect, RackChannel, Tempo};
use std::collections::BTreeMap;

/// How an object's saved name changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NameChange {
    /// It had no saved name and now has one.
    Named {
        name: String,
    },
    Renamed {
        old: String,
        new: String,
    },
    /// It had a saved name and now has none (FL shows its default name).
    Cleared {
        old: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum FlChange {
    /// A channel block appeared. `position` is its place in the newer
    /// file's rack (0-based); `generator` is the plugin it plays.
    ChannelAdded {
        position: usize,
        name: Option<String>,
        generator: Option<String>,
    },
    /// A channel block disappeared; `position` is in the older rack.
    ChannelRemoved {
        position: usize,
        name: Option<String>,
        generator: Option<String>,
    },
    /// The same channel, with a different saved name.
    ChannelName {
        position: usize,
        change: NameChange,
    },
    /// The same channel (it kept its name), playing a different generator.
    GeneratorChanged {
        position: usize,
        channel: Option<String>,
        old: Option<String>,
        new: Option<String>,
    },
    PatternAdded {
        number: u16,
        name: Option<String>,
    },
    PatternRemoved {
        number: u16,
        name: Option<String>,
    },
    PatternName {
        number: u16,
        change: NameChange,
    },
    /// An effect plugin appeared at this mixer position. An effect moved
    /// to another position reads as removed + added.
    EffectAdded {
        name: String,
        position: u16,
    },
    EffectRemoved {
        name: String,
        position: u16,
    },
    /// Only ever between two FL 25 saves — see the module doc.
    MixerInsertAdded {
        position: u16,
        name: Option<String>,
    },
    MixerInsertRemoved {
        position: u16,
        name: Option<String>,
    },
    MixerInsertName {
        position: u16,
        change: NameChange,
    },
    ArrangementAdded {
        position: usize,
        name: Option<String>,
    },
    ArrangementRemoved {
        position: usize,
        name: Option<String>,
    },
    ArrangementName {
        position: usize,
        change: NameChange,
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
/// channels, patterns, mixer effects, mixer inserts, and arrangements.
/// Never looks at raw bytes — see [`compare_with_bytes`] for the one fact
/// ([`FlChange::BytesChangedNothingReadable`]) that requires them.
pub fn compare(old: &Extracted, new: &Extracted) -> Vec<FlChange> {
    let mut changes = Vec::new();

    if let (Tempo::Known(from_bpm), Tempo::Known(to_bpm)) = (old.tempo, new.tempo) {
        if from_bpm != to_bpm {
            changes.push(FlChange::TempoChanged { from_bpm, to_bpm });
        }
    }

    compare_channels(&old.channel_rack, &new.channel_rack, &mut changes);
    let either_v25 = [old, new]
        .iter()
        .any(|e| e.format_status == FormatStatus::PartialV25ScalarsUnreadable);
    if !either_v25 {
        compare_patterns(old, new, &mut changes);
    }
    compare_effects(&old.mixer_effects, &new.mixer_effects, &mut changes);
    let both_v25 = [old, new]
        .iter()
        .all(|e| e.format_status == FormatStatus::PartialV25ScalarsUnreadable);
    compare_inserts(
        &old.mixer_inserts,
        &new.mixer_inserts,
        both_v25,
        &mut changes,
    );
    compare_arrangements(&old.arrangements, &new.arrangements, &mut changes);

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

fn name_change(old: &Option<String>, new: &Option<String>) -> Option<NameChange> {
    match (old, new) {
        (None, Some(name)) => Some(NameChange::Named { name: name.clone() }),
        (Some(old), None) => Some(NameChange::Cleared { old: old.clone() }),
        (Some(old), Some(new)) if old != new => Some(NameChange::Renamed {
            old: old.clone(),
            new: new.clone(),
        }),
        _ => None,
    }
}

/// Above this many (old gap × new gap) cells, the channel alignment skips
/// the exact longest-common-subsequence step and pairs channels in order
/// instead — linear, so a crafted or enormous rack can't make a comparison
/// quadratic. Real racks are far below it (the largest of the 178 real
/// files checked has 258 channels).
const MAX_ALIGNMENT_CELLS: usize = 1 << 20;

fn compare_channels(old: &[RackChannel], new: &[RackChannel], out: &mut Vec<FlChange>) {
    let prefix = old.iter().zip(new).take_while(|(a, b)| a == b).count();
    let max_suffix = old.len().min(new.len()) - prefix;
    let suffix = old
        .iter()
        .rev()
        .zip(new.iter().rev())
        .take(max_suffix)
        .take_while(|(a, b)| a == b)
        .count();
    let old_gap = &old[prefix..old.len() - suffix];
    let new_gap = &new[prefix..new.len() - suffix];

    let mut from = (0, 0);
    let end = (old_gap.len(), new_gap.len());
    for (a, b) in unchanged_anchors(old_gap, new_gap).into_iter().chain([end]) {
        pair_in_order(
            &old_gap[from.0..a],
            prefix + from.0,
            &new_gap[from.1..b],
            prefix + from.1,
            out,
        );
        from = (a + 1, b + 1);
    }
}

/// Index pairs of identical channels on a longest common subsequence of
/// the two gaps, or none when the gaps are too large to align exactly.
fn unchanged_anchors(old: &[RackChannel], new: &[RackChannel]) -> Vec<(usize, usize)> {
    let (n, m) = (old.len(), new.len());
    if n == 0 || m == 0 || n.saturating_mul(m) > MAX_ALIGNMENT_CELLS {
        return Vec::new();
    }
    // lcs[i * (m + 1) + j] = LCS length of old[i..] and new[j..].
    let mut lcs = vec![0u32; (n + 1) * (m + 1)];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs[i * (m + 1) + j] = if old[i] == new[j] {
                lcs[(i + 1) * (m + 1) + j + 1] + 1
            } else {
                lcs[(i + 1) * (m + 1) + j].max(lcs[i * (m + 1) + j + 1])
            };
        }
    }
    let mut anchors = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if old[i] == new[j] {
            anchors.push((i, j));
            i += 1;
            j += 1;
        } else if lcs[(i + 1) * (m + 1) + j] >= lcs[i * (m + 1) + j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    anchors
}

/// Pair up the channels between two anchors, in rack order. An old and a
/// new channel are the same channel only if they share a name or a
/// generator; otherwise the one on the longer side is added/removed.
fn pair_in_order(
    old: &[RackChannel],
    old_base: usize,
    new: &[RackChannel],
    new_base: usize,
    out: &mut Vec<FlChange>,
) {
    let removed = |i: usize, c: &RackChannel| FlChange::ChannelRemoved {
        position: old_base + i,
        name: c.name.clone(),
        generator: c.generator.clone(),
    };
    let added = |j: usize, c: &RackChannel| FlChange::ChannelAdded {
        position: new_base + j,
        name: c.name.clone(),
        generator: c.generator.clone(),
    };
    let (mut i, mut j) = (0, 0);
    while i < old.len() && j < new.len() {
        let (a, b) = (&old[i], &new[j]);
        if a.name == b.name || a.generator == b.generator {
            let position = new_base + j;
            if let Some(change) = name_change(&a.name, &b.name) {
                out.push(FlChange::ChannelName { position, change });
            }
            if a.generator != b.generator {
                out.push(FlChange::GeneratorChanged {
                    position,
                    channel: b.name.clone(),
                    old: a.generator.clone(),
                    new: b.generator.clone(),
                });
            }
            i += 1;
            j += 1;
            continue;
        }
        let (old_left, new_left) = (old.len() - i, new.len() - j);
        if old_left >= new_left {
            out.push(removed(i, a));
            i += 1;
        }
        if new_left >= old_left {
            out.push(added(j, b));
            j += 1;
        }
    }
    for (i, c) in old.iter().enumerate().skip(i) {
        out.push(removed(i, c));
    }
    for (j, c) in new.iter().enumerate().skip(j) {
        out.push(added(j, c));
    }
}

fn compare_patterns(old: &Extracted, new: &Extracted, out: &mut Vec<FlChange>) {
    let by_number = |e: &Extracted| -> BTreeMap<u16, Option<String>> {
        e.patterns
            .iter()
            .filter_map(|p| Some((p.number?, p.name.clone())))
            .collect()
    };
    let (old, new) = (by_number(old), by_number(new));
    let numbers: std::collections::BTreeSet<u16> = old.keys().chain(new.keys()).copied().collect();
    for number in numbers {
        match (old.get(&number), new.get(&number)) {
            (Some(a), Some(b)) => {
                if let Some(change) = name_change(a, b) {
                    out.push(FlChange::PatternName { number, change });
                }
            }
            (None, Some(name)) => out.push(FlChange::PatternAdded {
                number,
                name: name.clone(),
            }),
            (Some(name), None) => out.push(FlChange::PatternRemoved {
                number,
                name: name.clone(),
            }),
            (None, None) => {}
        }
    }
}

fn compare_inserts(
    old: &[Option<String>],
    new: &[Option<String>],
    positions_can_change: bool,
    out: &mut Vec<FlChange>,
) {
    for position in 0..old.len().max(new.len()) {
        let Ok(p) = u16::try_from(position) else {
            break;
        };
        match (old.get(position), new.get(position)) {
            (Some(a), Some(b)) => {
                if let Some(change) = name_change(a, b) {
                    out.push(FlChange::MixerInsertName {
                        position: p,
                        change,
                    });
                }
            }
            (None, Some(name)) if positions_can_change => out.push(FlChange::MixerInsertAdded {
                position: p,
                name: name.clone(),
            }),
            (Some(name), None) if positions_can_change => out.push(FlChange::MixerInsertRemoved {
                position: p,
                name: name.clone(),
            }),
            // A position only one FL version has (105 vs 127): not an
            // insert the user added — only its name, if any, is news.
            (a, b) => {
                let none = None;
                if let Some(change) = name_change(a.unwrap_or(&none), b.unwrap_or(&none)) {
                    out.push(FlChange::MixerInsertName {
                        position: p,
                        change,
                    });
                }
            }
        }
    }
}

fn compare_arrangements(old: &[Option<String>], new: &[Option<String>], out: &mut Vec<FlChange>) {
    for position in 0..old.len().max(new.len()) {
        match (old.get(position), new.get(position)) {
            (Some(a), Some(b)) => {
                if let Some(change) = name_change(a, b) {
                    out.push(FlChange::ArrangementName { position, change });
                }
            }
            (None, Some(name)) => out.push(FlChange::ArrangementAdded {
                position,
                name: name.clone(),
            }),
            (Some(name), None) => out.push(FlChange::ArrangementRemoved {
                position,
                name: name.clone(),
            }),
            (None, None) => {}
        }
    }
}

/// Effects are compared as a multiset of (name, position): no rename (the
/// whitelist asks for plugin added/removed only).
fn compare_effects(old: &[MixerEffect], new: &[MixerEffect], out: &mut Vec<FlChange>) {
    let mut counts: BTreeMap<&MixerEffect, (usize, usize)> = BTreeMap::new();
    for effect in old {
        counts.entry(effect).or_default().0 += 1;
    }
    for effect in new {
        counts.entry(effect).or_default().1 += 1;
    }
    let mut removed = Vec::new();
    for (effect, (in_old, in_new)) in counts {
        for _ in in_old..in_new {
            out.push(FlChange::EffectAdded {
                name: effect.name.clone(),
                position: effect.position,
            });
        }
        for _ in in_new..in_old {
            removed.push(FlChange::EffectRemoved {
                name: effect.name.clone(),
                position: effect.position,
            });
        }
    }
    out.extend(removed);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extract::Pattern;

    fn ch(name: Option<&str>, generator: Option<&str>) -> RackChannel {
        RackChannel {
            name: name.map(str::to_string),
            generator: generator.map(str::to_string),
        }
    }

    fn with_rack(channels: &[(Option<&str>, Option<&str>)]) -> Extracted {
        Extracted {
            channel_rack: channels.iter().map(|(n, g)| ch(*n, *g)).collect(),
            ..Extracted::default()
        }
    }

    fn named(names: &[&str]) -> Extracted {
        with_rack(&names.iter().map(|n| (Some(*n), None)).collect::<Vec<_>>())
    }

    fn s(x: &str) -> String {
        x.to_string()
    }

    fn with_patterns(patterns: &[(u16, Option<&str>)]) -> Extracted {
        Extracted {
            patterns: patterns
                .iter()
                .map(|(number, name)| Pattern {
                    number: Some(*number),
                    name: name.map(str::to_string),
                })
                .collect(),
            ..Extracted::default()
        }
    }

    fn with_inserts(names: &[Option<&str>]) -> Extracted {
        Extracted {
            mixer_inserts: names.iter().map(|n| n.map(str::to_string)).collect(),
            ..Extracted::default()
        }
    }

    fn with_effects(effects: &[(&str, u16)]) -> Extracted {
        Extracted {
            mixer_effects: effects
                .iter()
                .map(|(name, position)| MixerEffect {
                    name: s(name),
                    position: *position,
                })
                .collect(),
            ..Extracted::default()
        }
    }

    // ---- names are not objects (the review's four cases) -------------- //

    #[test]
    fn a_cleared_pattern_name_is_not_a_removed_pattern() {
        // Dropping a 193: the pattern (its number) is still there, it just
        // has no saved name now.
        let old = with_patterns(&[(1, Some("Clap Mute")), (2, None)]);
        let new = with_patterns(&[(1, None), (2, None)]);
        assert_eq!(
            compare(&old, &new),
            vec![FlChange::PatternName {
                number: 1,
                change: NameChange::Cleared {
                    old: s("Clap Mute")
                },
            }]
        );
    }

    #[test]
    fn a_cleared_insert_name_is_not_a_removed_insert() {
        let old = with_inserts(&[None, Some("Kick"), None]);
        let new = with_inserts(&[None, None, None]);
        assert_eq!(
            compare(&old, &new),
            vec![FlChange::MixerInsertName {
                position: 1,
                change: NameChange::Cleared { old: s("Kick") },
            }]
        );
    }

    #[test]
    fn a_new_insert_name_is_not_an_added_insert() {
        let old = with_inserts(&[None, None, None, None]);
        let new = with_inserts(&[None, None, None, Some("Vocals")]);
        assert_eq!(
            compare(&old, &new),
            vec![FlChange::MixerInsertName {
                position: 3,
                change: NameChange::Named { name: s("Vocals") },
            }]
        );
    }

    #[test]
    fn a_cleared_channel_name_is_not_a_removed_channel() {
        // The rack is still the same length and the channel keeps its
        // generator: it lost its saved name, it was not removed.
        let old = with_rack(&[
            (Some("Kick"), None),
            (Some("Vox GR"), Some("Fruity Balance")),
        ]);
        let new = with_rack(&[(Some("Kick"), None), (None, Some("Fruity Balance"))]);
        assert_eq!(
            compare(&old, &new),
            vec![FlChange::ChannelName {
                position: 1,
                change: NameChange::Cleared { old: s("Vox GR") },
            }]
        );
    }

    // ---- channels ----------------------------------------------------- //

    #[test]
    fn identical_extractions_compare_empty() {
        let e = named(&["Kick", "Snare"]);
        assert_eq!(compare(&e, &e), vec![]);
    }

    #[test]
    fn a_channel_added_in_the_middle_does_not_shift_the_rest_into_renames() {
        let old = named(&["Kick", "Snare", "Hat", "Clap"]);
        let new = named(&["Kick", "Snare", "Rim", "Hat", "Clap"]);
        assert_eq!(
            compare(&old, &new),
            vec![FlChange::ChannelAdded {
                position: 2,
                name: Some(s("Rim")),
                generator: None,
            }]
        );
    }

    #[test]
    fn separate_edits_across_the_rack_are_each_reported_once() {
        // First channel deleted, a middle one renamed, one appended.
        let old = named(&["Intro", "Kick", "Snare", "Hat", "Clap"]);
        let new = named(&["Kick", "Snare 2", "Hat", "Clap", "Perc"]);
        assert_eq!(
            compare(&old, &new),
            vec![
                FlChange::ChannelRemoved {
                    position: 0,
                    name: Some(s("Intro")),
                    generator: None,
                },
                FlChange::ChannelName {
                    position: 1,
                    change: NameChange::Renamed {
                        old: s("Snare"),
                        new: s("Snare 2"),
                    },
                },
                FlChange::ChannelAdded {
                    position: 4,
                    name: Some(s("Perc")),
                    generator: None,
                },
            ]
        );
    }

    #[test]
    fn a_new_generator_channel_is_one_change_carrying_its_generator() {
        // The real FL 25 pair the review named.
        let old = with_rack(&[(Some("808 Kick"), None)]);
        let new = with_rack(&[
            (Some("808 Kick"), None),
            (Some("Drumpad"), Some("Drumpad")),
            (Some("MIDI Out"), Some("MIDI Out")),
        ]);
        assert_eq!(
            compare(&old, &new),
            vec![
                FlChange::ChannelAdded {
                    position: 1,
                    name: Some(s("Drumpad")),
                    generator: Some(s("Drumpad")),
                },
                FlChange::ChannelAdded {
                    position: 2,
                    name: Some(s("MIDI Out")),
                    generator: Some(s("MIDI Out")),
                },
            ]
        );
    }

    #[test]
    fn a_replaced_generator_on_a_kept_channel_is_a_generator_change() {
        let old = with_rack(&[(Some("Lead"), Some("Sytrus")), (Some("Kick"), None)]);
        let new = with_rack(&[(Some("Lead"), Some("FLEX")), (Some("Kick"), None)]);
        assert_eq!(
            compare(&old, &new),
            vec![FlChange::GeneratorChanged {
                position: 0,
                channel: Some(s("Lead")),
                old: Some(s("Sytrus")),
                new: Some(s("FLEX")),
            }]
        );
    }

    #[test]
    fn a_rename_is_never_guessed_across_different_generators() {
        // Name and generator both changed: nothing ties the two channels
        // together, so they are one removed and one added.
        let old = with_rack(&[(Some("Sytrus"), Some("Sytrus"))]);
        let new = with_rack(&[(Some("FLEX"), Some("FLEX"))]);
        assert_eq!(
            compare(&old, &new),
            vec![
                FlChange::ChannelRemoved {
                    position: 0,
                    name: Some(s("Sytrus")),
                    generator: Some(s("Sytrus")),
                },
                FlChange::ChannelAdded {
                    position: 0,
                    name: Some(s("FLEX")),
                    generator: Some(s("FLEX")),
                },
            ]
        );
    }

    #[test]
    fn a_rename_keeping_the_generator_is_a_rename() {
        let old = with_rack(&[(Some("Kick"), None)]);
        let new = with_rack(&[(Some("808 Kick"), None)]);
        assert_eq!(
            compare(&old, &new),
            vec![FlChange::ChannelName {
                position: 0,
                change: NameChange::Renamed {
                    old: s("Kick"),
                    new: s("808 Kick"),
                },
            }]
        );
    }

    #[test]
    fn huge_racks_compare_in_linear_time() {
        // Crafted sizes: an earlier pairing was quadratic (20k channels vs
        // none took 5.9 s, 40k took 24.3 s, in a debug build).
        let many = |tag: &str, n: usize| -> Vec<RackChannel> {
            (0..n)
                .map(|i| ch(Some(&format!("{tag}{i}")), Some(&format!("{tag}g{i}"))))
                .collect()
        };
        let empty = Extracted::default();
        let a = Extracted {
            channel_rack: many("a", 40_000),
            ..Extracted::default()
        };
        let b = Extracted {
            channel_rack: many("b", 40_000),
            ..Extracted::default()
        };
        let started = std::time::Instant::now();
        assert_eq!(compare(&a, &empty).len(), 40_000);
        assert_eq!(compare(&empty, &a).len(), 40_000);
        assert_eq!(compare(&a, &b).len(), 80_000);
        assert!(compare(&a, &a).is_empty());
        let elapsed = started.elapsed();
        assert!(
            elapsed < std::time::Duration::from_secs(5),
            "took {elapsed:?}"
        );
    }

    // ---- patterns, inserts, arrangements ------------------------------ //

    #[test]
    fn patterns_are_added_and_removed_by_number() {
        let old = with_patterns(&[(1, Some("Intro")), (2, None)]);
        let new = with_patterns(&[(1, Some("Verse")), (3, None)]);
        assert_eq!(
            compare(&old, &new),
            vec![
                FlChange::PatternName {
                    number: 1,
                    change: NameChange::Renamed {
                        old: s("Intro"),
                        new: s("Verse"),
                    },
                },
                FlChange::PatternRemoved {
                    number: 2,
                    name: None,
                },
                FlChange::PatternAdded {
                    number: 3,
                    name: None,
                },
            ]
        );
    }

    #[test]
    fn patterns_are_not_compared_when_either_file_is_fl_25() {
        let mut old = with_patterns(&[(1, Some("Intro"))]);
        let new = Extracted {
            format_status: FormatStatus::PartialV25ScalarsUnreadable,
            ..Extracted::default()
        };
        assert!(compare(&old, &new).is_empty());
        old.format_status = FormatStatus::PartialV25ScalarsUnreadable;
        assert!(compare(&old, &new).is_empty());
    }

    #[test]
    fn inserts_are_only_added_or_removed_between_two_fl_25_saves() {
        // 105 vs 127 positions is two FL versions, not the user adding
        // inserts: only a name at a new position is news.
        let old = with_inserts(&[None, None]);
        let new = with_inserts(&[None, None, None, Some("Bus")]);
        assert_eq!(
            compare(&old, &new),
            vec![FlChange::MixerInsertName {
                position: 3,
                change: NameChange::Named { name: s("Bus") },
            }]
        );
        let v25 = |names: &[Option<&str>]| Extracted {
            format_status: FormatStatus::PartialV25ScalarsUnreadable,
            ..with_inserts(names)
        };
        assert_eq!(
            compare(&v25(&[None, None]), &v25(&[None, None, None])),
            vec![FlChange::MixerInsertAdded {
                position: 2,
                name: None,
            }]
        );
    }

    #[test]
    fn arrangements_are_compared_in_order() {
        let old = Extracted {
            arrangements: vec![Some(s("Arrangement"))],
            ..Extracted::default()
        };
        let new = Extracted {
            arrangements: vec![Some(s("Main")), Some(s("Radio edit"))],
            ..Extracted::default()
        };
        assert_eq!(
            compare(&old, &new),
            vec![
                FlChange::ArrangementName {
                    position: 0,
                    change: NameChange::Renamed {
                        old: s("Arrangement"),
                        new: s("Main"),
                    },
                },
                FlChange::ArrangementAdded {
                    position: 1,
                    name: Some(s("Radio edit")),
                },
            ]
        );
    }

    // ---- effects, tempo, bytes ---------------------------------------- //

    #[test]
    fn an_added_effect_is_reported_with_its_position() {
        let old = with_effects(&[("Fruity Limiter", 0)]);
        let new = with_effects(&[("Fruity Limiter", 0), ("Fruity Reeverb 2", 3)]);
        assert_eq!(
            compare(&old, &new),
            vec![FlChange::EffectAdded {
                name: s("Fruity Reeverb 2"),
                position: 3,
            }]
        );
    }

    #[test]
    fn an_effect_moved_to_another_position_is_removed_and_added() {
        let old = with_effects(&[("Fruity Reeverb 2", 3)]);
        let new = with_effects(&[("Fruity Reeverb 2", 4)]);
        assert_eq!(
            compare(&old, &new),
            vec![
                FlChange::EffectAdded {
                    name: s("Fruity Reeverb 2"),
                    position: 4,
                },
                FlChange::EffectRemoved {
                    name: s("Fruity Reeverb 2"),
                    position: 3,
                },
            ]
        );
    }

    #[test]
    fn tempo_change_is_reported_only_when_both_sides_are_known() {
        let at = |tempo| Extracted {
            tempo,
            ..Extracted::default()
        };
        assert_eq!(
            compare(&at(Tempo::Known(120.0)), &at(Tempo::Known(128.0))),
            vec![FlChange::TempoChanged {
                from_bpm: 120.0,
                to_bpm: 128.0
            }]
        );
        assert!(compare(&at(Tempo::Known(120.0)), &at(Tempo::Unknown)).is_empty());
        assert!(compare(
            &at(Tempo::Known(120.0)),
            &at(Tempo::PartialV25ScalarsUnreadable)
        )
        .is_empty());
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
        let old = named(&["Kick"]);
        let new = named(&["Kick", "Snare"]);
        assert_eq!(
            compare_with_bytes(&old, &new, false),
            vec![FlChange::ChannelAdded {
                position: 1,
                name: Some(s("Snare")),
                generator: None,
            }]
        );
    }
}
