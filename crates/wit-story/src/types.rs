//! The Story contract: every type the app, the CLI (`wit story --json`) and
//! the share page read. The JSON schema in `schema/library.schema.json` is
//! generated from these types and pinned by a test, so a change here is a
//! reviewable contract change, never a silent one.
//!
//! Three rules shape these types:
//!
//! - **No paths.** A Story may be pasted into a share page or a pilot
//!   report, so it carries file *names* only. The app resolves a
//!   [`SongId`] to a location through its own IPC, never through a Story.
//! - **Text is rendered here, once.** Every musician-facing string (session
//!   labels, sentences, capability notes) is produced by this crate, so the
//!   vocabulary rules are enforced in one place. Structured fields sit next
//!   to the text so a UI can style it (bold a track name, grey a routine
//!   save) without parsing it.
//! - **Honesty is in the type.** A sentence says how sure Wit is
//!   ([`Confidence`]) and which honesty tier produced it ([`Tier`]); a
//!   moment says whether Wit saw anything at all ([`Verdict`]).
//!
//! Timestamps are Unix seconds (UTC). Labels are rendered with the viewer's
//! UTC offset, which the caller passes in.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Bumped on any breaking change to these types.
pub const SCHEMA_VERSION: u32 = 1;

macro_rules! id_type {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema)]
        #[serde(transparent)]
        pub struct $name(pub String);

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

id_type!(
    /// One song on the Shelf: a project, with every lineage it has.
    SongId
);
id_type!(
    /// One line of saves: a Logic alternative, an Ableton lineage, an FL
    /// project with its autosaves.
    StoryId
);
id_type!(
    /// One kept save inside a Story.
    MomentId
);

/// Seconds since the Unix epoch, UTC.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(transparent)]
pub struct Timestamp(pub i64);

