//! Property test for ADR-0007's law: **no write ever lands inside a watched
//! root.**
//!
//! Each case builds a random world in a temp dir — nested folders whose
//! names collide by case (`Music`/`music`), by Unicode normal form
//! (`Café` NFC / NFD), and by full case folding (`straße`/`STRASSE`);
//! symlinks (live and dangling) pointing anywhere in it; fake Logic
//! packages — then picks watched roots and a Restores folder, both spelled
//! adversarially (`x/..` detours, `.` components, trailing separators, case
//! and normal-form flips, symlink hops, and on Windows the verbatim `\\?\`
//! form and very long names). It then throws every write primitive at it
//! with hostile song names and hostile relative paths.
//!
//! The oracle never trusts the path logic under test: it snapshots the
//! whole world before and after (without following symlinks) and decides
//! "is X inside Y" by **file identity** (`same-file`: dev+inode on Unix,
//! volume serial+file index on Windows) along X's physical ancestors.
//!
//! Asserted for every case:
//! 1. nothing that existed before was modified or removed — anywhere;
//! 2. nothing new appeared inside any watched root;
//! 3. if a `RestoresDir` was granted, the oracle agrees it overlaps no
//!    watched root, and every new entry is inside it (or is one of the
//!    folders created to make it).

use proptest::prelude::*;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use unicode_normalization::UnicodeNormalization;
use wit_platform::clone::{
    clone_tree, remove_dir_contents_in_restores, write_file_in_restores, RestoresDir, StagedRestore,
};
use wit_platform::roots::{RootKind, WatchedRoots};

fn names() -> Vec<String> {
    vec![
        "Music".into(),
        "music".into(),
        "MUSIC".into(),
        "Caf\u{e9}".into(),
        "Cafe\u{301}".into(),
        "Logic".into(),
        "logic".into(),
        "Song.logicx".into(),
        "Wit Restores".into(),
        "a".into(),
        "b".into(),
        "stra\u{df}e".into(),
        "STRASSE".into(),
        "L".repeat(60),
    ]
}

/// Hostile song names for `fresh_destination`.
fn songs() -> Vec<String> {
    vec![
        "Song".into(),
        "../../escape".into(),
        "/abs/path".into(),
        r"C:\Windows\x".into(),
        r"\\?\C:\x".into(),
        "CON".into(),
        ".hidden".into(),
        "Cafe\u{301}".into(),
        "a/b/../../..".into(),
        String::new(),
        "..".into(),
        ".".into(),
        "Music/Logic".into(),
        "x\u{0}y\nz".into(),
    ]
}

const EXTS: [&str; 6] = ["logicx", "als", "../x", "", "a/b", "flp"];
const EXTRA_REL: [&str; 7] = [
    "Alternatives",
    "000",
    "ProjectData",
    "CON",
    "a:b",
    "trail.",
    "new",
];

/// How a single name is spelled on the way to a path.
#[derive(Debug, Clone)]
enum Spell {
    Plain,
    FlipCase,
    FlipNorm,
    /// `<other>/../<name>` — a `..` detour through another name.
    ViaDotDot(usize),
    /// `./<name>`
    Dot,
}

fn spell_strategy() -> impl Strategy<Value = Spell> {
    prop_oneof![
        3 => Just(Spell::Plain),
        1 => Just(Spell::FlipCase),
        1 => Just(Spell::FlipNorm),
        1 => (0..names().len()).prop_map(Spell::ViaDotDot),
        1 => Just(Spell::Dot),
    ]
}

fn flip_case(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_lowercase() {
                c.to_ascii_uppercase()
            } else {
                c.to_ascii_lowercase()
            }
        })
        .collect()
}

fn flip_norm(s: &str) -> String {
    let nfc: String = s.nfc().collect();
    if nfc == s {
        s.nfd().collect()
    } else {
        nfc
    }
}

fn spelled(name: &str, spell: &Spell) -> Vec<String> {
    let all = names();
    match spell {
        Spell::Plain => vec![name.to_string()],
        Spell::FlipCase => vec![flip_case(name)],
        Spell::FlipNorm => vec![flip_norm(name)],
        Spell::ViaDotDot(i) => vec![all[*i].clone(), "..".into(), name.to_string()],
        Spell::Dot => vec![".".into(), name.to_string()],
    }
}

