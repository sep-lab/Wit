//! Which region sits on which track, at which bar — a Rust port of
//! `experiments/logic_region_map.py`, frozen and fixed by
//! [#52](https://github.com/sep-lab/Wit/pull/52). That script is the spec:
//! every constant below is copied from it byte for byte and pinned to
//! `docs/FORMATS.md` by `test_each_offset_equals_its_documented_value`
//! (mirroring `tests/test_logic_region_map.py`'s own test of the same name),
//! never invented independently here.
//!
//! Two payloads carry what the container census (`census.rs`) cannot:
//! `AuRg`/`gRuA` records are the region *objects* (name, length in source
//! frames, a per-region UUID, and a family index grouping copies of one
//! source file); `EvSq`/`qSvE` records are a grid of 16-byte typed units
//! that include 48-byte audio-placement groups (position, 1-based track
//! number, and a link back to a region family). Neither payload's offsets
//! are documented anywhere `wit-logic`'s other modules read from — see
//! `docs/FORMATS.md`'s "Region payloads" section for the measured decode
//! rates this module's tests replicate.
//!
//! **What this module does not do**, matching the frozen script's own
//! stated limits (`experiments/logic_region_map.py`'s module docstring):
//! - **Which copy.** A placement links to a region *family* (one source
//!   file), not to an individual region record. [`family_label`] (a
//!   diagnostic label, matching the frozen script's own report format)
//!   says "one of N copies" honestly rather than naming one, whenever a
//!   family holds more than one region object; [`diff_placements`] instead
//!   returns a [`RegionSubject`] with `stem`/`resolved`/`ambiguous` kept
//!   apart, so a caller can phrase the hedge in its own words (`wit-story`
//!   says "a 'stem' region", never a copy count — see that type's doc for
//!   why).
//! - **The UUID-keyed region-table diff** (rename/resize, present/absent by
//!   UUID) that the Python script's `report_diff` also computes. Only the
//!   placement diff ([`diff_placements`]) is ported, because that is what
//!   `wit-story`'s region wiring needs (region *added/removed/moved on the
//!   timeline*); the region-table diff has no [`wit_model::ChangeRecord`]
//!   this wave adds a variant for.
//! - **MIDI regions, pre-roll, or a mid-song meter or tempo map.**
//! - **A structural head-byte collision filter.** `docs/FORMATS.md` (after
//!   the 2026-08-16 library rescan, #57) reports that checking placement
//!   head byte `+7 == 0x00` would separate every one of the library's
//!   9,142 accepted placements from every one of its 346 rejects, with no
//!   bar bound needed — the evidence for adding this filter is stronger
//!   than the frozen script itself acts on. This port still doesn't add
//!   it: it matches the frozen script's own four `REJECT_REASONS`
//!   (`truncated`/`track_zero`/`before_origin`/`beyond_max_bar`) exactly,
//!   by deliberate choice, rather than porting the *script's* behavior on
//!   three of them and independently improving on the fourth — a fifth
//!   filter the spec itself doesn't implement is a product decision for a
//!   future change to make on purpose, not something to smuggle in while
//!   porting.
//! - Bar arithmetic here is **tick space only** ([`Placement::tick`]), at the
//!   frozen script's own fixed 4/4 assumption where a bar number is needed
//!   ([`Placement::bar_at_4_4`], [`format_bar`]) — exactly what the script's
//!   own "WHAT THIS DOES NOT HANDLE" section says about tempo and meter
//!   maps. Converting a tick to a real bar using the project's *actual*
//!   time signature (`metadata.rs`), and deciding when that conversion is
//!   only "about", is `wit-story`'s job — this module has no
//!   [`crate`]-external dependency and stays that way.

use crate::frame::{Record, RECORD_HEADER_LEN};
use std::collections::BTreeMap;

// --------------------------------------------------------------------------
// constants — copied from experiments/logic_region_map.py, never re-derived
// --------------------------------------------------------------------------

/// `u32 @ record+0x08 = familyIndex << 18`.
pub const IDX_FIELD_OFFSET: usize = 0x08;
pub const IDX_SHIFT: u32 = 18;

pub const REGION_LENGTH_OFFSET: usize = 0x3A;
pub const REGION_NAME_LEN_OFFSET: usize = 0x6E;
pub const REGION_NAME_OFFSET: usize = 0x70;
/// The suffix behind the (padded) name is a constant 133 bytes; addressed
/// from the padded name end, never from a fixed offset (the record grows
/// with the name).
pub const REGION_FIXED_SUFFIX_LEN: usize = 133;
pub const REGION_UUID_OFFSET_IN_SUFFIX: usize = 0x56;
pub const UUID_LEN: usize = 16;
/// The longest name on the measured chain is 30 bytes; a name longer than
/// this is a decode failure, not a name (matches `extract.rs`'s own
/// `MAX_NAME_LEN` discipline for the same field family).
pub const MAX_REGION_NAME_LEN: usize = 100;

pub const EVENT_LEN: usize = 16;
/// Byte `+7` of every 16-byte unit — **[inferred]**, counted, never
/// interpreted as a filter (see module doc).
pub const EVENT_TYPE_BYTE: usize = 7;
pub const TERMINATOR_EVENT: [u8; 16] = [
    0xf1, 0x00, 0x00, 0x00, 0xff, 0xff, 0xff, 0x3f, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

pub const PLACEMENT_MARKER: [u8; 4] = [0x24, 0x00, 0x00, 0x00];
pub const PLACEMENT_POSITION_OFFSET: usize = 0x04;
/// Upstream calls it a per-region id; measured **not** a per-placement key
/// (`docs/FORMATS.md`). Ported for parity; nothing here keys on it.
pub const PLACEMENT_EVENT_ID_OFFSET: usize = 0x10;
/// 1-based track number.
pub const PLACEMENT_TRACK_OFFSET: usize = 0x14;
pub const PLACEMENT_LINK_OFFSET: usize = 0x2C;
/// Three 16-byte units.
pub const PLACEMENT_GROUP_LEN: usize = 0x30;
/// Byte `+7` of a placement group's three units, on every placement group
/// measured. Counted (`placements_with_known_unit_types`), never filtered on
/// (see module doc).
pub const PLACEMENT_UNIT_TYPES: [u8; 3] = [0x00, 0x89, 0xBC];

pub const TICKS_PER_QUARTER: u32 = 960;
/// 4/4 only — see the module doc.
pub const TICKS_PER_BAR: u32 = TICKS_PER_QUARTER * 4;
/// Regions use origin 34560 (9 bars); tempo/marker/note events use a
/// different origin (38400) this module never reads.
pub const REGION_TIME_ORIGIN: u32 = 34560;
/// At 4/4 and 40 BPM, 10,000 bars is over 16 hours of music.
pub const MAX_PLACEMENT_BAR: u32 = 10_000;

pub const TAG_REGION: [u8; 4] = *b"gRuA";
pub const TAG_EVENTS: [u8; 4] = *b"qSvE";

// --------------------------------------------------------------------------
// types
// --------------------------------------------------------------------------

/// One decoded `gRuA` (`AuRg`) record: the region *object*.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Region {
    pub family: u32,
    pub name: String,
    /// Length in frames of the *source file's* sample rate — not
    /// necessarily the project's (`docs/FORMATS.md`).
    pub length_frames: u32,
    /// RFC 4122-shaped, formatted as `xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx`.
    pub uuid: String,
    /// The record's byte offset in the file — diagnostic only.
    pub offset: usize,
}

/// One decoded audio-placement group inside a `qSvE` (`EvSq`) payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Placement {
    pub track: u8,
    /// `REGION_TIME_ORIGIN + tick@960`.
    pub position: u32,
    pub family: u32,
    pub event_id: u32,
}

impl Placement {
    /// Ticks since [`REGION_TIME_ORIGIN`]. Placements are only ever
    /// constructed by [`parse_event_stream`], which already rejects any
    /// position before the origin, so this never underflows in practice;
    /// `saturating_sub` keeps that a guarantee rather than an assumption.
    pub fn tick(&self) -> u32 {
        self.position.saturating_sub(REGION_TIME_ORIGIN)
    }

