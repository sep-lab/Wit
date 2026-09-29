//! `FLhd`/`FLdt` container framing and the typed event stream: a Rust port
//! of `experiments/flp_parse.py`'s byte-walking logic, hardened against
//! untrusted input the way `wit-als`/`wit-logic` are.
//!
//! ```text
//! b'FLhd' + u32 header_len(=6) + { i16 format, u16 channels, u16 ppq }
//! b'FLdt' + u32 data_len       + event stream, to EOF of the declared length
//!
//! Event = u8 id, then a payload whose size is implied by the id:
//!   id   0- 63 : 1 byte      ("byte" events)
//!   id  64-127 : 2 bytes     ("word" events)
//!   id 128-191 : 4 bytes     ("dword" events) -- ONE NAMED EXCEPTION, below
//!   id 192-255 : varint length, then that many bytes ("variable" events)
//! ```
//!
//! **Divergence from `flp_parse.py` (measured, not in the prototype): on
//! FL Studio 25+, event id 172 (`0xAC`) is read as a 3-byte payload, not
//! the general dword rule's 4.** This is exactly the framing quirk
//! [issue #7](https://github.com/sep-lab/Wit/issues/7) names for FL Studio
//! v25: *"v25 also breaks the event-width rule at a fixed spot: ID 172
//! (0xAC) carries a 3-byte payload, so the stream must resync."* The
//! Python prototype was never run against a v25 file and does not special-
//! case it, so `flp_parse.py` desyncs on one and cannot be used as a golden
//! reference here. Verified against 5 real FL 25.2.5 saves (a current save
//! and 4 `Backup/` autosaves, **inferred** from their names and folder to
//! be one project): under the 4-byte rule, 2 of the 5 fail outright — a
//! varint prefix longer than [`MAX_VARINT_BYTES`] bytes, around event 60 —
//! and the other 3 reach EOF only by coincidence, with inconsistent event
//! counts (1540–1652). Treating id 172 as 3 bytes gives a clean EOF on all
//! 5, with self-consistent counts (1616–1710) and correctly decoded text
//! after the resync point.
//!
//! **Gated on the file's own declared version.** The rule applies only
//! when a `Version` event (id 199) walked *earlier in the same stream*
//! declares major version 25 or later. That costs nothing: `Version` is a
//! variable-length event, so reading it never depends on the id-172 rule,
//! and it is event #0 in all 178 real files checked (FL 8.5.0 – 25.2.5),
//! while 172 is event #4 on all 5 FL 25 saves. Id 172 occurs zero times
//! in the 173 pre-v25 files, so the gate changes nothing measured; what it
//! buys is that an FL 21–24 file (none available to check) that used 172
//! as an ordinary dword would still walk correctly, matching issue #7's
//! statement that "FL Studio projects from v10 through v24 parse cleanly".
//! A 172 with no `Version` before it (never observed) takes the general
//! 4-byte rule.

use std::fmt;

pub const HEADER_MAGIC: &[u8; 4] = b"FLhd";
pub const DATA_MAGIC: &[u8; 4] = b"FLdt";

/// FL Studio 25's resync exception (see module doc): id 172 carries a
/// 3-byte payload, not the general dword rule's 4.
const RESYNC_EVENT_ID: u8 = 172;
const RESYNC_EVENT_PAYLOAD_LEN: usize = 3;
/// The FL major version from which [`RESYNC_EVENT_ID`] is 3 bytes.
const RESYNC_MIN_MAJOR_VERSION: u32 = 25;
/// `Version` — the text event the resync gate reads (see module doc).
const VERSION_EVENT_ID: u8 = 199;

/// A u32 payload length never needs more than 5 continuation bytes (35
/// usable bits) — matches `flp_parse.py`'s `MAX_VARINT_BYTES`. Anything
/// longer is corrupt or hostile input; refusing it bounds what would
/// otherwise be an unbounded shift into an ever-larger integer.
const MAX_VARINT_BYTES: usize = 5;

/// Defensive cap on the number of events a single file can walk to. Each
/// event consumes at least one byte of `id`, so this is already bounded by
/// the file's length in practice; the explicit cap exists so a pathological
/// input can never grow the returned `Vec<RawEvent>` past a fixed ceiling
/// regardless of how the bound above is reasoned about elsewhere.
pub const MAX_EVENTS: usize = 8_000_000;

