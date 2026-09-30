//! A recursive folder watcher that reports **one event per save**, and only
//! once the save has finished.
//!
//! DAWs don't save in one write. Logic rewrites `ProjectData`, rotates nine
//! `Project File Backups`, and updates two plists; Live writes a temp file,
//! moves the old set into `Backup/`, and renames the new one into place;
//! other apps write a file in chunks. A raw watcher sees dozens of events
//! per save. This module turns them into one [`WatchEvent::Settled`] per
//! project, emitted only after that project's relevant files' `(size,
//! mtime)` have been unchanged for the settle window (default 2 s) **and**
//! no new raw event has arrived for it in that window.
//!
//! # Guarantees
//!
//! - **Read-only.** The watcher only stats files; it never opens one for
//!   writing (nor, for that matter, for reading).
//! - **The Restores folder is watched, like any root** (ADR-0007): a
//!   musician who keeps working in a restored copy keeps its history.
//!   [`WatchConfig::new`] requires the [`RestoresDir`], adds it as a root,
//!   and re-runs [`RestoresDir::check_against`] on the exact root set (so no
//!   watched root sits inside it). A restore in progress is invisible: it is
//!   staged under a hidden `.wit-staging-…` name and appears as one new
//!   project when it is renamed into place.
//! - **Wit's data dir is ignored.** Events under any folder passed to
//!   [`WatchConfig::ignore`] are dropped before classification.
//! - **Classification is a whitelist** (AGENTS.md: blacklists leak), and is
//!   a pure function ([`classify`]); debouncing is a pure state machine
//!   ([`Debouncer`]) driven by an injected clock. Both are unit-tested
//!   without touching the OS watcher.
//! - **No async runtime.** Events arrive on a `std::sync::mpsc` channel;
//!   read them with [`ProjectWatcher::recv`], [`recv_timeout`](ProjectWatcher::recv_timeout),
//!   [`try_recv`](ProjectWatcher::try_recv) or [`iter`](ProjectWatcher::iter).
//!   Dropping the watcher stops its thread.
//!
//! # What counts as a project, and which files count
//!
//! | Path under a watched root | Project root reported | Relevant (triggers + fingerprint) |
//! |---|---|---|
//! | inside `X.logicx/` or `X.band/` | the package | `Alternatives/*/ProjectData`, `Alternatives/*/MetaData.plist`, `Alternatives/*/Project File Backups/**`, `Resources/ProjectInformation.plist`, the package itself |
//! | `X.als` | the file | the file |
//! | `Backup/X [2026-05-05 095412].als` (Live's pattern) | `../X.als`, always (Live moves the old set into `Backup/` just before renaming the new one in) | the main set |
//! | `X.flp`, and `Backup/X (…).flp` / `Backup/X […].flp` | the file; a backup maps to `../X.flp` only if that exists (FL's backup naming is unverified) | the file |
//! | other DAW formats (`.rpp`, `.bwproject`, `.ardour`, `.song`, `.cpr`, `.dawproject`, …) | the file (`x.rpp-bak` → `x.rpp`) | the file |
//! | any other file, **in a user-added folder only** | the file (generic History tier) | the file |
//!
//! Never counted: a restore still being assembled (anything under a
//! `.wit-staging-…` entry in the Restores folder — it becomes one project
//! when it is renamed into place), hidden names (a leading `.` on any
//! component), editor
//! and download temp names (`~$x`, `x~`, `.tmp`, `.part`, …), `Media/`,
//! `Undo Data.nosync/` and UI-state files inside packages, and anything
//! under a `(A Document Being Saved By …)` folder (macOS safe-save
//! scratch). A project whose root has vanished when it settles (a moved
//! scratch copy, a deleted file) is dropped silently.
//!
//! Only events that can mean content changed are considered
//! ([`is_content_event`]): opens, reads, read-only closes and access-time
//! updates are dropped before classification.
//!
//! # Honest limits
//!
//! - The save sequences above are **inferred** from the on-disk results of
//!   real saves (FORMATS.md) and from how these apps are documented to save;
//!   the watcher was never run against a live DAW while writing this. The
//!   integration test replays synthetic save sequences (chunked writes,
//!   rename-into-place, backup rotation) on the real OS watcher.
//! - Two saves within one mtime tick that leave identical sizes (possible
//!   on HFS+'s 1 s or FAT's 2 s mtime) are reported once.
//! - A settled project is reported iff its fingerprint (size, mtime, and a
//!   change stamp: inode + ctime on Unix, creation time on Windows) differs
//!   from its **baseline** — the last one known, taken by a scan of every
//!   root when the watcher starts, then updated on each report. So a change
//!   is reported *whatever the files' ages* (an atomic replace by an old
//!   version, `cp -p`, `rsync -a`, unzip, a restore preserving mtimes), and
//!   an event that changed nothing (APFS reporting a clone's *source*,
//!   measured) is not. Events that arrive during the start-up scan make
//!   their project's baseline untrusted, so those are reported on settle.
//! - On Windows the change stamp is the creation time, so an in-place
//!   rewrite that keeps both size and mtime is not seen there.
//! - On Linux, reads are invisible only because opens/reads are filtered
//!   ([`is_content_event`]); on macOS, `stat`/`read_dir`/`read` produce no
//!   FSEvents at all.
//! - Memory is bounded: at most [`MAX_BASELINES`] fingerprints are kept and
//!   the start-up scan visits at most [`MAX_SCAN_ENTRIES`] entries; past
//!   either, a project's next change is reported even if it changed
//!   nothing (never the reverse).
//! - If the OS drops events (inotify queue overflow, FSEvents "must
//!   rescan"), a [`WatchEvent::NeedsRescan`] is sent for each root; the
//!   caller should rescan with discovery.

use crate::clone::{CloneError, FileId, RestoresDir, STAGING_PREFIX};
use crate::paths::{self, CaseSensitivity};
use crate::roots::{RootError, RootKind, WatchedRoot, WatchedRoots};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime};

/// Default settle window.
pub const DEFAULT_SETTLE: Duration = Duration::from_millis(2000);
/// How often pending projects are re-checked.
const TICK: Duration = Duration::from_millis(100);

