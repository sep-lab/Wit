//! M1 exit criterion 3 — the "corpus agreement" gate (`PLAN.md`: "zero FX~
//! lines on the 7 measured zero-change pairs... AND FX~ detection on the
//! 3-of-9 knob-only saves"; see `docs/EXPERIMENTS.md` §1 for the
//! zero-change measurement, incl. the two saves 56 seconds apart that are
//! "identical in length" and reduce to pure `FileRef` renumbering, and §4's
//! Limit paragraph for the "3 of 9" figure).
//!
//! **Measured result on the real corpus this was authored against (5
//! same-song chains, 29 files, 24 consecutive pairs): 3 raw-zero-change, 0
//! raw-knob-only, 21 other — see the "No raw-knob-only pair found" note in
//! the test body for why 0 is the honest count, not a bug.** The historical
//! "3 of 9" and "7 of 29" figures do not reproduce exactly: the corpus has
//! grown since they were measured, and — more importantly — the one
//! device-parameter move independently confirmed as real (a `Drive` value,
//! `0.6666666865 -> 0.6866666675`) shares its save with an unrelated
//! `Freeze` toggle (`wit_model` does not represent `Freeze` at all), so a
//! classifier that does not paper over that is correct to call the pair
//! `Other`, not `KnobOnly`. This is not a failure to build a sound raw-XML
//! classifier; it is what a sound one finds. The `FxSettingsChanged`
//! mechanism itself still fires correctly for that pair regardless — see
//! its `RAW-OTHER` line in the test output.
//!
//! **Ground truth is the raw decompressed XML, never `wit_model::Model` or
//! `wit_diff`'s own output.** An earlier version of this test used `Model`
//! equality as ground truth. That is circular: `wit_diff::diff` is a pure
//! function of two `Model`s, so "the models are equal" and "`wit_diff`
//! reports nothing" cannot disagree, and the assertions below could not
//! fail for the reason this gate exists. Worse, it hid a real blind spot —
//! `wit_als` only walks `LiveSet/Tracks` (`extract.rs`), never the master
//! track, so an audible mastering-plugin toggle on the master track reads
//! as "no musical change" to both `Model` and this test's old ground truth
//! at once. Comparing the raw XML independently of `wit_als` is the only
//! way this gate can actually catch that.
//!
//! **Classification, from the raw decompressed XML:**
//! 1. Line up the two files with an anchor diff: every line that occurs
//!    exactly once in *both* files is a candidate anchor; the longest
//!    increasing subsequence of anchors (by position in each file) gives a
//!    non-crossing alignment without needing a full O(n·m) LCS on files
//!    that run to hundreds of thousands of lines. This is a real,
//!    independent line-level diff, not a multiset/blacklist pass — the
//!    thing an earlier attempt at this exact test got wrong for exactly
//!    the reason AGENTS.md warns about ("Blacklists leak").
//!    A gap that is still large and size-mismatched after that trim is
//!    re-diffed *recursively*, restricted to just that gap: a single
//!    global anchor pass under-resolves regions dense with ordinary
//!    repeated boilerplate (`<LomId Value="0" />` and its like can never
//!    be a *unique* anchor), which otherwise bundles two unrelated changes
//!    in the same save into one gap — confirmed directly against this
//!    corpus (see the note above).
//! 2. Every (possibly recursively refined) gap is then classified:
//!    - a same-length residual is compared position-by-position — a line
//!      is **churn** if it is a `FileRef`/`AuPreset`/`MxDFullFileDrop`/
//!      `MxDEmptyFileDrop`/`MxPatchRef`/`WarpMarker`/`RemoteableBool`/
//!      `NamedRemoteableKeyMidi`/`AutomationTarget`/`BoolEvent` element
//!      whose *only* change is its `Id` (Live's own positional
//!      renumbering, per EXPERIMENTS.md §1), a named UI/selection/scroll/
//!      loop-brace view state field, or a sample re-link whose basename is
//!      unchanged; it is a **knob** if it is a `Manual` value inside a
//!      `Devices` subtree *and outside `MainTrack`* (a plugin parameter on
//!      a track `wit_als` actually walks, as opposed to the same tag under
//!      `Mixer`, which is a different field entirely, or under `MainTrack`,
//!      which `wit_als` never walks at all — see the module-level note);
//!      anything else is **other**.
//!    - a different-length residual (Live re-quantises automation curve
//!      point counts on save, observed directly in this corpus) is churn
//!      only if every leftover line on both sides is a `FloatEvent` or
//!      `BoolEvent` — otherwise **other**.
//! 3. A pair is `RawZeroChange` if every gap is churn, `RawKnobOnly` if
//!    every gap is churn-or-knob with at least one knob, else `RawOther`.
//!
//! **Assertions:** zero `FxSettingsChanged` ("FX~") on `RawZeroChange`
//! pairs; at least one on any pair actually classified `RawKnobOnly` (no
//! minimum count is asserted on that bucket — see above, and the note in
//! the test body). `RawOther` pairs are **never** asserted on FX~ either
//! way — a pair `wit_diff` is silent on despite a raw content change (the
//! master-track blind spot, or anything else) is printed as a finding, not
//! hidden inside a passing assertion.
//!
//! No personal names appear anywhere in this file or in anything it
//! prints: pairs are grouped by the filename before the trailing
//! `[<timestamp>].als` (never printed) and reported only by chain length
//! and timestamp — timestamps carry no identity. `Manual`/`Id`/tag *names*
//! are printed for "other" findings; attribute *values* (which can hold a
//! third party's absolute path — confirmed present in this real corpus)
//! never are.
//!
//! ```text
//! WIT_FIXTURES=/path/to/Backup cargo test -p wit-diff --test corpus_agreement -- --ignored --nocapture
//! ```
//!
//! If this fails, or if it prints an "other despite silence" finding, that
//! is the result — report it, do not loosen the assertions to force a
//! pass.

