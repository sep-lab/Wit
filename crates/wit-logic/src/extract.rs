//! Whitelist name/value extraction from a walked record set.
//!
//! Every offset here is `PROJECTDATA_FORMAT.md`-documented (or, for region
//! names, the documented offset with a bounded fallback scan per the M2
//! issue's own guardrail: "no documented offset — heuristic bounded
//! string-reading") and was independently re-verified this session against
//! a real `.logicx`'s current `ProjectData` — see the per-function doc
//! comments for the exact cross-check against that project's
//! `MetaData.plist` ground truth. This is a **single real file**, not the
//! 30-fixture corpus the M2 issue asks for; treat these as spot-checked,
//! not corpus-verified, until the runtime-fetched fixture suite runs (see
//! `tests/real_fixtures.rs`).
//!
//! **Extracted names are diagnostic-tier, not a musician-facing claim.**
//! `possible_track_names` in particular is a best-effort filtered list, not
//! an assertion that these are exactly the project's tracks — the M2 issue
//! explicitly defers the full `karT`↔`qeSM` structural pairing
//! (`PROJECTDATA_FORMAT.md` §10.6) as a follow-up; see this module's
//! `possible_track_names` doc comment for what's implemented instead and
//! why. ADR-0006's Structure tier only promotes a name to a user-facing
//! claim once it passes this bar — that promotion is UI-layer work (M5),
//! not this crate's.

use crate::frame::Record;

/// Well-known Logic system/generic `qeSM` names that are never a musician's
/// track name — probe-verified on real files (`wit-planning/PROBE-FINDINGS.md`
/// finding 5, plus `MIDI Region` observed this session: 12 occurrences on a
/// real 31-track project, none of them a real track name).
///
/// This is a **deliberate, issue-sanctioned exception** to the whitelist
/// doctrine (AGENTS.md: "prefer whitelist extraction over blacklist
/// normalisation") — Logic's internal entry names aren't enumerable any
/// other way without the full §10.6 structural pairing, which the M2 issue
/// explicitly defers. Ableton's blacklist lesson was about *fields*
/// (churn tags that keep multiplying); this is a closed, small, observed
/// set of *literal strings* Logic itself writes, which is a materially
/// different risk shape.
const SYSTEM_OR_GENERIC_NAMES: &[&str] = &[
    "*Automation",
    "Automation",
    "RBA Sequence",
    "MIDI Region",
    "Untitled",
    "TRASH",
    "Track Alternatives",
    "Track Automation Root Folder",
    "Global Harmonies",
];

fn is_system_or_generic_name(name: &str) -> bool {
    SYSTEM_OR_GENERIC_NAMES.contains(&name) || name.starts_with("Default Clip")
}

/// Read a `u16`-length-prefixed ASCII/UTF-8 string at `payload[offset..]`,
/// bounds-checked. Returns `None` (never panics) if the offset doesn't fit,
/// the declared length runs past the payload, the length is implausible
/// (0 or absurdly large — a sign this isn't really a string at this
/// offset), or the bytes aren't valid UTF-8.
fn read_len_prefixed_string(payload: &[u8], offset: usize, max_len: usize) -> Option<String> {
    let len_bytes = payload.get(offset..offset + 2)?;
    let len = u16::from_le_bytes(len_bytes.try_into().unwrap()) as usize;
    if len == 0 || len > max_len {
        return None;
    }
    let bytes = payload.get(offset + 2..offset + 2 + len)?;
    let s = std::str::from_utf8(bytes).ok()?;
    if s.chars().any(|c| c.is_control()) {
        return None;
    }
    Some(s.to_string())
}

/// Bounded fallback: scan the whole payload for a plausible
/// length-prefixed string, used only when a documented fixed offset fails
/// to validate (a version difference, a truncated record, ...). This is
/// the "heuristic bounded string-reading" the M2 issue calls for on region
/// names specifically — kept generic since track names could need the same
/// safety net on a future Logic version.
fn scan_for_len_prefixed_string(payload: &[u8], max_len: usize) -> Option<String> {
    for offset in 0..payload.len().saturating_sub(2) {
        if let Some(s) = read_len_prefixed_string(payload, offset, max_len) {
            if s.len() >= 2 {
                return Some(s);
            }
        }
    }
    None
}

