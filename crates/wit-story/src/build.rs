//! Saves on disk → [`Story`]. Read-only: this module opens project files
//! for reading and never writes anywhere.
//!
//! v1 builds straight from what the DAW itself keeps on disk (Logic's
//! backups, Live's `Backup/` autosaves), timed by each file's modification
//! time. Nothing here has been kept by Wit, and the wording says so: the
//! app's watcher adds "kept by Wit" moments from the store later, and the
//! Story shape doesn't change when it does.

use crate::clock::{about_duration, Clock};
use crate::sentence::{record_is_finite, sentence, SentenceContext};
use crate::types::*;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use wit_index::{AbletonLineage, LogicKind, LogicProject};
use wit_model::{BarPos, ChangeRecord, Model, TimeSignature as ModelTimeSignature};

/// Saves further apart than this start a new session (wit-planning/PLAN.md,
/// "The Story": sessions are clustered by gaps of more than 45 minutes).
pub const SESSION_GAP_SECS: i64 = 45 * 60;

/// More sentences than this and a moment also gets a one-line summary.
const SUMMARY_THRESHOLD: usize = 3;

const NEVER_CHANGED: &str = "Your project is never changed.";

/// Build the whole library under `root`: every Logic/GarageBand project and
/// Ableton lineage Wit can discover, the Shelf, and the trust panel.
///
/// `root_label` is how the owner sees the folder ("~/Music/Logic"). It goes
/// into the trust panel instead of the real path, **and it scopes every id**
/// (song, story, moment), so the same layout under two watched folders never
/// collides. The caller must therefore pass a label that is unique across
/// watched folders and never changes for a folder: changing it changes every
/// id under it.
pub fn build_library(root: &Path, root_label: &str, clock: &Clock) -> Library {
    let mut songs: Vec<(ShelfCard, Vec<Story>)> = Vec::new();

    for project in wit_index::discover_logic_projects(root) {
        let stories = logic_stories(&project, root, root_label, clock);
        if let Some(mut card) = shelf_card(&stories) {
            // Logic saves a window picture inside each alternative: free
            // artwork. The app fetches it by song id; the Story holds no path.
            if project
                .alternatives
                .iter()
                .any(|a| a.current.with_file_name("WindowImage.jpg").is_file())
            {
                card.artwork = Artwork::WindowImage;
            }
            songs.push((card, stories));
        }
    }
    for lineage in wit_index::discover_ableton_lineages(root) {
        let story = ableton_story(&lineage, root, root_label, clock);
        if let Some(card) = shelf_card(std::slice::from_ref(&story)) {
            songs.push((card, vec![story]));
        }
    }

    songs.sort_by(|(a, _), (b, _)| {
        b.last_worked
            .cmp(&a.last_worked)
            .then_with(|| a.title.cmp(&b.title))
            .then_with(|| a.song_id.cmp(&b.song_id))
    });

    let mut shelf = Vec::new();
    let mut stories = Vec::new();
    for (card, mut song_stories) in songs {
        // Stories in the same order as the card lists them.
        song_stories.sort_by_key(|s| {
            card.story_ids
                .iter()
                .position(|id| *id == s.id)
                .unwrap_or(usize::MAX)
        });
        shelf.push(card);
        stories.extend(song_stories);
    }

    Library {
        schema_version: SCHEMA_VERSION,
        shelf,
        stories,
        trust: TrustPanel {
            watched_folders: vec![root_label.to_string()],
            statements: vec![
                "Wit never changes your projects.".to_string(),
                "Nothing leaves this computer.".to_string(),
            ],
            counters: PilotCounters::default(),
        },
    }
}

// ---------------------------------------------------------------------------
// Readings: one parsed save
// ---------------------------------------------------------------------------

#[derive(Clone)]
enum Reading {
    Logic {
        extracted: wit_logic::Extracted,
        /// `MetaData.plist`, read from the save's own sibling file.
        /// `None` when that file is missing or unreadable — never guessed,
        /// and every plist-derived fact below degrades honestly when it's
        /// `None` rather than inventing one.
        metadata: Option<wit_logic::ProjectMetadata>,
        bytes: Vec<u8>,
    },
    Ableton {
        model: Model,
        bytes: Vec<u8>,
    },
    Unreadable,
}

impl Reading {
    fn bytes(&self) -> Option<&[u8]> {
        match self {
            Reading::Logic { bytes, .. } | Reading::Ableton { bytes, .. } => Some(bytes),
            Reading::Unreadable => None,
        }
    }

    fn tempo(&self) -> Option<f64> {
        let t = match self {
            Reading::Logic { extracted, .. } => extracted.tempo_bpm,
            Reading::Ableton { model, .. } => model.tempo_bpm,
            Reading::Unreadable => None,
        };
        t.filter(|x| x.is_finite())
    }

    fn logic_metadata(&self) -> Option<&wit_logic::ProjectMetadata> {
        match self {
            Reading::Logic { metadata, .. } => metadata.as_ref(),
            _ => None,
        }
    }
}

fn read_logic(path: &Path) -> Reading {
    let Ok(bytes) = std::fs::read(path) else {
        return Reading::Unreadable;
    };
    match wit_logic::walk(&bytes) {
        Ok(w) => {
            // MetaData.plist sits right beside ProjectData, in the
            // alternative and in every backup slot alike (metadata.rs).
            let metadata_path = path.with_file_name("MetaData.plist");
            let metadata = wit_logic::read_metadata_plist(&metadata_path).ok();
            Reading::Logic {
                extracted: w.extracted,
                metadata,
                bytes,
            }
        }
        Err(_) => Reading::Unreadable,
    }
}

fn read_ableton(path: &Path) -> Reading {
    let Ok(bytes) = std::fs::read(path) else {
        return Reading::Unreadable;
    };
    match wit_als::parse(&bytes) {
        Ok(model) => Reading::Ableton { model, bytes },
        Err(_) => Reading::Unreadable,
    }
}

struct Save {
    path: PathBuf,
    at: Timestamp,
    source: MomentSource,
}

fn mtime(path: &Path) -> Timestamp {
    let secs = std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    Timestamp(secs)
}

