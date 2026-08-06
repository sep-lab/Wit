//! End-to-end CLI tests: invoke the real `wit` binary against real files on
//! disk. `std::process::Command` + `env!("CARGO_BIN_EXE_wit")` is the
//! standard cargo-integration-test pattern for this — no extra dependency
//! needed for a smoke-level check.

use flate2::write::GzEncoder;
use flate2::Compression;
use std::io::Write;
use std::process::Command;

fn write_als(dir: &std::path::Path, name: &str, tempo: f64) -> std::path::PathBuf {
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
        <Ableton Creator="Ableton Live 12.0.5"><LiveSet><Tracks/>
            <MasterTrack><DeviceChain><Mixer><Tempo><Manual Value="{tempo}"/></Tempo></Mixer></DeviceChain></MasterTrack>
        </LiveSet></Ableton>"#
    );
    let mut enc = GzEncoder::new(Vec::new(), Compression::default());
    enc.write_all(xml.as_bytes()).unwrap();
    let bytes = enc.finish().unwrap();
    let path = dir.join(name);
    std::fs::write(&path, bytes).unwrap();
    path
}

#[test]
fn diff_als_reports_a_tempo_change() {
    let dir = tempfile_dir();
    let a = write_als(&dir, "a.als", 120.0);
    let b = write_als(&dir, "b.als", 124.0);

    let output = Command::new(env!("CARGO_BIN_EXE_wit"))
        .args(["diff-als", a.to_str().unwrap(), b.to_str().unwrap()])
        .output()
        .unwrap();

    assert!(output.status.success(), "{:?}", output);
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(
        stdout,
        "  1 semantic change(s)\n    TEMPO   120.0 -> 124.0 BPM\n"
    );
}

#[test]
fn diff_als_reports_no_musical_change_verbatim() {
    // tests/test_als_golden.py::test_no_change_message_is_exact — the exact
    // string docs/EXPERIMENTS.md and README quote.
    let dir = tempfile_dir();
    let a = write_als(&dir, "a.als", 120.0);
    let b = write_als(&dir, "b.als", 120.0);

    let output = Command::new(env!("CARGO_BIN_EXE_wit"))
        .args(["diff-als", a.to_str().unwrap(), b.to_str().unwrap()])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(
        stdout,
        "  no musical change detected (view / bookkeeping only)\n"
    );
}

#[test]
fn diff_als_reports_a_read_failure_without_panicking() {
    let dir = tempfile_dir();
    let missing = dir.join("does-not-exist.als");
    let a = write_als(&dir, "a.als", 120.0);

    let output = Command::new(env!("CARGO_BIN_EXE_wit"))
        .args(["diff-als", missing.to_str().unwrap(), a.to_str().unwrap()])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("failed to read"), "{stderr}");
}

fn tempfile_dir() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "wit-cli-test-{}-{}",
        std::process::id(),
        // A monotonically increasing counter would be nicer than the
        // pointer trick below, but this crate has no other dependency to
        // provide one and the tests never run concurrently against the
        // same directory (each computes its own unique path per test
        // thread via std::thread::current().id()'s Debug output).
        format!("{:?}", std::thread::current().id()).replace(['(', ')'], "")
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
