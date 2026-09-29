//! Property test: arbitrary truncation, byte mutation, or pure random
//! bytes must never panic `wit_flp::parse` — it always returns `Ok` or a
//! typed [`wit_flp::FlpError`].
//!
//! `.flp` is raw binary with hand-rolled offset arithmetic and a varint
//! decoder that reads a length from the file itself (`frame.rs`) — the
//! same adversarial-input shape `wit-als`/`wit-logic` already carry this
//! test for. AGENTS.md/PLAN's test pyramid, "Property" row.

use proptest::prelude::*;

fn valid_flp() -> Vec<u8> {
    let mut header_body = Vec::new();
    header_body.extend_from_slice(&0i16.to_le_bytes()); // format
    header_body.extend_from_slice(&2u16.to_le_bytes()); // channels
    header_body.extend_from_slice(&96u16.to_le_bytes()); // ppq

    let mut events = Vec::new();
    // Version (variable/text, latin-1)
    let version = b"20.8.3.2304\0";
    events.push(199u8);
    events.push(version.len() as u8);
    events.extend_from_slice(version);
    // ChanName (variable/text, latin-1)
    let name = b"Kick\0";
    events.push(192u8);
    events.push(name.len() as u8);
    events.extend_from_slice(name);
    // Tempo (dword)
    events.push(156u8);
    events.extend_from_slice(&130_000u32.to_le_bytes());
    // A byte and a word event, to exercise every fixed width class.
    events.push(0u8);
    events.push(1u8);
    events.push(64u8);
    events.extend_from_slice(&2u16.to_le_bytes());
    // The v25 resync exception, exercised even on a non-v25 fixture.
    events.push(172u8);
    events.extend_from_slice(&[1u8, 2, 3]);

    let mut data = Vec::new();
    data.extend_from_slice(b"FLhd");
    data.extend_from_slice(&(header_body.len() as u32).to_le_bytes());
    data.extend_from_slice(&header_body);
    data.extend_from_slice(b"FLdt");
    data.extend_from_slice(&(events.len() as u32).to_le_bytes());
    data.extend_from_slice(&events);
    data
}

proptest! {
    /// Truncating a valid .flp at any byte offset must never panic.
    #[test]
    fn truncation_never_panics(cut in 0usize..valid_flp().len()) {
        let bytes = valid_flp();
        let truncated = &bytes[..cut.min(bytes.len())];
        let _ = wit_flp::parse(truncated); // Ok or Err -- must not panic
    }

    /// Flipping any single byte of a valid .flp must never panic.
    #[test]
    fn single_byte_mutation_never_panics(idx in 0usize..valid_flp().len(), new_byte: u8) {
        let mut bytes = valid_flp();
        let i = idx.min(bytes.len().saturating_sub(1));
        bytes[i] = new_byte;
        let _ = wit_flp::parse(&bytes);
    }

    /// Arbitrary byte soup (not even FLhd-shaped) must never panic -- it
    /// must fail with a typed error, not an out-of-bounds panic.
    #[test]
    fn arbitrary_bytes_never_panic(bytes in prop::collection::vec(any::<u8>(), 0..4096)) {
        let _ = wit_flp::parse(&bytes);
    }

    /// A byte-soup event stream (valid header, garbage FLdt payload) must
    /// never panic either -- this is the shape most likely to desync the
    /// event walker into reading nonsense sizes.
    #[test]
    fn garbage_event_stream_never_panics(garbage in prop::collection::vec(any::<u8>(), 0..4096)) {
        let mut header_body = Vec::new();
        header_body.extend_from_slice(&0i16.to_le_bytes());
        header_body.extend_from_slice(&1u16.to_le_bytes());
        header_body.extend_from_slice(&96u16.to_le_bytes());
        let mut data = Vec::new();
        data.extend_from_slice(b"FLhd");
        data.extend_from_slice(&(header_body.len() as u32).to_le_bytes());
        data.extend_from_slice(&header_body);
        data.extend_from_slice(b"FLdt");
        data.extend_from_slice(&(garbage.len() as u32).to_le_bytes());
        data.extend_from_slice(&garbage);
        let _ = wit_flp::parse(&data);
    }
}
