//! Property test: arbitrary truncation, byte-mutation, or pure random
//! bytes must never panic `wit_logic::walk` — it always returns `Ok` or a
//! typed [`wit_logic::WalkError`].
//!
//! `ProjectData` is raw binary with hand-rolled offset arithmetic
//! (`frame.rs`, `extract.rs`) — an even more adversarial-input-prone
//! target than `wit-als`'s XML, since there's no format library between
//! this crate and the bytes. PLAN.md's test pyramid, "Property" row.

use proptest::prelude::*;

fn valid_container() -> Vec<u8> {
    let mut payload = Vec::new();
    for (tag, rec_payload) in [
        (b"gnoS", vec![0u8; 0x400]),
        (b"karT", vec![1u8; 16]),
        (b"qeSM", {
            let mut p = vec![0u8; 0x10];
            p.extend_from_slice(&5u16.to_le_bytes());
            p.extend_from_slice(b"Piano");
            p
        }),
        (b"gRuA", {
            let mut p = vec![0u8; 0x4a];
            p.extend_from_slice(&4u16.to_le_bytes());
            p.extend_from_slice(b"Take");
            p
        }),
    ] {
        payload.extend_from_slice(tag);
        payload.extend_from_slice(&[0u8; 0x1c - 4]);
        payload.extend_from_slice(&(rec_payload.len() as u32).to_le_bytes());
        payload.extend_from_slice(&[0u8; 0x24 - 0x20]);
        payload.extend_from_slice(&rec_payload);
    }
    let mut out = Vec::new();
    out.extend_from_slice(&[0x23, 0x47, 0xC0, 0xAB]);
    out.extend_from_slice(&[0xd0, 0x09]);
    out.extend_from_slice(&[0u8; 10]);
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(&[0u8; 4]);
    out.extend_from_slice(&payload);
    out
}

proptest! {
    #[test]
    fn truncation_never_panics(cut in 0usize..valid_container().len()) {
        let data = valid_container();
        let _ = wit_logic::walk(&data[..cut.min(data.len())]);
    }

    #[test]
    fn single_byte_mutation_never_panics(idx in 0usize..valid_container().len(), new_byte: u8) {
        let mut data = valid_container();
        let i = idx.min(data.len().saturating_sub(1));
        data[i] = new_byte;
        let _ = wit_logic::walk(&data);
    }

    #[test]
    fn arbitrary_bytes_never_panic(bytes in prop::collection::vec(any::<u8>(), 0..4096)) {
        let _ = wit_logic::walk(&bytes);
    }
}