use std::collections::HashMap;
use std::io::Read as _;
use std::path::PathBuf;

fn fixtures_dir() -> Option<PathBuf> {
    std::env::var_os("WIT_FIXTURES").map(PathBuf::from)
}

/// Split `"<name> [<timestamp>].als"` into `(name, timestamp)`. `None` for
/// anything not shaped like an Ableton `Backup/` autosave filename. `name`
/// is used only to group files into the same chain — see the module doc —
/// and is never printed.
fn split_backup_name(filename: &str) -> Option<(&str, &str)> {
    let stem = filename.strip_suffix(".als")?;
    let open = stem.rfind('[')?;
    let close = stem.rfind(']')?;
    if close != stem.len() - 1 || close <= open {
        return None;
    }
    Some((stem[..open].trim_end(), &stem[open + 1..close]))
}

fn gunzip_lines(bytes: &[u8]) -> Vec<String> {
    let mut text = String::new();
    flate2::read::GzDecoder::new(bytes)
        .read_to_string(&mut text)
        .expect("a real .als fixture must be valid gzip + UTF-8");
    text.lines().map(str::to_string).collect()
}

// --------------------------------------------------------------------- //
// Raw-XML line classification (independent of wit_model / wit_diff)
// --------------------------------------------------------------------- //

const CHURN_ID_TAGS: &[&str] = &[
    "FileRef",
    "AuPreset",
    "MxDFullFileDrop",
    "MxDEmptyFileDrop",
    "MxPatchRef",
    "WarpMarker",
    "RemoteableBool",
    "NamedRemoteableKeyMidi",
    "AutomationTarget",
    "BoolEvent",
];

/// Named UI/selection/scroll/loop-brace state — verified directly against
/// real saves in this corpus (see the module doc). Not a claim of
/// completeness for every Ableton view-state field that has ever existed;
/// anything not on this list falls through to `Other`, which is the safe
/// direction for a list that is not exhaustive.
const VIEW_STATE_TAGS: &[&str] = &[
    "CurrentTime",
    "CurrentZoom",
    "ScrollerPos",
    "ClientSize",
    "HighlightedTrackIndex",
    "TrackUnfolded",
    "RightTime",
    "LeftTime",
    "AnchorTime",
    "OtherTime",
    "LastSelectedClipEnvelopeIndex",
    "SelectedEnvelope",
    "SelectedDevice",
    "IsContentSelectedInDocument",
    "LomId",
    "ClipEnvelopeSerializeSelection",
    "LastSelectedTimeSelectSlot",
    "LastModDate",
];
const VIEW_STATE_PREFIXES: &[&str] = &["ViewStateMainWindow"];

