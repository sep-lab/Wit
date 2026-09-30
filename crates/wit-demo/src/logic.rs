//! Synthesise a `ProjectData` container byte-for-byte in the shape
//! `wit-logic` walks.
//!
//! Every offset used here is the one `wit-logic` reads, and the two must
//! stay in lockstep — that is deliberate. If someone changes an extraction
//! offset in `wit-logic` without changing it here, this crate's round-trip
//! tests fail, which is exactly the alarm you want: the demo library is the
//! thing a pilot user sees first, and a demo that silently stops producing
//! readable names would be worse than no demo.
//!
//! What this is **not**: a Logic file writer. The records carry no real
//! payload schema (issue #3 is still open), only the whitelisted fields
//! `wit-logic` extracts, padded with zeros. Logic itself would not open one
//! of these, and nothing here should ever be presented as if it would.
//!
//! **Regions and placements** (PLAN-V2 Logic lane): each `region_names`
//! entry becomes both a `gRuA` region object (sized to `wit-logic::regions`'
//! exact size law, with a family index, a length, and a UUID) and one
//! placement in a single `qSvE` event stream — indexed by the name's
//! position in `region_names`, so a name keeps the same family/UUID/position
//! across saves as the list only ever grows, and the placement diff sees
//! "added", never a spurious move. This is the same anti-coupling choice as
//! the rest of the file: offsets are copied here, never imported from
//! `wit-logic`, so a drift between the two fails a round-trip test instead
//! of silently agreeing with itself.
//!
//! **`MetaData.plist`** (`build_metadata_plist`) carries the same whitelisted
//! facts `wit-logic::metadata` reads: track count (derived from
//! `track_names.len()`, not a container count), tempo, and the audio file
//! list. Key, mode and time signature are fixed ("C major", 4/4) — the demo
//! chain doesn't model a key or meter change, so varying them would claim a
//! fact the chain doesn't tell a story about.

/// `d0 09` on real Logic files, `c5 09` on real GarageBand — both observed
/// by the probe (`wit-planning/PROBE-FINDINGS.md`). The walker accepts any
/// version word; using the real two keeps the demo honest about which app
/// each bundle is pretending to be.
pub const VERSION_LOGIC: [u8; 2] = [0xd0, 0x09];
pub const VERSION_GARAGEBAND: [u8; 2] = [0xc5, 0x09];

const MAGIC: [u8; 4] = [0x23, 0x47, 0xC0, 0xAB];
const ROOT_HEADER_LEN: usize = 0x18;
const RECORD_HEADER_LEN: usize = 0x24;

// Offsets `wit-logic::extract` reads. Named here rather than inlined so the
// coupling is greppable from both sides.
const QESM_NAME_OFFSET: usize = 0x10;
const GRUA_NAME_OFFSET: usize = 0x4a;
const LFUA_NAME_OFFSET: usize = 0x08;
const TEMPO_OFFSETS: [usize; 3] = [0x6e, 0xc6, 0x382];
/// An offset inside the `gnoS` payload that nothing reads — not a tempo
/// slot, not a name. Writing here changes the file's bytes while leaving
/// every extracted fact identical, which is how the demo reproduces the
/// measured 28% of real save pairs that are byte-different but
/// structurally identical (EXPERIMENTS.md §11).
const CHURN_OFFSET: usize = 0x200;
const GNOS_PAYLOAD_LEN: usize = 0x400;

// Offsets `wit-logic::regions` reads (record-relative there; payload here,
// same convention `GRUA_NAME_OFFSET` above already uses: payload = record
// offset - RECORD_HEADER_LEN). Named to match that module's own constant
// names so the coupling is greppable in both directions.
const IDX_FIELD_OFFSET: usize = 0x08; // header offset (record-relative, not payload)
const IDX_SHIFT: u32 = 18;
const REGION_LENGTH_OFFSET: usize = 0x16; // payload-relative (record 0x3A - 0x24)
const REGION_UUID_OFFSET_IN_SUFFIX: usize = 0x56;
const REGION_FIXED_SUFFIX_LEN: usize = 133;
const REGION_UUID_LEN: usize = 16;

