//! Restore-as-copy **primitives** (ADR-0007). The per-DAW restore logic —
//! which bytes to put where for Logic, Ableton, FL — lives in the restore
//! lane on top of these; this module only guarantees *where* bytes can go.
//!
//! # The law
//!
//! **A restore never modifies, overwrites, renames or deletes anything that
//! existed before the restore began.** It only *creates* one brand-new entry
//! directly inside the Restores folder — `<Song> — <date>`, then
//! `<Song> — <date> (2)`, … — assembling it under a hidden staging name
//! inside the Restores folder and renaming it into place at the end. It never
//! writes into an existing entry, including earlier restores.
//!
//! The Restores folder **may** be inside a watched root (a user may watch
//! `~/Music`, which holds `~/Music/Wit Restores`), and Wit watches it like any
//! other root so the musician keeps history in restored copies. It may
//! **not** be inside a DAW project (a `.logicx`/`.band` package, an Ableton
//! project folder, or the folder an `.als`/`.flp` sits in), and may **not**
//! be a parent of any other watched root.
//!
//! # Why that holds by construction
//!
//! 1. **A Restores folder is a type, not a path.** [`RestoresDir::new`] is
//!    its only constructor. It resolves the folder the way the OS will
//!    (symlinks, `..`, trailing separators; Unicode-normalised, case-folded
//!    comparison keys — see [`crate::paths`]), refuses a placement that
//!    breaks the rules above, and creates the folder one component at a
//!    time, re-resolving and re-checking after each, so a symlink planted
//!    mid-way can't redirect it.
//! 2. **No write function takes a destination path.** New entries are
//!    named only by a [`RestoreDest`] — a fresh name minted by
//!    [`RestoresDir::fresh_destination`] from a sanitised song name (no
//!    separators, no `..`, no leading dot, no Windows device names), checked
//!    free with `symlink_metadata` (so an existing file, folder, symlink or
//!    dangling symlink all count as taken).
//! 3. **Only the entry the current restore created can be changed.**
//!    Multi-step restores (clone a package, swap in a stored `ProjectData`,
//!    clear the copy's `Project File Backups`) go through a [`NewRestore`]
//!    handle: it owns a staging folder *this call* created, and accepts only
//!    paths *relative* to it — refused if they have a root, a drive prefix,
//!    `.`/`..`, a `:`, a reserved name or a trailing dot/space. There is no
//!    way to point it at an existing restore or a project.
//! 4. **Nothing that existed is ever opened for writing.** New files are
//!    created with `create_new` (which refuses to follow or reuse an
//!    existing entry), and the final rename refuses to replace anything
//!    (`hard_link`-then-unlink for files; check-then-rename for folders,
//!    where `rename(2)` can at most replace an *empty* folder that appeared
//!    in between). Intermediate folders inside the staging folder are
//!    walked with `symlink_metadata` and must be real directories.
//! 5. **Every step re-checks.** Before touching the disk,
//!    [`RestoresDir::revalidate`] re-resolves the Restores folder (it must
//!    still resolve to exactly the validated path) and re-checks the
//!    placement rules against the filesystem as it is now.
//! 6. **Copies never contain symlinks.** A symlink inside a project could
//!    point back into the original; a DAW opening the copy would then write
//!    through it. Cloning skips symlinks (and sockets/FIFOs) and lists them
//!    in the [`CloneReport`].
//!
//! [`crate::watch::WatchConfig::new`] re-runs [`RestoresDir::check_against`]
//! on the exact root set it is about to watch, so a root added after the
//! `RestoresDir` was built can't sit inside it either.
//! `tests/restore_never_touches_existing.rs` checks the law with proptest: after
//! any sequence of restores over generated trees (symlinks — including ones
//! planted inside Restores pointing into projects — `..`, trailing
//! separators, NFC/NFD, case variants, Restores inside a watched root),
//! every pre-existing file and folder is byte-for-byte and
//! metadata-identical, and each successful restore added exactly one new
//! entry directly under Restores.
//!
//! What remains out of scope, honestly: another process with the user's
//! own permissions racing Wit inside the Restores folder between a check
//! and the syscall that follows it. Every check is repeated immediately
//! before the operation it guards, which narrows that window to one syscall.
//!
//! # Atomicity
//!
//! A copy is assembled under a hidden `.wit-staging-…` name **inside** the
//! Restores folder (same volume, so the final rename is atomic) and renamed
//! to its fresh name only when complete. A [`NewRestore`] dropped without
//! [`NewRestore::commit`] removes its own staging folder. A crash mid-restore
//! leaves a hidden `.wit-staging-…` entry behind; Wit never deletes it on a
//! later run (it existed before that run's restore began), and the watcher
//! ignores hidden names.
//!
//! # Copy strategy
//!
//! Each file is first cloned copy-on-write with the `reflink-copy` crate
//! (APFS `clonefile`, btrfs/XFS `FICLONE`, ReFS block cloning), which
//! costs no space and never modifies the source. On the first file that
//! can't be cloned (different volume, unsupported filesystem) the rest are
//! copied byte-for-byte — **after** checking the volume has room for the
//! remaining bytes plus a margin (5%, at least 64 MiB). Sources are opened
//! read-only. ReFS cloning is the crate's least-tested path upstream; on
//! failure it falls back to copying like everything else.
//!
//! What this module does *not* decide: which bytes are restored. ADR-0007
//! requires they be bytes a DAW itself wrote (a stored `ProjectData`, a
//! stored `.als`); the restore lane is responsible for that.

use crate::paths;
use crate::roots::WatchedRoots;
use std::ffi::OsString;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

/// Prefix of the hidden staging entries this module creates.
pub const STAGING_PREFIX: &str = ".wit-staging-";
const TEMP_PREFIX: &str = ".wit-tmp-";
/// How many ` (n)` suffixes to try before giving up on a name.
const MAX_NAME_ATTEMPTS: u32 = 999;
/// Deepest directory nesting `clone_tree` will follow.
const MAX_CLONE_DEPTH: usize = 64;
/// Longest sanitised song/date part, in characters (the whole name must fit
/// in 255 bytes on every filesystem Wit targets).
const MAX_NAME_CHARS: usize = 100;
const MIN_SPACE_MARGIN: u64 = 64 * 1024 * 1024;
/// The folder Live creates inside every project folder.
const ABLETON_PROJECT_MARKER: &str = "Ableton Project Info";

