//! `ProjectData` container framing — the 24-byte root header and the flat
//! sequence of 36-byte-header records that follow it, to EOF.
//!
//! Transcribed from `jonkubis/LogicProFormatWriter`'s `PROJECTDATA_FORMAT.md`
//! §2 (MIT), pinned at SHA `1f77c5c37d49ccd9551cc8e9107750e8db2f1fed`
//! (`wit-planning/PROBE-FINDINGS.md`), and independently re-verified this
//! session against 10 real files (a real `.logicx`'s current `ProjectData`
//! plus its 9 on-disk `Project File Backups/00`–`08`, all version word
//! `d009`): every one walks to a byte-exact clean EOF. The probe additionally
//! confirmed the framing on Logic `d109` and GarageBand `c509` — more
//! version-word variants than `PROJECTDATA_FORMAT.md` documents. Accept every
//! version word; refuse only on an actual framing violation (bad magic, a
//! length field that doesn't match, or a record whose declared size runs
//! past EOF) — never on an unrecognized version word.
//!
//! ```text
//! ROOT FRAME (offset 0), 24-byte header:
//!   +0x00  4   magic 23 47 C0 AB
//!   +0x04  2   version word (varies — d009, d109, c509 all observed; accept all)
//!   +0x06  10  (stable, unvalidated — see PROJECTDATA_FORMAT.md §2)
//!   +0x10  4   uint32 LENGTH = filesize - 24        (little-endian)
//!   +0x14  4   (unvalidated)
//!   +0x18 ...  PAYLOAD = a flat sequence of RECORDS, to EOF
//!
//! Every RECORD (the first is always `gnoS`, the Song/root record):
//!   +0x00  4   tag (ASCII, e.g. "gnoS", "karT" — already the on-disk form)
//!   +0x04 ...  header fields, mostly unmapped
//!   +0x1c  4   uint32 PAYLOAD SIZE
//!   +0x24 ...  payload (record total size = 0x24 + payload_size)
//! ```

use std::fmt;

pub const MAGIC: [u8; 4] = [0x23, 0x47, 0xC0, 0xAB];
pub const ROOT_HEADER_LEN: usize = 0x18;
pub const RECORD_HEADER_LEN: usize = 0x24;
const LENGTH_FIELD_OFFSET: usize = 0x10;
const PAYLOAD_SIZE_FIELD_OFFSET: usize = 0x1c;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WalkError {
    /// Fewer than 24 bytes — can't even hold a root header.
    TooShortForRootHeader { len: usize },
    /// The first 4 bytes aren't `23 47 C0 AB` — this isn't a `ProjectData`
    /// file at all (or it's a format variant nobody has seen).
    BadMagic,
    /// The root `LENGTH` field doesn't match `filesize - 24`. Every real
    /// file measured this session matched exactly; a mismatch is treated as
    /// real corruption or truncation, not a version difference.
    LengthMismatch { declared: u32, expected: u64 },
    /// A record's header doesn't fit before EOF, or its declared payload
    /// size would run past EOF. This is the one case PLAN.md's UI copy
    /// should read as "newer Logic than Wit understands — compare bounces
    /// instead," not a crash.
    UnknownFraming {
        record_offset: usize,
        reason: &'static str,
    },
    /// Reading the file itself failed (permissions, doesn't exist, ...) —
    /// not a framing problem. The message is stored rather than the
    /// `std::io::Error` itself so `WalkError` can stay `PartialEq`.
    Io(String),
}

impl fmt::Display for WalkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WalkError::TooShortForRootHeader { len } => {
                write!(f, "{len} bytes is too short for a ProjectData root header (need 24)")
            }
            WalkError::BadMagic => write!(f, "missing ProjectData magic (23 47 C0 AB)"),
            WalkError::LengthMismatch { declared, expected } => write!(
                f,
                "root LENGTH field says {declared} but filesize-24 is {expected} — truncated or corrupt file"
            ),
            WalkError::UnknownFraming { record_offset, reason } => write!(
                f,
                "framing violation at record offset {record_offset:#x}: {reason} — newer Logic than Wit understands"
            ),
            WalkError::Io(msg) => write!(f, "failed to read ProjectData: {msg}"),
        }
    }
}

