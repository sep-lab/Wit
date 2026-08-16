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
    let mut out = Vec::with_capacity(RECORD_HEADER_LEN + payload.len());
    out.extend_from_slice(tag);
    out.extend_from_slice(&[0u8; 0x1c - 4]);
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

/// Build a complete `ProjectData` file. The first record is always `gnoS`,
/// as it is on every real file.
pub fn build_project_data(spec: &SongSpec, version: [u8; 2]) -> Vec<u8> {
    let mut body = record(b"gnoS", &gnos_payload(spec));
    for name in &spec.track_names {
        body.extend_from_slice(&record(b"qeSM", &len_prefixed_at(QESM_NAME_OFFSET, name)));
    }
    for name in &spec.region_names {
        body.extend_from_slice(&record(b"gRuA", &len_prefixed_at(GRUA_NAME_OFFSET, name)));
    }
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
        // 1 gnoS + 2 qeSM + 1 gRuA + 1 lFuA. walk_records only returns Ok
        // when it lands exactly on EOF, so this also asserts the framing.
        assert_eq!(wit_logic::walk_records(&data).unwrap().len(), 5);
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
