//! Discovery, content-addressed store, and SQLite registry for Wit's
//! indexed history of a musician's projects.
//!
//! **This is the only crate in the workspace that writes** — and it only
//! ever writes to app-data/scratch, never to a project path. That's
//! enforced by construction: [`store::Store::ingest_bytes`] takes bytes,
//! not a path, so there is no write API anywhere in this crate a caller
//! could hand a project path to even by mistake.

pub mod discover;
pub mod dupes;
pub mod registry;
pub mod scan;
pub mod store;

pub use discover::{
    discover_ableton_lineages, discover_logic_projects, AbletonLineage, LogicAlternative,
    LogicKind, LogicProject,
};
pub use dupes::{assert_no_home_paths, duplicate_report, DuplicateGroup, DuplicateReport};
pub use registry::{ProjectRow, Registry, RegistryError};
pub use scan::{scan, ScanResult};
pub use store::{Hash, Store, StoreError};