const PLACEMENT_MARKER: [u8; 4] = [0x24, 0x00, 0x00, 0x00];
const PLACEMENT_POSITION_OFFSET: usize = 0x04;
const PLACEMENT_EVENT_ID_OFFSET: usize = 0x10;
const PLACEMENT_TRACK_OFFSET: usize = 0x14;
const PLACEMENT_LINK_OFFSET: usize = 0x2C;
const PLACEMENT_GROUP_LEN: usize = 0x30;
const EVENT_LEN: usize = 16;
const EVENT_TYPE_BYTE: usize = 7;
const REGION_TIME_ORIGIN: u32 = 34560;
const TICKS_PER_BAR: u32 = 3840;

/// One save of a synthetic song — the whitelisted facts `wit-logic` can
/// actually see, plus a churn counter for bytes it cannot.
#[derive(Debug, Clone, PartialEq)]
pub struct SongSpec {
    pub tempo_bpm: f64,
    /// Becomes `qeSM` records. Note `wit-logic` filters system/generic
    /// names, so avoid those here unless testing the filter.
    pub track_names: Vec<String>,
    /// Becomes `gRuA` records.
    pub region_names: Vec<String>,
    /// Becomes `lFuA` records (UTF-16LE on disk).
    pub audio_file_names: Vec<String>,
    /// Bumped on a save that changed nothing Wit can see. Alters the bytes
    /// and nothing else.
    pub churn: u32,
}

impl SongSpec {
    /// A save that differs from this one only in bytes — the "you moved a
    /// fader Wit can't see, or Logic rewrote a UUID" case.
    pub fn with_churn(&self, churn: u32) -> SongSpec {
        SongSpec {
            churn,
            ..self.clone()
        }
    }
}

fn record(tag: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    record_with_idx(tag, 0, payload)
}

/// A record with a value in the header's `+0x08` `idx` field — `gRuA`'s
/// `familyIndex << 18` (`docs/FORMATS.md`). Every other record this file
/// writes uses `idx = 0` via [`record`], since nothing else reads that
/// field.
fn record_with_idx(tag: &[u8; 4], idx: u32, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(RECORD_HEADER_LEN + payload.len());
    out.extend_from_slice(tag);
    out.extend_from_slice(&[0u8; IDX_FIELD_OFFSET - 4]);
    out.extend_from_slice(&idx.to_le_bytes());
    out.extend_from_slice(&[0u8; 0x1c - (IDX_FIELD_OFFSET + 4)]);
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(&[0u8; RECORD_HEADER_LEN - 0x20]);
    out.extend_from_slice(payload);
    out
}

fn len_prefixed_at(offset: usize, name: &str) -> Vec<u8> {
    let mut p = vec![0u8; offset];
    p.extend_from_slice(&(name.len() as u16).to_le_bytes());
    p.extend_from_slice(name.as_bytes());
    p
}

fn utf16_len_prefixed_at(offset: usize, name: &str) -> Vec<u8> {
    let mut p = vec![0u8; offset];
    let units: Vec<u16> = name.encode_utf16().collect();
    p.extend_from_slice(&(units.len() as u16).to_le_bytes());
    for u in units {
        p.extend_from_slice(&u.to_le_bytes());
    }
    p
}

fn gnos_payload(spec: &SongSpec) -> Vec<u8> {
    let mut p = vec![0u8; GNOS_PAYLOAD_LEN];
    // `round(BPM * 10000)`, replicated at all three slots — wit-logic only
    // trusts a tempo when at least two agree.
    let ticks = (spec.tempo_bpm * 10_000.0).round() as u32;
    for off in TEMPO_OFFSETS {
        p[off..off + 4].copy_from_slice(&ticks.to_le_bytes());
    }
    p[CHURN_OFFSET..CHURN_OFFSET + 4].copy_from_slice(&spec.churn.to_le_bytes());
    p
}

/// A deterministic, distinct-per-index UUID — not a real Logic region UUID
/// shape claim, just enough to be distinct and stable so a name keeps its
/// identity across saves (`wit-logic::regions`' move-pairing needs a stable
/// family per name, not a stable UUID; this is for `region.rs`'s own
/// decode round-trip test only).
fn region_uuid(index: usize) -> [u8; REGION_UUID_LEN] {
    let mut u = [0u8; REGION_UUID_LEN];
    u[0] = (index + 1) as u8;
    u[1..].copy_from_slice(&[
        0x5e, 0xe0, 0x52, 0x64, 0x1c, 0x11, 0xf1, 0x85, 0x4a, 0x20, 0x5d, 0x0d, 0x40, 0xb5, 0x1c,
    ]);
    u
}

