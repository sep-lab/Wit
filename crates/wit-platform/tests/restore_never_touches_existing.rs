//! Property test for ADR-0007's law: **a restore never modifies, overwrites,
//! renames or deletes anything that existed before the restore began; each
//! restore only creates one brand-new entry directly under the Restores
//! folder.**
//!
//! Each case builds a random world in a temp dir — nested folders whose
//! names collide by case (`Music`/`music`), by Unicode normal form (`Café`
//! NFC / NFD), and by full case folding (`straße`/`STRASSE`); symlinks (live
//! and dangling) pointing anywhere in it; fake Logic packages, Ableton
//! project folders (`Ableton Project Info`) and loose `.als`/`.flp` files —
//! then picks watched roots and a Restores folder, both spelled
//! adversarially (`x/..` detours, `.` components, trailing separators, case
//! and normal-form flips, symlink hops, on Windows the verbatim `\\?\` form,
//! very long names). The Restores folder is often *inside* a watched root,
//! which ADR-0007 allows. Before restoring, it plants symlinks and hard
//! links **inside the Restores folder** that point into projects, named
//! exactly like the restores about to be created. Some cases then attack
//! the Restores folder itself: move it into a package (leaving a decoy at
//! its path), turn its parent into a project folder, or have a thread flip
//! its path to a symlink into a package *while* restores run. Multi-step
//! restores may find a symlink planted inside their own staging folder.
//! Then it runs a random sequence of restores with hostile song names and
//! hostile relative paths.
//!
//! The oracle never trusts the path logic under test: it snapshots the whole
//! world (without following symlinks) and decides "is X inside Y" by **file
//! identity** (`same-file`: dev+inode on Unix, volume serial+file index on
//! Windows) along X's physical ancestors.
//!
//! Asserted for every case:
//! 1. `RestoresDir::new` only ever creates folders (the Restores folder and
//!    its missing parents), never changes existing content, and grants a
//!    folder only if it obeys the placement rules: not inside a project
//!    folder of any DAW, and not a parent of any other watched root;
//! 2. across the restores, **every pre-existing file, folder, symlink and
//!    hard link is byte-for-byte and metadata-identical** (type, size,
//!    mtime, read-only flag, inode, mode, BSD flags, symlink target; the
//!    Restores folder's own mtime/size may change, as adding an entry must);
//! 3. every new path lies under a **new** entry directly inside the Restores
//!    folder, exactly one per successful restore; no new entry is a symlink
//!    or shares an inode with anything that existed; and no new entry was
//!    created while the Restores folder sat inside a project folder.

use proptest::prelude::*;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(unix)]
use std::sync::Arc;
use std::time::SystemTime;
use unicode_normalization::UnicodeNormalization;
use wit_platform::clone::{
    clone_tree, remove_dir_contents_in_restores, write_file_in_restores, RestoresDir,
};
use wit_platform::roots::{RootKind, WatchedRoots};