/// Everything that can go wrong. Every refusal is an error value, never a
/// panic and never a silent fallback to a different location.
#[derive(Debug)]
pub enum CloneError {
    /// The Restores folder would be inside a DAW project (a `.logicx`/`.band`
    /// package, an Ableton project folder, or the folder of an `.als`/`.flp`).
    InsideProject { restores: PathBuf, project: PathBuf },
    /// The Restores folder would be a parent of another watched root.
    ContainsWatchedRoot { restores: PathBuf, root: PathBuf },
    /// The Restores folder spelling can't be used (empty, `..` after a
    /// missing folder, not a directory, resolves somewhere unexpected).
    InvalidRestoresPath { path: PathBuf, reason: &'static str },
    /// The Restores folder no longer resolves to the path that was validated
    /// (moved, replaced by a symlink, deleted).
    RestoresMoved { expected: PathBuf },
    /// A relative path inside a new restore was refused.
    InvalidRelativePath { path: PathBuf, reason: &'static str },
    /// A name or extension was refused.
    InvalidName { name: String, reason: &'static str },
    /// Every candidate name up to ` (999)` is taken.
    NoFreeName { base: String },
    /// Not enough free space for a byte-for-byte copy.
    InsufficientSpace { needed: u64, available: u64 },
    /// A symlink (or special file) was found where a real file or directory
    /// was required.
    NotARealEntry { path: PathBuf },
    /// The source tree nests deeper than `clone_tree` follows.
    TooDeep { path: PathBuf },
    /// The clone source contains the Restores folder itself.
    SourceContainsRestores { source: PathBuf },
    Io {
        op: &'static str,
        path: PathBuf,
        source: io::Error,
    },
}

impl fmt::Display for CloneError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let d = |p: &Path| paths::display(p);
        match self {
            CloneError::InsideProject { restores, project } => write!(
                f,
                "the Restores folder {} is inside the project {} — pick a folder outside any project",
                d(restores),
                d(project)
            ),
            CloneError::ContainsWatchedRoot { restores, root } => write!(
                f,
                "the Restores folder {} contains the watched folder {} — pick a Restores folder that doesn't",
                d(restores),
                d(root)
            ),
            CloneError::InvalidRestoresPath { path, reason } => {
                write!(f, "can't use {} as the Restores folder: {reason}", d(path))
            }
            CloneError::RestoresMoved { expected } => write!(
                f,
                "the Restores folder {} moved or changed since it was checked; refusing to write",
                d(expected)
            ),
            CloneError::InvalidRelativePath { path, reason } => {
                write!(f, "refusing path {:?} inside a restore: {reason}", path)
            }
            CloneError::InvalidName { name, reason } => {
                write!(f, "refusing name {name:?}: {reason}")
            }
            CloneError::NoFreeName { base } => {
                write!(f, "no free name for {base:?} in the Restores folder")
            }
            CloneError::InsufficientSpace { needed, available } => write!(
                f,
                "not enough free space for the copy: needs {needed} bytes, {available} available"
            ),
            CloneError::NotARealEntry { path } => {
                write!(f, "{} is a symlink or special file; refusing", d(path))
            }
            CloneError::TooDeep { path } => {
                write!(f, "{} nests too deeply to copy", d(path))
            }
            CloneError::SourceContainsRestores { source } => write!(
                f,
                "{} contains the Restores folder; refusing to copy it into itself",
                d(source)
            ),
            CloneError::Io { op, path, source } => {
                write!(f, "{op} {}: {source}", d(path))
            }
        }
    }
}

impl std::error::Error for CloneError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            CloneError::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

fn io_err<'a>(op: &'static str, path: &'a Path) -> impl FnOnce(io::Error) -> CloneError + 'a {
    move |source| CloneError::Io {
        op,
        path: path.to_path_buf(),
        source,
    }
}

#[derive(Debug)]
struct Inner {
    /// Canonical (on Windows: verbatim) path of the Restores folder.
    dir: PathBuf,
    /// Canonical watched roots it was validated against.
    roots: Vec<PathBuf>,
}

/// The one folder restores are created in — proven, at construction and
/// again before every step, not to be inside a DAW project and not to
/// contain another watched root. See the [module docs](self) for the law.
///
/// Cheap to clone (it's an `Arc`).
#[derive(Debug, Clone)]
pub struct RestoresDir {
    inner: Arc<Inner>,
}

fn has_extension(path: &Path, exts: &[&str]) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| exts.iter().any(|x| e.eq_ignore_ascii_case(x)))
}

/// The DAW project `restores` would sit inside, if any:
/// - it or an ancestor is a `.logicx`/`.band` package;
/// - it or an ancestor is an Ableton project folder (has `Ableton Project Info`);
/// - its parent directly holds an `.als` or `.flp` file — the folder a
///   project file sits in is that project's folder.
///
/// Works on a path whose tail doesn't exist yet (those parts are checked by
/// name only).
fn enclosing_project(restores: &Path) -> Option<PathBuf> {
    for dir in restores.ancestors() {
        if has_extension(dir, &["logicx", "band"]) {
            return Some(dir.to_path_buf());
        }
        let marker = dir.join(ABLETON_PROJECT_MARKER);
        if fs::symlink_metadata(&marker).is_ok_and(|m| m.is_dir()) {
            return Some(dir.to_path_buf());
        }
    }
    let parent = restores.parent()?;
    let entries = fs::read_dir(parent).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        let is_file = entry.file_type().is_ok_and(|t| t.is_file());
        if is_file && has_extension(&path, &["als", "flp"]) {
            return Some(parent.to_path_buf());
        }
    }
    None
}

/// ADR-0007's placement rules for a canonical `restores` against canonical
/// `roots`: not inside a project, and not a parent of any *other* root. (It
/// may be inside a root, or be a root itself.) Case-folded on every OS, so
/// the "contains a root" check errs toward refusing.
fn check_placement(restores: &Path, roots: &[PathBuf]) -> Result<(), CloneError> {
    use paths::CaseSensitivity::Insensitive;
    for root in roots {
        let root_inside = paths::is_within_canonical(root, restores, Insensitive);
        let same = root_inside && paths::is_within_canonical(restores, root, Insensitive);
        if root_inside && !same {
            return Err(CloneError::ContainsWatchedRoot {
                restores: restores.to_path_buf(),
                root: root.clone(),
            });
        }
    }
    if let Some(project) = enclosing_project(restores) {
        return Err(CloneError::InsideProject {
            restores: restores.to_path_buf(),
            project,
        });
    }
    Ok(())
}

impl RestoresDir {
    /// Validate `path` against `watched`, create it if needed, and return
    /// the only handle Wit's restore functions accept.
    ///
    /// Refuses (creating nothing) a path that resolves inside a DAW project
    /// or above another watched root; a path whose missing tail contains
    /// `..`; and a path that exists but isn't a directory. Being inside a
    /// watched root, or being one, is fine.
    pub fn new(path: &Path, watched: &WatchedRoots) -> Result<RestoresDir, CloneError> {
        if path.as_os_str().is_empty() {
            return Err(CloneError::InvalidRestoresPath {
                path: path.to_path_buf(),
                reason: "empty path",
            });
        }
        let roots: Vec<PathBuf> = watched.iter().map(|r| r.path().to_path_buf()).collect();

        // 1. Check where it *would* be, before creating anything.
        let planned = paths::canonicalize_lenient(path).map_err(io_err("resolve", path))?;
        check_placement(&planned, &roots)?;

        // 2. Create the missing tail one component at a time, re-resolving
        //    and re-checking after each.
        let absolute = std::path::absolute(path).map_err(io_err("resolve", path))?;
        let components: Vec<Component<'_>> = absolute.components().collect();
        let mut split = components.len();
        let mut base = loop {
            if split == 0 {
                return Err(CloneError::InvalidRestoresPath {
                    path: path.to_path_buf(),
                    reason: "no part of the path exists",
                });
            }
            if !matches!(components[split - 1], Component::Prefix(_)) {
                let head: PathBuf = components[..split].iter().collect();
                if let Ok(c) = fs::canonicalize(&head) {
                    break c;
                }
            }
            split -= 1;
        };
        for component in &components[split..] {
            let Component::Normal(name) = component else {
                return Err(CloneError::InvalidRestoresPath {
                    path: path.to_path_buf(),
                    reason: "it has '..' or '.' after a folder that doesn't exist yet",
                });
            };
            let next = base.join(name);
            let created = match fs::create_dir(&next) {
                Ok(()) => true,
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => false,
                Err(e) => return Err(io_err("create folder", &next)(e)),
            };
            let resolved = fs::canonicalize(&next).map_err(io_err("resolve", &next))?;
            if let Err(e) = check_placement(&resolved, &roots) {
                if created {
                    // Only ever an empty folder this call just made.
                    let _ = fs::remove_dir(&next);
                }
                return Err(e);
            }
            base = resolved;
        }

        // 3. The authoritative check: the OS's own resolution of the
        //    caller's spelling must be exactly what we built and checked.
        let dir = fs::canonicalize(path).map_err(io_err("resolve", path))?;
        if !fs::metadata(&dir)
            .map_err(io_err("inspect", &dir))?
            .is_dir()
        {
            return Err(CloneError::InvalidRestoresPath {
                path: path.to_path_buf(),
                reason: "it isn't a folder",
            });
        }
        if dir != base {
            return Err(CloneError::InvalidRestoresPath {
                path: path.to_path_buf(),
                reason: "it resolves differently from one moment to the next",
            });
        }
        check_placement(&dir, &roots)?;
        Ok(RestoresDir {
            inner: Arc::new(Inner { dir, roots }),
        })
    }

