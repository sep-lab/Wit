//! The Story contract, pinned.
//!
//! - `schema/library.schema.json` must equal the schema generated from the
//!   types: a contract change is always a reviewable file change.
//! - `fixtures/demo-library.json` must equal the Library built from
//!   `just demo-library` — the UI is developed against this file, so it has
//!   to be what the real builder produces, not a hand-written mock.
//! - Every sentence obeys the invariants the UI relies on, and no Wit-authored
//!   text uses a banned word or carries a home-directory path.
//!
//! To regenerate both files after an intended change:
//! `WIT_UPDATE_SNAPSHOTS=1 cargo test -p wit-story --test contract`

use std::path::PathBuf;
use wit_story::{build_library, library_schema_json, to_json, Clock, Library, Timestamp, Verdict};

/// "Now" for the fixture: Sat 2026-09-26 12:00 UTC, two days after
/// Coastline's last session, so labels read as weekdays.
const FIXTURE_NOW: i64 = 1_790_424_000;

fn crate_file(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel)
}

fn demo_library() -> (tempfile::TempDir, Library) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("demo");
    wit_demo::build_demo_library(&root).unwrap();
    let clock = Clock {
        now: Timestamp(FIXTURE_NOW),
        utc_offset_minutes: 0,
    };
    let lib = build_library(&root, "demo library", clock);
    (dir, lib)
}

fn check_snapshot(rel: &str, actual: &str) {
    let path = crate_file(rel);
    if std::env::var_os("WIT_UPDATE_SNAPSHOTS").is_some() {
        std::fs::write(&path, actual).unwrap();
        return;
    }
    let expected = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        expected == actual,
        "{rel} is out of date with the code. If the change is intended, run \
         `WIT_UPDATE_SNAPSHOTS=1 cargo test -p wit-story --test contract` and \
         review the file in your PR."
    );
}

#[test]
fn schema_file_matches_the_types() {
    check_snapshot("schema/library.schema.json", &library_schema_json());
}

#[test]
fn fixture_is_the_real_builder_run_on_the_demo_library() {
    let (_dir, lib) = demo_library();
    check_snapshot("fixtures/demo-library.json", &to_json(&lib));
}

#[test]
fn fixture_round_trips_through_json() {
    let (_dir, lib) = demo_library();
    let json = to_json(&lib);
    let back: Library = serde_json::from_str(&json).unwrap();
    assert_eq!(back, lib);
}

#[test]
fn building_twice_is_byte_identical() {
    let (_a, a) = demo_library();
    let (_b, b) = demo_library();
    assert_eq!(to_json(&a), to_json(&b));
}

#[test]
fn no_banned_words_in_wits_own_text() {
    let (_dir, lib) = demo_library();
    let violations = wit_story::vocab::library_violations(&lib);
    assert!(violations.is_empty(), "{violations:#?}");
}

#[test]
fn no_paths_leak_into_the_story() {
    let (dir, lib) = demo_library();
    let json = to_json(&lib);
    wit_index::assert_no_home_paths(&json).unwrap();
    let tmp = dir.path().to_string_lossy();
    assert!(!json.contains(tmp.as_ref()), "the library root leaked");
    assert!(!json.contains("ProjectData"), "a file path leaked");
}

#[test]
fn sentence_and_heat_invariants_hold() {
    let (_dir, lib) = demo_library();
    for story in &lib.stories {
        let mut last = i64::MIN;
        for session in &story.sessions {
            for m in &session.moments {
                assert!(m.at.0 >= last, "moments out of order in {}", story.id);
                last = m.at.0;
                assert_eq!(m.weight as usize, m.sentences.len());
                assert_eq!(m.verdict == Verdict::Changed, !m.sentences.is_empty());
                assert_eq!(m.note.is_some(), m.sentences.is_empty());
                for h in &m.track_heat {
                    assert!((h.track as usize) < story.tracks.len());
                    assert!((1..=3).contains(&h.level));
                }
                for s in &m.sentences {
                    let joined: String = s.spans.iter().map(|x| x.text.as_str()).collect();
                    assert_eq!(joined, s.text);
                }
            }
        }
    }
}

/// The demo's Coastline Logic chain reproduces the measured 33% rate of
/// saves with nothing Wit can see (EXPERIMENTS.md §11). The Story must show
/// those as routine saves, not invent changes for them.
#[test]
fn coastline_shows_three_saves_with_nothing_visible() {
    let (_dir, lib) = demo_library();
    let story = lib
        .stories
        .iter()
        .find(|s| s.id.0 == "logic:Logic/Coastline.logicx#000")
        .unwrap();
    let verdicts: Vec<Verdict> = story
        .sessions
        .iter()
        .flat_map(|s| s.moments.iter().map(|m| m.verdict))
        .collect();
    assert_eq!(verdicts.len(), 10);
    assert_eq!(
        verdicts
            .iter()
            .filter(|v| **v == Verdict::NothingVisible)
            .count(),
        3
    );
    assert_eq!(story.sessions.len(), 3, "three evenings");
    assert!(story.overview.is_some(), "oldest-to-newest compare exists");
}