#[derive(Debug, PartialEq, Eq)]
pub enum FlpError {
    /// Fewer than 8 bytes — can't even hold the `FLhd` magic + length.
    TooShortForHeaderMagic {
        len: usize,
    },
    BadHeaderMagic,
    /// The declared header length doesn't leave room for the 6 fixed fields
    /// (`format`, `channels`, `ppq`), or runs past EOF.
    HeaderTooShortOrTruncated {
        declared_len: u32,
    },
    BadDataMagic,
    /// The declared `FLdt` length runs past EOF.
    DataLengthRunsPastEof {
        declared_len: u32,
        available: u64,
    },
    /// An event id byte, or its fixed-width payload, would run past EOF.
    EventRunsPastEof {
        offset: usize,
    },
    /// A variable-length event's varint prefix exceeds [`MAX_VARINT_BYTES`]
    /// — corrupt, or a deliberately hostile file (mirrors
    /// `flp_parse.py`'s `FlpParseError` for the same condition).
    VarintTooLong {
        offset: usize,
    },
    /// An event's payload — fixed-width or variable-length — would run
    /// past EOF.
    PayloadRunsPastEof {
        offset: usize,
        declared_len: u64,
    },
    /// More events than [`MAX_EVENTS`] — refused rather than growing the
    /// event vector without bound.
    TooManyEvents {
        limit: usize,
    },
    Io(String),
}

impl fmt::Display for FlpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FlpError::TooShortForHeaderMagic { len } => {
                write!(f, "{len} byte(s) is too short to hold the FLhd header")
            }
            FlpError::BadHeaderMagic => write!(f, "missing FLhd magic — not an .flp file"),
            FlpError::HeaderTooShortOrTruncated { declared_len } => write!(
                f,
                "FLhd declares a {declared_len}-byte header body, which doesn't fit \
                 the format/channels/ppq fields or runs past EOF"
            ),
            FlpError::BadDataMagic => write!(f, "missing FLdt magic after the header"),
            FlpError::DataLengthRunsPastEof {
                declared_len,
                available,
            } => write!(
                f,
                "FLdt declares {declared_len} byte(s) of event stream, but only \
                 {available} remain in the file"
            ),
            FlpError::EventRunsPastEof { offset } => {
                write!(f, "an event at offset {offset:#x} runs past EOF")
            }
            FlpError::VarintTooLong { offset } => write!(
                f,
                "varint starting at offset {offset:#x} exceeds {MAX_VARINT_BYTES} bytes \
                 (corrupt length, or a deliberately hostile file)"
            ),
            FlpError::PayloadRunsPastEof {
                offset,
                declared_len,
            } => write!(
                f,
                "event at offset {offset:#x} declares a {declared_len}-byte payload \
                 that runs past EOF"
            ),
            FlpError::TooManyEvents { limit } => {
                write!(f, "more than {limit} events — refusing to keep walking")
            }
            FlpError::Io(msg) => write!(f, "failed to read .flp: {msg}"),
        }
    }
}

impl std::error::Error for FlpError {}

impl From<std::io::Error> for FlpError {
    fn from(e: std::io::Error) -> Self {
        FlpError::Io(e.to_string())
    }
}

/// The `FLhd` chunk's three fixed fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    /// Signed per the format (`flp_parse.py`: `struct.unpack("<hHH")` —
    /// this is genuinely `i16`, not `u16`; a format id of `-1` must not
    /// read back as `65535`).
    pub format: i16,
    pub channels: u16,
    pub ppq: u16,
}

/// One event: its id, its byte offset (for diagnostics), and its payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawEvent<'a> {
    pub id: u8,
    pub offset: usize,
    pub payload: &'a [u8],
}

impl<'a> RawEvent<'a> {
    fn width_class(id: u8) -> Option<usize> {
        match id {
            0..=63 => Some(1),
            64..=127 => Some(2),
            128..=191 => Some(4),
            _ => None, // variable-width; sized by a varint instead
        }
    }
}

fn need(data: &[u8], pos: usize, n: usize) -> Option<&[u8]> {
    data.get(pos..pos.checked_add(n)?)
}

fn need_byte(data: &[u8], pos: usize) -> Option<u8> {
    data.get(pos).copied()
}