    /// The canonical path (with `\\?\` on Windows — use for I/O).
    pub fn path(&self) -> &Path {
        &self.inner.dir
    }

    /// For the settings UI.
    pub fn display(&self) -> String {
        paths::display(&self.inner.dir)
    }

    /// Check this Restores folder against another root set — the watcher
    /// calls this with the exact set it is about to watch.
    pub fn check_against(&self, watched: &WatchedRoots) -> Result<(), CloneError> {
        let roots: Vec<PathBuf> = watched.iter().map(|r| r.path().to_path_buf()).collect();
        check_placement(&self.inner.dir, &roots)
    }

    /// Re-prove the placement against the filesystem as it is *now*: the
    /// folder must still resolve to exactly the validated path, still be a
    /// real directory, still not be inside a project, and still not contain
    /// another watched root (as recorded, and as those roots resolve today).
    /// Called before every step of every restore.
    pub fn revalidate(&self) -> Result<(), CloneError> {
        let dir = &self.inner.dir;
        let moved = || CloneError::RestoresMoved {
            expected: dir.clone(),
        };
        let now = fs::canonicalize(dir).map_err(|_| moved())?;
        let meta = fs::symlink_metadata(dir).map_err(|_| moved())?;
        if now != *dir || !meta.is_dir() {
            return Err(moved());
        }
        check_placement(dir, &self.inner.roots)?;
        let fresh: Vec<PathBuf> = self
            .inner
            .roots
            .iter()
            .filter_map(|r| fs::canonicalize(r).ok())
            .collect();
        check_placement(dir, &fresh)
    }

    /// Mint a fresh, unused destination `<Song> — <date>[.ext]`, or
    /// `<Song> — <date> (2)[.ext]` … if that is taken. Nothing is created
    /// yet; the name is re-checked (and bumped if needed) when the restore
    /// is committed, so two restores racing for a name both succeed.
    ///
    /// `song` and `date_label` are sanitised (separators, `..`, control
    /// characters, leading dots, Windows-reserved names and characters,
    /// trailing dots/spaces). `extension` must be 1–16 ASCII alphanumerics
    /// (`"logicx"`, `"als"`, `"flp"`), or `None` for a plain folder.
    pub fn fresh_destination(
        &self,
        song: &str,
        date_label: &str,
        extension: Option<&str>,
    ) -> Result<RestoreDest, CloneError> {
        self.revalidate()?;
        let song = sanitize_component(song, "Untitled");
        let date = sanitize_component(date_label, "");
        let base = if date.is_empty() {
            song
        } else {
            format!("{song} — {date}")
        };
        let extension = match extension {
            None => None,
            Some(ext) => {
                let ok = !ext.is_empty()
                    && ext.len() <= 16
                    && ext.bytes().all(|b| b.is_ascii_alphanumeric());
                if !ok {
                    return Err(CloneError::InvalidName {
                        name: ext.to_string(),
                        reason: "an extension must be 1-16 ASCII letters or digits",
                    });
                }
                Some(ext.to_string())
            }
        };
        let mut dest = RestoreDest {
            restores: self.clone(),
            base,
            extension,
            attempt: 1,
            name: String::new(),
        };
        dest.pick_free_name()?;
        Ok(dest)
    }

    /// Start a multi-step restore: mint a fresh destination and create its
    /// (empty) staging folder. Shorthand for
    /// [`fresh_destination`](Self::fresh_destination) + [`NewRestore::begin`].
    pub fn begin_restore(
        &self,
        song: &str,
        date_label: &str,
        extension: Option<&str>,
    ) -> Result<NewRestore, CloneError> {
        NewRestore::begin(self.fresh_destination(song, date_label, extension)?)
    }
}

/// Make `raw` safe as (part of) one file name on macOS, Windows and Linux.
fn sanitize_component(raw: &str, fallback: &str) -> String {
    let normalized = paths::nfc(raw);
    let mut out = String::with_capacity(normalized.len());
    for c in normalized.chars() {
        match c {
            '/' | '\\' | ':' | '|' => out.push('-'),
            '*' | '?' | '"' | '<' | '>' => out.push('_'),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    let trimmed = out
        .trim()
        .trim_start_matches(['.', ' '])
        .trim_end_matches(['.', ' ']);
    let mut name: String = trimmed.chars().take(MAX_NAME_CHARS).collect();
    name = name.trim_end_matches(['.', ' ']).to_string();
    if name.is_empty() {
        return fallback.to_string();
    }
    if paths::is_windows_reserved_name(&name) {
        name.insert(0, '_');
    }
    name
}

/// A fresh name in a Restores folder, not yet written. Single-use: every
/// write function consumes it.
#[derive(Debug)]
pub struct RestoreDest {
    restores: RestoresDir,
    base: String,
    extension: Option<String>,
    attempt: u32,
    name: String,
}

impl RestoreDest {
    /// The file name this restore will get (if still free at commit time).
    pub fn file_name(&self) -> &str {
        &self.name
    }

    /// The full path it will get (if still free at commit time).
    pub fn path(&self) -> PathBuf {
        self.restores.path().join(&self.name)
    }

    pub fn restores(&self) -> &RestoresDir {
        &self.restores
    }

    fn render(&self) -> String {
        let stem = if self.attempt <= 1 {
            self.base.clone()
        } else {
            format!("{} ({})", self.base, self.attempt)
        };
        match &self.extension {
            Some(ext) => format!("{stem}.{ext}"),
            None => stem,
        }
    }

    /// Advance to the first name (from the current attempt on) with nothing
    /// at it — checked with `symlink_metadata`, so a dangling symlink counts
    /// as taken.
    fn pick_free_name(&mut self) -> Result<(), CloneError> {
        while self.attempt <= MAX_NAME_ATTEMPTS {
            let name = self.render();
            let candidate = self.restores.path().join(&name);
            match fs::symlink_metadata(&candidate) {
                Err(e) if e.kind() == io::ErrorKind::NotFound => {
                    self.name = name;
                    return Ok(());
                }
                Err(e) => return Err(io_err("inspect", &candidate)(e)),
                Ok(_) => self.attempt += 1,
            }
        }
        Err(CloneError::NoFreeName {
            base: self.base.clone(),
        })
    }
}

fn unique_suffix() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!(
        "{}-{}-{}",
        std::process::id(),
        nanos,
        COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}

/// Rename `from` to `to` without replacing anything at `to`.
///
/// Files use `hard_link` + `remove_file` (atomic no-clobber on APFS, ext4,
/// btrfs, XFS, NTFS); where hard links aren't supported (FAT/exFAT, some
/// network shares) it falls back to check-then-rename. Directories use
/// check-then-rename: on Unix `rename(2)` can then only replace an *empty*
/// directory that appeared in between (no data lost) and fails on a
/// non-empty one; on Windows it fails if anything exists.
fn rename_no_clobber(from: &Path, to: &Path, is_dir: bool) -> io::Result<()> {
    let exists = |p: &Path| match fs::symlink_metadata(p) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    };
    if !is_dir {
        match fs::hard_link(from, to) {
            Ok(()) => return fs::remove_file(from),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => return Err(e),
            Err(_) => {} // unsupported here: fall through
        }
    }
    if exists(to)? {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "destination exists",
        ));
    }
    fs::rename(from, to)
}

