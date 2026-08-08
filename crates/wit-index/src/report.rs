//! `wit logic-report` (M2.5, [issue #15](https://github.com/sep-lab/Wit/issues/15)):
//! the library-wide "reality gate". Walks every discovered Logic/GarageBand
//! alternative's chain — `Project File Backups/00`–`09` (oldest first) then
//! the current `ProjectData`, matching [`LogicAlternative`]'s own field
//! order — compares every consecutive pair at `wit-logic`'s Structure
//! honesty tier, and reports the three statistics the issue asks for:
//!
//! - % of saves with **any** structural change Wit can see
//!   ([`LogicLibraryReport::structural_change_percent`])
//! - distribution of change counts per save
//!   ([`LogicLibraryReport::change_count_distribution`], via
//!   [`wit_logic::change_count`])
//! - how often two adjacent saves are byte-different but structurally
//!   identical ([`LogicLibraryReport::byte_different_structurally_identical`])
//!
//! Read-only — this only reads `ProjectData` bytes (via `wit_logic::walk_file`
//! and a plain `std::fs::read` for the byte-identity check) and never writes
//! or modifies anything under the library root, the same discipline
//! `discover.rs` and `dupes.rs` already follow.

use crate::discover::discover_logic_projects;
use std::path::{Path, PathBuf};

/// One consecutive-pair comparison within a single alternative's chain.
#[derive(Debug, Clone, PartialEq)]
pub struct SavePairResult {
    pub project_name: String,
    pub alternative_name: String,
    pub older: PathBuf,
    pub newer: PathBuf,
    pub structural_change: bool,
    /// `wit_logic::change_count` — always `0` when `structural_change` is
    /// `false` (see that function's doc comment for why).
    pub change_count: usize,
    pub bytes_identical: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct LogicLibraryReport {
    pub projects_scanned: usize,
    pub alternatives_scanned: usize,
    pub pairs: Vec<SavePairResult>,
    /// `ProjectData` paths that failed to read or failed to walk. Excluded
    /// from every pair touching them rather than silently dropped from
    /// the count — one bad file must not blank the whole report (mirrors
    /// `duplicate_report`'s and `wit-index::scan`'s read-error handling).
    pub read_errors: Vec<PathBuf>,
}

impl LogicLibraryReport {
    pub fn total_pairs(&self) -> usize {
        self.pairs.len()
    }

    pub fn pairs_with_structural_change(&self) -> usize {
        self.pairs.iter().filter(|p| p.structural_change).count()
    }

    /// Issue #15's first published statistic: % of saves with *any*
    /// structural change Wit can see. `0.0` on an empty report rather than
    /// `NaN` — mirrors `DuplicateReport::duplicate_percent`.
    pub fn structural_change_percent(&self) -> f64 {
        if self.pairs.is_empty() {
            0.0
        } else {
            self.pairs_with_structural_change() as f64 / self.pairs.len() as f64 * 100.0
        }
    }

    /// Issue #15's third statistic: adjacent saves that differ byte-for-byte
    /// but walk to the same Structure-tier verdict — the case that makes
    /// raw byte comparison useless as a change signal on this format (see
    /// `wit-logic`'s module docs).
    pub fn byte_different_structurally_identical(&self) -> usize {
        self.pairs
            .iter()
            .filter(|p| !p.bytes_identical && !p.structural_change)
            .count()
    }

    /// Issue #15's second statistic: how many pairs land at each
    /// `change_count`, ascending by count. Bucket `0` holds every pair
    /// `semantic_equal` called `NoStructuralChange`.
    pub fn change_count_distribution(&self) -> Vec<(usize, usize)> {
        let mut buckets: std::collections::BTreeMap<usize, usize> =
            std::collections::BTreeMap::new();
        for pair in &self.pairs {
            *buckets.entry(pair.change_count).or_insert(0) += 1;
        }
        buckets.into_iter().collect()
    }
}

/// Scan `root` for Logic/GarageBand bundles and report M2.5's three
/// statistics across every alternative's chain.
pub fn logic_report(root: &Path) -> LogicLibraryReport {
    let mut report = LogicLibraryReport::default();
    let projects = discover_logic_projects(root);
    report.projects_scanned = projects.len();

    for project in &projects {
        for alt in &project.alternatives {
            report.alternatives_scanned += 1;

            let mut chain: Vec<&PathBuf> = alt.backups.iter().collect();
            chain.push(&alt.current);

            let mut walks: Vec<(&PathBuf, wit_logic::Walked)> = Vec::new();
            for path in chain {
                match wit_logic::walk_file(path) {
                    Ok(w) => walks.push((path, w)),
                    Err(_) => report.read_errors.push(path.clone()),
                }
            }

            for window in walks.windows(2) {
                let (path_a, a) = &window[0];
                let (path_b, b) = &window[1];
                let structural_change =
                    wit_logic::semantic_equal(a, b) == wit_logic::Verdict::StructuralChange;
                let change_count = wit_logic::change_count(a, b);
                let bytes_identical = wit_logic::bytes_equal(
                    &std::fs::read(path_a).unwrap_or_default(),
                    &std::fs::read(path_b).unwrap_or_default(),
                );
                report.pairs.push(SavePairResult {
                    project_name: project.name.clone(),
                    alternative_name: alt.name.clone(),
                    older: (*path_a).clone(),
                    newer: (*path_b).clone(),
                    structural_change,
                    change_count,
                    bytes_identical,
                });
            }
        }
    }

    report
}

#[cfg(test)]
mod tests {
    use super::*;