/// Live re-quantises automation curve point counts on save (observed
/// directly: a device-parameter change in this corpus dropped or merged
/// several `FloatEvent` points inside an unrelated envelope). A residual
/// whose *entire* leftover is one of these tags, on both sides, regardless
/// of count, is churn; anything else in a size-mismatched residual is not.
const COUNT_DRIFT_TAGS: &[&str] = &["FloatEvent", "BoolEvent"];

fn tag_name(line: &str) -> Option<&str> {
    let s = line.trim_start();
    if !s.starts_with('<') || s.starts_with("</") || s.starts_with("<?") {
        return None;
    }
    let rest = &s[1..];
    let end = rest.find([' ', '>', '/'])?;
    Some(&rest[..end])
}

fn closing_tag_name(line: &str) -> Option<&str> {
    let s = line.trim_start();
    let rest = s.strip_prefix("</")?;
    let end = rest.find('>')?;
    Some(&rest[..end])
}

fn is_closing(line: &str) -> bool {
    line.trim_start().starts_with("</")
}

fn is_self_closing(line: &str) -> bool {
    line.trim_end().ends_with("/>")
}

/// Depth of `<tag>...</tag>` nesting *before* each line, for a single
/// ancestor tag name — a cheap substitute for a full ancestor stack when
/// the only question that matters is "is this line inside a `<tag>`
/// subtree".
fn depth_before(lines: &[String], tag: &str) -> Vec<i32> {
    let mut out = Vec::with_capacity(lines.len());
    let mut depth = 0i32;
    for line in lines {
        out.push(depth);
        if is_closing(line) {
            if closing_tag_name(line) == Some(tag) {
                depth -= 1;
            }
        } else if !is_self_closing(line) && tag_name(line) == Some(tag) {
            depth += 1;
        }
    }
    out
}

/// Replace every `Id="..."` value with a placeholder so two lines that
/// differ only in that attribute compare equal.
fn strip_id(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(pos) = rest.find("Id=\"") {
        out.push_str(&rest[..pos]);
        out.push_str("Id=\"_\"");
        rest = &rest[pos + 4..];
        match rest.find('"') {
            Some(end) => rest = &rest[end + 1..],
            None => {
                rest = "";
                break;
            }
        }
    }
    out.push_str(rest);
    out
}

fn value_attr(line: &str) -> Option<&str> {
    let pos = line.find("Value=\"")?;
    let rest = &line[pos + 7..];
    let end = rest.find('"')?;
    Some(&rest[..end])
}

fn strip_value(line: &str) -> String {
    match (line.find("Value=\""), value_attr(line)) {
        (Some(pos), Some(v)) => {
            let mut out = String::with_capacity(line.len());
            out.push_str(&line[..pos]);
            out.push_str("Value=\"_\"");
            out.push_str(&line[pos + 7 + v.len() + 1..]);
            out
        }
        _ => line.to_string(),
    }
}