fn is_name_taken(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::AlreadyExists | io::ErrorKind::DirectoryNotEmpty
    )
}

/// Move a finished staging entry to `dest`'s name, bumping ` (n)` if a
/// racing writer took the name first.
fn commit_entry(
    staging: &Path,
    mut dest: RestoreDest,
    is_dir: bool,
) -> Result<PathBuf, CloneError> {
    loop {
        dest.restores.revalidate()?;
        let target = dest.path();
        match rename_no_clobber(staging, &target, is_dir) {
            Ok(()) => {
                sync_dir(dest.restores.path());
                return Ok(target);
            }
            Err(e) if is_name_taken(&e) => {
                dest.attempt += 1;
                dest.pick_free_name()?;
            }
            Err(e) => return Err(io_err("move into place", &target)(e)),
        }
    }
}

/// Best-effort `fsync` of a directory so a completed rename survives a
/// crash (Unix only; Windows has no directory handles for this).
fn sync_dir(dir: &Path) {
    if cfg!(unix) {
        if let Ok(f) = File::open(dir) {
            let _ = f.sync_all();
        }
    }
}

/// Write `bytes` to a brand-new file at `path` (`create_new`: never
/// follows or reuses an existing entry), flushed to disk.
fn write_new_file(path: &Path, bytes: &[u8]) -> Result<(), CloneError> {
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(io_err("create", path))?;
    let result = f
        .write_all(bytes)
        .and_then(|()| f.sync_all())
        .map_err(io_err("write", path));
    if result.is_err() {
        drop(f);
        let _ = fs::remove_file(path);
    }
    result
}

/// What a clone did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CloneReport {
    /// Files cloned copy-on-write (no extra space used).
    pub reflinked_files: u64,
    /// Files copied byte-for-byte.
    pub copied_files: u64,
    pub copied_bytes: u64,
    /// Total size of the regular files in the source.
    pub total_bytes: u64,
    /// Symlinks in the source, relative to it, **not** reproduced.
    pub skipped_symlinks: Vec<PathBuf>,
    /// Sockets, FIFOs and devices, relative to the source, not reproduced.
    pub skipped_special: Vec<PathBuf>,
}

/// A finished restore.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Restored {
    /// Where it landed (canonical; `\\?\` on Windows).
    pub path: PathBuf,
    pub report: CloneReport,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CopyMode {
    TryReflink,
    Copy,
}

struct CloneCtx {
    mode: CopyMode,
    remaining: u64,
    space_checked: bool,
    space_probe: PathBuf,
    report: CloneReport,
}

/// Sum the sizes of the regular files under `src` (symlinks not followed).
fn tree_size(src: &Path, depth: usize) -> Result<u64, CloneError> {
    if depth > MAX_CLONE_DEPTH {
        return Err(CloneError::TooDeep {
            path: src.to_path_buf(),
        });
    }
    let meta = fs::symlink_metadata(src).map_err(io_err("inspect", src))?;
    if meta.is_file() {
        return Ok(meta.len());
    }
    if !meta.is_dir() {
        return Ok(0);
    }
    let mut total = 0u64;
    for entry in fs::read_dir(src).map_err(io_err("list", src))? {
        let entry = entry.map_err(io_err("list", src))?;
        total = total.saturating_add(tree_size(&entry.path(), depth + 1)?);
    }
    Ok(total)
}

fn ensure_space(ctx: &mut CloneCtx) -> Result<(), CloneError> {
    if ctx.space_checked {
        return Ok(());
    }
    let available = fs4::available_space(&ctx.space_probe)
        .map_err(io_err("check free space", &ctx.space_probe))?;
    let margin = (ctx.remaining / 20).max(MIN_SPACE_MARGIN);
    let needed = ctx.remaining.saturating_add(margin);
    if available < needed {
        return Err(CloneError::InsufficientSpace { needed, available });
    }
    ctx.space_checked = true;
    Ok(())
}

/// Copy one regular file: reflink first, byte copy (after the space check)
/// if that isn't possible. `dst` must not exist.
fn clone_file(src: &Path, dst: &Path, len: u64, ctx: &mut CloneCtx) -> Result<(), CloneError> {
    if ctx.mode == CopyMode::TryReflink {
        match reflink_copy::reflink(src, dst) {
            Ok(()) => {
                ctx.report.reflinked_files += 1;
                ctx.remaining = ctx.remaining.saturating_sub(len);
                return Ok(());
            }
            Err(_) => {
                // `dst` lives in our own staging folder; clear any partial
                // entry the failed clone left before copying instead.
                if fs::symlink_metadata(dst).is_ok() {
                    fs::remove_file(dst).map_err(io_err("remove", dst))?;
                }
                ctx.mode = CopyMode::Copy;
            }
        }
    }
    ensure_space(ctx)?;
    let mut input = File::open(src).map_err(io_err("open (read-only)", src))?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(dst)
        .map_err(io_err("create", dst))?;
    let copied = io::copy(&mut input, &mut output).map_err(io_err("copy", dst))?;
    if let Ok(modified) = input.metadata().and_then(|m| m.modified()) {
        let _ = output.set_modified(modified);
    }
    output.sync_all().map_err(io_err("flush", dst))?;
    ctx.report.copied_files += 1;
    ctx.report.copied_bytes += copied;
    ctx.remaining = ctx.remaining.saturating_sub(len);
    Ok(())
}

/// Recreate the contents of directory `src` inside the existing directory
/// `dst`, skipping symlinks and special files.
fn clone_dir_contents(
    src: &Path,
    dst: &Path,
    rel: &Path,
    depth: usize,
    ctx: &mut CloneCtx,
) -> Result<(), CloneError> {
    if depth > MAX_CLONE_DEPTH {
        return Err(CloneError::TooDeep {
            path: src.to_path_buf(),
        });
    }
    let mut entries: Vec<fs::DirEntry> = fs::read_dir(src)
        .map_err(io_err("list", src))?
        .collect::<Result<_, _>>()
        .map_err(io_err("list", src))?;
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let name = entry.file_name();
        let from = entry.path();
        let to = dst.join(&name);
        let rel = rel.join(&name);
        let meta = fs::symlink_metadata(&from).map_err(io_err("inspect", &from))?;
        let kind = meta.file_type();
        if kind.is_symlink() {
            ctx.report.skipped_symlinks.push(rel);
        } else if kind.is_dir() {
            fs::create_dir(&to).map_err(io_err("create folder", &to))?;
            clone_dir_contents(&from, &to, &rel, depth + 1, ctx)?;
        } else if kind.is_file() {
            clone_file(&from, &to, meta.len(), ctx)?;
        } else {
            ctx.report.skipped_special.push(rel);
        }
    }
    Ok(())
}