const MARKER: &str = "Ableton Project Info";
const STAGING_PLANT: &str = "planted-link";

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
        MARKER.into(),
        "Jam.BAND".into(),
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
        "Song\u{202E}3pm.als".into(),
        "🎹".repeat(120),
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
/// Names planted inside the Restores folder: exactly the names the ops
/// below will try to create first.
const PLANTED: [&str; 5] = [
    "Song — 2026-09-29.als",
    "Song — d.logicx",
    "Staged — d",
    "Song — d (2).logicx",
    "Song — 2026-09-29 (2).flp",
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

/// One component of a relative path handed to a `NewRestore`.
#[derive(Debug, Clone)]
enum RelPart {
    Name(usize),
    Extra(usize),
    Up,
    Here,
    /// The absolute path of a file inside a watched root.
    AbsoluteWatched,
    /// The name of an entry planted in the Restores folder.
    Planted(usize),
    /// The symlink planted inside this restore's own staging folder.
    StagingPlant,
}

fn rel_strategy() -> impl Strategy<Value = Vec<RelPart>> {
    prop::collection::vec(
        prop_oneof![
            3 => (0..names().len()).prop_map(RelPart::Name),
            3 => (0..EXTRA_REL.len()).prop_map(RelPart::Extra),
            1 => Just(RelPart::Up),
            1 => Just(RelPart::Here),
            1 => Just(RelPart::AbsoluteWatched),
            1 => (0..PLANTED.len()).prop_map(RelPart::Planted),
            2 => Just(RelPart::StagingPlant),
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
    Multi {
        from_root: Option<usize>,
        /// Plant a symlink to this dir inside the restore's staging folder.
        plant_in_staging: Option<usize>,
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
            prop::option::weighted(0.4, 0usize..8),
            prop::collection::vec(rel_strategy(), 0..4),
            prop::collection::vec(rel_strategy(), 0..3),
            any::<bool>(),
        )
            .prop_map(
                |(from_root, plant_in_staging, writes, removes, commit)| Op::Multi {
                    from_root,
                    plant_in_staging,
                    writes,
                    removes,
                    commit,
                }
            ),
    ]
}

/// Something to plant inside the Restores folder before restoring.
#[derive(Debug, Clone)]
struct Plant {
    name: usize,
    target_dir: usize,
    /// Point at a file in the target (`f.txt`) rather than the folder.
    to_file: bool,
    /// A hard link (files only) instead of a symlink.
    hard: bool,
}

/// An attack on the Restores folder itself, after it was granted.
#[derive(Debug, Clone, Copy)]
enum Swap {
    None,
    /// Move the real Restores folder into a package; leave a decoy folder
    /// at its path.
    MoveIntoPackage,
    /// Make the Restores folder's parent an Ableton project folder.
    ParentBecomesProject,
    /// A thread flips the Restores path to a symlink into a package and back
    /// while the restores run (Unix).
    FlipSymlink,
    /// Move the real Restores folder into a package and leave a *symlink* to
    /// it at the old path (so the path still resolves to the same folder).
    SymlinkLeftBehind,
    /// Move the Restores folder's parent into a package and replace it with
    /// a symlink to where it went.
    AncestorSymlink,
}

#[derive(Debug, Clone)]
struct Scenario {
    /// Folders to create, as paths of name indices (depth 1–3).
    dirs: Vec<Vec<usize>>,
    /// `(dir, flp)`: a loose `loose.als` / `loose.flp` inside that dir.
    project_files: Vec<(usize, bool)>,
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
    plants: Vec<Plant>,
    swap: Swap,
    ops: Vec<Op>,
}

fn scenario_strategy() -> impl Strategy<Value = Scenario> {
    let n = names().len();
    let plant = (0..PLANTED.len(), 0usize..8, any::<bool>(), any::<bool>()).prop_map(
        |(name, target_dir, to_file, hard)| Plant {
            name,
            target_dir,
            to_file,
            hard,
        },
    );
    let swap = prop_oneof![
        6 => Just(Swap::None),
        1 => Just(Swap::MoveIntoPackage),
        1 => Just(Swap::ParentBecomesProject),
        2 => Just(Swap::FlipSymlink),
        1 => Just(Swap::SymlinkLeftBehind),
        1 => Just(Swap::AncestorSymlink),
    ];
    (
        prop::collection::vec(prop::collection::vec(0..n, 1..=3), 1..8),
        prop::collection::vec((0usize..8, any::<bool>()), 0..2),
        prop::collection::vec((0usize..8, 0usize..8, prop::bool::weighted(0.2)), 0..4),
        prop::collection::vec((0usize..8, spell_strategy(), any::<bool>()), 1..4),
        prop_oneof![
            1 => Just(Start::World),
            2 => (0usize..3).prop_map(Start::Root),
        ],
        prop::collection::vec(step_strategy(), 1..=5),
        (any::<bool>(), any::<bool>()),
        prop::collection::vec(plant, 0..4),
        swap,
        prop::collection::vec(op_strategy(), 1..5),
    )
        .prop_map(
            |(
                dirs,
                project_files,
                links,
                roots,
                start,
                steps,
                (trailing_separator, verbatim),
                plants,
                swap,
                ops,
            )| {
                Scenario {
                    dirs,
                    project_files,
                    links,
                    roots,
                    start,
                    steps,
                    trailing_separator,
                    verbatim,
                    plants,
                    swap,
                    ops,
                }
            },
        )
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Kind {
    File(Vec<u8>),
    Dir,
    Link(PathBuf),
    Other,
}

/// Everything about one entry the law says must not change.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    kind: Kind,
    len: u64,
    modified: Option<SystemTime>,
    readonly: bool,
    ino: u64,
    mode: u32,
    flags: u32,
}

#[cfg(unix)]
fn ino_mode(m: &fs::Metadata) -> (u64, u32) {
    use std::os::unix::fs::MetadataExt;
    (m.ino(), m.mode())
}
#[cfg(not(unix))]
fn ino_mode(_m: &fs::Metadata) -> (u64, u32) {
    (0, 0)
}

#[cfg(target_os = "macos")]
fn bsd_flags(m: &fs::Metadata) -> u32 {
    use std::os::macos::fs::MetadataExt;
    m.st_flags()
}
#[cfg(not(target_os = "macos"))]
fn bsd_flags(_m: &fs::Metadata) -> u32 {
    0
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
            let kind = if meta.file_type().is_symlink() {
                Kind::Link(fs::read_link(&p).unwrap_or_default())
            } else if meta.is_dir() {
                stack.push(p.clone());
                Kind::Dir
            } else if meta.is_file() {
                Kind::File(fs::read(&p).unwrap_or_default())
            } else {
                Kind::Other
            };
            let (ino, mode) = ino_mode(&meta);
            let entry = Entry {
                kind,
                len: meta.len(),
                modified: meta.modified().ok(),
                readonly: meta.permissions().readonly(),
                ino,
                mode,
                flags: bsd_flags(&meta),
            };
            out.insert(p, entry);
        }
    }
    out
}

