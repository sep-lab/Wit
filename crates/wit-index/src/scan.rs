//! `wit scan` orchestration: discover, archive-before-recycle ingest into
//! the [`Store`], and record in the [`Registry`].
//!
//! Slot keys are derived from **stable identifiers** — a Logic backup's
//! own slot directory name (`"00"`, `"01"`, ...) and an Ableton save's own
//! filename — never a positional index into the discovered list. A
//! positional index shifts on rescan if a new save turns up earlier in
//! the timeline than anything seen before, which would silently
//! mis-attribute an already-recorded slot to a different physical file.
//! Stable identifiers make that impossible: the same physical save always
//! maps to the same slot key, rescan after rescan.

use crate::discover::{discover_ableton_lineages, discover_logic_projects, LogicKind};
use crate::registry::Registry;
use crate::store::Store;
use std::path::Path;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ScanResult {
    pub logic_projects_found: usize,
    pub ableton_lineages_found: usize,
    pub new_versions_ingested: usize,
    pub read_errors: usize,
}

/// Scan `root`, archive-before-recycle every discovered version into
/// `store`, and record it in `registry`. `now` is the ingestion timestamp
/// (Unix seconds) — passed in rather than read from the system clock so
/// callers (and tests) control it explicitly.
pub fn scan(root: &Path, store: &Store, registry: &Registry, now: i64) -> ScanResult {
    let mut result = ScanResult::default();

    let logic_projects = discover_logic_projects(root);
    result.logic_projects_found = logic_projects.len();
    for project in &logic_projects {
        let kind_str = match project.kind {
            LogicKind::Logic => "logic",
            LogicKind::GarageBand => "garageband",
        };
        let Ok(project_id) = registry.upsert_project(
            &project.name,
            &project.bundle_path.to_string_lossy(),
            kind_str,
        ) else {
            result.read_errors += 1;
            continue;
        };
        for alt in &project.alternatives {
            for backup_path in &alt.backups {
                let slot_name = backup_path
                    .parent()
                    .and_then(|p| p.file_name())
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "unknown".to_string());
                let slot_key = format!("{}/backup/{slot_name}", alt.name);
                ingest_one(
                    backup_path,
                    &slot_key,
                    project_id,
                    store,
                    registry,
                    now,
                    &mut result,
                );
            }
            let slot_key = format!("{}/current", alt.name);
            ingest_one(
                &alt.current,
                &slot_key,
                project_id,
                store,
                registry,
                now,
                &mut result,
            );
        }
    }

    let lineages = discover_ableton_lineages(root);
    result.ableton_lineages_found = lineages.len();
    for lineage in &lineages {
        // Ableton lineages aren't a single bundle directory the way a
        // .logicx is, so the "bundle_path" key is synthesized from the
        // lineage name plus its parent directory (stable across rescans,
        // unique across lineages of the same name in different folders).
        let parent = lineage
            .saves
            .first()
            .and_then(|p| p.parent())
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        let bundle_key = format!("{parent}::{}", lineage.name);
        let Ok(project_id) = registry.upsert_project(&lineage.name, &bundle_key, "ableton") else {
            result.read_errors += 1;
            continue;
        };
        for save_path in &lineage.saves {
            let slot_key = save_path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "unknown".to_string());
            ingest_one(
                save_path,
                &slot_key,
                project_id,
                store,
                registry,
                now,
                &mut result,
            );
        }
    }

    result
}

