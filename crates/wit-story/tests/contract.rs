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
use wit_story::{
    build_library, library_schema_json, to_json, Clock, Confidence, Library, MomentSource,
    Timestamp, Verdict,
};

/// "Now" for the fixture: Sat 2026-09-26 12:00 UTC, two days after
/// Coastline's last session, so labels read as weekdays.
const FIXTURE_NOW: i64 = 1_790_424_000;

fn crate_file(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel)
}

fn fixture_clock() -> Clock {
    Clock::fixed(Timestamp(FIXTURE_NOW), 0)
}

fn demo_library() -> (tempfile::TempDir, Library) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("demo");
    wit_demo::build_demo_library(&root).unwrap();
    let lib = build_library(&root, "demo library", &fixture_clock());
    (dir, lib)
}

fn check_snapshot(rel: &str, actual: &str) {
    let path = crate_file(rel);
    if std::env::var_os("WIT_UPDATE_SNAPSHOTS").is_some() {
        std::fs::write(&path, actual).unwrap();
        return;
    }
    // A Windows checkout may convert the pinned file to CRLF; the contract
    // is the content, not the line endings.
    let expected = std::fs::read_to_string(&path)
        .unwrap_or_default()
        .replace("\r\n", "\n");
    if expected != actual {
        let first_diff = expected
            .lines()
            .zip(actual.lines())
            .position(|(e, a)| e != a)
            .unwrap_or_else(|| expected.lines().count().min(actual.lines().count()));
        panic!(
            "{rel} is out of date with the code (first difference at line {}). If the \
             change is intended, run `WIT_UPDATE_SNAPSHOTS=1 cargo test -p wit-story \
             --test contract` and review the file in your PR.",
            first_diff + 1
        );
    }
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
        .find(|s| s.header.title == "Coastline" && s.header.daw_label == "Logic")
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

/// An inferred sentence always hedges in its words, not only in a field a
/// screen reader or copy-as-text would never show.
#[test]
fn every_inferred_sentence_says_probably() {
    let (_dir, lib) = demo_library();
    let mut seen = 0;
    for story in &lib.stories {
        let sentences = story
            .sessions
            .iter()
            .flat_map(|s| s.moments.iter().flat_map(|m| m.sentences.iter()))
            .chain(story.overview.iter().flat_map(|o| o.sentences.iter()));
        for s in sentences {
            if s.confidence == Confidence::Inferred {
                seen += 1;
                assert!(s.text.starts_with("Probably"), "{}", s.text);
            }
        }
    }
    assert!(
        seen > 0,
        "the demo has an inferred rename; the check must run"
    );
}

/// Nothing in v1 is kept by Wit, so nothing may say Wit kept it.
#[test]
fn only_moments_wit_kept_say_wit_kept_them() {
    let (_dir, lib) = demo_library();
    for story in &lib.stories {
        assert!(
            !story.header.kept.label.contains("kept") || story.header.kept.kept_by_wit > 0,
            "{}",
            story.header.kept.label
        );
        for m in story.sessions.iter().flat_map(|s| s.moments.iter()) {
            if m.source != MomentSource::KeptByWit {
                let note = m.note.as_deref().unwrap_or("");
                assert!(!note.contains("Wit kept"), "{note}");
            }
        }
    }
}

/// Logic recycles its oldest backup on every save. Moment ids must not
/// shift when that happens, or every share pin and compare would point at
/// a different save after the next save.
#[test]
fn moment_ids_survive_the_oldest_save_disappearing() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("demo");
    wit_demo::build_demo_library(&root).unwrap();
    let ids = |lib: &Library| -> Vec<String> {
        lib.stories
            .iter()
            .find(|s| s.header.title == "Coastline" && s.header.daw_label == "Logic")
            .unwrap()
            .sessions
            .iter()
            .flat_map(|s| s.moments.iter().map(|m| m.id.0.clone()))
            .collect()
    };
    let before = ids(&build_library(&root, "demo", &fixture_clock()));
    let oldest = root.join("Logic/Coastline.logicx/Alternatives/000/Project File Backups/00");
    std::fs::remove_dir_all(&oldest).unwrap();
    let after = ids(&build_library(&root, "demo", &fixture_clock()));
    assert_eq!(after.len(), before.len() - 1);
    assert_eq!(after, before[1..].to_vec());
}