fn same(a: &Path, b: &Path) -> bool {
    same_file::is_same_file(a, b).unwrap_or(false)
}

/// `x` is `root` or lies beneath it — by file identity along `x`'s physical
/// ancestors. Independent of how either is spelled.
fn inside_by_identity(x: &Path, root: &Path) -> bool {
    let Ok(canonical) = fs::canonicalize(x) else {
        return false;
    };
    canonical.ancestors().any(|a| same(a, root))
}

fn has_ext(p: &Path, exts: &[&str]) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| exts.iter().any(|x| e.eq_ignore_ascii_case(x)))
}

/// The oracle's own reading of "a DAW project folder" (ADR-0007).
fn is_project_folder(dir: &Path) -> bool {
    if has_ext(dir, &["logicx", "band"]) || dir.join(MARKER).is_dir() {
        return true;
    }
    if let Some(name) = dir.file_name() {
        let mut flp = name.to_os_string();
        flp.push(".flp");
        if dir.join(flp).is_file() {
            return true;
        }
    }
    fs::read_dir(dir.join("Backup")).is_ok_and(|entries| {
        entries
            .flatten()
            .any(|e| e.path().is_file() && has_ext(&e.path(), &["flp"]))
    })
}

fn inside_a_project(dir: &Path) -> bool {
    fs::canonicalize(dir).is_ok_and(|c| c.ancestors().any(is_project_folder))
}

fn symlink(target: &Path, link: &Path) -> bool {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link).is_ok()
    }
    #[cfg(windows)]
    {
        // Needs Developer Mode or elevation; best effort.
        if target.is_dir() {
            std::os::windows::fs::symlink_dir(target, link).is_ok()
        } else {
            std::os::windows::fs::symlink_file(target, link).is_ok()
        }
    }
}

fn with_trailing_separator(p: &Path) -> PathBuf {
    let mut s: OsString = p.as_os_str().to_os_string();
    s.push(std::path::MAIN_SEPARATOR_STR);
    PathBuf::from(s)
}

struct Built {
    _tmp: tempfile::TempDir,
    /// Wit's data folder (the staging journal) — outside the sandbox, so the
    /// oracle's "only one new entry under Restores" doesn't see it.
    _data_tmp: tempfile::TempDir,
    data: PathBuf,
    /// The world lives six levels below the temp dir, so however many `..`
    /// a generated spelling climbs (at most five), it stays in the sandbox.
    world: PathBuf,
    sandbox: PathBuf,
    dirs: Vec<PathBuf>,
    /// Watched roots, canonical, as the oracle knows them.
    roots: Vec<PathBuf>,
    watched: WatchedRoots,
    restores_spelling: PathBuf,
}