/// Everything the app's first screens need, in one document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Library {
    pub schema_version: u32,
    /// One card per song, most recently worked first.
    pub shelf: Vec<ShelfCard>,
    pub stories: Vec<Story>,
    pub trust: TrustPanel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Daw {
    Logic,
    GarageBand,
    Ableton,
    FlStudio,
    /// The generic History tier: every save kept and restorable, no
    /// "what changed" sentences.
    Other,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ShelfCard {
    pub song_id: SongId,
    pub title: String,
    pub daw: Daw,
    pub last_worked: Option<Timestamp>,
    /// "Thu 23:05", or "21 Sep" when older than a week.
    pub last_worked_label: Option<String>,
    pub moments_kept: u32,
    /// Other members of this song's family (copies, alternatives,
    /// restores) — the "+2 copies" badge. 0 = no badge.
    pub copies: u32,
    pub artwork: Artwork,
    /// The tray's one line for this song. Worded "no changes Wit can see",
    /// never "nothing new".
    pub digest: String,
    /// The Story for each lineage, newest-worked first. Always non-empty.
    pub story_ids: Vec<StoryId>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Artwork {
    /// Logic's own `WindowImage.jpg`. The app fetches it by [`SongId`].
    WindowImage,
    /// No artwork on disk: draw one from this seed, so a song always gets
    /// the same picture.
    Generated { seed: u32 },
}

/// One line of saves, read as a story.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Story {
    pub id: StoryId,
    pub song_id: SongId,
    pub header: SongHeader,
    /// Heat-strip rows, in the DAW's track order where known.
    pub tracks: Vec<TrackLane>,
    /// Oldest first. Saves more than 45 minutes apart start a new session.
    pub sessions: Vec<Session>,
    /// The first compare the Song view shows: oldest kept moment against
    /// the newest. Never an adjacent pair (a quarter of real adjacent saves
    /// show nothing — design-review-1). `None` with fewer than two moments.
    pub overview: Option<Comparison>,
    /// What Wit can and can't see for this DAW, in plain words. Shown under
    /// every change card.
    pub capability: Vec<CapabilityNote>,
    pub family: Option<Family>,
    pub send_ready: Option<SendReady>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SongHeader {
    pub title: String,
    /// Which line of the song this is, when it has more than one
    /// ("Alternative 2"). `None` for a single-lineage song.
    pub lineage: Option<String>,
    pub daw: Daw,
    /// "Logic", "GarageBand", "Live", "FL Studio".
    pub daw_label: String,
    pub tempo_bpm: Option<f64>,
    /// "C minor". Only when the project states it.
    pub key: Option<String>,
    /// "4/4". Only when the project states it.
    pub time_signature: Option<String>,
    pub last_worked: Option<Timestamp>,
    /// "Logic · 98 BPM · C minor · last worked Thu 23:05".
    pub subtitle: String,
    pub kept: KeptSummary,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct KeptSummary {
    pub moments: u32,
    /// How many saves the DAW itself keeps (Logic keeps 10 backups, Live 10
    /// autosaves per set). `None` when unknown.
    pub daw_keeps: Option<u32>,
    /// "41 moments kept · Logic keeps 10".
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TrackLane {
    /// Stable within a Story: the DAW's track id where it has one,
    /// otherwise the name.
    pub key: String,
    pub name: String,
    /// "#RRGGBB" in the DAW's own track colour, when Wit can read it.
    pub color: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Session {
    pub id: String,
    pub started: Timestamp,
    pub ended: Timestamp,
    pub duration: Option<SessionDuration>,
    /// "Mon · night · about 1h 40m".
    pub label: String,
    /// Oldest first.
    pub moments: Vec<Moment>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SessionDuration {
    pub secs: u64,
    pub source: DurationSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DurationSource {
    /// First to last save. Always shown as "about": the session started
    /// before the first save.
    SaveTimes,
    /// The DAW's process was seen open and quit.
    DawRunning,
}

/// One kept save.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Moment {
    pub id: MomentId,
    pub at: Timestamp,
    /// "Thu 23:05".
    pub label: String,
    pub source: MomentSource,
    /// "Logic backup 04", "current save", "kept by Wit".
    pub source_label: String,
    pub verdict: Verdict,
    /// How much changed, for the timeline tick's height: the number of
    /// sentences. 0 for a routine save.
    pub weight: u32,
    /// Compared with the moment before it in this Story.
    pub sentences: Vec<Sentence>,
    /// The line shown when there are no sentences ("No change Wit can see.
    /// Probably a mix move, or you just hit save. Wit kept it anyway.").
    pub note: Option<String>,
    /// Sparse heat-strip column for this moment.
    pub track_heat: Vec<TrackHeat>,
    /// Source audio behind the new parts, for "Listen to the new parts".
    pub listen: Vec<ListenRef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MomentSource {
    /// `Alternatives/<alternative>/Project File Backups/<slot>/`.
    LogicBackup {
        alternative: String,
        slot: String,
    },
    /// `Alternatives/<alternative>/ProjectData` — the save on disk now.
    LogicCurrent {
        alternative: String,
    },
    /// A Live autosave in the set's `Backup/` folder.
    AbletonAutosave,
    /// A Live set saved by hand.
    AbletonSave,
    /// An FL autosave in `Backup/`.
    FlAutosave,
    FlSave,
    /// A save the DAW has since recycled; Wit kept a copy first.
    KeptByWit,
    /// A copy Wit wrote into the Restores folder (ADR-0007).
    Restored {
        from: MomentId,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// The oldest moment: nothing to compare with.
    First,
    /// Wit can name at least one change.
    Changed,
    /// The file changed, but nothing Wit can read did: a routine save, or
    /// a knob or fader move Wit can't see yet.
    NothingVisible,
    /// Byte-for-byte the same file as the moment before.
    Identical,
    /// Wit couldn't read this save (for example a newer DAW version).
    Unreadable,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TrackHeat {
    /// Index into [`Story::tracks`].
    pub track: u32,
    /// 1 = a little, 2 = more, 3 = a lot (or the track was added/removed).
    pub level: u8,
}

/// One plain sentence about one change.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Sentence {
    pub icon: Icon,
    /// The whole sentence as plain text (copy-as-text, the CLI, screen
    /// readers). Always equal to the concatenation of `spans`.
    pub text: String,
    /// The same text, split so a UI can style names without parsing.
    pub spans: Vec<Span>,
    /// The track this sentence is about, when known.
    pub track: Option<String>,
    pub place: Option<Place>,
    /// "bars 17–32 · Verse 2", "whole song", "about bar 9".
    pub place_label: Option<String>,
    pub confidence: Confidence,
    pub tier: Tier,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Span {
    pub kind: SpanKind,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SpanKind {
    /// Wit's own words.
    Plain,
    /// A name the musician chose (track, region, file, marker…). Render as
    /// text, never as markup: it can contain anything.
    Track,
    Region,
    File,
    Plugin,
    Marker,
    /// A number with its unit ("124 BPM", "−6.0 dB").
    Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Icon {
    Add,
    Remove,
    Move,
    Trim,
    Duplicate,
    Rename,
    Record,
    Tempo,
    Key,
    Meter,
    Mute,
    Mix,
    Plugin,
    Marker,
    Automation,
    Midi,
    Tracks,
    AudioFile,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Place {
    WholeSong,
    /// 1-based bars. `end` is inclusive.
    Bars {
        start: f64,
        end: Option<f64>,
        /// "about bar N": Wit assumed a tempo or meter to get here.
        approximate: bool,
        /// The marker or locator the start falls in ("Chorus 2").
        section: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    /// Read directly from the file through a verified mapping.
    Exact,
    /// Read from the file, then converted with an assumption (a bar
    /// position under a tempo change).
    Approximate,
    /// Wit's best reading of indirect evidence (one name gone and one new
    /// name in the same save, read as a rename).
    Inferred,
}

/// The honesty tiers (wit-planning/PLAN.md "Honesty tiers").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Tier {
    /// Logic/GarageBand: names, regions, files, tempo. No knobs or faders.
    Structure,
    /// Ableton (and FL ≤ v24): the full musical vocabulary.
    Semantic,
    /// Any DAW, from two bounces.
    Ears,
    /// Any DAW: every save kept and restorable, no sentences.
    History,
}

/// A compare between any two moments of one Story.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Comparison {
    pub from: MomentId,
    pub to: MomentId,
    /// "Since Mon 21 Sep" / "What changed at Thu 23:05".
    pub heading: String,
    /// "compared with the save before" / "oldest kept moment to newest".
    pub subheading: String,
    pub verdict: Verdict,
    pub sentences: Vec<Sentence>,
    pub note: Option<String>,
    pub track_heat: Vec<TrackHeat>,
    pub listen: Vec<ListenRef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CapabilityNote {
    pub tier: Option<Tier>,
    pub text: String,
}

/// A song's copies and lines, drawn as branches.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Family {
    pub members: Vec<FamilyMember>,
    /// Why Wit thinks these belong together.
    pub evidence: Vec<FamilyEvidence>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FamilyMember {
    pub song_id: SongId,
    pub story_id: Option<StoryId>,
    pub title: String,
    pub relation: Relation,
    /// Index into [`Family::members`] of the member this one came from.
    pub parent: Option<u32>,
    pub last_worked: Option<Timestamp>,
    /// This member is the Story being viewed.
    pub is_current: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Relation {
    Original,
    /// A Logic alternative inside the same project.
    Alternative,
    /// A separate project saved from this one with Save As.
    SaveAsCopy,
    /// A copy Wit restored (ADR-0007).
    Restored {
        from: MomentId,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FamilyEvidence {
    /// Lines inside one project (Logic alternatives): certain.
    SameProject,
    /// Byte-identical audio files in both.
    SharedAudio {
        files: u32,
    },
    /// The same region identities (Logic region UUIDs survive Save As).
    SharedRegions {
        regions: u32,
    },
    SimilarName,
}

/// "Will this open on my friend's machine?"
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SendReady {
    pub ready: bool,
    /// "Ready to send" / "2 things to check before you send".
    pub headline: String,
    pub plugins: Vec<PluginUse>,
    /// Audio the project uses from outside its own folder. Names only.
    pub outside_files: Vec<OutsideFile>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PluginUse {
    pub name: String,
    pub tracks: Vec<String>,
    /// Ships with the DAW. `None` when Wit can't tell.
    pub stock: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct OutsideFile {
    pub file_name: String,
    /// "outside the project folder", "missing on this computer".
    pub reason: String,
}

/// Source audio to play for "Listen to the new parts". The app resolves
/// `file_name` inside the song's own folder; a Story never holds a path.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ListenRef {
    pub file_name: String,
    pub region: Option<String>,
    pub start_secs: Option<f64>,
    pub duration_secs: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TrustPanel {
    /// Folder names as the owner sees them ("~/Music/Logic"). Shown only on
    /// the owner's own screen; never copied into a share page.
    pub watched_folders: Vec<String>,
    /// "Wit never changes your projects.", "Nothing leaves this computer."
    pub statements: Vec<String>,
    pub counters: PilotCounters,
}

/// Local-only counters (no telemetry, no network). The pilot report copies
/// them only when the owner presses *Copy pilot report*.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
pub struct PilotCounters {
    pub compares_opened: u32,
    pub shares_created: u32,
    pub restores_made: u32,
    pub since: Option<Timestamp>,
}