/// What kind of project a settled event is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ProjectKind {
    /// A `.logicx` package.
    Logic,
    /// A `.band` package.
    GarageBand,
    /// An `.als` file.
    Ableton,
    /// An `.flp` file.
    FlStudio,
    /// Another DAW's project file (`.rpp`, `.bwproject`, …): History tier.
    OtherDaw,
    /// Any other file in a user-added folder: History tier.
    Generic,
}

/// A project, identified by the path Wit versions it under.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProjectRoot {
    pub path: PathBuf,
    pub kind: ProjectKind,
}

/// Other DAWs' project formats (lower-case), recognised in every root.
const OTHER_DAW_EXTENSIONS: &[&str] = &[
    "rpp",        // REAPER
    "bwproject",  // Bitwig Studio
    "ardour",     // Ardour
    "song",       // Studio One
    "cpr",        // Cubase
    "npr",        // Nuendo
    "ptx",        // Pro Tools
    "dawproject", // DAWproject exchange
    "mmpz",       // LMMS
    "mmp",        // LMMS (uncompressed)
    "rns",        // Renoise
    "reason",     // Reason
    "aup3",       // Audacity
];

fn lower_ext(name: &str) -> Option<String> {
    let (_, ext) = name.rsplit_once('.')?;
    Some(ext.to_ascii_lowercase())
}

/// Temp/lock names editors and browsers leave next to real files.
fn is_scratch_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    name.starts_with('.')
        || name.starts_with("~$")
        || name.ends_with('~')
        || name == "Icon\r"
        || lower == "thumbs.db"
        || lower == "desktop.ini"
        || [
            ".tmp",
            ".temp",
            ".swp",
            ".part",
            ".crdownload",
            ".download",
            ".partial",
        ]
        .iter()
        .any(|s| lower.ends_with(s))
}

fn in_backup_dir(file: &Path) -> bool {
    file.parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.eq_ignore_ascii_case("Backup"))
}

/// Live's backup naming, `Backup/<name> [YYYY-MM-DD HHMMSS].als` (the same
/// pattern `wit-index`'s discovery groups lineages by): the main set is
/// `../<name>.als`, **unconditionally** — Live moves the old set into
/// `Backup/` a moment before renaming the new one into place, so checking
/// whether the main set exists at that instant would race.
fn live_backup_owner(file: &Path) -> Option<PathBuf> {
    if !in_backup_dir(file) {
        return None;
    }
    let stem = file.file_stem()?.to_str()?;
    if stem.len() < 20 || !stem.is_char_boundary(stem.len() - 20) {
        return None;
    }
    let (name, tail) = stem.split_at(stem.len() - 20);
    let b = tail.as_bytes();
    let digits = |r: std::ops::Range<usize>| b[r].iter().all(u8::is_ascii_digit);
    let ok = b[0] == b' '
        && b[1] == b'['
        && digits(2..6)
        && b[6] == b'-'
        && digits(7..9)
        && b[9] == b'-'
        && digits(10..12)
        && b[12] == b' '
        && digits(13..19)
        && b[19] == b']';
    if !ok || name.is_empty() {
        return None;
    }
    Some(file.parent()?.parent()?.join(format!("{name}.als")))
}

/// Any other `Backup/<prefix> [..].ext` or `Backup/<prefix> (..).ext`
/// (FL Studio's backup naming isn't verified): the main project is
/// `../<prefix>.ext` — only when that exists, so a user's own file that
/// merely looks like a backup is never swallowed.
fn backup_owner(file: &Path, ext: &str, exists: &dyn Fn(&Path) -> bool) -> Option<PathBuf> {
    if !in_backup_dir(file) {
        return None;
    }
    let parent = file.parent()?;
    let stem = file.file_stem()?.to_str()?;
    for sep in [" [", " ("] {
        if let Some(i) = stem.rfind(sep) {
            let owner = parent.parent()?.join(format!("{}.{ext}", &stem[..i]));
            if exists(&owner) {
                return Some(owner);
            }
        }
    }
    None
}

/// Is `inner` (a path relative to a `.logicx`/`.band` package) one of the
/// files a save rewrites?
fn is_relevant_in_package(inner: &[&str]) -> bool {
    matches!(
        inner,
        [] | ["Alternatives", _, "ProjectData" | "MetaData.plist"]
            | ["Alternatives", _, "Project File Backups", ..]
            | ["Resources", "ProjectInformation.plist"]
    )
}

/// Map a raw event path to the project it belongs to — or `None` if it is
/// irrelevant. Pure apart from `exists`, which is only consulted to map a
/// `Backup/` file to its main project.
///
/// `path` must already be canonical and lie under `root` (the watcher
/// guarantees both).
pub fn classify(
    path: &Path,
    root: &WatchedRoot,
    exists: &dyn Fn(&Path) -> bool,
) -> Option<ProjectRoot> {
    if !paths::is_within_canonical(path, root.path(), CaseSensitivity::Insensitive) {
        return None;
    }
    let skip = root.path().components().count();
    let rel: Vec<Component<'_>> = path.components().skip(skip).collect();
    let mut names: Vec<&str> = Vec::with_capacity(rel.len());
    for c in &rel {
        let Component::Normal(n) = c else { return None };
        names.push(n.to_str()?);
    }
    // A restore being assembled (`.wit-staging-…` inside the Restores
    // folder) is not a project until it is renamed into place — checked by
    // name explicitly, not only via the hidden-name rule, so the guarantee
    // survives a change to the staging prefix.
    if names.iter().any(|n| {
        n.starts_with(STAGING_PREFIX)
            || n.starts_with('.')
            || n.starts_with("(A Document Being Saved By")
    }) {
        return None;
    }
    let mut base = root.path().to_path_buf();
    for (i, name) in names.iter().enumerate() {
        base.push(name);
        let ext = lower_ext(name);
        let package_kind = match ext.as_deref() {
            Some("logicx") => Some(ProjectKind::Logic),
            Some("band") => Some(ProjectKind::GarageBand),
            _ => None,
        };
        if let Some(kind) = package_kind {
            return is_relevant_in_package(&names[i + 1..])
                .then_some(ProjectRoot { path: base, kind });
        }
        if i + 1 < names.len() {
            continue;
        }
        // The last component: a file.
        if is_scratch_name(name) {
            return None;
        }
        let file = base;
        let kind = match ext.as_deref() {
            Some("als") => ProjectKind::Ableton,
            Some("flp") => ProjectKind::FlStudio,
            Some("rpp-bak") => {
                let owner = file.with_extension("rpp");
                return Some(ProjectRoot {
                    path: if exists(&owner) { owner } else { file },
                    kind: ProjectKind::OtherDaw,
                });
            }
            Some(e) if OTHER_DAW_EXTENSIONS.contains(&e) => ProjectKind::OtherDaw,
            _ if root.kind() == RootKind::UserFolder => ProjectKind::Generic,
            _ => return None,
        };
        let path = match (kind, ext.as_deref()) {
            (ProjectKind::Ableton, Some(e)) => live_backup_owner(&file)
                .or_else(|| backup_owner(&file, e, exists))
                .unwrap_or(file),
            (ProjectKind::FlStudio, Some(e)) => backup_owner(&file, e, exists).unwrap_or(file),
            _ => file,
        };
        return Some(ProjectRoot { path, kind });
    }
    None
}

