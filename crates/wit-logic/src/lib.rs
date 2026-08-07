//! Logic Pro / GarageBand `ProjectData` reader — container walk, tag
//! census, and whitelist name/tempo extraction. Read-only: no write-path,
//! no payload-schema interpretation beyond what `PROJECTDATA_FORMAT.md`
//! documents (mapping the rest is issue #3, out of scope here).
//!
//! One walker covers both `.logicx` (Logic) and `.band` (GarageBand) — same
//! magic, same root-record tag (`gnoS`), version word varies (`d009`,
//! `d109`, `c509` all observed on real files; see `frame.rs`).
//!
//! **`semantic_equal` v1 is census + [`Extracted`] equality only**
//! (M2 tracking issue guardrail). Byte comparison
//! ([`bytes_equal`]) is a diagnostic — "did the file change at all" — never
//! the verdict; there is no empirically-discovered volatile-field mask yet
//! to make raw-byte comparison meaningful (a real project's saves 04/05/06
//! were measured with identical size and census but differing bytes — see
//! `daw-format-ground-truth` memory / `wit-planning/PLAN.md`).

mod census;
mod extract;
mod frame;

pub use census::{census, Census};
pub use extract::{audio_file_names, extract, region_names, tempo_bpm, track_names, Extracted};
pub use frame::{parse_root_header, walk_records, Record, RootHeader, WalkError};

/// A fully walked file: its root header, tag census, and extracted names —
/// everything [`semantic_equal`] needs.
#[derive(Debug, Clone, PartialEq)]
pub struct Walked {
    pub root: RootHeader,
    pub census: Census,
    pub extracted: Extracted,
}

/// Walk a `ProjectData` file's raw bytes end to end: parse the root header,
/// walk every record, compute the census, and extract names/tempo.
pub fn walk(data: &[u8]) -> Result<Walked, WalkError> {
    let root = parse_root_header(data)?;
    let records = walk_records(data)?;
    Ok(Walked {
        root,
        census: census::census(&records),
        extracted: extract::extract(&records),
    })
}

/// Walk a `ProjectData` file from disk.
pub fn walk_file(path: &std::path::Path) -> Result<Walked, WalkError> {
    let bytes = std::fs::read(path).map_err(|e| WalkError::Io(e.to_string()))?;
    walk(&bytes)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Census and extracted names/tempo are identical. This does NOT mean
    /// the raw bytes are identical (they very often aren't — see the
    /// module doc) and does NOT mean nothing musically changed (Logic's
    /// per-save UUID/float churn is invisible at this level, and so is any
    /// knob or fader move — the Structure honesty tier's entire point).
    NoStructuralChange,
    /// Census or an extracted field differs between the two walks.
    StructuralChange,
}

/// Compare two walks. **v1 = census + [`Extracted`] equality only** — see
/// the module doc for why byte comparison is deliberately excluded from
/// the verdict.
pub fn semantic_equal(a: &Walked, b: &Walked) -> Verdict {
    if a.census == b.census && a.extracted == b.extracted {
        Verdict::NoStructuralChange
    } else {
        Verdict::StructuralChange
    }
}

/// Count of distinct signals that differ between two walks: one per
/// census tag whose count differs, one per name added to or removed from
/// each of the three [`Extracted`] name lists, and one if tempo differs.
/// **Diagnostic granularity only — not part of the [`Verdict`].**
/// `semantic_equal` stays a strict boolean at v1 (M2 tracking issue
/// guardrail); this exists to answer a different question — *how much*
/// changed on saves that already report `StructuralChange` — for M2.5's
/// "distribution of change counts per save" (issue #15). Guaranteed to be
/// `0` exactly when `semantic_equal` reports `NoStructuralChange`, since
/// both are derived from the same census/extracted equality checks.
pub fn change_count(a: &Walked, b: &Walked) -> usize {
    let mut count = 0usize;
    let tags: std::collections::BTreeSet<&String> =
        a.census.keys().chain(b.census.keys()).collect();
    for tag in tags {
        if a.census.get(tag).copied().unwrap_or(0) != b.census.get(tag).copied().unwrap_or(0) {
            count += 1;
        }
    }
    count += symmetric_diff_count(
        &a.extracted.possible_track_names,
        &b.extracted.possible_track_names,
    );
    count += symmetric_diff_count(&a.extracted.region_names, &b.extracted.region_names);
    count += symmetric_diff_count(&a.extracted.audio_file_names, &b.extracted.audio_file_names);
    if a.extracted.tempo_bpm != b.extracted.tempo_bpm {
        count += 1;
    }
    count
}

fn symmetric_diff_count(a: &[String], b: &[String]) -> usize {
    let sa: std::collections::BTreeSet<&String> = a.iter().collect();
    let sb: std::collections::BTreeSet<&String> = b.iter().collect();
    sa.symmetric_difference(&sb).count()
}