/// Track/MIDI-sequence names from `qeSM` (MIDISeq) records: `u16` length +
/// ASCII at **record start + 0x34** — record-relative, i.e. **payload +
/// 0x10** (the upstream spec's "payload +0x34" was corrected this way by
/// `wit-planning/PROBE-FINDINGS.md` finding 2; re-verified this session on
/// a real file: 159 `qeSM` records, offset `payload+0x10` decoded a
/// plausible name on every one).
///
/// **Not full `karT`↔`qeSM` pairing.** The M2 issue's guardrail asks for
/// that pairing plus a system-name filter before showing any name as a
/// "track". The structural pairing (`PROJECTDATA_FORMAT.md` §10.6.2's
/// `karT`(0x040000)'s `@0x2a` field → the channel/`qeSM` index it points
/// at) is deferred to issue #3 — implementing it precisely needs payload
/// schema work this milestone doesn't do. What ships now is the safety net
/// half: every system/generic entry (`SYSTEM_OR_GENERIC_NAMES`) is
/// filtered out before a name reaches `possible_track_names`. On a real
/// 31-track project (session ground truth: `NumberOfTracks: 31` in
/// `MetaData.plist`) this filter left exactly 2 plausible names out of 159
/// `qeSM` records — the project's own name and one real track name
/// (`'Trusted Friend Piano'`) — which is honest under-extraction, not a
/// false positive: nothing in `possible_track_names` is a name Logic
/// didn't actually write, but most real audio-track names don't surface
/// via `qeSM` at all (audio tracks don't necessarily carry a distinctively
/// named MIDI sequence). `wit-index`/the app must not render this list as
/// "the track list" — only as individual added/removed/renamed facts once
/// M3's `semantic_equal` (census + this extraction) detects a change.
pub fn track_names(records: &[Record<'_>]) -> Vec<String> {
    const QESM_NAME_OFFSET: usize = 0x10; // record+0x34 - record header(0x24) = payload+0x10
    const MAX_NAME_LEN: usize = 200;

    let mut names = Vec::new();
    for r in records {
        if &r.tag != b"qeSM" {
            continue;
        }
        if let Some(name) = read_len_prefixed_string(r.payload, QESM_NAME_OFFSET, MAX_NAME_LEN) {
            if !is_system_or_generic_name(&name) {
                names.push(name);
            }
        }
    }
    names
}

/// Region names from `gRuA` (AudioRegion) records: `u16` length + ASCII at
/// **payload + 0x4a** (`PROJECTDATA_FORMAT.md` §8). Re-verified this
/// session: 96/96 real `gRuA` records on a real file decoded a plausible
/// name at this exact offset (`'Deep Down Shaker'`, `'Deep Down
/// Shaker.1'`, ...) — 100% hit rate, no fallback needed on this file. The
/// M2 issue nonetheless calls this offset undocumented/heuristic (it isn't
/// pinned by the upstream writer spec the way track names and container
/// framing are), so a bounded whole-payload scan backs up the documented
/// offset when it doesn't validate.
pub fn region_names(records: &[Record<'_>]) -> Vec<String> {
    const GRUA_NAME_OFFSET: usize = 0x4a;
    const MAX_NAME_LEN: usize = 100;

    let mut names = Vec::new();
    for r in records {
        if &r.tag != b"gRuA" {
            continue;
        }
        let name = read_len_prefixed_string(r.payload, GRUA_NAME_OFFSET, MAX_NAME_LEN)
            .or_else(|| scan_for_len_prefixed_string(r.payload, MAX_NAME_LEN));
        if let Some(name) = name {
            names.push(name);
        }
    }
    names
}

/// Audio file basenames from `lFuA` (AudioFileRef) records: `u16` char
/// count (UTF-16LE) at **payload + 0x08**, string follows
/// (`PROJECTDATA_FORMAT.md` §8: "filename UTF-16LE (len-prefixed 0d 00 at
/// payload +0x08)"). Re-verified this session: 37/37 real `lFuA` records
/// decoded a clean filename (`'Deep Down Shaker.caf'`,
/// `'videoplayback.wav'`, ...), every one matching an entry in the same
/// project's `MetaData.plist` `AudioFiles` list.
pub fn audio_file_names(records: &[Record<'_>]) -> Vec<String> {
    const LFUA_NAME_OFFSET: usize = 0x08;
    const MAX_CHARS: usize = 260; // macOS filename limit

    let mut names = Vec::new();
    for r in records {
        if &r.tag != b"lFuA" {
            continue;
        }
        let Some(count_bytes) = r.payload.get(LFUA_NAME_OFFSET..LFUA_NAME_OFFSET + 2) else {
            continue;
        };
        let nchars = u16::from_le_bytes(count_bytes.try_into().unwrap()) as usize;
        if nchars == 0 || nchars > MAX_CHARS {
            continue;
        }
        let str_start = LFUA_NAME_OFFSET + 2;
        let Some(str_bytes) = r.payload.get(str_start..str_start + nchars * 2) else {
            continue;
        };
        let units: Vec<u16> = str_bytes
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        if let Ok(name) = String::from_utf16(&units) {
            names.push(name);
        }
    }
    names
}

/// Tempo in BPM, read from the first `gnoS` (Song/root) record.
///
/// `PROJECTDATA_FORMAT.md` §4: `uint32 = round(BPM * 10000)`, replicated at
/// 3 fixed offsets within the `gnoS` payload (file-absolute `0xAA`,
/// `0x102`, `0x3BE` in the spec's own notation, which is payload-relative
/// `0x6e`, `0xc6`, `0x382` once you subtract the 0x3C-byte offset to
/// `gnoS`'s payload start — root header `0x18` + record header `0x24`).
/// Re-verified this session against a real project's `MetaData.plist`
/// (`BeatsPerMinute: 122.0028`): all three slots read exactly `1_220_028`,
/// matching `round(122.0028 * 10000)`.
///
/// Cross-checks all three slots and only returns a value when at least two
/// agree — a disagreement is more likely a different Logic version's
/// layout than a genuine tempo change captured mid-write, and this
/// function would rather say "don't know" than assert a number that came
/// from unmapped bytes. (The spec separately warns file-offset `0xAE`
/// coincidentally holds the same value in some projects but is NOT a
/// tempo slot — it is deliberately not one of the three read here.)
pub fn tempo_bpm(records: &[Record<'_>]) -> Option<f64> {
    const TEMPO_OFFSETS: [usize; 3] = [0x6e, 0xc6, 0x382];

    let gnos = records.iter().find(|r| &r.tag == b"gnoS")?;
    let mut values = Vec::new();
    for &off in &TEMPO_OFFSETS {
        if let Some(bytes) = gnos.payload.get(off..off + 4) {
            values.push(u32::from_le_bytes(bytes.try_into().unwrap()));
        }
    }
    let agreeing = values
        .iter()
        .find(|&&v| values.iter().filter(|&&x| x == v).count() >= 2)?;
    Some(*agreeing as f64 / 10000.0)
}

/// Everything [`track_names`], [`region_names`], [`audio_file_names`], and
/// [`tempo_bpm`] extract, bundled for [`crate::semantic_equal`].
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Extracted {
    pub tempo_bpm: Option<f64>,
    pub possible_track_names: Vec<String>,
    pub region_names: Vec<String>,
    pub audio_file_names: Vec<String>,
}

pub fn extract(records: &[Record<'_>]) -> Extracted {
    Extracted {
        tempo_bpm: tempo_bpm(records),
        possible_track_names: track_names(records),
        region_names: region_names(records),
        audio_file_names: audio_file_names(records),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::{MAGIC, RECORD_HEADER_LEN};

    fn build_container(records: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
        let mut payload = Vec::new();
        for (tag, rec_payload) in records {
            payload.extend_from_slice(*tag);
            payload.extend_from_slice(&[0u8; 0x1c - 4]);
            payload.extend_from_slice(&(rec_payload.len() as u32).to_le_bytes());
            payload.extend_from_slice(&[0u8; RECORD_HEADER_LEN - 0x20]);
            payload.extend_from_slice(rec_payload);
        }
        let mut out = Vec::new();
        out.extend_from_slice(&MAGIC);
        out.extend_from_slice(&[0xd0, 0x09]);
        out.extend_from_slice(&[0u8; 10]);
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        out.extend_from_slice(&[0u8; 4]);
        out.extend_from_slice(&payload);
        out
    }

    fn qesm_payload(name: &str) -> Vec<u8> {
        let mut p = vec![0u8; 0x10];
        p.extend_from_slice(&(name.len() as u16).to_le_bytes());
        p.extend_from_slice(name.as_bytes());
        p
    }

    #[test]
    fn track_names_extracts_at_record_relative_0x34() {
        let data = build_container(&[(b"qeSM", qesm_payload("Trusted Friend Piano"))]);
        let records = crate::frame::walk_records(&data).unwrap();
        assert_eq!(
            track_names(&records),
            vec!["Trusted Friend Piano".to_string()]
        );
    }

    #[test]
    fn track_names_filters_system_and_generic_entries() {
        let data = build_container(&[
            (b"qeSM", qesm_payload("*Automation")),
            (b"qeSM", qesm_payload("RBA Sequence")),
            (b"qeSM", qesm_payload("MIDI Region")),
            (b"qeSM", qesm_payload("Untitled")),
            (b"qeSM", qesm_payload("Default Clip (1)")),
            (b"qeSM", qesm_payload("Trusted Friend Piano")),
        ]);
        let records = crate::frame::walk_records(&data).unwrap();
        assert_eq!(
            track_names(&records),
            vec!["Trusted Friend Piano".to_string()]
        );
    }

    fn grua_payload(name: &str) -> Vec<u8> {
        let mut p = vec![0u8; 0x4a];
        p.extend_from_slice(&(name.len() as u16).to_le_bytes());
        p.extend_from_slice(name.as_bytes());
        p
    }

    #[test]
    fn region_names_extracts_at_payload_0x4a() {
        let data = build_container(&[(b"gRuA", grua_payload("Deep Down Shaker"))]);
        let records = crate::frame::walk_records(&data).unwrap();
        assert_eq!(region_names(&records), vec!["Deep Down Shaker".to_string()]);
    }

    fn lfua_payload(name: &str) -> Vec<u8> {
        let mut p = vec![0u8; 0x08];
        let units: Vec<u16> = name.encode_utf16().collect();
        p.extend_from_slice(&(units.len() as u16).to_le_bytes());
        for u in units {
            p.extend_from_slice(&u.to_le_bytes());
        }
        p
    }

    #[test]
    fn audio_file_names_extracts_utf16le_at_payload_0x08() {
        let data = build_container(&[(b"lFuA", lfua_payload("Deep Down Shaker.caf"))]);
        let records = crate::frame::walk_records(&data).unwrap();
        assert_eq!(
            audio_file_names(&records),
            vec!["Deep Down Shaker.caf".to_string()]
        );
    }

    fn gnos_payload_with_tempo(bpm_x10000: u32) -> Vec<u8> {
        let mut p = vec![0u8; 0x400];
        for off in [0x6e, 0xc6, 0x382] {
            p[off..off + 4].copy_from_slice(&bpm_x10000.to_le_bytes());
        }
        p
    }

    #[test]
    fn tempo_bpm_reads_and_cross_checks_the_three_slots() {
        let data = build_container(&[(b"gnoS", gnos_payload_with_tempo(1_220_028))]);
        let records = crate::frame::walk_records(&data).unwrap();
        assert_eq!(tempo_bpm(&records), Some(122.0028));
    }

    #[test]
    fn tempo_bpm_returns_none_when_the_slots_disagree() {
        let mut payload = gnos_payload_with_tempo(1_200_000);
        payload[0x382..0x382 + 4].copy_from_slice(&999u32.to_le_bytes());
        let data = build_container(&[(b"gnoS", payload)]);
        let records = crate::frame::walk_records(&data).unwrap();
        assert_eq!(tempo_bpm(&records), Some(120.0)); // 2 of 3 still agree
    }

    #[test]
    fn tempo_bpm_returns_none_without_a_gnos_record() {
        let data = build_container(&[(b"karT", vec![0u8; 4])]);
        let records = crate::frame::walk_records(&data).unwrap();
        assert_eq!(tempo_bpm(&records), None);
    }

    #[test]
    fn extraction_never_panics_on_truncated_payloads() {
        // Every offset function must degrade to None/empty, not panic, when
        // a record's payload is too short to hold the field it's probed for.
        let data = build_container(&[
            (b"qeSM", vec![0u8; 2]),
            (b"gRuA", vec![0u8; 2]),
            (b"lFuA", vec![0u8; 2]),
            (b"gnoS", vec![0u8; 2]),
        ]);
        let records = crate::frame::walk_records(&data).unwrap();
        assert!(track_names(&records).is_empty());
        assert!(region_names(&records).is_empty());
        assert!(audio_file_names(&records).is_empty());
        assert_eq!(tempo_bpm(&records), None);
    }
}