/// Whether a raw event can mean *content* changed. Opens, reads and
/// read-only closes cannot, and must be dropped: Linux reports `IN_OPEN` and
/// `IN_CLOSE_NOWRITE` for directories too, so without this filter the
/// watcher's own `read_dir` of `Project File Backups` while fingerprinting
/// would re-arm the debounce on every tick and a Logic save would never
/// settle (caught by the ubuntu CI leg). A close after writing is kept;
/// unknown kinds are kept (conservative).
pub fn is_content_event(kind: &notify::EventKind) -> bool {
    use notify::event::{AccessKind, AccessMode, EventKind, MetadataKind, ModifyKind};
    match kind {
        EventKind::Access(AccessKind::Close(AccessMode::Write)) => true,
        EventKind::Access(_) => false,
        EventKind::Modify(ModifyKind::Metadata(MetadataKind::AccessTime)) => false,
        _ => true,
    }
}

/// What identifies one version of a file, cheaply: size, mtime, and a
/// change stamp the writer can't forge — `(inode, ctime)` on Unix (an
/// atomic replace gets a new inode; any write or `utimes` bumps ctime),
/// `(0, creation time)` on Windows (a replace-by-rename brings the new
/// file's creation time).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FileStamp {
    pub len: u64,
    pub modified: Option<SystemTime>,
    pub change: (u64, i64),
}

impl FileStamp {
    fn of(meta: &std::fs::Metadata) -> FileStamp {
        FileStamp {
            len: meta.len(),
            modified: meta.modified().ok(),
            change: change_stamp(meta),
        }
    }
}

#[cfg(unix)]
fn change_stamp(meta: &std::fs::Metadata) -> (u64, i64) {
    use std::os::unix::fs::MetadataExt;
    (
        meta.ino(),
        meta.ctime()
            .saturating_mul(1_000_000_000)
            .saturating_add(meta.ctime_nsec()),
    )
}

#[cfg(windows)]
fn change_stamp(meta: &std::fs::Metadata) -> (u64, i64) {
    use std::os::windows::fs::MetadataExt;
    (0, meta.creation_time() as i64)
}

#[cfg(not(any(unix, windows)))]
fn change_stamp(_meta: &std::fs::Metadata) -> (u64, i64) {
    (0, 0)
}

/// `(relative path, stamp)` of every relevant file of a project, sorted.
pub type Fingerprint = Vec<(PathBuf, FileStamp)>;

fn stat_into(path: &Path, rel: PathBuf, out: &mut Fingerprint) {
    if let Ok(m) = std::fs::symlink_metadata(path) {
        if m.is_file() {
            out.push((rel, FileStamp::of(&m)));
        }
    }
}

fn subdirs(dir: &Path) -> Vec<(String, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| Some((e.file_name().to_str()?.to_string(), e.path())))
        .collect()
}

/// Stat a project's relevant files. `None` if the project is gone.
pub fn fingerprint(project: &ProjectRoot) -> Option<Fingerprint> {
    let meta = std::fs::symlink_metadata(&project.path).ok()?;
    let mut fp = Fingerprint::new();
    match project.kind {
        ProjectKind::Logic | ProjectKind::GarageBand => {
            if !meta.is_dir() {
                return None;
            }
            let root = &project.path;
            for (alt, alt_dir) in subdirs(&root.join("Alternatives")) {
                let rel = PathBuf::from("Alternatives").join(&alt);
                for f in ["ProjectData", "MetaData.plist"] {
                    stat_into(&alt_dir.join(f), rel.join(f), &mut fp);
                }
                let backups = alt_dir.join("Project File Backups");
                for (slot, slot_dir) in subdirs(&backups) {
                    if let Ok(files) = std::fs::read_dir(&slot_dir) {
                        for f in files.flatten() {
                            let name = f.file_name();
                            stat_into(
                                &f.path(),
                                rel.join("Project File Backups").join(&slot).join(name),
                                &mut fp,
                            );
                        }
                    }
                }
            }
            stat_into(
                &root.join("Resources/ProjectInformation.plist"),
                PathBuf::from("Resources/ProjectInformation.plist"),
                &mut fp,
            );
        }
        _ => {
            if !meta.is_file() {
                return None;
            }
            fp.push((PathBuf::new(), FileStamp::of(&meta)));
        }
    }
    fp.sort();
    Some(fp)
}

#[derive(Debug)]
struct Pending {
    last_event: Instant,
    fp: Option<Option<Fingerprint>>,
    fp_since: Instant,
}

/// Most projects whose last-known fingerprint is remembered. Past this, the
/// oldest-keyed entries are forgotten — a forgotten project's next event is
/// then reported even if nothing changed (never the reverse), so the bound
/// can only cost a spurious event, never lost history.
pub const MAX_BASELINES: usize = 100_000;

/// The write-stability state machine, separated from the OS watcher so it
/// can be tested with a fake clock and scripted fingerprints.
#[derive(Debug)]
pub struct Debouncer {
    settle: Duration,
    pending: BTreeMap<ProjectRoot, Pending>,
    /// The last fingerprint known for each project: from the scan at watch
    /// start, or the last one reported.
    baselines: BTreeMap<PathBuf, Fingerprint>,
}

impl Debouncer {
    pub fn new(settle: Duration) -> Debouncer {
        Debouncer {
            settle,
            pending: BTreeMap::new(),
            baselines: BTreeMap::new(),
        }
    }

