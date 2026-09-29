//! PLAN-V2 (2026-09-29, "Phase C → Ableton") real-material checks:
//! locators, scale and time signature parse on every file in the real
//! chain, and the master-bus plugin toggle this wave fixed
//! (`wit-als/src/extract.rs`'s module doc: "a real, reviewed save toggled a
//! mastering plugin off on the master bus, and Wit said nothing") now
//! reports a change.
//!
//! Mirrors `real_fixtures.rs`'s and `corpus_agreement.rs`'s `WIT_FIXTURES`
//! discipline: loudly skipped by default, read-only, never touches
//! anything the operator did not point it at. Grouped by chain
//! (`corpus_agreement.rs`'s own reasoning: a flat cross-song sort mixes
//! different songs into nonsense pairs), and — because this repository is
//! public — **no song name is ever printed or asserted on**: the
//! master-bus pair is identified by its position and timestamp only.
//!
//! ```text
//! WIT_FIXTURES=/path/to/Backup cargo test -p wit-diff --test real_fixtures_plan_v2 -- --ignored --nocapture
//! ```
//!
//! If the master-bus assertion fails on a *different* real chain than the
//! one this was written against (one with no master-bus plugin at all),
//! that is an expected, honest gap in that corpus, not a bug — see the
//! assertion's own message.

use std::collections::HashMap;
use std::path::PathBuf;
use wit_model::ChangeRecord;

fn fixtures_dir() -> Option<PathBuf> {
    std::env::var_os("WIT_FIXTURES").map(PathBuf::from)
}

/// Split `"<name> [<timestamp>].als"` into `(name, timestamp)`. Copied from
/// `corpus_agreement.rs` rather than shared — `tests/*.rs` files each build
/// as their own binary — and, unlike that copy, the name is discarded
/// immediately after grouping and never appears in a message here.
fn split_backup_name(filename: &str) -> Option<(&str, &str)> {
    let stem = filename.strip_suffix(".als")?;
    let open = stem.rfind('[')?;
    let close = stem.rfind(']')?;
    if close != stem.len() - 1 || close <= open {
        return None;
    }
    Some((stem[..open].trim_end(), &stem[open + 1..close]))
}

/// Whether a change record touches the named bus/track — Fx* records carry
/// a plain `String`, Plugin* records an `Option<String>`, so this is the
/// one place that difference is papered over.
fn touches(record: &ChangeRecord, label: &str) -> bool {
    match record {
        ChangeRecord::FxAdded { track, .. }
        | ChangeRecord::FxRemoved { track, .. }
        | ChangeRecord::FxReordered { track }
        | ChangeRecord::FxSettingsChanged { track, .. } => track == label,
        ChangeRecord::PluginAdded { track, .. } | ChangeRecord::PluginRemoved { track, .. } => {
            track.as_deref() == Some(label)
        }
        _ => false,
    }
}

#[test]
#[ignore = "opt-in: set WIT_FIXTURES and pass --ignored to run against real material"]
fn locators_scale_time_signature_parse_and_the_master_bus_toggle_is_seen() {
    let Some(dir) = fixtures_dir() else {
        eprintln!(
            "WIT_FIXTURES not set — skipped. To run: \
             WIT_FIXTURES=/path/to/Backup cargo test -p wit-diff --test real_fixtures_plan_v2 -- --ignored --nocapture"
        );
        return;
    };

    let entries: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("cannot read WIT_FIXTURES={dir:?}: {e}"))
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("als"))
        .collect();

    // Grouped by chain name — discarded immediately after grouping; every
    // message from here on identifies a file only by chain-relative
    // position and Ableton's own save timestamp.
    let mut chains: HashMap<String, Vec<(String, PathBuf)>> = HashMap::new();
    for path in &entries {
        let Some(filename) = path.file_name().and_then(|f| f.to_str()) else {
            continue;
        };
        let (name, timestamp) = match split_backup_name(filename) {
            Some((n, t)) => (n.to_string(), t.to_string()),
            None => (filename.to_string(), String::new()),
        };
        chains
            .entry(name)
            .or_default()
            .push((timestamp, path.clone()));
    }

    let mut total_files = 0usize;
    let mut parse_errors = 0usize;
    let mut with_time_signature = 0usize;
    let mut with_key = 0usize;
    let mut with_locators = 0usize;
    let mut master_bus_changes: Vec<String> = Vec::new();

    let mut chain_names: Vec<String> = chains.keys().cloned().collect();
    chain_names.sort();

    for chain_name in &chain_names {
        let mut versions = chains[chain_name].clone();
        versions.sort_by(|a, b| a.0.cmp(&b.0));

        let models: Vec<Option<(usize, String, wit_model::Model)>> = versions
            .iter()
            .enumerate()
            .map(|(i, (timestamp, path))| {
                let bytes = std::fs::read(path).unwrap();
                total_files += 1;
                match wit_als::parse(&bytes) {
                    Ok(model) => {
                        if model.time_signature.is_some() {
                            with_time_signature += 1;
                        }
                        if model.key.is_some() {
                            with_key += 1;
                        }
                        if !model.locators.is_empty() {
                            with_locators += 1;
                        }
                        Some((i, timestamp.clone(), model))
                    }
                    Err(e) => {
                        parse_errors += 1;
                        eprintln!("  SKIP (parse error at position {i}): {e}");
                        None
                    }
                }
            })
            .collect();

        for pair in models.windows(2) {
            let (Some((i_a, ts_a, a)), Some((i_b, ts_b, b))) = (&pair[0], &pair[1]) else {
                continue;
            };
            let Some(label) = b
                .master_track_label
                .as_deref()
                .or(a.master_track_label.as_deref())
            else {
                continue;
            };
            let records = wit_diff::diff(a, b);
            if records.iter().any(|r| touches(r, label)) {
                master_bus_changes.push(format!("position {i_a} [{ts_a}] -> {i_b} [{ts_b}]"));
            }
        }
    }

    eprintln!(
        "\n{total_files} file(s) walked across {} chain(s) ({parse_errors} parse error(s)); \
         {with_time_signature} with a time signature, {with_key} with a key, {with_locators} \
         with at least one locator; {} master-bus change(s): {master_bus_changes:?}",
        chain_names.len(),
        master_bus_changes.len(),
    );

    assert!(total_files > 0, "no .als files parsed under {dir:?}");
    assert_eq!(
        parse_errors, 0,
        "every file in the real chain must parse cleanly (untrusted-input handling should mean \
         a parse failure never happens on a file Live itself wrote)"
    );
    assert!(
        with_time_signature > 0,
        "expected at least one file with an explicit clip-level time signature (measured: every \
         file in the real 29-file chain) — found none under {dir:?}; the corpus may have changed"
    );
    assert!(
        with_locators > 0,
        "expected at least one file with an arrangement locator (measured: 3 of 29 in the real \
         chain) — found none under {dir:?}; the corpus may have changed"
    );
    assert!(
        !master_bus_changes.is_empty(),
        "expected the real chain's known master-bus plugin toggle to now report a change — this \
         is the exact bug PLAN-V2 fixed ('a real save toggled a mastering plugin off on the \
         master bus, and Wit said nothing') — found none under {dir:?}; a different WIT_FIXTURES \
         chain with no master-bus plugin at all would legitimately show none here"
    );
}
