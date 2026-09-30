//! FL Studio `.flp` reader — a Rust port of `experiments/flp_parse.py`'s
//! event-stream walk, hardened against untrusted input (`frame.rs`), plus
//! whitelist name/tempo extraction (`extract.rs`) and a structural
//! comparison (`compare.rs`) this crate adds beyond what the Python
//! prototype does (it only ever printed a survey, never modeled or
//! compared a project).
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
//!     id 128-191 : 4 bytes   (id 172 is 3 bytes on FL 25+ — see frame.rs)
//!     id 192-255 : varint length, then that many bytes
//! ```
//!
//! # Version support
//!
//! **≤ v24 is fully supported, across two different real on-disk shapes.**
//! FL Studio changed where a channel's own name is stored between FL
//! 11.1.1 and FL 11.5.14 (measured on real files; see `extract.rs`'s
//! `NEW_CHANNEL_SCHEME_MIN_VERSION` doc for the evidence and the
//! unmeasured 11.2–11.4 gap) — this crate reads whichever shape the file's
//! own declared version says it should have.
//!
//! **v25+ is partial, by design, not by oversight**
//! ([issue #7](https://github.com/sep-lab/Wit/issues/7)): an unsolved,
//! offset-dependent obfuscation keystream corrupts scalar (fixed-width)
//! events on v25-era files. This crate's own event-stream walk still
//! resyncs correctly (see `frame.rs`'s id-172 exception, verified against
//! 5 real v25.2.5 files), channel names decode cleanly and number exactly
//! the file's declared channel count on those 5 files, and generator and
//! effect plugin names decode cleanly. Pattern names and mixer insert
//! names are **unverified rather than confirmed** on v25 (ids 193 and 204
//! never occur in any of those 5 files), as is which mixer insert an
//! effect is attributed to. [`extract::Tempo`], the one scalar field this
//! crate reads, degrades to [`extract::Tempo::PartialV25ScalarsUnreadable`]
//! instead of a number that was measured to be garbage (a real v25 file's
//! `Tempo` dword decoded to 252,566.982 BPM).
//!
//! # Divergences from `experiments/flp_parse.py`
//!
//! The Python prototype is frozen (never edited) and was never run
//! against an FL 11.5+ or v25 file, so it is not a golden reference for
//! either. Every divergence is named where it happens, and summarized
//! here:
//!
//! 1. **`frame.rs`: on FL 25+, event id 172 is read as 3 bytes, not the
//!    general dword rule's 4** — the v25 resync fix issue #7 names, gated
//!    on the file's own `Version` event (event #0 in all 178 real files
//!    checked). Id 172 never occurs in the 173 pre-v25 files checked.
//! 2. **`extract.rs`: what a name means depends on where it sits and on
//!    the file's FL version.** Channel names come from id 192 before FL
//!    11.5 and from the `PluginName` (203) in each channel's own header
//!    from FL 11.5 on; a channel block ends at the next `NewChan` or at the
//!    first arrangement/playlist/mixer event, so a later 203 (a renamed
//!    mixer effect, say) is never read as a channel's name.
//!    `DefPluginName` (201) is a channel's generator in a channel header
//!    and an effect in the mixer — on every version, which the prototype
//!    (one flat text listing) never distinguished. And every name is kept
//!    on the object it names (a channel's rack position, a pattern's
//!    number, a mixer position, an arrangement's order), because FL saves
//!    most names only when the user set one.
//! 3. **`extract.rs`: tempo (id 156) and a best-effort arrangement name
//!    (id 241) are new** — neither is decoded as a *semantic* field by the
//!    prototype, which prints one generic, unlabelled text row per id in
//!    `TEXT_EVENTS` (its own `EVENT_NAMES` table already names id 204
//!    `"InsertName"`, so that id is not new, only extracting it as a
//!    structured field is).
//! 4. **`compare.rs` has no Python counterpart at all** — `flp_parse.py`
//!    never compares two files.

mod compare;
mod extract;
mod frame;

pub use compare::{compare, compare_with_bytes, FlChange, NameChange};
pub use extract::{
    decode_text, extract, major_version, Extracted, FormatStatus, MixerEffect, Pattern,
    RackChannel, Tempo, MAX_NUMBERED_INSERT,
};
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
        // FL >= 11.5 shape, byte for byte as measured on real files:
        // NewChan (64, word), 21 (byte), DefPluginName (201, empty -- a
        // plain Sampler channel), 212, PluginName (203, the channel's own
        // display name) -- see extract.rs's module doc.
        let mut events = Vec::new();
        events.push(199u8); // Version, latin-1
        let version = b"20.8.3.2304\0";
        events.push(version.len() as u8); // varint fits in one byte
        events.extend_from_slice(version);
        events.push(64); // NewChan (word)
        events.extend_from_slice(&0u16.to_le_bytes());
        events.extend_from_slice(&[21, 0]); // byte event
        events.push(201); // DefPluginName, empty
        events.push(1); // varint length 1 (just the NUL terminator)
        events.push(0);
        events.extend_from_slice(&[212, 0]); // empty plugin data
        events.push(203); // PluginName -- the channel's display name
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
        assert_eq!(extracted.channel_names(), vec!["Kick".to_string()]);
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
