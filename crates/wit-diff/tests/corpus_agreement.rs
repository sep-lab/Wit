//! M1 corpus-agreement check — evidence *toward* ROADMAP exit criterion 3
//! ("the differ agrees with subtree hashing on which saves are musically
//! empty — closing the known device-parameter gap"), **not a closure of
//! it.** `wit-planning/PLAN.md` states the gate as two halves: zero `FX~`
//! lines on saves with no real change, and `FX~` detection on saves whose
//! only real change is a device parameter ("knob-only"). See
//! `docs/EXPERIMENTS.md` §1 for the historical "7 of 29 zero-change" figure
//! and §4's Limit paragraph for the historical "3 of 9" knob-only figure.
//!
//! **What this file can and cannot fail on today:**
//! - **The false-`FX~` half can fail on real material.** On the real
//!   corpus this was measured against (a `Backup/` folder of 29 files in 5
//!   same-name chains, 24 consecutive pairs, all saved on or before
//!   2026-05-12) the raw classifier below finds 6 zero-change pairs — each
//!   also hand-checked against a plain `diff` of the decompressed XML to be
//!   only id renumbering, view state and same-basename relinks — and
//!   `wit_diff` reports no `FX~` on any of them. The other 18 pairs are
//!   `Other`; in 7 of those `wit_diff` reports nothing at all (printed as
//!   findings). If device fingerprinting ever picked up id churn, the
//!   zero-change pairs fail (mutation-checked: making `wit_als`'s
//!   fingerprint also hash `Id` attributes fails all 6).
//! - **The knob-only half is untested on real material.** The same corpus
//!   has 0 knob-only pairs: the 3 saves that move a device parameter on a
//!   walked track each change something else too, so they land in
//!   `Other`, where nothing is asserted (the run prints whether `FX~` was
//!   reported for them). On that corpus the knob half cannot fail —
//!   removing `FxSettingsChanged` detection from `wit_diff` entirely still
//!   passes the real run. It is exercised only by the synthetic, code-built
//!   fixture in this file
//!   (`synthetic_knob_only_pair_is_knob_only_and_reported_as_fx_tilde`),
//!   which runs in CI and does fail under that mutation.
//! - **Exit criterion 3 stays open.** Besides the untested knob half on
//!   real saves, `wit_als` never walks `LiveSet/MainTrack` (a real
//!   master-track device toggle in this corpus is invisible to the whole
//!   `Model`); that gap stays open until the Ableton extractor work in
//!   PR #65 lands.
//!
//! The historical "7 of 29" and "3 of 9" figures do not reproduce here,
//! and that is a difference of **method, not of corpus growth.** Nothing
//! was added: EXPERIMENTS.md describes the corpus behind "7 of 29" as 30
//! autosaves spanning Feb–May 2026; the measured folder holds 29 `.als`
//! files, none dated after 2026-05-12, and "7 of 29" was committed on
//! 2026-08-05. That pass took 29 consecutive pairs across the whole
//! 30-file corpus and classified raw changed lines; this one pairs only
//! saves of the same set name (24 pairs from 29 files) and classifies
//! each aligned line pair as described below.
//!
//! **Ground truth is the raw decompressed XML, never `wit_model::Model` or
//! `wit_diff`'s own output.** An earlier version of this test used `Model`
//! equality as ground truth. That is circular: `wit_diff::diff` is a pure
//! function of two `Model`s, so "the models are equal" and "`wit_diff`
//! reports nothing" cannot disagree, and the assertions could not fail for
//! the reason this check exists. It also hid the master-track blind spot
//! above.
//!
//! **Classification, from the raw decompressed XML:**
//! 1. Line up the two files with an anchor diff: every line that occurs
//!    exactly once in *both* files is a candidate anchor; the longest
//!    increasing subsequence of anchors gives a non-crossing alignment
//!    without an O(n·m) LCS over files of hundreds of thousands of lines.
//!    A gap that is still large and size-mismatched after trimming its own
//!    common prefix/suffix is re-diffed recursively within just that gap
//!    (repeated boilerplate such as `<LomId Value="0" />` can never be a
//!    *unique* anchor, so one global pass under-resolves dense regions).
//!    Alignment compares lines with the `Id` of the positional-id tags
//!    listed in step 3 masked (`align_key`): Live renumbers those by +1, so
//!    a renumbered line can be byte-identical to a *different* element's old
//!    line and would otherwise become a false anchor. Masking there is the
//!    same rule step 3 applies ("only the `Id` changed" is churn), applied
//!    earlier; classification itself always reads the raw lines.
//! 2. Anchors are unique lines only, so a gap still holds **unchanged**
//!    lines between its edits (repeated closing tags, `LomId`s, sibling
//!    tracks' identical device lines, ...). Those are dropped before
//!    anything is classified:
//!    - a same-length gap is compared position by position, skipping every
//!      position where the old and new line are identical;
//!    - a different-length gap is aligned with a real LCS, and only the
//!      unmatched lines count. Unmatched runs of equal length are then
//!      compared position by position as above; the rest exist on one side
//!      only.
//! 3. Each remaining changed line pair is:
//!    - **churn** if it is a `FileRef`/`AuPreset`/`MxDFullFileDrop`/
//!      `MxDEmptyFileDrop`/`MxPatchRef`/`WarpMarker`/`RemoteableBool`/
//!      `NamedRemoteableKeyMidi`/`AutomationTarget`/`BoolEvent`/`FloatEvent`
//!      element whose *only* change is its `Id` (Live's positional
//!      renumbering, EXPERIMENTS.md §1: one upstream insertion shifts every
//!      later id), a named UI/selection/scroll/loop-brace view-state field,
//!      a `RelativePathType` flip, or a sample `Path`/`RelativePath` rewrite
//!      whose basename is unchanged (see "same-basename relinks" below);
//!    - a **knob** if it is a `Manual` value inside a `Devices` subtree of a
//!      track under `LiveSet/Tracks` (the tracks `wit_als` walks — not the
//!      `Mixer`'s own `Manual`s, and not `MainTrack`/`PreHearTrack`
//!      devices, which `wit_als` never visits);
//!    - **other** in every other case — including any line that exists on
//!      only one side. In particular an automation breakpoint added or
//!      removed (a `FloatEvent`/`BoolEvent` count change) is a real edit to
//!      an envelope and is `Other`, not churn.
//! 4. A pair is `RawZeroChange` if every changed line is churn,
//!    `RawKnobOnly` if every changed line is churn-or-knob with at least
//!    one knob, else `RawOther`.
//!
//! **Same-basename relinks.** `wit_als` already reduces a sample reference
//! to its basename (`extract.rs`), and the audio a `FileRef` points at is
//! pinned by its `OriginalFileSize`/`OriginalCrc` lines, which are compared
//! like any other line — a relink to different audio changes them and lands
//! in `Other`. On the measured corpus the relinks this rule absorbs are
//! whole-folder moves: in one pair, 777 absolute `Path` values change and
//! every one of them is the same single directory-prefix rewrite with the
//! remainder of the path identical, while `RelativePath`, `OriginalCrc` and
//! `OriginalFileSize` are unchanged on every one.
//!
//! **Assertions:** zero `FxSettingsChanged` ("FX~") on `RawZeroChange`
//! pairs; at least one on any pair classified `RawKnobOnly` (no minimum
//! count is asserted on that bucket, see above). `RawOther` pairs are never
//! asserted on either way: a pair `wit_diff` is silent on despite a real
//! raw change is printed as a finding, and a pair that moves a device
//! parameter *alongside* other changes is printed with whether `wit_diff`
//! reported `FX~` for it.
//!
//! No personal names appear anywhere in this file or in anything it
//! prints: pairs are grouped by the filename before the trailing
//! `[<timestamp>].als` (never printed) and reported only by chain length
//! and timestamp. Tag *names* are printed for "other" findings; attribute
//! *values* (which can hold a third party's absolute path — confirmed
//! present in this real corpus) never are.
//!
//! ```text
//! WIT_FIXTURES=/path/to/Backup cargo test -p wit-diff --test corpus_agreement -- --ignored --nocapture
//! ```
//!
//! If this fails, or if it prints a finding, that is the result — report
//! it, do not loosen the assertions to force a pass.

