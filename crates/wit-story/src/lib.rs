//! The Story: what Wit tells a musician about their song, as one typed,
//! JSON-serialisable document — sessions of saves ("moments"), plain
//! sentences about what changed, a track heat strip, what Wit can and can't
//! see, and the song's family. The app, `wit story --json` and the share
//! page all read this; nothing else invents musician-facing text.
//!
//! - [`types`] is the contract. `schema/library.schema.json` is generated
//!   from it and pinned by `tests/contract.rs`.
//! - [`build`] turns the saves a DAW keeps on disk into a Story. Read-only.
//! - [`sentence`] is the only place a `ChangeRecord` becomes words.
//! - [`vocab`] is the banned-vocabulary lint.
//! - `fixtures/demo-library.json` is the Library built from
//!   `just demo-library`, which the UI is developed against.
//!
//! See wit-planning/PLAN-V2 "Phase C — Contract first" for why this landed
//! before the extractors that feed it.

pub mod build;
pub mod clock;
pub mod sentence;
pub mod types;
pub mod vocab;

pub use build::{ableton_story, build_library, logic_stories, SESSION_GAP_SECS};
pub use clock::Clock;
pub use types::*;

/// The JSON schema of [`Library`], pretty-printed with a trailing newline —
/// exactly what `schema/library.schema.json` holds.
pub fn library_schema_json() -> String {
    let schema = schemars::schema_for!(Library);
    let mut s = serde_json::to_string_pretty(&schema).expect("schema serialises");
    s.push('\n');
    s
}

/// A Library as pretty JSON with a trailing newline — the fixture format.
pub fn to_json(library: &Library) -> String {
    let mut s = serde_json::to_string_pretty(library).expect("library serialises");
    s.push('\n');
    s
}