fn ingest_one(
    path: &Path,
    slot_key: &str,
    project_id: i64,
    store: &Store,
    registry: &Registry,
    now: i64,
    result: &mut ScanResult,
) {
    let Ok(bytes) = std::fs::read(path) else {
        result.read_errors += 1;
        return;
    };
    let Ok(hash) = store.ingest_bytes(&bytes) else {
        result.read_errors += 1;
        return;
    };
    // Always ingest byte-different saves — retention is never decided by
    // a semantic verdict (M3 guardrail). record_version_if_new already
    // implements exactly that: it inserts iff this (project, slot, hash)
    // triple hasn't been seen, with no verdict computation anywhere in
    // the path.
    if registry
        .record_version_if_new(project_id, slot_key, hash, now)
        .unwrap_or(false)
    {
        result.new_versions_ingested += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(path: &Path, bytes: &[u8]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    fn setup() -> (tempfile::TempDir, tempfile::TempDir, tempfile::TempDir) {
        (
            tempfile::tempdir().unwrap(),
            tempfile::tempdir().unwrap(),
            tempfile::tempdir().unwrap(),
        )
    }

    #[test]
    fn scan_discovers_and_ingests_a_logic_project_with_backups() {
        let (library, store_dir, db_dir) = setup();
        let bundle = library.path().join("Song.logicx");
        touch(
            &bundle.join("Alternatives/000/ProjectData"),
            b"current save bytes",
        );
        touch(
            &bundle.join("Alternatives/000/Project File Backups/00/ProjectData"),
            b"backup 00 bytes",
        );
        touch(
            &bundle.join("Alternatives/000/Project File Backups/01/ProjectData"),
            b"backup 01 bytes",
        );

        let store = Store::open(store_dir.path()).unwrap();
        let registry = Registry::open(db_dir.path().join("wit.db")).unwrap();
        let result = scan(library.path(), &store, &registry, 1000);

        assert_eq!(result.logic_projects_found, 1);
        assert_eq!(result.new_versions_ingested, 3);
        assert_eq!(result.read_errors, 0);

        let projects = registry.list_projects().unwrap();
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].version_count, 3);
    }

    #[test]
    fn rescanning_unchanged_files_is_idempotent() {
        let (library, store_dir, db_dir) = setup();
        let bundle = library.path().join("Song.logicx");
        touch(
            &bundle.join("Alternatives/000/ProjectData"),
            b"current save bytes",
        );

        let store = Store::open(store_dir.path()).unwrap();
        let registry = Registry::open(db_dir.path().join("wit.db")).unwrap();

        let first = scan(library.path(), &store, &registry, 1000);
        assert_eq!(first.new_versions_ingested, 1);

        let second = scan(library.path(), &store, &registry, 2000);
        assert_eq!(
            second.new_versions_ingested, 0,
            "rescan must not double-count an unchanged file"
        );

        let projects = registry.list_projects().unwrap();
        assert_eq!(projects[0].version_count, 1);
    }

    #[test]
    fn a_new_backup_slot_between_scans_is_ingested_without_disturbing_the_others() {
        let (library, store_dir, db_dir) = setup();
        let bundle = library.path().join("Song.logicx");
        touch(&bundle.join("Alternatives/000/ProjectData"), b"v1");

        let store = Store::open(store_dir.path()).unwrap();
        let registry = Registry::open(db_dir.path().join("wit.db")).unwrap();
        scan(library.path(), &store, &registry, 1000);

        // Logic wrote a new save (current ProjectData changed content).
        touch(
            &bundle.join("Alternatives/000/ProjectData"),
            b"v2 - different bytes",
        );
        let result = scan(library.path(), &store, &registry, 2000);
        assert_eq!(result.new_versions_ingested, 1);

        let projects = registry.list_projects().unwrap();
        assert_eq!(
            projects[0].version_count, 2,
            "both v1 and v2 must be preserved — archive-before-recycle"
        );
    }

    #[test]
    fn scan_discovers_and_ingests_an_ableton_lineage() {
        let (library, store_dir, db_dir) = setup();
        touch(
            &library.path().join("Song [2026-05-05 095412].als"),
            b"save 1",
        );
        touch(
            &library.path().join("Song [2026-05-05 095508].als"),
            b"save 2",
        );

        let store = Store::open(store_dir.path()).unwrap();
        let registry = Registry::open(db_dir.path().join("wit.db")).unwrap();
        let result = scan(library.path(), &store, &registry, 1000);

        assert_eq!(result.ableton_lineages_found, 1);
        assert_eq!(result.new_versions_ingested, 2);
    }

    #[test]
    fn source_files_are_never_modified_by_a_scan() {
        let (library, store_dir, db_dir) = setup();
        let project_data = library
            .path()
            .join("Song.logicx/Alternatives/000/ProjectData");
        touch(&project_data, b"original bytes - must survive untouched");
        let fingerprint_before = crate::store::Hash::of(&std::fs::read(&project_data).unwrap());

        let store = Store::open(store_dir.path()).unwrap();
        let registry = Registry::open(db_dir.path().join("wit.db")).unwrap();
        scan(library.path(), &store, &registry, 1000);
        scan(library.path(), &store, &registry, 2000); // rescan too

        let fingerprint_after = crate::store::Hash::of(&std::fs::read(&project_data).unwrap());
        assert_eq!(fingerprint_before, fingerprint_after);
    }
}