/// Decode FL's LEB128-style unsigned varint starting at `pos`, a position
/// within `region` (never the whole file — see [`walk_events`], which
/// only ever calls this with `region` bounded to the declared `FLdt`
/// payload, so a lying varint can never read past that boundary into
/// whatever bytes happen to follow it in the file). Returns
/// `(value, new_pos)`, `new_pos` also relative to `region`. Refuses more
/// than [`MAX_VARINT_BYTES`] continuation bytes rather than looping (or
/// shifting an ever-larger integer) forever.
fn decode_varint(region: &[u8], pos: usize, base_offset: usize) -> Result<(u64, usize), FlpError> {
    let start = pos;
    let mut pos = pos;
    let mut value: u64 = 0;
    let mut shift: u32 = 0;
    for _ in 0..MAX_VARINT_BYTES {
        let byte = need_byte(region, pos).ok_or(FlpError::EventRunsPastEof {
            offset: base_offset + pos,
        })?;
        pos += 1;
        value |= u64::from(byte & 0x7f) << shift;
        shift += 7;
        if byte & 0x80 == 0 {
            return Ok((value, pos));
        }
    }
    Err(FlpError::VarintTooLong {
        offset: base_offset + start,
    })
}

/// Parse the `FLhd` chunk. Returns the header and the byte offset the
/// `FLdt` chunk starts at.
pub fn parse_header(data: &[u8]) -> Result<(Header, usize), FlpError> {
    if data.len() < 8 {
        return Err(FlpError::TooShortForHeaderMagic { len: data.len() });
    }
    if &data[0..4] != HEADER_MAGIC {
        return Err(FlpError::BadHeaderMagic);
    }
    let header_len = u32::from_le_bytes(data[4..8].try_into().unwrap());
    let body = need(data, 8, header_len as usize).ok_or(FlpError::HeaderTooShortOrTruncated {
        declared_len: header_len,
    })?;
    if body.len() < 6 {
        return Err(FlpError::HeaderTooShortOrTruncated {
            declared_len: header_len,
        });
    }
    let format = i16::from_le_bytes(body[0..2].try_into().unwrap());
    let channels = u16::from_le_bytes(body[2..4].try_into().unwrap());
    let ppq = u16::from_le_bytes(body[4..6].try_into().unwrap());
    let data_chunk_offset = 8 + header_len as usize;
    Ok((
        Header {
            format,
            channels,
            ppq,
        },
        data_chunk_offset,
    ))
}

/// Parse the `FLdt` chunk header at `pos` and walk every event in it.
/// Bounds-checked at every step: a truncated magic, a length that runs
/// past EOF, or an event whose payload would run past EOF is a typed
/// [`FlpError`], never a panic.
pub fn walk_events(data: &[u8], pos: usize) -> Result<Vec<RawEvent<'_>>, FlpError> {
    let magic = need(data, pos, 4).ok_or(FlpError::EventRunsPastEof { offset: pos })?;
    if magic != DATA_MAGIC {
        return Err(FlpError::BadDataMagic);
    }
    let len_bytes = need(data, pos + 4, 4).ok_or(FlpError::EventRunsPastEof { offset: pos })?;
    let declared_len = u32::from_le_bytes(len_bytes.try_into().unwrap());
    let start = pos + 8;
    let end = start
        .checked_add(declared_len as usize)
        .filter(|&e| e <= data.len())
        .ok_or(FlpError::DataLengthRunsPastEof {
            declared_len,
            available: data.len().saturating_sub(start) as u64,
        })?;
    // Every bound below is checked against `region`, never `data` — a
    // lying fixed-width or varint payload length must never be able to
    // read past the declared FLdt boundary into whatever bytes happen to
    // follow it in the file. Bounding only against `data.len()` (the
    // whole file) would silently accept a corrupt length as long as the
    // rest of the file happened to be long enough to absorb it — exactly
    // the kind of trust-the-declared-length bug this walker exists to
    // refuse.
    let region = &data[start..end];

    let mut events = Vec::new();
    let mut cursor = 0usize;
    // Set by the first Version event walked; gates the id-172 rule.
    let mut resync_applies = false;
    let mut version_seen = false;
    while cursor < region.len() {
        let event_offset = cursor;
        let id = need_byte(region, cursor).ok_or(FlpError::EventRunsPastEof {
            offset: start + cursor,
        })?;
        cursor += 1;

        let size = if id == RESYNC_EVENT_ID && resync_applies {
            RESYNC_EVENT_PAYLOAD_LEN
        } else if let Some(fixed) = RawEvent::width_class(id) {
            fixed
        } else {
            let (varint_len, new_cursor) = decode_varint(region, cursor, start)?;
            cursor = new_cursor;
            usize::try_from(varint_len).map_err(|_| FlpError::PayloadRunsPastEof {
                offset: start + event_offset,
                declared_len: varint_len,
            })?
        };

        let payload = need(region, cursor, size).ok_or(FlpError::PayloadRunsPastEof {
            offset: start + event_offset,
            declared_len: size as u64,
        })?;
        cursor += size;

        if id == VERSION_EVENT_ID && !version_seen {
            version_seen = true;
            resync_applies = crate::extract::major_version(&crate::extract::decode_text(payload))
                .is_some_and(|major| major >= RESYNC_MIN_MAJOR_VERSION);
        }

        if events.len() >= MAX_EVENTS {
            return Err(FlpError::TooManyEvents { limit: MAX_EVENTS });
        }
        events.push(RawEvent {
            id,
            offset: start + event_offset,
            payload,
        });
    }
    // The loop only advances `cursor` to a bounds-checked position
    // <= region.len(), so exiting the loop with cursor > region.len() is
    // impossible; this asserts the invariant rather than silently
    // trusting it.
    debug_assert_eq!(cursor, region.len());
    Ok(events)
}

