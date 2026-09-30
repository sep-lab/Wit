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
//! # Where the Restores folder may be
//!
//! It **may** be inside a watched root, or be one (a user may watch
//! `~/Music`, which holds `~/Music/Wit Restores`); Wit watches it like any
//! other root. It may **not** be inside a DAW project, and may **not** be a
//! parent of any other watched root. A folder is a *project folder* when:
//!
//! | DAW | Project folder |
//! |---|---|
//! | Logic Pro / GarageBand | a `.logicx` / `.band` package (the package itself) |
//! | Ableton Live | a folder containing an `Ableton Project Info` folder (Live creates one in every project folder) |
//! | FL Studio | a folder `X` containing `X.flp` (a project saved in its own folder), or containing a `Backup/` folder that holds `.flp` autosaves |
//!
//! The Restores folder is refused if it, or **any** ancestor, is a project
//! folder. A loose `.als`/`.flp` merely sitting in an ancestor does not make
//! that ancestor a project (so a stray set in `~/Music` doesn't break the
//! default `~/Music/Wit Restores`). Checks **fail closed**: if an ancestor
//! can't be inspected (permission denied), the folder is refused.
//!
//! "Contains a watched root" is decided twice, and either refuses: by
//! Unicode-normalised, case-folded **spelling**, and by **file identity**
//! (`dev`+`inode` / volume+file index) along the root's ancestors — so an
//! alias that spelling can't see (a macOS firmlink such as
//! `/System/Volumes/Data/…`, a bind mount) is still caught.
//!
//! # Why the law holds by construction
//!
//! 1. **A Restores folder is a type, not a path.** [`RestoresDir::new`] is
//!    its only constructor. It refuses bad placements before creating
//!    anything, creates missing folders one at a time through directory
//!    handles, re-checking each, and records the folder's file identity.
//! 2. **Every step works through a pinned handle.** Each restore opens the
//!    Restores folder as a directory handle (`O_DIRECTORY|O_NOFOLLOW` on
//!    Unix; on Windows without `FILE_SHARE_DELETE`, so it can't be renamed
//!    or deleted while held), checks its identity is the one validated, and
//!    then does **everything** relative to that handle: `mkdirat`,
//!    `openat(O_CREAT|O_EXCL|O_NOFOLLOW)`, `fclonefileat`/`FICLONE`,
//!    `renameatx_np(RENAME_EXCL)`/`renameat2(RENAME_NOREPLACE)`, `unlinkat`
//!    (see `fsops`). Swapping the folder's path for a symlink mid-restore
//!    can't redirect a write: the handle still names the original folder.
//!    Before each step the path is re-checked to still name that folder
//!    (so a folder *moved* into a project is refused from the next step on).
//! 3. **No write function takes a destination path.** New entries are named
//!    by a [`RestoreDest`] minted from a sanitised song name, checked free
//!    through the handle (anything at the name — file, folder, symlink,
//!    dangling symlink — counts as taken). Follow-up steps go through a
//!    [`NewRestore`], which accepts only plain relative names inside the
//!    staging folder *it* created and opens each level with `O_NOFOLLOW`.
//! 4. **Nothing that existed is opened for writing or replaced.** Files are
//!    created with `O_EXCL`; the final move refuses to replace anything.
//!    Copies never reproduce symlinks, and never share an inode with the
//!    source (clones are copy-on-write; a source carrying a Finder lock
//!    `uchg`/`uappnd` is byte-copied instead, so the copy is never locked).
//! 5. **Only what this run created is ever removed** — a failed step
//!    removes its own new entry (verified by identity), never anything that
//!    was already there. Leftovers from a *crash* are journaled in Wit's own
//!    data folder ([`RestoresDir::staging_leftovers`]) and removed only on an
//!    explicit call, only if they still match the journal.
//!
//! `tests/restore_never_touches_existing.rs` checks the law with proptest:
//! after any sequence of restores over generated trees (symlinks — including
//! ones planted inside Restores and inside a restore's staging folder —
//! `..`, trailing separators, NFC/NFD, case variants, Restores inside a
//! watched root, the Restores folder moved into a project, and its path
//! flipped to a symlink mid-restore), every pre-existing file and folder is
//! byte-for-byte and metadata-identical (incl. inode, mode, flags), new
//! entries contain no symlinks and share no inode with anything that
//! existed, and each successful restore added exactly one new entry
//! directly under Restores.
//!
//! What remains, honestly: the Restores folder *itself* being moved (by
//! another process of the same user) between the pre-step check and the
//! syscall that follows. Wit's writes then land in that moved folder —
//! still only new entries, never touching anything that existed.
//!
//! # Copy strategy
//!
//! Each file is first cloned copy-on-write (APFS `fclonefileat`, btrfs/XFS
//! `FICLONE`, ReFS via `reflink-copy`), which costs no space and never
//! modifies the source (measured on APFS: the source's size, inode, mtime,
//! ctime and flags are unchanged). On the first file that can't be cloned
//! the rest are copied byte-for-byte — **after** checking the volume has
//! room for the remaining bytes plus a margin (5%, at least 64 MiB).
//! Sources are opened read-only.
//!
//! What this module does *not* decide: which bytes are restored. ADR-0007
//! requires they be bytes a DAW itself wrote; the restore lane owns that.

mod fsops;
mod journal;
mod names;

pub use fsops::FileId;

use crate::paths;
use crate::roots::WatchedRoots;
use fsops::{Cloned, Dir, Renamed};
use journal::Journal;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::fs::{self, File};
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
const MIN_SPACE_MARGIN: u64 = 64 * 1024 * 1024;
/// The folder Live creates inside every project folder.
const ABLETON_PROJECT_MARKER: &str = "Ableton Project Info";