    // Minimal ProjectData container builder — same layout as
    // `wit-logic/src/frame.rs` documents (magic, version word, root
    // LENGTH, then a flat record sequence), reimplemented here because
    // `wit-logic`'s own builder is private to its crate's test module.
    const MAGIC: [u8; 4] = [0x23, 0x47, 0xC0, 0xAB];
    const RECORD_HEADER_LEN: usize = 0x24;

    fn build_container(records: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
        let mut payload = Vec::new();
        for (tag, rec_payload) in records {
            payload.extend_from_slice(*tag);
            payload.extend_from_slice(&[0u8; 0x1c - 4]);
            payload.extend_from_slice(&(rec_payload.len() as u32).to_le_bytes());
            payload.extend_from_slice(&[0u8; RECORD_HEADER_LEN - 0x20]);
            payload.extend_from_slice(rec_payload);
        }
        let mut out = Vec::new();
        out.extend_from_slice(&MAGIC);
        out.extend_from_slice(&[0xd0, 0x09]);
        out.extend_from_slice(&[0u8; 10]);
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        out.extend_from_slice(&[0u8; 4]);
        out.extend_from_slice(&payload);
        out
    }

    fn write(path: &Path, bytes: &[u8]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    #[test]
    fn computes_the_three_statistics_on_a_crafted_two_pair_chain() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = dir.path().join("Song.logicx");

        // v1 and v2: same tag census (byte-different filler, same length)
        // -> NoStructuralChange, change_count 0, bytes differ.
        let v1 = build_container(&[(b"karT", vec![0xAA; 8])]);
        let v2 = build_container(&[(b"karT", vec![0xBB; 8])]);
        // v3: adds a new record tag -> StructuralChange, change_count >= 1.
        let v3 = build_container(&[(b"karT", vec![0xBB; 8]), (b"gRuA", vec![0u8; 8])]);
        assert_ne!(v1, v2);

        write(
            &bundle.join("Alternatives/000/Project File Backups/00/ProjectData"),
            &v1,
        );
        write(
            &bundle.join("Alternatives/000/Project File Backups/01/ProjectData"),
            &v2,
        );
        write(&bundle.join("Alternatives/000/ProjectData"), &v3);

        let report = logic_report(dir.path());

        assert_eq!(report.projects_scanned, 1);
        assert_eq!(report.alternatives_scanned, 1);
        assert_eq!(report.total_pairs(), 2);
        assert!(report.read_errors.is_empty());

        // Pair 1 (00 -> 01): byte-different, structurally identical.
        // Pair 2 (01 -> current): structural change.
        assert_eq!(report.pairs_with_structural_change(), 1);
        assert_eq!(report.structural_change_percent(), 50.0);
        assert_eq!(report.byte_different_structurally_identical(), 1);

        let distribution = report.change_count_distribution();
        assert_eq!(distribution[0], (0, 1)); // pair 1: change_count 0
        assert!(distribution.iter().any(|&(count, n)| count >= 1 && n == 1)); // pair 2
    }

    #[test]
    fn an_unreadable_projectdata_is_recorded_as_a_read_error_not_a_panic() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = dir.path().join("Song.logicx");
        // Not a valid ProjectData container -- walk_file must fail cleanly.
        write(
            &bundle.join("Alternatives/000/ProjectData"),
            b"not a real ProjectData file",
        );

        let report = logic_report(dir.path());
        assert_eq!(report.projects_scanned, 1);
        assert_eq!(report.total_pairs(), 0);
        assert_eq!(report.read_errors.len(), 1);
    }

    #[test]
    fn an_empty_library_reports_zero_everything_not_nan() {
        let dir = tempfile::tempdir().unwrap();
        let report = logic_report(dir.path());
        assert_eq!(report.projects_scanned, 0);
        assert_eq!(report.total_pairs(), 0);
        assert_eq!(report.structural_change_percent(), 0.0);
        assert_eq!(report.byte_different_structurally_identical(), 0);
        assert!(report.change_count_distribution().is_empty());
    }
}