/// Resolve a clone source: follow a symlink *at* `src` (reading through it
/// is harmless), require a real file or directory, and refuse a source that
/// contains the Restores folder (the copy would recurse into itself).
fn resolve_source(
    src: &Path,
    restores: &RestoresDir,
) -> Result<(PathBuf, fs::Metadata), CloneError> {
    let resolved = fs::canonicalize(src).map_err(io_err("resolve", src))?;
    let meta = fs::symlink_metadata(&resolved).map_err(io_err("inspect", &resolved))?;
    if !(meta.is_dir() || meta.is_file()) {
        return Err(CloneError::NotARealEntry { path: resolved });
    }
    if paths::is_within_canonical(
        restores.path(),
        &resolved,
        paths::CaseSensitivity::Insensitive,
    ) {
        return Err(CloneError::SourceContainsRestores { source: resolved });
    }
    Ok((resolved, meta))
}

/// Copy `src` (a project package directory, or a single project file) into
/// the Restores folder under `dest`'s fresh name, copy-on-write where the
/// filesystem allows. The copy is built under a hidden staging name and
/// renamed into place only when complete; on any error nothing is left
/// behind. `src` is only ever read.
pub fn clone_tree(src: &Path, dest: RestoreDest) -> Result<Restored, CloneError> {
    let (source, meta) = resolve_source(src, &dest.restores)?;
    if meta.is_dir() {
        let mut staged = NewRestore::begin(dest)?;
        let report = staged.clone_tree_from(&source)?;
        let path = staged.commit()?;
        return Ok(Restored { path, report });
    }
    dest.restores.revalidate()?;
    let staging = dest
        .restores
        .path()
        .join(format!("{STAGING_PREFIX}{}", unique_suffix()));
    let mut ctx = CloneCtx {
        mode: CopyMode::TryReflink,
        remaining: meta.len(),
        space_checked: false,
        space_probe: dest.restores.path().to_path_buf(),
        report: CloneReport {
            total_bytes: meta.len(),
            ..CloneReport::default()
        },
    };
    let result = clone_file(&source, &staging, meta.len(), &mut ctx)
        .and_then(|()| commit_entry(&staging, dest, false));
    match result {
        Ok(path) => Ok(Restored {
            path,
            report: ctx.report,
        }),
        Err(e) => {
            let _ = fs::remove_file(&staging);
            Err(e)
        }
    }
}

/// Write `bytes` as a new file under `dest`'s fresh name (e.g. a stored
/// `.als` or `.flp`). Written to a hidden temp file, flushed, then renamed
/// into place without replacing anything.
pub fn write_file_in_restores(dest: RestoreDest, bytes: &[u8]) -> Result<PathBuf, CloneError> {
    dest.restores.revalidate()?;
    let staging = dest
        .restores
        .path()
        .join(format!("{STAGING_PREFIX}{}", unique_suffix()));
    write_new_file(&staging, bytes)?;
    commit_entry(&staging, dest, false).inspect_err(|_| {
        let _ = fs::remove_file(&staging);
    })
}

/// Clear the contents of `rel` inside the restore being built (e.g. the
/// copy's `Alternatives/000/Project File Backups`, so a restored project
/// doesn't carry the original's backup history). See
/// [`NewRestore::remove_dir_contents`]; this free function exists so the
/// only removal API is visibly scoped to the entry the current restore
/// created — it cannot name an existing restore or a project.
pub fn remove_dir_contents_in_restores(
    staged: &mut NewRestore,
    rel: &Path,
) -> Result<usize, CloneError> {
    staged.remove_dir_contents(rel)
}

/// Validate a path *relative to a new restore*: only plain names — no
/// root, drive prefix, `.`/`..`, `:`, separators inside a name, control
/// characters, Windows-reserved names or trailing dots/spaces.
fn validate_relative(rel: &Path, allow_empty: bool) -> Result<Vec<OsString>, CloneError> {
    let refuse = |reason: &'static str| CloneError::InvalidRelativePath {
        path: rel.to_path_buf(),
        reason,
    };
    let mut names = Vec::new();
    for component in rel.components() {
        let Component::Normal(name) = component else {
            return Err(refuse("only plain folder and file names are allowed"));
        };
        let Some(text) = name.to_str() else {
            return Err(refuse("names must be valid Unicode"));
        };
        if text.is_empty()
            || text.contains(['/', '\\', ':'])
            || text.chars().any(char::is_control)
            || text.ends_with(['.', ' '])
            || paths::is_windows_reserved_name(text)
        {
            return Err(refuse(
                "a name contains a character or form Wit won't write",
            ));
        }
        names.push(name.to_os_string());
    }
    if names.is_empty() && !allow_empty {
        return Err(refuse("empty path"));
    }
    Ok(names)
}

/// The one entry the current restore is creating — the **only** handle
/// through which anything can be written or removed. It owns a hidden
/// staging folder that [`NewRestore::begin`] just created inside the
/// Restores folder; every path it accepts is relative to that folder, so it
/// cannot reach an earlier restore, a project, or anything else that
/// existed before this restore began. Used for multi-step restores (clone a
/// package, swap in a stored `ProjectData`, clear its backups) that must
/// appear all at once or not at all.
///
/// [`commit`](NewRestore::commit) renames it to its fresh name; dropping it
/// without committing deletes the staging folder (and only that).
#[derive(Debug)]
pub struct NewRestore {
    dest: Option<RestoreDest>,
    staging: PathBuf,
}

impl NewRestore {
    /// Create an empty, brand-new staging folder for `dest`.
    pub fn begin(dest: RestoreDest) -> Result<NewRestore, CloneError> {
        dest.restores.revalidate()?;
        let staging = dest
            .restores
            .path()
            .join(format!("{STAGING_PREFIX}{}", unique_suffix()));
        fs::create_dir(&staging).map_err(io_err("create staging folder", &staging))?;
        Ok(NewRestore {
            dest: Some(dest),
            staging,
        })
    }

    /// The staging folder, for reading back what was staged.
    pub fn staging_path(&self) -> &Path {
        &self.staging
    }

    fn restores(&self) -> &RestoresDir {
        &self
            .dest
            .as_ref()
            .expect("a NewRestore holds its destination until commit")
            .restores
    }

    /// Check the staging folder is still a real directory directly inside
    /// the (revalidated) Restores folder.
    fn check_staging(&self) -> Result<(), CloneError> {
        self.restores().revalidate()?;
        let meta = fs::symlink_metadata(&self.staging).map_err(io_err("inspect", &self.staging))?;
        if !meta.is_dir() || self.staging.parent() != Some(self.restores().path()) {
            return Err(CloneError::NotARealEntry {
                path: self.staging.clone(),
            });
        }
        Ok(())
    }