/// Raw byte comparison — a diagnostic only ("did the file change at all"),
/// never fed into [`semantic_equal`]'s verdict. Exposed because `wit logic
/// probe` reports it alongside the structural verdict, honestly labeled as
/// a separate fact.
pub fn bytes_equal(a: &[u8], b: &[u8]) -> bool {
    a == b
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_container(records: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
        let mut payload = Vec::new();
        for (tag, rec_payload) in records {
            payload.extend_from_slice(*tag);
            payload.extend_from_slice(&[0u8; 0x1c - 4]);
            payload.extend_from_slice(&(rec_payload.len() as u32).to_le_bytes());
            payload.extend_from_slice(&[0u8; frame::RECORD_HEADER_LEN - 0x20]);
            payload.extend_from_slice(rec_payload);
        }
        let mut out = Vec::new();
        out.extend_from_slice(&frame::MAGIC);
        out.extend_from_slice(&[0xd0, 0x09]);
        out.extend_from_slice(&[0u8; 10]);
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        out.extend_from_slice(&[0u8; 4]);
        out.extend_from_slice(&payload);
        out
    }

    #[test]
    fn identical_saves_walk_to_no_structural_change() {
        let data_a = build_container(&[(b"karT", vec![0u8; 8])]);
        let data_b = data_a.clone();
        let a = walk(&data_a).unwrap();
        let b = walk(&data_b).unwrap();
        assert_eq!(semantic_equal(&a, &b), Verdict::NoStructuralChange);
    }

    #[test]
    fn byte_different_but_census_and_names_identical_is_still_no_structural_change() {
        // Mirrors the real-world case (PLAN.md): saves 04/05/06 had
        // identical size AND census yet differed in bytes (a regenerated
        // plugin-state UUID). Simulate by padding one record's payload
        // with different-but-same-length filler bytes.
        let data_a = build_container(&[(b"karT", vec![0xAA; 8])]);
        let data_b = build_container(&[(b"karT", vec![0xBB; 8])]);
        assert_ne!(data_a, data_b);
        let a = walk(&data_a).unwrap();
        let b = walk(&data_b).unwrap();
        assert_eq!(semantic_equal(&a, &b), Verdict::NoStructuralChange);
        assert!(!bytes_equal(&data_a, &data_b));
    }

    #[test]
    fn a_new_record_tag_is_a_structural_change() {
        let data_a = build_container(&[(b"karT", vec![0u8; 8])]);
        let data_b = build_container(&[(b"karT", vec![0u8; 8]), (b"gRuA", vec![0u8; 8])]);
        let a = walk(&data_a).unwrap();
        let b = walk(&data_b).unwrap();
        assert_eq!(semantic_equal(&a, &b), Verdict::StructuralChange);
    }

    #[test]
    fn version_word_never_affects_the_verdict() {
        // Accept every version word (frame.rs) — two saves that differ
        // ONLY in version word but are otherwise structurally identical
        // must still report NoStructuralChange.
        let mut data_a = build_container(&[(b"karT", vec![0u8; 8])]);
        let mut data_b = data_a.clone();
        data_a[4..6].copy_from_slice(&[0xd0, 0x09]);
        data_b[4..6].copy_from_slice(&[0xc5, 0x09]);
        let a = walk(&data_a).unwrap();
        let b = walk(&data_b).unwrap();
        assert_ne!(a.root.version_word, b.root.version_word);
        assert_eq!(semantic_equal(&a, &b), Verdict::NoStructuralChange);
    }

    #[test]
    fn change_count_is_zero_exactly_when_no_structural_change() {
        let data_a = build_container(&[(b"karT", vec![0xAA; 8])]);
        let data_b = build_container(&[(b"karT", vec![0xBB; 8])]);
        let a = walk(&data_a).unwrap();
        let b = walk(&data_b).unwrap();
        assert_eq!(semantic_equal(&a, &b), Verdict::NoStructuralChange);
        assert_eq!(change_count(&a, &b), 0);
    }

    #[test]
    fn change_count_counts_one_per_differing_census_tag() {
        let data_a = build_container(&[(b"karT", vec![0u8; 8])]);
        let data_b = build_container(&[(b"karT", vec![0u8; 8]), (b"gRuA", vec![0u8; 8])]);
        let a = walk(&data_a).unwrap();
        let b = walk(&data_b).unwrap();
        assert_eq!(semantic_equal(&a, &b), Verdict::StructuralChange);
        // Only "gRuA" changed count (0 -> 1); "karT" is unchanged (1 -> 1).
        assert_eq!(change_count(&a, &b), 1);
    }

    #[test]
    fn change_count_is_symmetric() {
        let data_a = build_container(&[(b"karT", vec![0u8; 8])]);
        let data_b = build_container(&[(b"karT", vec![0u8; 8]), (b"gRuA", vec![0u8; 8])]);
        let a = walk(&data_a).unwrap();
        let b = walk(&data_b).unwrap();
        assert_eq!(change_count(&a, &b), change_count(&b, &a));
    }
}