    /// Record what `path` looked like before any event (the watch-start scan).
    pub fn set_baseline(&mut self, path: PathBuf, fp: Fingerprint) {
        self.baselines.insert(path, fp);
        while self.baselines.len() > MAX_BASELINES {
            self.baselines.pop_first();
        }
    }

    /// Forget what `path` looked like, so its next settle is reported.
    pub fn forget(&mut self, path: &Path) {
        self.baselines.remove(path);
    }

    /// A relevant raw event for `project` arrived at `now`.
    pub fn observe(&mut self, project: ProjectRoot, now: Instant) {
        self.pending
            .entry(project)
            .and_modify(|p| p.last_event = now)
            .or_insert(Pending {
                last_event: now,
                fp: None,
                fp_since: now,
            });
    }

    /// Re-stat every pending project and return the ones that have settled:
    /// no raw event **and** an unchanged fingerprint for the whole settle
    /// window. A settled project is reported iff its fingerprint differs
    /// from its baseline (the last one known) — **whatever the files'
    /// ages**: an atomic replace by an old version, `cp -p`, `rsync -a`,
    /// unzip, a Finder copy-replace, or a restore that preserves mtimes all
    /// change size, inode or ctime and are reported. Events that changed
    /// nothing are dropped: on APFS, cloning a watched file (which a restore
    /// does) makes FSEvents report the *source* as created and modified,
    /// but its size, inode, mtime and ctime are untouched (measured), so its
    /// fingerprint equals its baseline. A project with no baseline (new, or
    /// forgotten) is reported. A project that vanished is dropped.
    pub fn poll(
        &mut self,
        now: Instant,
        mut stat: impl FnMut(&ProjectRoot) -> Option<Fingerprint>,
    ) -> Vec<ProjectRoot> {
        let mut settled = Vec::new();
        let mut done = Vec::new();
        for (project, pending) in self.pending.iter_mut() {
            let fp = stat(project);
            if pending.fp.as_ref() != Some(&fp) {
                pending.fp = Some(fp);
                pending.fp_since = now;
            }
            let quiet = now.saturating_duration_since(pending.last_event) >= self.settle;
            let stable = now.saturating_duration_since(pending.fp_since) >= self.settle;
            if quiet && stable {
                done.push(project.clone());
            }
        }
        for project in done {
            let Some(pending) = self.pending.remove(&project) else {
                continue;
            };
            let Some(Some(fp)) = pending.fp else {
                self.baselines.remove(&project.path);
                continue;
            };
            if self.baselines.get(&project.path) == Some(&fp) {
                continue;
            }
            self.set_baseline(project.path.clone(), fp);
            settled.push(project);
        }
        settled
    }

    /// Whether anything is waiting to settle.
    pub fn is_idle(&self) -> bool {
        self.pending.is_empty()
    }
}

/// Most filesystem entries the watch-start baseline scan visits; past this,
/// remaining projects simply have no baseline (their first event is
/// reported — conservative).
pub const MAX_SCAN_ENTRIES: usize = 200_000;
const MAX_SCAN_DEPTH: usize = 32;

/// Walk every outermost root and fingerprint each project found, so a
/// later event can be compared with what was there before it.
fn scan_baselines(config: &WatchConfig, debouncer: &mut Debouncer, stop: &AtomicBool) {
    let exists = |p: &Path| p.exists();
    let mut visited = 0usize;
    let mut seen: BTreeSet<PathBuf> = BTreeSet::new();
    let mut stack: Vec<(PathBuf, usize)> = config
        .roots
        .outermost()
        .iter()
        .map(|r| (r.path().to_path_buf(), 0))
        .collect();
    while let Some((dir, depth)) = stack.pop() {
        if stop.load(Ordering::Relaxed) || visited >= MAX_SCAN_ENTRIES {
            return;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            visited += 1;
            let path = entry.path();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_symlink() || config.is_ignored(&path) {
                continue;
            }
            let Some(root) = config.roots.root_for(&path) else {
                continue;
            };
            let is_package = kind.is_dir()
                && path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
                    e.eq_ignore_ascii_case("logicx") || e.eq_ignore_ascii_case("band")
                });
            if kind.is_dir() && !is_package {
                let hidden = entry.file_name().to_string_lossy().starts_with('.');
                if !hidden && depth < MAX_SCAN_DEPTH {
                    stack.push((path, depth + 1));
                }
                continue;
            }
            if let Some(project) = classify(&path, root, &exists) {
                if seen.insert(project.path.clone()) {
                    if let Some(fp) = fingerprint(&project) {
                        debouncer.set_baseline(project.path, fp);
                    }
                }
            }
        }
    }
}

/// What the watcher reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatchEvent {
    /// A project finished saving.
    Settled {
        project: ProjectRoot,
        /// The watched root it was found under.
        root: PathBuf,
        at: SystemTime,
    },
    /// The OS dropped events for this root; rescan it with discovery.
    NeedsRescan { root: PathBuf },
    /// A non-fatal watcher error (e.g. a folder that couldn't be watched).
    Error(String),
}

#[derive(Debug)]
pub enum WatchError {
    /// The Restores folder breaks ADR-0007's placement rules for this root
    /// set (a root sits inside it, or it sits inside a project).
    Restores(CloneError),
    /// The Restores folder couldn't be added as a watched root.
    Root(RootError),
    /// The OS watcher couldn't start or couldn't watch a root.
    Notify(notify::Error),
    /// No roots to watch.
    NoRoots,
}

impl fmt::Display for WatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WatchError::Restores(e) => write!(f, "{e}"),
            WatchError::Root(e) => write!(f, "{e}"),
            WatchError::Notify(e) => write!(f, "the file watcher failed: {e}"),
            WatchError::NoRoots => write!(f, "there are no folders to watch"),
        }
    }
}

impl std::error::Error for WatchError {}

/// What to watch, and what to ignore.
#[derive(Debug, Clone)]
pub struct WatchConfig {
    roots: WatchedRoots,
    ignore: Vec<PathBuf>,
    settle: Duration,
}