/// One step of the Restores folder's spelling.
#[derive(Debug, Clone)]
enum Step {
    Name(usize, Spell),
    Link(usize),
    Up,
    Here,
    Fresh(u8),
}

fn step_strategy() -> impl Strategy<Value = Step> {
    prop_oneof![
        4 => (0..names().len(), spell_strategy()).prop_map(|(i, s)| Step::Name(i, s)),
        2 => (0usize..3).prop_map(Step::Link),
        1 => Just(Step::Up),
        1 => Just(Step::Here),
        2 => (0u8..3).prop_map(Step::Fresh),
    ]
}

#[derive(Debug, Clone)]
enum Start {
    World,
    Root(usize),
}

/// One component of a relative path handed to a staged restore.
#[derive(Debug, Clone)]
enum RelPart {
    Name(usize),
    Extra(usize),
    Up,
    Here,
    /// The absolute path of a file inside a watched root.
    AbsoluteWatched,
}

fn rel_strategy() -> impl Strategy<Value = Vec<RelPart>> {
    prop::collection::vec(
        prop_oneof![
            3 => (0..names().len()).prop_map(RelPart::Name),
            3 => (0..EXTRA_REL.len()).prop_map(RelPart::Extra),
            1 => Just(RelPart::Up),
            1 => Just(RelPart::Here),
            1 => Just(RelPart::AbsoluteWatched),
        ],
        0..5,
    )
}

#[derive(Debug, Clone)]
enum Op {
    Write {
        song: usize,
        ext: Option<usize>,
    },
    Clone {
        root: usize,
        song: usize,
    },
    Staged {
        from_root: Option<usize>,
        writes: Vec<Vec<RelPart>>,
        removes: Vec<Vec<RelPart>>,
        commit: bool,
    },
}

fn op_strategy() -> impl Strategy<Value = Op> {
    let song = 0..songs().len();
    prop_oneof![
        (song.clone(), prop::option::of(0..EXTS.len()))
            .prop_map(|(song, ext)| Op::Write { song, ext }),
        (0usize..3, song).prop_map(|(root, song)| Op::Clone { root, song }),
        (
            prop::option::of(0usize..3),
            prop::collection::vec(rel_strategy(), 0..4),
            prop::collection::vec(rel_strategy(), 0..3),
            any::<bool>(),
        )
            .prop_map(|(from_root, writes, removes, commit)| Op::Staged {
                from_root,
                writes,
                removes,
                commit,
            }),
    ]
}

#[derive(Debug, Clone)]
struct Scenario {
    /// Folders to create, as paths of name indices (depth 1–3).
    dirs: Vec<Vec<usize>>,
    /// `(parent dir, target dir, dangling)`: a symlink `lnk<i>` inside
    /// `parent` pointing at `target` (or at a missing path).
    links: Vec<(usize, usize, bool)>,
    /// `(dir, spelling of its last name, added as a user folder)`.
    roots: Vec<(usize, Spell, bool)>,
    start: Start,
    steps: Vec<Step>,
    trailing_separator: bool,
    /// Windows: spell the world root in verbatim `\\?\` form.
    verbatim: bool,
    ops: Vec<Op>,
}

fn scenario_strategy() -> impl Strategy<Value = Scenario> {
    let n = names().len();
    (
        prop::collection::vec(prop::collection::vec(0..n, 1..=3), 1..8),
        prop::collection::vec((0usize..8, 0usize..8, prop::bool::weighted(0.2)), 0..4),
        prop::collection::vec((0usize..8, spell_strategy(), any::<bool>()), 1..4),
        prop_oneof![Just(Start::World), (0usize..3).prop_map(Start::Root)],
        prop::collection::vec(step_strategy(), 1..=5),
        any::<bool>(),
        any::<bool>(),
        prop::collection::vec(op_strategy(), 1..5),
    )
        .prop_map(
            |(dirs, links, roots, start, steps, trailing_separator, verbatim, ops)| Scenario {
                dirs,
                links,
                roots,
                start,
                steps,
                trailing_separator,
                verbatim,
                ops,
            },
        )
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Entry {
    File(Vec<u8>),
    Dir,
    Link(PathBuf),
    Other,
}

/// Every entry under `dir`, never following symlinks.
fn snapshot(dir: &Path) -> BTreeMap<PathBuf, Entry> {
    let mut out = BTreeMap::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = fs::read_dir(&d) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            let Ok(meta) = fs::symlink_metadata(&p) else {
                continue;
            };
            let entry = if meta.file_type().is_symlink() {
                Entry::Link(fs::read_link(&p).unwrap_or_default())
            } else if meta.is_dir() {
                stack.push(p.clone());
                Entry::Dir
            } else if meta.is_file() {
                Entry::File(fs::read(&p).unwrap_or_default())
            } else {
                Entry::Other
            };
            out.insert(p, entry);
        }
    }
    out
}

