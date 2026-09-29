//! The M2 gate: walk every real Logic save in the MIT-licensed
//! `jonkubis/LogicProFormatWriter` corpus, fetched at run time at a pinned
//! commit. **Loudly skipped** unless `WIT_JONKUBIS_DIR` points at a checkout;
//! the files are never committed (CI refuses `ProjectData` paths).
//!
//! ```text
//! just logic-fixtures-gate
//! # or by hand:
//! git clone https://github.com/jonkubis/LogicProFormatWriter /tmp/lpfw
//! git -C /tmp/lpfw checkout 1f77c5c37d49ccd9551cc8e9107750e8db2f1fed
//! WIT_JONKUBIS_DIR=/tmp/lpfw cargo test -p wit-logic --test jonkubis_fixtures -- --ignored --nocapture
//! ```
//!
//! What it asserts, for every `ProjectData` found anywhere under the
//! checkout: the container walks to a clean end of file, and extraction
//! returns (no panic, no error). What it reports, not asserts: the version
//! words seen and how many saves yielded a tempo, a track-list name, a region
//! name and an audio file name. Those counts describe this corpus (Logic
//! 11.x fixtures written by the upstream project's own tooling), not real
//! users' libraries.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The upstream commit this gate is pinned to (also cited in frame.rs).
const PINNED_SHA: &str = "1f77c5c37d49ccd9551cc8e9107750e8db2f1fed";

fn find_project_data(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    if depth > 12 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries.filter_map(|e| e.ok()).map(|e| e.path()).collect();
    paths.sort();
    for p in paths {
        if p.file_name().is_some_and(|n| n == ".git") {
            continue;
        }
        if p.is_dir() {
            find_project_data(&p, out, depth + 1);
        } else if p.file_name().is_some_and(|n| n == "ProjectData") {
            out.push(p);
        }
    }
}

#[test]
#[ignore = "opt-in: set WIT_JONKUBIS_DIR to a LogicProFormatWriter checkout at the pinned commit and pass --ignored"]
fn every_upstream_fixture_walks_clean() {
    let Some(dir) = std::env::var_os("WIT_JONKUBIS_DIR").map(PathBuf::from) else {
        eprintln!(
            "WIT_JONKUBIS_DIR not set — skipped. Run `just logic-fixtures-gate` to fetch the \
             corpus at {PINNED_SHA} into a temp dir and run this gate."
        );
        return;
    };

    let mut files = Vec::new();
    find_project_data(&dir, &mut files, 0);
    assert!(!files.is_empty(), "no ProjectData files under {dir:?}");

    let mut failures = Vec::new();
    let mut versions: BTreeMap<String, usize> = BTreeMap::new();
    let (mut tempo, mut names, mut regions, mut audio) = (0, 0, 0, 0);
    for f in &files {
        let rel = f.strip_prefix(&dir).unwrap_or(f).display().to_string();
        match wit_logic::walk_file(f) {
            Ok(w) => {
                let word = format!(
                    "{:02x}{:02x}",
                    w.root.version_word[0], w.root.version_word[1]
                );
                *versions.entry(word).or_insert(0) += 1;
                tempo += usize::from(w.extracted.tempo_bpm.is_some());
                names += usize::from(!w.extracted.possible_track_names.is_empty());
                regions += usize::from(!w.extracted.region_names.is_empty());
                audio += usize::from(!w.extracted.audio_file_names.is_empty());
            }
            Err(e) => failures.push(format!("{rel}: {e:?}")),
        }
    }

    eprintln!("jonkubis/LogicProFormatWriter @ {PINNED_SHA}");
    eprintln!(
        "  ProjectData files: {}, walked clean: {}",
        files.len(),
        files.len() - failures.len()
    );
    eprintln!("  version words: {versions:?}");
    eprintln!(
        "  with a tempo: {tempo}, a track-list name: {names}, a region name: {regions}, \
         an audio file name: {audio}"
    );
    assert!(
        failures.is_empty(),
        "{} of {} fixtures failed to walk:\n{}",
        failures.len(),
        files.len(),
        failures.join("\n")
    );
}
