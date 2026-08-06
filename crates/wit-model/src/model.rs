//! The semantic model: what a parser (`wit-als`, later `wit-logic`) extracts
//! from a project file. Mirrors `experiments/als_semantic_diff.py`'s
//! `build_model()` output field-for-field — see that module's docstring for
//! why extraction is whitelist-based, not blacklist-based.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// A project's structure at one point in time — one save, one backup, one
/// Alternative.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Model {
    /// The DAW version string that wrote this file (Ableton's `Creator`
    /// attribute), if the parser read one.
    pub creator: Option<String>,
    /// Tempo in BPM, if the parser read one. `None` means "not read", never
    /// "zero" — a missing tempo must never render as a change to 0 BPM.
    pub tempo_bpm: Option<f64>,
    /// Track id -> track. Keyed and iterated in [`TrackId`] order.
    pub tracks: BTreeMap<TrackId, Track>,
}

impl Model {
    /// Sample basename -> the set of track names whose clips reference it.
    /// Built on demand rather than stored, mirroring
    /// `als_semantic_diff.py`'s `model["samples"]` — the rename-bijection
    /// guard in `wit-diff` (M1 named bug fix 3) is the sole consumer.
    pub fn sample_index(&self) -> BTreeMap<String, BTreeSet<String>> {
        let mut index: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for track in self.tracks.values() {
            for clip in track.clips.values() {
                if !clip.sample.is_empty() {
                    index
                        .entry(clip.sample.clone())
                        .or_default()
                        .insert(track.name.clone());
                }
            }
        }
        index
    }
}

/// A track id, ordered numerically when both operands parse as integers and
/// falling back to lexicographic string order otherwise ("numeric-then-lex").
///
/// This is a **deliberate divergence** from
/// `als_semantic_diff.py`'s plain `sorted()` over string ids (pure
/// lexicographic order — where `"10"` sorts before `"2"`). Ableton track ids
/// are monotonic integer counters in practice, so numeric order is the
/// order a musician expects a track list in; kept as string ids because a
/// parser is never required to hand back a value that parses as an integer
/// (e.g. a future format's ids), and losing an unparseable id would be a
/// silent data drop the whitelist-extraction doctrine forbids.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TrackId(pub String);

impl From<&str> for TrackId {
    fn from(s: &str) -> Self {
        TrackId(s.to_string())
    }
}

impl From<String> for TrackId {
    fn from(s: String) -> Self {
        TrackId(s)
    }
}

