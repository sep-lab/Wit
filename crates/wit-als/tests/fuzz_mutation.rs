//! Property test: arbitrary truncation or byte-mutation of a valid `.als`
//! never panics — it always returns `Ok` or a typed [`wit_als::AlsError`].
//!
//! PLAN.md's test pyramid, "Property" row: "arbitrary truncation/mutation
//! of factory-built inputs never panics, never over-allocates past
//! Limits, always returns typed errors." A `.als` is an untrusted file by
//! construction (`wit-als/src/reader.rs`'s module doc); this is the
//! adversarial-input guarantee the hardening in `reader.rs` exists to
//! provide, checked against inputs no hand-written test enumerates.

use proptest::prelude::*;

fn valid_als() -> Vec<u8> {
    let xml = br#"<?xml version="1.0" encoding="UTF-8"?>
    <Ableton Creator="Ableton Live 12.0.5"><LiveSet><Tracks>
        <AudioTrack Id="1"><Name><EffectiveName Value="T"/></Name>
        <DeviceChain><Mixer>
            <Volume><Manual Value="0.8"/></Volume>
            <Pan><Manual Value="0.0"/></Pan>
            <Speaker><Manual Value="true"/></Speaker>
        </Mixer>
        <DeviceChain><Devices><Eq8 Id="0"><On><Manual Value="1"/></On></Eq8></Devices></DeviceChain>
        <MainSequencer><Sample><ArrangerAutomation><Events>
            <AudioClip Id="9"><CurrentStart Value="0.0"/><CurrentEnd Value="4.0"/>
                <Name Value="clip"/><Disabled Value="false"/>
                <SampleRef><FileRef><RelativePath Value="Samples/kick.wav"/></FileRef></SampleRef>
            </AudioClip>
        </Events></ArrangerAutomation></Sample></MainSequencer>
        </DeviceChain></AudioTrack>
    </Tracks>
    <MasterTrack><DeviceChain><Mixer><Tempo><Manual Value="120.0"/></Tempo></Mixer></DeviceChain></MasterTrack>
    </LiveSet></Ableton>"#;

    use flate2::write::GzEncoder;
    use flate2::Compression;
    use std::io::Write;
    let mut enc = GzEncoder::new(Vec::new(), Compression::default());
    enc.write_all(xml).unwrap();
    enc.finish().unwrap()
}

proptest! {
    /// Truncating a valid .als at any byte offset must never panic.
    #[test]
    fn truncation_never_panics(cut in 0usize..valid_als().len()) {
        let bytes = valid_als();
        let truncated = &bytes[..cut.min(bytes.len())];
        let _ = wit_als::parse(truncated); // Ok or Err — must not panic
    }

    /// Flipping any single byte of a valid .als must never panic.
    #[test]
    fn single_byte_mutation_never_panics(idx in 0usize..valid_als().len(), new_byte: u8) {
        let mut bytes = valid_als();
        let i = idx.min(bytes.len().saturating_sub(1));
        bytes[i] = new_byte;
        let _ = wit_als::parse(&bytes);
    }

    /// Arbitrary byte soup (not even gzip-shaped) must never panic — it
    /// must fail with a typed error, not a decoder crash.
    #[test]
    fn arbitrary_bytes_never_panic(bytes in prop::collection::vec(any::<u8>(), 0..4096)) {
        let _ = wit_als::parse(&bytes);
    }
}