impl std::error::Error for WalkError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootHeader {
    /// The two bytes at file offset +0x04 — `d0 09`, `d1 09`, `c5 09`, and
    /// others all seen on real files. Never validated, only reported (e.g.
    /// for `wit logic probe --jsonl` and diagnostics).
    pub version_word: [u8; 2],
}

/// One record: its tag, its byte offset in the file (for diagnostics), and
/// its payload slice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record<'a> {
    pub tag: [u8; 4],
    pub offset: usize,
    pub payload: &'a [u8],
}

impl<'a> Record<'a> {
    /// The tag as a string. Real tags are always 4 printable ASCII bytes
    /// (`gnoS`, `karT`, ...); a non-ASCII tag would mean the walk has
    /// already gone off the rails, so this never panics — it just produces
    /// a lossy string an error message or diagnostic can still show.
    pub fn tag_str(&self) -> String {
        String::from_utf8_lossy(&self.tag).into_owned()
    }
}

/// Parse the 24-byte root header and return it plus the offset the record
/// sequence starts at (`ROOT_HEADER_LEN`, always — kept explicit so callers
/// never hardcode the magic number twice).
pub fn parse_root_header(data: &[u8]) -> Result<RootHeader, WalkError> {
    if data.len() < ROOT_HEADER_LEN {
        return Err(WalkError::TooShortForRootHeader { len: data.len() });
    }
    if data[0..4] != MAGIC {
        return Err(WalkError::BadMagic);
    }
    let declared = u32::from_le_bytes(
        data[LENGTH_FIELD_OFFSET..LENGTH_FIELD_OFFSET + 4]
            .try_into()
            .unwrap(),
    );
    let expected = (data.len() - ROOT_HEADER_LEN) as u64;
    if declared as u64 != expected {
        return Err(WalkError::LengthMismatch { declared, expected });
    }
    Ok(RootHeader {
        version_word: [data[4], data[5]],
    })
}