fn build(s: &Scenario) -> Built {
    let all = names();
    let tmp = tempfile::tempdir().unwrap();
    let data_tmp = tempfile::tempdir().unwrap();
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
            if has_ext(&p, &["logicx", "band"]) {
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
    for (dir, flp) in &s.project_files {
        let name = if *flp { "loose.flp" } else { "loose.als" };
        let _ = fs::write(dirs[dir % dirs.len()].join(name), b"project file");
    }
    for (i, (parent, target, dangling)) in s.links.iter().enumerate() {
        let parent = &dirs[parent % dirs.len()];
        let target = if *dangling {
            world.join("missing/deeper")
        } else {
            dirs[target % dirs.len()].clone()
        };
        symlink(&target, &parent.join(format!("lnk{i}")));
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
        data: data_tmp.path().join("WitData"),
        _tmp: tmp,
        _data_tmp: data_tmp,
        world,
        sandbox,
        dirs,
        roots,
        watched,
        restores_spelling: restores,
    }
}

/// Plant symlinks and hard links **inside the Restores folder**, pointing
/// into the world's projects, named like the restores about to be created.
fn plant(s: &Scenario, b: &Built, restores: &RestoresDir) -> usize {
    let mut planted = 0;
    for p in &s.plants {
        let at = restores.path().join(PLANTED[p.name]);
        if fs::symlink_metadata(&at).is_ok() {
            continue;
        }
        let dir = &b.dirs[p.target_dir % b.dirs.len()];
        let file = dir.join("f.txt");
        let ok = if p.hard && file.is_file() {
            fs::hard_link(&file, &at).is_ok()
        } else if p.to_file {
            symlink(&file, &at)
        } else {
            symlink(dir, &at)
        };
        planted += usize::from(ok);
    }
    planted
}

/// A package folder to aim attacks at (created if the world has none).
fn a_package(b: &Built) -> PathBuf {
    b.dirs
        .iter()
        .find(|d| has_ext(d, &["logicx", "band"]))
        .cloned()
        .unwrap_or_else(|| {
            let p = b.world.join("Target.logicx");
            let _ = fs::create_dir_all(p.join("Alternatives/000"));
            let _ = fs::write(p.join("Alternatives/000/ProjectData"), b"target bytes");
            p
        })
}

/// Apply a pre-restore attack on the Restores folder. Returns whether the
/// folder was moved away (so no restore may complete).
fn apply_swap(s: &Scenario, b: &Built, restores: &RestoresDir) -> bool {
    match s.swap {
        Swap::None | Swap::FlipSymlink => false,
        Swap::MoveIntoPackage => {
            let package = a_package(b);
            let into = package.join("moved-restores");
            if fs::rename(restores.path(), &into).is_ok() {
                // A decoy at the old path: same name, different folder.
                let _ = fs::create_dir(restores.path());
                true
            } else {
                false
            }
        }
        Swap::ParentBecomesProject => {
            if let Some(parent) = restores.path().parent() {
                let _ = fs::create_dir(parent.join(MARKER));
            }
            false
        }
        Swap::SymlinkLeftBehind => {
            let into = a_package(b).join("moved-restores");
            if fs::rename(restores.path(), &into).is_ok() {
                symlink(&into, restores.path());
                true
            } else {
                false
            }
        }
        Swap::AncestorSymlink => {
            // Only a parent strictly inside the world (never the sandbox's
            // own scaffolding), and only if it can be moved.
            let Some(parent) = restores.path().parent() else {
                return false;
            };
            let world = fs::canonicalize(&b.world).unwrap();
            let inside_world = parent.starts_with(&world) && parent != world;
            let Some(name) = parent.file_name() else {
                return false;
            };
            if !inside_world {
                return false;
            }
            let holder = b.world.join("Holder.logicx");
            let _ = fs::create_dir_all(holder.join("Alternatives/000"));
            let _ = fs::write(holder.join("Alternatives/000/ProjectData"), b"holder bytes");
            let moved = holder.join(name);
            if fs::rename(parent, &moved).is_ok() {
                symlink(&moved, parent);
                true
            } else {
                false
            }
        }
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
            RelPart::Planted(i) => p.push(PLANTED[*i]),
            RelPart::StagingPlant => p.push(STAGING_PLANT),
            RelPart::AbsoluteWatched => {
                let base = b.roots.first().unwrap_or(&b.world);
                p = base.join("f.txt").join(&p);
            }
        }
    }
    p
}

/// Run the ops; return how many restores completed (each should have created
/// exactly one new entry).
fn run_ops(s: &Scenario, b: &Built, restores: &RestoresDir) -> usize {
    let songs = songs();
    let mut completed = 0;
    for op in &s.ops {
        match op {
            Op::Write { song, ext } => {
                if let Ok(dest) =
                    restores.fresh_destination(&songs[*song], "2026-09-29", ext.map(|e| EXTS[e]))
                {
                    completed += usize::from(write_file_in_restores(dest, b"restored").is_ok());
                }
            }
            Op::Clone { root, song } => {
                let src = if b.roots.is_empty() {
                    b.dirs[*root % b.dirs.len()].clone()
                } else {
                    b.roots[*root % b.roots.len()].clone()
                };
                if let Ok(dest) = restores.fresh_destination(&songs[*song], "d", Some("logicx")) {
                    completed += usize::from(clone_tree(&src, dest).is_ok());
                }
            }
            Op::Multi {
                from_root,
                plant_in_staging,
                writes,
                removes,
                commit,
            } => {
                let Ok(mut new) = restores.begin_restore("Staged", "d", None) else {
                    continue;
                };
                if let Some(r) = from_root {
                    let src = b.roots.get(*r % b.roots.len().max(1)).unwrap_or(&b.world);
                    let _ = new.clone_tree_from(src);
                }
                if let Some(target) = plant_in_staging {
                    // An adversary with the user's permissions plants a link
                    // inside the restore's own staging folder.
                    let target = &b.dirs[target % b.dirs.len()];
                    symlink(target, &new.staging_path().join(STAGING_PLANT));
                }
                for w in writes {
                    let _ = new.write_file(&rel_path(w, b), b"swapped bytes");
                }
                for r in removes {
                    let _ = remove_dir_contents_in_restores(&mut new, &rel_path(r, b));
                }
                if *commit {
                    completed += usize::from(new.commit().is_ok());
                }
            }
        }
    }
    completed
}