/// Everything that can go wrong. Every refusal is an error value, never a
/// panic and never a silent fallback to a different location.
#[derive(Debug)]
pub enum CloneError {
    /// The Restores folder would be inside (or be) a DAW project folder.
    InsideProject {
        restores: PathBuf,
        project: PathBuf,
        kind: &'static str,
    },
    /// The Restores folder would be a parent of another watched root.
    ContainsWatchedRoot { restores: PathBuf, root: PathBuf },
    /// A placement check couldn't read what it needed to (fail closed).
    CannotVerifyPlacement { path: PathBuf, source: io::Error },
    /// The Restores folder spelling can't be used.
    InvalidRestoresPath { path: PathBuf, reason: &'static str },
    /// Wit's data folder can't be used for the staging journal.
    InvalidDataDir { path: PathBuf, reason: &'static str },
    /// The Restores folder no longer is the folder that was validated
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
    /// Not a journaled, still-matching staging leftover; refusing to remove.
    NotAStagingLeftover { name: String },
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
            CloneError::InsideProject { restores, project, kind } => write!(
                f,
                "the Restores folder {} is inside {} ({kind}) — pick a folder outside any project",
                d(restores),
                d(project)
            ),
            CloneError::ContainsWatchedRoot { restores, root } => write!(
                f,
                "the Restores folder {} contains the watched folder {} — pick a Restores folder that doesn't",
                d(restores),
                d(root)
            ),
            CloneError::CannotVerifyPlacement { path, source } => write!(
                f,
                "can't check {} ({source}); refusing rather than guessing",
                d(path)
            ),
            CloneError::InvalidRestoresPath { path, reason } => {
                write!(f, "can't use {} as the Restores folder: {reason}", d(path))
            }
            CloneError::InvalidDataDir { path, reason } => {
                write!(f, "can't use {} as Wit's data folder: {reason}", d(path))
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
            CloneError::NotAStagingLeftover { name } => write!(
                f,
                "{name:?} is not a journaled, unchanged staging leftover; refusing to remove it"
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
            CloneError::Io { source, .. } | CloneError::CannotVerifyPlacement { source, .. } => {
                Some(source)
            }
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

// ---------------------------------------------------------------------------
// Placement rules
// ---------------------------------------------------------------------------

fn has_extension(path: &Path, exts: &[&str]) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| exts.iter().any(|x| e.eq_ignore_ascii_case(x)))
}

/// Inspect `p` without following it. `None` when it doesn't exist; an error
/// (fail closed) when it can't be inspected.
fn probe(p: &Path) -> Result<Option<fs::Metadata>, CloneError> {
    match fs::symlink_metadata(p) {
        Ok(m) => Ok(Some(m)),
        Err(e)
            if matches!(
                e.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
            ) =>
        {
            Ok(None)
        }
        Err(source) => Err(CloneError::CannotVerifyPlacement {
            path: p.to_path_buf(),
            source,
        }),
    }
}

/// What DAW project folder `dir` is, if any (see the module docs' table).
fn project_folder(dir: &Path) -> Result<Option<&'static str>, CloneError> {
    if has_extension(dir, &["logicx", "band"]) {
        return Ok(Some("a Logic/GarageBand package"));
    }
    if probe(&dir.join(ABLETON_PROJECT_MARKER))?.is_some_and(|m| m.is_dir()) {
        return Ok(Some("an Ableton project folder"));
    }
    if let Some(name) = dir.file_name() {
        let mut flp = name.to_os_string();
        flp.push(".flp");
        if probe(&dir.join(flp))?.is_some_and(|m| m.is_file()) {
            return Ok(Some("an FL Studio project folder"));
        }
    }
    let backup = dir.join("Backup");
    if probe(&backup)?.is_some_and(|m| m.is_dir()) {
        let entries =
            fs::read_dir(&backup).map_err(|source| CloneError::CannotVerifyPlacement {
                path: backup.clone(),
                source,
            })?;
        for entry in entries {
            let entry = entry.map_err(|source| CloneError::CannotVerifyPlacement {
                path: backup.clone(),
                source,
            })?;
            if entry.file_type().is_ok_and(|t| t.is_file())
                && has_extension(&entry.path(), &["flp"])
            {
                return Ok(Some("an FL Studio project folder (it holds FL autosaves)"));
            }
        }
    }
    Ok(None)
}

/// The project folder `restores` would sit inside (or be), if any.
fn enclosing_project(restores: &Path) -> Result<Option<(PathBuf, &'static str)>, CloneError> {
    for dir in restores.ancestors().filter(|d| !d.as_os_str().is_empty()) {
        if let Some(kind) = project_folder(dir)? {
            return Ok(Some((dir.to_path_buf(), kind)));
        }
    }
    Ok(None)
}

/// ADR-0007's placement rules for `restores` (canonical) against canonical
/// `roots`. `id` is the Restores folder's identity once it exists.
fn check_placement(
    restores: &Path,
    id: Option<FileId>,
    roots: &[PathBuf],
) -> Result<(), CloneError> {
    use paths::CaseSensitivity::Insensitive;
    for root in roots {
        let contains = || CloneError::ContainsWatchedRoot {
            restores: restores.to_path_buf(),
            root: root.clone(),
        };
        // By spelling (case-folded, NFC): catches case/normal-form variants.
        let root_inside = paths::is_within_canonical(root, restores, Insensitive);
        let same_spelling = root_inside && paths::is_within_canonical(restores, root, Insensitive);
        if root_inside && !same_spelling {
            return Err(contains());
        }
        // By identity: catches aliases spelling can't see (firmlinks, bind mounts).
        if let Some(id) = id {
            let root_is_restores = FileId::of_path(root).is_ok_and(|r| r == id);
            let under = root
                .ancestors()
                .skip(1)
                .filter(|a| !a.as_os_str().is_empty())
                .any(|a| FileId::of_path(a).is_ok_and(|x| x == id));
            if under && !root_is_restores {
                return Err(contains());
            }
        }
    }
    if let Some((project, kind)) = enclosing_project(restores)? {
        return Err(CloneError::InsideProject {
            restores: restores.to_path_buf(),
            project,
            kind,
        });
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// RestoresDir
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct Inner {
    /// Canonical path of the Restores folder (on Windows: verbatim).
    dir: PathBuf,
    /// Its file identity — what every restore re-verifies.
    id: FileId,
    /// Canonical watched roots it was validated against.
    roots: Vec<PathBuf>,
    /// Wit's own data folder (canonical), holding the staging journal.
    data_dir: PathBuf,
    journal: Journal,
}

/// The one folder restores are created in — validated at construction and
/// re-checked before every step. See the [module docs](self) for the law.
///
/// Cheap to clone (it's an `Arc`).
#[derive(Debug, Clone)]
pub struct RestoresDir {
    inner: Arc<Inner>,
}

impl RestoresDir {
    /// Validate `path` against `watched`, create it if needed, and return
    /// the only handle Wit's restore functions accept. `data_dir` is Wit's
    /// own data folder (created if needed); the staging journal lives there,
    /// and it must not be inside the Restores folder.
    ///
    /// Refuses (creating nothing that stays) a path that resolves inside a
    /// DAW project folder or above another watched root; a path whose
    /// missing tail contains `..`; a path through a symlink created while
    /// building it; and a path that exists but isn't a directory. Being
    /// inside a watched root, or being one, is fine.
    pub fn new(
        path: &Path,
        watched: &WatchedRoots,
        data_dir: &Path,
    ) -> Result<RestoresDir, CloneError> {
        if path.as_os_str().is_empty() {
            return Err(CloneError::InvalidRestoresPath {
                path: path.to_path_buf(),
                reason: "empty path",
            });
        }
        let roots: Vec<PathBuf> = watched.iter().map(|r| r.path().to_path_buf()).collect();

        // 1. Check where it *would* be, before creating anything.
        let planned = paths::canonicalize_lenient(path).map_err(io_err("resolve", path))?;
        check_placement(&planned, None, &roots)?;

        // 2. Create the missing tail one folder at a time, through handles.
        let absolute = std::path::absolute(path).map_err(io_err("resolve", path))?;
        let components: Vec<Component<'_>> = absolute.components().collect();
        let mut split = components.len();
        let base = loop {
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
        let mut dir = Dir::open(&base).map_err(io_err("open", &base))?;
        // Folders this call created, innermost last, to undo on refusal.
        let mut created: Vec<(Dir, OsString)> = Vec::new();
        let undo = |created: Vec<(Dir, OsString)>| {
            for (parent, name) in created.into_iter().rev() {
                let _ = parent.remove_empty_dir(&name);
            }
        };
        for component in &components[split..] {
            let Component::Normal(name) = component else {
                drop(dir); // close before undo (Windows can't remove an open folder)
                undo(created);
                return Err(CloneError::InvalidRestoresPath {
                    path: path.to_path_buf(),
                    reason: "it has '..' or '.' after a folder that doesn't exist yet",
                });
            };
            let made = match dir.mkdir(name) {
                Ok(()) => true,
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => false,
                Err(e) => {
                    let err = io_err("create folder", &dir.path().join(name))(e);
                    drop(dir);
                    undo(created);
                    return Err(err);
                }
            };
            let child = match dir.open_dir(name) {
                Ok(c) => c,
                Err(_) => {
                    if made {
                        let _ = dir.remove_empty_dir(name);
                    }
                    drop(dir);
                    undo(created);
                    return Err(CloneError::InvalidRestoresPath {
                        path: path.to_path_buf(),
                        reason: "a part of it is a symlink or not a folder",
                    });
                }
            };
            let checked = child
                .id()
                .map_err(io_err("inspect", child.path()))
                .and_then(|id| check_placement(child.path(), Some(id), &roots));
            if let Err(e) = checked {
                drop(child);
                if made {
                    let _ = dir.remove_empty_dir(name);
                }
                drop(dir);
                undo(created);
                return Err(e);
            }
            let parent = std::mem::replace(&mut dir, child);
            if made {
                created.push((parent, name.to_os_string()));
            }
        }

        // 3. The authoritative check: the OS's own resolution of the caller's
        //    spelling must be exactly the folder we hold.
        let checked = (|| {
            let id = dir.id().map_err(io_err("inspect", dir.path()))?;
            let resolved = fs::canonicalize(path).map_err(io_err("resolve", path))?;
            if FileId::of_path(&resolved).ok() != Some(id) {
                return Err(CloneError::InvalidRestoresPath {
                    path: path.to_path_buf(),
                    reason: "it resolves to a different folder from one moment to the next",
                });
            }
            for spelling in [&resolved, &dir.path().to_path_buf()] {
                check_placement(spelling, Some(id), &roots)?;
            }
            Ok((id, resolved))
        })();
        // Close the innermost handle before any undo: Windows can't remove
        // a folder that is still open.
        drop(dir);
        let (id, resolved) = match checked {
            Ok(v) => v,
            Err(e) => {
                undo(created);
                return Err(e);
            }
        };
        drop(created); // keep them: validation passed

        // 4. Wit's data folder (journal): never inside the Restores folder.
        fs::create_dir_all(data_dir).map_err(io_err("create folder", data_dir))?;
        let data = fs::canonicalize(data_dir).map_err(io_err("resolve", data_dir))?;
        let data_inside =
            paths::is_within_canonical(&data, &resolved, paths::CaseSensitivity::Insensitive)
                || data
                    .ancestors()
                    .any(|a| FileId::of_path(a).is_ok_and(|x| x == id));
        if data_inside {
            return Err(CloneError::InvalidDataDir {
                path: data,
                reason: "it is inside the Restores folder",
            });
        }
        let journal = Journal::open(&data).map_err(io_err("open journal in", &data))?;
        Ok(RestoresDir {
            inner: Arc::new(Inner {
                dir: resolved,
                id,
                roots,
                data_dir: data,
                journal,
            }),
        })
    }

    /// The canonical path (with `\\?\` on Windows — use for I/O).
    pub fn path(&self) -> &Path {
        &self.inner.dir
    }

    /// Wit's data folder this Restores folder journals into (the watcher
    /// ignores it by construction).
    pub fn data_dir(&self) -> &Path {
        &self.inner.data_dir
    }

    /// For the settings UI.
    pub fn display(&self) -> String {
        paths::display(&self.inner.dir)
    }

    /// Check this Restores folder against another root set — the watcher
    /// calls this with the exact set it is about to watch.
    pub fn check_against(&self, watched: &WatchedRoots) -> Result<(), CloneError> {
        let roots: Vec<PathBuf> = watched.iter().map(|r| r.path().to_path_buf()).collect();
        check_placement(&self.inner.dir, Some(self.inner.id), &roots)
    }

    /// Re-prove the placement against the filesystem as it is *now*: the
    /// path must still name exactly the validated folder, and it must still
    /// not be inside a project or contain another watched root (as recorded,
    /// and as those roots resolve today).
    pub fn revalidate(&self) -> Result<(), CloneError> {
        let moved = || CloneError::RestoresMoved {
            expected: self.inner.dir.clone(),
        };
        if FileId::of_path(&self.inner.dir).map_err(|_| moved())? != self.inner.id {
            return Err(moved());
        }
        check_placement(&self.inner.dir, Some(self.inner.id), &self.inner.roots)?;
        let fresh: Vec<PathBuf> = self
            .inner
            .roots
            .iter()
            .filter_map(|r| fs::canonicalize(r).ok())
            .collect();
        check_placement(&self.inner.dir, Some(self.inner.id), &fresh)
    }

    /// Open a pinned handle on the Restores folder for one restore: it must
    /// be the validated folder, and the placement must still hold.
    fn pin(&self) -> Result<Dir, CloneError> {
        let moved = || CloneError::RestoresMoved {
            expected: self.inner.dir.clone(),
        };
        let dir = Dir::open(&self.inner.dir).map_err(|_| moved())?;
        if dir.id().map_err(|_| moved())? != self.inner.id {
            return Err(moved());
        }
        self.revalidate()?;
        Ok(dir)
    }

    /// The per-step check during a restore: the path still names the folder
    /// the handle pins, and the placement still holds.
    fn check_pinned(&self, dir: &Dir) -> Result<(), CloneError> {
        let moved = || CloneError::RestoresMoved {
            expected: self.inner.dir.clone(),
        };
        if dir.id().map_err(|_| moved())? != self.inner.id {
            return Err(moved());
        }
        self.revalidate()
    }

    /// Mint a fresh, unused destination `<Song> — <date>[.ext]`, or
    /// `<Song> — <date> (2)[.ext]` … if that is taken. Nothing is created
    /// yet; the name is re-checked (and bumped if needed) when the restore
    /// is committed, so two restores racing for a name both succeed.
    ///
    /// `song` and `date_label` are sanitised and shortened to fit every
    /// filesystem (see `names`). `extension` must be 1–16 ASCII
    /// alphanumerics (`"logicx"`, `"als"`, `"flp"`), or `None` for a folder.
    pub fn fresh_destination(
        &self,
        song: &str,
        date_label: &str,
        extension: Option<&str>,
    ) -> Result<RestoreDest, CloneError> {
        let root = self.pin()?;
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
        let song = names::sanitize_component(song, "Untitled");
        let date = names::sanitize_component(date_label, "");
        let base = names::fit_base(&song, &date, extension.as_deref());
        let mut dest = RestoreDest {
            restores: self.clone(),
            base,
            extension,
            attempt: 1,
            name: String::new(),
        };
        dest.pick_free_name(&root)?;
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

    /// Staging entries a crashed run left in this Restores folder that the
    /// journal proves are Wit's own (created, never finished, same name and
    /// file identity). Entries that are gone are closed in the journal.
    /// Nothing is removed — show these to the user, then call
    /// [`remove_staging_leftover`](Self::remove_staging_leftover).
    pub fn staging_leftovers(&self) -> Result<Vec<StagingLeftover>, CloneError> {
        let root = self.pin()?;
        let journal = &self.inner.journal;
        let pending = journal
            .pending(self.inner.id)
            .map_err(io_err("read journal in", &self.inner.data_dir))?;
        let mut out = Vec::new();
        for p in pending {
            let name = OsStr::new(&p.name);
            match root.kind(name) {
                Ok(None) => {
                    let _ = journal.finished(self.inner.id, &p.name);
                }
                Ok(Some(_)) => {
                    if root.id_of(name).is_ok_and(|id| id == p.entry) {
                        out.push(StagingLeftover {
                            path: self.inner.dir.join(&p.name),
                            name: p.name,
                            created: p.created,
                            id: p.entry,
                        });
                    }
                }
                Err(e) => return Err(io_err("inspect", &self.inner.dir.join(&p.name))(e)),
            }
        }
        let _ = journal.compact_if_idle();
        Ok(out)
    }

    /// Remove one staging leftover — only if it is still journaled as Wit's
    /// own, unfinished, and still the same entry (name and file identity).
    pub fn remove_staging_leftover(&self, leftover: &StagingLeftover) -> Result<(), CloneError> {
        let refuse = || CloneError::NotAStagingLeftover {
            name: leftover.name.clone(),
        };
        if !leftover.name.starts_with(STAGING_PREFIX) {
            return Err(refuse());
        }
        let root = self.pin()?;
        let journal = &self.inner.journal;
        let still_pending = journal
            .pending(self.inner.id)
            .map_err(io_err("read journal in", &self.inner.data_dir))?
            .iter()
            .any(|p| p.name == leftover.name && p.entry == leftover.id);
        let name = OsStr::new(&leftover.name);
        if !still_pending || !root.id_of(name).is_ok_and(|id| id == leftover.id) {
            return Err(refuse());
        }
        root.remove_tree(name)
            .map_err(io_err("remove", &leftover.path))?;
        let _ = journal.finished(self.inner.id, &leftover.name);
        Ok(())
    }
}

/// A crash leftover that the journal proves is Wit's own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagingLeftover {
    pub name: String,
    pub path: PathBuf,
    pub created: SystemTime,
    id: FileId,
}

// ---------------------------------------------------------------------------
// Destinations and committing
// ---------------------------------------------------------------------------

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
    /// at it, checked through the pinned handle without following links.
    fn pick_free_name(&mut self, root: &Dir) -> Result<(), CloneError> {
        while self.attempt <= MAX_NAME_ATTEMPTS {
            let name = self.render();
            match root.kind(OsStr::new(&name)) {
                Ok(None) => {
                    self.name = name;
                    return Ok(());
                }
                Ok(Some(_)) => self.attempt += 1,
                Err(e) => return Err(io_err("inspect", &root.path().join(&name))(e)),
            }
        }
        Err(CloneError::NoFreeName {
            base: self.base.clone(),
        })
    }
}

fn staging_name() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!(
        "{STAGING_PREFIX}{}-{}-{}",
        std::process::id(),
        nanos,
        COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}

/// A finished restore.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Restored {
    /// Where it landed (canonical; `\\?\` on Windows).
    pub path: PathBuf,
    pub report: CloneReport,
    /// Only on filesystems without an atomic no-replace rename: the restore
    /// is complete at `path`, but its staging name couldn't be removed and
    /// is still there (another name for the same file). It stays in the
    /// journal, so [`RestoresDir::staging_leftovers`] lists it.
    pub leftover_staging: Option<PathBuf>,
}

/// Move a finished staging entry to `dest`'s name through the pinned root,
/// bumping ` (n)` if a racing writer took the name first. Never replaces.
fn commit_entry(
    root: &Dir,
    staging: &str,
    mut dest: RestoreDest,
) -> Result<(PathBuf, Option<PathBuf>), CloneError> {
    let restores = dest.restores.clone();
    loop {
        restores.check_pinned(root)?;
        let target = restores.path().join(&dest.name);
        match root.rename_noreplace(OsStr::new(staging), OsStr::new(&dest.name)) {
            Ok(Renamed::Moved) => {
                root.sync();
                let _ = restores.inner.journal.finished(restores.inner.id, staging);
                return Ok((target, None));
            }
            Ok(Renamed::LinkedLeftover) => {
                root.sync();
                return Ok((target, Some(restores.path().join(staging))));
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                dest.attempt += 1;
                dest.pick_free_name(root)?;
            }
            Err(e) => return Err(io_err("move into place", &target)(e)),
        }
    }
}

/// Journal a staging entry this call just created; if that fails, remove
/// the entry (it's ours) and fail.
fn journal_created(restores: &RestoresDir, root: &Dir, name: &str) -> Result<FileId, CloneError> {
    let path = restores.path().join(name);
    let recorded = root.id_of(OsStr::new(name)).and_then(|id| {
        restores
            .inner
            .journal
            .created(restores.inner.id, name, id)
            .map(|()| id)
    });
    match recorded {
        Ok(id) => Ok(id),
        Err(e) => {
            let _ = root.remove_tree(OsStr::new(name));
            Err(io_err("journal", &path)(e))
        }
    }
}

/// Remove a staging entry this call created (verified by identity) and
/// close it in the journal.
fn discard_own(restores: &RestoresDir, root: &Dir, name: &str, id: FileId) {
    let n = OsStr::new(name);
    if root.id_of(n).is_ok_and(|x| x == id) && root.remove_tree(n).is_ok() {
        let _ = restores.inner.journal.finished(restores.inner.id, name);
    }
}

// ---------------------------------------------------------------------------
// Copying
// ---------------------------------------------------------------------------

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
    restores_id: FileId,
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

fn space_for(probe: &Path, bytes: u64) -> Result<(), CloneError> {
    let available = fs4::available_space(probe).map_err(io_err("check free space", probe))?;
    let needed = bytes.saturating_add((bytes / 20).max(MIN_SPACE_MARGIN));
    if available < needed {
        return Err(CloneError::InsufficientSpace { needed, available });
    }
    Ok(())
}

/// Does the source carry a lock/append-only flag that a clone would copy?
/// (Finder "Locked" is `uchg`.) Such files are byte-copied instead, so a
/// restored copy is never locked.
#[cfg(target_os = "macos")]
fn is_locked(meta: &fs::Metadata) -> bool {
    use std::os::macos::fs::MetadataExt;
    const UF_IMMUTABLE: u32 = 0x2;
    const UF_APPEND: u32 = 0x4;
    const UF_NOUNLINK: u32 = 0x10;
    const SF_IMMUTABLE: u32 = 0x2_0000;
    const SF_APPEND: u32 = 0x4_0000;
    const SF_NOUNLINK: u32 = 0x10_0000;
    meta.st_flags()
        & (UF_IMMUTABLE | UF_APPEND | UF_NOUNLINK | SF_IMMUTABLE | SF_APPEND | SF_NOUNLINK)
        != 0
}

#[cfg(not(target_os = "macos"))]
fn is_locked(_meta: &fs::Metadata) -> bool {
    // Linux FICLONE doesn't copy inode flags; Windows clones get their
    // read-only attribute cleared in `fsops`.
    false
}

/// Copy one regular file into `dst/name`: clone first, byte copy (after a
/// space check) if that isn't possible. `name` must not exist. On error,
/// removes only what this call created.
fn clone_file(
    src_path: &Path,
    meta: &fs::Metadata,
    dst: &Dir,
    name: &OsStr,
    ctx: &mut CloneCtx,
) -> Result<(), CloneError> {
    let dst_path = dst.path().join(name);
    let mut src = File::open(src_path).map_err(io_err("open (read-only)", src_path))?;
    let len = meta.len();
    let locked = is_locked(meta);
    let mut out: Option<File> = None;
    if ctx.mode == CopyMode::TryReflink && !locked {
        match dst.clone_file(&src, src_path, name) {
            Ok(Cloned::Yes) => {
                ctx.report.reflinked_files += 1;
                ctx.remaining = ctx.remaining.saturating_sub(len);
                return Ok(());
            }
            Ok(Cloned::No(created)) => {
                ctx.mode = CopyMode::Copy;
                out = created;
            }
            // Includes AlreadyExists: nothing of ours to remove.
            Err(e) => return Err(io_err("clone", &dst_path)(e)),
        }
    }
    let space = if ctx.mode == CopyMode::TryReflink {
        space_for(&ctx.space_probe, len) // a single locked file
    } else if ctx.space_checked {
        Ok(())
    } else {
        ctx.space_checked = true;
        space_for(&ctx.space_probe, ctx.remaining)
    };
    if let Err(e) = space {
        if out.take().is_some() {
            let _ = dst.remove_file(name);
        }
        return Err(e);
    }
    let mut out = match out {
        Some(f) => f,
        None => dst.create_file(name).map_err(io_err("create", &dst_path))?,
    };
    let copied = (|| {
        let n = io::copy(&mut src, &mut out)?;
        if let Ok(modified) = meta.modified() {
            // `out` was opened for writing, which Windows needs for this.
            let _ = out.set_modified(modified);
        }
        out.sync_all()?;
        Ok::<u64, io::Error>(n)
    })();
    match copied {
        Ok(n) => {
            ctx.report.copied_files += 1;
            ctx.report.copied_bytes += n;
            ctx.remaining = ctx.remaining.saturating_sub(len);
            Ok(())
        }
        Err(e) => {
            drop(out);
            let _ = dst.remove_file(name);
            Err(io_err("copy", &dst_path)(e))
        }
    }
}

/// Recreate the contents of directory `src` inside the pinned `dst`,
/// skipping symlinks and special files, refusing to descend into the
/// Restores folder itself.
fn clone_dir_contents(
    src: &Path,
    dst: &Dir,
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
        let rel = rel.join(&name);
        let meta = fs::symlink_metadata(&from).map_err(io_err("inspect", &from))?;
        let kind = meta.file_type();
        if kind.is_symlink() {
            ctx.report.skipped_symlinks.push(rel);
        } else if kind.is_dir() {
            if FileId::of_path(&from).is_ok_and(|id| id == ctx.restores_id) {
                return Err(CloneError::SourceContainsRestores { source: from });
            }
            let to = dst.path().join(&name);
            dst.mkdir(&name).map_err(io_err("create folder", &to))?;
            let child = dst.open_dir(&name).map_err(io_err("open", &to))?;
            clone_dir_contents(&from, &child, &rel, depth + 1, ctx)?;
        } else if kind.is_file() {
            clone_file(&from, &meta, dst, &name, ctx)?;
        } else {
            ctx.report.skipped_special.push(rel);
        }
    }
    Ok(())
}

/// Resolve a clone source: follow a symlink *at* `src` (reading through it
/// is harmless), require a real file or directory, and refuse a source that
/// contains the Restores folder — by spelling or by identity.
fn resolve_source(
    src: &Path,
    restores: &RestoresDir,
) -> Result<(PathBuf, fs::Metadata), CloneError> {
    let resolved = fs::canonicalize(src).map_err(io_err("resolve", src))?;
    let meta = fs::symlink_metadata(&resolved).map_err(io_err("inspect", &resolved))?;
    if !(meta.is_dir() || meta.is_file()) {
        return Err(CloneError::NotARealEntry { path: resolved });
    }
    let by_spelling = paths::is_within_canonical(
        restores.path(),
        &resolved,
        paths::CaseSensitivity::Insensitive,
    );
    let by_identity = FileId::of_path(&resolved).is_ok_and(|src_id| {
        restores
            .path()
            .ancestors()
            .filter(|a| !a.as_os_str().is_empty())
            .any(|a| FileId::of_path(a).is_ok_and(|x| x == src_id))
    });
    if by_spelling || by_identity {
        return Err(CloneError::SourceContainsRestores { source: resolved });
    }
    Ok((resolved, meta))
}

/// Copy `src` (a project package directory, or a single project file) into
/// the Restores folder under `dest`'s fresh name, copy-on-write where the
/// filesystem allows. The copy is built under a hidden staging name and
/// renamed into place only when complete; on any error nothing of it is
/// left behind. `src` is only ever read.
pub fn clone_tree(src: &Path, dest: RestoreDest) -> Result<Restored, CloneError> {
    let (source, meta) = resolve_source(src, &dest.restores)?;
    if meta.is_dir() {
        let mut new = NewRestore::begin(dest)?;
        let report = new.clone_tree_from(&source)?;
        let mut restored = new.commit()?;
        restored.report = report;
        return Ok(restored);
    }
    let restores = dest.restores.clone();
    let root = restores.pin()?;
    let staging = staging_name();
    let mut ctx = CloneCtx {
        mode: CopyMode::TryReflink,
        remaining: meta.len(),
        space_checked: false,
        space_probe: restores.path().to_path_buf(),
        restores_id: restores.inner.id,
        report: CloneReport {
            total_bytes: meta.len(),
            ..CloneReport::default()
        },
    };
    clone_file(&source, &meta, &root, OsStr::new(&staging), &mut ctx)?;
    let id = journal_created(&restores, &root, &staging)?;
    match commit_entry(&root, &staging, dest) {
        Ok((path, leftover_staging)) => Ok(Restored {
            path,
            report: ctx.report,
            leftover_staging,
        }),
        Err(e) => {
            discard_own(&restores, &root, &staging, id);
            Err(e)
        }
    }
}

/// Write `bytes` as a new file under `dest`'s fresh name (e.g. a stored
/// `.als` or `.flp`). Written to a hidden staging file, flushed, then moved
/// into place without replacing anything.
pub fn write_file_in_restores(dest: RestoreDest, bytes: &[u8]) -> Result<Restored, CloneError> {
    let restores = dest.restores.clone();
    let root = restores.pin()?;
    let staging = staging_name();
    let path = restores.path().join(&staging);
    let mut f = root
        .create_file(OsStr::new(&staging))
        .map_err(io_err("create", &path))?;
    let id = journal_created(&restores, &root, &staging)?;
    if let Err(e) = f.write_all(bytes).and_then(|()| f.sync_all()) {
        drop(f);
        discard_own(&restores, &root, &staging, id);
        return Err(io_err("write", &path)(e));
    }
    drop(f);
    match commit_entry(&root, &staging, dest) {
        Ok((path, leftover_staging)) => Ok(Restored {
            path,
            report: CloneReport {
                total_bytes: bytes.len() as u64,
                copied_files: 1,
                copied_bytes: bytes.len() as u64,
                ..CloneReport::default()
            },
            leftover_staging,
        }),
        Err(e) => {
            discard_own(&restores, &root, &staging, id);
            Err(e)
        }
    }
}

/// Clear the contents of `rel` inside the restore being built (e.g. the
/// copy's `Alternatives/000/Project File Backups`, so a restored project
/// doesn't carry the original's backup history). See
/// [`NewRestore::remove_dir_contents`]; this free function exists so the
/// only removal API is visibly scoped to the entry the current restore
/// created — it cannot name an existing restore or a project.
pub fn remove_dir_contents_in_restores(
    new: &mut NewRestore,
    rel: &Path,
) -> Result<usize, CloneError> {
    new.remove_dir_contents(rel)
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

// ---------------------------------------------------------------------------
// NewRestore
// ---------------------------------------------------------------------------

/// The one entry the current restore is creating — the **only** handle
/// through which anything can be written or removed. It owns a hidden
/// staging folder that [`NewRestore::begin`] just created inside the
/// Restores folder, holds pinned handles on both, and accepts only paths
/// relative to its staging folder, opening each level without following
/// symlinks — so it cannot reach an earlier restore, a project, or anything
/// else that existed before this restore began.
///
/// [`commit`](NewRestore::commit) moves it to its fresh name; dropping it
/// without committing deletes its staging folder (and only that, verified
/// by identity).
#[derive(Debug)]
pub struct NewRestore {
    restores: RestoresDir,
    root: Dir,
    staging: Option<Dir>,
    staging_name: String,
    staging_id: FileId,
    dest: Option<RestoreDest>,
    report: CloneReport,
}

impl NewRestore {
    /// Create an empty, brand-new staging folder for `dest`.
    pub fn begin(dest: RestoreDest) -> Result<NewRestore, CloneError> {
        let restores = dest.restores.clone();
        let root = restores.pin()?;
        let name = staging_name();
        let path = restores.path().join(&name);
        root.mkdir(OsStr::new(&name))
            .map_err(io_err("create staging folder", &path))?;
        let staging_id = journal_created(&restores, &root, &name)?;
        let staging = match root.open_dir(OsStr::new(&name)) {
            Ok(d) if d.id().is_ok_and(|id| id == staging_id) => d,
            _ => {
                discard_own(&restores, &root, &name, staging_id);
                return Err(CloneError::NotARealEntry { path });
            }
        };
        Ok(NewRestore {
            restores,
            root,
            staging: Some(staging),
            staging_name: name,
            staging_id,
            dest: Some(dest),
            report: CloneReport::default(),
        })
    }

    /// Where the staging folder is, for reading back what was staged.
    pub fn staging_path(&self) -> PathBuf {
        self.restores.path().join(&self.staging_name)
    }

    /// The per-step check: Restores still pinned and in place, the staging
    /// folder still the one this restore created.
    fn check(&self) -> Result<&Dir, CloneError> {
        self.restores.check_pinned(&self.root)?;
        let staging = self
            .staging
            .as_ref()
            .ok_or_else(|| CloneError::NotARealEntry {
                path: self.staging_path(),
            })?;
        let ok = self
            .root
            .id_of(OsStr::new(&self.staging_name))
            .is_ok_and(|id| id == self.staging_id)
            && staging.id().is_ok_and(|id| id == self.staging_id);
        if !ok {
            return Err(CloneError::NotARealEntry {
                path: self.staging_path(),
            });
        }
        Ok(staging)
    }

    /// Open (and with `create`, make) the folders `names` inside the staging
    /// folder, one level at a time with `O_NOFOLLOW` — a symlink anywhere on
    /// the way is an error, never followed. `Ok(None)` if a level is missing
    /// and `create` is off.
    fn walk_dirs(&self, names: &[OsString], create: bool) -> Result<Option<Dir>, CloneError> {
        let staging = self.check()?;
        let mut current: Option<Dir> = None;
        for name in names {
            let parent = current.as_ref().unwrap_or(staging);
            let path = parent.path().join(name);
            match parent.kind(name).map_err(io_err("inspect", &path))? {
                Some(fsops::EntryKind::Dir) => {}
                Some(_) => return Err(CloneError::NotARealEntry { path }),
                None if create => parent.mkdir(name).map_err(io_err("create folder", &path))?,
                None => return Ok(None),
            }
            let next = parent
                .open_dir(name)
                .map_err(|_| CloneError::NotARealEntry { path: path.clone() })?;
            current = Some(next);
        }
        match current {
            Some(d) => Ok(Some(d)),
            None => {
                let d = self
                    .root
                    .open_dir(OsStr::new(&self.staging_name))
                    .map_err(|_| CloneError::NotARealEntry {
                        path: self.staging_path(),
                    })?;
                Ok(Some(d))
            }
        }
    }

    /// Clone the *contents* of the directory `src` into the staging folder
    /// (so the new restore becomes a copy of `src`). The staging folder must
    /// still be empty.
    pub fn clone_tree_from(&mut self, src: &Path) -> Result<CloneReport, CloneError> {
        let (source, meta) = resolve_source(src, &self.restores)?;
        if !meta.is_dir() {
            return Err(CloneError::InvalidRelativePath {
                path: source,
                reason: "a new restore clones a folder; use clone_tree for a single file",
            });
        }
        let staging = self.check()?;
        let not_empty = !staging
            .entries()
            .map_err(io_err("list", &self.staging_path()))?
            .is_empty();
        if not_empty {
            return Err(CloneError::InvalidRelativePath {
                path: self.staging_path(),
                reason: "the new restore already has content",
            });
        }
        let total = tree_size(&source, 0)?;
        let mut ctx = CloneCtx {
            mode: CopyMode::TryReflink,
            remaining: total,
            space_checked: false,
            space_probe: self.restores.path().to_path_buf(),
            restores_id: self.restores.inner.id,
            report: CloneReport {
                total_bytes: total,
                ..CloneReport::default()
            },
        };
        clone_dir_contents(&source, staging, Path::new(""), 0, &mut ctx)?;
        self.report = ctx.report.clone();
        Ok(ctx.report)
    }

    /// Write `bytes` at `rel` inside the new restore, creating parent
    /// folders as needed. **Replaces** a file already staged there (that's
    /// how a stored `ProjectData` is swapped into a cloned package) — by
    /// writing a new file and renaming it over the staged entry, never by
    /// writing into it.
    pub fn write_file(&mut self, rel: &Path, bytes: &[u8]) -> Result<(), CloneError> {
        let names = validate_relative(rel, false)?;
        let (file_name, parents) = names.split_last().expect("validated as non-empty");
        let parent = self
            .walk_dirs(parents, true)?
            .expect("walk_dirs with create always returns a folder");
        let target = parent.path().join(file_name);
        if parent.kind(file_name).map_err(io_err("inspect", &target))?
            == Some(fsops::EntryKind::Dir)
        {
            return Err(CloneError::NotARealEntry { path: target });
        }
        let temp = OsString::from(format!(
            "{TEMP_PREFIX}{}",
            staging_name().trim_start_matches(STAGING_PREFIX)
        ));
        let temp_path = parent.path().join(&temp);
        let mut f = parent
            .create_file(&temp)
            .map_err(io_err("create", &temp_path))?;
        let written = f.write_all(bytes).and_then(|()| f.sync_all());
        drop(f);
        if let Err(e) = written.and_then(|()| parent.rename_replace(&temp, file_name)) {
            let _ = parent.remove_file(&temp);
            return Err(io_err("write", &target)(e));
        }
        Ok(())
    }

    /// Remove everything inside the folder `rel` of the new restore (keeping
    /// the folder itself). A missing folder is not an error. Symlinks are
    /// removed as links, never followed. Returns how many entries were
    /// removed. An empty `rel` clears the whole new restore.
    pub fn remove_dir_contents(&mut self, rel: &Path) -> Result<usize, CloneError> {
        let names = validate_relative(rel, true)?;
        let Some(dir) = self.walk_dirs(&names, false)? else {
            return Ok(0);
        };
        let entries = dir.entries().map_err(io_err("list", dir.path()))?;
        for entry in &entries {
            dir.remove_tree(entry)
                .map_err(io_err("remove", &dir.path().join(entry)))?;
        }
        Ok(entries.len())
    }

    /// Move the finished copy to its fresh name (bumping ` (n)` if the name
    /// was taken meanwhile) and return where it landed.
    pub fn commit(mut self) -> Result<Restored, CloneError> {
        let staging = self.check()?;
        // Refuse to publish a tree someone tampered with while it was being
        // built: a symlink or extra hard link planted inside the staging
        // folder would let a DAW opening the restored copy write through it
        // into a project. (Drop then removes the staging folder.)
        verify_only_own_entries(staging, 0)?;
        // Windows can't rename a folder Wit itself holds open.
        self.staging = None;
        let dest = self
            .dest
            .take()
            .expect("a NewRestore holds its destination until commit");
        match commit_entry(&self.root, &self.staging_name, dest) {
            Ok((path, leftover_staging)) => Ok(Restored {
                path,
                report: std::mem::take(&mut self.report),
                leftover_staging,
            }),
            Err(e) => {
                discard_own(
                    &self.restores,
                    &self.root,
                    &self.staging_name,
                    self.staging_id,
                );
                Err(e)
            }
        }
    }
}

/// Everything under `dir` must be a real folder or a single-link regular
/// file — what a restore itself creates. Walked through handles, never
/// following links.
fn verify_only_own_entries(dir: &Dir, depth: usize) -> Result<(), CloneError> {
    if depth > MAX_CLONE_DEPTH {
        return Err(CloneError::TooDeep {
            path: dir.path().to_path_buf(),
        });
    }
    for name in dir.entries().map_err(io_err("list", dir.path()))? {
        let path = dir.path().join(&name);
        match dir.kind(&name).map_err(io_err("inspect", &path))? {
            None => {}
            Some(fsops::EntryKind::File) => {
                if dir.links_of(&name).map_err(io_err("inspect", &path))? > 1 {
                    return Err(CloneError::NotARealEntry { path });
                }
            }
            Some(fsops::EntryKind::Dir) => {
                let child = dir
                    .open_dir(&name)
                    .map_err(|_| CloneError::NotARealEntry { path: path.clone() })?;
                verify_only_own_entries(&child, depth + 1)?;
            }
            Some(_) => return Err(CloneError::NotARealEntry { path }),
        }
    }
    Ok(())
}

impl Drop for NewRestore {
    fn drop(&mut self) {
        if self.dest.is_none() {
            return; // committed (or already cleaned up by a failed commit)
        }
        self.staging = None; // close before removing (Windows)
        discard_own(
            &self.restores,
            &self.root,
            &self.staging_name,
            self.staging_id,
        );
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
mod tests;
