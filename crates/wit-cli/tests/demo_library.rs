//! End-to-end tests for `wit demo-library` — the M5 (issue #18) recipe that
//! makes the app demoable on a machine with no real Logic library.
//!
//! `wit-demo`'s own tests cover what the generated files contain; these
//! cover the thing only the binary can prove — that the command wires up,
//! reports honestly, and refuses the one input that could destroy something.

use std::process::Command;

fn wit(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_wit"))
        .args(args)
        .output()
        .unwrap()
}

fn tempfile_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "wit-demo-test-{}-{}-{}",
        std::process::id(),
        tag,
        format!("{:?}", std::thread::current().id()).replace(['(', ')'], "")
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn demo_library_writes_a_library_the_other_commands_can_read() {
    let dir = tempfile_dir("build");
    let dest = dir.join("library");

    let out = wit(&["demo-library", dest.to_str().unwrap()]);
    assert!(out.status.success(), "demo-library should succeed");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("2 Logic project(s)") && stdout.contains("21 version(s)"),
        "unexpected report: {stdout}"
    );
    // The honesty line is not decoration — a demo fixture must never be
    // mistaken for something Logic could open.
    assert!(stdout.contains("Logic and Live cannot open them"));

    // The point of the whole command: `wit scan` finds what it wrote.
    let scan = wit(&[
        "scan",
        dest.to_str().unwrap(),
        "--data-dir",
        dir.join("index").to_str().unwrap(),
    ]);
    assert!(scan.status.success());
    let scanned = String::from_utf8_lossy(&scan.stdout);
    assert!(
        scanned.contains("3 Logic/GarageBand project(s), 1 Ableton lineage(s)"),
        "scan did not discover the demo library: {scanned}"
    );
    assert!(scanned.contains("Coastline (logic): 10 version(s)"));

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn demo_library_refuses_a_non_empty_destination_and_changes_nothing() {
    let dir = tempfile_dir("refuse");
    let precious = dir.join("MyRealSong.logicx");
    std::fs::write(&precious, b"not a demo").unwrap();

    let out = wit(&["demo-library", dir.to_str().unwrap()]);
    assert!(!out.status.success(), "must refuse a non-empty destination");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("refusing to write a demo library over it"),
        "the refusal must say why: {stderr}"
    );
    assert_eq!(std::fs::read(&precious).unwrap(), b"not a demo");
    assert!(!dir.join("Logic").exists());

    std::fs::remove_dir_all(&dir).unwrap();
}