/// `x` is `root` or lies beneath it — by file identity along `x`'s physical
/// ancestors. Independent of how either is spelled.
fn inside_by_identity(x: &Path, root: &Path) -> bool {
    let Ok(canonical) = fs::canonicalize(x) else {
        return false;
    };
    canonical
        .ancestors()
        .any(|a| same_file::is_same_file(a, root).unwrap_or(false))
}

fn symlink_dir(target: &Path, link: &Path) -> bool {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link).is_ok()
    }
    #[cfg(windows)]
    {
        // Needs Developer Mode or elevation; best effort.
        std::os::windows::fs::symlink_dir(target, link).is_ok()
    }
}

fn with_trailing_separator(p: &Path) -> PathBuf {
    let mut s: OsString = p.as_os_str().to_os_string();
    s.push(std::path::MAIN_SEPARATOR_STR);
    PathBuf::from(s)
}

struct Built {
    _tmp: tempfile::TempDir,
    /// The world lives six levels below the temp dir, so however many `..`
    /// a generated spelling climbs (at most five), it stays in the sandbox.
    world: PathBuf,
    dirs: Vec<PathBuf>,
    /// Watched roots as added (spelled) and as the oracle knows them.
    roots: Vec<PathBuf>,
    watched: WatchedRoots,
    restores_spelling: PathBuf,
}

fn build(s: &Scenario) -> Built {
    let all = names();
    let tmp = tempfile::tempdir().unwrap();
    let sandbox = fs::canonicalize(tmp.path()).unwrap();
    let world = sandbox.join("s1/s2/s3/s4/s5/world");
    fs::create_dir_all(&world).unwrap();

    let mut dirs = Vec::new();
    for spec in &s.dirs {
        let mut p = world.clone();
        for &i in spec {
            p.push(&all[i]);
        }
        if fs::create_dir_all(&p).is_ok() && p.is_dir() {
            let _ = fs::write(p.join("f.txt"), b"watched bytes");
            if p.extension().is_some_and(|e| e == "logicx") {
                let alt = p.join("Alternatives/000");
                let _ = fs::create_dir_all(&alt);
                let _ = fs::write(alt.join("ProjectData"), b"project bytes");
            }
            dirs.push(p);
        }
    }
    if dirs.is_empty() {
        dirs.push(world.clone());
    }
    for (i, (parent, target, dangling)) in s.links.iter().enumerate() {
        let parent = &dirs[parent % dirs.len()];
        let target = if *dangling {
            world.join("missing/deeper")
        } else {
            dirs[target % dirs.len()].clone()
        };
        symlink_dir(&target, &parent.join(format!("lnk{i}")));
    }

    let mut watched = WatchedRoots::new();
    let mut roots = Vec::new();
    for (dir, spell, user) in &s.roots {
        let real = &dirs[dir % dirs.len()];
        let Some(last) = real.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let mut spelled_path = real.parent().unwrap().to_path_buf();
        for c in spelled(last, spell) {
            spelled_path.push(c);
        }
        let kind = if *user {
            RootKind::UserFolder
        } else {
            RootKind::Discovery
        };
        if watched.add(&spelled_path, kind).is_ok() {
            roots.push(fs::canonicalize(&spelled_path).unwrap());
        }
    }

    let world_spelled = if cfg!(windows) && !s.verbatim {
        dunce::simplified(&world).to_path_buf()
    } else {
        world.clone()
    };
    let mut restores = match s.start {
        Start::Root(i) if !s.roots.is_empty() => {
            let (dir, _, _) = &s.roots[i % s.roots.len()];
            dirs[dir % dirs.len()].clone()
        }
        _ => world_spelled,
    };
    for step in &s.steps {
        match step {
            Step::Name(i, spell) => {
                for c in spelled(&all[*i], spell) {
                    restores.push(c);
                }
            }
            Step::Link(i) => restores.push(format!("lnk{i}")),
            Step::Up => restores.push(".."),
            Step::Here => restores.push("."),
            Step::Fresh(i) => restores.push(format!("new{i}")),
        }
    }
    if s.trailing_separator {
        restores = with_trailing_separator(&restores);
    }
    Built {
        _tmp: tmp,
        world,
        dirs,
        roots,
        watched,
        restores_spelling: restores,
    }
}