    /// The bar this placement falls on, **assuming a constant 4/4** — the
    /// frozen script's own fixed conversion (see the module doc). Not what
    /// `wit-story` shows a musician; that uses the project's real time
    /// signature instead.
    pub fn bar_at_4_4(&self) -> f64 {
        self.tick() as f64 / TICKS_PER_BAR as f64 + 1.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RejectReason {
    /// The 48-byte group runs past the end of the payload.
    Truncated,
    /// The track byte is 1-based, so 0 is decisively not a placement.
    TrackZero,
    /// `position < REGION_TIME_ORIGIN` — also where a genuine pre-roll
    /// placement would land: counted, not decoded (module doc).
    BeforeOrigin,
    BeyondMaxBar,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegionMapError {
    /// A `qSvE` payload whose length is not a multiple of [`EVENT_LEN`] —
    /// every field behind it would be a guess (mirrors the frozen script's
    /// `strict=True`, the mode both its single-file and `--diff` paths
    /// use).
    Misaligned { record_offset: usize, len: usize },
}

impl std::fmt::Display for RegionMapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RegionMapError::Misaligned { record_offset, len } => write!(
                f,
                "qSvE at {record_offset:#x} has a {len}-byte payload, not a multiple of {EVENT_LEN}"
            ),
        }
    }
}

impl std::error::Error for RegionMapError {}

/// Everything decoded from one save: every region object seen, every
/// resolved placement, and the reject/type-count diagnostics the golden
/// tests pin.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Song {
    pub regions: Vec<Region>,
    /// Sorted by `(position, track)` — timeline order, matching the frozen
    /// script's own `song.placements.sort(...)`.
    pub placements: Vec<Placement>,
    /// Every `gRuA` record seen, whether or not it decoded — the
    /// denominator for a real decode *rate* (the #45 bug this script's own
    /// fix corrected: counting only decoded records made the rate 100% by
    /// construction).
    pub region_records_seen: usize,
    pub event_type_counts: BTreeMap<u8, usize>,
    pub event_stream_count: usize,
    pub event_streams_with_one_terminator: usize,
    pub event_streams_ending_in_terminator: usize,
    pub placement_rejects: BTreeMap<RejectReason, usize>,
    pub placements_with_known_unit_types: usize,
}

impl Song {
    pub fn placement_markers_rejected(&self) -> usize {
        self.placement_rejects.values().sum()
    }

    /// Region objects grouped by family index, in family order — the same
    /// grouping the frozen script's `Song.families()` returns, but sorted
    /// (a `BTreeMap`) rather than insertion-ordered, which is exactly what
    /// the script's own `family_surplus`/`--scan` call sites immediately do
    /// with it (`sorted(song.families().items())`).
    pub fn families(&self) -> BTreeMap<u32, Vec<&Region>> {
        let mut out: BTreeMap<u32, Vec<&Region>> = BTreeMap::new();
        for region in &self.regions {
            out.entry(region.family).or_default().push(region);
        }
        out
    }
}

fn format_uuid(raw: &[u8]) -> String {
    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }
    format!(
        "{}-{}-{}-{}-{}",
        hex(&raw[0..4]),
        hex(&raw[4..6]),
        hex(&raw[6..8]),
        hex(&raw[8..10]),
        hex(&raw[10..16])
    )
}

/// Decode one `gRuA` record. `None` if it does not decode cleanly — the
/// caller (`parse`) counts every `gRuA` record it sees regardless, so a
/// `None` here is a counted decode failure, never a silent drop.
pub fn parse_region(data: &[u8], record: &Record<'_>) -> Option<Region> {
    let base = record.offset;
    let total = RECORD_HEADER_LEN + record.payload.len();
    if total < REGION_NAME_OFFSET + 2 {
        return None;
    }

    let name_len = u16::from_le_bytes(
        data.get(base + REGION_NAME_LEN_OFFSET..base + REGION_NAME_LEN_OFFSET + 2)?
            .try_into()
            .ok()?,
    ) as usize;
    if name_len == 0 || name_len > MAX_REGION_NAME_LEN {
        return None;
    }

    let padded = name_len + (name_len & 1);
    let name_end = REGION_NAME_OFFSET + padded;
    if name_end + REGION_FIXED_SUFFIX_LEN != total {
        // The size law didn't hold: every offset behind the name is a guess.
        return None;
    }

    let raw_name = data.get(base + REGION_NAME_OFFSET..base + REGION_NAME_OFFSET + name_len)?;
    let name = std::str::from_utf8(raw_name).ok()?;
    // The frozen script rejects `ord(ch) < 0x20` only (C0 controls) — never
    // DEL (U+007F) or a C1 control (U+0080–U+009F), which Rust's own
    // `char::is_control` *does* reject (found in review: this divergence
    // alone made a meaningful share of a 2,000-random-container decode
    // check disagree with the Python spec). `< '\u{20}'` is the literal,
    // exact port.
    if name.chars().any(|c| c < '\u{20}') {
        return None;
    }

    let idx_bytes = data.get(base + IDX_FIELD_OFFSET..base + IDX_FIELD_OFFSET + 4)?;
    let idx = u32::from_le_bytes(idx_bytes.try_into().ok()?);
    let length_bytes = data.get(base + REGION_LENGTH_OFFSET..base + REGION_LENGTH_OFFSET + 4)?;
    let length_frames = u32::from_le_bytes(length_bytes.try_into().ok()?);
    let uuid_at = base + name_end + REGION_UUID_OFFSET_IN_SUFFIX;
    let uuid_bytes = data.get(uuid_at..uuid_at + UUID_LEN)?;

    Some(Region {
        family: idx >> IDX_SHIFT,
        name: name.to_string(),
        length_frames,
        uuid: format_uuid(uuid_bytes),
        offset: base,
    })
}

/// What decoding one `qSvE` payload found.
#[derive(Debug, Clone, Default, PartialEq)]
struct EventStreamResult {
    placements: Vec<Placement>,
    type_counts: BTreeMap<u8, usize>,
    rejects: BTreeMap<RejectReason, usize>,
    terminators: usize,
    ends_in_terminator: bool,
    placements_with_known_unit_types: usize,
}

/// Decode one `qSvE` record's 16-byte unit grid. Placement groups are
/// located by their marker on this payload's own unit grid, bounded by this
/// one record — framing-aware addressing, never a file-wide scan.
///
/// `Err` when `payload.len()` is not a multiple of [`EVENT_LEN`] — every
/// field behind it would be a guess (mirrors the frozen script's
/// `strict=True`).
fn parse_event_stream(payload: &[u8]) -> Result<EventStreamResult, ()> {
    if !payload.len().is_multiple_of(EVENT_LEN) {
        return Err(());
    }
    let mut out = EventStreamResult::default();
    let max_position = REGION_TIME_ORIGIN + MAX_PLACEMENT_BAR * TICKS_PER_BAR;

    for i in (0..payload.len()).step_by(EVENT_LEN) {
        let type_byte = payload[i + EVENT_TYPE_BYTE];
        *out.type_counts.entry(type_byte).or_insert(0) += 1;
        if payload[i..i + EVENT_LEN] == TERMINATOR_EVENT {
            out.terminators += 1;
        }
        if payload[i..i + 4] != PLACEMENT_MARKER {
            continue;
        }

        let reason = if i + PLACEMENT_GROUP_LEN > payload.len() {
            Some(RejectReason::Truncated)
        } else {
            let track = payload[i + PLACEMENT_TRACK_OFFSET];
            let position = u32::from_le_bytes(
                payload[i + PLACEMENT_POSITION_OFFSET..i + PLACEMENT_POSITION_OFFSET + 4]
                    .try_into()
                    .unwrap(),
            );
            if track < 1 {
                Some(RejectReason::TrackZero)
            } else if position < REGION_TIME_ORIGIN {
                Some(RejectReason::BeforeOrigin)
            } else if position > max_position {
                Some(RejectReason::BeyondMaxBar)
            } else {
                None
            }
        };

        if let Some(reason) = reason {
            *out.rejects.entry(reason).or_insert(0) += 1;
            continue;
        }

        let track = payload[i + PLACEMENT_TRACK_OFFSET];
        let position = u32::from_le_bytes(
            payload[i + PLACEMENT_POSITION_OFFSET..i + PLACEMENT_POSITION_OFFSET + 4]
                .try_into()
                .unwrap(),
        );
        let event_id = u32::from_le_bytes(
            payload[i + PLACEMENT_EVENT_ID_OFFSET..i + PLACEMENT_EVENT_ID_OFFSET + 4]
                .try_into()
                .unwrap(),
        );
        let link = u32::from_le_bytes(
            payload[i + PLACEMENT_LINK_OFFSET..i + PLACEMENT_LINK_OFFSET + 4]
                .try_into()
                .unwrap(),
        );
        let unit_types = [
            payload[i + EVENT_TYPE_BYTE],
            payload[i + EVENT_LEN + EVENT_TYPE_BYTE],
            payload[i + 2 * EVENT_LEN + EVENT_TYPE_BYTE],
        ];
        if unit_types == PLACEMENT_UNIT_TYPES {
            out.placements_with_known_unit_types += 1;
        }
        out.placements.push(Placement {
            track,
            position,
            family: link / 4,
            event_id,
        });
    }

    out.ends_in_terminator =
        payload.len() >= EVENT_LEN && payload[payload.len() - EVENT_LEN..] == TERMINATOR_EVENT;
    Ok(out)
}

