//! Tag census — a count of each record tag in a walked file.
//!
//! **Census numbers are an internal dirty-signal only — never rendered to a
//! user with a musician noun** (M2 tracking issue guardrail). A real project
//! measured ~35 user-visible tracks against 248–260 `karT` records; this
//! session measured `karT: 260` on a real 31-track project (`NumberOfTracks`
//! per `MetaData.plist`) — an ~8.4× multiplier, reconfirming record↔entity
//! multiplicity is not 1:1. A tag graduates to a human-facing count only
//! after probe-validated multiplicity (issue #3, not this milestone).

use crate::frame::Record;
use std::collections::BTreeMap;

pub type Census = BTreeMap<String, usize>;

pub fn census(records: &[Record<'_>]) -> Census {
    let mut counts = Census::new();
    for r in records {
        *counts.entry(r.tag_str()).or_insert(0) += 1;
    }
    counts
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::walk_records;

    #[test]
    fn counts_each_tag() {
        // Reuse frame's own builder indirectly via a hand-rolled minimal
        // container to keep this test self-contained.
        let mut payload = Vec::new();
        for tag in [b"gnoS", b"karT", b"karT", b"karT"] {
            payload.extend_from_slice(tag);
            payload.extend_from_slice(&[0u8; 0x1c - 4]);
            payload.extend_from_slice(&0u32.to_le_bytes());
            payload.extend_from_slice(&[0u8; 0x24 - 0x20]);
        }
        let mut data = Vec::new();
        data.extend_from_slice(&crate::frame::MAGIC);
        data.extend_from_slice(&[0xd0, 0x09]);
        data.extend_from_slice(&[0u8; 10]);
        data.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        data.extend_from_slice(&[0u8; 4]);
        data.extend_from_slice(&payload);

        let records = walk_records(&data).unwrap();
        let c = census(&records);
        assert_eq!(c["gnoS"], 1);
        assert_eq!(c["karT"], 3);
        assert_eq!(c.len(), 2);
    }
}
