//! Discovery, content-addressed store, and SQLite registry for Wit's
//! indexed history of a musician's projects.
//!
//! **This is the only crate in the workspace that writes Wit's own data**
//! — and it only ever writes to app-data/scratch, never to a project path.
//! That's enforced by construction: [`store::Store::ingest_bytes`] takes
//! bytes, not a path, so there is no write API anywhere in this crate a
//! caller could hand a project path to even by mistake. (The one other
//! writer in the workspace is `wit-platform`'s restore-as-copy module,
//! which can only write inside a validated Restores folder.)

pub mod discover;
pub mod dupes;
pub mod registry;
pub mod report;
pub mod scan;
pub mod store;

pub use discover::{
    discover_ableton_lineages, discover_logic_projects, AbletonLineage, LogicAlternative,
    LogicKind, LogicProject,
};
pub use dupes::{assert_no_home_paths, duplicate_report, DuplicateGroup, DuplicateReport};
pub use registry::{ProjectRow, Registry, RegistryError};
pub use report::{logic_report, LogicLibraryReport, SavePairResult};
pub use scan::{scan, ScanResult};
pub use store::{Hash, Store, StoreError};