fn rel_path(parts: &[RelPart], b: &Built) -> PathBuf {
    let all = names();
    let mut p = PathBuf::new();
    for part in parts {
        match part {
            RelPart::Name(i) => p.push(&all[*i]),
            RelPart::Extra(i) => p.push(EXTRA_REL[*i]),
            RelPart::Up => p.push(".."),
            RelPart::Here => p.push("."),
            RelPart::AbsoluteWatched => {
                let base = b.roots.first().unwrap_or(&b.world);
                p = base.join("f.txt").join(&p);
            }
        }
    }
    p
}

fn run_ops(s: &Scenario, b: &Built, restores: &RestoresDir) {
    let songs = songs();
    for op in &s.ops {
        match op {
            Op::Write { song, ext } => {
                if let Ok(dest) =
                    restores.fresh_destination(&songs[*song], "2026-09-29", ext.map(|e| EXTS[e]))
                {
                    let _ = write_file_in_restores(dest, b"restored bytes");
                }
            }
            Op::Clone { root, song } => {
                let src = if b.roots.is_empty() {
                    b.dirs[*root % b.dirs.len()].clone()
                } else {
                    b.roots[*root % b.roots.len()].clone()
                };
                if let Ok(dest) = restores.fresh_destination(&songs[*song], "d", Some("logicx")) {
                    let _ = clone_tree(&src, dest);
                }
            }
            Op::Staged {
                from_root,
                writes,
                removes,
                commit,
            } => {
                let Ok(dest) = restores.fresh_destination("Staged", "d", None) else {
                    continue;
                };
                let Ok(mut staged) = StagedRestore::begin(dest) else {
                    continue;
                };
                if let Some(r) = from_root {
                    let src = b.roots.get(*r % b.roots.len().max(1)).unwrap_or(&b.world);
                    let _ = staged.clone_tree_from(src);
                }
                for w in writes {
                    let _ = staged.write_file(&rel_path(w, b), b"swapped bytes");
                }
                for r in removes {
                    let _ = remove_dir_contents_in_restores(&mut staged, &rel_path(r, b));
                }
                if *commit {
                    let _ = staged.commit();
                }
            }
        }
    }
}

/// What one case did — used to prove the generator isn't vacuous.
#[derive(Debug, Default, Clone, Copy)]
struct Outcome {
    granted: bool,
    new_files_in_restores: usize,
}

fn check(s: &Scenario) -> Result<Outcome, TestCaseError> {
    let b = build(s);
    let mut outcome = Outcome::default();
    let sandbox = b.world.ancestors().nth(6).unwrap().to_path_buf();
    let before = snapshot(&sandbox);

    let granted = RestoresDir::new(&b.restores_spelling, &b.watched);
    if let Ok(restores) = &granted {
        // The oracle must agree the granted folder overlaps no watched root.
        for root in &b.roots {
            prop_assert!(
                !inside_by_identity(restores.path(), root),
                "granted Restores {} is inside watched root {}",
                restores.path().display(),
                root.display()
            );
            prop_assert!(
                !inside_by_identity(root, restores.path()),
                "granted Restores {} contains watched root {}",
                restores.path().display(),
                root.display()
            );
        }
        prop_assert!(
            inside_by_identity(restores.path(), &sandbox),
            "Restores escaped the sandbox"
        );
        outcome.granted = true;
        run_ops(s, &b, restores);
    }

    let after = snapshot(&sandbox);
    // 1. Nothing pre-existing changed or vanished, anywhere.
    for (path, entry) in &before {
        prop_assert_eq!(
            after.get(path),
            Some(entry),
            "pre-existing entry {} was modified or removed",
            path.display()
        );
    }
    for path in after.keys().filter(|p| !before.contains_key(*p)) {
        // 2. Nothing new inside any watched root.
        for root in &b.roots {
            prop_assert!(
                !inside_by_identity(path, root),
                "new entry {} is inside watched root {}",
                path.display(),
                root.display()
            );
        }
        // 3. Everything new is in the granted Restores folder, or is a
        //    folder created on the way to it.
        match &granted {
            Ok(restores) => {
                let in_restores = inside_by_identity(path, restores.path());
                let on_the_way = inside_by_identity(restores.path(), path);
                prop_assert!(
                    in_restores || on_the_way,
                    "new entry {} is outside the Restores folder {}",
                    path.display(),
                    restores.path().display()
                );
                if in_restores && fs::symlink_metadata(path).is_ok_and(|m| m.is_file()) {
                    outcome.new_files_in_restores += 1;
                }
            }
            Err(_) => {
                // A refused Restores folder may leave only empty folders
                // that passed the check on the way; never a file.
                prop_assert!(
                    fs::symlink_metadata(path).is_ok_and(|m| m.is_dir()),
                    "a refused Restores folder left a file at {}",
                    path.display()
                );
            }
        }
    }
    Ok(outcome)
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 192,
        // Don't write `proptest-regressions/` into the source tree; a
        // failure prints its minimal case and seed instead.
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    #[test]
    fn no_write_ever_lands_inside_a_watched_root(s in scenario_strategy()) {
        check(&s)?;
    }
}

