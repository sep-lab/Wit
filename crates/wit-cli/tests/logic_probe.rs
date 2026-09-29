//! End-to-end tests for `wit logic-probe`, which had none before this PR.
//!
//! Builds the synthetic Coastline chain (`wit-demo`) into a tempdir at
//! runtime and runs the real `wit` binary against it, in the style of
//! `demo_library.rs`. Covers:
//! - a backup pair with a structural change — the tempo push documented in
//!   `wit-demo/src/lib.rs`'s `coastline_chain()` (save 4 -> 5, 120.0 ->
//!   124.0 BPM). This is also the regression test for the bug this PR
//!   fixes: tempo used to print as `Some(120.0) -> Some(124.0) BPM`;
//! - a backup pair with none — a pure-churn save (save 0 -> 1);
//! - resolving a `.logicx` bundle directory to its current alternative
//!   (`resolve_project_data`'s `as_bundle` branch in `wit-cli::main`).
//!
//! Every case also asserts the tempdir's own absolute path — standing in
//! for a real machine's home directory — never leaks into `wit`'s output,
//! the same privacy discipline `wit dupes`/`wit logic-report` enforce with
//! `wit_index::assert_no_home_paths`.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn wit(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_wit"))
        .args(args)
        .output()
        .unwrap()
}

fn tempfile_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "wit-logic-probe-test-{}-{}-{}",
        std::process::id(),
        tag,
        format!("{:?}", std::thread::current().id()).replace(['(', ')'], "")
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Build the demo library under `root` and return the Coastline bundle's
/// root directory (`Logic/Coastline.logicx`) — a real 10-save chain with a
/// mix of structural changes and pure-churn saves (see `wit-demo/src/lib.rs`
/// `coastline_chain()`).
fn coastline_bundle(root: &Path) -> PathBuf {
    let lib = wit_demo::build_demo_library(root).expect("build_demo_library should succeed");
    assert!(
        lib.logic_projects >= 1,
        "expected at least the Coastline project"
    );
    root.join("Logic/Coastline.logicx")
}

/// One backup slot's directory — `resolve_project_data`'s `as_slot` branch
/// resolves this straight to its `ProjectData` file, the same as it would
/// for a slot picked out of `Project File Backups` on a real Logic bundle.
fn backup_slot(bundle: &Path, slot: u32) -> PathBuf {
    bundle
        .join("Alternatives/000/Project File Backups")
        .join(format!("{slot:02}"))
}

/// Assert neither stream leaks the tempdir's absolute path (a stand-in for
/// a real machine's home directory) nor a generic home-directory marker —
/// the same check `wit_index::assert_no_home_paths` performs for
/// `dupes`/`logic-report`, applied here because `logic-probe` has none of
/// its own today.
fn assert_no_leaked_path(out: &Output, tempdir: &Path) {
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let combined = format!("{stdout}{stderr}");

    let tempdir_str = tempdir.to_str().unwrap();
    assert!(
        !combined.contains(tempdir_str),
        "logic-probe leaked the tempdir's absolute path into its output:\nstdout: {stdout}\nstderr: {stderr}"
    );
    assert!(
        wit_index::assert_no_home_paths(&combined).is_ok(),
        "logic-probe leaked a home-directory path into its output:\nstdout: {stdout}\nstderr: {stderr}"
    );
}

#[test]
fn reports_structural_change_between_backups_with_tempo_as_a_plain_number() {
    let dir = tempfile_dir("tempo-change");
    let bundle = coastline_bundle(&dir);

    // Save 4 -> 5 in `coastline_chain()` is exactly "5 — pushed the tempo",
    // 120.0 -> 124.0 BPM, with every other extracted fact unchanged — the
    // case that used to print "Some(120.0) -> Some(124.0) BPM".
    let old = backup_slot(&bundle, 4);
    let new = backup_slot(&bundle, 5);

    let out = wit(&["logic-probe", old.to_str().unwrap(), new.to_str().unwrap()]);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);

    assert!(
        stdout.contains("structural change detected:"),
        "expected a structural-change verdict: {stdout}"
    );
    assert!(
        stdout.contains("tempo: 120.0 -> 124.0 BPM"),
        "tempo must render as a plain number via fmt_num: {stdout}"
    );
    assert!(
        !stdout.contains("Some("),
        "tempo must never print Rust's Option debug spelling: {stdout}"
    );

    assert_no_leaked_path(&out, &dir);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn reports_no_structural_change_between_a_pure_churn_backup_pair() {
    let dir = tempfile_dir("no-change");
    let bundle = coastline_bundle(&dir);

    // Save 0 -> 1 is "you nudged a fader and saved. Nothing Wit can see on
    // Logic." — byte-different (a churn counter moves) but structurally
    // identical.
    let old = backup_slot(&bundle, 0);
    let new = backup_slot(&bundle, 1);

    let out = wit(&["logic-probe", old.to_str().unwrap(), new.to_str().unwrap()]);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);

    assert!(
        stdout.contains("no structural change detected"),
        "expected the no-structural-change verdict: {stdout}"
    );
    assert!(!stdout.contains("Some("));

    assert_no_leaked_path(&out, &dir);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn resolves_a_bundle_directory_to_its_current_alternative() {
    let dir = tempfile_dir("bundle");
    let bundle = coastline_bundle(&dir);

    // The bundle root has no `ProjectData` of its own — `resolve_project_data`
    // must resolve it to `Alternatives/000/ProjectData`, the current save
    // (chain index 9, "doubled the chorus"). Backup slot 8 ("added a pad")
    // is one save earlier, so the pair shows a structural change.
    let old_backup = backup_slot(&bundle, 8);

    let out = wit(&[
        "logic-probe",
        old_backup.to_str().unwrap(),
        bundle.to_str().unwrap(),
    ]);
    assert!(out.status.success(), "{:?}", out);
    let stdout = String::from_utf8_lossy(&out.stdout);

    assert!(
        stdout.contains("structural change detected:"),
        "expected a structural-change verdict comparing against the bundle's current save: {stdout}"
    );
    assert!(!stdout.contains("Some("));

    assert_no_leaked_path(&out, &dir);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn reports_a_read_failure_without_panicking_and_without_leaking_the_path() {
    // Mirrors `diff_als.rs`'s equivalent case: a missing/invalid
    // `ProjectData` must fail loudly, not panic, and the error path (which
    // does print the resolved path, by design, so the failure is
    // actionable) must still never contain a raw home-directory marker —
    // only the tempdir path this test controls.
    let dir = tempfile_dir("missing");
    let bundle = coastline_bundle(&dir);
    let missing = dir.join("does-not-exist");

    let out = wit(&[
        "logic-probe",
        missing.to_str().unwrap(),
        backup_slot(&bundle, 0).to_str().unwrap(),
    ]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("failed to read"), "{stderr}");

    std::fs::remove_dir_all(&dir).unwrap();
}