/// Parse both chunks end to end: `FLhd` then `FLdt`.
pub fn parse_container(data: &[u8]) -> Result<(Header, Vec<RawEvent<'_>>), FlpError> {
    let (header, data_chunk_offset) = parse_header(data)?;
    let events = walk_events(data, data_chunk_offset)?;
    Ok((header, events))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header_bytes(format: i16, channels: u16, ppq: u16) -> Vec<u8> {
        let mut body = Vec::new();
        body.extend_from_slice(&format.to_le_bytes());
        body.extend_from_slice(&channels.to_le_bytes());
        body.extend_from_slice(&ppq.to_le_bytes());
        let mut out = Vec::new();
        out.extend_from_slice(HEADER_MAGIC);
        out.extend_from_slice(&(body.len() as u32).to_le_bytes());
        out.extend_from_slice(&body);
        out
    }

    #[test]
    fn parses_header_fields() {
        let (header, offset) = parse_header(&header_bytes(0, 18, 96)).unwrap();
        assert_eq!(
            header,
            Header {
                format: 0,
                channels: 18,
                ppq: 96
            }
        );
        assert_eq!(offset, 8 + 6);
    }

    #[test]
    fn format_field_is_signed() {
        let (header, _) = parse_header(&header_bytes(-1, 1, 96)).unwrap();
        assert_eq!(header.format, -1);
    }

    #[test]
    fn rejects_bad_header_magic() {
        let mut bytes = header_bytes(0, 1, 96);
        bytes[0] = b'X';
        assert_eq!(parse_header(&bytes), Err(FlpError::BadHeaderMagic));
    }

    #[test]
    fn too_short_for_header_magic_is_typed_not_a_panic() {
        assert_eq!(
            parse_header(&[0u8; 4]),
            Err(FlpError::TooShortForHeaderMagic { len: 4 })
        );
    }

    #[test]
    fn header_length_running_past_eof_is_typed_not_a_panic() {
        let mut bytes = header_bytes(0, 1, 96);
        bytes[4..8].copy_from_slice(&1_000_000u32.to_le_bytes());
        assert!(matches!(
            parse_header(&bytes),
            Err(FlpError::HeaderTooShortOrTruncated { .. })
        ));
    }

    fn container_with(events: &[u8]) -> Vec<u8> {
        let mut out = header_bytes(0, 1, 96);
        out.extend_from_slice(DATA_MAGIC);
        out.extend_from_slice(&(events.len() as u32).to_le_bytes());
        out.extend_from_slice(events);
        out
    }

    #[test]
    fn empty_event_stream_is_not_an_error() {
        let data = container_with(&[]);
        let (_, events) = parse_container(&data).unwrap();
        assert!(events.is_empty());
    }

    #[test]
    fn rejects_bad_data_magic() {
        let mut data = container_with(&[]);
        let dt_offset = 8 + 6;
        data[dt_offset] = b'X';
        assert_eq!(parse_container(&data), Err(FlpError::BadDataMagic));
    }

    #[test]
    fn byte_word_dword_classes_consume_exactly_their_width() {
        // id=5 (byte), id=70 (word), id=150 (dword), interleaved with a
        // sentinel byte event so a mis-sized read would desync the stream.
        let events = [
            5u8, 0xAA, // byte event
            70, 0x11, 0x22, // word event
            150, 0x01, 0x02, 0x03, 0x04, // dword event
            6, 0x99, // sentinel byte event
        ];
        let data = container_with(&events);
        let (_, walked) = parse_container(&data).unwrap();
        assert_eq!(walked.len(), 4);
        assert_eq!(
            walked[0],
            RawEvent {
                id: 5,
                offset: 22,
                payload: &[0xAA]
            }
        );
        assert_eq!(walked[3].id, 6);
        assert_eq!(walked[3].payload, &[0x99]);
    }

    #[test]
    fn variable_width_event_reads_a_varint_length_prefix() {
        let mut events = vec![209u8, 0xAC, 0x02]; // varint(300) = 0xAC 0x02
        events.extend(std::iter::repeat_n(0x01u8, 300));
        events.push(1);
        events.push(7);
        let data = container_with(&events);
        let (_, walked) = parse_container(&data).unwrap();
        assert_eq!(walked.len(), 2);
        assert_eq!(walked[0].id, 209);
        assert_eq!(walked[0].payload.len(), 300);
        assert_eq!(walked[1].payload, &[7]);
    }

    #[test]
    fn a_varint_longer_than_five_bytes_is_refused() {
        // 5 continuation bytes then a 6th still with the continuation bit set.
        let events = [209u8, 0x80, 0x80, 0x80, 0x80, 0x80, 0x01];
        let data = container_with(&events);
        assert!(matches!(
            parse_container(&data),
            Err(FlpError::VarintTooLong { .. })
        ));
    }

    #[test]
    fn a_variable_payload_declared_past_eof_is_refused_not_a_panic() {
        let events = [209u8, 0xFF, 0x7F]; // declares a huge payload, no bytes follow
        let data = container_with(&events);
        assert!(matches!(
            parse_container(&data),
            Err(FlpError::PayloadRunsPastEof { .. })
        ));
    }

    #[test]
    fn a_truncated_fixed_width_payload_is_refused_not_a_panic() {
        let events = [150u8, 0x01, 0x02]; // dword event, only 2 of 4 payload bytes present
        let data = container_with(&events);
        assert!(matches!(
            parse_container(&data),
            Err(FlpError::PayloadRunsPastEof { .. })
        ));
    }

    /// A latin-1 `Version` (199) event, as event #0 of a real file.
    fn version_event(version: &str) -> Vec<u8> {
        let mut out = vec![199u8, (version.len() + 1) as u8];
        out.extend_from_slice(version.as_bytes());
        out.push(0);
        out
    }

    #[test]
    fn event_id_172_is_read_as_three_bytes_on_fl_25() {
        // The v25 resync exception (module doc), in the measured position:
        // Version first, then 172 as 3 bytes; a sentinel byte event proves
        // the stream resynced correctly.
        let mut events = version_event("25.2.5.5055");
        events.extend_from_slice(&[172u8, 0x01, 0x02, 0x03, 9, 0x55]);
        let data = container_with(&events);
        let (_, walked) = parse_container(&data).unwrap();
        assert_eq!(walked.len(), 3);
        assert_eq!(walked[1].id, 172);
        assert_eq!(walked[1].payload, &[0x01, 0x02, 0x03]);
        assert_eq!(walked[2].id, 9);
        assert_eq!(walked[2].payload, &[0x55]);
    }

    #[test]
    fn event_id_172_is_an_ordinary_dword_before_fl_25() {
        // Gated on the declared version: an FL 24 (or older) file that
        // used 172 as a plain dword must still walk correctly.
        for version in ["24.1.1.4234", "20.8.3.2304", "10.0.0"] {
            let mut events = version_event(version);
            events.extend_from_slice(&[172u8, 0x01, 0x02, 0x03, 0x04, 9, 0x55]);
            let data = container_with(&events);
            let (_, walked) = parse_container(&data).unwrap();
            assert_eq!(walked.len(), 3, "{version}");
            assert_eq!(walked[1].payload, &[0x01, 0x02, 0x03, 0x04], "{version}");
            assert_eq!(walked[2].payload, &[0x55], "{version}");
        }
    }

    #[test]
    fn event_id_172_before_any_version_event_is_an_ordinary_dword() {
        // Never observed (Version is event #0 in every real file checked);
        // the general rule applies.
        let mut events = vec![172u8, 0x01, 0x02, 0x03, 0x04];
        events.extend(version_event("25.2.5.5055"));
        let data = container_with(&events);
        let (_, walked) = parse_container(&data).unwrap();
        assert_eq!(walked.len(), 2);
        assert_eq!(walked[0].payload, &[0x01, 0x02, 0x03, 0x04]);
    }

    #[test]
    fn declared_data_length_past_eof_is_refused() {
        let mut data = container_with(&[1, 2]);
        let dlen_offset = 8 + 6 + 4;
        data[dlen_offset..dlen_offset + 4].copy_from_slice(&1_000_000u32.to_le_bytes());
        assert!(matches!(
            parse_container(&data),
            Err(FlpError::DataLengthRunsPastEof { .. })
        ));
    }
}