/// A `gRuA` region-object payload satisfying `wit-logic::regions`' exact
/// size law (`payload_size == 209 + nlen + (nlen & 1)`), so the region
/// decodes there the same way a real one does — not just the bare
/// len-prefixed name `extract.rs`'s simpler whitelist reads.
fn grua_payload(name: &str, length_frames: u32, uuid: [u8; REGION_UUID_LEN]) -> Vec<u8> {
    let raw = name.as_bytes();
    let name_len = raw.len();
    let padded = name_len + (name_len & 1);
    let name_end = GRUA_NAME_OFFSET + 2 + padded;
    let uuid_at = name_end + REGION_UUID_OFFSET_IN_SUFFIX;
    let total = name_end + REGION_FIXED_SUFFIX_LEN;

    let mut p = vec![0u8; total];
    p[REGION_LENGTH_OFFSET..REGION_LENGTH_OFFSET + 4].copy_from_slice(&length_frames.to_le_bytes());
    p[GRUA_NAME_OFFSET..GRUA_NAME_OFFSET + 2].copy_from_slice(&(name_len as u16).to_le_bytes());
    p[GRUA_NAME_OFFSET + 2..GRUA_NAME_OFFSET + 2 + name_len].copy_from_slice(raw);
    p[uuid_at..uuid_at + REGION_UUID_LEN].copy_from_slice(&uuid);
    p
}

/// One 48-byte audio-placement group, in the exact shape
/// `wit-logic::regions::parse_event_stream` reads: headed by the `24 00 00
/// 00` marker, byte `+7` of its three units `00`/`89`/`bc` (the pattern
/// every real placement group measured carries).
fn placement_group(
    track: u8,
    position: u32,
    family: u32,
    event_id: u32,
) -> [u8; PLACEMENT_GROUP_LEN] {
    let mut g = [0u8; PLACEMENT_GROUP_LEN];
    g[0..4].copy_from_slice(&PLACEMENT_MARKER);
    g[PLACEMENT_POSITION_OFFSET..PLACEMENT_POSITION_OFFSET + 4]
        .copy_from_slice(&position.to_le_bytes());
    g[PLACEMENT_EVENT_ID_OFFSET..PLACEMENT_EVENT_ID_OFFSET + 4]
        .copy_from_slice(&event_id.to_le_bytes());
    g[PLACEMENT_TRACK_OFFSET] = track;
    g[EVENT_LEN + EVENT_TYPE_BYTE] = 0x89;
    g[2 * EVENT_LEN + EVENT_TYPE_BYTE] = 0xBC;
    g[PLACEMENT_LINK_OFFSET..PLACEMENT_LINK_OFFSET + 4]
        .copy_from_slice(&(family * 4).to_le_bytes());
    g
}

/// Build a complete `ProjectData` file. The first record is always `gnoS`,
/// as it is on every real file.
///
/// Each `region_names` entry becomes a `gRuA` region object **and** one
/// placement in a single `qSvE` event stream, both keyed by the name's
/// index in `region_names` — stable across saves as the list only ever
/// grows (see the module doc). Track 1, spaced 4 bars apart: the demo
/// exercises the region wiring, not a claim about which track a region is
/// really on.
pub fn build_project_data(spec: &SongSpec, version: [u8; 2]) -> Vec<u8> {
    let mut body = record(b"gnoS", &gnos_payload(spec));
    for name in &spec.track_names {
        body.extend_from_slice(&record(b"qeSM", &len_prefixed_at(QESM_NAME_OFFSET, name)));
    }

    let mut placements = Vec::new();
    for (index, name) in spec.region_names.iter().enumerate() {
        let family = index as u32;
        let length_frames = 100_000 + family * 1_000;
        body.extend_from_slice(&record_with_idx(
            b"gRuA",
            family << IDX_SHIFT,
            &grua_payload(name, length_frames, region_uuid(index)),
        ));
        let position = REGION_TIME_ORIGIN + family * 4 * TICKS_PER_BAR;
        placements.extend_from_slice(&placement_group(1, position, family, family + 1));
    }
    body.extend_from_slice(&record(b"qSvE", &placements));

    for name in &spec.audio_file_names {
        body.extend_from_slice(&record(
            b"lFuA",
            &utf16_len_prefixed_at(LFUA_NAME_OFFSET, name),
        ));
    }

    let mut out = Vec::with_capacity(ROOT_HEADER_LEN + body.len());
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&version);
    out.extend_from_slice(&[0u8; 10]);
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(&[0u8; 4]);
    out.extend_from_slice(&body);
    out
}