impl WatchConfig {
    /// Watch `roots` **plus the Restores folder**, never Wit's own data
    /// folder (both taken from `restores`, so neither can be forgotten).
    ///
    /// The Restores folder is added as a user folder (so a restored copy of
    /// any format keeps its history) — unless it already *is* one of the
    /// roots, which then keeps its own kind (a discovery root is not widened
    /// to the generic tier). Fails if a root in `roots` sits inside the
    /// Restores folder or the Restores folder has ended up inside a project —
    /// so no root set that breaks ADR-0007's placement rules is ever watched.
    pub fn new(mut roots: WatchedRoots, restores: &RestoresDir) -> Result<WatchConfig, WatchError> {
        restores
            .check_against(&roots)
            .map_err(WatchError::Restores)?;
        restores.revalidate().map_err(WatchError::Restores)?;
        let restores_id = FileId::of_path(restores.path()).ok();
        let already_a_root = roots
            .iter()
            .any(|r| restores_id.is_some() && FileId::of_path(r.path()).ok() == restores_id);
        if !already_a_root {
            roots
                .add(restores.path(), RootKind::UserFolder)
                .map_err(WatchError::Root)?;
        }
        Ok(WatchConfig {
            roots,
            ignore: vec![restores.data_dir().to_path_buf()],
            settle: DEFAULT_SETTLE,
        })
    }

    /// Also ignore everything under `dir`.
    pub fn ignore(mut self, dir: &Path) -> WatchConfig {
        if let Ok(c) = paths::canonicalize_lenient(dir) {
            self.ignore.push(c);
        }
        self
    }

    /// Change the settle window (default [`DEFAULT_SETTLE`]).
    pub fn settle(mut self, settle: Duration) -> WatchConfig {
        self.settle = settle;
        self
    }

    fn is_ignored(&self, path: &Path) -> bool {
        self.ignore
            .iter()
            .any(|dir| paths::is_within_canonical(path, dir, CaseSensitivity::Insensitive))
    }
}

/// A running watcher. Drop it to stop.
pub struct ProjectWatcher {
    events: Receiver<WatchEvent>,
    stop: Arc<AtomicBool>,
    watcher: Option<notify::RecommendedWatcher>,
    thread: Option<JoinHandle<()>>,
}

impl fmt::Debug for ProjectWatcher {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProjectWatcher").finish_non_exhaustive()
    }
}

impl ProjectWatcher {
    /// Start watching every outermost root recursively. The OS watch is
    /// registered first; the thread then fingerprints every existing project
    /// (the baseline scan) before reporting anything.
    pub fn start(config: WatchConfig) -> Result<ProjectWatcher, WatchError> {
        use notify::Watcher;
        if config.roots.is_empty() {
            return Err(WatchError::NoRoots);
        }
        let (raw_tx, raw_rx) = mpsc::channel::<notify::Result<notify::Event>>();
        let mut watcher = notify::RecommendedWatcher::new(
            raw_tx,
            notify::Config::default().with_follow_symlinks(false),
        )
        .map_err(WatchError::Notify)?;
        for root in config.roots.outermost() {
            watcher
                .watch(
                    paths::simplified(root.path()),
                    notify::RecursiveMode::Recursive,
                )
                .map_err(WatchError::Notify)?;
        }
        let (out_tx, out_rx) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let thread = std::thread::Builder::new()
            .name("wit-watch".into())
            .spawn(move || run_loop(config, raw_rx, out_tx, thread_stop))
            .map_err(|e| WatchError::Notify(notify::Error::io(e)))?;
        Ok(ProjectWatcher {
            events: out_rx,
            stop,
            watcher: Some(watcher),
            thread: Some(thread),
        })
    }

    /// Block until the next event (`None` once the watcher has stopped).
    pub fn recv(&self) -> Option<WatchEvent> {
        self.events.recv().ok()
    }

    pub fn recv_timeout(&self, timeout: Duration) -> Result<WatchEvent, RecvTimeoutError> {
        self.events.recv_timeout(timeout)
    }

    pub fn try_recv(&self) -> Option<WatchEvent> {
        self.events.try_recv().ok()
    }

    /// Blocking iterator over events; ends when the watcher stops.
    pub fn iter(&self) -> impl Iterator<Item = WatchEvent> + '_ {
        self.events.iter()
    }
}