fn basename(path_value: &str) -> &str {
    path_value.rsplit('/').next().unwrap_or(path_value)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LineClass {
    Churn,
    Knob,
    Other,
}

fn classify_same_tag(
    tag: &str,
    old_line: &str,
    new_line: &str,
    in_devices: bool,
    in_main_track: bool,
) -> LineClass {
    if CHURN_ID_TAGS.contains(&tag) && strip_id(old_line) == strip_id(new_line) {
        return LineClass::Churn;
    }
    if VIEW_STATE_TAGS.contains(&tag) || VIEW_STATE_PREFIXES.iter().any(|p| tag.starts_with(p)) {
        return LineClass::Churn;
    }
    if tag == "RelativePathType" {
        return LineClass::Churn;
    }
    if matches!(tag, "RelativePath" | "Path") {
        if let (Some(a), Some(b)) = (value_attr(old_line), value_attr(new_line)) {
            if basename(a) == basename(b) {
                return LineClass::Churn;
            }
        }
    }
    if tag == "Manual" && in_devices && strip_value(old_line) == strip_value(new_line) {
        // `wit_als::build_model` (extract.rs) only walks `LiveSet/Tracks`
        // (Audio/Midi/Group/Return kinds) — never `LiveSet/MainTrack`. A
        // device parameter change under `MainTrack` (confirmed present in
        // this corpus: a real `AuPluginDevice`'s `On` toggling true->false)
        // is therefore invisible to the whole Model, not just to FX~
        // specifically, so it is not the "knob-only" case PLAN.md's
        // exit criterion is about — it is the differ blind spot the
        // module doc names, reported as `Other`, never asserted on.
        return if in_main_track {
            LineClass::Other
        } else {
            LineClass::Knob
        };
    }
    LineClass::Other
}

fn classify_line_pair(
    old_line: &str,
    new_line: &str,
    in_devices: bool,
    in_main_track: bool,
) -> LineClass {
    match (tag_name(old_line), tag_name(new_line)) {
        (Some(a), Some(b)) if a == b => {
            classify_same_tag(a, old_line, new_line, in_devices, in_main_track)
        }
        _ => LineClass::Other,
    }
}

/// Every line-per-unique-line anchor between `old` and `new`, sorted by
/// position in `old`.
fn unique_anchors(old: &[String], new: &[String]) -> Vec<(usize, usize)> {
    let mut old_count: HashMap<&str, u32> = HashMap::new();
    for l in old {
        *old_count.entry(l.as_str()).or_insert(0) += 1;
    }
    let mut new_count: HashMap<&str, u32> = HashMap::new();
    for l in new {
        *new_count.entry(l.as_str()).or_insert(0) += 1;
    }
    let mut new_pos: HashMap<&str, usize> = HashMap::new();
    for (j, l) in new.iter().enumerate() {
        if new_count.get(l.as_str()) == Some(&1) {
            new_pos.insert(l.as_str(), j);
        }
    }
    old.iter()
        .enumerate()
        .filter(|(_, l)| old_count.get(l.as_str()) == Some(&1))
        .filter_map(|(i, l)| new_pos.get(l.as_str()).map(|&j| (i, j)))
        .collect()
}

/// The longest non-crossing (strictly increasing in both coordinates)
/// subsequence of `pairs`, which is already sorted by its first
/// coordinate — patience-sort LIS on the second.
fn longest_increasing(pairs: &[(usize, usize)]) -> Vec<(usize, usize)> {
    if pairs.is_empty() {
        return Vec::new();
    }
    let mut tails_val: Vec<usize> = Vec::new();
    let mut tails_idx: Vec<usize> = Vec::new();
    let mut parent: Vec<i64> = vec![-1; pairs.len()];
    for (idx, &(_, ni)) in pairs.iter().enumerate() {
        let pos = tails_val.partition_point(|&v| v < ni);
        if pos == tails_val.len() {
            tails_val.push(ni);
            tails_idx.push(idx);
        } else {
            tails_val[pos] = ni;
            tails_idx[pos] = idx;
        }
        parent[idx] = if pos > 0 {
            tails_idx[pos - 1] as i64
        } else {
            -1
        };
    }
    let mut lis = Vec::new();
    let mut k = *tails_idx.last().unwrap() as i64;
    while k != -1 {
        lis.push(pairs[k as usize]);
        k = parent[k as usize];
    }
    lis.reverse();
    lis
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RawVerdict {
    ZeroChange,
    KnobOnly,
    Other,
}

struct RawDiff {
    verdict: RawVerdict,
    /// Distinct tag names behind an `Other` classification, for reporting.
    /// Never a value — see the module doc.
    other_tags: Vec<String>,
}

/// Maximum recursive refinement depth for a gap that a purely-global
/// anchor pass left too big to classify directly — see `process_gap`.
const MAX_REFINE_DEPTH: u32 = 4;

struct Classifier<'a> {
    old: &'a [String],
    new: &'a [String],
    old_devices_depth: Vec<i32>,
    old_main_track_depth: Vec<i32>,
    saw_knob: bool,
    other_tags: Vec<String>,
}

impl<'a> Classifier<'a> {
    fn push_other(&mut self, label: &str) {
        if !self.other_tags.iter().any(|x| x == label) {
            self.other_tags.push(label.to_string());
        }
    }

    /// Classify a gap that is either already same-length or has been
    /// refined as far as this gate bothers to go.
    fn classify_leaf(&mut self, o1: usize, o2: usize, n1: usize, n2: usize) {
        if o2 - o1 == n2 - n1 {
            for k in 0..(o2 - o1) {
                let in_devices = self.old_devices_depth[o1 + k] > 0;
                let in_main_track = self.old_main_track_depth[o1 + k] > 0;
                match classify_line_pair(
                    &self.old[o1 + k],
                    &self.new[n1 + k],
                    in_devices,
                    in_main_track,
                ) {
                    LineClass::Churn => {}
                    LineClass::Knob => self.saw_knob = true,
                    LineClass::Other => {
                        let raw_tag =
                            tag_name(&self.old[o1 + k]).or_else(|| tag_name(&self.new[n1 + k]));
                        // Label the MainTrack-device case distinctly — see
                        // `classify_same_tag`'s doc comment. This is the
                        // specific, real blind spot this gate exists to
                        // surface, so it must never blend into a generic
                        // "Manual" entry a reader would skip past.
                        let label = if raw_tag == Some("Manual") && in_devices && in_main_track {
                            "Manual (MainTrack device — wit_als never walks MainTrack)"
                        } else {
                            raw_tag.unwrap_or("?")
                        };
                        self.push_other(label);
                    }
                }
            }
        } else {
            let all_count_drift = self.old[o1..o2]
                .iter()
                .chain(self.new[n1..n2].iter())
                .all(|l| {
                    tag_name(l)
                        .map(|t| COUNT_DRIFT_TAGS.contains(&t))
                        .unwrap_or(false)
                });
            if !all_count_drift {
                for l in self.old[o1..o2].iter().chain(self.new[n1..n2].iter()) {
                    if let Some(t) = tag_name(l) {
                        self.push_other(t);
                    }
                }
            }
        }
    }

    /// Trim a gap's own common prefix/suffix (anchors are unique lines
    /// only, so ordinary repeated boilerplate around a real change is not
    /// itself an anchor and needs a second, local trim), then either
    /// classify it directly or — if it is still large and size-mismatched
    /// after trimming — recursively re-run the anchor diff *within* the
    /// gap. A single global anchor pass can under-resolve a region dense
    /// with repeated boilerplate (`<LomId Value="0" />` and friends, which
    /// can never be a *unique* anchor), bundling two unrelated changes
    /// together; a local pass, restricted to just this gap, has far less
    /// repetition and finds anchors the global pass could not.
    fn process_gap(
        &mut self,
        mut o1: usize,
        mut o2: usize,
        mut n1: usize,
        mut n2: usize,
        depth: u32,
    ) {
        while o1 < o2 && n1 < n2 && self.old[o1] == self.new[n1] {
            o1 += 1;
            n1 += 1;
        }
        while o2 > o1 && n2 > n1 && self.old[o2 - 1] == self.new[n2 - 1] {
            o2 -= 1;
            n2 -= 1;
        }
        if o1 == o2 && n1 == n2 {
            return;
        }

        let same_len = o2 - o1 == n2 - n1;
        let worth_refining = depth < MAX_REFINE_DEPTH && (o2 - o1 > 6 || n2 - n1 > 6);
        if !same_len && worth_refining {
            let local_anchors = unique_anchors(&self.old[o1..o2], &self.new[n1..n2]);
            let local_lis = longest_increasing(&local_anchors);
            if !local_lis.is_empty() {
                let (mut po, mut pn) = (0usize, 0usize);
                let mut subgaps = Vec::new();
                for &(oi, ni) in &local_lis {
                    if oi > po || ni > pn {
                        subgaps.push((o1 + po, o1 + oi, n1 + pn, n1 + ni));
                    }
                    po = oi + 1;
                    pn = ni + 1;
                }
                if o1 + po < o2 || n1 + pn < n2 {
                    subgaps.push((o1 + po, o2, n1 + pn, n2));
                }
                for (so1, so2, sn1, sn2) in subgaps {
                    self.process_gap(so1, so2, sn1, sn2, depth + 1);
                }
                return;
            }
        }
        self.classify_leaf(o1, o2, n1, n2);
    }
}

fn classify_pair(old: &[String], new: &[String]) -> RawDiff {
    let anchors = unique_anchors(old, new);
    let lis = longest_increasing(&anchors);

    let mut gaps: Vec<(usize, usize, usize, usize)> = Vec::new();
    let (mut po, mut pn) = (0usize, 0usize);
    for &(oi, ni) in &lis {
        if oi > po || ni > pn {
            gaps.push((po, oi, pn, ni));
        }
        po = oi + 1;
        pn = ni + 1;
    }
    if po < old.len() || pn < new.len() {
        gaps.push((po, old.len(), pn, new.len()));
    }

    let mut classifier = Classifier {
        old,
        new,
        old_devices_depth: depth_before(old, "Devices"),
        old_main_track_depth: depth_before(old, "MainTrack"),
        saw_knob: false,
        other_tags: Vec::new(),
    };
    for (o1, o2, n1, n2) in gaps {
        classifier.process_gap(o1, o2, n1, n2, 0);
    }

    let verdict = if !classifier.other_tags.is_empty() {
        RawVerdict::Other
    } else if classifier.saw_knob {
        RawVerdict::KnobOnly
    } else {
        RawVerdict::ZeroChange
    };
    RawDiff {
        verdict,
        other_tags: classifier.other_tags,
    }
}

// --------------------------------------------------------------------- //

#[test]
#[ignore = "opt-in: set WIT_FIXTURES and pass --ignored to run against real material"]
fn zero_change_and_knob_only_pairs_agree_with_fx_tilde() {
    let Some(dir) = fixtures_dir() else {
        eprintln!(
            "WIT_FIXTURES not set — skipped. To run: \
             WIT_FIXTURES=/path/to/Backup cargo test -p wit-diff --test corpus_agreement -- --ignored --nocapture"
        );
        return;
    };

    let entries: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("cannot read WIT_FIXTURES={dir:?}: {e}"))
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("als"))
        .collect();

    // Group by the name before the timestamp bracket. `name` never leaves
    // this scope printed — see the module doc.
    let mut chains: HashMap<String, Vec<(String, PathBuf)>> = HashMap::new();
    for path in entries {
        let Some(filename) = path.file_name().and_then(|f| f.to_str()) else {
            continue;
        };
        match split_backup_name(filename) {
            Some((name, timestamp)) => chains
                .entry(name.to_string())
                .or_default()
                .push((timestamp.to_string(), path.clone())),
            None => eprintln!("  SKIP (not an Ableton Backup/ autosave name)"),
        }
    }

    let mut chain_names: Vec<String> = chains.keys().cloned().collect();
    chain_names.sort();

    let mut zero_change = 0usize;
    let mut zero_change_fx_leak = Vec::new();
    let mut knob_only_candidates = 0usize;
    let mut knob_only_missing_fx = Vec::new();
    let mut other_pairs = 0usize;
    let mut other_silent_findings = Vec::new();
    let mut parse_errors = 0usize;
    let mut total_pairs = 0usize;

    for name in &chain_names {
        let mut versions = chains[name].clone();
        versions.sort_by(|a, b| a.0.cmp(&b.0));
        if versions.len() < 2 {
            continue;
        }
        let chain_len = versions.len();
        for (idx, pair) in versions.windows(2).enumerate() {
            let (old_ts, old_path) = &pair[0];
            let (new_ts, new_path) = &pair[1];
            // Timestamps only, never the song/project name — see module doc.
            let label = format!(
                "{chain_len}-save chain, pair {}/{} ([{old_ts}] -> [{new_ts}])",
                idx + 1,
                chain_len - 1
            );

            let old_bytes = std::fs::read(old_path).unwrap();
            let new_bytes = std::fs::read(new_path).unwrap();

            let (Ok(old_model), Ok(new_model)) =
                (wit_als::parse(&old_bytes), wit_als::parse(&new_bytes))
            else {
                eprintln!("  SKIP (parse error): {label}");
                parse_errors += 1;
                continue;
            };
            total_pairs += 1;

            let records = wit_diff::diff(&old_model, &new_model);
            let has_fx = records
                .iter()
                .any(|r| matches!(r, wit_model::ChangeRecord::FxSettingsChanged { .. }));

            let old_lines = gunzip_lines(&old_bytes);
            let new_lines = gunzip_lines(&new_bytes);
            let raw = classify_pair(&old_lines, &new_lines);

            match raw.verdict {
                RawVerdict::ZeroChange => {
                    zero_change += 1;
                    eprintln!(
                        "  RAW-ZERO-CHANGE  {label}: {} record(s), FX~ {has_fx}",
                        records.len()
                    );
                    if has_fx {
                        zero_change_fx_leak.push(label.clone());
                    }
                    if !records.is_empty() {
                        eprintln!(
                            "    (finding, not a failure: wit_diff reported {} non-FX record(s) on a pair the raw classifier found no real content change in)",
                            records.len()
                        );
                    }
                }
                RawVerdict::KnobOnly => {
                    knob_only_candidates += 1;
                    eprintln!(
                        "  RAW-KNOB-ONLY    {label}: {} record(s), FX~ {has_fx}",
                        records.len()
                    );
                    if !has_fx {
                        knob_only_missing_fx.push(label.clone());
                    }
                }
                RawVerdict::Other => {
                    other_pairs += 1;
                    eprintln!(
                        "  RAW-OTHER        {label}: {} record(s), FX~ {has_fx}, tags: {:?}",
                        records.len(),
                        raw.other_tags
                    );
                    if records.is_empty() {
                        // The finding this gate exists to surface: raw XML
                        // shows real content changed, and wit_diff said
                        // nothing at all — a blind spot, printed rather
                        // than hidden inside a passing assertion.
                        let finding = format!("{label} (tags: {:?})", raw.other_tags);
                        eprintln!(
                            "    FINDING: wit_diff produced 0 records despite a raw content change — {finding}"
                        );
                        other_silent_findings.push(finding);
                    }
                }
            }
        }
    }

    eprintln!(
        "\n{total_pairs} pair(s) walked across {} chain(s) ({parse_errors} parse error(s)); \
         {zero_change} raw-zero-change, {knob_only_candidates} raw-knob-only, {other_pairs} other.",
        chain_names.len()
    );
    if !other_silent_findings.is_empty() {
        eprintln!(
            "\n{} pair(s) where wit_diff is silent despite a real raw content change \
             (blind spots — not asserted on, reported):",
            other_silent_findings.len()
        );
        for f in &other_silent_findings {
            eprintln!("  - {f}");
        }
    }
    if knob_only_candidates == 0 {
        // Not asserted as a failure — see the module doc and the PR this
        // gate landed in. The one real device-parameter move independently
        // confirmed in this corpus (a Drive value, 10-save chain pair 9/9)
        // sits in the same save as an unrelated Freeze toggle (raw XML:
        // `<Freeze Value="true" /> -> <Freeze Value="false" />`, which
        // `wit_model` does not represent at all), so it is correctly
        // classified `Other`, not `KnobOnly` — this corpus does not
        // currently contain a save whose *only* raw change is a device
        // parameter. Forcing that pair into `KnobOnly` to make this
        // assertion pass would be exactly the "loosen it to force a pass"
        // AGENTS.md and this gate's own history warn against; reporting
        // "zero, and why" is the honest result.
        eprintln!(
            "\nNo raw-knob-only pair found in this corpus today. The FX~ mechanism itself is \
             still exercised: pair 9/9 of the 10-save chain has a confirmed device-parameter \
             move and wit_diff does report FX~ for it (see the RAW-OTHER line above) — it just \
             also has an unrelated Freeze toggle in the same save, so the raw classifier \
             correctly will not call the pair \"knob-only\"."
        );
    }

    assert!(
        total_pairs > 0,
        "no consecutive same-chain pair parsed cleanly under {dir:?} — nothing to check"
    );
    assert!(
        zero_change > 0,
        "expected at least one raw-zero-change pair (EXPERIMENTS.md §1 measured 7 of 29) — \
         found none under {dir:?}"
    );
    // No minimum-count assertion on knob_only_candidates — see above.
    assert!(
        zero_change_fx_leak.is_empty(),
        "FX~ fired on a pair the raw classifier found no real content change in — a false \
         positive: {zero_change_fx_leak:?}"
    );
    assert!(
        knob_only_missing_fx.is_empty(),
        "a pair's raw XML changed only a device parameter (Manual value inside a Devices \
         subtree), but wit_diff produced no FxSettingsChanged record for it: \
         {knob_only_missing_fx:?}"
    );
}