/// A `MetaData.plist` sibling for one save: the same whitelisted facts
/// `wit-logic::metadata::read_metadata_plist` reads. `NumberOfTracks` comes
/// from `track_names.len()` (already a whitelisted count, not a container
/// tally) so it changes exactly when the chain adds or removes a track
/// name. Key, mode and time signature are fixed — see the module doc for
/// why the demo doesn't vary them.
pub fn build_metadata_plist(spec: &SongSpec) -> Vec<u8> {
    let mut dict = plist::Dictionary::new();
    dict.insert(
        "NumberOfTracks".into(),
        plist::Value::Integer((spec.track_names.len() as i64).into()),
    );
    dict.insert("BeatsPerMinute".into(), plist::Value::Real(spec.tempo_bpm));
    dict.insert("SongKey".into(), plist::Value::String("C".into()));
    dict.insert("SongGenderKey".into(), plist::Value::String("major".into()));
    dict.insert(
        "SongSignatureNumerator".into(),
        plist::Value::Integer(4.into()),
    );
    dict.insert(
        "SongSignatureDenominator".into(),
        plist::Value::Integer(4.into()),
    );
    dict.insert("SampleRate".into(), plist::Value::Integer(48_000.into()));
    let audio_files: Vec<plist::Value> = spec
        .audio_file_names
        .iter()
        .map(|name| plist::Value::String(format!("Audio Files/{name}")))
        .collect();
    dict.insert("AudioFiles".into(), plist::Value::Array(audio_files));

    let mut buf = Vec::new();
    plist::Value::Dictionary(dict)
        .to_writer_binary(&mut buf)
        .expect("writing an in-memory plist never fails");
    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> SongSpec {
        SongSpec {
            tempo_bpm: 122.0028,
            track_names: vec!["Rhodes".into(), "Upright Bass".into()],
            region_names: vec!["Verse Rhodes".into()],
            audio_file_names: vec!["Upright Bass.caf".into()],
            churn: 0,
        }
    }

    #[test]
    fn a_generated_file_walks_to_a_clean_eof() {
        let data = build_project_data(&spec(), VERSION_LOGIC);
        let header = wit_logic::parse_root_header(&data).unwrap();
        assert_eq!(header.version_word, VERSION_LOGIC);
        // 1 gnoS + 2 qeSM + 1 gRuA + 1 qSvE + 1 lFuA. walk_records only
        // returns Ok when it lands exactly on EOF, so this also asserts the
        // framing.
        assert_eq!(wit_logic::walk_records(&data).unwrap().len(), 6);
    }

    #[test]
    fn every_whitelisted_field_survives_a_round_trip_through_wit_logic() {
        let spec = spec();
        let data = build_project_data(&spec, VERSION_LOGIC);
        let walked = wit_logic::walk(&data).unwrap();
        assert_eq!(walked.extracted.tempo_bpm, Some(122.0028));
        assert_eq!(walked.extracted.possible_track_names, spec.track_names);
        assert_eq!(walked.extracted.region_names, spec.region_names);
        assert_eq!(walked.extracted.audio_file_names, spec.audio_file_names);
    }

    #[test]
    fn regions_and_placements_decode_through_wit_logics_own_region_map() {
        let spec = spec();
        let data = build_project_data(&spec, VERSION_LOGIC);
        let song = wit_logic::parse_regions_bytes(&data).unwrap();
        assert_eq!(song.regions.len(), 1);
        assert_eq!(song.regions[0].name, "Verse Rhodes");
        assert_eq!(song.placements.len(), 1);
        assert_eq!(song.placements[0].track, 1);
        assert_eq!(song.placements[0].family, 0);
        assert_eq!(
            wit_logic::family_label(&song, song.placements[0].family),
            "Verse Rhodes"
        );
    }

    #[test]
    fn an_added_region_is_an_added_placement_not_a_move() {
        let a = build_project_data(&spec(), VERSION_LOGIC);
        let mut changed = spec();
        changed.region_names.push("Chorus Rhodes".into());
        let b = build_project_data(&changed, VERSION_LOGIC);

        let song_a = wit_logic::parse_regions_bytes(&a).unwrap();
        let song_b = wit_logic::parse_regions_bytes(&b).unwrap();
        // The existing region's family/position must not shift when a new
        // one is appended, or the diff would report a spurious move.
        assert_eq!(song_a.placements[0], song_b.placements[0]);

        let changes = wit_logic::diff_placements(&song_a, &song_b);
        assert_eq!(changes.len(), 1);
        assert!(matches!(
            &changes[0],
            wit_logic::PlacementChange::Added { subject, .. } if subject.stem == "Chorus Rhodes"
        ));
    }

    #[test]
    fn metadata_plist_carries_the_whitelisted_facts() {
        let spec = spec();
        let bytes = build_metadata_plist(&spec);
        let meta = wit_logic::read_metadata_plist_bytes(&bytes).unwrap();
        assert_eq!(meta.number_of_tracks, Some(spec.track_names.len() as u32));
        assert_eq!(meta.bpm, Some(spec.tempo_bpm));
        assert_eq!(meta.audio_files, spec.audio_file_names);
    }

    #[test]
    fn number_of_tracks_in_the_plist_changes_when_a_track_is_added() {
        let mut grown = spec();
        grown.track_names.push("Wurli Pad".into());
        let meta = wit_logic::read_metadata_plist_bytes(&build_metadata_plist(&grown)).unwrap();
        assert_eq!(meta.number_of_tracks, Some(3));
    }

    #[test]
    fn churn_changes_the_bytes_and_nothing_wit_can_see() {
        // The 28%-of-real-pairs case from EXPERIMENTS.md §11, reproduced on
        // demand: different bytes, identical verdict.
        let a = build_project_data(&spec(), VERSION_LOGIC);
        let b = build_project_data(&spec().with_churn(1), VERSION_LOGIC);
        assert_ne!(a, b, "churn must change the bytes");
        assert_eq!(a.len(), b.len());
        let (wa, wb) = (wit_logic::walk(&a).unwrap(), wit_logic::walk(&b).unwrap());
        assert_eq!(
            wit_logic::semantic_equal(&wa, &wb),
            wit_logic::Verdict::NoStructuralChange
        );
    }

    #[test]
    fn adding_a_region_is_a_structural_change() {
        let a = build_project_data(&spec(), VERSION_LOGIC);
        let mut changed = spec();
        changed.region_names.push("Chorus Rhodes".into());
        let b = build_project_data(&changed, VERSION_LOGIC);
        let (wa, wb) = (wit_logic::walk(&a).unwrap(), wit_logic::walk(&b).unwrap());
        assert_eq!(
            wit_logic::semantic_equal(&wa, &wb),
            wit_logic::Verdict::StructuralChange
        );
    }

    #[test]
    fn a_tempo_change_is_visible() {
        let a = build_project_data(&spec(), VERSION_LOGIC);
        let mut faster = spec();
        faster.tempo_bpm = 124.0;
        let b = build_project_data(&faster, VERSION_LOGIC);
        assert_eq!(
            wit_logic::walk(&b).unwrap().extracted.tempo_bpm,
            Some(124.0)
        );
        let (wa, wb) = (wit_logic::walk(&a).unwrap(), wit_logic::walk(&b).unwrap());
        assert_eq!(
            wit_logic::semantic_equal(&wa, &wb),
            wit_logic::Verdict::StructuralChange
        );
    }

    #[test]
    fn generation_is_deterministic() {
        assert_eq!(
            build_project_data(&spec(), VERSION_LOGIC),
            build_project_data(&spec(), VERSION_LOGIC)
        );
    }

    #[test]
    fn a_garageband_file_carries_the_garageband_version_word() {
        let data = build_project_data(&spec(), VERSION_GARAGEBAND);
        assert_eq!(
            wit_logic::parse_root_header(&data).unwrap().version_word,
            VERSION_GARAGEBAND
        );
    }
}