impl fmt::Display for TrackId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl PartialOrd for TrackId {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for TrackId {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self.0.parse::<u64>(), other.0.parse::<u64>()) {
            (Ok(a), Ok(b)) => a.cmp(&b).then_with(|| self.0.cmp(&other.0)),
            _ => self.0.cmp(&other.0),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Track {
    pub id: TrackId,
    pub name: String,
    pub kind: TrackKind,
    /// The `Color` swatch index, as Ableton's raw XML string ("13"). `None`
    /// only if the parser could not find the element at all.
    pub color: Option<String>,
    /// Rounded to 3 decimal places by the parser (the noise gate — Live
    /// rewrites floats with sub-millesimal differences on every save; see
    /// [`crate::round3`]). `None` if there was no `DeviceChain`/`Mixer` to
    /// read (e.g. some Return/Group track schemas).
    pub volume: Option<f64>,
    pub pan: Option<f64>,
    /// Ableton's raw XML attribute string ("true"/"false"), not a bool — the
    /// golden output renders it verbatim (`test_als_golden.py`:
    /// "output enabled: true -> false"), so re-parsing it into a `bool` and
    /// back would be a pointless round trip that could only introduce a bug.
    pub speaker: Option<String>,
    /// Devices in chain order — order is musically meaningful (signal
    /// flow), so this is a `Vec`, not a set.
    pub devices: Vec<Device>,
    /// Clip id -> clip. Keyed and iterated in lexicographic string order,
    /// matching `sorted(set(...))` over Ableton's decimal-string clip ids in
    /// `als_semantic_diff.py` exactly (no numeric-then-lex divergence here —
    /// see [`TrackId`] for why tracks are different).
    pub clips: BTreeMap<String, Clip>,
    pub automation_lanes: usize,
    pub notes: usize,
}

impl Track {
    /// Device tags in chain order, ignoring settings fingerprints — this is
    /// the whole comparison surface `als_semantic_diff.py`'s `devices` field
    /// ever had (device *presence and order*, not parameter values).
    pub fn device_tags(&self) -> Vec<&str> {
        self.devices.iter().map(|d| d.tag.as_str()).collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackKind {
    Audio,
    Midi,
    Group,
    Return,
}

impl TrackKind {
    /// The exact Ableton XML element tag — what `TRACK+`/`TRACK-` golden
    /// lines render (`"TRACK+  added 'Vox' (AudioTrack)"`), not a friendly
    /// noun. A `render_friendly` translation is future work (PLAN.md
    /// workspace_layout: "units translation... device display names" lands
    /// alongside the app).
    pub fn xml_tag(self) -> &'static str {
        match self {
            TrackKind::Audio => "AudioTrack",
            TrackKind::Midi => "MidiTrack",
            TrackKind::Group => "GroupTrack",
            TrackKind::Return => "ReturnTrack",
        }
    }

    pub fn from_xml_tag(tag: &str) -> Option<Self> {
        match tag {
            "AudioTrack" => Some(TrackKind::Audio),
            "MidiTrack" => Some(TrackKind::Midi),
            "GroupTrack" => Some(TrackKind::Group),
            "ReturnTrack" => Some(TrackKind::Return),
            _ => None,
        }
    }
}

/// One device in a track's chain: its type, and a fingerprint of its
/// parameter values.
///
/// `als_semantic_diff.py` only ever recorded the device *tag* — a save that
/// only turns a filter cutoff knob reports "no musical change", which the
/// module's own docstring names as "the single biggest gap." `fingerprint`
/// is new in the Rust port (M1 named bug fix 3): `wit-als` hashes every
/// `<Manual Value="...">` parameter reachable under the device element, so a
/// same-tag, same-position device whose fingerprint changed becomes a
/// `FX~ … settings changed` line instead of silence. Whitelist, not
/// blacklist, per AGENTS.md: only `Manual Value` attributes feed the hash,
/// so unrelated bookkeeping churn (`LomId`, `AutomationTarget` ids, which
/// shift when upstream elements are inserted) cannot cause a false positive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    pub tag: String,
    pub fingerprint: Fingerprint,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Clip {
    pub id: String,
    pub name: String,
    /// Rounded to 3 decimal places, matching `_num()` in the Python model.
    pub start: f64,
    pub end: f64,
    /// The sample's basename only — never the full path, which differs
    /// between machines and can embed another person's home directory
    /// (`als_semantic_diff.py`: `sample.split("/")[-1]`). Empty string if
    /// the clip has no sample reference.
    pub sample: String,
    pub disabled: bool,
}

/// A byte fingerprint over a device's parameter values. Not itself a hash
/// algorithm choice worth exposing generically — `wit-als` always produces
/// this via BLAKE3 (ADR-0004) — but kept as a newtype so `Device` doesn't
/// carry a raw `[u8; 32]` with no type-level meaning.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Fingerprint(pub [u8; 32]);

impl fmt::Debug for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Fingerprint({})", hex(&self.0))
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn track_id_orders_numerically_not_lexicographically() {
        let mut ids = vec![
            TrackId::from("100"),
            TrackId::from("20"),
            TrackId::from("9"),
        ];
        ids.sort();
        assert_eq!(
            ids,
            vec![
                TrackId::from("9"),
                TrackId::from("20"),
                TrackId::from("100"),
            ]
        );
    }

    #[test]
    fn track_id_falls_back_to_lexicographic_for_non_numeric_ids() {
        let mut ids = vec![TrackId::from("b"), TrackId::from("a")];
        ids.sort();
        assert_eq!(ids, vec![TrackId::from("a"), TrackId::from("b")]);
    }

    #[test]
    fn sample_index_maps_basename_to_track_names() {
        let mut model = Model::default();
        let mut clips = BTreeMap::new();
        clips.insert(
            "9".to_string(),
            Clip {
                id: "9".to_string(),
                name: String::new(),
                start: 0.0,
                end: 4.0,
                sample: "kick.wav".to_string(),
                disabled: false,
            },
        );
        model.tracks.insert(
            TrackId::from("1"),
            Track {
                id: TrackId::from("1"),
                name: "Drums".to_string(),
                kind: TrackKind::Audio,
                color: None,
                volume: None,
                pan: None,
                speaker: None,
                devices: vec![],
                clips,
                automation_lanes: 0,
                notes: 0,
            },
        );
        let index = model.sample_index();
        assert_eq!(index["kick.wav"], BTreeSet::from(["Drums".to_string()]));
    }
}