/// Ids are opaque: no folder or file name inside them.
#[test]
fn ids_carry_no_names() {
    let (_dir, lib) = demo_library();
    for story in &lib.stories {
        for id in [&story.id.0, &story.song_id.0] {
            assert!(!id.contains("Coastline") && !id.contains('/'), "{id}");
        }
    }
}

/// Live writes its first autosave into `Backup/` after the set was saved by
/// hand. The song's ids must not change when that happens.
#[test]
fn ableton_ids_survive_the_first_autosave() {
    let demo = tempfile::tempdir().unwrap();
    wit_demo::build_demo_library(&demo.path().join("d")).unwrap();
    let backup = demo.path().join("d/Ableton/Coastline Project/Backup");
    let mut saves: Vec<std::path::PathBuf> = std::fs::read_dir(&backup)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    saves.sort();

    let lib_dir = tempfile::tempdir().unwrap();
    let root = lib_dir.path().join("lib");
    let set = root.join("Song Project/Coastline.als");
    std::fs::create_dir_all(set.parent().unwrap()).unwrap();
    let stamp = |p: &std::path::Path, secs: u64| {
        std::fs::File::options()
            .write(true)
            .open(p)
            .unwrap()
            .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs))
            .unwrap();
    };
    std::fs::copy(&saves[4], &set).unwrap();
    stamp(&set, 1_767_645_857);
    let ids = |lib: &Library| -> Vec<String> {
        let story = &lib.stories[0];
        let mut v = vec![story.song_id.0.clone(), story.id.0.clone()];
        v.extend(
            story
                .sessions
                .iter()
                .flat_map(|s| s.moments.iter().map(|m| m.id.0.clone())),
        );
        v
    };
    let before = ids(&build_library(&root, "lib", &fixture_clock()));

    let auto = root.join("Song Project/Backup/Coastline [2026-01-05 200133].als");
    std::fs::create_dir_all(auto.parent().unwrap()).unwrap();
    std::fs::copy(&saves[3], &auto).unwrap();
    stamp(&auto, 1_767_643_293);
    let lib = build_library(&root, "lib", &fixture_clock());
    assert_eq!(lib.stories.len(), 1, "one lineage");
    let after = ids(&lib);
    assert_eq!(after[0], before[0], "song id");
    assert_eq!(after[1], before[1], "story id");
    assert!(
        after.contains(&before[2]),
        "the hand save keeps its moment id"
    );
}

/// The same folder layout under two watched roots must not collide.
#[test]
fn ids_are_unique_across_watched_folders() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    wit_demo::build_demo_library(a.path()).unwrap();
    wit_demo::build_demo_library(b.path()).unwrap();
    let la = build_library(a.path(), "~/Music", &fixture_clock());
    let lb = build_library(b.path(), "~/Documents", &fixture_clock());
    for (x, y) in la.shelf.iter().zip(lb.shelf.iter()) {
        assert_ne!(x.song_id, y.song_id);
    }
}

/// PLAN-V2 (2026-09-29, "Phase C → Ableton"): once a project's time
/// signature is actually read, an Ableton bar position is exact, not
/// "about" — and a position inside a locator's range names that section.
/// The demo's Ableton chain has a save that moves a clip from bar 1 into
/// the "Chorus" locator at bar 5 for exactly this reason.
#[test]
fn ableton_bars_are_exact_and_sectioned_once_the_meter_is_read() {
    let (_dir, lib) = demo_library();
    let story = lib
        .stories
        .iter()
        .find(|s| s.header.daw_label == "Live")
        .expect("the demo library has an Ableton story");
    assert_eq!(story.header.time_signature.as_deref(), Some("4/4"));

    let moved = story
        .sessions
        .iter()
        .flat_map(|s| s.moments.iter())
        .flat_map(|m| m.sentences.iter())
        .find(|s| s.text.starts_with("Moved clip"))
        .expect("the demo's move-into-chorus save produced a sentence");

    assert_eq!(moved.confidence, Confidence::Exact);
    match &moved.place {
        Some(wit_story::Place::Bars {
            approximate,
            section,
            ..
        }) => {
            assert!(!approximate, "bars must be exact once the meter is read");
            assert_eq!(section.as_deref(), Some("Chorus"));
        }
        other => panic!("expected Place::Bars, got {other:?}"),
    }
    assert_eq!(moved.place_label.as_deref(), Some("bars 5–8 · Chorus"));
}
