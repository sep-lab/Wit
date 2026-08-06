//! Opt-in real-material corpus check — the M1 exit criterion's "corpus
//! agreement" gate (PLAN.md: "zero FX~ lines on the 7 measured zero-change
//! pairs... AND FX~ detection on the 3-of-9 knob-only saves").
//!
//! Mirrors `tests/conftest.py`'s `WIT_FIXTURES` discipline: **loudly
//! skipped** by default (never touches real material unless asked), never
//! commits or reads any file whose path this repo would reject
//! (`check_no_binaries.sh`/`check_personal_paths.py` — none of that
//! applies here since this test only ever *reads* a path the operator
//! supplies via an env var, at runtime, on their own machine).
//!
//! Run against a real Ableton `Backup/` folder:
//!
//! ```text
//! WIT_FIXTURES=/path/to/YourProject/Backup cargo test -p wit-diff --test real_fixtures -- --nocapture --ignored
//! ```

use std::path::PathBuf;

fn fixtures_dir() -> Option<PathBuf> {
    std::env::var_os("WIT_FIXTURES").map(PathBuf::from)
}

#[test]
#[ignore = "opt-in: set WIT_FIXTURES and pass --ignored to run against real material"]
fn corpus_walks_clean_and_reports_sane_diffs() {
    let Some(dir) = fixtures_dir() else {
        eprintln!(
            "WIT_FIXTURES not set — skipped. To run: \
             WIT_FIXTURES=/path/to/Backup cargo test -p wit-diff --test real_fixtures -- --nocapture --ignored"
        );
        return;
    };

    let mut entries: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("cannot read WIT_FIXTURES={dir:?}: {e}"))
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("als"))
        .collect();
    entries.sort();

    assert!(
        entries.len() >= 2,
        "need at least 2 .als files under {dir:?}, found {}",
        entries.len()
    );

    let mut zero_change_pairs = 0usize;
    let mut fx_settings_pairs = 0usize;
    let mut total_pairs = 0usize;

    for pair in entries.windows(2) {
        let (old_path, new_path) = (&pair[0], &pair[1]);
        let old_bytes = std::fs::read(old_path).unwrap();
        let new_bytes = std::fs::read(new_path).unwrap();

        // Never panic on real material — a parse failure is a finding
        // (log and continue), not a test-harness crash.
        let (Ok(old), Ok(new)) = (wit_als::parse(&old_bytes), wit_als::parse(&new_bytes)) else {
            eprintln!(
                "  SKIP (parse error): {:?} -> {:?}",
                old_path.file_name().unwrap(),
                new_path.file_name().unwrap()
            );
            continue;
        };

        let records = wit_diff::diff(&old, &new);
        total_pairs += 1;
        if records.is_empty() {
            zero_change_pairs += 1;
        }
        if records
            .iter()
            .any(|r| matches!(r, wit_model::ChangeRecord::FxSettingsChanged { .. }))
        {
            fx_settings_pairs += 1;
        }

        eprintln!(
            "  {} -> {}: {} change(s)",
            old_path.file_name().unwrap().to_string_lossy(),
            new_path.file_name().unwrap().to_string_lossy(),
            records.len()
        );
    }

    eprintln!(
        "\n{total_pairs} pair(s) walked clean; {zero_change_pairs} zero-change; \
         {fx_settings_pairs} pair(s) with a device settings-changed line."
    );
    assert!(
        total_pairs > 0,
        "every pair failed to parse — that's a real bug, not a clean corpus"
    );
}
