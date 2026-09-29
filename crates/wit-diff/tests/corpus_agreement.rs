//! M1 exit criterion 3 — the "corpus agreement" gate (`PLAN.md`: "zero FX~
//! lines on the 7 measured zero-change pairs... AND FX~ detection on the
//! 3-of-9 knob-only saves"; see `docs/EXPERIMENTS.md` §1 for the
//! zero-change measurement, incl. the two saves 56 seconds apart that are
//! "identical in length" and reduce to pure `FileRef` renumbering, and §4's
//! Limit paragraph for the "3 of 9" figure).
//!
//! `real_fixtures.rs` in this crate reports aggregate zero-change/FX~
//! counts over the whole corpus but never pins those two named categories
//! to specific pairs or asserts the FX~ property either of them actually
//! rests on — this test does both.
//!
//! **Classification never uses `wit_diff`'s own output to decide which
//! pairs count as which category** — that would make the assertions below
//! trivially true by construction. Instead it uses `wit_model::Model`
//! equality directly — the same whitelist extraction `wit_als::parse`
//! already produces and the rest of the pipeline already trusts, per
//! `Device`'s own doc comment ("Whitelist, not blacklist, per AGENTS.md").
//! An earlier version of this test blacklisted known-churn *raw XML lines*
//! (`FileRef`/`AuPreset` renumbering) instead, exactly the anti-pattern
//! AGENTS.md warns against ("Blacklists leak") — it flagged 12 of 24 real
//! pairs as "knob-only" because residual churn the blacklist did not name
//! (warp-marker re-analysis, `AutomationTarget` ids — see EXPERIMENTS.md
//! §3's own correction) read as "real content changed". Comparing the
//! already-whitelisted `Model` has none of that noise by construction.
//!
//! - **zero-change**: `old_model == new_model` (every whitelisted field
//!   identical) => assert `wit_diff::diff` produces no record at all — in
//!   particular no `FxSettingsChanged` ("FX~"), the specific false positive
//!   the exit criterion names.
//! - **knob-only candidate**: the models differ, but only in a `Device`
//!   `Fingerprint` (compare with every fingerprint zeroed on both sides) —
//!   every other field (track add/remove/rename, volume, pan, mute, clips,
//!   tempo, sample) is identical, so the only way this could be a real
//!   edit is a device parameter tweak => assert at least one
//!   `FxSettingsChanged` record. The selection step never looks at
//!   `wit_diff`'s output, so it cannot manufacture the result it then
//!   checks.
//!
//! No personal names are hardcoded: pairs are grouped by the part of the
//! filename before the trailing `[<timestamp>].als` — Ableton's own
//! `Backup/` naming — and sorted by that timestamp within each group. A
//! flat, cross-song sort (as `real_fixtures.rs` does) is deliberately not
//! used here: a real `Backup/` folder can hold more than one song, or the
//! same evolving song saved under several names in sequence, and
//! consecutive-pairing across two different songs would be nonsense.
//!
//! Mirrors `real_fixtures.rs`'s and `tests/conftest.py`'s `WIT_FIXTURES`
//! discipline: **loudly skipped** by default, read-only, never touches
//! anything the operator did not point it at.
//!
//! ```text
//! WIT_FIXTURES=/path/to/Backup cargo test -p wit-diff --test corpus_agreement -- --ignored --nocapture
//! ```
//!
//! If this fails on real material, that is the finding — report exactly
//! which pair disagreed and why. Do not loosen the assertions to force a
//! pass.

use std::collections::HashMap;
use std::path::PathBuf;
use wit_model::{Fingerprint, Model};

fn fixtures_dir() -> Option<PathBuf> {
    std::env::var_os("WIT_FIXTURES").map(PathBuf::from)
}

/// Split `"<name> [<timestamp>].als"` into `(name, timestamp)`. `None` for
/// anything not shaped like an Ableton `Backup/` autosave filename.
fn split_backup_name(filename: &str) -> Option<(&str, &str)> {
    let stem = filename.strip_suffix(".als")?;
    let open = stem.rfind('[')?;
    let close = stem.rfind(']')?;
    if close != stem.len() - 1 || close <= open {
        return None;
    }
    Some((stem[..open].trim_end(), &stem[open + 1..close]))
}

/// `model` with every device's parameter fingerprint replaced by a shared
/// placeholder — comparing two of these tells you whether two models agree
/// on everything *except* device settings, without ever consulting
/// `wit_diff`.
fn with_fingerprints_zeroed(mut model: Model) -> Model {
    for track in model.tracks.values_mut() {
        for device in &mut track.devices {
            device.fingerprint = Fingerprint([0u8; 32]);
        }
    }
    model
}

