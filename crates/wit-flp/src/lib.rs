//! FL Studio `.flp` reader — a Rust port of `experiments/flp_parse.py`'s
//! event-stream walk, hardened against untrusted input (`frame.rs`), plus
//! whitelist name/tempo extraction (`extract.rs`) and a structural
//! comparison (`compare.rs`) this crate adds beyond what the Python
//! prototype does (it only ever printed a survey, never modeled or
//! diffed a project).
//!
//! # Format
//!
//! `docs/FORMATS.md`'s FL Studio section and `experiments/flp_parse.py`'s
//! module docstring are the ground truth this module ports:
//!
//! ```text
//! 'FLhd' + u32 header_len(=6) + { i16 format, u16 channels, u16 ppq }
//! 'FLdt' + u32 data_len       + event stream, to EOF of that length
//! Event = u8 id, then a payload sized by the id:
//!     id   0- 63 : 1 byte
//!     id  64-127 : 2 bytes
//!     id 128-191 : 4 bytes   (id 172 is a documented 3-byte exception — see frame.rs)
//!     id 192-255 : varint length, then that many bytes
//! ```
//!
//! # Version support
//!
//! **≤ v24 is fully supported.** Whitelist extraction covers channel
//! names, pattern names, plugin (generator/effect) names, mixer insert
//! names, a best-effort arrangement name, tempo, and the header/version
//! fields.
//!
//! **v25+ is partial, by design, not by oversight**
//! ([issue #7](https://github.com/sep-lab/Wit/issues/7)): an unsolved,
//! offset-dependent obfuscation keystream corrupts scalar (fixed-width)
//! events on v25-era files. This crate's own event-stream walk still
//! resyncs correctly (see `frame.rs`'s id-172 exception, verified against
//! 5 real v25.2.5 files this session), and every *text* event this crate
//! whitelists decodes cleanly and correctly on those same files — measured
//! this session, not assumed. Only [`extract::Tempo`], the one scalar
//! field this crate reads, degrades to
//! [`extract::Tempo::PartialV25ScalarsUnreadable`] instead of a number
//! that looks plausible but was measured to be garbage (a real v25 file's
//! `Tempo` dword decoded to 252,566.982 BPM).
//!
//! # Divergences from `experiments/flp_parse.py`
//!
//! The Python prototype is frozen (never edited) and was never run
//! against a v25 file, so it is not a golden reference for that case.
//! Every divergence is named where it happens, and summarized here:
//!
//! 1. **`frame.rs`: event id 172 is read as 3 bytes, not the general
//!    dword rule's 4** — the v25 resync fix issue #7 names. Verified
//!    inert (id 172 never appears) on real pre-v25 files.
//! 2. **`extract.rs`: tempo (id 156), mixer insert names (id 204), and a
//!    best-effort arrangement name (id 241) are new** — the prototype's
//!    `EVENT_NAMES` table never named these as semantic fields; it only
//!    ever printed a raw, generic text row for whatever id happened to
//!    fall in `TEXT_EVENTS`. All three (with per-id confidence noted in
//!    `extract.rs`) were identified and verified against real files this
//!    session, not carried over from any existing documentation.
//! 3. **`compare.rs` has no Python counterpart at all** — `flp_parse.py`
//!    never compares two files.

mod compare;
mod extract;
mod frame;

pub use compare::{compare, compare_with_bytes, FlChange};
pub use extract::{decode_text, extract, major_version, Extracted, FormatStatus, Tempo};
pub use frame::{parse_container, FlpError, Header, RawEvent, MAX_EVENTS};

/// Parse an `.flp` file's raw bytes end to end: header, event stream,
/// whitelist extraction.
pub fn parse(bytes: &[u8]) -> Result<Extracted, FlpError> {
    let (header, events) = frame::parse_container(bytes)?;
    Ok(extract::extract(&header, &events))
}

/// Parse an `.flp` file from disk.
pub fn parse_file(path: &std::path::Path) -> Result<Extracted, FlpError> {
    let bytes = std::fs::read(path)?;
    parse(&bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_end_to_end_on_a_minimal_synthetic_project() {
        let mut events = Vec::new();
        events.push(199u8); // Version, latin-1
        let version = b"20.8.3.2304\0";
        events.push(version.len() as u8); // varint fits in one byte
        events.extend_from_slice(version);
        events.push(192); // ChanName
        let name = b"Kick\0";
        events.push(name.len() as u8);
        events.extend_from_slice(name);
        events.push(156); // Tempo (dword)
        events.extend_from_slice(&130_000u32.to_le_bytes());

        let mut header = Vec::new();
        header.extend_from_slice(&0i16.to_le_bytes());
        header.extend_from_slice(&2u16.to_le_bytes());
        header.extend_from_slice(&96u16.to_le_bytes());

        let mut data = Vec::new();
        data.extend_from_slice(crate::frame::HEADER_MAGIC);
        data.extend_from_slice(&(header.len() as u32).to_le_bytes());
        data.extend_from_slice(&header);
        data.extend_from_slice(crate::frame::DATA_MAGIC);
        data.extend_from_slice(&(events.len() as u32).to_le_bytes());
        data.extend_from_slice(&events);

        let extracted = parse(&data).unwrap();
        assert_eq!(extracted.channels, 2);
        assert_eq!(extracted.fl_version, Some("20.8.3.2304".to_string()));
        assert_eq!(extracted.channel_names, vec!["Kick".to_string()]);
        assert_eq!(extracted.tempo, Tempo::Known(130.0));
        assert_eq!(extracted.format_status, FormatStatus::Complete);
    }

    #[test]
    fn parse_file_reports_a_typed_io_error_for_a_missing_file() {
        let err =
            parse_file(std::path::Path::new("/nonexistent/path/does-not-exist.flp")).unwrap_err();
        assert!(matches!(err, FlpError::Io(_)));
    }
}