/// Walk every record in the payload (after the root header), in file order.
/// Bounds-checked at every step — a record whose header or declared payload
/// size would run past `data.len()` is a typed [`WalkError`], never a panic
/// and never a silent truncation.
pub fn walk_records(data: &[u8]) -> Result<Vec<Record<'_>>, WalkError> {
    let mut records = Vec::new();
    let mut pos = ROOT_HEADER_LEN;

    while pos < data.len() {
        if pos + RECORD_HEADER_LEN > data.len() {
            return Err(WalkError::UnknownFraming {
                record_offset: pos,
                reason: "record header runs past EOF",
            });
        }
        let tag: [u8; 4] = data[pos..pos + 4].try_into().unwrap();
        let payload_size = u32::from_le_bytes(
            data[pos + PAYLOAD_SIZE_FIELD_OFFSET..pos + PAYLOAD_SIZE_FIELD_OFFSET + 4]
                .try_into()
                .unwrap(),
        ) as usize;
        let total =
            RECORD_HEADER_LEN
                .checked_add(payload_size)
                .ok_or(WalkError::UnknownFraming {
                    record_offset: pos,
                    reason: "payload size overflows",
                })?;
        let record_end = pos.checked_add(total).ok_or(WalkError::UnknownFraming {
            record_offset: pos,
            reason: "record size overflows",
        })?;
        if record_end > data.len() {
            return Err(WalkError::UnknownFraming {
                record_offset: pos,
                reason: "declared payload size runs past EOF",
            });
        }
        records.push(Record {
            tag,
            offset: pos,
            payload: &data[pos + RECORD_HEADER_LEN..record_end],
        });
        pos = record_end;
    }
    // The loop invariant (pos <= data.len(), advanced only to a
    // bounds-checked record_end) guarantees pos == data.len() here — a
    // clean EOF is the only way out of the loop.
    debug_assert_eq!(pos, data.len());
    Ok(records)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal valid container: root header + the given records
    /// (tag, payload bytes).
    fn build(version: [u8; 2], records: &[(&[u8; 4], &[u8])]) -> Vec<u8> {
        let mut payload = Vec::new();
        for (tag, rec_payload) in records {
            payload.extend_from_slice(*tag);
            payload.extend_from_slice(&[0u8; 0x1c - 4]); // unmapped header fields
            payload.extend_from_slice(&(rec_payload.len() as u32).to_le_bytes());
            payload.extend_from_slice(&[0u8; RECORD_HEADER_LEN - 0x20]); // pad to 0x24
            payload.extend_from_slice(rec_payload);
        }
        let mut out = Vec::new();
        out.extend_from_slice(&MAGIC);
        out.extend_from_slice(&version);
        out.extend_from_slice(&[0u8; 10]);
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        out.extend_from_slice(&[0u8; 4]);
        out.extend_from_slice(&payload);
        out
    }

    #[test]
    fn walks_a_minimal_synthetic_container_clean() {
        let data = build(
            [0xd0u8, 0x09],
            &[(b"gnoS", b"hello"), (b"karT", b"world!!")],
        );
        let header = parse_root_header(&data).unwrap();
        assert_eq!(header.version_word, [0xd0u8, 0x09]);
        let records = walk_records(&data).unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].tag_str(), "gnoS");
        assert_eq!(records[0].payload, b"hello");
        assert_eq!(records[1].tag_str(), "karT");
        assert_eq!(records[1].payload, b"world!!");
    }

    #[test]
    fn accepts_every_version_word() {
        // The probe found d009 (Logic), d109 (newer Logic), and c509
        // (GarageBand) on real files — more than PROJECTDATA_FORMAT.md
        // documents. Never gate on this field.
        for version in [[0xd0u8, 0x09], [0xd1, 0x09], [0xc5, 0x09], [0xff, 0xff]] {
            let data = build(version, &[(b"gnoS", b"x")]);
            assert!(
                parse_root_header(&data).is_ok(),
                "version {version:?} should be accepted"
            );
        }
    }

    #[test]
    fn rejects_bad_magic() {
        let mut data = build([0xd0u8, 0x09], &[(b"gnoS", b"x")]);
        data[0] = 0x00;
        assert_eq!(parse_root_header(&data), Err(WalkError::BadMagic));
    }

    #[test]
    fn too_short_for_root_header_is_typed_not_a_panic() {
        assert!(matches!(
            parse_root_header(&[0u8; 10]),
            Err(WalkError::TooShortForRootHeader { len: 10 })
        ));
    }

    #[test]
    fn length_mismatch_is_refused() {
        let mut data = build([0xd0u8, 0x09], &[(b"gnoS", b"x")]);
        // Corrupt the declared length field.
        data[LENGTH_FIELD_OFFSET] ^= 0xff;
        assert!(matches!(
            parse_root_header(&data),
            Err(WalkError::LengthMismatch { .. })
        ));
    }

    #[test]
    fn a_record_claiming_a_payload_past_eof_is_unknown_framing_not_a_panic() {
        let mut data = build([0xd0u8, 0x09], &[(b"gnoS", b"x")]);
        // Corrupt the one record's declared payload size to something huge.
        let payload_size_off = ROOT_HEADER_LEN + PAYLOAD_SIZE_FIELD_OFFSET;
        data[payload_size_off..payload_size_off + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(matches!(
            walk_records(&data),
            Err(WalkError::UnknownFraming { .. })
        ));
    }

    #[test]
    fn a_truncated_record_header_is_unknown_framing_not_a_panic() {
        let data = build([0xd0u8, 0x09], &[(b"gnoS", b"x")]);
        // Truncate mid-header of the (only) record.
        let truncated = &data[..ROOT_HEADER_LEN + 5];
        // Root header parse doesn't see this (length mismatch fires first);
        // exercise walk_records directly against a hand-truncated buffer
        // that still claims the original length so we hit the *record*
        // bounds check, not the root one.
        let mut manual = truncated.to_vec();
        let new_len = (manual.len() - ROOT_HEADER_LEN) as u32;
        manual[LENGTH_FIELD_OFFSET..LENGTH_FIELD_OFFSET + 4]
            .copy_from_slice(&new_len.to_le_bytes());
        assert!(parse_root_header(&manual).is_ok());
        assert!(matches!(
            walk_records(&manual),
            Err(WalkError::UnknownFraming {
                reason: "record header runs past EOF",
                ..
            })
        ));
    }

    #[test]
    fn an_empty_container_walks_to_zero_records() {
        let data = build([0xd0u8, 0x09], &[]);
        assert_eq!(walk_records(&data).unwrap().len(), 0);
    }
}
