//! Hardened `.als` (Ableton Live Set) reader.
//!
//! A `.als` is gzipped XML (`docs/FORMATS.md`); this crate turns one into a
//! [`wit_model::Model`] the same way `experiments/als_semantic_diff.py`'s
//! `build_model()` does, whitelist field by whitelist field, with three
//! differences from the Python prototype, all named in
//! `wit-planning/PLAN.md`'s M1 milestone:
//!
//! 1. Tempo is found generically (any `Tempo/Manual` under `LiveSet`), not
//!    by requiring the host element be named `MasterTrack` — Live 12.3
//!    renamed it to `MainTrack` (see `extract.rs`).
//! 2. Every device carries a parameter fingerprint, not just its tag — see
//!    [`wit_model::Device`].
//! 3. Untrusted input is treated as untrusted: bounded decompression, a
//!    rejected DOCTYPE, and a depth-capped parser (see `reader.rs`).
//!
//! (The other two M1 named bug fixes — deterministic sample-rename ordering
//! and the rename-bijection guard — are `wit-diff`'s, not this crate's;
//! they are about comparing two models, not extracting one.)

mod dom;
mod extract;
mod reader;

pub use extract::build_model;
pub use reader::{AlsError, MAX_DECOMPRESSED_BYTES, MAX_DEPTH};

/// Parse a `.als` file's raw bytes (still gzip-compressed, as read off
/// disk) into a [`wit_model::Model`].
pub fn parse(bytes: &[u8]) -> Result<wit_model::Model, AlsError> {
    let xml_bytes = reader::bounded_gunzip(bytes, MAX_DECOMPRESSED_BYTES)?;
    let root = reader::parse_xml(&xml_bytes, MAX_DEPTH)?;
    Ok(build_model(&root))
}

/// Parse a `.als` file from disk.
pub fn parse_file(path: &std::path::Path) -> Result<wit_model::Model, AlsError> {
    let bytes = std::fs::read(path)?;
    parse(&bytes)
}