/// A thread flipping the Restores path between the real folder and a
/// symlink into a package, until stopped; the real folder is back in place
/// when it returns.
#[cfg(unix)]
fn start_flipper(
    restores: &Path,
    package: PathBuf,
) -> (Arc<AtomicBool>, std::thread::JoinHandle<()>) {
    let stop = Arc::new(AtomicBool::new(false));
    let s2 = Arc::clone(&stop);
    let real = restores.to_path_buf();
    let aside = real.with_file_name(".restores-aside");
    let handle = std::thread::spawn(move || {
        while !s2.load(Ordering::SeqCst) {
            if fs::rename(&real, &aside).is_ok() {
                let _ = std::os::unix::fs::symlink(&package, &real);
                std::thread::yield_now();
                let _ = fs::remove_file(&real);
                let _ = fs::rename(&aside, &real);
            }
        }
    });
    (stop, handle)
}

/// What one case did — used to prove the generator isn't vacuous.
#[derive(Debug, Default, Clone, Copy)]
struct Outcome {
    granted: bool,
    inside_a_root: bool,
    planted: usize,
    swapped: bool,
    restores_completed: usize,
}

fn check(s: &Scenario) -> Result<Outcome, TestCaseError> {
    let b = build(s);
    let mut outcome = Outcome::default();

    // --- Setup: RestoresDir::new ------------------------------------------
    let pre_setup = snapshot(&b.sandbox);
    let granted = RestoresDir::new(&b.restores_spelling, &b.watched, &b.data);
    let post_setup = snapshot(&b.sandbox);
    for (path, entry) in &pre_setup {
        let after = post_setup.get(path);
        prop_assert!(
            after.is_some_and(|a| a.kind == entry.kind),
            "setup changed or removed {}",
            path.display()
        );
    }
    for path in post_setup.keys().filter(|p| !pre_setup.contains_key(*p)) {
        prop_assert!(
            fs::symlink_metadata(path).is_ok_and(|m| m.is_dir()),
            "setup created something other than a folder: {}",
            path.display()
        );
        if let Ok(restores) = &granted {
            prop_assert!(
                inside_by_identity(restores.path(), path),
                "setup created {} which is not on the way to the Restores folder",
                path.display()
            );
        }
    }
    let Ok(restores) = granted else {
        return Ok(outcome);
    };
    outcome.granted = true;
    let r = restores.path().to_path_buf();
    let r_id = same_file::Handle::from_path(&r).unwrap();
    prop_assert!(
        inside_by_identity(&r, &b.sandbox),
        "Restores escaped the sandbox"
    );

    // --- Placement rules (oracle by identity and by on-disk names) --------
    for root in &b.roots {
        prop_assert!(
            !(inside_by_identity(root, &r) && !same(root, &r)),
            "granted Restores {} contains watched root {}",
            r.display(),
            root.display()
        );
        outcome.inside_a_root |= inside_by_identity(&r, root) && !same(root, &r);
    }
    prop_assert!(
        !inside_a_project(&r),
        "granted Restores {} is inside a project",
        r.display()
    );

    // --- The restores ------------------------------------------------------
    outcome.planted = plant(s, &b, &restores);
    let moved_away = apply_swap(s, &b, &restores);
    outcome.swapped = !matches!(s.swap, Swap::None);
    let restores_parent = r.parent().map(Path::to_path_buf);
    // Any package the flipping thread aims at must exist before the snapshot.
    let flip_target = matches!(s.swap, Swap::FlipSymlink).then(|| a_package(&b));
    let before = snapshot(&b.sandbox);
    #[cfg(unix)]
    let flipper = flip_target.map(|target| start_flipper(&r, target));
    #[cfg(not(unix))]
    let _ = flip_target;
    outcome.restores_completed = run_ops(s, &b, &restores);
    #[cfg(unix)]
    if let Some((stop, handle)) = flipper {
        stop.store(true, Ordering::SeqCst);
        handle.join().unwrap();
    }
    let after = snapshot(&b.sandbox);

    // 2. Every pre-existing entry is byte-for-byte and metadata-identical.
    let flipped = matches!(s.swap, Swap::FlipSymlink);
    for (path, entry) in &before {
        let Some(now) = after.get(path) else {
            return Err(TestCaseError::fail(format!(
                "pre-existing {} was removed or renamed",
                path.display()
            )));
        };
        let is_restores = same_file::Handle::from_path(path).is_ok_and(|h| h == r_id);
        let is_flipped_parent = flipped && restores_parent.as_deref() == Some(path.as_path());
        if is_restores || is_flipped_parent {
            // Adding an entry must change the Restores folder's own mtime and
            // size (and the flipping thread changes its parent's).
            prop_assert_eq!(&now.kind, &entry.kind);
            continue;
        }
        prop_assert_eq!(now, entry, "pre-existing {} was modified", path.display());
    }
    // 3. Everything new: under one new entry directly inside the real
    //    Restores folder, one per completed restore, no symlinks, no inode
    //    shared with anything that existed, never inside a project folder.
    let old_inodes: BTreeSet<u64> = before
        .values()
        .filter(|e| matches!(e.kind, Kind::File(_)) && e.ino != 0)
        .map(|e| e.ino)
        .collect();
    let mut tops: BTreeSet<PathBuf> = BTreeSet::new();
    for (path, entry) in after.iter().filter(|(p, _)| !before.contains_key(*p)) {
        prop_assert!(
            !matches!(entry.kind, Kind::Link(_)),
            "a restore created a symlink: {}",
            path.display()
        );
        prop_assert!(
            !(matches!(entry.kind, Kind::File(_)) && old_inodes.contains(&entry.ino)),
            "{} shares an inode with a pre-existing file",
            path.display()
        );
        let top = path.ancestors().find(|a| {
            a.parent()
                .is_some_and(|parent| same_file::Handle::from_path(parent).is_ok_and(|h| h == r_id))
        });
        let Some(top) = top else {
            return Err(TestCaseError::fail(format!(
                "new entry {} is not inside the Restores folder {}",
                path.display(),
                r.display()
            )));
        };
        prop_assert!(
            !before.contains_key(top),
            "{} was written inside the pre-existing entry {}",
            path.display(),
            top.display()
        );
        prop_assert!(
            !inside_a_project(top.parent().unwrap()),
            "{} was created while the Restores folder sat inside a project",
            path.display()
        );
        tops.insert(top.to_path_buf());
    }
    if moved_away {
        prop_assert_eq!(
            outcome.restores_completed,
            0,
            "a restore completed after Restores moved"
        );
    }
    prop_assert_eq!(
        tops.len(),
        outcome.restores_completed,
        "new top-level entries {:?} vs completed restores",
        tops
    );
    Ok(outcome)
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 256,
        // Don't write `proptest-regressions/` into the source tree; a
        // failure prints its minimal case and seed instead.
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    #[test]
    fn a_restore_only_ever_creates_one_new_entry_in_restores(s in scenario_strategy()) {
        check(&s)?;
    }
}

