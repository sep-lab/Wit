//! `wit story` end to end on the demo library: the text a musician reads,
//! and the JSON the app reads, from the real binary.

use std::process::Command;

fn wit() -> Command {
    Command::new(env!("CARGO_BIN_EXE_wit"))
}

fn demo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    wit_demo::build_demo_library(&dir.path().join("demo")).unwrap();
    dir
}

#[test]
fn story_prints_sessions_and_plain_sentences() {
    let dir = demo();
    let out = wit()
        .args(["story", "--utc-offset-minutes", "0"])
        .arg(dir.path().join("demo"))
        .output()
        .unwrap();
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("Coastline"), "{text}");
    assert!(text.contains("Tempo 120 → 124 BPM"), "{text}");
    assert!(text.contains("No change Wit can see"), "{text}");
    assert!(text.contains("Wit can't see knob and fader moves in Logic yet."));
    assert!(!text.contains("Some("), "{text}");
    let root = dir.path().to_string_lossy();
    assert!(!text.contains(root.as_ref()), "the folder path leaked");
}

#[test]
fn story_json_is_the_contract() {
    let dir = demo();
    let out = wit()
        .args(["story", "--json", "--utc-offset-minutes", "0"])
        .arg(dir.path().join("demo"))
        .output()
        .unwrap();
    assert!(out.status.success());
    let lib: wit_story::Library = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(lib.schema_version, wit_story::SCHEMA_VERSION);
    assert_eq!(lib.shelf.len(), 4);
    assert_eq!(lib.stories.len(), 5);
}

#[test]
fn story_refuses_a_file() {
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("x.txt");
    std::fs::write(&f, "x").unwrap();
    let out = wit().arg("story").arg(&f).output().unwrap();
    assert!(!out.status.success());
}
