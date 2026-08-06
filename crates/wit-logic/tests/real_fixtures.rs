//! Opt-in real-material check against a real `.logicx` bundle — mirrors the
//! `WIT_FIXTURES` discipline (`tests/conftest.py`, `wit-diff/tests/real_fixtures.rs`):
//! **loudly skipped** by default, never touches real material unless asked.
//!
//! Run against a real Logic project:
//!
//! ```text
//! WIT_LOGIC_PROJECT="/path/to/Song.logicx" cargo test -p wit-logic --test real_fixtures -- --nocapture --ignored
//! ```
//!
//! This is a **single real project's spot check**, not the 30-fixture
//! `jonkubis/LogicProFormatWriter` corpus the M2 issue asks for as the
//! mechanically-checkable gate — that corpus fetch is a separate follow-up
//! (network-gated, opt-in, never committed; see the issue for the pinned SHA).

use std::path::{Path, PathBuf};

fn project_path() -> Option<PathBuf> {
    std::env::var_os("WIT_LOGIC_PROJECT").map(PathBuf::from)
}

fn backup_chain(bundle: &Path) -> Vec<PathBuf> {
    let mut chain = Vec::new();
    let current = bundle.join("Alternatives/000/ProjectData");
    let backups_dir = bundle.join("Alternatives/000/Project File Backups");
    if let Ok(entries) = std::fs::read_dir(&backups_dir) {
        let mut backup_dirs: Vec<PathBuf> =
            entries.filter_map(|e| e.ok()).map(|e| e.path()).collect();
        backup_dirs.sort();
        for dir in backup_dirs {
            let pd = dir.join("ProjectData");
            if pd.exists() {
                chain.push(pd);
            }
        }
    }
    if current.exists() {
        chain.push(current);
    }
    chain
}

#[test]
#[ignore = "opt-in: set WIT_LOGIC_PROJECT to a real .logicx bundle path and pass --ignored"]
fn real_project_walks_clean_and_tempo_matches_metadata_plist() {
    let Some(bundle) = project_path() else {
        eprintln!(
            "WIT_LOGIC_PROJECT not set — skipped. To run: \
             WIT_LOGIC_PROJECT=/path/to/Song.logicx cargo test -p wit-logic --test real_fixtures -- --nocapture --ignored"
        );
        return;
    };

    let chain = backup_chain(&bundle);
    assert!(
        !chain.is_empty(),
        "no ProjectData files found under {bundle:?}"
    );

    let mut walks = Vec::new();
    for path in &chain {
        let bytes = std::fs::read(path).unwrap();
        match wit_logic::walk(&bytes) {
            Ok(w) => {
                eprintln!(
                    "  {}: version {:02x?}, {} tags, tempo={:?}, {} track name(s), {} region name(s), {} audio file(s)",
                    path.display(),
                    w.root.version_word,
                    w.census.len(),
                    w.extracted.tempo_bpm,
                    w.extracted.possible_track_names.len(),
                    w.extracted.region_names.len(),
                    w.extracted.audio_file_names.len(),
                );
                walks.push((path.clone(), w));
            }
            Err(e) => panic!("walk failed on real file {path:?}: {e}"),
        }
    }

    // Cross-check tempo against MetaData.plist, if present and if `plutil`
    // is available (macOS-only tool; the whole test is opt-in and
    // local-machine-only anyway).
    let metadata_plist = bundle.join("Alternatives/000/MetaData.plist");
    if metadata_plist.exists() {
        if let Ok(output) = std::process::Command::new("plutil")
            .args(["-extract", "BeatsPerMinute", "raw", "-o", "-"])
            .arg(&metadata_plist)
            .output()
        {
            if output.status.success() {
                let bpm_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if let Ok(ground_truth_bpm) = bpm_str.parse::<f64>() {
                    let (_, current_walk) = walks.last().unwrap();
                    eprintln!(
                        "  MetaData.plist BeatsPerMinute = {ground_truth_bpm}, extracted tempo_bpm = {:?}",
                        current_walk.extracted.tempo_bpm
                    );
                    assert_eq!(
                        current_walk.extracted.tempo_bpm,
                        Some(ground_truth_bpm),
                        "extracted tempo must match MetaData.plist ground truth"
                    );
                }
            }
        }
    }

    // Structural-equality spot check across the chain: report which
    // consecutive pairs are NoStructuralChange vs StructuralChange.
    let mut no_change = 0;
    let mut changed = 0;
    for pair in walks.windows(2) {
        let (path_a, a) = &pair[0];
        let (path_b, b) = &pair[1];
        let verdict = wit_logic::semantic_equal(a, b);
        eprintln!(
            "  {} -> {}: {verdict:?}",
            path_a.file_name().unwrap().to_string_lossy(),
            path_b.file_name().unwrap().to_string_lossy()
        );
        match verdict {
            wit_logic::Verdict::NoStructuralChange => no_change += 1,
            wit_logic::Verdict::StructuralChange => changed += 1,
        }
    }
    eprintln!(
        "\n{} pair(s): {no_change} no-structural-change, {changed} structural-change",
        walks.len().saturating_sub(1)
    );
}