/// The generator must exercise every side of the law: Restores folders that
/// are refused and granted, granted *inside* a watched root, with links
/// planted in them, attacked, and restores that really complete.
#[test]
fn the_generator_is_not_vacuous() {
    use proptest::strategy::ValueTree;
    use proptest::test_runner::TestRunner;
    let mut runner = TestRunner::deterministic();
    let (mut granted, mut refused, mut inside, mut planted, mut swapped, mut completed) =
        (0, 0, 0, 0, 0, 0);
    for _ in 0..200 {
        let scenario = scenario_strategy().new_tree(&mut runner).unwrap().current();
        let o = check(&scenario).unwrap_or_else(|e| panic!("{e}"));
        if o.granted {
            granted += 1;
        } else {
            refused += 1;
        }
        inside += usize::from(o.inside_a_root);
        planted += usize::from(o.planted > 0);
        swapped += usize::from(o.granted && o.swapped);
        completed += usize::from(o.restores_completed > 0);
    }
    eprintln!(
        "granted {granted}, refused {refused}, inside a watched root {inside}, \
         with planted links {planted}, attacked {swapped}, with completed restores {completed} (of 200)"
    );
    assert!(granted >= 20, "only {granted}/200 granted");
    assert!(refused >= 20, "only {refused}/200 refused");
    assert!(
        inside >= 10,
        "only {inside}/200 granted inside a watched root"
    );
    assert!(
        planted >= 10,
        "only {planted}/200 had links planted in Restores"
    );
    assert!(
        swapped >= 10,
        "only {swapped}/200 granted folders were attacked"
    );
    assert!(completed >= 20, "only {completed}/200 completed a restore");
}

#[allow(clippy::too_many_arguments)]
fn scenario(
    dirs: Vec<Vec<usize>>,
    roots: Vec<(usize, Spell, bool)>,
    start: Start,
    steps: Vec<Step>,
    ops: Vec<Op>,
) -> Scenario {
    Scenario {
        dirs,
        project_files: vec![],
        links: vec![],
        roots,
        start,
        steps,
        trailing_separator: false,
        verbatim: false,
        plants: vec![],
        swap: Swap::None,
        ops,
    }
}