    /// Walk (and with `create`, make) the directories `names` inside the
    /// staging folder, requiring each to be a real directory — never a
    /// symlink. Returns the final directory.
    fn walk_dirs(&self, names: &[OsString], create: bool) -> Result<Option<PathBuf>, CloneError> {
        let mut dir = self.staging.clone();
        for name in names {
            dir.push(name);
            match fs::symlink_metadata(&dir) {
                Ok(m) if m.is_dir() => {}
                Ok(_) => return Err(CloneError::NotARealEntry { path: dir }),
                Err(e) if e.kind() == io::ErrorKind::NotFound => {
                    if !create {
                        return Ok(None);
                    }
                    fs::create_dir(&dir).map_err(io_err("create folder", &dir))?;
                }
                Err(e) => return Err(io_err("inspect", &dir)(e)),
            }
        }
        Ok(Some(dir))
    }

    /// Clone the *contents* of the directory `src` into the staging folder
    /// (so the new restore becomes a copy of `src`). The staging folder
    /// must still be empty.
    pub fn clone_tree_from(&mut self, src: &Path) -> Result<CloneReport, CloneError> {
        self.check_staging()?;
        let (source, meta) = resolve_source(src, self.restores())?;
        if !meta.is_dir() {
            return Err(CloneError::InvalidRelativePath {
                path: source,
                reason: "a new restore clones a folder; use clone_tree for a single file",
            });
        }
        let not_empty = fs::read_dir(&self.staging)
            .map_err(io_err("list", &self.staging))?
            .next()
            .is_some();
        if not_empty {
            return Err(CloneError::InvalidRelativePath {
                path: self.staging.clone(),
                reason: "the new restore already has content",
            });
        }
        let total = tree_size(&source, 0)?;
        let mut ctx = CloneCtx {
            mode: CopyMode::TryReflink,
            remaining: total,
            space_checked: false,
            space_probe: self.staging.clone(),
            report: CloneReport {
                total_bytes: total,
                ..CloneReport::default()
            },
        };
        let staging = self.staging.clone();
        clone_dir_contents(&source, &staging, Path::new(""), 0, &mut ctx)?;
        Ok(ctx.report)
    }

    /// Write `bytes` at `rel` inside the new restore, creating parent
    /// folders as needed. **Replaces** a file already staged there (that's
    /// how a stored `ProjectData` is swapped into a cloned package) — by
    /// writing a new file and renaming it over the old entry, never by
    /// writing into the old file.
    pub fn write_file(&mut self, rel: &Path, bytes: &[u8]) -> Result<(), CloneError> {
        self.check_staging()?;
        let names = validate_relative(rel, false)?;
        let (file_name, parents) = names.split_last().expect("validated as non-empty");
        let parent = self
            .walk_dirs(parents, true)?
            .expect("walk_dirs with create always returns a folder");
        let target = parent.join(file_name);
        match fs::symlink_metadata(&target) {
            Ok(m) if m.is_dir() => {
                return Err(CloneError::NotARealEntry { path: target });
            }
            _ => {}
        }
        let temp = parent.join(format!("{TEMP_PREFIX}{}", unique_suffix()));
        write_new_file(&temp, bytes)?;
        fs::rename(&temp, &target).map_err(|e| {
            let _ = fs::remove_file(&temp);
            io_err("move into place", &target)(e)
        })
    }

    /// Remove everything inside the folder `rel` of the new restore
    /// (keeping the folder itself). A missing folder is not an error.
    /// Symlinks are removed as links, never followed. Returns how many
    /// entries were removed. An empty `rel` clears the whole new restore.
    pub fn remove_dir_contents(&mut self, rel: &Path) -> Result<usize, CloneError> {
        self.check_staging()?;
        let names = validate_relative(rel, true)?;
        let Some(dir) = self.walk_dirs(&names, false)? else {
            return Ok(0);
        };
        let mut removed = 0;
        for entry in fs::read_dir(&dir).map_err(io_err("list", &dir))? {
            let entry = entry.map_err(io_err("list", &dir))?;
            let path = entry.path();
            let kind = entry.file_type().map_err(io_err("inspect", &path))?;
            if kind.is_dir() {
                fs::remove_dir_all(&path).map_err(io_err("remove", &path))?;
            } else {
                fs::remove_file(&path).map_err(io_err("remove", &path))?;
            }
            removed += 1;
        }
        Ok(removed)
    }

    /// Move the finished copy to its fresh name (bumping ` (n)` if the name
    /// was taken meanwhile) and return where it landed.
    pub fn commit(mut self) -> Result<PathBuf, CloneError> {
        self.check_staging()?;
        let dest = self
            .dest
            .take()
            .expect("a NewRestore holds its destination until commit");
        let restores = dest.restores.clone();
        match commit_entry(&self.staging, dest, true) {
            Ok(path) => Ok(path),
            Err(e) => {
                // Put the destination back so Drop can clean up.
                self.dest = Some(RestoreDest {
                    restores,
                    base: String::new(),
                    extension: None,
                    attempt: 1,
                    name: String::new(),
                });
                Err(e)
            }
        }
    }
}

impl Drop for NewRestore {
    fn drop(&mut self) {
        // Committed: `dest` was taken and the staging folder was renamed away.
        let Some(dest) = &self.dest else { return };
        let ours = self.staging.parent() == Some(dest.restores.path())
            && self
                .staging
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(STAGING_PREFIX));
        if ours {
            if let Ok(meta) = fs::symlink_metadata(&self.staging) {
                if meta.is_dir() {
                    let _ = fs::remove_dir_all(&self.staging);
                }
            }
        }
    }
}