/// The tricky cases by name, so each is exercised on every run regardless
/// of what the generator happens to draw.
#[test]
fn named_tricky_cases_are_refused_or_safe() {
    let spell = |i: usize| Step::Name(i, Spell::Plain);
    let cases: Vec<(&str, Scenario)> = vec![
        (
            "restores via a symlink into the watched root",
            Scenario {
                dirs: vec![vec![0, 5], vec![9]],
                links: vec![(1, 0, false)],
                roots: vec![(0, Spell::Plain, false)],
                start: Start::World,
                steps: vec![spell(9), Step::Link(0), Step::Fresh(0)],
                trailing_separator: false,
                verbatim: false,
                ops: vec![Op::Write {
                    song: 0,
                    ext: Some(1),
                }],
            },
        ),
        (
            // Lexically `world/b/new1` (outside); physically
            // `Music/Logic/new1` (inside the root) — POSIX resolves the
            // link before the `..`.
            "`..` out of a symlinked folder lands inside the root",
            Scenario {
                dirs: vec![vec![0, 5, 9], vec![10], vec![0, 5]],
                links: vec![(1, 0, false)],
                roots: vec![(2, Spell::Plain, false)],
                start: Start::World,
                steps: vec![spell(10), Step::Link(0), Step::Up, Step::Fresh(1)],
                trailing_separator: true,
                verbatim: false,
                ops: vec![Op::Clone { root: 0, song: 1 }],
            },
        ),
        (
            "case-variant spelling of the root",
            Scenario {
                dirs: vec![vec![0, 5]],
                links: vec![],
                roots: vec![(0, Spell::Plain, true)],
                start: Start::World,
                steps: vec![spell(1), spell(6), Step::Fresh(0)],
                trailing_separator: false,
                verbatim: false,
                ops: vec![Op::Write { song: 2, ext: None }],
            },
        ),
        (
            "NFD spelling of an NFC root",
            Scenario {
                dirs: vec![vec![3]],
                links: vec![],
                roots: vec![(0, Spell::Plain, false)],
                start: Start::World,
                steps: vec![spell(4), Step::Fresh(2)],
                trailing_separator: true,
                verbatim: true,
                ops: vec![Op::Write {
                    song: 7,
                    ext: Some(1),
                }],
            },
        ),
        (
            "full case folding: STRASSE vs straße",
            Scenario {
                dirs: vec![vec![11]],
                links: vec![],
                roots: vec![(0, Spell::Plain, false)],
                start: Start::World,
                steps: vec![spell(12), Step::Fresh(0)],
                trailing_separator: false,
                verbatim: false,
                ops: vec![Op::Write { song: 0, ext: None }],
            },
        ),
        (
            "restores equal to the root, spelled with `x/..` and `.`",
            Scenario {
                dirs: vec![vec![0, 5], vec![0, 9]],
                links: vec![],
                roots: vec![(0, Spell::ViaDotDot(9), false)],
                start: Start::Root(0),
                steps: vec![Step::Here],
                trailing_separator: true,
                verbatim: false,
                ops: vec![Op::Write { song: 0, ext: None }],
            },
        ),
        (
            "restores above the root",
            Scenario {
                dirs: vec![vec![0, 5]],
                links: vec![],
                roots: vec![(0, Spell::Plain, false)],
                start: Start::World,
                steps: vec![spell(0)],
                trailing_separator: false,
                verbatim: false,
                ops: vec![Op::Write { song: 0, ext: None }],
            },
        ),
        (
            "restores through a dangling symlink",
            Scenario {
                dirs: vec![vec![9]],
                links: vec![(0, 0, true)],
                roots: vec![(0, Spell::Plain, false)],
                start: Start::World,
                steps: vec![spell(9), Step::Link(0), Step::Fresh(0)],
                trailing_separator: false,
                verbatim: false,
                ops: vec![Op::Write { song: 0, ext: None }],
            },
        ),
        (
            "a valid Restores folder next to the root, with hostile staged paths",
            Scenario {
                dirs: vec![vec![0, 5, 7], vec![0]],
                links: vec![],
                roots: vec![(0, Spell::Plain, false)],
                start: Start::World,
                steps: vec![spell(0), spell(8)],
                trailing_separator: false,
                verbatim: true,
                ops: vec![Op::Staged {
                    from_root: Some(0),
                    writes: vec![
                        vec![RelPart::Up, RelPart::Extra(2)],
                        vec![RelPart::AbsoluteWatched],
                        vec![RelPart::Extra(0), RelPart::Extra(1), RelPart::Extra(2)],
                        vec![RelPart::Extra(3)],
                    ],
                    removes: vec![vec![], vec![RelPart::Up], vec![RelPart::AbsoluteWatched]],
                    commit: true,
                }],
            },
        ),
        (
            "very long names",
            Scenario {
                dirs: vec![vec![13, 13, 13]],
                links: vec![],
                roots: vec![(0, Spell::Plain, false)],
                start: Start::World,
                steps: vec![spell(13), spell(13), Step::Fresh(0), spell(13)],
                trailing_separator: false,
                verbatim: true,
                ops: vec![Op::Clone { root: 0, song: 0 }],
            },
        ),
    ];
    for (name, scenario) in cases {
        if let Err(e) = check(&scenario) {
            panic!("{name}: {e}");
        }
    }
}