/// The tricky cases by name, so each is exercised on every run regardless
/// of what the generator happens to draw.
#[test]
fn named_tricky_cases() {
    let name = |i: usize| Step::Name(i, Spell::Plain);
    let write = Op::Write {
        song: 0,
        ext: Some(1),
    };
    let multi_through_plant = Op::Multi {
        from_root: None,
        plant_in_staging: Some(0),
        writes: vec![
            vec![RelPart::StagingPlant, RelPart::Extra(2)],
            vec![RelPart::StagingPlant, RelPart::Name(9)],
        ],
        removes: vec![vec![RelPart::StagingPlant]],
        commit: true,
    };
    let mut cases: Vec<(&str, Scenario)> = Vec::new();

    let mut s = scenario(
        vec![vec![0, 5], vec![9]],
        vec![(0, Spell::Plain, false)],
        Start::World,
        vec![name(9), Step::Link(0), Step::Fresh(0)],
        vec![write.clone()],
    );
    s.links = vec![(1, 0, false)];
    cases.push(("Restores reached through a symlink into a watched root", s));

    let mut s = scenario(
        vec![vec![0, 5, 7]],
        vec![(0, Spell::Plain, false)],
        Start::World,
        vec![name(0), name(5), name(7), Step::Fresh(0)],
        vec![write.clone()],
    );
    s.trailing_separator = true;
    cases.push(("Restores inside a Logic package: refused", s));

    let s = scenario(
        vec![vec![0, 15]],
        vec![(0, Spell::Plain, false)],
        Start::World,
        vec![name(1), Step::Name(15, Spell::FlipCase), Step::Fresh(1)],
        vec![write.clone()],
    );
    cases.push(("Restores inside a case-variant .band package: refused", s));

    let s = scenario(
        vec![vec![9, 14], vec![9, 5]],
        vec![(1, Spell::Plain, true)],
        Start::World,
        vec![name(9), Step::Fresh(0)],
        vec![write.clone()],
    );
    cases.push(("Restores inside an Ableton project folder: refused", s));

    let s = scenario(
        vec![vec![0, 5]],
        vec![(0, Spell::Plain, false)],
        Start::World,
        vec![name(0)],
        vec![write.clone()],
    );
    cases.push(("Restores above a watched root: refused", s));

    let mut s = scenario(
        vec![vec![0, 5, 7], vec![0]],
        vec![(1, Spell::Plain, true)],
        Start::Root(0),
        vec![name(8)],
        vec![
            write.clone(),
            Op::Clone { root: 0, song: 0 },
            Op::Multi {
                from_root: Some(0),
                plant_in_staging: None,
                writes: vec![
                    vec![RelPart::Up, RelPart::Extra(2)],
                    vec![RelPart::AbsoluteWatched],
                    vec![RelPart::Extra(0), RelPart::Extra(1), RelPart::Extra(2)],
                    vec![RelPart::Extra(3)],
                    vec![RelPart::Up, RelPart::Planted(1), RelPart::Extra(2)],
                ],
                removes: vec![
                    vec![],
                    vec![RelPart::Up],
                    vec![RelPart::AbsoluteWatched],
                    vec![RelPart::Up, RelPart::Planted(2)],
                ],
                commit: true,
            },
        ],
    );
    s.plants = vec![
        Plant {
            name: 0,
            target_dir: 0,
            to_file: true,
            hard: true,
        },
        Plant {
            name: 1,
            target_dir: 0,
            to_file: false,
            hard: false,
        },
        Plant {
            name: 2,
            target_dir: 0,
            to_file: false,
            hard: false,
        },
    ];
    cases.push((
        "Restores inside ~/Music (a watched root) with links planted into the package",
        s,
    ));

    let mut s = scenario(
        vec![vec![0, 5, 7], vec![0]],
        vec![(1, Spell::Plain, true)],
        Start::Root(0),
        vec![name(8)],
        vec![multi_through_plant.clone()],
    );
    s.plants = vec![];
    cases.push((
        "A symlink planted inside the restore's own staging folder",
        s,
    ));

    for swap in [
        Swap::SymlinkLeftBehind,
        Swap::AncestorSymlink,
        Swap::MoveIntoPackage,
        Swap::ParentBecomesProject,
        Swap::FlipSymlink,
    ] {
        let mut s = scenario(
            vec![vec![0, 5, 7], vec![0, 9]],
            vec![(0, Spell::Plain, false)],
            Start::World,
            vec![name(0), name(9), name(8)],
            vec![
                write.clone(),
                Op::Clone { root: 0, song: 0 },
                multi_through_plant.clone(),
                write.clone(),
            ],
        );
        s.swap = swap;
        cases.push(("the Restores folder attacked after it was granted", s));
    }

    let mut s = scenario(
        vec![vec![3], vec![4]],
        vec![(0, Spell::FlipNorm, false)],
        Start::World,
        vec![name(4), Step::Fresh(2)],
        vec![write.clone()],
    );
    s.verbatim = true;
    cases.push(("NFD spelling of an NFC root, verbatim on Windows", s));

    let mut s = scenario(
        vec![vec![0, 5], vec![0, 9]],
        vec![(0, Spell::ViaDotDot(9), false)],
        Start::Root(0),
        vec![Step::Here],
        vec![write.clone(), Op::Clone { root: 0, song: 0 }],
    );
    s.trailing_separator = true;
    cases.push(("Restores equal to the root, spelled with `x/..` and `.`", s));

    let mut s = scenario(
        vec![vec![13, 13, 13]],
        vec![(0, Spell::Plain, false)],
        Start::World,
        vec![name(13), name(13), Step::Fresh(0), name(13)],
        vec![Op::Clone { root: 0, song: 0 }],
    );
    s.verbatim = true;
    cases.push(("very long names", s));

    for (label, scenario) in cases {
        if let Err(e) = check(&scenario) {
            panic!("{label} ({:?}): {e}", scenario.swap);
        }
    }
}

