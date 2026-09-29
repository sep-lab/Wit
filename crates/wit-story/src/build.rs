//! Kept saves on disk → [`Story`]. Read-only: this module opens project
//! files for reading and never writes anywhere.
//!
//! v1 builds straight from what the DAW keeps on disk (Logic's backups, Live's
//! `Backup/` autosaves), timed by each file's modification time. The app's
//! watcher adds "kept by Wit" moments from the store later; the Story shape
//! doesn't change when it does.

use crate::clock::{about_duration, Clock};
use crate::sentence::{sentence, SentenceContext};
use crate::types::*;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use wit_index::{AbletonLineage, LogicKind, LogicProject};
use wit_model::{ChangeRecord, Model};

/// Saves further apart than this start a new session (wit-planning/PLAN.md,
/// "The Story": sessions are clustered by gaps of more than 45 minutes).
pub const SESSION_GAP_SECS: i64 = 45 * 60;

const NEVER_CHANGED: &str = "Your project is never changed.";

/// Build the whole library under `root`: every Logic/GarageBand project and
/// Ableton lineage Wit can discover, the Shelf, and the trust panel.
/// `root_label` is how the owner sees the folder ("~/Music/Logic"); it goes
/// into the trust panel instead of the real path.
pub fn build_library(root: &Path, root_label: &str, clock: Clock) -> Library {
    let mut songs: Vec<(ShelfCard, Vec<Story>)> = Vec::new();

    for project in wit_index::discover_logic_projects(root) {
        let stories = logic_stories(&project, root, clock);
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
        let story = ableton_story(&lineage, root, clock);
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
    for (card, song_stories) in songs {
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

enum Reading {
    Logic {
        extracted: wit_logic::Extracted,
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
}

fn read_logic(path: &Path) -> Reading {
    let Ok(bytes) = std::fs::read(path) else {
        return Reading::Unreadable;
    };
    match wit_logic::walk(&bytes) {
        Ok(w) => Reading::Logic {
            extracted: w.extracted,
            bytes,
        },
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

/// Path relative to the library root with `/` separators — the basis of
/// ids. Never absolute, so ids never carry a home directory.
fn relative_id(path: &Path, root: &Path) -> String {
    let rel = path.strip_prefix(root).unwrap_or(path);
    let parts: Vec<String> = rel
        .components()
        .filter_map(|c| match c {
            std::path::Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    if parts.is_empty() {
        path.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    } else {
        parts.join("/")
    }
}

fn fnv1a(s: &str) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for b in s.bytes() {
        h ^= u32::from(b);
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

// ---------------------------------------------------------------------------
// Track lanes (heat-strip rows)
// ---------------------------------------------------------------------------

#[derive(Clone, Default)]
struct Lanes {
    lanes: Vec<TrackLane>,
    by_name: BTreeMap<String, usize>,
    by_key: BTreeMap<String, usize>,
}

impl Lanes {
    fn ensure(&mut self, key: &str, name: &str) -> usize {
        if let Some(&i) = self.by_key.get(key) {
            if self.lanes[i].name != name {
                self.lanes[i].name = name.to_string();
            }
            self.by_name.insert(name.to_string(), i);
            return i;
        }
        let i = self.lanes.len();
        self.lanes.push(TrackLane {
            key: key.to_string(),
            name: name.to_string(),
            color: None,
        });
        self.by_key.insert(key.to_string(), i);
        self.by_name.insert(name.to_string(), i);
        i
    }

    /// A rename: the new name joins the old name's lane.
    fn alias(&mut self, old: &str, new: &str) {
        match self.by_name.get(old).copied() {
            Some(i) => {
                self.lanes[i].name = new.to_string();
                self.by_name.insert(new.to_string(), i);
            }
            None => {
                self.ensure(new, new);
            }
        }
    }

    fn index(&self, name: &str) -> Option<usize> {
        self.by_name.get(name).copied()
    }
}

/// Heat for one set of sentences: how many sentences touch each track,
/// capped at 3; a track added or removed is always 3.
fn heat(sentences: &[Sentence], lanes: &Lanes) -> Vec<TrackHeat> {
    let mut levels: BTreeMap<usize, u8> = BTreeMap::new();
    for s in sentences {
        let Some(track) = &s.track else { continue };
        let Some(i) = lanes.index(track) else {
            continue;
        };
        let whole_track = matches!(s.icon, Icon::Add | Icon::Remove)
            && s.spans
                .iter()
                .find(|sp| sp.kind != SpanKind::Plain)
                .is_some_and(|sp| sp.kind == SpanKind::Track);
        let entry = levels.entry(i).or_insert(0);
        *entry = if whole_track {
            3
        } else {
            entry.saturating_add(1).min(3)
        };
    }
    levels
        .into_iter()
        .map(|(track, level)| TrackHeat {
            track: track as u32,
            level,
        })
        .collect()
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
        let first = match self.daw {
            Daw::Logic | Daw::GarageBand => {
                format!("Wit can't see knob and fader moves in {} yet.", self.label)
            }
            Daw::Ableton => "Wit can say a plugin's settings changed, but not how.".to_string(),
            Daw::FlStudio | Daw::Other => {
                "Wit keeps every save, but can't read inside this project yet.".to_string()
            }
        };
        vec![
            CapabilityNote {
                tier: Some(self.tier),
                text: first,
            },
            CapabilityNote {
                tier: None,
                text: NEVER_CHANGED.to_string(),
            },
        ]
    }

    fn nothing_visible_note(&self) -> String {
        match self.daw {
            Daw::Ableton => "No musical change Wit can see — maybe a view change, or you just \
                             hit save. Wit kept it anyway."
                .to_string(),
            _ => "No change Wit can see — maybe a knob or fader move, or you just hit save. \
                  Wit kept it anyway."
                .to_string(),
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
    note: Option<String>,
}

fn compare(a: &Reading, b: &Reading, facts: &DawFacts, lanes: &mut Lanes) -> PairResult {
    let sentences = match (a, b) {
        (_, Reading::Unreadable) => {
            return PairResult {
                verdict: Verdict::Unreadable,
                sentences: vec![],
                note: Some(facts.unreadable_note()),
            }
        }
        // wit-logic's own verdict also counts census changes; a census-only
        // change has no sentence Wit may show (ADR-0006's census-noun ban),
        // so here it reads as "nothing visible", which is the honest wording.
        (Reading::Logic { extracted: ea, .. }, Reading::Logic { extracted: eb, .. }) => {
            logic_sentences(ea, eb, facts, lanes)
        }
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
                .map(|r| sentence(r, &ctx))
                .collect()
        }
        _ => vec![],
    };
    if !sentences.is_empty() {
        return PairResult {
            verdict: Verdict::Changed,
            sentences,
            note: None,
        };
    }
    if a.bytes().is_some() && a.bytes() == b.bytes() {
        PairResult {
            verdict: Verdict::Identical,
            sentences,
            note: Some("Saved again with nothing changed. Wit kept it anyway.".to_string()),
        }
    } else {
        PairResult {
            verdict: Verdict::NothingVisible,
            sentences,
            note: Some(facts.nothing_visible_note()),
        }
    }
}

fn ordered_unique(names: &[String]) -> Vec<&String> {
    let mut seen = BTreeSet::new();
    names.iter().filter(|n| seen.insert(n.as_str())).collect()
}

fn counts(names: &[String]) -> BTreeMap<&str, usize> {
    let mut m = BTreeMap::new();
    for n in names {
        *m.entry(n.as_str()).or_insert(0) += 1;
    }
    m
}

/// Logic at the Structure tier: tempo, names in the track list, regions,
/// audio files. Nothing here is a census count (ADR-0006).
fn logic_sentences(
    a: &wit_logic::Extracted,
    b: &wit_logic::Extracted,
    facts: &DawFacts,
    lanes: &mut Lanes,
) -> Vec<Sentence> {
    let ctx = SentenceContext {
        tier: facts.tier,
        beats_per_bar: None,
    };
    let mut out = Vec::new();

    if let (Some(from_bpm), Some(to_bpm)) = (a.tempo_bpm, b.tempo_bpm) {
        if from_bpm != to_bpm {
            out.push(sentence(
                &ChangeRecord::TempoChanged { from_bpm, to_bpm },
                &ctx,
            ));
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
    // Names already known to share a heat-strip row (a rename inferred
    // between two saves in the middle of this range) are that same rename.
    let mut paired = Vec::new();
    removed.retain(|old| {
        let lane = lanes.index(old);
        match added
            .iter()
            .position(|new| lane.is_some() && lanes.index(new) == lane)
        {
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
        lanes.alias(old, new);
        out.push(crate::sentence::logic_name_renamed(old, new, facts.tier));
    } else {
        for name in added {
            lanes.ensure(name, name);
            out.push(crate::sentence::logic_name_added(name, facts.tier));
        }
        for name in removed {
            out.push(crate::sentence::logic_name_removed(name, facts.tier));
        }
    }

    // Regions: a name that appears more often now was added (at least
    // once); less often, removed. Which copy is not knowable from names.
    let ra = counts(&a.region_names);
    let rb = counts(&b.region_names);
    for name in ordered_unique(&b.region_names) {
        if rb[name.as_str()] > ra.get(name.as_str()).copied().unwrap_or(0) {
            out.push(sentence(
                &ChangeRecord::RegionAdded {
                    track: None,
                    name: name.clone(),
                    at: None,
                },
                &ctx,
            ));
        }
    }
    for name in ordered_unique(&a.region_names) {
        if ra[name.as_str()] > rb.get(name.as_str()).copied().unwrap_or(0) {
            out.push(sentence(
                &ChangeRecord::RegionRemoved {
                    track: None,
                    name: name.clone(),
                    at: None,
                },
                &ctx,
            ));
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
}

fn assemble(meta: StoryMeta, saves: Vec<Save>, read: fn(&Path) -> Reading, clock: Clock) -> Story {
    let facts = meta.facts;
    let mut lanes = Lanes::default();
    let mut moments: Vec<Moment> = Vec::new();
    let mut prev: Option<Reading> = None;
    let mut first_readable: Option<Reading> = None;
    let mut last_readable_idx: Option<usize> = None;
    let mut first_readable_idx: Option<usize> = None;
    let mut last_tempo = None;

    for (i, save) in saves.iter().enumerate() {
        let reading = read(&save.path);
        if let Reading::Logic { extracted, .. } = &reading {
            if first_readable.is_none() {
                for n in ordered_unique(&extracted.possible_track_names) {
                    lanes.ensure(n, n);
                }
            }
            last_tempo = extracted.tempo_bpm.or(last_tempo);
        }
        if let Reading::Ableton { model, .. } = &reading {
            if first_readable.is_none() {
                for (id, t) in &model.tracks {
                    lanes.ensure(&id.0, &t.name);
                }
            }
            last_tempo = model.tempo_bpm.or(last_tempo);
        }

        let result = match (&prev, &reading) {
            (_, Reading::Unreadable) => PairResult {
                verdict: Verdict::Unreadable,
                sentences: vec![],
                note: Some(facts.unreadable_note()),
            },
            (None, _) => PairResult {
                verdict: Verdict::First,
                sentences: vec![],
                note: Some(if i == 0 {
                    "The oldest save Wit has for this song.".to_string()
                } else {
                    "The oldest save Wit can read for this song.".to_string()
                }),
            },
            (Some(p), r) => compare(p, r, &facts, &mut lanes),
        };

        let track_heat = heat(&result.sentences, &lanes);
        moments.push(Moment {
            id: MomentId(format!("{}@{i}", meta.story_id)),
            at: save.at,
            label: clock.moment_label(save.at),
            source_label: source_label(&save.source),
            source: save.source.clone(),
            verdict: result.verdict,
            weight: result.sentences.len() as u32,
            sentences: result.sentences,
            note: result.note,
            track_heat,
            listen: vec![],
        });

        if !matches!(reading, Reading::Unreadable) {
            if first_readable_idx.is_none() {
                first_readable_idx = Some(i);
            }
            last_readable_idx = Some(i);
            if first_readable.is_none() {
                first_readable = Some(match &reading {
                    Reading::Logic { extracted, bytes } => Reading::Logic {
                        extracted: extracted.clone(),
                        bytes: bytes.clone(),
                    },
                    Reading::Ableton { model, bytes } => Reading::Ableton {
                        model: model.clone(),
                        bytes: bytes.clone(),
                    },
                    Reading::Unreadable => Reading::Unreadable,
                });
            }
            prev = Some(reading);
        }
    }

    // The overview: oldest readable moment → newest readable moment.
    let overview = match (
        first_readable_idx,
        last_readable_idx,
        &first_readable,
        &prev,
    ) {
        (Some(f), Some(l), Some(a), Some(b)) if f < l => {
            let mut scratch = lanes.clone();
            let r = compare(a, b, &facts, &mut scratch);
            let from = &moments[f];
            let to = &moments[l];
            Some(Comparison {
                from: from.id.clone(),
                to: to.id.clone(),
                heading: format!("Since {}", clock.day_label(from.at)),
                subheading: "from the oldest moment Wit has to the newest".to_string(),
                verdict: r.verdict,
                track_heat: heat(&r.sentences, &scratch),
                sentences: r.sentences,
                note: r.note,
                listen: vec![],
            })
        }
        _ => None,
    };

    let sessions = group_sessions(moments, clock);
    let last_worked = sessions.last().map(|s| s.ended);
    let n_moments: u32 = sessions.iter().map(|s| s.moments.len() as u32).sum();

    let mut subtitle = vec![facts.label.to_string()];
    if let Some(bpm) = last_tempo {
        subtitle.push(format!("{} BPM", crate::sentence::friendly_num(bpm)));
    }
    if let Some(t) = last_worked {
        subtitle.push(format!("last worked {}", clock.moment_label(t)));
    }

    let moments_word = if n_moments == 1 { "moment" } else { "moments" };
    let kept_label = match facts.keeps {
        Some(k) => format!(
            "{n_moments} {moments_word} kept · {} keeps {k}",
            facts.label
        ),
        None => format!("{n_moments} {moments_word} kept"),
    };

    Story {
        id: meta.story_id,
        song_id: meta.song_id,
        header: SongHeader {
            title: meta.title,
            lineage: meta.lineage,
            daw: facts.daw,
            daw_label: facts.label.to_string(),
            tempo_bpm: last_tempo,
            key: None,
            time_signature: None,
            last_worked,
            subtitle: subtitle.join(" · "),
            kept: KeptSummary {
                moments: n_moments,
                daw_keeps: facts.keeps,
                label: kept_label,
            },
        },
        tracks: lanes.lanes,
        sessions,
        overview,
        capability: facts.capability(),
        family: meta.family,
        send_ready: None,
    }
}

fn group_sessions(moments: Vec<Moment>, clock: Clock) -> Vec<Session> {
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
    groups
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
        .collect()
}

/// One Story per Logic alternative.
pub fn logic_stories(project: &LogicProject, root: &Path, clock: Clock) -> Vec<Story> {
    let facts = DawFacts::logic(project.kind);
    let prefix = match project.kind {
        LogicKind::Logic => "logic",
        LogicKind::GarageBand => "garageband",
    };
    let song_id = SongId(format!(
        "{prefix}:{}",
        relative_id(&project.bundle_path, root)
    ));
    let many = project.alternatives.len() > 1;

    let family = many.then(|| Family {
        members: project
            .alternatives
            .iter()
            .enumerate()
            .map(|(i, alt)| FamilyMember {
                song_id: song_id.clone(),
                story_id: Some(StoryId(format!("{song_id}#{}", alt.name))),
                title: format!("{} · Alternative {}", project.name, i + 1),
                relation: if i == 0 {
                    Relation::Original
                } else {
                    Relation::Alternative
                },
                parent: if i == 0 { None } else { Some(0) },
                last_worked: Some(mtime(&alt.current)),
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
            // Real save order is by time; slot names are only a tiebreak.
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
                },
                saves,
                read_logic,
                clock,
            )
        })
        .collect()
}

/// One Story for an Ableton lineage (a set and its `Backup/` autosaves).
pub fn ableton_story(lineage: &AbletonLineage, root: &Path, clock: Clock) -> Story {
    let dir = lineage
        .saves
        .first()
        .and_then(|p| p.parent())
        .map(|p| relative_id(p, root))
        .unwrap_or_default();
    let song_id = SongId(format!("ableton:{dir}/{}", lineage.name));
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
        },
        saves,
        read_ableton,
        clock,
    )
}

fn shelf_card(stories: &[Story]) -> Option<ShelfCard> {
    let newest = stories.iter().max_by_key(|s| s.header.last_worked)?;
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
        _ => "1 moment kept so far".to_string(),
    };
    let mut story_ids: Vec<(Option<Timestamp>, StoryId)> = stories
        .iter()
        .map(|s| (s.header.last_worked, s.id.clone()))
        .collect();
    story_ids.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
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
        copies,
        artwork: Artwork::Generated {
            seed: fnv1a(&newest.song_id.0),
        },
        digest,
        story_ids: story_ids.into_iter().map(|(_, id)| id).collect(),
    })
}