#[test]
#[ignore = "opt-in: set WIT_FIXTURES and pass --ignored to run against real material"]
fn zero_change_and_knob_only_pairs_agree_with_fx_tilde() {
    let Some(dir) = fixtures_dir() else {
        eprintln!(
            "WIT_FIXTURES not set — skipped. To run: \
             WIT_FIXTURES=/path/to/Backup cargo test -p wit-diff --test corpus_agreement -- --ignored --nocapture"
        );
        return;
    };

    let entries: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("cannot read WIT_FIXTURES={dir:?}: {e}"))
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("als"))
        .collect();

    // Group by the name before the timestamp bracket. Never flatten
    // different chains into one sorted list — see the module doc.
    let mut chains: HashMap<String, Vec<(String, PathBuf)>> = HashMap::new();
    for path in entries {
        let Some(filename) = path.file_name().and_then(|f| f.to_str()) else {
            continue;
        };
        match split_backup_name(filename) {
            Some((name, timestamp)) => chains
                .entry(name.to_string())
                .or_default()
                .push((timestamp.to_string(), path.clone())),
            None => eprintln!("  SKIP (not an Ableton Backup/ autosave name): {filename}"),
        }
    }

    let mut chain_names: Vec<String> = chains.keys().cloned().collect();
    chain_names.sort();

    let mut zero_change = 0usize;
    let mut zero_change_leak = Vec::new();
    let mut knob_only_candidates = 0usize;
    let mut knob_only_missing_fx = Vec::new();
    let mut other_pairs = 0usize;
    let mut parse_errors = 0usize;
    let mut total_pairs = 0usize;

    for name in &chain_names {
        let mut versions = chains[name].clone();
        versions.sort_by(|a, b| a.0.cmp(&b.0));
        if versions.len() < 2 {
            continue;
        }
        for pair in versions.windows(2) {
            let (old_ts, old_path) = &pair[0];
            let (new_ts, new_path) = &pair[1];
            let old_bytes = std::fs::read(old_path).unwrap();
            let new_bytes = std::fs::read(new_path).unwrap();
            let label = format!("{name} [{old_ts}] -> [{new_ts}]");

            let (Ok(old_model), Ok(new_model)) =
                (wit_als::parse(&old_bytes), wit_als::parse(&new_bytes))
            else {
                eprintln!("  SKIP (parse error): {label}");
                parse_errors += 1;
                continue;
            };
            total_pairs += 1;

            let records = wit_diff::diff(&old_model, &new_model);
            let has_fx = records
                .iter()
                .any(|r| matches!(r, wit_model::ChangeRecord::FxSettingsChanged { .. }));

            if old_model == new_model {
                zero_change += 1;
                eprintln!("  ZERO-CHANGE  {label}: {} record(s)", records.len());
                if !records.is_empty() {
                    zero_change_leak.push(format!("{label} ({} record(s))", records.len()));
                }
            } else if with_fingerprints_zeroed(old_model) == with_fingerprints_zeroed(new_model) {
                knob_only_candidates += 1;
                eprintln!(
                    "  KNOB-ONLY?   {label}: {} record(s), FX~ present: {has_fx}",
                    records.len()
                );
                if !has_fx {
                    knob_only_missing_fx.push(label);
                }
            } else {
                other_pairs += 1;
                eprintln!("  CHANGED      {label}: {} record(s)", records.len());
            }
        }
    }

    eprintln!(
        "\n{total_pairs} pair(s) walked across {} chain(s) ({parse_errors} parse error(s)); \
         {zero_change} zero-change, {knob_only_candidates} knob-only candidate(s), \
         {other_pairs} other change(s).",
        chain_names.len()
    );

    assert!(
        total_pairs > 0,
        "no consecutive same-name pair parsed cleanly under {dir:?} — nothing to check"
    );
    assert!(
        zero_change > 0,
        "expected at least one zero-change pair (EXPERIMENTS.md §1 measured 7 of 29) — found \
         none under {dir:?}; the corpus may have changed since that measurement"
    );
    assert!(
        knob_only_candidates > 0,
        "expected at least one knob-only-candidate pair (EXPERIMENTS.md §4's Limit measured 3 \
         of 9) — found none under {dir:?}; the corpus may have changed since that measurement"
    );
    assert!(
        zero_change_leak.is_empty(),
        "wit_diff produced a record on a pair whose Model was byte-for-byte identical — a false \
         positive: {zero_change_leak:?}"
    );
    assert!(
        knob_only_missing_fx.is_empty(),
        "a pair's Model differs only in a device fingerprint (a knob/parameter move) but \
         wit_diff produced no FxSettingsChanged record for it: {knob_only_missing_fx:?}"
    );
}