/// The generator must exercise both sides of the law: Restores folders that
/// are refused, and ones that are granted and really receive files.
#[test]
fn the_generator_is_not_vacuous() {
    use proptest::strategy::ValueTree;
    use proptest::test_runner::TestRunner;
    let mut runner = TestRunner::deterministic();
    let (mut granted, mut refused, mut landed) = (0, 0, 0);
    for _ in 0..150 {
        let scenario = scenario_strategy().new_tree(&mut runner).unwrap().current();
        let outcome = check(&scenario).unwrap_or_else(|e| panic!("{e}"));
        if outcome.granted {
            granted += 1;
        } else {
            refused += 1;
        }
        landed += usize::from(outcome.new_files_in_restores > 0);
    }
    eprintln!("granted {granted}, refused {refused}, cases with files written {landed}");
    assert!(granted >= 15, "only {granted}/150 Restores folders granted");
    assert!(refused >= 15, "only {refused}/150 Restores folders refused");
    assert!(landed >= 10, "only {landed}/150 cases wrote a file");
}

/// A sanity check that the property isn't vacuous: in the plain case a
/// Restores folder *is* granted and the restore *does* land.
#[test]
fn the_happy_path_really_writes() {
    let dir = tempfile::tempdir().unwrap();
    let base = fs::canonicalize(dir.path()).unwrap();
    fs::create_dir_all(base.join("Music/Logic")).unwrap();
    let mut roots = WatchedRoots::new();
    roots
        .add(&base.join("Music/Logic"), RootKind::Discovery)
        .unwrap();
    let restores = RestoresDir::new(&base.join("Music/Wit Restores"), &roots).unwrap();
    let dest = restores
        .fresh_destination("Song", "2026-09-29", Some("als"))
        .unwrap();
    let landed = write_file_in_restores(dest, b"bytes").unwrap();
    assert_eq!(fs::read(&landed).unwrap(), b"bytes");
    assert!(inside_by_identity(&landed, restores.path()));
}