use std::borrow::Cow;
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

/// Elements that carry a positional `Id` Live renumbers on save. A line of
/// one of these whose only change is its `Id` is churn.
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
    // Automation breakpoints: the same `Time`/`Value` with a shifted `Id`
    // (observed in the corpus, 10-save chain pair 9/9: 22 breakpoints
    // renumbered by +1 to +16). A breakpoint *added or removed* is not
    // this — it is a line on one side only, and is `Other`.
    "FloatEvent",
];

/// Named UI/selection/scroll/loop-brace state — verified directly against
/// real saves in this corpus. Not a claim of completeness for every Ableton
/// view-state field that has ever existed; anything not on this list falls
/// through to `Other`, which is the safe direction for a list that is not
/// exhaustive.
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

/// Automation breakpoint elements — used only to label an added/removed
/// breakpoint distinctly in a finding. They are never churn when a line
/// exists on one side only.
const BREAKPOINT_TAGS: &[&str] = &["FloatEvent", "BoolEvent"];

/// An unequal-length gap is aligned with a quadratic LCS only up to this
/// many cells (16 MB of `u32`); beyond it the gap is reported, whole, as
/// an unaligned `Other` region — conservative, never churn.
const LCS_CELL_CAP: usize = 4_000_000;

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

/// A changed line's label in a finding: its element name, `/Name` for a
/// closing tag, `(text)` for anything else. Never a value.
fn line_label(line: &str) -> String {
    if let Some(t) = tag_name(line) {
        t.to_string()
    } else if let Some(t) = closing_tag_name(line) {
        format!("/{t}")
    } else {
        "(text)".to_string()
    }
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

/// A line's alignment key: itself, or — for a positional-id element — the
/// line with its `Id` masked, so a renumbered element still lines up with
/// its old self. Masking only these tags is the same rule the churn
/// classification applies, not a looser one.
fn align_key(line: &str) -> Cow<'_, str> {
    match tag_name(line) {
        Some(t) if CHURN_ID_TAGS.contains(&t) => Cow::Owned(strip_id(line)),
        _ => Cow::Borrowed(line),
    }
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

/// Where a line sits, as far as the knob rule cares.
#[derive(Debug, Clone, Copy)]
struct Place {
    in_devices: bool,
    in_tracks: bool,
    in_main_track: bool,
}

fn classify_same_tag(tag: &str, old_line: &str, new_line: &str, place: Place) -> LineClass {
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
    if tag == "Manual"
        && place.in_devices
        && place.in_tracks
        && strip_value(old_line) == strip_value(new_line)
    {
        return LineClass::Knob;
    }
    LineClass::Other
}

/// The label for an `Other` line pair whose tags match. A `Manual` inside a
/// device that `wit_als` never walks (`MainTrack`, `PreHearTrack`) is the
/// specific blind spot this check exists to surface, so it is named, never
/// blended into a generic "Manual" a reader would skip past.
fn other_label(tag: &str, place: Place) -> String {
    if tag == "Manual" && place.in_devices && !place.in_tracks {
        if place.in_main_track {
            "Manual (MainTrack device — wit_als never walks MainTrack)".to_string()
        } else {
            "Manual (device outside LiveSet/Tracks — wit_als never walks it)".to_string()
        }
    } else {
        tag.to_string()
    }
}

/// Every line-per-unique-line anchor between `old` and `new` (alignment
/// keys, see `align_key`), sorted by position in `old`.
fn unique_anchors<S: AsRef<str>>(old: &[S], new: &[S]) -> Vec<(usize, usize)> {
    let mut old_count: HashMap<&str, u32> = HashMap::new();
    for l in old {
        *old_count.entry(l.as_ref()).or_insert(0) += 1;
    }
    let mut new_count: HashMap<&str, u32> = HashMap::new();
    for l in new {
        *new_count.entry(l.as_ref()).or_insert(0) += 1;
    }
    let mut new_pos: HashMap<&str, usize> = HashMap::new();
    for (j, l) in new.iter().enumerate() {
        if new_count.get(l.as_ref()) == Some(&1) {
            new_pos.insert(l.as_ref(), j);
        }
    }
    old.iter()
        .enumerate()
        .filter(|(_, l)| old_count.get(l.as_ref()) == Some(&1))
        .filter_map(|(i, l)| new_pos.get(l.as_ref()).map(|&j| (i, j)))
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

/// Matched index pairs of a longest common subsequence of `a` and `b`
/// (quadratic DP — the caller bounds the size).
fn lcs_matches(a: &[Cow<'_, str>], b: &[Cow<'_, str>]) -> Vec<(usize, usize)> {
    let (n, m) = (a.len(), b.len());
    let w = m + 1;
    // dp[i * w + j] = LCS length of a[i..] and b[j..].
    let mut dp = vec![0u32; (n + 1) * w];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[i * w + j] = if a[i] == b[j] {
                dp[(i + 1) * w + j + 1] + 1
            } else {
                dp[(i + 1) * w + j].max(dp[i * w + j + 1])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    let mut out = Vec::new();
    while i < n && j < m {
        if a[i] == b[j] {
            out.push((i, j));
            i += 1;
            j += 1;
        } else if dp[(i + 1) * w + j] >= dp[i * w + j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RawVerdict {
    ZeroChange,
    KnobOnly,
    Other,
}

struct RawDiff {
    verdict: RawVerdict,
    /// True if any changed line was a knob move — also for `Other` pairs,
    /// where it means "a device parameter moved alongside other changes".
    saw_knob: bool,
    /// Distinct labels behind an `Other` classification, for reporting.
    /// Never a value — see the module doc.
    other_tags: Vec<String>,
}

/// Maximum recursive refinement depth for a gap that a purely-global
/// anchor pass left too big to classify directly — see `process_gap`.
const MAX_REFINE_DEPTH: u32 = 4;

struct Classifier<'a> {
    old: &'a [String],
    new: &'a [String],
    /// `align_key` of every line — what alignment (anchors, trimming, LCS)
    /// compares; classification always looks at the raw lines.
    old_keys: Vec<Cow<'a, str>>,
    new_keys: Vec<Cow<'a, str>>,
    old_devices_depth: Vec<i32>,
    old_tracks_depth: Vec<i32>,
    old_main_track_depth: Vec<i32>,
    saw_knob: bool,
    other_tags: Vec<String>,
}

impl<'a> Classifier<'a> {
    fn push_other(&mut self, label: String) {
        if !self.other_tags.contains(&label) {
            self.other_tags.push(label);
        }
    }

    fn place(&self, old_index: usize) -> Place {
        Place {
            in_devices: self.old_devices_depth[old_index] > 0,
            in_tracks: self.old_tracks_depth[old_index] > 0,
            in_main_track: self.old_main_track_depth[old_index] > 0,
        }
    }

    /// Compare `len` lines position by position from `old[o]` / `new[n]`,
    /// skipping every position where the two lines are identical — a gap
    /// between unique anchors still contains unchanged lines, and those
    /// are not changes.
    fn classify_aligned(&mut self, o: usize, n: usize, len: usize) {
        for k in 0..len {
            let (old_line, new_line) = (&self.old[o + k], &self.new[n + k]);
            if old_line == new_line {
                continue;
            }
            let place = self.place(o + k);
            match (tag_name(old_line), tag_name(new_line)) {
                (Some(a), Some(b)) if a == b => {
                    match classify_same_tag(a, old_line, new_line, place) {
                        LineClass::Churn => {}
                        LineClass::Knob => self.saw_knob = true,
                        LineClass::Other => {
                            let label = other_label(a, place);
                            self.push_other(label);
                        }
                    }
                }
                _ => {
                    self.push_other(line_label(old_line));
                    self.push_other(line_label(new_line));
                }
            }
        }
    }

    /// Lines present on one side only (after alignment) — a real insertion
    /// or deletion, never churn. An added/removed automation breakpoint is
    /// labelled as such; closing-tag labels are dropped when the run also
    /// names an opening tag (they add nothing but noise).
    fn classify_one_sided(&mut self, lines: Vec<String>) {
        let any_open = lines.iter().any(|l| tag_name(l).is_some());
        for l in &lines {
            let label = match tag_name(l) {
                Some(t) if BREAKPOINT_TAGS.contains(&t) => {
                    format!("{t} (automation breakpoint added/removed)")
                }
                Some(t) => t.to_string(),
                None if any_open && closing_tag_name(l).is_some() => continue,
                None => line_label(l),
            };
            self.push_other(label);
        }
    }

    /// Classify a gap that is either already same-length or has been
    /// refined as far as this check bothers to go.
    fn classify_leaf(&mut self, o1: usize, o2: usize, n1: usize, n2: usize) {
        let (old_len, new_len) = (o2 - o1, n2 - n1);
        if old_len == new_len {
            self.classify_aligned(o1, n1, old_len);
            return;
        }
        if old_len.saturating_mul(new_len) > LCS_CELL_CAP {
            self.push_other(format!(
                "(unaligned region: {old_len} old vs {new_len} new lines)"
            ));
            return;
        }
        // Drop the lines both sides share (by LCS) before looking at what
        // changed, then walk the unmatched runs between matches.
        let matches = lcs_matches(&self.old_keys[o1..o2], &self.new_keys[n1..n2]);
        let (mut pi, mut pj) = (0usize, 0usize);
        for (i, j) in matches
            .into_iter()
            .chain(std::iter::once((old_len, new_len)))
        {
            let (dels, ins) = (i - pi, j - pj);
            if dels == ins {
                self.classify_aligned(o1 + pi, n1 + pj, dels);
            } else {
                let lines: Vec<String> = self.old[o1 + pi..o1 + i]
                    .iter()
                    .chain(self.new[n1 + pj..n1 + j].iter())
                    .cloned()
                    .collect();
                self.classify_one_sided(lines);
            }
            // A matched pair whose raw lines differ differs only in a
            // positional `Id` (that is all `align_key` masks): churn.
            pi = i + 1;
            pj = j + 1;
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
        while o1 < o2 && n1 < n2 && self.old_keys[o1] == self.new_keys[n1] {
            o1 += 1;
            n1 += 1;
        }
        while o2 > o1 && n2 > n1 && self.old_keys[o2 - 1] == self.new_keys[n2 - 1] {
            o2 -= 1;
            n2 -= 1;
        }
        if o1 == o2 && n1 == n2 {
            return;
        }

        let same_len = o2 - o1 == n2 - n1;
        let worth_refining = depth < MAX_REFINE_DEPTH && (o2 - o1 > 6 || n2 - n1 > 6);
        if !same_len && worth_refining {
            let local_anchors = unique_anchors(&self.old_keys[o1..o2], &self.new_keys[n1..n2]);
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
    let old_keys: Vec<Cow<'_, str>> = old.iter().map(|l| align_key(l)).collect();
    let new_keys: Vec<Cow<'_, str>> = new.iter().map(|l| align_key(l)).collect();
    let anchors = unique_anchors(&old_keys, &new_keys);
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
        old_keys,
        new_keys,
        old_devices_depth: depth_before(old, "Devices"),
        old_tracks_depth: depth_before(old, "Tracks"),
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
        saw_knob: classifier.saw_knob,
        other_tags: classifier.other_tags,
    }
}

fn has_fx_tilde(records: &[wit_model::ChangeRecord]) -> bool {
    records
        .iter()
        .any(|r| matches!(r, wit_model::ChangeRecord::FxSettingsChanged { .. }))
}

// --------------------------------------------------------------------- //
// Synthetic, code-built fixtures (no real files) — run in CI
// --------------------------------------------------------------------- //

/// What varies between two synthetic saves. Everything else is fixed.
#[derive(Clone, Copy)]
struct Synth {
    /// The first track's `AutoFilter` cutoff — a device parameter on a
    /// track `wit_als` walks.
    cutoff: &'static str,
    /// Added to every positional id (`AutomationTarget`, `FloatEvent`,
    /// `FileRef`), the way Live renumbers on save. The tests use +1, as
    /// Live does (EXPERIMENTS.md §1: `Id="130"` -> `"131"`), so a shifted
    /// line can equal a *different* element's unshifted line — which is
    /// why alignment masks these ids (`align_key`).
    id_shift: u32,
    /// Breakpoints in the first track's automation envelope.
    breakpoints: u32,
    /// The first track's selection/fold view state (two view-state fields
    /// with unchanged, repeated lines between them).
    selected: bool,
    /// The master track's `Limiter` on/off — a device `wit_als` never walks.
    main_device_on: &'static str,
}

const BASE: Synth = Synth {
    cutoff: "0.5",
    id_shift: 0,
    breakpoints: 3,
    selected: false,
    main_device_on: "true",
};

/// One audio track: view state, an automation envelope, a mixer, a clip
/// with a sample reference, and an `AutoFilter` with `On`, `Gain` and
/// `Cutoff` parameters. Positional ids are `id_base + s.id_shift` onwards.
fn synthetic_track(track_id: u32, name: &str, id_base: u32, s: Synth) -> String {
    let id = |offset: u32| id_base + offset + s.id_shift;
    let (cutoff, selected) = (s.cutoff, s.selected);
    let events: String = (0..s.breakpoints)
        .map(|i| {
            format!(
                "\t\t\t\t\t\t\t\t\t<FloatEvent Id=\"{}\" Time=\"{}\" Value=\"0.5\" />\n",
                id(10 + i),
                i * 4
            )
        })
        .collect();
    format!(
        r#"			<AudioTrack Id="{track_id}">
				<LomId Value="0" />
				<IsContentSelectedInDocument Value="{selected}" />
				<TrackDelay>
					<Value Value="0" />
					<IsValueSampleBased Value="false" />
				</TrackDelay>
				<TrackUnfolded Value="{selected}" />
				<Name>
					<EffectiveName Value="{name}" />
				</Name>
				<AutomationEnvelopes>
					<Envelopes>
						<AutomationEnvelope Id="0">
							<Automation>
								<Events>
{events}								</Events>
							</Automation>
						</AutomationEnvelope>
					</Envelopes>
				</AutomationEnvelopes>
				<DeviceChain>
					<Mixer>
						<LomId Value="0" />
						<Volume>
							<LomId Value="0" />
							<Manual Value="0.7943282127" />
							<AutomationTarget Id="{t0}">
								<LockEnvelope Value="0" />
							</AutomationTarget>
						</Volume>
					</Mixer>
					<MainSequencer>
						<Sample>
							<ArrangerAutomation>
								<Events>
									<AudioClip Id="{track_id}" Time="0">
										<CurrentStart Value="0" />
										<CurrentEnd Value="16" />
										<Name Value="{name} take" />
										<Disabled Value="false" />
										<SampleRef>
											<FileRef Id="{f0}">
												<RelativePathType Value="3" />
												<RelativePath Value="Samples/Recorded/{name} take.wav" />
												<Path Value="/Projects/Song Project/Samples/Recorded/{name} take.wav" />
												<OriginalFileSize Value="1048576" />
												<OriginalCrc Value="{track_id}2345" />
											</FileRef>
										</SampleRef>
									</AudioClip>
								</Events>
							</ArrangerAutomation>
						</Sample>
					</MainSequencer>
					<DeviceChain>
						<Devices>
							<AutoFilter Id="0">
								<LomId Value="0" />
								<On>
									<LomId Value="0" />
									<Manual Value="true" />
									<AutomationTarget Id="{t1}">
										<LockEnvelope Value="0" />
									</AutomationTarget>
								</On>
								<Gain>
									<LomId Value="0" />
									<Manual Value="1" />
									<AutomationTarget Id="{t2}">
										<LockEnvelope Value="0" />
									</AutomationTarget>
								</Gain>
								<Cutoff>
									<LomId Value="0" />
									<Manual Value="{cutoff}" />
									<AutomationTarget Id="{t3}">
										<LockEnvelope Value="0" />
									</AutomationTarget>
								</Cutoff>
							</AutoFilter>
						</Devices>
					</DeviceChain>
				</DeviceChain>
			</AudioTrack>
"#,
        f0 = id(0),
        t0 = id(1),
        t1 = id(2),
        t2 = id(3),
        t3 = id(4),
    )
}

/// A small Live-12-shaped set, one element per line, tab-indented the way
/// Live writes it (the classifier is line-based, so shape matters): two
/// audio tracks with identical device chains, plus a `MainTrack` with a
/// `Limiter`. Parameter names are illustrative, not a claim about either
/// device's real parameter list. Because the two tracks are siblings, their
/// device lines repeat at the same depth — as sibling tracks' lines do in a
/// real set — so they are never unique anchors: within one track, the
/// lines between the `On` and `Gain` automation-target ids (including an
/// unchanged device `Manual`) all sit in the same gap as those two ids.
fn synthetic_xml(s: Synth) -> String {
    let first = synthetic_track(8, "Bass", 100, s);
    // The second track never varies except for id renumbering.
    let second = synthetic_track(
        9,
        "Keys",
        200,
        Synth {
            id_shift: s.id_shift,
            ..BASE
        },
    );
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<Ableton MajorVersion="5" MinorVersion="12.0_12300" Creator="Ableton Live 12.3" Revision="">
	<LiveSet>
		<Tracks>
{first}{second}		</Tracks>
		<MainTrack>
			<LomId Value="0" />
			<DeviceChain>
				<Mixer>
					<LomId Value="0" />
					<Tempo>
						<LomId Value="0" />
						<Manual Value="120" />
						<AutomationTarget Id="{t0}">
							<LockEnvelope Value="0" />
						</AutomationTarget>
					</Tempo>
				</Mixer>
				<DeviceChain>
					<Devices>
						<Limiter Id="0">
							<LomId Value="0" />
							<On>
								<LomId Value="0" />
								<Manual Value="{main_on}" />
								<AutomationTarget Id="{t1}">
									<LockEnvelope Value="0" />
								</AutomationTarget>
							</On>
						</Limiter>
					</Devices>
				</DeviceChain>
			</DeviceChain>
		</MainTrack>
	</LiveSet>
</Ableton>
"#,
        t0 = 300 + s.id_shift,
        t1 = 301 + s.id_shift,
        main_on = s.main_device_on,
    )
}

fn gzip(xml: &str) -> Vec<u8> {
    use std::io::Write as _;
    let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    enc.write_all(xml.as_bytes()).unwrap();
    enc.finish().unwrap()
}

/// Classify a synthetic pair from its gzipped bytes (the same path the
/// real-corpus run takes) and diff it through the real `wit_als` +
/// `wit_diff`. Also returns how many lines of the XML differ.
fn run_synthetic(old: Synth, new: Synth) -> (RawDiff, Vec<wit_model::ChangeRecord>, usize) {
    let (old_xml, new_xml) = (synthetic_xml(old), synthetic_xml(new));
    let changed_lines = old_xml
        .lines()
        .zip(new_xml.lines())
        .filter(|(a, b)| a != b)
        .count()
        + old_xml.lines().count().abs_diff(new_xml.lines().count());
    let (old_bytes, new_bytes) = (gzip(&old_xml), gzip(&new_xml));
    let raw = classify_pair(&gunzip_lines(&old_bytes), &gunzip_lines(&new_bytes));
    let old_model = wit_als::parse(&old_bytes).expect("synthetic .als must parse");
    let new_model = wit_als::parse(&new_bytes).expect("synthetic .als must parse");
    (raw, wit_diff::diff(&old_model, &new_model), changed_lines)
}

/// The knob-only half of the check, on material where it *can* fail: two
/// saves identical except for one device `Manual` value on a regular
/// track. The raw classifier must call it knob-only, and `wit_diff` must
/// report `FX~` for it.
///
/// Mutation-checked: deleting the `FxSettingsChanged` push in
/// `wit_diff::diff` (so `FX~` is never emitted) makes this test fail. The
/// real-corpus run cannot catch that mutation — its knob-only bucket is
/// empty (see the module doc).
#[test]
fn synthetic_knob_only_pair_is_knob_only_and_reported_as_fx_tilde() {
    let turned = Synth {
        cutoff: "0.75",
        ..BASE
    };
    let (raw, records, changed_lines) = run_synthetic(BASE, turned);
    assert_eq!(
        changed_lines, 1,
        "the fixture must differ in exactly one line"
    );
    assert_eq!(
        raw.verdict,
        RawVerdict::KnobOnly,
        "other: {:?}",
        raw.other_tags
    );
    assert!(
        has_fx_tilde(&records),
        "a knob-only save produced no FX~ record: {records:?}"
    );
}

/// The zero-change half, synthetically — the shape of every real
/// zero-change save in the measured corpus: every positional id shifts by
/// +1 and some view state changes, nothing else. The two view-state lines
/// have unchanged, repeated lines between them (a `TrackDelay` block,
/// closing tag included) inside the same gap; the classifier must skip
/// those rather than count them as changes (the bug an earlier revision of
/// this file had). `wit_diff` must report no `FX~`.
///
/// Mutation-checked: making `wit_als`'s device fingerprint also hash `Id`
/// attributes makes this test (and the real-corpus run) fail; so does
/// removing the skip of identical lines in `classify_aligned`, or taking
/// `FloatEvent` out of `CHURN_ID_TAGS`.
#[test]
fn synthetic_id_renumbering_is_zero_change_and_not_fx_tilde() {
    let resaved = Synth {
        id_shift: 1,
        selected: true,
        ..BASE
    };
    let (raw, records, changed_lines) = run_synthetic(BASE, resaved);
    assert!(changed_lines > 5, "the fixture must renumber several ids");
    assert_eq!(
        raw.verdict,
        RawVerdict::ZeroChange,
        "other: {:?}",
        raw.other_tags
    );
    assert!(
        !has_fx_tilde(&records),
        "FX~ fired on pure id renumbering: {records:?}"
    );
}

/// An automation breakpoint removed is a real edit to an envelope, not
/// save-time churn — even when every other line only renumbers.
#[test]
fn synthetic_breakpoint_count_change_is_other_not_churn() {
    let fewer = Synth {
        breakpoints: 2,
        id_shift: 1,
        ..BASE
    };
    let (raw, _, _) = run_synthetic(BASE, fewer);
    assert_eq!(raw.verdict, RawVerdict::Other);
    assert_eq!(
        raw.other_tags,
        vec!["FloatEvent (automation breakpoint added/removed)".to_string()]
    );
}

/// A device parameter on `MainTrack` is not knob-only: `wit_als` does not
/// walk `MainTrack` (until the extractor work in PR #65 lands), so this is
/// the named blind spot, reported as `Other`. Asserts only the raw
/// classifier, so it keeps holding once `wit_diff` learns the master bus.
#[test]
fn synthetic_main_track_device_change_is_other_not_knob_only() {
    let toggled = Synth {
        main_device_on: "false",
        ..BASE
    };
    let (raw, _, changed_lines) = run_synthetic(BASE, toggled);
    assert_eq!(changed_lines, 1);
    assert_eq!(raw.verdict, RawVerdict::Other);
    assert_eq!(
        raw.other_tags,
        vec!["Manual (MainTrack device — wit_als never walks MainTrack)".to_string()]
    );
}

/// A sample relinked to a new folder (same basename, same CRC and size) is
/// churn; the same relink with a different CRC is a different file.
#[test]
fn same_basename_relink_is_churn_unless_the_audio_changed() {
    let old = synthetic_xml(BASE);
    let moved = old.replace("/Projects/Song Project/", "/Archive/Song Project/");
    let as_lines = |s: &str| s.lines().map(str::to_string).collect::<Vec<_>>();
    assert_eq!(
        classify_pair(&as_lines(&old), &as_lines(&moved)).verdict,
        RawVerdict::ZeroChange
    );
    let other_audio = moved.replace(
        "<OriginalCrc Value=\"82345\" />",
        "<OriginalCrc Value=\"54321\" />",
    );
    let raw = classify_pair(&as_lines(&old), &as_lines(&other_audio));
    assert_eq!(raw.verdict, RawVerdict::Other);
    assert_eq!(raw.other_tags, vec!["OriginalCrc".to_string()]);
}

// --------------------------------------------------------------------- //
// Real-corpus run (opt-in)
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
    let mut other_with_knob = Vec::new();
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
            let has_fx = has_fx_tilde(&records);

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
                            "    (finding, not a failure: wit_diff reported {} record(s) on a pair the raw classifier found no real content change in)",
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
                        "  RAW-OTHER        {label}: {} record(s), FX~ {has_fx}, device knob moved too: {}, tags: {:?}",
                        records.len(),
                        raw.saw_knob,
                        raw.other_tags
                    );
                    if raw.saw_knob {
                        other_with_knob.push((label.clone(), has_fx));
                    }
                    if records.is_empty() {
                        // Raw XML shows real content changed, and wit_diff
                        // said nothing at all — a blind spot, printed
                        // rather than hidden inside a passing assertion.
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
    if !other_with_knob.is_empty() {
        eprintln!(
            "\n{} other pair(s) also move a device parameter on a walked track, alongside \
             other changes (not asserted on — FX~ there is not the knob-only case):",
            other_with_knob.len()
        );
        for (label, has_fx) in &other_with_knob {
            eprintln!("  - {label}: FX~ {has_fx}");
        }
    }
    if knob_only_candidates == 0 {
        // Not asserted as a failure: forcing a mixed pair into KnobOnly to
        // make an assertion pass would be exactly the "loosen it to force
        // a pass" AGENTS.md warns against. Say plainly what that means.
        eprintln!(
            "\nNo raw-knob-only pair in this corpus: the knob-only half of the check is \
             untested on this material and cannot fail here. It is covered only by \
             synthetic_knob_only_pair_is_knob_only_and_reported_as_fx_tilde."
        );
    }

    assert!(
        total_pairs > 0,
        "no consecutive same-chain pair parsed cleanly under {dir:?} — nothing to check"
    );
    assert!(
        zero_change > 0,
        "expected at least one raw-zero-change pair (the false-FX~ half has nothing to check \
         otherwise) — found none under {dir:?}"
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
         subtree of a walked track), but wit_diff produced no FxSettingsChanged record for it: \
         {knob_only_missing_fx:?}"
    );
}