/// Path relative to the library root with `/` separators. Only ever hashed
/// into an id, never put in a Story.
fn relative_key(path: &Path, root: &Path) -> String {
    let rel = path.strip_prefix(root).unwrap_or(path);
    rel.components()
        .filter_map(|c| match c {
            std::path::Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn fnv1a64(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// An opaque song id: a DAW prefix plus a hash of where the project is,
/// so an id never carries a path or a folder name.
fn song_id(prefix: &str, location: &str) -> SongId {
    SongId(format!("{prefix}:{:016x}", fnv1a64(location)))
}

// ---------------------------------------------------------------------------
// Track lanes (heat-strip rows)
// ---------------------------------------------------------------------------

/// Heat-strip rows. Only DAWs whose parser knows real track identities get
/// rows (Ableton track ids). Logic gets none until the Logic lane pairs
/// names with tracks: a name list that mixes tracks, MIDI regions and the
/// project's own title is not a track list (wit-logic `extract.rs`).
#[derive(Clone, Default)]
struct Lanes {
    lanes: Vec<TrackLane>,
    by_key: BTreeMap<String, usize>,
    /// Name → row, or `None` when two tracks share the name (ambiguous).
    by_name: BTreeMap<String, Option<usize>>,
}

impl Lanes {
    fn ensure(&mut self, key: &str, name: &str) -> usize {
        let i = match self.by_key.get(key) {
            Some(&i) => {
                self.lanes[i].name = name.to_string();
                i
            }
            None => {
                let i = self.lanes.len();
                self.lanes.push(TrackLane {
                    key: key.to_string(),
                    name: name.to_string(),
                    color: None,
                });
                self.by_key.insert(key.to_string(), i);
                i
            }
        };
        match self.by_name.get(name) {
            Some(Some(j)) if *j != i => {
                self.by_name.insert(name.to_string(), None);
            }
            Some(_) => {}
            None => {
                self.by_name.insert(name.to_string(), Some(i));
            }
        }
        i
    }

    fn index(&self, name: &str) -> Option<usize> {
        self.by_name.get(name).copied().flatten()
    }

    /// Point each sentence at its row, when that row is unambiguous.
    fn assign(&self, sentences: &mut [Sentence]) {
        for s in sentences {
            s.lane = s
                .track
                .as_deref()
                .and_then(|t| self.index(t))
                .map(|i| i as u32);
        }
    }
}

/// Heat for one set of sentences: how many sentences touch each row,
/// capped at 3; a track added or removed is always 3.
fn heat(sentences: &[Sentence]) -> Vec<TrackHeat> {
    let mut levels: BTreeMap<u32, u8> = BTreeMap::new();
    for s in sentences {
        let Some(lane) = s.lane else { continue };
        let whole_track = matches!(s.icon, Icon::Add | Icon::Remove)
            && s.spans
                .iter()
                .find(|sp| sp.kind != SpanKind::Plain)
                .is_some_and(|sp| sp.kind == SpanKind::Track);
        let entry = levels.entry(lane).or_insert(0);
        *entry = if whole_track {
            3
        } else {
            entry.saturating_add(1).min(3)
        };
    }
    levels
        .into_iter()
        .map(|(track, level)| TrackHeat { track, level })
        .collect()
}

/// "14 changes Wit can see: 9 regions added, 3 new audio files, 2 probable
/// renames" — for busy
/// saves only. Counts extracted, named changes (sentences), never container
/// records.
fn summary(sentences: &[Sentence]) -> Option<String> {
    if sentences.len() <= SUMMARY_THRESHOLD {
        return None;
    }
    // (singular, plural) per kind of change, in first-seen order. The noun
    // comes from what the sentence is about (its first name span), so a busy
    // save reads "12 regions added", not "12 added"; an inferred sentence is
    // counted as a probable one, keeping its hedge.
    let mut groups: Vec<((&str, &str), usize)> = Vec::new();
    for s in sentences {
        let about = s
            .spans
            .iter()
            .find(|sp| sp.kind != SpanKind::Plain)
            .map(|sp| sp.kind);
        let noun = match (s.icon, about) {
            (Icon::Rename, _) if s.confidence == Confidence::Inferred => {
                ("probable rename", "probable renames")
            }
            (Icon::Add, Some(SpanKind::Region)) => ("region added", "regions added"),
            (Icon::Remove, Some(SpanKind::Region)) => ("region removed", "regions removed"),
            (Icon::Add, Some(SpanKind::Track)) => ("track added", "tracks added"),
            (Icon::Remove, Some(SpanKind::Track)) => ("track removed", "tracks removed"),
            (Icon::Add, Some(SpanKind::Name)) => {
                ("new track or MIDI region", "new tracks or MIDI regions")
            }
            (Icon::Remove, Some(SpanKind::Name)) => (
                "track or MIDI region removed",
                "tracks or MIDI regions removed",
            ),
            (Icon::Add, _) => ("addition", "additions"),
            (Icon::Remove, _) => ("removal", "removals"),
            (Icon::AudioFile, _) if s.text.starts_with("New") => {
                ("new audio file", "new audio files")
            }
            (Icon::AudioFile, _) => ("audio file removed", "audio files removed"),
            (Icon::Move, _) => ("move", "moves"),
            (Icon::Trim, _) => ("resize", "resizes"),
            (Icon::Duplicate, _) => ("duplicate", "duplicates"),
            (Icon::Rename, _) => ("rename", "renames"),
            (Icon::Record, _) => ("sample swap", "sample swaps"),
            (Icon::Mix | Icon::Mute, _) => ("mix change", "mix changes"),
            (Icon::Plugin, _) => ("plugin change", "plugin changes"),
            (Icon::Midi | Icon::Automation, _) => {
                ("MIDI or automation change", "MIDI or automation changes")
            }
            (Icon::Marker, _) => ("marker change", "marker changes"),
            (Icon::Tempo | Icon::Key | Icon::Meter | Icon::Tracks, _) => {
                ("song setting", "song settings")
            }
        };
        match groups.iter_mut().find(|(n, _)| *n == noun) {
            Some((_, c)) => *c += 1,
            None => groups.push((noun, 1)),
        }
    }
    let parts: Vec<String> = groups
        .iter()
        .map(|((one, many), n)| format!("{n} {}", if *n == 1 { one } else { many }))
        .collect();
    Some(format!(
        "{} changes Wit can see: {}",
        sentences.len(),
        parts.join(", ")
    ))
}

// ---------------------------------------------------------------------------
// Per-DAW facts
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct DawFacts {
    daw: Daw,
    label: &'static str,
    keeps: Option<u32>,
    tier: Tier,
}

impl DawFacts {
    fn logic(kind: LogicKind) -> Self {
        match kind {
            LogicKind::Logic => DawFacts {
                daw: Daw::Logic,
                label: "Logic",
                // Backups 00–09 (probe-verified, wit-planning/PROBE-FINDINGS.md).
                keeps: Some(10),
                tier: Tier::Structure,
            },
            LogicKind::GarageBand => DawFacts {
                daw: Daw::GarageBand,
                label: "GarageBand",
                // GarageBand keeps no Project File Backups (measured, ADR-0006).
                keeps: None,
                tier: Tier::Structure,
            },
        }
    }

    fn ableton() -> Self {
        DawFacts {
            daw: Daw::Ableton,
            label: "Live",
            // Live keeps 10 autosaves per set (measured on a real set's
            // Backup/ folder, PLAN-V2 "What's missing").
            keeps: Some(10),
            tier: Tier::Semantic,
        }
    }

    fn capability(&self) -> Vec<CapabilityNote> {
        let mut notes = Vec::new();
        match self.daw {
            Daw::Logic | Daw::GarageBand => {
                notes.push(CapabilityNote {
                    tier: Some(self.tier),
                    text: format!("Wit can't see knob and fader moves in {} yet.", self.label),
                });
                notes.push(CapabilityNote {
                    tier: Some(self.tier),
                    text: format!(
                        "Wit can't tell {}'s tracks apart yet, so there's no track strip for \
                         this song.",
                        self.label
                    ),
                });
            }
            Daw::Ableton => {
                notes.push(CapabilityNote {
                    tier: Some(self.tier),
                    text: "Wit can say a plugin's settings changed, but not how.".to_string(),
                });
                notes.push(CapabilityNote {
                    tier: Some(self.tier),
                    text: "Wit can't yet see edits inside MIDI clips or automation lanes — only \
                           how many notes or lanes there are."
                        .to_string(),
                });
            }
            Daw::FlStudio | Daw::Other => notes.push(CapabilityNote {
                tier: Some(Tier::History),
                text: "Wit can't read inside this project yet.".to_string(),
            }),
        }
        notes.push(CapabilityNote {
            tier: None,
            text: NEVER_CHANGED.to_string(),
        });
        notes
    }

    fn nothing_visible_note(&self) -> &'static str {
        match self.daw {
            Daw::Ableton => {
                "No musical change Wit can see — maybe a note or automation edit, a view \
                 change, or you just hit save."
            }
            _ => "No change Wit can see — maybe a knob or fader move, or you just hit save.",
        }
    }

    fn unreadable_note(&self) -> String {
        format!(
            "Wit couldn't read this save. It may come from a newer {} than Wit understands — \
             compare two bounces instead.",
            self.label
        )
    }
}

/// Only a moment Wit itself kept may say so.
fn with_kept_clause(note: &str, source: &MomentSource) -> String {
    match source {
        MomentSource::KeptByWit => format!("{note} Wit kept a copy."),
        _ => note.to_string(),
    }
}

fn source_label(source: &MomentSource) -> String {
    match source {
        MomentSource::LogicBackup { slot, .. } => format!("Logic backup {slot}"),
        MomentSource::LogicCurrent { .. } => "current save".to_string(),
        MomentSource::AbletonAutosave => "Live autosave".to_string(),
        MomentSource::AbletonSave => "saved in Live".to_string(),
        MomentSource::FlAutosave => "FL Studio autosave".to_string(),
        MomentSource::FlSave => "saved in FL Studio".to_string(),
        MomentSource::KeptByWit => "kept by Wit".to_string(),
        MomentSource::Restored { .. } => "restored copy".to_string(),
    }
}

// ---------------------------------------------------------------------------
// Comparing two readings
// ---------------------------------------------------------------------------

struct PairResult {
    verdict: Verdict,
    sentences: Vec<Sentence>,
}

/// Logic names Wit has read as renames, so the oldest-to-newest overview can
/// pair a name that was renamed partway through the range.
#[derive(Clone, Default)]
struct NameRoots {
    root: BTreeMap<String, String>,
}

impl NameRoots {
    fn of<'a>(&'a self, name: &'a str) -> &'a str {
        self.root.get(name).map(String::as_str).unwrap_or(name)
    }

    fn rename(&mut self, old: &str, new: &str) {
        let r = self.of(old).to_string();
        self.root.insert(new.to_string(), r);
    }
}

fn compare(
    a: &Reading,
    b: &Reading,
    facts: &DawFacts,
    lanes: &mut Lanes,
    roots: &mut NameRoots,
) -> PairResult {
    let mut sentences = match (a, b) {
        (_, Reading::Unreadable) => {
            return PairResult {
                verdict: Verdict::Unreadable,
                sentences: vec![],
            }
        }
        // wit-logic's own verdict also counts census changes; a census-only
        // change has no sentence Wit may show (ADR-0006's census-noun ban),
        // so here it reads as "nothing visible", which is the honest wording.
        (
            Reading::Logic {
                extracted: ea,
                metadata: ma,
                bytes: ba,
            },
            Reading::Logic {
                extracted: eb,
                metadata: mb,
                bytes: bb,
            },
        ) => logic_sentences(ea, eb, ma, mb, ba, bb, facts, roots),
        (Reading::Ableton { model: ma, .. }, Reading::Ableton { model: mb, .. }) => {
            for (id, t) in &mb.tracks {
                lanes.ensure(&id.0, &t.name);
            }
            let ctx = SentenceContext {
                tier: facts.tier,
                beats_per_bar: None,
            };
            wit_diff::diff(ma, mb)
                .iter()
                .filter(|r| record_is_finite(r))
                .map(|r| sentence(r, &ctx))
                .collect()
        }
        _ => vec![],
    };
    lanes.assign(&mut sentences);
    let verdict = if !sentences.is_empty() {
        Verdict::Changed
    } else if a.bytes().is_some() && a.bytes() == b.bytes() {
        Verdict::Identical
    } else {
        Verdict::NothingVisible
    };
    PairResult { verdict, sentences }
}

fn ordered_unique(names: &[String]) -> Vec<&String> {
    let mut seen = BTreeSet::new();
    names.iter().filter(|n| seen.insert(n.as_str())).collect()
}

/// `MetaData.plist`'s `SongKey`/`SongGenderKey`, combined the way
/// `SongHeader.key`'s doc comment shows it ("C minor"). `None` unless the
/// project states a key — never inferred from anything else.
fn logic_key_label(meta: &wit_logic::ProjectMetadata) -> Option<String> {
    match (&meta.key, &meta.mode) {
        (Some(key), Some(mode)) => Some(format!("{key} {mode}")),
        (Some(key), None) => Some(key.clone()),
        (None, _) => None,
    }
}

/// Quarter-note ticks per bar from `MetaData.plist`'s own time signature —
/// `numerator * 4 / denominator` quarter-note beats per bar, at Logic's
/// fixed 960-tick-per-quarter-note grid (`wit_logic::TICKS_PER_QUARTER`;
/// `docs/FORMATS.md`'s region-payload section). `None` when the project
/// doesn't state a (non-degenerate) time signature.
fn logic_ticks_per_bar(meta: Option<&wit_logic::ProjectMetadata>) -> Option<f64> {
    let ts = meta?.time_signature?;
    if ts.denominator == 0 {
        return None;
    }
    let beats_per_bar = ts.numerator as f64 * 4.0 / ts.denominator as f64;
    (beats_per_bar.is_finite() && beats_per_bar > 0.0)
        .then_some(beats_per_bar * wit_logic::TICKS_PER_QUARTER as f64)
}

/// A placement's raw tick position, converted to a bar using the project's
/// *actual* time signature when Wit has one — `wit_logic::regions`
/// deliberately stays at a fixed 4/4 (see that module's doc), so this
/// honest conversion is `wit-story`'s job.
///
/// **Always "about", never "exact".** Even when both saves agree on the
/// project's *current* time signature, Wit never reads Logic's tempo/meter
/// *map* — there is no way to rule out a signature change somewhere between
/// the region-time origin and this position, only to know what the project
/// states *now*. `docs/FORMATS.md`'s own "WHAT THIS DOES NOT HANDLE" note
/// on tempo/meter maps is exactly this gap. So this always marks the
/// result approximate; only the *number* gets more accurate when Wit knows
/// the real signature (falling back to `wit_logic::TICKS_PER_BAR`'s 4/4
/// assumption when it doesn't).
fn logic_bar_pos(position: u32, ticks_per_bar: Option<f64>) -> BarPos {
    let tick = position.saturating_sub(wit_logic::REGION_TIME_ORIGIN) as f64;
    let tpb = ticks_per_bar.unwrap_or(wit_logic::TICKS_PER_BAR as f64);
    BarPos::about(tick / tpb + 1.0)
}

/// Logic at the Structure tier: tempo, names in the track list, plist facts
/// (track count, key, time signature), regions, audio files. Nothing here
/// is a census count (ADR-0006).
#[allow(clippy::too_many_arguments)]
fn logic_sentences(
    a: &wit_logic::Extracted,
    b: &wit_logic::Extracted,
    meta_a: &Option<wit_logic::ProjectMetadata>,
    meta_b: &Option<wit_logic::ProjectMetadata>,
    bytes_a: &[u8],
    bytes_b: &[u8],
    facts: &DawFacts,
    roots: &mut NameRoots,
) -> Vec<Sentence> {
    let ctx = SentenceContext {
        tier: facts.tier,
        beats_per_bar: None,
    };
    let mut out = Vec::new();

    if let (Some(from_bpm), Some(to_bpm)) = (a.tempo_bpm, b.tempo_bpm) {
        let r = ChangeRecord::TempoChanged { from_bpm, to_bpm };
        if from_bpm != to_bpm && record_is_finite(&r) {
            out.push(sentence(&r, &ctx));
        }
    }

    // Track-list names. wit-logic can't yet pair `qeSM` names with tracks
    // (karT pairing is the Logic lane's job), so a name may be a track or a
    // MIDI region, and the sentence says so.
    let names_a: BTreeSet<&str> = a.possible_track_names.iter().map(String::as_str).collect();
    let names_b: BTreeSet<&str> = b.possible_track_names.iter().map(String::as_str).collect();
    let mut removed: Vec<&String> = ordered_unique(&a.possible_track_names)
        .into_iter()
        .filter(|n| !names_b.contains(n.as_str()))
        .collect();
    let mut added: Vec<&String> = ordered_unique(&b.possible_track_names)
        .into_iter()
        .filter(|n| !names_a.contains(n.as_str()))
        .collect();
    // Names already read as one rename chain (a rename inferred between two
    // saves in the middle of this range) pair up again.
    let mut paired = Vec::new();
    removed.retain(|old| {
        let root = roots.of(old).to_string();
        match added.iter().position(|new| roots.of(new) == root) {
            Some(j) => {
                paired.push((*old, added.remove(j)));
                false
            }
            None => true,
        }
    });
    for (old, new) in paired {
        out.push(crate::sentence::logic_name_renamed(old, new, facts.tier));
    }
    if removed.len() == 1 && added.len() == 1 {
        let (old, new) = (removed[0], added[0]);
        roots.rename(old, new);
        out.push(crate::sentence::logic_name_renamed(old, new, facts.tier));
    } else {
        for name in added {
            out.push(crate::sentence::logic_name_added(name, facts.tier));
        }
        for name in removed {
            out.push(crate::sentence::logic_name_removed(name, facts.tier));
        }
    }

    // Plist-derived facts: the DAW's own stated counts/settings, never a
    // container tally (ADR-0006's census-noun ban).
    if let (Some(ma), Some(mb)) = (meta_a, meta_b) {
        if let (Some(from), Some(to)) = (ma.number_of_tracks, mb.number_of_tracks) {
            if from != to {
                out.push(sentence(
                    &ChangeRecord::TrackCountChanged {
                        from: from as usize,
                        to: to as usize,
                    },
                    &ctx,
                ));
            }
        }
        // A save missing only `SongGenderKey` (mode) while the key letter
        // itself is unchanged must not read as a key change: comparing the
        // *labels* ("C" vs "C major") would report one purely because one
        // side didn't state a mode, not because anything really changed.
        // Only report when the key itself differs, or when both sides
        // state a mode and those modes differ.
        let key_changed = if ma.key != mb.key {
            true
        } else {
            matches!((&ma.mode, &mb.mode), (Some(x), Some(y)) if x != y)
        };
        if key_changed {
            out.push(sentence(
                &ChangeRecord::KeyChanged {
                    from: logic_key_label(ma),
                    to: logic_key_label(mb),
                },
                &ctx,
            ));
        }
        if let (Some(ts_a), Some(ts_b)) = (ma.time_signature, mb.time_signature) {
            if (ts_a.numerator, ts_a.denominator) != (ts_b.numerator, ts_b.denominator) {
                out.push(sentence(
                    &ChangeRecord::TimeSignatureChanged {
                        from: ModelTimeSignature {
                            numerator: ts_a.numerator,
                            denominator: ts_a.denominator,
                        },
                        to: ModelTimeSignature {
                            numerator: ts_b.numerator,
                            denominator: ts_b.denominator,
                        },
                    },
                    &ctx,
                ));
            }
        }
    }

    // Regions on the timeline: the placement-level diff from `regions.rs`,
    // exact wherever it decodes cleanly on both sides. Bars use each side's
    // *own* time signature (the old save's for a removed/from position,
    // the new save's for an added/to position — a project that changed
    // meter between the two must not have its old positions read through
    // the new meter) when Wit has one; every position is still marked
    // "about" regardless (see `logic_bar_pos`). A family holding more than
    // one region object is named the way the frozen script's own
    // `family_subject` does ("a 'stem' region"), never a copy count, and a
    // family with no region object at all (measured on GarageBand, which
    // writes no `gRuA` records) is dropped rather than shown as Wit's own
    // `"<unresolved family N>"` placeholder — see `wit_logic::regions`'
    // `RegionSubject` doc. When either side fails to parse (a shape this
    // port doesn't handle), Wit says nothing about regions for this pair
    // rather than falling back to a guess.
    let ticks_per_bar_a = logic_ticks_per_bar(meta_a.as_ref());
    let ticks_per_bar_b = logic_ticks_per_bar(meta_b.as_ref());
    if let (Ok(song_a), Ok(song_b)) = (
        wit_logic::parse_regions_bytes(bytes_a),
        wit_logic::parse_regions_bytes(bytes_b),
    ) {
        for change in wit_logic::diff_placements(&song_a, &song_b) {
            match change {
                wit_logic::PlacementChange::Moved { subject, from, to } => {
                    if !subject.resolved {
                        continue;
                    }
                    let from_bar = logic_bar_pos(from.1, ticks_per_bar_a);
                    let to_bar = logic_bar_pos(to.1, ticks_per_bar_b);
                    if from.0 == to.0 {
                        out.push(crate::sentence::logic_region_moved(
                            &subject.stem,
                            subject.ambiguous,
                            to.0,
                            from_bar,
                            to_bar,
                            facts.tier,
                        ));
                    } else {
                        // `RegionMoved` has one `track` field — it cannot
                        // honestly say "moved from track 3 to track 5" (the
                        // frozen script's own cross-track wording names
                        // both). Reported as a removal from the old track
                        // plus an addition on the new one instead of
                        // silently dropping the old track, matching the
                        // subject already resolved for the move.
                        out.push(crate::sentence::logic_region_removed(
                            &subject.stem,
                            subject.ambiguous,
                            from.0,
                            from_bar,
                            facts.tier,
                        ));
                        out.push(crate::sentence::logic_region_added(
                            &subject.stem,
                            subject.ambiguous,
                            to.0,
                            to_bar,
                            facts.tier,
                        ));
                    }
                }
                wit_logic::PlacementChange::Added {
                    subject,
                    track,
                    position,
                } => {
                    if !subject.resolved {
                        continue;
                    }
                    out.push(crate::sentence::logic_region_added(
                        &subject.stem,
                        subject.ambiguous,
                        track,
                        logic_bar_pos(position, ticks_per_bar_b),
                        facts.tier,
                    ));
                }
                wit_logic::PlacementChange::Removed {
                    subject,
                    track,
                    position,
                } => {
                    if !subject.resolved {
                        continue;
                    }
                    out.push(crate::sentence::logic_region_removed(
                        &subject.stem,
                        subject.ambiguous,
                        track,
                        logic_bar_pos(position, ticks_per_bar_a),
                        facts.tier,
                    ));
                }
            }
        }
    }

    let fa: BTreeSet<&str> = a.audio_file_names.iter().map(String::as_str).collect();
    let fb: BTreeSet<&str> = b.audio_file_names.iter().map(String::as_str).collect();
    for name in ordered_unique(&b.audio_file_names) {
        if !fa.contains(name.as_str()) {
            out.push(sentence(
                &ChangeRecord::AudioFileAdded { name: name.clone() },
                &ctx,
            ));
        }
    }
    for name in ordered_unique(&a.audio_file_names) {
        if !fb.contains(name.as_str()) {
            out.push(sentence(
                &ChangeRecord::AudioFileRemoved { name: name.clone() },
                &ctx,
            ));
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Assembling a Story
// ---------------------------------------------------------------------------

struct StoryMeta {
    song_id: SongId,
    story_id: StoryId,
    title: String,
    lineage: Option<String>,
    facts: DawFacts,
    family: Option<Family>,
    /// A capability note when `Resources/ProjectInformation.plist` names a
    /// Logic newer than `wit_logic::KNOWN_MAX_MAJOR_VERSION` — `None` for
    /// every other DAW, and for Logic whenever Wit can't read that plist or
    /// the project isn't newer.
    newer_daw_warning: Option<String>,
}

/// Moment ids come from the save's own time, so they don't shift when the
/// DAW recycles an old backup. Two saves in the same second get `~1`, `~2`.
fn moment_ids(story_id: &StoryId, saves: &[Save]) -> Vec<MomentId> {
    let mut seen: BTreeMap<i64, usize> = BTreeMap::new();
    saves
        .iter()
        .map(|s| {
            let n = seen.entry(s.at.0).or_insert(0);
            let id = if *n == 0 {
                format!("{story_id}@{}", s.at.0)
            } else {
                format!("{story_id}@{}~{n}", s.at.0)
            };
            *n += 1;
            MomentId(id)
        })
        .collect()
}

fn assemble(meta: StoryMeta, saves: Vec<Save>, read: fn(&Path) -> Reading, clock: &Clock) -> Story {
    let facts = meta.facts;
    let ids = moment_ids(&meta.story_id, &saves);
    let mut lanes = Lanes::default();
    let mut roots = NameRoots::default();
    let mut moments: Vec<Moment> = Vec::new();
    // The last reading Wit could read, and the moment it belongs to.
    let mut prev: Option<(Reading, usize)> = None;
    let mut first: Option<(Reading, usize)> = None;
    let mut last_tempo = None;
    let mut last_key: Option<String> = None;
    let mut last_time_sig: Option<ModelTimeSignature> = None;

    for (i, save) in saves.iter().enumerate() {
        let reading = read(&save.path);
        // Direct assignment, never `.or(last_key)`: once this save's
        // `MetaData.plist` has actually been read, `None` here means the
        // project states no key *now* — a real "cleared" fact the header
        // must show, not something to paper over with a stale earlier
        // value while the timeline itself says "Cleared the key". Only
        // skip the update entirely (falling through to whatever the last
        // Logic-metadata-bearing save left behind) when this save has no
        // metadata to read at all — found in review.
        if let Some(meta) = reading.logic_metadata() {
            last_key = logic_key_label(meta);
            last_time_sig = meta.time_signature.map(|ts| ModelTimeSignature {
                numerator: ts.numerator,
                denominator: ts.denominator,
            });
        }
        if first.is_none() {
            if let Reading::Ableton { model, .. } = &reading {
                for (id, t) in &model.tracks {
                    lanes.ensure(&id.0, &t.name);
                }
            }
        }
        last_tempo = reading.tempo().or(last_tempo);
        let label = clock.moment_label(save.at);

        let (verdict, sentences, compared_with) = match (&prev, &reading) {
            (_, Reading::Unreadable) => (Verdict::Unreadable, vec![], None),
            (None, _) => (Verdict::First, vec![], None),
            (Some((p, pi)), r) => {
                let res = compare(p, r, &facts, &mut lanes, &mut roots);
                (res.verdict, res.sentences, Some(*pi))
            }
        };

        let prev_label = compared_with.map(|pi| moments[pi].label.clone());
        let (heading, subheading, note) = match verdict {
            Verdict::First => (
                "The oldest moment".to_string(),
                format!("{label} · nothing earlier to compare with"),
                Some(if i == 0 {
                    "The oldest save Wit has for this song.".to_string()
                } else {
                    "The oldest save Wit can read for this song.".to_string()
                }),
            ),
            Verdict::Changed => (
                format!("What changed at {label}"),
                format!(
                    "compared with the save at {}",
                    prev_label.clone().unwrap_or_default()
                ),
                None,
            ),
            Verdict::NothingVisible => (
                "Routine save".to_string(),
                format!(
                    "{label} · compared with the save at {}",
                    prev_label.clone().unwrap_or_default()
                ),
                Some(with_kept_clause(facts.nothing_visible_note(), &save.source)),
            ),
            Verdict::Identical => (
                "Saved again, nothing changed".to_string(),
                format!(
                    "{label} · compared with the save at {}",
                    prev_label.clone().unwrap_or_default()
                ),
                Some(with_kept_clause(
                    "Saved again with nothing changed.",
                    &save.source,
                )),
            ),
            Verdict::Unreadable => (
                "Wit couldn't read this save".to_string(),
                label.clone(),
                Some(facts.unreadable_note()),
            ),
        };

        let listen: Vec<ListenRef> = vec![];
        moments.push(Moment {
            id: ids[i].clone(),
            at: save.at,
            label,
            heading,
            subheading,
            compared_with: compared_with.map(|pi| ids[pi].clone()),
            source_label: source_label(&save.source),
            source: save.source.clone(),
            verdict,
            weight: sentences.len() as u32,
            summary: summary(&sentences),
            track_heat: heat(&sentences),
            sentences,
            note,
            actions: Actions {
                listen: !listen.is_empty(),
                // ADR-0007: off until a restored copy is shown to open in
                // this DAW.
                open_as_copy: false,
                // The share page (M6) doesn't exist yet.
                send: false,
            },
            listen,
        });

        if !matches!(reading, Reading::Unreadable) {
            if first.is_none() {
                first = Some((reading.clone(), i));
            }
            prev = Some((reading, i));
        }
    }

    // The overview: oldest readable moment → newest readable moment.
    let overview = match (&first, &prev) {
        (Some((a, fi)), Some((b, li))) if fi < li => {
            let mut scratch_lanes = lanes.clone();
            let mut scratch_roots = roots.clone();
            let r = compare(a, b, &facts, &mut scratch_lanes, &mut scratch_roots);
            let from = &moments[*fi];
            let to = &moments[*li];
            let note = match r.verdict {
                Verdict::Identical => {
                    Some("Nothing changed between the oldest and newest moment.".to_string())
                }
                Verdict::NothingVisible => {
                    Some("No changes Wit can see between the oldest and newest moment.".to_string())
                }
                _ => None,
            };
            Some(Comparison {
                from: from.id.clone(),
                to: to.id.clone(),
                heading: format!("Since {}", clock.day_label(from.at)),
                subheading: "from the oldest moment Wit has to the newest".to_string(),
                verdict: r.verdict,
                summary: summary(&r.sentences),
                track_heat: heat(&r.sentences),
                sentences: r.sentences,
                note,
                listen: vec![],
            })
        }
        _ => None,
    };

    let sessions = group_sessions(moments, clock);
    let last_worked = sessions.last().map(|s| s.ended);
    let all: Vec<&Moment> = sessions.iter().flat_map(|s| s.moments.iter()).collect();
    let n_moments = all.len() as u32;
    let kept_by_wit = all
        .iter()
        .filter(|m| m.source == MomentSource::KeptByWit)
        .count() as u32;

    let mut subtitle = vec![facts.label.to_string()];
    if let Some(bpm) = last_tempo {
        subtitle.push(format!("{} BPM", crate::sentence::friendly_num(bpm)));
    }
    if let Some(t) = last_worked {
        subtitle.push(format!("last worked {}", clock.moment_label(t)));
    }

    let kept_label = kept_label(n_moments, kept_by_wit, facts.label, facts.keeps, 1);

    let family_label = meta.family.as_ref().map(|f| {
        let all_alternatives = f
            .members
            .iter()
            .all(|m| matches!(m.relation, Relation::Alternative));
        if all_alternatives {
            format!("{} alternatives of this song", f.members.len())
        } else {
            format!("{} copies of this song", f.members.len())
        }
    });

    let mut capability = facts.capability();
    if let Some(note) = meta.newer_daw_warning {
        capability.push(CapabilityNote {
            tier: Some(facts.tier),
            text: note,
        });
    }

    Story {
        id: meta.story_id,
        song_id: meta.song_id,
        header: SongHeader {
            title: meta.title,
            lineage: meta.lineage,
            daw: facts.daw,
            daw_label: facts.label.to_string(),
            tempo_bpm: last_tempo,
            key: last_key,
            time_signature: last_time_sig.map(|ts| ts.to_string()),
            last_worked,
            subtitle: subtitle.join(" · "),
            kept: KeptSummary {
                moments: n_moments,
                daw_keeps: facts.keeps,
                kept_by_wit,
                label: kept_label,
            },
            family_label,
        },
        tracks: lanes.lanes,
        sessions,
        overview,
        capability,
        family: meta.family,
        send_ready: None,
    }
}

/// "41 moments kept · Logic keeps 10 backups" once Wit has kept some; "11
/// moments on disk · Logic keeps 10 backups" while every moment is still the
/// DAW's own (the current save plus its backups, so the count can exceed the
/// backups). With more than one line (Logic alternatives) the DAW clause says
/// "per alternative", so "30 on disk" never reads as if someone else kept 20.
/// The one wording for both the Story header and the Shelf card.
fn kept_label(
    moments: u32,
    kept_by_wit: u32,
    daw_label: &str,
    daw_keeps: Option<u32>,
    lines: usize,
) -> String {
    let word = if moments == 1 { "moment" } else { "moments" };
    let per = if lines > 1 { " per alternative" } else { "" };
    let keeps = match daw_keeps {
        Some(k) => format!("{daw_label} keeps {k} backups{per}"),
        None => format!("{daw_label} keeps no backups"),
    };
    if kept_by_wit > 0 {
        format!("{moments} {word} kept · {keeps}")
    } else {
        format!("{moments} {word} on disk · {keeps}")
    }
}

fn group_sessions(moments: Vec<Moment>, clock: &Clock) -> Vec<Session> {
    let mut groups: Vec<Vec<Moment>> = Vec::new();
    for m in moments {
        let new_session = match groups.last().and_then(|g| g.last()) {
            Some(last) => m.at.0 - last.at.0 > SESSION_GAP_SECS,
            None => true,
        };
        if new_session {
            groups.push(vec![m]);
        } else if let Some(g) = groups.last_mut() {
            g.push(m);
        }
    }
    let mut sessions: Vec<Session> = groups
        .into_iter()
        .enumerate()
        .map(|(i, moments)| {
            let started = moments[0].at;
            let ended = moments[moments.len() - 1].at;
            let span = (ended.0 - started.0).max(0) as u64;
            let about = about_duration(span);
            let duration = about.as_ref().map(|_| SessionDuration {
                secs: span,
                source: DurationSource::SaveTimes,
            });
            let mut label = format!(
                "{} · {}",
                clock.day_label(started),
                clock.part_of_day(started)
            );
            if let Some(a) = about {
                label.push_str(" · ");
                label.push_str(&a);
            }
            Session {
                id: format!("s{i}"),
                started,
                ended,
                duration,
                label,
                moments,
            }
        })
        .collect();
    // Two sessions on the same day and part of day both get their start
    // times, so the timeline never shows two sessions it can't tell apart.
    let day_part = |s: &Session| {
        format!(
            "{} · {}",
            clock.day_label(s.started),
            clock.part_of_day(s.started)
        )
    };
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    for s in &sessions {
        *seen.entry(day_part(s)).or_insert(0) += 1;
    }
    for s in &mut sessions {
        if seen[&day_part(s)] > 1 {
            s.label = format!("{} · from {}", s.label, clock.time_label(s.started));
        }
    }
    sessions
}

/// One Story per Logic alternative.
pub fn logic_stories(
    project: &LogicProject,
    root: &Path,
    root_label: &str,
    clock: &Clock,
) -> Vec<Story> {
    let facts = DawFacts::logic(project.kind);
    let prefix = match project.kind {
        LogicKind::Logic => "logic",
        LogicKind::GarageBand => "garageband",
    };
    let song_id = song_id(
        prefix,
        &format!("{root_label}/{}", relative_key(&project.bundle_path, root)),
    );
    let many = project.alternatives.len() > 1;
    let newer_daw_warning = newer_logic_warning(project);

    // Logic doesn't record which alternative another was made from, so
    // they are siblings, not a tree.
    let family = many.then(|| Family {
        members: project
            .alternatives
            .iter()
            .enumerate()
            .map(|(i, alt)| FamilyMember {
                song_id: song_id.clone(),
                story_id: Some(StoryId(format!("{song_id}#{}", alt.name))),
                title: format!("{} · Alternative {}", project.name, i + 1),
                relation: Relation::Alternative,
                parent: None,
                last_worked: Some(mtime(&alt.current)),
                last_worked_label: Some(clock.moment_label(mtime(&alt.current))),
                is_current: false,
            })
            .collect(),
        evidence: vec![FamilyEvidence::SameProject],
    });

    project
        .alternatives
        .iter()
        .enumerate()
        .map(|(i, alt)| {
            let story_id = StoryId(format!("{song_id}#{}", alt.name));
            let mut saves: Vec<Save> = alt
                .backups
                .iter()
                .map(|p| Save {
                    at: mtime(p),
                    source: MomentSource::LogicBackup {
                        alternative: alt.name.clone(),
                        slot: p
                            .parent()
                            .and_then(|s| s.file_name())
                            .map(|s| s.to_string_lossy().into_owned())
                            .unwrap_or_default(),
                    },
                    path: p.clone(),
                })
                .collect();
            saves.push(Save {
                at: mtime(&alt.current),
                source: MomentSource::LogicCurrent {
                    alternative: alt.name.clone(),
                },
                path: alt.current.clone(),
            });
            // Logic's backup slots are a ring: slot order is not save order
            // once it wraps. Time is.
            saves.sort_by_key(|s| s.at);

            let family = family.clone().map(|mut f| {
                for m in &mut f.members {
                    m.is_current = m.story_id.as_ref() == Some(&story_id);
                }
                f
            });
            assemble(
                StoryMeta {
                    song_id: song_id.clone(),
                    story_id,
                    title: project.name.clone(),
                    lineage: many.then(|| format!("Alternative {}", i + 1)),
                    facts,
                    family,
                    newer_daw_warning: newer_daw_warning.clone(),
                },
                saves,
                read_logic,
                clock,
            )
        })
        .collect()
}

/// A capability note when the bundle's `Resources/ProjectInformation.plist`
/// names a Logic build newer than `wit_logic::KNOWN_MAX_MAJOR_VERSION` — the
/// one early-warning signal Wit has that a save may use a shape nothing
/// here has been verified against. `None` when that plist is missing,
/// unreadable, doesn't name a version, or names one Wit already knows.
fn newer_logic_warning(project: &LogicProject) -> Option<String> {
    let info_path = project
        .bundle_path
        .join("Resources/ProjectInformation.plist");
    let info = wit_logic::read_project_information(&info_path).ok()?;
    let last_saved_from = info.last_saved_from?;
    wit_logic::is_newer_than_known(&last_saved_from).then(|| {
        "This project was last saved from a newer Logic than Wit has verified — some facts \
         here may be incomplete."
            .to_string()
    })
}

/// One Story for an Ableton lineage (a set and its `Backup/` autosaves).
pub fn ableton_story(
    lineage: &AbletonLineage,
    root: &Path,
    root_label: &str,
    clock: &Clock,
) -> Story {
    // The set's own folder: a save in `Backup/` belongs to the folder above
    // it. Otherwise the id would change the first time Live writes an
    // autosave, because `Backup/…` can sort before the set itself.
    let dir = lineage
        .saves
        .iter()
        .filter_map(|p| p.parent())
        .map(|d| match d.file_name() {
            Some(n) if n == "Backup" => d.parent().unwrap_or(d),
            _ => d,
        })
        .map(|d| relative_key(d, root))
        .min()
        .unwrap_or_default();
    let song_id = song_id("ableton", &format!("{root_label}/{dir}/{}", lineage.name));
    let mut saves: Vec<Save> = lineage
        .saves
        .iter()
        .map(|p| {
            let in_backup = p
                .parent()
                .and_then(|d| d.file_name())
                .is_some_and(|n| n == "Backup");
            Save {
                at: mtime(p),
                source: if in_backup {
                    MomentSource::AbletonAutosave
                } else {
                    MomentSource::AbletonSave
                },
                path: p.clone(),
            }
        })
        .collect();
    saves.sort_by_key(|s| s.at);
    assemble(
        StoryMeta {
            story_id: StoryId(song_id.0.clone()),
            song_id,
            title: lineage.name.clone(),
            lineage: None,
            facts: DawFacts::ableton(),
            family: None,
            newer_daw_warning: None,
        },
        saves,
        read_ableton,
        clock,
    )
}

fn shelf_card(stories: &[Story]) -> Option<ShelfCard> {
    let newest = stories.iter().max_by_key(|s| s.header.last_worked)?;
    // Every line of a card is one project, so one DAW; the labels below rely
    // on it.
    debug_assert!(stories.iter().all(|s| s.header.daw == newest.header.daw));
    let moments_kept = stories.iter().map(|s| s.header.kept.moments).sum();
    let copies = newest
        .family
        .as_ref()
        .map(|f| f.members.len().saturating_sub(1) as u32)
        .unwrap_or(0);
    let digest = match newest.sessions.last() {
        Some(session) if newest.header.kept.moments > 1 => {
            let visible: u32 = session.moments.iter().map(|m| m.weight).sum();
            match visible {
                0 => "No changes Wit can see in your last session".to_string(),
                1 => "1 change Wit can see in your last session".to_string(),
                n => format!("{n} changes Wit can see in your last session"),
            }
        }
        _ => "1 moment so far".to_string(),
    };
    let mut story_ids: Vec<(Option<Timestamp>, StoryId)> = stories
        .iter()
        .map(|s| (s.header.last_worked, s.id.clone()))
        .collect();
    story_ids.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    let seed = u32::try_from(fnv1a64(&newest.song_id.0) & 0xffff_ffff).unwrap_or(0);
    Some(ShelfCard {
        song_id: newest.song_id.clone(),
        title: newest.header.title.clone(),
        daw: newest.header.daw,
        last_worked: newest.header.last_worked,
        last_worked_label: newest
            .sessions
            .last()
            .and_then(|s| s.moments.last())
            .map(|m| m.label.clone()),
        moments_kept,
        moments_label: kept_label(
            moments_kept,
            stories.iter().map(|s| s.header.kept.kept_by_wit).sum(),
            &newest.header.daw_label,
            newest.header.kept.daw_keeps,
            stories.len(),
        ),
        daw_label: newest.header.daw_label.clone(),
        copies,
        copies_label: newest.header.family_label.as_ref().map(|_| {
            let all_alternatives = newest.family.as_ref().is_some_and(|f| {
                f.members
                    .iter()
                    .all(|m| matches!(m.relation, Relation::Alternative))
            });
            match (copies, all_alternatives) {
                (1, true) => "+1 alternative".to_string(),
                (n, true) => format!("+{n} alternatives"),
                (1, false) => "+1 copy".to_string(),
                (n, false) => format!("+{n} copies"),
            }
        }),
        artwork: Artwork::Generated { seed },
        digest,
        story_ids: story_ids.into_iter().map(|(_, id)| id).collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::kept_label;

    #[test]
    fn kept_label_credits_whoever_kept_the_saves() {
        assert_eq!(
            kept_label(11, 0, "Logic", Some(10), 1),
            "11 moments on disk · Logic keeps 10 backups"
        );
        assert_eq!(
            kept_label(30, 0, "Logic", Some(10), 3),
            "30 moments on disk · Logic keeps 10 backups per alternative"
        );
        assert_eq!(
            kept_label(41, 30, "Logic", Some(10), 1),
            "41 moments kept · Logic keeps 10 backups"
        );
        assert_eq!(
            kept_label(1, 0, "GarageBand", None, 1),
            "1 moment on disk · GarageBand keeps no backups"
        );
        assert_eq!(
            kept_label(3, 2, "GarageBand", None, 1),
            "3 moments kept · GarageBand keeps no backups"
        );
    }
}

#[cfg(test)]
mod logic_tests {
    use super::*;

    // A minimal, valid, empty `ProjectData` — root header only, no
    // records. Enough for `wit_logic::parse_regions_bytes` to succeed with
    // an empty `Song`, so the region-diff block below contributes nothing
    // and a test can isolate the plist-derived-facts sentences it cares
    // about.
    fn minimal_project_data() -> Vec<u8> {
        let mut out = vec![0u8; 0x18];
        out[0..4].copy_from_slice(&[0x23, 0x47, 0xC0, 0xAB]);
        out[4..6].copy_from_slice(&[0xd0, 0x09]);
        out
    }

    fn meta(
        tracks: u32,
        key: Option<&str>,
        mode: Option<&str>,
        numerator: u16,
        denominator: u16,
    ) -> wit_logic::ProjectMetadata {
        wit_logic::ProjectMetadata {
            number_of_tracks: Some(tracks),
            key: key.map(String::from),
            mode: mode.map(String::from),
            time_signature: Some(wit_logic::MetadataTimeSignature {
                numerator,
                denominator,
            }),
            sample_rate: Some(48_000),
            bpm: Some(120.0),
            audio_files: vec![],
        }
    }

    fn logic_facts() -> DawFacts {
        DawFacts::logic(wit_index::LogicKind::Logic)
    }

    fn sentences_between(
        meta_a: wit_logic::ProjectMetadata,
        meta_b: wit_logic::ProjectMetadata,
        bytes_a: &[u8],
        bytes_b: &[u8],
    ) -> Vec<Sentence> {
        logic_sentences(
            &wit_logic::Extracted::default(),
            &wit_logic::Extracted::default(),
            &Some(meta_a),
            &Some(meta_b),
            bytes_a,
            bytes_b,
            &logic_facts(),
            &mut NameRoots::default(),
        )
    }

    #[test]
    fn a_track_count_change_is_reported() {
        let empty = minimal_project_data();
        let sentences = sentences_between(
            meta(2, Some("C"), Some("major"), 4, 4),
            meta(3, Some("C"), Some("major"), 4, 4),
            &empty,
            &empty,
        );
        assert!(
            sentences.iter().any(|s| s.text == "Track count 2 → 3"),
            "{sentences:?}"
        );
    }

    #[test]
    fn a_key_and_mode_change_is_reported() {
        let empty = minimal_project_data();
        let sentences = sentences_between(
            meta(2, Some("C"), Some("major"), 4, 4),
            meta(2, Some("C"), Some("minor"), 4, 4),
            &empty,
            &empty,
        );
        assert!(
            sentences.iter().any(|s| s.text == "Key C major → C minor"),
            "{sentences:?}"
        );
    }

    /// Found in review: comparing the combined "C"/"C major" labels
    /// directly reported a change whenever one side simply didn't state a
    /// mode, even though the key itself never changed.
    #[test]
    fn a_missing_mode_alone_is_not_reported_as_a_key_change() {
        let empty = minimal_project_data();
        let sentences = sentences_between(
            meta(2, Some("C"), None, 4, 4),
            meta(2, Some("C"), Some("major"), 4, 4),
            &empty,
            &empty,
        );
        assert!(
            !sentences.iter().any(|s| s.icon == Icon::Key),
            "{sentences:?}"
        );
    }

    #[test]
    fn a_key_actually_clearing_is_still_reported_even_with_no_mode_on_either_side() {
        let empty = minimal_project_data();
        let sentences = sentences_between(
            meta(2, Some("C"), None, 4, 4),
            meta(2, None, None, 4, 4),
            &empty,
            &empty,
        );
        assert!(
            sentences.iter().any(|s| s.icon == Icon::Key),
            "{sentences:?}"
        );
    }

    #[test]
    fn a_time_signature_change_is_reported() {
        let empty = minimal_project_data();
        let sentences = sentences_between(
            meta(2, Some("C"), Some("major"), 4, 4),
            meta(2, Some("C"), Some("major"), 3, 4),
            &empty,
            &empty,
        );
        assert!(
            sentences
                .iter()
                .any(|s| s.text == "Time signature 4/4 → 3/4"),
            "{sentences:?}"
        );
    }

    #[test]
    fn logic_bar_positions_are_always_about_never_exact() {
        let known = logic_bar_pos(wit_logic::REGION_TIME_ORIGIN, Some(3840.0));
        assert!(known.approximate, "a known time signature must still hedge");
        let unknown = logic_bar_pos(wit_logic::REGION_TIME_ORIGIN, None);
        assert!(unknown.approximate);
    }

    // ---------------------------------------------------------------- //
    // region placement bytes — a minimal local builder, since
    // `wit-demo`'s own generator always places on track 1 and the
    // cross-track test below specifically needs two different tracks.
    // ---------------------------------------------------------------- //

    fn record_with_idx(tag: &[u8; 4], idx: u32, payload: &[u8]) -> Vec<u8> {
        let mut out = vec![0u8; 0x24];
        out[0..4].copy_from_slice(tag);
        out[0x08..0x0C].copy_from_slice(&idx.to_le_bytes());
        out[0x1C..0x20].copy_from_slice(&(payload.len() as u32).to_le_bytes());
        out.extend_from_slice(payload);
        out
    }

    fn grua_payload(name: &str, length_frames: u32, uuid: [u8; 16]) -> Vec<u8> {
        const NAME_LEN_OFF: usize = 0x4A;
        const NAME_OFF: usize = 0x4C;
        const LENGTH_OFF: usize = 0x16;
        const UUID_OFF_IN_SUFFIX: usize = 0x56;
        const FIXED_SUFFIX_LEN: usize = 133;
        let raw = name.as_bytes();
        let padded = raw.len() + (raw.len() & 1);
        let name_end = NAME_OFF + padded;
        let uuid_at = name_end + UUID_OFF_IN_SUFFIX;
        let mut p = vec![0u8; name_end + FIXED_SUFFIX_LEN];
        p[LENGTH_OFF..LENGTH_OFF + 4].copy_from_slice(&length_frames.to_le_bytes());
        p[NAME_LEN_OFF..NAME_LEN_OFF + 2].copy_from_slice(&(raw.len() as u16).to_le_bytes());
        p[NAME_OFF..NAME_OFF + raw.len()].copy_from_slice(raw);
        p[uuid_at..uuid_at + 16].copy_from_slice(&uuid);
        p
    }

    fn placement_group(track: u8, position: u32, family: u32, event_id: u32) -> [u8; 48] {
        let mut g = [0u8; 48];
        g[0..4].copy_from_slice(&[0x24, 0, 0, 0]);
        g[0x04..0x08].copy_from_slice(&position.to_le_bytes());
        g[0x10..0x14].copy_from_slice(&event_id.to_le_bytes());
        g[0x14] = track;
        g[0x10 + 7] = 0x89;
        g[0x20 + 7] = 0xBC;
        g[0x2C..0x30].copy_from_slice(&(family * 4).to_le_bytes());
        g
    }

    fn project_data_with_one_region(name: &str, family: u32, track: u8, position: u32) -> Vec<u8> {
        let mut body = record_with_idx(
            b"gRuA",
            family << 18,
            &grua_payload(name, 100_000, [1u8; 16]),
        );
        body.extend_from_slice(&record_with_idx(
            b"qSvE",
            0,
            &placement_group(track, position, family, 1),
        ));
        let mut out = vec![0u8; 0x18];
        out[0..4].copy_from_slice(&[0x23, 0x47, 0xC0, 0xAB]);
        out[4..6].copy_from_slice(&[0xd0, 0x09]);
        out[0x10..0x14].copy_from_slice(&(body.len() as u32).to_le_bytes());
        out.extend_from_slice(&body);
        out
    }

    /// Found in review: `RegionMoved` has one `track` field, so rendering a
    /// cross-track move through it silently dropped the *old* track,
    /// reading as a same-track, zero-distance move ("moved on track 5, 0
    /// bars later"). It must instead read as a removal from the old track
    /// and an addition on the new one.
    #[test]
    fn a_cross_track_move_is_a_removal_and_an_addition_not_a_no_op() {
        let a = project_data_with_one_region("Kick", 2, 3, wit_logic::REGION_TIME_ORIGIN);
        let b = project_data_with_one_region(
            "Kick",
            2,
            5,
            wit_logic::REGION_TIME_ORIGIN + wit_logic::TICKS_PER_BAR,
        );
        let sentences = sentences_between(
            meta(2, Some("C"), Some("major"), 4, 4),
            meta(2, Some("C"), Some("major"), 4, 4),
            &a,
            &b,
        );
        assert!(
            !sentences.iter().any(|s| s.icon == Icon::Move),
            "a cross-track move must never render as Moved: {sentences:?}"
        );
        let removed = sentences.iter().any(|s| {
            s.icon == Icon::Remove && s.text.contains("Kick") && s.text.contains("track 3")
        });
        let added = sentences
            .iter()
            .any(|s| s.icon == Icon::Add && s.text.contains("Kick") && s.text.contains("track 5"));
        assert!(removed, "expected a removal from track 3: {sentences:?}");
        assert!(added, "expected an addition on track 5: {sentences:?}");
    }

    #[test]
    fn a_same_track_move_is_still_reported_as_moved() {
        let a = project_data_with_one_region("Kick", 2, 3, wit_logic::REGION_TIME_ORIGIN);
        let b = project_data_with_one_region(
            "Kick",
            2,
            3,
            wit_logic::REGION_TIME_ORIGIN + wit_logic::TICKS_PER_BAR,
        );
        let sentences = sentences_between(
            meta(2, Some("C"), Some("major"), 4, 4),
            meta(2, Some("C"), Some("major"), 4, 4),
            &a,
            &b,
        );
        assert!(
            sentences.iter().any(|s| s.icon == Icon::Move
                && s.text.contains("Kick")
                && s.text.contains("track 3")),
            "{sentences:?}"
        );
    }

    // ---------------------------------------------------------------- //
    // the newer-Logic-than-verified capability note
    // ---------------------------------------------------------------- //

    fn write_project_information(bundle: &Path, last_saved_from: &str) {
        let resources = bundle.join("Resources");
        std::fs::create_dir_all(&resources).unwrap();
        let mut dict = plist::Dictionary::new();
        dict.insert(
            "LastSavedFrom".into(),
            plist::Value::String(last_saved_from.to_string()),
        );
        plist::Value::Dictionary(dict)
            .to_writer_binary(
                std::fs::File::create(resources.join("ProjectInformation.plist")).unwrap(),
            )
            .unwrap();
    }

    fn bare_logic_project(bundle_path: PathBuf) -> LogicProject {
        LogicProject {
            name: "Song".to_string(),
            bundle_path,
            kind: wit_index::LogicKind::Logic,
            alternatives: vec![],
        }
    }

    #[test]
    fn a_newer_logic_version_gets_a_capability_note() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = dir.path().join("Song.logicx");
        write_project_information(&bundle, "Logic Pro Creator Studio 13.0 (7000)");
        let project = bare_logic_project(bundle);
        assert!(newer_logic_warning(&project).is_some());
    }

    #[test]
    fn a_known_logic_version_gets_no_note() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = dir.path().join("Song.logicx");
        write_project_information(&bundle, "Logic Pro Creator Studio 12.2 (6644)");
        let project = bare_logic_project(bundle);
        assert!(newer_logic_warning(&project).is_none());
    }

    #[test]
    fn a_missing_project_information_plist_gets_no_note_not_a_panic() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = dir.path().join("Song.logicx");
        std::fs::create_dir_all(&bundle).unwrap();
        let project = bare_logic_project(bundle);
        assert!(newer_logic_warning(&project).is_none());
    }
}
