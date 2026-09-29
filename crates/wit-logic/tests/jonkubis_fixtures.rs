//! The M2 gate: walk every `ProjectData` in the MIT-licensed
//! `jonkubis/LogicProFormatWriter` corpus, fetched at run time at a pinned
//! commit, and check what Wit extracts against the ground truth Logic itself
//! wrote next to each one. **Loudly skipped** unless `WIT_JONKUBIS_DIR`
//! points at a checkout; the files are never committed (CI refuses
//! `ProjectData` paths).
//!
//! ```text
//! just logic-fixtures-gate
//! # or by hand:
//! git clone https://github.com/jonkubis/LogicProFormatWriter /tmp/lpfw
//! git -C /tmp/lpfw checkout 1f77c5c37d49ccd9551cc8e9107750e8db2f1fed
//! WIT_JONKUBIS_DIR=/tmp/lpfw cargo test -p wit-logic --test jonkubis_fixtures -- --ignored --nocapture
//! ```
//!
//! **The material.** At the pinned commit: 87 `.logicx` bundles holding 113
//! `ProjectData` files. Upstream describes them as sessions made in Logic
//! ("donor sessions", `DONORS.md`); every bundle's
//! `ProjectInformation.plist` records Logic Pro 11.2.2 as the app that last
//! saved it, and at least one is an upstream export that was then opened in
//! Logic. So these are Logic-written files from one Logic version, not a
//! sample of real users' libraries.
//!
//! **What it asserts**, for every `ProjectData`:
//! - the container walks to a clean end of file (a typed walk error fails
//!   the gate; extraction itself cannot error, so the gate also shows it
//!   doesn't panic);
//! - where macOS `plutil` is available (it reads Logic's `MetaData.plist`,
//!   the ground truth beside each save): the extracted tempo equals
//!   `BeatsPerMinute`, and every extracted audio file name is in
//!   `AudioFiles`.
//!
//! When the checkout is at the pinned commit it also asserts the file count,
//! so a partial checkout can't pass.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// The upstream commit this gate is pinned to (also cited in frame.rs).
const PINNED_SHA: &str = "1f77c5c37d49ccd9551cc8e9107750e8db2f1fed";
/// `ProjectData` files at [`PINNED_SHA`] (measured 2026-09-29).
const PINNED_FILE_COUNT: usize = 113;

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

fn checkout_head(dir: &Path) -> Option<String> {
    let out = std::process::Command::new("git")
        .args(["-C"])
        .arg(dir)
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// `plutil -extract <key> <fmt> -o - <plist>` → stdout, or `None` if plutil
/// is missing or the key isn't there.
fn plutil(plist: &Path, key: &str, fmt: &str) -> Option<String> {
    let out = std::process::Command::new("plutil")
        .args(["-extract", key, fmt, "-o", "-"])
        .arg(plist)
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn plutil_available() -> bool {
    std::process::Command::new("plutil")
        .arg("-help")
        .output()
        .is_ok()
}

#[test]
#[ignore = "opt-in: set WIT_JONKUBIS_DIR to a LogicProFormatWriter checkout at the pinned commit and pass --ignored"]
fn every_upstream_fixture_walks_clean_and_matches_logics_own_metadata() {
    let Some(dir) = std::env::var_os("WIT_JONKUBIS_DIR").map(PathBuf::from) else {
        eprintln!(
            "WIT_JONKUBIS_DIR not set — skipped. Run `just logic-fixtures-gate` to fetch the \
             corpus at {PINNED_SHA} into a temp dir and run this gate."
        );
        return;
    };

    let head = checkout_head(&dir);
    let mut files = Vec::new();
    find_project_data(&dir, &mut files, 0);
    assert!(!files.is_empty(), "no ProjectData files under {dir:?}");
    if head.as_deref() == Some(PINNED_SHA) {
        assert_eq!(
            files.len(),
            PINNED_FILE_COUNT,
            "the pinned commit holds {PINNED_FILE_COUNT} ProjectData files; a partial checkout?"
        );
    }

    let ground_truth = plutil_available();
    let mut failures = Vec::new();
    let mut versions: BTreeMap<String, usize> = BTreeMap::new();
    let (mut tempo_checked, mut audio_checked) = (0, 0);
    for f in &files {
        let rel = f.strip_prefix(&dir).unwrap_or(f).display().to_string();
        let w = match wit_logic::walk_file(f) {
            Ok(w) => w,
            Err(e) => {
                failures.push(format!("{rel}: walk failed: {e:?}"));
                continue;
            }
        };
        let word = format!(
            "{:02x}{:02x}",
            w.root.version_word[0], w.root.version_word[1]
        );
        *versions.entry(word).or_insert(0) += 1;

        let plist = f.with_file_name("MetaData.plist");
        if !(ground_truth && plist.is_file()) {
            continue;
        }
        if let Some(bpm) =
            plutil(&plist, "BeatsPerMinute", "raw").and_then(|s| s.parse::<f64>().ok())
        {
            tempo_checked += 1;
            match w.extracted.tempo_bpm {
                Some(t) if (t - bpm).abs() < 1e-3 => {}
                other => {
                    failures.push(format!("{rel}: tempo {other:?}, MetaData.plist says {bpm}"))
                }
            }
        }
        if let Some(json) = plutil(&plist, "AudioFiles", "json") {
            let listed: Vec<String> = serde_json::from_str(&json).unwrap_or_default();
            let names: BTreeSet<String> = listed
                .iter()
                .map(|p| p.rsplit('/').next().unwrap_or(p).to_string())
                .collect();
            audio_checked += 1;
            for name in &w.extracted.audio_file_names {
                if !names.contains(name) {
                    failures.push(format!(
                        "{rel}: extracted audio file {name:?} is not in MetaData.plist AudioFiles"
                    ));
                }
            }
        }
    }

    eprintln!(
        "jonkubis/LogicProFormatWriter checkout at {} (gate pinned to {PINNED_SHA})",
        head.as_deref().unwrap_or("an unknown commit")
    );
    eprintln!(
        "  ProjectData files: {}; version words: {versions:?}",
        files.len()
    );
    if ground_truth {
        eprintln!(
            "  checked against MetaData.plist: tempo on {tempo_checked}, audio file names on \
             {audio_checked}"
        );
    } else {
        eprintln!("  plutil not available — ground-truth checks skipped (walk only)");
    }
    assert!(
        failures.is_empty(),
        "{} problem(s) across {} fixtures:\n{}",
        failures.len(),
        files.len(),
        failures.join("\n")
    );
}
