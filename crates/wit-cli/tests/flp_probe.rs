//! End-to-end tests for `wit flp-probe`: builds small `.flp` files in the
//! byte shapes measured on real FL Studio projects (see `wit-flp`'s
//! `extract.rs`), runs the real `wit` binary on them, and pins the exact
//! lines it prints. Covers the review findings the output depended on: a
//! new channel's generator is not a second "plugin added" line; a nameless
//! last channel never borrows a mixer effect's name; effects are read on
//! every FL version; and a name that is cleared or set is reported as a
//! name change on its channel, pattern or mixer insert — never as that
//! object being added or removed. Every run's own wording is also checked
//! against the Story contract's banned-vocabulary lint.

use std::path::{Path, PathBuf};
use std::process::Command;

fn wit(args: &[&Path]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_wit"))
        .arg("flp-probe")
        .args(args)
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    String::from_utf8(out.stdout).unwrap()
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("wit-flp-probe-test-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A tiny FL event-stream writer, literal ids throughout.
#[derive(Default)]
struct Flp(Vec<u8>);

impl Flp {
    fn text(self, id: u8, s: &str) -> Self {
        let mut payload = s.as_bytes().to_vec();
        payload.push(0);
        self.var(id, &payload)
    }
    fn utf16(self, id: u8, s: &str) -> Self {
        let mut payload: Vec<u8> = s.encode_utf16().flat_map(u16::to_le_bytes).collect();
        payload.extend_from_slice(&[0, 0]);
        self.var(id, &payload)
    }
    fn var(mut self, id: u8, payload: &[u8]) -> Self {
        assert!(payload.len() < 128, "one-byte varint only");
        self.0.push(id);
        self.0.push(payload.len() as u8);
        self.0.extend_from_slice(payload);
        self
    }
    fn byte(mut self, id: u8) -> Self {
        self.0.extend_from_slice(&[id, 0]);
        self
    }
    fn word(self, id: u8) -> Self {
        self.word_value(id, 0)
    }
    fn word_value(mut self, id: u8, value: u16) -> Self {
        self.0.push(id);
        self.0.extend_from_slice(&value.to_le_bytes());
        self
    }
    fn dword(mut self, id: u8) -> Self {
        self.0.extend_from_slice(&[id, 0, 0, 0, 0]);
        self
    }
    /// A channel in the measured FL >= 11.5 header shape:
    /// `64, 21, 201 generator, 212, 203 name`, then ordinary channel events.
    fn channel(self, name: &str, generator: &str) -> Self {
        self.word(64)
            .byte(21)
            .text(201, generator)
            .var(212, &[0; 4])
            .text(203, name)
            .dword(155)
            .dword(128)
    }
    fn write(self, path: &Path, channels: u16) {
        let mut out = b"FLhd".to_vec();
        out.extend_from_slice(&6u32.to_le_bytes());
        out.extend_from_slice(&0i16.to_le_bytes());
        out.extend_from_slice(&channels.to_le_bytes());
        out.extend_from_slice(&96u16.to_le_bytes());
        out.extend_from_slice(b"FLdt");
        out.extend_from_slice(&(self.0.len() as u32).to_le_bytes());
        out.extend_from_slice(&self.0);
        std::fs::write(path, out).unwrap();
    }
}

/// Wit's own words in a run: every line except the file-path lines (a
/// path is the machine's, not Wit's wording).
fn assert_no_banned_words(stdout: &str, paths: &[&Path]) {
    for line in stdout.lines() {
        if paths.iter().any(|p| line.trim() == p.display().to_string()) {
            // A path is the machine's, not Wit's wording.
            continue;
        }
        let banned = wit_story::vocab::banned_words(line);
        assert!(banned.is_empty(), "{banned:?} in {line:?}");
    }
}

fn fl25_project(extra: impl FnOnce(Flp) -> Flp) -> Flp {
    let base = Flp::default()
        .text(199, "25.2.5.5055")
        .dword(156) // Tempo: scrambled on FL 25, so never shown as a number
        .channel("808 Kick", "")
        .channel("FLEX Bass", "FLEX");
    extra(base)
        .word(99)
        .text(241, "Arrangement")
        .var(236, &[0; 4])
        .text(201, "Emphasizer")
}

#[test]
fn a_new_generator_channel_prints_one_line_not_a_channel_and_a_plugin() {
    let dir = scratch("generator");
    let (old, new) = (dir.join("old.flp"), dir.join("new.flp"));
    fl25_project(|p| p).write(&old, 2);
    fl25_project(|p| {
        p.channel("Drumpad", "Drumpad")
            .channel("MIDI Out", "MIDI Out")
    })
    .write(&new, 4);

    let stdout = wit(&[&old, &new]);
    let tail: Vec<&str> = stdout
        .lines()
        .skip_while(|l| !l.contains("thing(s) changed"))
        .collect();
    assert_eq!(
        tail,
        vec![
            "  2 thing(s) changed:",
            "    channel added: 'Drumpad' (generator plugin 'Drumpad')",
            "    channel added: 'MIDI Out' (generator plugin 'MIDI Out')",
        ],
        "{stdout}"
    );
    assert!(stdout.contains("    generator plugins: FLEX, Drumpad, MIDI Out\n"));
    assert!(stdout.contains("    effect plugins: Emphasizer (Master)\n"));
    assert!(stdout.contains("tempo: can't read yet"));
    assert!(stdout.contains("note: this FL Studio version scrambles"));
    assert_no_banned_words(&stdout, &[&old, &new]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_nameless_last_channel_never_borrows_a_mixer_effects_name() {
    // FL 20.7's `Surround mix.flp` template shape: one Sampler channel
    // with no name saved, then the arrangement, then the Master insert's
    // "Control Surface" plugin renamed "Mix".
    let dir = scratch("nameless");
    let path = dir.join("surround.flp");
    Flp::default()
        .text(199, "20.7.0.1702")
        .word(64)
        .byte(21)
        .text(201, "")
        .var(212, &[0; 4])
        .dword(155)
        .dword(128)
        .word(99)
        .text(241, "Arrangement")
        .var(233, &[0; 8])
        .text(204, " ")
        .var(236, &[0; 4])
        .text(201, "Control Surface")
        .var(212, &[0; 4])
        .text(203, "Mix")
        .write(&path, 1);

    let stdout = wit(&[&path]);
    assert!(!stdout.contains("channel names:"), "{stdout}");
    assert!(
        stdout.contains("    channels with no name saved: 1\n"),
        "{stdout}"
    );
    assert!(
        stdout.contains("    effect plugins: Control Surface (Master)\n"),
        "{stdout}"
    );
    assert_no_banned_words(&stdout, &[&path]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn removing_an_effect_reads_the_same_on_fl_10_and_fl_20() {
    // Before this fix an FL 10 project said "plugin removed" and an FL 20
    // one fell back to "something changed that Wit can't read yet".
    let dir = scratch("effects");
    for (version, tag) in [("10.0.0", "fl10"), ("20.8.0.1377", "fl20")] {
        let project = |with_maximus: bool| {
            let mut p = Flp::default()
                .text(199, version)
                .word(64)
                .byte(21)
                .text(201, "")
                .var(212, &[0; 4])
                .text(203, "Kick") // the FL >= 11.5 name; ignored on FL 10
                .text(192, "Kick") // the pre-11.5 name; ignored on FL 20
                .var(233, &[0; 8])
                .var(236, &[0; 4])
                .text(201, "Fruity Limiter");
            if with_maximus {
                p = p.text(201, "Maximus");
            }
            p
        };
        let (old, new) = (
            dir.join(format!("{tag}-old.flp")),
            dir.join(format!("{tag}-new.flp")),
        );
        project(true).write(&old, 1);
        project(false).write(&new, 1);
        let stdout = wit(&[&old, &new]);
        let tail: Vec<&str> = stdout
            .lines()
            .skip_while(|l| !l.contains("thing(s) changed"))
            .collect();
        assert_eq!(
            tail,
            vec![
                "  1 thing(s) changed:",
                "    effect plugin removed: 'Maximus' (Master)",
            ],
            "{version}: {stdout}"
        );
        assert!(
            stdout.contains("    channel names: Kick\n"),
            "{version}: {stdout}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_fl_version_text_is_sanitized_like_names() {
    let dir = scratch("sanitize");
    let path = dir.join("hostile.flp");
    Flp::default()
        .utf16(199, "20.8\u{1b}[2J\u{202E}.0")
        .word(64)
        .byte(21)
        .text(201, "")
        .var(212, &[0; 4])
        .utf16(203, "Kick\u{202E}")
        .write(&path, 1);
    let stdout = wit(&[&path]);
    assert!(
        stdout.contains("FL Studio version: 20.8[2J.0  "),
        "{stdout}"
    );
    assert!(stdout.contains("    channel names: Kick\n"), "{stdout}");
    assert!(!stdout.contains('\u{1b}') && !stdout.contains('\u{202E}'));
    let _ = std::fs::remove_dir_all(&dir);
}

/// The review's four "a name is not an object" mutations, applied to one
/// small FL 20.8 project: two channels, two patterns (only the first
/// named), an arrangement, and five mixer positions (the Master with an
/// effect, insert 1 named "Kick").
#[derive(Clone, Copy, Default)]
struct Mutation {
    drop_first_pattern_name: bool,
    drop_first_insert_name: bool,
    name_insert_3: bool,
    drop_last_channel_name: bool,
}

fn fl20_project(m: Mutation) -> Flp {
    let mut p = Flp::default()
        .text(199, "20.8.0.1377")
        .dword(156)
        .channel("Kick", "")
        .word(64)
        .byte(21)
        .text(201, "")
        .var(212, &[0; 4]);
    if !m.drop_last_channel_name {
        p = p.text(203, "Balance - Vocal GR - Volume");
    }
    p = p.dword(155).word_value(65, 1);
    if !m.drop_first_pattern_name {
        p = p.text(193, "Clap Mute");
    }
    p = p
        .word_value(65, 2)
        .word(99)
        .text(241, "Arrangement")
        .var(236, &[0; 4])
        .text(201, "Maximus");
    if !m.drop_first_insert_name {
        p = p.text(204, "Kick");
    }
    p = p.var(236, &[0; 4]).var(236, &[0; 4]);
    if m.name_insert_3 {
        p = p.text(204, "Vocals");
    }
    p.var(236, &[0; 4]).var(236, &[0; 4])
}

fn changes_between(tag: &str, old: Mutation, new: Mutation) -> Vec<String> {
    let dir = scratch(tag);
    let (a, b) = (dir.join("old.flp"), dir.join("new.flp"));
    fl20_project(old).write(&a, 2);
    fl20_project(new).write(&b, 2);
    let stdout = wit(&[&a, &b]);
    assert_no_banned_words(&stdout, &[&a, &b]);
    let _ = std::fs::remove_dir_all(&dir);
    stdout
        .lines()
        .skip_while(|l| !l.contains("thing(s) changed"))
        .map(str::to_string)
        .collect()
}

#[test]
fn a_cleared_pattern_name_is_not_a_removed_pattern() {
    let cleared = Mutation {
        drop_first_pattern_name: true,
        ..Mutation::default()
    };
    assert_eq!(
        changes_between("pattern", Mutation::default(), cleared),
        vec![
            "  1 thing(s) changed:",
            "    pattern 1 name cleared (was 'Clap Mute')",
        ]
    );
}

#[test]
fn a_cleared_insert_name_is_not_a_removed_insert() {
    let cleared = Mutation {
        drop_first_insert_name: true,
        ..Mutation::default()
    };
    assert_eq!(
        changes_between("insert-cleared", Mutation::default(), cleared),
        vec![
            "  1 thing(s) changed:",
            "    mixer insert 1 name cleared (was 'Kick')",
        ]
    );
}

#[test]
fn a_new_insert_name_is_not_an_added_insert() {
    let named = Mutation {
        name_insert_3: true,
        ..Mutation::default()
    };
    assert_eq!(
        changes_between("insert-named", Mutation::default(), named),
        vec![
            "  1 thing(s) changed:",
            "    mixer insert 3 named: 'Vocals'",
        ]
    );
}

#[test]
fn a_cleared_channel_name_is_not_a_removed_channel() {
    let cleared = Mutation {
        drop_last_channel_name: true,
        ..Mutation::default()
    };
    assert_eq!(
        changes_between("channel-cleared", Mutation::default(), cleared),
        vec![
            "  1 thing(s) changed:",
            "    channel name cleared (was 'Balance - Vocal GR - Volume')",
        ]
    );
}

#[test]
fn the_printed_path_is_sanitized_too() {
    let dir = scratch("path");
    let path = dir.join("a\u{202E}b.flp");
    fl20_project(Mutation::default()).write(&path, 2);
    let stdout = wit(&[&path]);
    assert!(!stdout.contains('\u{202E}'), "{stdout}");
    assert!(stdout.contains("ab.flp\n"), "{stdout}");
    let _ = std::fs::remove_dir_all(&dir);
}