/// Review finding 1, as a deterministic test: flip the Restores path to a
/// symlink into a package for a while during restores; nothing may appear
/// inside the package.
#[cfg(unix)]
#[test]
fn flipping_the_restores_path_never_redirects_a_write() {
    let tmp = tempfile::tempdir().unwrap();
    let base = fs::canonicalize(tmp.path()).unwrap();
    let alt = base.join("Music/Logic/Song.logicx/Alternatives/000");
    fs::create_dir_all(&alt).unwrap();
    fs::write(alt.join("ProjectData"), b"current").unwrap();
    let mut roots = WatchedRoots::new();
    roots
        .add(&base.join("Music/Logic"), RootKind::Discovery)
        .unwrap();
    let restores = RestoresDir::new(&base.join("Restores"), &roots, &base.join("WitData")).unwrap();
    let package_before = snapshot(&base.join("Music"));
    let (stop, flipper) = start_flipper(restores.path(), alt.clone());
    let end = std::time::Instant::now() + std::time::Duration::from_secs(3);
    let (mut ok, mut tries) = (0, 0);
    while std::time::Instant::now() < end {
        tries += 1;
        if let Ok(dest) = restores.fresh_destination("Race", "d", Some("als")) {
            ok += usize::from(write_file_in_restores(dest, b"x").is_ok());
        }
        if let Ok(mut new) = restores.begin_restore("RaceDir", "d", None) {
            let _ = new.write_file(Path::new("inner/f"), b"x");
            ok += usize::from(new.commit().is_ok());
        }
    }
    stop.store(true, Ordering::SeqCst);
    flipper.join().unwrap();
    assert_eq!(
        snapshot(&base.join("Music")),
        package_before,
        "nothing changed or appeared inside the package"
    );
    let landed = fs::read_dir(restores.path()).unwrap().count();
    eprintln!(
        "flip race: {tries} attempts, {ok} completed, {landed} entries in the real Restores folder"
    );
    assert!(
        landed >= ok,
        "every completed restore is in the real Restores folder"
    );
}

/// The refusals the named cases rely on really are refusals.
#[test]
fn placement_refusals_are_real() {
    let dir = tempfile::tempdir().unwrap();
    let base = fs::canonicalize(dir.path()).unwrap();
    fs::create_dir_all(base.join("Music/Song.logicx")).unwrap();
    fs::create_dir_all(base.join("Live/Set Project").join(MARKER)).unwrap();
    fs::create_dir_all(base.join("FL/Proj")).unwrap();
    fs::write(base.join("FL/Proj/Proj.flp"), b"flp").unwrap();
    fs::create_dir_all(base.join("Music/Logic")).unwrap();
    let data = base.join("WitData");
    let mut roots = WatchedRoots::new();
    roots
        .add(&base.join("Music/Logic"), RootKind::Discovery)
        .unwrap();
    for bad in [
        base.join("Music/Song.logicx/R"),
        base.join("Music/SONG.LOGICX/R/"),
        base.join("Live/Set Project/R"),
        base.join("FL/Proj/deeper/R"),
        base.join("Music"),
    ] {
        assert!(
            RestoresDir::new(&bad, &roots, &data).is_err(),
            "{} should be refused",
            bad.display()
        );
    }
    RestoresDir::new(&base.join("Music/Logic/Wit Restores"), &roots, &data).unwrap();
}

/// The happy path really writes, and exactly one new entry appears.
#[test]
fn the_happy_path_really_writes() {
    let dir = tempfile::tempdir().unwrap();
    let base = fs::canonicalize(dir.path()).unwrap();
    fs::create_dir_all(base.join("Music/Logic")).unwrap();
    let data_dir = tempfile::tempdir().unwrap();
    let mut roots = WatchedRoots::new();
    roots
        .add(&base.join("Music"), RootKind::UserFolder)
        .unwrap();
    let restores =
        RestoresDir::new(&base.join("Music/Wit Restores"), &roots, data_dir.path()).unwrap();
    let before = snapshot(&base);
    let dest = restores
        .fresh_destination("Song", "2026-09-29", Some("als"))
        .unwrap();
    let landed = write_file_in_restores(dest, b"bytes").unwrap().path;
    let after = snapshot(&base);
    assert_eq!(fs::read(&landed).unwrap(), b"bytes");
    let new: Vec<&PathBuf> = after.keys().filter(|p| !before.contains_key(*p)).collect();
    assert_eq!(new.len(), 1, "{new:?}");
    assert!(same(new[0], &landed));
}