impl Drop for ProjectWatcher {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        drop(self.watcher.take());
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Feed one raw notify event to the debouncer. `fresh` marks events that
/// arrived while the baseline scan ran: their projects' baselines may
/// already include the change, so they are forgotten (reported on settle).
fn handle_event(
    config: &WatchConfig,
    debouncer: &mut Debouncer,
    rescan: &mut BTreeSet<PathBuf>,
    event: notify::Event,
    now: Instant,
    during_scan: bool,
) {
    let exists = |p: &Path| p.exists();
    if event.need_rescan() {
        rescan.extend(
            config
                .roots
                .outermost()
                .iter()
                .map(|r| r.path().to_path_buf()),
        );
    }
    if !is_content_event(&event.kind) {
        return;
    }
    for path in &event.paths {
        if config.is_ignored(path) {
            continue;
        }
        let Some(root) = config.roots.root_for(path) else {
            continue;
        };
        if let Some(project) = classify(path, root, &exists) {
            if during_scan {
                debouncer.forget(&project.path);
            }
            debouncer.observe(project, now);
        }
    }
}

fn run_loop(
    config: WatchConfig,
    raw: Receiver<notify::Result<notify::Event>>,
    out: Sender<WatchEvent>,
    stop: Arc<AtomicBool>,
) {
    let mut debouncer = Debouncer::new(config.settle);
    scan_baselines(&config, &mut debouncer, &stop);
    let mut first_pass = true;
    loop {
        if stop.load(Ordering::SeqCst) {
            return;
        }
        let first = if first_pass {
            None
        } else {
            match raw.recv_timeout(TICK) {
                Ok(e) => Some(e),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        };
        let now = Instant::now();
        let mut rescan: BTreeSet<PathBuf> = BTreeSet::new();
        for raw_event in first.into_iter().chain(raw.try_iter()) {
            match raw_event {
                Ok(event) => {
                    handle_event(&config, &mut debouncer, &mut rescan, event, now, first_pass)
                }
                Err(e) => {
                    if out.send(WatchEvent::Error(e.to_string())).is_err() {
                        return;
                    }
                }
            }
        }
        first_pass = false;
        for root in rescan {
            if out.send(WatchEvent::NeedsRescan { root }).is_err() {
                return;
            }
        }
        if debouncer.is_idle() {
            continue;
        }
        for project in debouncer.poll(Instant::now(), fingerprint) {
            let root = config
                .roots
                .root_for(&project.path)
                .map(|r| r.path().to_path_buf())
                .unwrap_or_default();
            let event = WatchEvent::Settled {
                project,
                root,
                at: SystemTime::now(),
            };
            if out.send(event).is_err() {
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root(kind: RootKind) -> (tempfile::TempDir, WatchedRoot) {
        let dir = tempfile::tempdir().unwrap();
        let mut roots = WatchedRoots::new();
        roots.add(dir.path(), kind).unwrap();
        let r = roots.iter().next().unwrap().clone();
        (dir, r)
    }

    fn at(r: &WatchedRoot, rel: &str) -> PathBuf {
        rel.split('/')
            .fold(r.path().to_path_buf(), |p, c| p.join(c))
    }

    fn never(_: &Path) -> bool {
        false
    }

    #[test]
    fn logic_package_whitelist() {
        let (_d, r) = root(RootKind::Discovery);
        let pkg = at(&r, "Songs/Tune.logicx");
        for rel in [
            "Songs/Tune.logicx",
            "Songs/Tune.logicx/Alternatives/000/ProjectData",
            "Songs/Tune.logicx/Alternatives/001/MetaData.plist",
            "Songs/Tune.logicx/Alternatives/000/Project File Backups/03/ProjectData",
            "Songs/Tune.logicx/Resources/ProjectInformation.plist",
        ] {
            let got = classify(&at(&r, rel), &r, &never).unwrap_or_else(|| panic!("{rel}"));
            assert_eq!(
                got,
                ProjectRoot {
                    path: pkg.clone(),
                    kind: ProjectKind::Logic
                },
                "{rel}"
            );
        }
        for rel in [
            "Songs/Tune.logicx/Media/Audio Files/Take 1.wav",
            "Songs/Tune.logicx/Alternatives/000/Undo Data.nosync/1",
            "Songs/Tune.logicx/Alternatives/000/DisplayState.plist",
            "Songs/Tune.logicx/Alternatives/000/WindowImage.jpg",
            "Songs/Tune.logicx/Alternatives/000/Autosave/ProjectData",
            "(A Document Being Saved By Logic Pro)/Tune.logicx/Alternatives/000/ProjectData",
            "Songs/.Tune.logicx/Alternatives/000/ProjectData",
        ] {
            assert_eq!(classify(&at(&r, rel), &r, &never), None, "{rel}");
        }
        let band = classify(&at(&r, "Jam.band/Alternatives/000/ProjectData"), &r, &never).unwrap();
        assert_eq!(band.kind, ProjectKind::GarageBand);
    }

    #[test]
    fn ableton_and_fl_backups_map_to_their_main_file_when_it_exists() {
        let (_d, r) = root(RootKind::Discovery);
        let main = at(&r, "Set Project/Set.als");
        let exists = |p: &Path| p == main;
        let got = classify(
            &at(&r, "Set Project/Backup/Set [2026-05-05 095412].als"),
            &r,
            &exists,
        )
        .unwrap();
        assert_eq!(
            got,
            ProjectRoot {
                path: main.clone(),
                kind: ProjectKind::Ableton
            }
        );
        // Live's own naming maps even before the new main set is renamed in
        // (it settles, or is dropped if the main set never appears).
        let before_rename = classify(
            &at(&r, "Set Project/Backup/Set [2026-05-05 095412].als"),
            &r,
            &never,
        )
        .unwrap();
        assert_eq!(before_rename.path, main);
        // A look-alike that isn't Live's pattern needs its owner to exist.
        let lookalike = at(&r, "Other/Backup/Mix (final).als");
        assert_eq!(classify(&lookalike, &r, &never).unwrap().path, lookalike);
        let not_backup = at(&r, "Other/Set [2026-05-05 095412].als");
        assert_eq!(classify(&not_backup, &r, &never).unwrap().path, not_backup);

        let flp = at(&r, "Beat.flp");
        let exists = |p: &Path| p == flp;
        let got = classify(
            &at(&r, "Backup/Beat (overwritten at 1709h24).flp"),
            &r,
            &exists,
        )
        .unwrap();
        assert_eq!(
            got,
            ProjectRoot {
                path: flp.clone(),
                kind: ProjectKind::FlStudio
            }
        );
        assert_eq!(
            classify(&flp, &r, &never).unwrap().kind,
            ProjectKind::FlStudio
        );
    }

    #[test]
    fn scratch_and_hidden_names_never_count() {
        let (_d, r) = root(RootKind::UserFolder);
        for rel in [
            ".Set.als.tmp",
            "Set.als.tmp",
            "~$notes.docx",
            "notes.txt~",
            "x.part",
            "Thumbs.db",
            ".DS_Store",
            "a/.git/HEAD",
        ] {
            assert_eq!(classify(&at(&r, rel), &r, &never), None, "{rel}");
        }
    }

    #[test]
    fn generic_tier_only_in_user_folders() {
        let (_d, user) = root(RootKind::UserFolder);
        let (_e, disc) = root(RootKind::Discovery);
        assert_eq!(
            classify(&at(&user, "a/take.txt"), &user, &never)
                .unwrap()
                .kind,
            ProjectKind::Generic
        );
        assert_eq!(classify(&at(&disc, "a/take.txt"), &disc, &never), None);
        for (rel, kind) in [
            ("song.rpp", ProjectKind::OtherDaw),
            ("x.bwproject", ProjectKind::OtherDaw),
            ("Y.SONG", ProjectKind::OtherDaw),
        ] {
            assert_eq!(
                classify(&at(&disc, rel), &disc, &never).unwrap().kind,
                kind,
                "{rel}"
            );
        }
        let rpp = at(&disc, "song.rpp");
        let exists = |p: &Path| p == rpp;
        assert_eq!(
            classify(&at(&disc, "song.rpp-bak"), &disc, &exists)
                .unwrap()
                .path,
            rpp
        );
    }

    #[test]
    fn a_restore_being_staged_is_not_a_project_until_renamed() {
        let (_d, r) = root(RootKind::UserFolder);
        for rel in [
            ".wit-staging-123-456-0",
            ".wit-staging-123-456-0/Alternatives/000/ProjectData",
            ".wit-staging-123-456-0/Alternatives/000/.wit-tmp-9",
        ] {
            assert_eq!(classify(&at(&r, rel), &r, &never), None, "{rel}");
        }
        let landed = classify(&at(&r, "Song — 2026-09-29.logicx"), &r, &never).unwrap();
        assert_eq!(landed.kind, ProjectKind::Logic);
    }

    #[test]
    fn only_content_events_count() {
        use notify::event::{
            AccessKind, AccessMode, CreateKind, DataChange, EventKind, MetadataKind, ModifyKind,
            RenameMode,
        };
        for kind in [
            EventKind::Access(AccessKind::Open(AccessMode::Any)),
            EventKind::Access(AccessKind::Read),
            EventKind::Access(AccessKind::Close(AccessMode::Read)),
            EventKind::Modify(ModifyKind::Metadata(MetadataKind::AccessTime)),
        ] {
            assert!(!is_content_event(&kind), "{kind:?}");
        }
        for kind in [
            EventKind::Access(AccessKind::Close(AccessMode::Write)),
            EventKind::Create(CreateKind::File),
            EventKind::Modify(ModifyKind::Data(DataChange::Any)),
            EventKind::Modify(ModifyKind::Name(RenameMode::To)),
            EventKind::Modify(ModifyKind::Metadata(MetadataKind::WriteTime)),
            EventKind::Remove(notify::event::RemoveKind::Any),
            EventKind::Any,
            EventKind::Other,
        ] {
            assert!(is_content_event(&kind), "{kind:?}");
        }
    }

    #[test]
    fn paths_outside_the_root_are_ignored() {
        let (_d, r) = root(RootKind::UserFolder);
        let (_e, other) = root(RootKind::UserFolder);
        assert_eq!(classify(&at(&other, "x.als"), &r, &never), None);
    }

    fn ms(t0: Instant, n: u64) -> Instant {
        t0 + Duration::from_millis(n)
    }

    fn stamp(len: u64, change: i64) -> FileStamp {
        FileStamp {
            len,
            modified: None,
            change: (1, change),
        }
    }

    fn fp(size: u64) -> Option<Fingerprint> {
        Some(vec![(PathBuf::new(), stamp(size, 0))])
    }

    #[test]
    fn debouncer_waits_for_quiet_and_stable_then_emits_once() {
        let t0 = Instant::now();
        let p = ProjectRoot {
            path: "/x/a.als".into(),
            kind: ProjectKind::Ableton,
        };
        let mut d = Debouncer::new(Duration::from_millis(500));
        // A save that writes for 900 ms in 150 ms chunks.
        let mut size = 0;
        let mut out = Vec::new();
        for step in 0..=6 {
            size += 10;
            d.observe(p.clone(), ms(t0, step * 150));
            out.extend(d.poll(ms(t0, step * 150), |_| fp(size)));
        }
        assert!(out.is_empty(), "nothing settles mid-save");
        // Quiet from 900 ms; stable since the last size change at 900 ms.
        assert!(d.poll(ms(t0, 1_300), |_| fp(size)).is_empty());
        assert_eq!(d.poll(ms(t0, 1_400), |_| fp(size)), vec![p.clone()]);
        assert!(d.is_idle());
        // A late duplicate event with an identical fingerprint is swallowed.
        d.observe(p.clone(), ms(t0, 1_500));
        assert!(d.poll(ms(t0, 1_500), |_| fp(size)).is_empty());
        assert!(d.poll(ms(t0, 2_100), |_| fp(size)).is_empty());
        // The next real save emits again.
        d.observe(p.clone(), ms(t0, 3_000));
        assert!(d.poll(ms(t0, 3_000), |_| fp(size + 1)).is_empty());
        assert_eq!(d.poll(ms(t0, 3_600), |_| fp(size + 1)), vec![p]);
    }

    #[test]
    fn debouncer_waits_for_files_that_change_without_events() {
        let t0 = Instant::now();
        let p = ProjectRoot {
            path: "/x/a.als".into(),
            kind: ProjectKind::Ableton,
        };
        let mut d = Debouncer::new(Duration::from_millis(500));
        d.observe(p.clone(), t0);
        assert!(d.poll(t0, |_| fp(1)).is_empty());
        // No events, but the size keeps moving (coalesced/lost events).
        assert!(d.poll(ms(t0, 600), |_| fp(2)).is_empty());
        assert!(d.poll(ms(t0, 1_000), |_| fp(2)).is_empty());
        assert_eq!(d.poll(ms(t0, 1_100), |_| fp(2)), vec![p]);
    }

    #[test]
    fn an_event_that_changed_nothing_is_not_a_save() {
        // e.g. APFS reporting a clone's *source* as created + modified.
        let t0 = Instant::now();
        let p = ProjectRoot {
            path: "/x/a.logicx".into(),
            kind: ProjectKind::Logic,
        };
        let mut d = Debouncer::new(Duration::from_millis(500));
        d.set_baseline(p.path.clone(), fp(7).unwrap());
        d.observe(p.clone(), t0);
        assert!(d.poll(t0, |_| fp(7)).is_empty());
        assert!(d.poll(ms(t0, 600), |_| fp(7)).is_empty());
        assert!(d.is_idle(), "settled, and dropped as unchanged");
    }

    #[test]
    fn a_change_is_reported_whatever_the_age_of_its_files() {
        // An atomic replace by an older version: same mtime, different
        // size/inode/ctime — the "±30 s" rule used to drop this.
        let t0 = Instant::now();
        let p = ProjectRoot {
            path: "/x/a.als".into(),
            kind: ProjectKind::Ableton,
        };
        let old = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        let before = vec![(
            PathBuf::new(),
            FileStamp {
                len: 7,
                modified: Some(old),
                change: (10, 1),
            },
        )];
        let after = vec![(
            PathBuf::new(),
            FileStamp {
                len: 9,
                modified: Some(old),
                change: (11, 2),
            },
        )];
        let mut d = Debouncer::new(Duration::from_millis(500));
        d.set_baseline(p.path.clone(), before);
        d.observe(p.clone(), t0);
        assert!(d.poll(t0, |_| Some(after.clone())).is_empty());
        assert_eq!(
            d.poll(ms(t0, 600), |_| Some(after.clone())),
            vec![p.clone()]
        );
        // Same bytes, same size, same mtime, but rewritten in place (ctime moved).
        let rewritten = vec![(
            PathBuf::new(),
            FileStamp {
                len: 9,
                modified: Some(old),
                change: (11, 3),
            },
        )];
        d.observe(p.clone(), ms(t0, 1_000));
        assert!(d
            .poll(ms(t0, 1_000), |_| Some(rewritten.clone()))
            .is_empty());
        assert_eq!(d.poll(ms(t0, 1_600), |_| Some(rewritten.clone())), vec![p]);
    }

    #[test]
    fn a_project_without_a_baseline_is_reported() {
        let t0 = Instant::now();
        let p = ProjectRoot {
            path: "/x/new.als".into(),
            kind: ProjectKind::Ableton,
        };
        let mut d = Debouncer::new(Duration::from_millis(500));
        d.observe(p.clone(), t0);
        assert!(d.poll(t0, |_| fp(1)).is_empty());
        assert_eq!(d.poll(ms(t0, 600), |_| fp(1)), vec![p.clone()]);
        // Forgetting a baseline re-arms reporting even for no change.
        d.forget(&p.path);
        d.observe(p.clone(), ms(t0, 1_000));
        assert!(d.poll(ms(t0, 1_000), |_| fp(1)).is_empty());
        assert_eq!(d.poll(ms(t0, 1_600), |_| fp(1)), vec![p]);
    }

    #[test]
    fn baselines_are_bounded() {
        let mut d = Debouncer::new(Duration::from_millis(500));
        for i in 0..(MAX_BASELINES + 10) {
            d.set_baseline(PathBuf::from(format!("/x/{i:07}.als")), fp(1).unwrap());
        }
        assert_eq!(d.baselines.len(), MAX_BASELINES);
    }

    #[test]
    fn debouncer_drops_projects_that_vanish() {
        let t0 = Instant::now();
        let p = ProjectRoot {
            path: "/x/gone.als".into(),
            kind: ProjectKind::Ableton,
        };
        let mut d = Debouncer::new(Duration::from_millis(100));
        d.observe(p, t0);
        assert!(d.poll(t0, |_| None).is_empty());
        assert!(d.poll(ms(t0, 200), |_| None).is_empty());
        assert!(d.is_idle());
    }

    #[test]
    fn fingerprint_reads_only_the_whitelisted_package_files() {
        let dir = tempfile::tempdir().unwrap();
        let pkg = dir.path().join("T.logicx");
        let alt = pkg.join("Alternatives/000");
        std::fs::create_dir_all(alt.join("Project File Backups/00")).unwrap();
        std::fs::create_dir_all(pkg.join("Media/Audio Files")).unwrap();
        std::fs::create_dir_all(pkg.join("Resources")).unwrap();
        std::fs::write(alt.join("ProjectData"), b"pd").unwrap();
        std::fs::write(alt.join("DisplayState.plist"), b"ui").unwrap();
        std::fs::write(alt.join("Project File Backups/00/ProjectData"), b"old").unwrap();
        std::fs::write(pkg.join("Media/Audio Files/take.raw"), b"audio").unwrap();
        std::fs::write(pkg.join("Resources/ProjectInformation.plist"), b"info").unwrap();
        let project = ProjectRoot {
            path: pkg.clone(),
            kind: ProjectKind::Logic,
        };
        let got: Vec<PathBuf> = fingerprint(&project)
            .unwrap()
            .into_iter()
            .map(|(p, ..)| p)
            .collect();
        assert_eq!(
            got,
            vec![
                PathBuf::from("Alternatives/000/Project File Backups/00/ProjectData"),
                PathBuf::from("Alternatives/000/ProjectData"),
                PathBuf::from("Resources/ProjectInformation.plist"),
            ]
        );
        assert_eq!(
            fingerprint(&ProjectRoot {
                path: dir.path().join("none.als"),
                kind: ProjectKind::Ableton
            }),
            None
        );
    }

    #[test]
    fn config_watches_restores_and_refuses_roots_inside_it() {
        let dir = tempfile::tempdir().unwrap();
        let music = dir.path().join("Music");
        std::fs::create_dir_all(music.join("Logic")).unwrap();
        let mut roots = WatchedRoots::new();
        roots
            .add(&music.join("Logic"), RootKind::Discovery)
            .unwrap();
        let data = dir.path().join("WitData");
        let restores = RestoresDir::new(&music.join("Wit Restores"), &roots, &data).unwrap();
        // The Restores folder becomes a watched root of its own, and Wit's
        // data folder is ignored by construction ...
        let config = WatchConfig::new(roots.clone(), &restores).unwrap();
        assert!(config.roots.iter().any(|r| r.path() == restores.path()));
        assert_eq!(config.ignore, vec![restores.data_dir().to_path_buf()]);
        // ... a root around it (the user watching ~/Music) is fine ...
        let mut wider = roots.clone();
        wider.add(&music, RootKind::UserFolder).unwrap();
        assert!(WatchConfig::new(wider, &restores).is_ok());
        // ... a root inside it is not.
        let inner = restores.path().join("inner");
        std::fs::create_dir(&inner).unwrap();
        let mut inside = roots;
        inside.add(&inner, RootKind::UserFolder).unwrap();
        assert!(matches!(
            WatchConfig::new(inside, &restores),
            Err(WatchError::Restores(_))
        ));
    }

    #[test]
    fn a_restores_folder_that_is_a_discovery_root_keeps_its_kind() {
        let dir = tempfile::tempdir().unwrap();
        let logic = dir.path().join("Music/Logic");
        std::fs::create_dir_all(&logic).unwrap();
        let mut roots = WatchedRoots::new();
        roots.add(&logic, RootKind::Discovery).unwrap();
        let restores = RestoresDir::new(&logic, &roots, &dir.path().join("WitData")).unwrap();
        let config = WatchConfig::new(roots, &restores).unwrap();
        assert_eq!(config.roots.len(), 1);
        assert_eq!(
            config.roots.iter().next().unwrap().kind(),
            RootKind::Discovery,
            "not widened to the generic tier"
        );
    }
}