/// `YYYY-MM-DD` (UTC) for `t` — a default `date_label` for
/// [`RestoresDir::fresh_destination`] that needs no time-zone database.
/// Callers with the user's local date should pass that instead.
pub fn date_label_utc(t: SystemTime) -> String {
    let days = t
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() / 86_400)
        .unwrap_or(0) as i64;
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::roots::RootKind;
    use std::time::Duration;

    struct World {
        _dir: tempfile::TempDir,
        base: PathBuf,
        roots: WatchedRoots,
    }

    /// A temp dir with one watched root holding a fake package.
    fn world() -> World {
        let dir = tempfile::tempdir().unwrap();
        let base = fs::canonicalize(dir.path()).unwrap();
        let watched = base.join("Music/Logic");
        let package = watched.join("Song.logicx/Alternatives/000");
        fs::create_dir_all(package.join("Project File Backups/00")).unwrap();
        fs::write(package.join("ProjectData"), b"current").unwrap();
        fs::write(
            package.join("Project File Backups/00/ProjectData"),
            b"older",
        )
        .unwrap();
        let mut roots = WatchedRoots::new();
        roots.add(&watched, RootKind::Discovery).unwrap();
        World {
            _dir: dir,
            base,
            roots,
        }
    }

    fn snapshot(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
        let mut out = Vec::new();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(d) = stack.pop() {
            for e in fs::read_dir(&d).unwrap() {
                let p = e.unwrap().path();
                if p.is_dir() {
                    stack.push(p);
                } else {
                    out.push((p.clone(), fs::read(&p).unwrap()));
                }
            }
        }
        out.sort();
        out
    }

    #[test]
    fn restores_may_be_inside_or_equal_to_a_watched_root() {
        let w = world();
        let inside = w.base.join("Music/Logic/Wit Restores");
        RestoresDir::new(&inside, &w.roots).unwrap();
        assert!(inside.is_dir());
        RestoresDir::new(&w.base.join("Music/Logic"), &w.roots).unwrap();
    }

    #[test]
    fn refuses_restores_above_another_watched_root() {
        let w = world();
        assert!(matches!(
            RestoresDir::new(&w.base.join("Music"), &w.roots),
            Err(CloneError::ContainsWatchedRoot { .. })
        ));
    }

    #[test]
    fn refuses_restores_inside_a_project_and_creates_nothing() {
        let w = world();
        let in_package = w.base.join("Music/Logic/Song.logicx/Wit Restores");
        assert!(matches!(
            RestoresDir::new(&in_package, &w.roots),
            Err(CloneError::InsideProject { .. })
        ));
        assert!(
            !in_package.exists(),
            "a refused Restores folder is never created"
        );
        // Spelled with `..`, a trailing separator, and a case-variant extension.
        let sneaky = w.base.join("Other/../MUSIC/logic/Song.LOGICX/x/");
        assert!(RestoresDir::new(&sneaky, &w.roots).is_err());
        assert!(!w.base.join("Other").exists());

        // An Ableton project folder (Live's marker folder inside it).
        let live = w.base.join("Live/Set Project");
        fs::create_dir_all(live.join("Ableton Project Info")).unwrap();
        assert!(matches!(
            RestoresDir::new(&live.join("Restores"), &w.roots),
            Err(CloneError::InsideProject { .. })
        ));
        // Next to an FL Studio project file.
        let fl = w.base.join("FL");
        fs::create_dir_all(&fl).unwrap();
        fs::write(fl.join("Beat.flp"), b"flp").unwrap();
        assert!(matches!(
            RestoresDir::new(&fl.join("Restores"), &w.roots),
            Err(CloneError::InsideProject { .. })
        ));
        assert!(!fl.join("Restores").exists());
    }

    #[test]
    fn a_restore_inside_the_watched_root_leaves_every_existing_file_alone() {
        let w = world();
        let restores =
            RestoresDir::new(&w.base.join("Music/Logic/Wit Restores"), &w.roots).unwrap();
        let before = snapshot(&w.base.join("Music"));
        let dest = restores
            .fresh_destination("Song", "d", Some("logicx"))
            .unwrap();
        let restored = clone_tree(&w.base.join("Music/Logic/Song.logicx"), dest).unwrap();
        let after = snapshot(&w.base.join("Music"));
        let new: Vec<&PathBuf> = after
            .iter()
            .filter(|e| !before.contains(e))
            .map(|(p, _)| p)
            .collect();
        assert!(
            before.iter().all(|e| after.contains(e)),
            "an existing file changed"
        );
        assert!(new.iter().all(|p| p.starts_with(&restored.path)), "{new:?}");
    }

    #[test]
    fn creates_a_valid_restores_folder_and_its_parents() {
        let w = world();
        let path = w.base.join("Music/Wit Restores/nested");
        let restores = RestoresDir::new(&path, &w.roots).unwrap();
        assert!(path.is_dir());
        assert_eq!(restores.path(), fs::canonicalize(&path).unwrap());
        restores.revalidate().unwrap();
    }

    // POSIX only: on Windows, `..` is resolved lexically before a path ever
    // reaches Wit (`PathBuf::join` onto a verbatim path and `GetFullPathName`
    // both collapse it), so there is no `..` left to refuse.
    #[cfg(unix)]
    #[test]
    fn refuses_dot_dot_after_a_missing_folder() {
        let w = world();
        let path = w.base.join("missing/../Restores");
        let err = RestoresDir::new(&path, &w.roots);
        // Lexically this is fine, but the OS can't create it that way.
        assert!(err.is_err());
    }

    #[test]
    fn clone_tree_copies_a_package_and_never_touches_the_source() {
        let w = world();
        let restores = RestoresDir::new(&w.base.join("Restores"), &w.roots).unwrap();
        let src = w.base.join("Music/Logic/Song.logicx");
        let before = snapshot(&w.base.join("Music"));
        let dest = restores
            .fresh_destination("Song", "2026-09-29", Some("logicx"))
            .unwrap();
        assert_eq!(dest.file_name(), "Song — 2026-09-29.logicx");
        let restored = clone_tree(&src, dest).unwrap();
        assert_eq!(snapshot(&w.base.join("Music")), before);
        assert_eq!(
            fs::read(restored.path.join("Alternatives/000/ProjectData")).unwrap(),
            b"current"
        );
        assert_eq!(restored.report.total_bytes, 12);
        assert_eq!(
            restored.report.reflinked_files + restored.report.copied_files,
            2
        );
        // No staging leftovers.
        let names: Vec<String> = fs::read_dir(restores.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["Song — 2026-09-29.logicx".to_string()]);
    }

    #[test]
    fn a_second_restore_gets_a_fresh_name_and_never_overwrites() {
        let w = world();
        let restores = RestoresDir::new(&w.base.join("Restores"), &w.roots).unwrap();
        let a = write_file_in_restores(
            restores
                .fresh_destination("Song", "d", Some("als"))
                .unwrap(),
            b"first",
        )
        .unwrap();
        let b = write_file_in_restores(
            restores
                .fresh_destination("Song", "d", Some("als"))
                .unwrap(),
            b"second",
        )
        .unwrap();
        assert_ne!(a, b);
        assert!(b.ends_with("Song — d (2).als"));
        assert_eq!(fs::read(&a).unwrap(), b"first");
        assert_eq!(fs::read(&b).unwrap(), b"second");
    }

    #[test]
    fn a_name_taken_after_minting_is_bumped_at_commit() {
        let w = world();
        let restores = RestoresDir::new(&w.base.join("Restores"), &w.roots).unwrap();
        let dest = restores
            .fresh_destination("Song", "d", Some("als"))
            .unwrap();
        // Someone else takes the name first.
        fs::write(dest.path(), b"theirs").unwrap();
        let ours = write_file_in_restores(dest, b"ours").unwrap();
        assert!(ours.ends_with("Song — d (2).als"));
        assert_eq!(
            fs::read(restores.path().join("Song — d.als")).unwrap(),
            b"theirs"
        );
    }

    #[test]
    fn staged_restore_swaps_a_file_and_clears_backups_atomically() {
        let w = world();
        let restores = RestoresDir::new(&w.base.join("Restores"), &w.roots).unwrap();
        let src = w.base.join("Music/Logic/Song.logicx");
        let before = snapshot(&w.base.join("Music"));
        let dest = restores
            .fresh_destination("Song", "old", Some("logicx"))
            .unwrap();
        let final_path = dest.path();
        let mut staged = NewRestore::begin(dest).unwrap();
        staged.clone_tree_from(&src).unwrap();
        assert!(!final_path.exists(), "nothing appears before commit");
        staged
            .write_file(Path::new("Alternatives/000/ProjectData"), b"older")
            .unwrap();
        let removed = remove_dir_contents_in_restores(
            &mut staged,
            Path::new("Alternatives/000/Project File Backups"),
        )
        .unwrap();
        assert_eq!(removed, 1);
        let landed = staged.commit().unwrap();
        assert_eq!(landed, final_path);
        assert_eq!(
            fs::read(landed.join("Alternatives/000/ProjectData")).unwrap(),
            b"older"
        );
        assert!(
            fs::read_dir(landed.join("Alternatives/000/Project File Backups"))
                .unwrap()
                .next()
                .is_none()
        );
        assert_eq!(
            snapshot(&w.base.join("Music")),
            before,
            "the source is untouched"
        );
    }

    #[test]
    fn dropping_a_staged_restore_leaves_nothing_behind() {
        let w = world();
        let restores = RestoresDir::new(&w.base.join("Restores"), &w.roots).unwrap();
        let dest = restores.fresh_destination("Song", "x", None).unwrap();
        let mut staged = NewRestore::begin(dest).unwrap();
        staged.write_file(Path::new("a/b"), b"x").unwrap();
        drop(staged);
        assert_eq!(fs::read_dir(restores.path()).unwrap().count(), 0);
    }

    #[test]
    fn staged_paths_refuse_escapes() {
        let w = world();
        let restores = RestoresDir::new(&w.base.join("Restores"), &w.roots).unwrap();
        let dest = restores.fresh_destination("Song", "x", None).unwrap();
        let mut staged = NewRestore::begin(dest).unwrap();
        let absolute = w
            .base
            .join("Music/Logic/Song.logicx/Alternatives/000/ProjectData");
        for bad in [
            Path::new("../escape"),
            Path::new("a/../../escape"),
            Path::new("./a"),
            absolute.as_path(),
            Path::new("a:stream"),
            Path::new("CON"),
            Path::new("trailing."),
            Path::new(""),
        ] {
            assert!(
                staged.write_file(bad, b"x").is_err(),
                "should refuse {}",
                bad.display()
            );
        }
        assert!(staged.remove_dir_contents(Path::new("..")).is_err());
        assert_eq!(
            fs::read(&absolute).unwrap(),
            b"current",
            "the watched project is untouched"
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_in_the_source_are_skipped_not_reproduced() {
        let w = world();
        let restores = RestoresDir::new(&w.base.join("Restores"), &w.roots).unwrap();
        let src = w.base.join("Music/Logic/Song.logicx");
        std::os::unix::fs::symlink(w.base.join("Music/Logic"), src.join("Media")).unwrap();
        let dest = restores
            .fresh_destination("Song", "x", Some("logicx"))
            .unwrap();
        let restored = clone_tree(&src, dest).unwrap();
        assert_eq!(
            restored.report.skipped_symlinks,
            vec![PathBuf::from("Media")]
        );
        assert!(fs::symlink_metadata(restored.path.join("Media")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_planted_inside_a_staged_restore_is_not_followed() {
        let w = world();
        let restores = RestoresDir::new(&w.base.join("Restores"), &w.roots).unwrap();
        let dest = restores.fresh_destination("Song", "x", None).unwrap();
        let mut staged = NewRestore::begin(dest).unwrap();
        let project = w.base.join("Music/Logic/Song.logicx/Alternatives/000");
        std::os::unix::fs::symlink(&project, staged.staging_path().join("evil")).unwrap();
        assert!(matches!(
            staged.write_file(Path::new("evil/ProjectData"), b"pwned"),
            Err(CloneError::NotARealEntry { .. })
        ));
        assert!(staged.remove_dir_contents(Path::new("evil")).is_err());
        assert_eq!(fs::read(project.join("ProjectData")).unwrap(), b"current");
    }

    #[cfg(unix)]
    #[test]
    fn a_restores_folder_swapped_for_a_symlink_is_caught_before_writing() {
        let w = world();
        let path = w.base.join("Restores");
        let restores = RestoresDir::new(&path, &w.roots).unwrap();
        fs::remove_dir(&path).unwrap();
        std::os::unix::fs::symlink(w.base.join("Music/Logic"), &path).unwrap();
        assert!(matches!(
            restores.fresh_destination("Song", "x", Some("als")),
            Err(CloneError::RestoresMoved { .. })
        ));
    }

    #[test]
    fn names_are_sanitised() {
        assert_eq!(
            sanitize_component("../../etc/passwd", "U"),
            "-..-etc-passwd"
        );
        assert_eq!(sanitize_component("  .hidden  ", "U"), "hidden");
        assert_eq!(
            sanitize_component("a/b\\c:d*e?f\"g<h>i|j", "U"),
            "a-b-c-d_e_f_g_h_i-j"
        );
        assert_eq!(sanitize_component("CON", "U"), "_CON");
        assert_eq!(sanitize_component("song.", "U"), "song");
        assert_eq!(sanitize_component("\u{0}\n", "U"), "U");
        assert_eq!(sanitize_component("Cafe\u{301}", "U"), "Caf\u{e9}");
        assert_eq!(
            sanitize_component(&"x".repeat(500), "U").chars().count(),
            MAX_NAME_CHARS
        );
    }

    #[test]
    fn bad_extensions_are_refused() {
        let w = world();
        let restores = RestoresDir::new(&w.base.join("Restores"), &w.roots).unwrap();
        for ext in ["", "a/b", "../x", "logicx ", &"a".repeat(17)] {
            assert!(
                restores.fresh_destination("S", "d", Some(ext)).is_err(),
                "{ext}"
            );
        }
    }

    #[test]
    fn leftovers_from_an_earlier_crash_are_never_touched() {
        let w = world();
        let restores = RestoresDir::new(&w.base.join("Restores"), &w.roots).unwrap();
        let leftover = restores.path().join(format!("{STAGING_PREFIX}crashed"));
        fs::create_dir(&leftover).unwrap();
        fs::write(leftover.join("half.bin"), b"half").unwrap();
        let earlier = restores.path().join("Song — d.als");
        fs::write(&earlier, b"an earlier restore the user kept working on").unwrap();
        let landed = write_file_in_restores(
            restores
                .fresh_destination("Song", "d", Some("als"))
                .unwrap(),
            b"new",
        )
        .unwrap();
        assert!(landed.ends_with("Song — d (2).als"));
        assert_eq!(fs::read(leftover.join("half.bin")).unwrap(), b"half");
        assert_eq!(
            fs::read(&earlier).unwrap(),
            b"an earlier restore the user kept working on"
        );
    }

    #[test]
    fn check_against_catches_a_root_added_later() {
        let w = world();
        let restores = RestoresDir::new(&w.base.join("Restores"), &w.roots).unwrap();
        // A root *around* the Restores folder is fine ...
        let mut around = w.roots.clone();
        around.add(&w.base, RootKind::UserFolder).unwrap();
        restores.check_against(&around).unwrap();
        // ... a root *inside* it is not.
        let inner = restores.path().join("inner");
        fs::create_dir(&inner).unwrap();
        let mut inside = w.roots.clone();
        inside.add(&inner, RootKind::UserFolder).unwrap();
        assert!(matches!(
            restores.check_against(&inside),
            Err(CloneError::ContainsWatchedRoot { .. })
        ));
    }

    #[test]
    fn a_source_containing_the_restores_folder_is_refused() {
        let w = world();
        let restores = RestoresDir::new(&w.base.join("Other/Restores"), &w.roots).unwrap();
        let dest = restores.fresh_destination("All", "x", None).unwrap();
        assert!(matches!(
            clone_tree(&w.base.join("Other"), dest),
            Err(CloneError::SourceContainsRestores { .. })
        ));
        assert_eq!(fs::read_dir(restores.path()).unwrap().count(), 0);
    }

    #[test]
    fn utc_date_labels() {
        assert_eq!(date_label_utc(UNIX_EPOCH), "1970-01-01");
        assert_eq!(
            date_label_utc(UNIX_EPOCH + Duration::from_secs(1_790_640_000)),
            "2026-09-29"
        );
        assert_eq!(
            date_label_utc(UNIX_EPOCH + Duration::from_secs(951_782_400)),
            "2000-02-29"
        );
    }
}