/// Parse one save's already-walked records into a [`Song`]. `records` must
/// come from [`crate::walk_records`] on the same `data` — record offsets are
/// resolved back into `data` to reach header fields (the family index)
/// outside the payload slice.
///
/// Refuses (returns `Err`) on the first misaligned `qSvE` payload, mirroring
/// the frozen script's `strict=True` — the mode its own `--diff` and
/// single-save paths use. A caller comparing two saves should treat that as
/// "Wit can't see region changes for this save", not a panic or a guess.
pub fn parse(data: &[u8], records: &[Record<'_>]) -> Result<Song, RegionMapError> {
    let mut song = Song::default();
    for record in records {
        if record.tag == TAG_REGION {
            song.region_records_seen += 1;
            if let Some(region) = parse_region(data, record) {
                song.regions.push(region);
            }
        } else if record.tag == TAG_EVENTS {
            song.event_stream_count += 1;
            let result =
                parse_event_stream(record.payload).map_err(|()| RegionMapError::Misaligned {
                    record_offset: record.offset,
                    len: record.payload.len(),
                })?;
            song.placements.extend(result.placements);
            for (k, v) in result.type_counts {
                *song.event_type_counts.entry(k).or_insert(0) += v;
            }
            for (k, v) in result.rejects {
                *song.placement_rejects.entry(k).or_insert(0) += v;
            }
            song.placements_with_known_unit_types += result.placements_with_known_unit_types;
            if result.terminators == 1 {
                song.event_streams_with_one_terminator += 1;
            }
            if result.ends_in_terminator {
                song.event_streams_ending_in_terminator += 1;
            }
        }
    }
    song.placements.sort_by_key(|p| (p.position, p.track));
    Ok(song)
}

/// Parse a `ProjectData` file's raw bytes directly, walking its records
/// itself. Bounded and typed like every other entry point in this crate —
/// see [`crate::WalkError`] for the container-framing errors this can
/// surface before region parsing even starts.
pub fn parse_bytes(data: &[u8]) -> Result<Song, RegionParseError> {
    // `walk_records` assumes its caller already validated the root header
    // (in particular that `data.len() >= ROOT_HEADER_LEN`, exactly what
    // `parse_root_header` checks) — the same order `crate::walk` uses.
    // Skipping this step is a real bug, not a defensive nicety: on an empty
    // or too-short `data`, `walk_records` alone trips its own internal
    // `debug_assert_eq!(pos, data.len())` (found by
    // `arbitrary_bytes_never_panic_region_parse`'s proptest, minimal
    // failing input `[]`).
    crate::frame::parse_root_header(data).map_err(RegionParseError::Container)?;
    let records = crate::frame::walk_records(data).map_err(RegionParseError::Container)?;
    parse(data, &records).map_err(RegionParseError::Regions)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegionParseError {
    Container(crate::WalkError),
    Regions(RegionMapError),
}

impl std::fmt::Display for RegionParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RegionParseError::Container(e) => write!(f, "{e}"),
            RegionParseError::Regions(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for RegionParseError {}

// --------------------------------------------------------------------------
// joining placements to regions — "which copy"
// --------------------------------------------------------------------------

/// The stem name of a placement's region family, and how many region
/// objects it holds. Where the family holds one region object the name is
/// exact; where it holds more, the count is the caller's cue to phrase the
/// result without claiming to know which copy it was. The stem is the
/// shortest name, ties broken by the name itself, so the choice never
/// depends on iteration order.
pub fn family_name(song: &Song, family: u32) -> (String, usize) {
    let families = song.families();
    let Some(regions) = families.get(&family) else {
        return (format!("<unresolved family {family}>"), 0);
    };
    let mut names: Vec<&str> = regions.iter().map(|r| r.name.as_str()).collect();
    names.sort();
    names.dedup();
    // The frozen script's tie-break is `len(n)` — Python's `len()` on a
    // `str` counts *characters*, not UTF-8 bytes. `n.len()` here is a byte
    // count, so a multi-byte name (a Persian one, e.g. — Unicode track and
    // region names are expected input, not an edge case) can be judged
    // "longer" than a same-or-fewer-character ASCII name purely because it
    // takes more bytes to encode, picking the wrong stem.
    let stem = names
        .into_iter()
        .min_by_key(|n| (n.chars().count(), *n))
        .unwrap_or_default();
    (stem.to_string(), regions.len())
}

/// How a change honestly names a family: the stem, or the stem plus how
/// many copies it has when which one is on the timeline is not decodable.
/// A diagnostic/map-report label — matches the frozen script's own
/// `family_label`. [`diff_placements`] uses [`RegionSubject`] instead,
/// which keeps `stem`/`resolved`/`ambiguous` separate so a caller can
/// decide how (or whether) to say so, rather than a single pre-formatted
/// string.
pub fn family_label(song: &Song, family: u32) -> String {
    let (stem, copies) = family_name(song, family);
    if copies <= 1 {
        stem
    } else {
        format!("{stem} (one of {copies} copies)")
    }
}

/// How honestly a placement change can name the region family it's about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegionSubject {
    /// The shortest name among the family's region objects
    /// ([`family_name`]) — `"<unresolved family N>"`, Wit's own
    /// placeholder rather than a musician-chosen name, when `resolved` is
    /// `false`.
    pub stem: String,
    /// `false` when the family has no region object at all (measured on
    /// GarageBand, which writes no `gRuA` records: every placement's
    /// family is then unresolved). A caller should say nothing about which
    /// region this is — never show `stem`'s placeholder to a musician —
    /// rather than guess.
    pub resolved: bool,
    /// `true` when the family holds more than one region object, so which
    /// physical copy this placement is cannot be told apart from its
    /// siblings. Deliberately not a copy *count*: a family can hold region
    /// objects that were never placed at all, so "how many copies" would
    /// overstate how many placements this one could actually be confused
    /// with.
    pub ambiguous: bool,
}

fn subject_of(song: &Song, family: u32) -> RegionSubject {
    let (stem, copies) = family_name(song, family);
    RegionSubject {
        stem,
        resolved: copies > 0,
        ambiguous: copies > 1,
    }
}

// --------------------------------------------------------------------------
// the move-pairing diff
// --------------------------------------------------------------------------

/// What changed between two saves' placements.
#[derive(Debug, Clone, PartialEq)]
pub enum PlacementChange {
    /// A family with exactly one placement gone and exactly one new is an
    /// unambiguous move — never guessed when more than one of a family
    /// moved at once (module doc; `docs/FORMATS.md` "How the diff pairs
    /// placements").
    Moved {
        /// Resolved against the *new* song, matching the frozen script.
        subject: RegionSubject,
        from: (u8, u32),
        to: (u8, u32),
    },
    Removed {
        /// Resolved against the *old* song — what the region was called
        /// (or last known as) at the moment it disappeared, never the
        /// name it may have taken on since.
        subject: RegionSubject,
        track: u8,
        position: u32,
    },
    Added {
        /// Resolved against the *new* song.
        subject: RegionSubject,
        track: u8,
        position: u32,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct PlacementKey {
    track: u8,
    position: u32,
    family: u32,
}

fn keyed(placements: &[Placement]) -> BTreeMap<PlacementKey, i64> {
    let mut m = BTreeMap::new();
    for p in placements {
        *m.entry(PlacementKey {
            track: p.track,
            position: p.position,
            family: p.family,
        })
        .or_insert(0) += 1;
    }
    m
}

/// What happened on the timeline between two saves.
///
/// Placements are compared as `(track, position, family)` multisets. Exact
/// matches cancel (mirroring Python's `Counter` subtraction: only the
/// *positive* remainder on each side survives). What's left is grouped by
/// family: a family with exactly one placement gone and exactly one new is
/// reported as [`PlacementChange::Moved`]; everything else is reported as
/// added/removed. Output is primarily in timeline order (position, then
/// track, then moved-before-removed-before-added at a tied position).
///
/// **Tie-break note.** The frozen script breaks a tie in that same primary
/// key by the *rendered line's text* — effectively an alphabetical
/// secondary sort by region name. Two different families added or removed
/// at the exact same `(track, position)` in the same category is the only
/// way to hit that tie (a real but rare shape); this port's secondary
/// order there is family-id ascending (a `BTreeSet<u32>` iterated in that
/// order, stable-sorted), not a name comparison — deterministic, but not
/// guaranteed byte-identical to the frozen script's tie order in that one
/// rare case.
pub fn diff_placements(old: &Song, new: &Song) -> Vec<PlacementChange> {
    let before = keyed(&old.placements);
    let after = keyed(&new.placements);

    let mut gone: BTreeMap<u32, Vec<(u8, u32)>> = BTreeMap::new();
    let mut came: BTreeMap<u32, Vec<(u8, u32)>> = BTreeMap::new();
    let keys: std::collections::BTreeSet<PlacementKey> =
        before.keys().chain(after.keys()).copied().collect();
    for key in keys {
        let b = before.get(&key).copied().unwrap_or(0);
        let a = after.get(&key).copied().unwrap_or(0);
        if b > a {
            gone.entry(key.family)
                .or_default()
                .extend(std::iter::repeat_n(
                    (key.track, key.position),
                    (b - a) as usize,
                ));
        }
        if a > b {
            came.entry(key.family)
                .or_default()
                .extend(std::iter::repeat_n(
                    (key.track, key.position),
                    (a - b) as usize,
                ));
        }
    }

    let families: std::collections::BTreeSet<u32> =
        gone.keys().chain(came.keys()).copied().collect();
    let mut events: Vec<((u32, u8, u8), PlacementChange)> = Vec::new();
    for family in families {
        let mut was = gone.remove(&family).unwrap_or_default();
        let mut now = came.remove(&family).unwrap_or_default();
        was.sort();
        now.sort();
        if was.len() == 1 && now.len() == 1 {
            let (t0, p0) = was[0];
            let (t1, p1) = now[0];
            let subject = subject_of(new, family);
            events.push((
                (p0, t0, 0),
                PlacementChange::Moved {
                    subject,
                    from: (t0, p0),
                    to: (t1, p1),
                },
            ));
            continue;
        }
        for (track, position) in was {
            let subject = subject_of(old, family);
            events.push((
                (position, track, 1),
                PlacementChange::Removed {
                    subject,
                    track,
                    position,
                },
            ));
        }
        for (track, position) in now {
            let subject = subject_of(new, family);
            events.push((
                (position, track, 2),
                PlacementChange::Added {
                    subject,
                    track,
                    position,
                },
            ));
        }
    }
    events.sort_by_key(|(k, _)| *k);
    events.into_iter().map(|(_, c)| c).collect()
}

/// The exact bar a tick position falls on, at [`TICKS_PER_BAR`] — a whole
/// bar prints as `9`; a fraction prints as a decimal when three places hold
/// it exactly (`55.75`), otherwise as a mixed fraction (`66 5/6`), so a
/// one-tick move never prints as no move at all. Ported for parity with the
/// frozen script's `format_bar`; `wit-story` does not use this (it converts
/// with the project's real time signature instead — see the module doc).
pub fn format_bar(position: u32) -> String {
    let tick = position.saturating_sub(REGION_TIME_ORIGIN) as i64;
    let bar_num = tick + TICKS_PER_BAR as i64; // (tick / TICKS_PER_BAR + 1) as a fraction over TICKS_PER_BAR
    let bar_den = TICKS_PER_BAR as i64;
    let g = gcd(bar_num.unsigned_abs(), bar_den.unsigned_abs()) as i64;
    let (num, den) = (bar_num / g, bar_den / g);
    if den == 1 {
        return num.to_string();
    }
    // Three decimal places, exact only when the denominator divides 1000.
    if 1000 % den == 0 {
        let thousandths = num * (1000 / den);
        let whole = thousandths / 1000;
        let part = thousandths % 1000;
        let s = format!("{whole}.{part:03}");
        let trimmed = s.trim_end_matches('0').trim_end_matches('.');
        return trimmed.to_string();
    }
    let whole = num / den;
    let rest_num = num - whole * den;
    format!("{whole} {rest_num}/{den}")
}

fn gcd(a: u64, b: u64) -> u64 {
    if b == 0 {
        a.max(1)
    } else {
        gcd(b, a % b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::{walk_records, MAGIC, ROOT_HEADER_LEN};

    // ---------------------------------------------------------------- //
    // golden bytes — literal offsets, independent of this module's own
    // constants (mirrors tests/test_logic_region_map.py's golden section)
    // ---------------------------------------------------------------- //

    const GOLDEN_UUID_SNARE: &str = "2d5ee052-641c-11f1-854a-205d0d40b51c";
    const GOLDEN_UUID_KICK: &str = "a17c3e08-0b2d-11f1-9c3e-6e0b9a1f4d27";

    fn hex_into(rec: &mut [u8], range: std::ops::Range<usize>, hex: &str) {
        let bytes: Vec<u8> = hex
            .split_whitespace()
            .map(|b| u8::from_str_radix(b, 16).unwrap())
            .collect();
        rec[range].copy_from_slice(&bytes);
    }

    /// gRuA, odd-length name: "Snare" (5 bytes, padded to 6), family 7.
    /// 251 bytes = 0x70 + 6 + 133. payload_size 215 = 209 + 5 + 1. The UUID
    /// sits at 0x70 + 6 + 0x56 = 0xCC.
    fn golden_region_snare() -> Vec<u8> {
        let mut rec = vec![0u8; 251];
        rec[0x00..0x04].copy_from_slice(b"gRuA");
        hex_into(&mut rec, 0x08..0x0C, "00 00 1c 00"); // 7 << 18
        hex_into(&mut rec, 0x1C..0x20, "d7 00 00 00"); // payload size 215
        hex_into(&mut rec, 0x3A..0x3E, "c5 a5 02 00"); // 173509 frames
        hex_into(&mut rec, 0x6E..0x70, "05 00"); // name length 5
        rec[0x70..0x75].copy_from_slice(b"Snare");
        hex_into(
            &mut rec,
            0xCC..0xDC,
            "2d 5e e0 52 64 1c 11 f1 85 4a 20 5d 0d 40 b5 1c",
        );
        rec
    }

    /// gRuA, even-length name: "Kick" (4 bytes, no pad), family 9. 249 bytes
    /// = 0x70 + 4 + 133. payload_size 213 = 209 + 4. UUID at 0xCA.
    fn golden_region_kick() -> Vec<u8> {
        let mut rec = vec![0u8; 249];
        rec[0x00..0x04].copy_from_slice(b"gRuA");
        hex_into(&mut rec, 0x08..0x0C, "00 00 24 00"); // 9 << 18
        hex_into(&mut rec, 0x1C..0x20, "d5 00 00 00"); // payload size 213
        hex_into(&mut rec, 0x3A..0x3E, "c0 96 03 00"); // 235200 frames
        hex_into(&mut rec, 0x6E..0x70, "04 00");
        rec[0x70..0x74].copy_from_slice(b"Kick");
        hex_into(
            &mut rec,
            0xCA..0xDA,
            "a1 7c 3e 08 0b 2d 11 f1 9c 3e 6e 0b 9a 1f 4d 27",
        );
        rec
    }

    /// qSvE holding one 48-byte audio placement group plus the terminator.
    /// Track 3, bar 55.75 (position 34560 + 54.75 * 3840 = 244800 =
    /// 0x0003bc40), event id 0x64, link 0x1c (= family 7 * 4). The three
    /// units carry byte +7 = 00 / 89 / bc.
    fn golden_event_stream() -> Vec<u8> {
        let payload_hex = "24 00 00 00 40 bc 03 00 00 00 00 00 00 00 00 00 \
                            64 00 00 00 03 00 00 89 00 00 00 00 00 00 00 00 \
                            00 00 00 00 00 00 00 bc 00 00 00 00 1c 00 00 00 \
                            f1 00 00 00 ff ff ff 3f 00 00 00 00 00 00 00 00";
        let payload: Vec<u8> = payload_hex
            .split_whitespace()
            .map(|b| u8::from_str_radix(b, 16).unwrap())
            .collect();
        assert_eq!(payload.len(), 64);
        let mut rec = vec![0u8; RECORD_HEADER_LEN];
        rec[0x00..0x04].copy_from_slice(b"qSvE");
        hex_into(&mut rec, 0x08..0x0C, "00 00 04 00");
        hex_into(&mut rec, 0x1C..0x20, "40 00 00 00"); // payload size 64
        rec.extend_from_slice(&payload);
        rec
    }

    fn golden_container() -> Vec<u8> {
        let body: Vec<u8> = [
            golden_region_snare(),
            golden_region_kick(),
            golden_event_stream(),
        ]
        .concat();
        assert_eq!(body.len(), 600);
        let mut root = vec![0u8; ROOT_HEADER_LEN];
        root[0x00..0x04].copy_from_slice(&MAGIC);
        root[0x04..0x06].copy_from_slice(&[0xd0, 0x09]);
        hex_into(&mut root, 0x10..0x14, "58 02 00 00"); // 600 body bytes
        [root, body].concat()
    }

    fn parse_golden() -> Song {
        let data = golden_container();
        let records = walk_records(&data).unwrap();
        parse(&data, &records).unwrap()
    }

    #[test]
    fn test_golden_bytes_decode_both_regions_exactly() {
        let song = parse_golden();
        assert_eq!(song.region_records_seen, 2);
        let got: Vec<(u32, &str, u32, &str, usize)> = song
            .regions
            .iter()
            .map(|r| {
                (
                    r.family,
                    r.name.as_str(),
                    r.length_frames,
                    r.uuid.as_str(),
                    r.offset,
                )
            })
            .collect();
        assert_eq!(
            got,
            vec![
                (7, "Snare", 173509, GOLDEN_UUID_SNARE, 0x18),
                (9, "Kick", 235200, GOLDEN_UUID_KICK, 0x18 + 251),
            ]
        );
    }

    #[test]
    fn test_golden_bytes_decode_the_placement_exactly() {
        let song = parse_golden();
        let got: Vec<(u8, u32, u32, u32)> = song
            .placements
            .iter()
            .map(|p| (p.track, p.position, p.family, p.event_id))
            .collect();
        assert_eq!(got, vec![(3, 244800, 7, 0x64)]);
        let placement = song.placements[0];
        assert_eq!(placement.tick(), 210240);
        assert_eq!(format_bar(placement.position), "55.75");
        assert_eq!(family_label(&song, placement.family), "Snare");
    }

    #[test]
    fn test_golden_bytes_decode_the_unit_grid_exactly() {
        let song = parse_golden();
        let counts: BTreeMap<u8, usize> = song.event_type_counts.clone();
        let expected: BTreeMap<u8, usize> = [(0x00u8, 1), (0x89, 1), (0xBC, 1), (0x3F, 1)]
            .into_iter()
            .collect();
        assert_eq!(counts, expected);
        assert_eq!(song.event_stream_count, 1);
        assert_eq!(song.event_streams_with_one_terminator, 1);
        assert_eq!(song.event_streams_ending_in_terminator, 1);
        assert_eq!(song.placements_with_known_unit_types, 1);
        assert_eq!(song.placement_markers_rejected(), 0);
    }

    // ---------------------------------------------------------------- //
    // every offset pinned to its docs/FORMATS.md value
    // ---------------------------------------------------------------- //

    fn formats_md() -> String {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/FORMATS.md");
        std::fs::read_to_string(path).unwrap()
    }

    #[test]
    fn every_offset_equals_its_documented_value() {
        let doc = formats_md();
        let cases: &[(&str, bool)] = &[
            (
                "| `+0x08` | u32 | `familyIndex << 18`",
                IDX_FIELD_OFFSET == 0x08,
            ),
            ("`familyIndex << 18`", IDX_SHIFT == 18),
            (
                "| `+0x3A` | u32 | Region length",
                REGION_LENGTH_OFFSET == 0x3A,
            ),
            (
                "| `+0x6E` | u16 | Name length",
                REGION_NAME_LEN_OFFSET == 0x6E,
            ),
            ("| `+0x70` |", REGION_NAME_OFFSET == 0x70),
            ("a constant 133 bytes", REGION_FIXED_SUFFIX_LEN == 133),
            (
                "| name end `+0x56` | 16 B | Region UUID",
                REGION_UUID_OFFSET_IN_SUFFIX == 0x56,
            ),
            (
                "| `+0x04` | u32 | Position",
                PLACEMENT_POSITION_OFFSET == 0x04,
            ),
            ("| `+0x10` | u32 |", PLACEMENT_EVENT_ID_OFFSET == 0x10),
            (
                "| `+0x14` | u8 | **Track number, 1-based**",
                PLACEMENT_TRACK_OFFSET == 0x14,
            ),
            (
                "| `+0x2c` | u32 | Region link",
                PLACEMENT_LINK_OFFSET == 0x2C,
            ),
            ("Position = `34560 + tick@960`", REGION_TIME_ORIGIN == 34560),
            (
                "headed by `24 00 00 00`",
                PLACEMENT_MARKER == [0x24, 0x00, 0x00, 0x00],
            ),
        ];
        for (doc_text, matches_constant) in cases {
            assert!(
                matches_constant,
                "constant drifted from its own literal: {doc_text}"
            );
            assert!(
                doc.contains(doc_text),
                "docs/FORMATS.md no longer says {doc_text:?} — code and doc have drifted"
            );
        }
    }

    #[test]
    fn the_non_offset_constants_are_the_measured_ones() {
        assert_eq!(TAG_REGION, *b"gRuA");
        assert_eq!(TAG_EVENTS, *b"qSvE");
        assert_eq!(UUID_LEN, 16);
        assert_eq!(
            (EVENT_LEN, EVENT_TYPE_BYTE, PLACEMENT_GROUP_LEN),
            (16, 7, 48)
        );
        assert_eq!(
            TERMINATOR_EVENT,
            [
                0xf1, 0x00, 0x00, 0x00, 0xff, 0xff, 0xff, 0x3f, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x00
            ]
        );
        assert_eq!(PLACEMENT_UNIT_TYPES, [0x00, 0x89, 0xBC]);
        assert_eq!((TICKS_PER_QUARTER, TICKS_PER_BAR), (960, 3840));
        assert_eq!((MAX_REGION_NAME_LEN, MAX_PLACEMENT_BAR), (100, 10_000));
    }

    // ---------------------------------------------------------------- //
    // builders, on top of the offsets pinned above
    // ---------------------------------------------------------------- //

    fn build_record(tag: &[u8; 4], idx: u32, payload: &[u8]) -> Vec<u8> {
        let mut header = vec![0u8; RECORD_HEADER_LEN];
        header[0..4].copy_from_slice(tag);
        header[IDX_FIELD_OFFSET..IDX_FIELD_OFFSET + 4].copy_from_slice(&idx.to_le_bytes());
        header[0x1C..0x20].copy_from_slice(&(payload.len() as u32).to_le_bytes());
        [header, payload.to_vec()].concat()
    }

    fn build_container(records: &[Vec<u8>]) -> Vec<u8> {
        let body: Vec<u8> = records.concat();
        let mut root = vec![0u8; ROOT_HEADER_LEN];
        root[0x00..0x04].copy_from_slice(&MAGIC);
        root[0x04..0x06].copy_from_slice(&[0xd0, 0x09]);
        root[0x10..0x14].copy_from_slice(&(body.len() as u32).to_le_bytes());
        [root, body].concat()
    }

    fn build_region(name: &str, length_frames: u32, uuid: [u8; 16], family: u32) -> Vec<u8> {
        let raw = name.as_bytes();
        let name_len = raw.len();
        let padded = name_len + (name_len & 1);
        let length_at = REGION_LENGTH_OFFSET - RECORD_HEADER_LEN;
        let name_len_at = REGION_NAME_LEN_OFFSET - RECORD_HEADER_LEN;
        let name_at = REGION_NAME_OFFSET - RECORD_HEADER_LEN;
        let uuid_at = name_at + padded + REGION_UUID_OFFSET_IN_SUFFIX;
        let total = name_at + padded + REGION_FIXED_SUFFIX_LEN;

        let mut payload = vec![0u8; total];
        payload[length_at..length_at + 4].copy_from_slice(&length_frames.to_le_bytes());
        payload[name_len_at..name_len_at + 2].copy_from_slice(&(name_len as u16).to_le_bytes());
        payload[name_at..name_at + name_len].copy_from_slice(raw);
        payload[uuid_at..uuid_at + UUID_LEN].copy_from_slice(&uuid);
        build_record(&TAG_REGION, family << IDX_SHIFT, &payload)
    }

    fn build_placement(track: u8, bar: f64, family: u32, event_id: u32) -> [u8; 48] {
        let mut event = [0u8; PLACEMENT_GROUP_LEN];
        event[0..4].copy_from_slice(&PLACEMENT_MARKER);
        let position = REGION_TIME_ORIGIN + ((bar - 1.0) * TICKS_PER_BAR as f64).round() as u32;
        event[PLACEMENT_POSITION_OFFSET..PLACEMENT_POSITION_OFFSET + 4]
            .copy_from_slice(&position.to_le_bytes());
        event[PLACEMENT_EVENT_ID_OFFSET..PLACEMENT_EVENT_ID_OFFSET + 4]
            .copy_from_slice(&event_id.to_le_bytes());
        event[PLACEMENT_TRACK_OFFSET] = track;
        event[EVENT_LEN + EVENT_TYPE_BYTE] = 0x89;
        event[2 * EVENT_LEN + EVENT_TYPE_BYTE] = 0xBC;
        event[PLACEMENT_LINK_OFFSET..PLACEMENT_LINK_OFFSET + 4]
            .copy_from_slice(&(family * 4).to_le_bytes());
        event
    }

    fn build_event_stream(events: &[[u8; 48]]) -> Vec<u8> {
        let payload: Vec<u8> = events.iter().flat_map(|e| e.iter().copied()).collect();
        build_record(&TAG_EVENTS, 0, &payload)
    }

    fn uuid_bytes(seed: u8) -> [u8; 16] {
        let mut u = [0u8; 16];
        u[0] = seed;
        u[1..].copy_from_slice(&[
            0x5e, 0xe0, 0x52, 0x64, 0x1c, 0x11, 0xf1, 0x85, 0x4a, 0x20, 0x5d, 0x0d, 0x40, 0xb5,
            0x1c,
        ]);
        u
    }

    fn song_with(regions: &[(&str, u32, u8, u32)], placements: &[(u8, f64, u32, u32)]) -> Song {
        let region_records: Vec<Vec<u8>> = regions
            .iter()
            .map(|(name, len, uuid_seed, family)| {
                build_region(name, *len, uuid_bytes(*uuid_seed), *family)
            })
            .collect();
        let events: Vec<[u8; 48]> = placements
            .iter()
            .map(|(track, bar, family, event_id)| build_placement(*track, *bar, *family, *event_id))
            .collect();
        let mut records = region_records;
        records.push(build_event_stream(&events));
        let data = build_container(&records);
        let recs = walk_records(&data).unwrap();
        parse(&data, &recs).unwrap()
    }

    #[test]
    fn an_even_length_name_decodes_with_its_length_and_uuid() {
        let song = song_with(&[("Deep Down Shaker", 173509, 0x2D, 0)], &[]);
        assert_eq!(song.regions.len(), 1);
        assert_eq!(song.regions[0].name, "Deep Down Shaker");
        assert_eq!(song.regions[0].length_frames, 173509);
    }

    #[test]
    fn an_odd_length_name_is_padded_and_still_decodes() {
        let song = song_with(&[("Slow Drift Beat", 1676673, 0x2D, 0)], &[]);
        assert_eq!(song.regions[0].name, "Slow Drift Beat");
    }

    #[test]
    fn a_record_that_fails_to_decode_is_still_counted_as_seen() {
        let mut broken = build_region("Kick", 100, uuid_bytes(2), 0);
        // Corrupt the name-length field so the size law fails.
        let name_len_off = RECORD_HEADER_LEN + (REGION_NAME_LEN_OFFSET - RECORD_HEADER_LEN);
        broken[name_len_off..name_len_off + 2].copy_from_slice(&0u16.to_le_bytes());
        let good = build_region("Loop", 100, uuid_bytes(1), 0);
        let data = build_container(&[good, broken]);
        let recs = walk_records(&data).unwrap();
        let song = parse(&data, &recs).unwrap();
        assert_eq!((song.regions.len(), song.region_records_seen), (1, 2));
    }

    #[test]
    fn a_name_that_is_not_utf8_is_skipped() {
        let mut record = build_region("Kick", 100, uuid_bytes(1), 0);
        let name_off = RECORD_HEADER_LEN + (REGION_NAME_OFFSET - RECORD_HEADER_LEN);
        record[name_off..name_off + 4].copy_from_slice(&[0xff, 0xfe, 0xfd, 0xfc]);
        let data = build_container(&[record]);
        let recs = walk_records(&data).unwrap();
        assert!(parse(&data, &recs).unwrap().regions.is_empty());
    }

    /// Found in review: `char::is_control` also rejects DEL (U+007F) and
    /// the C1 controls (U+0080–U+009F), where the frozen script's
    /// `ord(ch) < 0x20` rejects only the C0 controls. This is the literal
    /// port — DEL decodes.
    #[test]
    fn a_name_holding_del_still_decodes() {
        let name = "Ki\u{7f}ck";
        let song = song_with(&[(name, 100, 1, 0)], &[]);
        assert_eq!(song.regions.len(), 1);
        assert_eq!(song.regions[0].name, name);
    }

    #[test]
    fn a_name_holding_a_c0_control_character_is_still_skipped() {
        let name = "Ki\u{07}ck"; // BEL, 0x07 -- below 0x20 on both sides.
        let song = song_with(&[(name, 100, 1, 0)], &[]);
        assert!(song.regions.is_empty());
    }

    #[test]
    fn regions_sharing_an_idx_form_one_family() {
        let song = song_with(
            &[
                ("Silky Acid Bass", 651323, 1, 7),
                ("Silky Acid Bass.1", 651323, 2, 7),
                ("Windmill Synth", 338688, 3, 9),
            ],
            &[],
        );
        let sizes: BTreeMap<u32, usize> = song
            .families()
            .into_iter()
            .map(|(f, r)| (f, r.len()))
            .collect();
        assert_eq!(sizes, [(7, 2), (9, 1)].into_iter().collect());
    }

    // ---------------------------------------------------------------- //
    // qSvE / rejects
    // ---------------------------------------------------------------- //

    #[test]
    fn a_payload_not_a_multiple_of_sixteen_is_refused() {
        let record = build_record(&TAG_EVENTS, 0, &[0u8; 20]);
        let data = build_container(&[record]);
        let recs = walk_records(&data).unwrap();
        assert!(matches!(
            parse(&data, &recs),
            Err(RegionMapError::Misaligned { .. })
        ));
    }

    #[test]
    fn a_placement_decodes_to_its_track_bar_and_family() {
        let song = song_with(&[], &[(7, 16.0, 2, 0x58)]);
        assert_eq!(song.placements.len(), 1);
        assert_eq!(song.placements[0].track, 7);
        assert_eq!(song.placements[0].bar_at_4_4(), 16.0);
        assert_eq!(song.placements[0].family, 2);
    }

    #[test]
    fn a_half_bar_position_is_kept_not_rounded() {
        let song = song_with(&[], &[(3, 55.5, 1, 0x58)]);
        assert_eq!(song.placements[0].bar_at_4_4(), 55.5);
    }

    #[test]
    fn a_marker_carrying_track_zero_is_rejected_as_a_collision() {
        let song = song_with(&[], &[(0, 5.0, 1, 0x58)]);
        assert!(song.placements.is_empty());
        assert_eq!(
            song.placement_rejects.get(&RejectReason::TrackZero),
            Some(&1)
        );
    }

    #[test]
    fn a_marker_before_the_region_time_origin_is_rejected() {
        let mut event = build_placement(4, 1.0, 1, 0x58);
        event[PLACEMENT_POSITION_OFFSET..PLACEMENT_POSITION_OFFSET + 4]
            .copy_from_slice(&12u32.to_le_bytes());
        let data = build_container(&[build_event_stream(&[event])]);
        let recs = walk_records(&data).unwrap();
        let song = parse(&data, &recs).unwrap();
        assert!(song.placements.is_empty());
        assert_eq!(
            song.placement_rejects.get(&RejectReason::BeforeOrigin),
            Some(&1)
        );
    }

    #[test]
    fn a_marker_positioned_beyond_any_real_song_is_rejected() {
        let mut event = build_placement(4, 1.0, 1, 0x58);
        event[PLACEMENT_POSITION_OFFSET..PLACEMENT_POSITION_OFFSET + 4]
            .copy_from_slice(&3_154_082_048u32.to_le_bytes());
        let data = build_container(&[build_event_stream(&[event])]);
        let recs = walk_records(&data).unwrap();
        let song = parse(&data, &recs).unwrap();
        assert!(song.placements.is_empty());
        assert_eq!(
            song.placement_rejects.get(&RejectReason::BeyondMaxBar),
            Some(&1)
        );
    }

    #[test]
    fn a_group_truncated_by_the_end_of_the_payload_is_rejected() {
        let mut payload = PLACEMENT_MARKER.to_vec();
        payload.extend(std::iter::repeat_n(0u8, 12));
        let record = build_record(&TAG_EVENTS, 0, &payload);
        let data = build_container(&[record]);
        let recs = walk_records(&data).unwrap();
        let song = parse(&data, &recs).unwrap();
        assert!(song.placements.is_empty());
        assert_eq!(song.placement_markers_rejected(), 1);
        assert_eq!(
            song.placement_rejects.get(&RejectReason::Truncated),
            Some(&1)
        );
    }

    #[test]
    fn a_group_whose_unit_types_differ_is_decoded_but_not_counted_as_known() {
        let mut event = build_placement(4, 1.0, 1, 0x58);
        event[2 * EVENT_LEN + EVENT_TYPE_BYTE] = 0x88;
        let data = build_container(&[build_event_stream(&[event])]);
        let recs = walk_records(&data).unwrap();
        let song = parse(&data, &recs).unwrap();
        assert_eq!(
            (song.placements.len(), song.placements_with_known_unit_types),
            (1, 0)
        );
    }

    #[test]
    fn placements_are_returned_in_timeline_order() {
        let song = song_with(&[], &[(1, 21.0, 1, 1), (2, 5.0, 2, 2), (3, 16.0, 3, 3)]);
        let bars: Vec<f64> = song.placements.iter().map(|p| p.bar_at_4_4()).collect();
        assert_eq!(bars, vec![5.0, 16.0, 21.0]);
    }

    // ---------------------------------------------------------------- //
    // joining placements to regions
    // ---------------------------------------------------------------- //

    #[test]
    fn a_single_region_family_names_its_placement_exactly() {
        let song = song_with(&[("Slow Drift Beat", 1676673, 1, 5)], &[(22, 17.0, 5, 1)]);
        assert_eq!(family_label(&song, 5), "Slow Drift Beat");
    }

    #[test]
    fn a_multi_copy_family_reports_the_stem_and_the_count() {
        let song = song_with(
            &[
                ("Angelic Vocal FX 03", 156145, 1, 8),
                ("Angelic Vocal FX 03.1", 156145, 2, 8),
            ],
            &[(13, 1.0, 8, 1)],
        );
        assert_eq!(
            family_label(&song, 8),
            "Angelic Vocal FX 03 (one of 2 copies)"
        );
    }

    #[test]
    fn the_stem_tie_break_does_not_depend_on_iteration_order() {
        for order in [["Beta", "Alfa"], ["Alfa", "Beta"]] {
            let song = song_with(
                &[(order[0], 1, 1, 3), (order[1], 1, 2, 3)],
                &[(1, 1.0, 3, 1)],
            );
            assert_eq!(family_name(&song, 3), ("Alfa".to_string(), 2));
        }
    }

    /// Found in review: a byte-length tie-break picks the wrong stem for a
    /// multi-byte name. "آواز" (Persian, 4 characters) is 8 bytes in UTF-8;
    /// "ABCDE" is 5 characters in 5 bytes. Python's `len()` counts
    /// characters, so it picks "آواز" (4 < 5 characters) — a byte-length
    /// tie-break would wrongly pick "ABCDE" (5 < 8 bytes) instead. Persian
    /// names are expected input on this field, not an edge case.
    #[test]
    fn the_stem_tie_break_counts_characters_not_bytes() {
        let song = song_with(&[("آواز", 1, 1, 3), ("ABCDE", 1, 2, 3)], &[(1, 1.0, 3, 1)]);
        assert_eq!(family_name(&song, 3), ("آواز".to_string(), 2));
    }

    #[test]
    fn a_placement_linking_to_no_region_says_so_rather_than_inventing_a_name() {
        let song = song_with(&[], &[(1, 1.0, 4, 1)]);
        assert_eq!(family_label(&song, 4), "<unresolved family 4>");
    }

    // ---------------------------------------------------------------- //
    // move-pairing semantics
    // ---------------------------------------------------------------- //

    #[test]
    fn a_region_moving_one_bar_is_the_only_thing_reported() {
        let old = song_with(&[("Deep Down Shaker", 173509, 1, 2)], &[(1, 1.0, 2, 1)]);
        let new = song_with(&[("Deep Down Shaker", 173509, 1, 2)], &[(1, 5.0, 2, 1)]);
        let changes = diff_placements(&old, &new);
        assert_eq!(changes.len(), 1);
        assert!(matches!(
            &changes[0],
            PlacementChange::Moved { subject, from: (1, _), to: (1, _) }
                if subject.stem == "Deep Down Shaker" && subject.resolved && !subject.ambiguous
        ));
    }

    #[test]
    fn moving_one_region_past_another_reports_only_that_move() {
        let old = song_with(
            &[("Kick", 1000, 1, 1), ("Snare", 1000, 2, 2)],
            &[(1, 1.0, 1, 1), (1, 5.0, 2, 2)],
        );
        let new = song_with(
            &[("Kick", 1000, 1, 1), ("Snare", 1000, 2, 2)],
            &[(1, 5.0, 2, 2), (1, 9.0, 1, 1)],
        );
        let changes = diff_placements(&old, &new);
        assert_eq!(changes.len(), 1);
        match &changes[0] {
            PlacementChange::Moved { subject, from, to } => {
                assert_eq!(subject.stem, "Kick");
                assert_eq!(*from, (1, REGION_TIME_ORIGIN));
                assert_eq!(*to, (1, REGION_TIME_ORIGIN + 8 * TICKS_PER_BAR));
            }
            other => panic!("expected a move, got {other:?}"),
        }
    }

    #[test]
    fn two_regions_swapping_positions_are_both_reported() {
        let old = song_with(
            &[("Kick", 1000, 1, 1), ("Snare", 1000, 2, 2)],
            &[(1, 1.0, 1, 1), (1, 5.0, 2, 2)],
        );
        let new = song_with(
            &[("Kick", 1000, 1, 1), ("Snare", 1000, 2, 2)],
            &[(1, 1.0, 2, 2), (1, 5.0, 1, 1)],
        );
        let changes = diff_placements(&old, &new);
        assert_eq!(changes.len(), 2, "{changes:?}");
        assert!(changes
            .iter()
            .all(|c| matches!(c, PlacementChange::Moved { .. })));
    }

    #[test]
    fn an_ambiguous_same_family_pairing_is_not_guessed() {
        let old = song_with(&[("Kick", 1000, 1, 1)], &[(1, 1.0, 1, 1), (1, 5.0, 1, 2)]);
        let new = song_with(&[("Kick", 1000, 1, 1)], &[(1, 9.0, 1, 3), (1, 13.0, 1, 4)]);
        let changes = diff_placements(&old, &new);
        assert_eq!(changes.len(), 4, "{changes:?}");
        let (removed, added): (Vec<_>, Vec<_>) = changes
            .iter()
            .partition(|c| matches!(c, PlacementChange::Removed { .. }));
        assert_eq!(removed.len(), 2);
        assert_eq!(added.len(), 2);
    }

    /// Found in review: a mutant relaxing the move rule from
    /// `was.len() == 1 && now.len() == 1` to `was.len() >= 1 && now.len()
    /// == 1` survived every existing test. Two placements gone, one new,
    /// for the same family: not a move (which one moved and which one was
    /// simply removed is exactly the ambiguity `docs/FORMATS.md`'s "How the
    /// diff pairs placements" refuses to guess at).
    #[test]
    fn a_two_gone_one_new_pairing_is_not_a_move() {
        let old = song_with(&[("Kick", 1000, 1, 1)], &[(1, 1.0, 1, 1), (1, 5.0, 1, 2)]);
        let new = song_with(&[("Kick", 1000, 1, 1)], &[(1, 9.0, 1, 3)]);
        let changes = diff_placements(&old, &new);
        assert_eq!(changes.len(), 3, "{changes:?}");
        assert!(
            !changes
                .iter()
                .any(|c| matches!(c, PlacementChange::Moved { .. })),
            "{changes:?}"
        );
        let (removed, added): (Vec<_>, Vec<_>) = changes
            .iter()
            .partition(|c| matches!(c, PlacementChange::Removed { .. }));
        assert_eq!(removed.len(), 2);
        assert_eq!(added.len(), 1);
    }

    /// The symmetric mutant: relaxing `now.len() == 1` to `now.len() >= 1`
    /// while leaving `was.len() == 1` alone. One placement gone, two new,
    /// for the same family: still not a move, for the same reason as
    /// above.
    #[test]
    fn a_one_gone_two_new_pairing_is_not_a_move() {
        let old = song_with(&[("Kick", 1000, 1, 1)], &[(1, 1.0, 1, 1)]);
        let new = song_with(&[("Kick", 1000, 1, 1)], &[(1, 9.0, 1, 3), (1, 13.0, 1, 4)]);
        let changes = diff_placements(&old, &new);
        assert_eq!(changes.len(), 3, "{changes:?}");
        assert!(
            !changes
                .iter()
                .any(|c| matches!(c, PlacementChange::Moved { .. })),
            "{changes:?}"
        );
        let (removed, added): (Vec<_>, Vec<_>) = changes
            .iter()
            .partition(|c| matches!(c, PlacementChange::Removed { .. }));
        assert_eq!(removed.len(), 1);
        assert_eq!(added.len(), 2);
    }

    #[test]
    fn a_region_replaced_by_another_family_is_removed_plus_added_not_moved() {
        let old = song_with(
            &[("Snare", 1000, 1, 2), ("Clap", 1000, 2, 3)],
            &[(1, 5.0, 2, 1)],
        );
        let new = song_with(
            &[("Snare", 1000, 1, 2), ("Clap", 1000, 2, 3)],
            &[(1, 9.0, 3, 1)],
        );
        let changes = diff_placements(&old, &new);
        assert_eq!(changes.len(), 2);
        assert!(
            matches!(&changes[0], PlacementChange::Removed { subject, .. } if subject.stem == "Snare")
        );
        assert!(
            matches!(&changes[1], PlacementChange::Added { subject, .. } if subject.stem == "Clap")
        );
    }

    /// Found in review: resolving a `Removed` placement's subject against
    /// the *new* song (rather than the old one) survived every existing
    /// test, because none of them renamed a region in the same save-pair
    /// its placement disappeared from. This one does: family 5's only
    /// region object is renamed from "Take 1" to "Chorus" and its
    /// placement is removed in the same pair, so a `Removed` naming the
    /// *new* name would be wrong — the removal happened to the region
    /// still called "Take 1" at that moment.
    #[test]
    fn a_removed_placements_subject_is_resolved_against_the_old_song() {
        let old = song_with(&[("Take 1", 1000, 1, 5)], &[(1, 1.0, 5, 1)]);
        let new = song_with(&[("Chorus", 1000, 1, 5)], &[]);
        let changes = diff_placements(&old, &new);
        assert_eq!(changes.len(), 1);
        assert!(matches!(
            &changes[0],
            PlacementChange::Removed { subject, .. } if subject.stem == "Take 1"
        ));
    }

    #[test]
    fn only_the_placement_that_moved_is_reported() {
        let old = song_with(&[("Loop", 235200, 1, 2)], &[(1, 1.0, 2, 1), (1, 5.0, 2, 2)]);
        let new = song_with(&[("Loop", 235200, 1, 2)], &[(1, 1.0, 2, 1), (1, 9.0, 2, 2)]);
        let changes = diff_placements(&old, &new);
        assert_eq!(changes.len(), 1);
        assert!(
            matches!(&changes[0], PlacementChange::Moved { subject, .. } if subject.stem == "Loop")
        );
    }

    #[test]
    fn a_move_within_a_multi_copy_family_does_not_claim_which_copy() {
        let regions: &[(&str, u32, u8, u32)] = &[
            ("Silky Acid Bass", 651323, 1, 8),
            ("Silky Acid Bass.1", 651323, 2, 8),
        ];
        let old = song_with(regions, &[(4, 1.0, 8, 1)]);
        let new = song_with(regions, &[(4, 9.0, 8, 1)]);
        let changes = diff_placements(&old, &new);
        assert_eq!(changes.len(), 1);
        assert!(matches!(
            &changes[0],
            PlacementChange::Moved { subject, .. }
                if subject.stem == "Silky Acid Bass" && subject.ambiguous
        ));
    }

    #[test]
    fn an_added_placement_is_reported_without_pairing_guesses() {
        let regions: &[(&str, u32, u8, u32)] = &[("Loop", 235200, 1, 2)];
        let old = song_with(regions, &[(3, 1.0, 2, 1)]);
        let new = song_with(regions, &[(3, 1.0, 2, 1), (3, 9.0, 2, 2)]);
        let changes = diff_placements(&old, &new);
        assert_eq!(changes.len(), 1);
        assert!(
            matches!(&changes[0], PlacementChange::Added { subject, .. } if subject.stem == "Loop")
        );
    }

    #[test]
    fn a_family_with_no_region_object_is_reported_as_unresolved_not_a_guess() {
        // GarageBand writes no `gRuA` records at all, so every placement's
        // family is unresolved there — this is the shape that must never
        // reach a musician as if `<unresolved family N>` were a real name.
        let song = song_with(&[], &[(1, 1.0, 4, 1)]);
        assert_eq!(family_label(&song, 4), "<unresolved family 4>");
        let old = Song::default();
        let changes = diff_placements(&old, &song);
        assert_eq!(changes.len(), 1);
        assert!(matches!(
            &changes[0],
            PlacementChange::Added { subject, .. } if !subject.resolved
        ));
    }

    /// Found in review: a duplicate placement (same track/position/family
    /// twice) only partially cancelling — the surviving copy of the pair
    /// must still be reported once, not zero times, which a set-based
    /// dedup (instead of a true multiset) would get wrong.
    #[test]
    fn a_duplicate_placement_only_partially_cancels() {
        let regions: &[(&str, u32, u8, u32)] = &[("Loop", 1, 1, 2)];
        let old = song_with(regions, &[(1, 1.0, 2, 1), (1, 1.0, 2, 2)]);
        let new = song_with(regions, &[(1, 1.0, 2, 1)]);
        let changes = diff_placements(&old, &new);
        assert_eq!(changes.len(), 1, "{changes:?}");
        assert!(matches!(
            &changes[0],
            PlacementChange::Removed { subject, track: 1, position }
                if subject.stem == "Loop" && *position == REGION_TIME_ORIGIN
        ));
    }

    /// Found in review: a sort key that drops `position` (e.g. sorting only
    /// by `(track, category)`) would still pass every other test here,
    /// since none of them puts two events on the same track needing
    /// position to order them.
    #[test]
    fn removed_events_on_one_track_sort_by_position_ascending() {
        let regions: &[(&str, u32, u8, u32)] = &[("A", 1, 1, 1), ("B", 1, 2, 2)];
        let old = song_with(regions, &[(1, 9.0, 1, 1), (1, 5.0, 2, 2)]);
        let new = song_with(regions, &[]);
        let changes = diff_placements(&old, &new);
        assert_eq!(changes.len(), 2, "{changes:?}");
        let positions: Vec<u32> = changes
            .iter()
            .map(|c| match c {
                PlacementChange::Removed { position, .. } => *position,
                other => panic!("expected Removed, got {other:?}"),
            })
            .collect();
        assert!(
            positions[0] < positions[1],
            "must sort ascending by position: {positions:?}"
        );
    }

    #[test]
    fn two_identical_saves_report_no_change() {
        let song = song_with(&[("Loop", 235200, 1, 2)], &[(1, 1.0, 2, 1)]);
        assert!(diff_placements(&song, &song).is_empty());
    }

    // ---------------------------------------------------------------- //
    // format_bar — exact, never rounded
    // ---------------------------------------------------------------- //

    #[test]
    fn format_bar_is_exact() {
        let cases: &[(u32, &str)] = &[
            (0, "1"),
            (8 * 3840, "9"),
            (54 * 3840 + 1920, "55.5"),
            (54 * 3840 + 2880, "55.75"),
            (65 * 3840 + 3200, "66 5/6"),
            (99 * 3840 + 1, "100 1/3840"),
        ];
        for (tick, text) in cases {
            assert_eq!(format_bar(REGION_TIME_ORIGIN + tick), *text, "tick={tick}");
        }
    }
}
