//! Opt-in real-material check for `logic_report` — M2.5 (issue #15), the
//! reality gate. Mirrors the `WIT_FIXTURES`/`WIT_LOGIC_PROJECT` discipline
//! (`wit-diff/tests/real_fixtures.rs`, `wit-logic/tests/real_fixtures.rs`):
//! **loudly skipped** by default, never touches real material unless asked.
//!
//! `WIT_LOGIC_PROJECT` (in `wit-logic`) points at a single `.logicx`/`.band`
//! bundle. This test's `WIT_LOGIC_LIBRARY` points at a **library root** —
//! a directory that may contain many such bundles — because issue #15
//! explicitly asks for the statistics across a whole library, not one
//! project.
//!
//! Run against a real Logic library:
//!
//! ```text
//! WIT_LOGIC_LIBRARY="/path/to/YourLibrary" cargo test -p wit-index --test real_fixtures -- --nocapture --ignored
//! ```
//!
//! The published number in `docs/EXPERIMENTS.md` was produced by this exact
//! command against **one** real project (n=1), not the 30-project / 26 GB
//! library the issue asks for — that run still needs to happen on a machine
//! that has it. See the EXPERIMENTS.md entry for the honestly-labeled n=1
//! result and what it does and does not answer.

use std::path::PathBuf;

fn library_root() -> Option<PathBuf> {
    std::env::var_os("WIT_LOGIC_LIBRARY").map(PathBuf::from)
}

#[test]
#[ignore = "opt-in: set WIT_LOGIC_LIBRARY to a real Logic library root and pass --ignored"]
fn real_library_reports_the_three_m2_5_statistics() {
    let Some(root) = library_root() else {
        eprintln!(
            "WIT_LOGIC_LIBRARY not set — skipped. To run: \
             WIT_LOGIC_LIBRARY=/path/to/YourLibrary cargo test -p wit-index --test real_fixtures -- --nocapture --ignored"
        );
        return;
    };

    let report = wit_index::logic_report(&root);

    assert!(
        report.projects_scanned > 0,
        "no Logic/GarageBand project found under WIT_LOGIC_LIBRARY={root:?}"
    );

    eprintln!(
        "scanned {} project(s), {} alternative(s), {} consecutive save pair(s)",
        report.projects_scanned,
        report.alternatives_scanned,
        report.total_pairs()
    );

    if report.total_pairs() == 0 {
        eprintln!("no consecutive save pairs found — nothing more to report");
        return;
    }

    eprintln!(
        "{:.1}% of save pairs show a structural change Wit can see ({} of {})",
        report.structural_change_percent(),
        report.pairs_with_structural_change(),
        report.total_pairs()
    );
    eprintln!("distribution of change counts per save pair:");
    for (count, n) in report.change_count_distribution() {
        eprintln!("  {count} change(s): {n} pair(s)");
    }
    let byte_different_but_same = report.byte_different_structurally_identical();
    eprintln!(
        "{byte_different_but_same} pair(s) ({:.1}%) are byte-different but structurally identical",
        byte_different_but_same as f64 / report.total_pairs() as f64 * 100.0
    );
    if !report.read_errors.is_empty() {
        eprintln!(
            "{} ProjectData file(s) could not be read or walked",
            report.read_errors.len()
        );
    }

    // Sanity, not a correctness assertion about the *content* of the
    // library (issue #15's exit criterion is a judgment call for a human,
    // not something this test decides): every percentage must be a real
    // percentage, and the distribution must account for every pair.
    assert!((0.0..=100.0).contains(&report.structural_change_percent()));
    let distributed: usize = report
        .change_count_distribution()
        .iter()
        .map(|(_, n)| n)
        .sum();
    assert_eq!(distributed, report.total_pairs());
}
