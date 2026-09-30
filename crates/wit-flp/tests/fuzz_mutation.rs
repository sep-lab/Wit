//! Property test: arbitrary truncation, byte mutation, or pure random
//! bytes must never panic `wit_flp::parse` — it always returns `Ok` or a
//! typed [`wit_flp::FlpError`].
//!
//! `.flp` is raw binary with hand-rolled offset arithmetic and a varint
//! decoder that reads a length from the file itself (`frame.rs`) — the
//! same adversarial-input shape `wit-als`/`wit-logic` already carry this
//! test for. AGENTS.md/PLAN's test pyramid, "Property" row.

use proptest::prelude::*;

/// A small valid project. `v25` picks the FL 25 shape (id 172 is 3 bytes,
/// see `frame.rs`) or the FL 20 one (172 is an ordinary 4-byte dword), so
/// both framing paths get mutated.
fn valid_flp(v25: bool) -> Vec<u8> {
    let mut header_body = Vec::new();
    header_body.extend_from_slice(&0i16.to_le_bytes()); // format
    header_body.extend_from_slice(&2u16.to_le_bytes()); // channels
    header_body.extend_from_slice(&96u16.to_le_bytes()); // ppq

    let mut events = Vec::new();
    // Version (variable/text, latin-1)
    let version: &[u8] = if v25 {
        b"25.2.5.5055\0"
    } else {
        b"20.8.3.2304\0"
    };
    events.push(199u8);
    events.push(version.len() as u8);
    events.extend_from_slice(version);
    // The resync exception right after it, as on real FL 25 files.
    events.push(172u8);
    events.extend_from_slice(if v25 {
        &[1u8, 2, 3][..]
    } else {
        &[1u8, 2, 3, 4][..]
    });
    // A channel block: NewChan (word), 21 (byte), DefPluginName, 212,
    // PluginName (the channel's name on FL >= 11.5).
    events.push(64u8);
    events.extend_from_slice(&0u16.to_le_bytes());
    events.push(21u8);
    events.push(0u8);
    let generator = b"FPC\0";
    events.push(201u8);
    events.push(generator.len() as u8);
    events.extend_from_slice(generator);
    events.push(212u8);
    events.push(0u8);
    let name = b"Kick\0";
    events.push(203u8);
    events.push(name.len() as u8);
    events.extend_from_slice(name);
    // Tempo (dword)
    events.push(156u8);
    events.extend_from_slice(&130_000u32.to_le_bytes());
    // The mixer: one position with one effect.
    events.push(236u8);
    events.push(0u8);
    let effect = b"Maximus\0";
    events.push(201u8);
    events.push(effect.len() as u8);
    events.extend_from_slice(effect);

    let mut data = Vec::new();
    data.extend_from_slice(b"FLhd");
    data.extend_from_slice(&(header_body.len() as u32).to_le_bytes());
    data.extend_from_slice(&header_body);
    data.extend_from_slice(b"FLdt");
    data.extend_from_slice(&(events.len() as u32).to_le_bytes());
    data.extend_from_slice(&events);
    data
}

/// The un-mutated fixtures really are valid, on both framing paths — so
/// the properties below mutate something that parses, not noise.
#[test]
fn both_fixtures_parse_clean() {
    for v25 in [false, true] {
        let e = wit_flp::parse(&valid_flp(v25)).unwrap();
        assert_eq!(e.channel_names(), vec!["Kick".to_string()], "v25={v25}");
        assert_eq!(e.generator_names(), vec!["FPC".to_string()], "v25={v25}");
        assert_eq!(e.mixer_effects.len(), 1, "v25={v25}");
    }
}

proptest! {
    /// Truncating a valid .flp at any byte offset must never panic.
    #[test]
    fn truncation_never_panics(v25: bool, cut in 0usize..valid_flp(true).len()) {
        let bytes = valid_flp(v25);
        let truncated = &bytes[..cut.min(bytes.len())];
        let _ = wit_flp::parse(truncated); // Ok or Err -- must not panic
    }

    /// Flipping any single byte of a valid .flp must never panic.
    #[test]
    fn single_byte_mutation_never_panics(
        v25: bool,
        idx in 0usize..valid_flp(false).len(),
        new_byte: u8,
    ) {
        let mut bytes = valid_flp(v25);
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
