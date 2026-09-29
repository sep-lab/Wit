//! Where Wit looks for projects, and where it puts restores.
//!
//! # Guarantees
//!
//! - **Pure and hermetic.** [`candidate_roots`] and [`default_restores_dir`]
//!   take a [`HomeLayout`] and a [`TargetOs`] and touch no filesystem;
//!   [`default_roots`] only adds an `is_dir()` filter. Tests build a fake
//!   home in a temp dir — nothing here ever looks at the real `~`.
//! - **Only folders that exist are proposed.** A Windows user without FL
//!   Studio doesn't get an FL Studio root.
//! - **The default Restores folder is never inside a default root** on any
//!   OS (unit-tested for all three): on Linux and Windows `Music` itself is
//!   a discovery root, so Restores lives next to it rather than in it.
//! - **[`WatchedRoots`] holds canonical paths only** (symlinks resolved,
//!   verbatim prefix kept on Windows), so containment checks against it are
//!   exact.
//!
//! # Default discovery roots
//!
//! | OS | Roots (each only if it exists) |
//! |---|---|
//! | macOS | `~/Music/Logic`, `~/Music/GarageBand`, `~/Music/Ableton` (Live's User Library — sets are often saved elsewhere), `~/Documents/Image-Line/FL Studio/Projects` |
//! | Windows | `%USERPROFILE%\Documents\Image-Line\FL Studio\Projects`, `%USERPROFILE%\Documents\Ableton`, `%USERPROFILE%\Music`, plus the same two under `OneDrive\Documents` (folder redirection) |
//! | Linux | `~/Music`, `~/Documents` (both honour `~/.config/user-dirs.dirs`), and REAPER's project folder `~/Documents/REAPER Media` or `~/REAPER Media` when not already covered |
//!
//! The REAPER location on Linux and the OneDrive redirection on Windows are
//! **inferred** from those products' documented defaults, not verified on a
//! real install. Anything else is one "Add folder" away ([`UserFolders`]).
//!
//! # Default Restores folder
//!
//! | OS | Folder |
//! |---|---|
//! | macOS | `~/Music/Wit Restores` |
//! | Windows | `%USERPROFILE%\Documents\Wit Restores` |
//! | Linux | `~/Wit Restores` |

use crate::daw::Daw;
use crate::paths;
use crate::TargetOs;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

/// The handful of per-user folders root discovery is computed from. Built
/// from a home dir so tests are hermetic; [`HomeLayout::detect`] is the only
/// function that reads the environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HomeLayout {
    pub home: PathBuf,
    pub documents: PathBuf,
    pub music: PathBuf,
}

impl HomeLayout {
    /// `Documents` and `Music` directly under `home` — the default on macOS,
    /// Windows, and an English-locale Linux desktop.
    pub fn from_home(home: impl Into<PathBuf>) -> HomeLayout {
        let home = home.into();
        HomeLayout {
            documents: home.join("Documents"),
            music: home.join("Music"),
            home,
        }
    }

    /// Apply a Linux `user-dirs.dirs` file's `XDG_MUSIC_DIR` and
    /// `XDG_DOCUMENTS_DIR` (localised desktops rename them: `~/Musik`,
    /// `~/Documents` → `~/Dokumente`, …).
    pub fn with_xdg_user_dirs(mut self, user_dirs_file: &str) -> HomeLayout {
        if let Some(music) = parse_xdg_user_dir(user_dirs_file, "XDG_MUSIC_DIR", &self.home) {
            self.music = music;
        }
        if let Some(docs) = parse_xdg_user_dir(user_dirs_file, "XDG_DOCUMENTS_DIR", &self.home) {
            self.documents = docs;
        }
        self
    }

    /// The real user's layout: `%USERPROFILE%` on Windows, `$HOME`
    /// elsewhere, plus `user-dirs.dirs` on Linux. `None` if no home is set.
    pub fn detect() -> Option<HomeLayout> {
        let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
        let home = std::env::var_os(var).filter(|h| !h.is_empty())?;
        let layout = HomeLayout::from_home(PathBuf::from(home));
        if TargetOs::current() != TargetOs::Linux {
            return Some(layout);
        }
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .filter(|c| !c.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| layout.home.join(".config"));
        match std::fs::read_to_string(config.join("user-dirs.dirs")) {
            Ok(text) => Some(layout.with_xdg_user_dirs(&text)),
            Err(_) => Some(layout),
        }
    }
}

/// Read one `KEY="$HOME/Name"` line from an XDG `user-dirs.dirs` file. A
/// value of exactly `$HOME` means "disabled" in the XDG spec, and returns
/// `None` rather than the whole home directory.
pub fn parse_xdg_user_dir(text: &str, key: &str, home: &Path) -> Option<PathBuf> {
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('#') {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        if k.trim() != key {
            continue;
        }
        let v = v.trim().trim_matches('"');
        if v == "$HOME" || v == "$HOME/" {
            return None;
        }
        if let Some(rest) = v.strip_prefix("$HOME/") {
            return Some(home.join(rest));
        }
        if v.starts_with('/') {
            return Some(PathBuf::from(v));
        }
        return None;
    }
    None
}

/// A folder Wit proposes to watch by default.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateRoot {
    pub path: PathBuf,
    /// The DAW whose default save location this is, if it belongs to one.
    pub daw: Option<Daw>,
    /// A short, honest note for the settings UI.
    pub note: &'static str,
}

fn candidate(path: PathBuf, daw: Option<Daw>, note: &'static str) -> CandidateRoot {
    CandidateRoot { path, daw, note }
}

/// Every default root for `os`, **whether or not it exists** (pure; no I/O).
pub fn candidate_roots(layout: &HomeLayout, os: TargetOs) -> Vec<CandidateRoot> {
    let fl = |docs: &Path| docs.join("Image-Line").join("FL Studio").join("Projects");
    match os {
        TargetOs::MacOs => vec![
            candidate(
                layout.music.join("Logic"),
                Some(Daw::LogicPro),
                "Logic Pro's default project folder",
            ),
            candidate(
                layout.music.join("GarageBand"),
                Some(Daw::GarageBand),
                "GarageBand's default project folder",
            ),
            candidate(
                layout.music.join("Ableton"),
                Some(Daw::AbletonLive),
                "Live's User Library — sets are often saved elsewhere; add that folder too",
            ),
            candidate(
                fl(&layout.documents),
                Some(Daw::FlStudio),
                "FL Studio's default project folder",
            ),
        ],
        TargetOs::Windows => {
            let mut roots = vec![
                candidate(
                    fl(&layout.documents),
                    Some(Daw::FlStudio),
                    "FL Studio's default project folder",
                ),
                candidate(
                    layout.documents.join("Ableton"),
                    Some(Daw::AbletonLive),
                    "Ableton Live's default folder",
                ),
                candidate(layout.music.clone(), None, "your Music folder"),
            ];
            let onedrive_docs = layout.home.join("OneDrive").join("Documents");
            if onedrive_docs != layout.documents {
                roots.push(candidate(
                    fl(&onedrive_docs),
                    Some(Daw::FlStudio),
                    "FL Studio's project folder under OneDrive folder redirection",
                ));
                roots.push(candidate(
                    onedrive_docs.join("Ableton"),
                    Some(Daw::AbletonLive),
                    "Ableton Live's folder under OneDrive folder redirection",
                ));
            }
            roots
        }
        TargetOs::Linux => vec![
            candidate(layout.music.clone(), None, "your Music folder"),
            candidate(layout.documents.clone(), None, "your Documents folder"),
            candidate(
                layout.documents.join("REAPER Media"),
                Some(Daw::Reaper),
                "REAPER's default project folder",
            ),
            candidate(
                layout.home.join("REAPER Media"),
                Some(Daw::Reaper),
                "REAPER's project folder",
            ),
        ],
    }
}

/// The default roots for this OS that **exist** and are directories, with
/// any root that lies inside another dropped (watching `~/Documents` already
/// covers `~/Documents/REAPER Media`).
pub fn default_roots(layout: &HomeLayout) -> Vec<CandidateRoot> {
    let existing: Vec<CandidateRoot> = candidate_roots(layout, TargetOs::current())
        .into_iter()
        .filter(|c| c.path.is_dir())
        .collect();
    dedupe_nested(existing)
}

/// Drop every root that lies inside (or equals) an earlier-kept root.
pub fn dedupe_nested(roots: Vec<CandidateRoot>) -> Vec<CandidateRoot> {
    let mut kept: Vec<CandidateRoot> = Vec::new();
    for root in roots {
        if kept.iter().any(|k| paths::is_within(&root.path, &k.path)) {
            continue;
        }
        kept.retain(|k| !paths::is_within(&k.path, &root.path));
        kept.push(root);
    }
    kept
}

/// Where restores go by default on `os` (pure; not created here — see
/// [`crate::clone::RestoresDir::new`]).
pub fn default_restores_dir(layout: &HomeLayout, os: TargetOs) -> PathBuf {
    match os {
        TargetOs::MacOs => layout.music.join("Wit Restores"),
        TargetOs::Windows => layout.documents.join("Wit Restores"),
        TargetOs::Linux => layout.home.join("Wit Restores"),
    }
}

/// How events under a root are classified by [`crate::watch`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RootKind {
    /// A default DAW folder: only recognised project formats count.
    Discovery,
    /// A folder the user added: recognised formats, **plus** any other file
    /// (the generic "History" tier — every save kept, no semantic diff).
    UserFolder,
}

/// One watched folder, canonicalised.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchedRoot {
    path: PathBuf,
    kind: RootKind,
}

impl WatchedRoot {
    /// The canonical path (on Windows, with its `\\?\` prefix).
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn kind(&self) -> RootKind {
        self.kind
    }

    /// For the settings UI.
    pub fn display(&self) -> String {
        paths::display(&self.path)
    }
}

#[derive(Debug)]
pub enum RootError {
    /// The folder doesn't exist or isn't a directory.
    NotADirectory(PathBuf),
    Io(PathBuf, io::Error),
}

impl fmt::Display for RootError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RootError::NotADirectory(p) => {
                write!(f, "{} isn't a folder Wit can watch", paths::display(p))
            }
            RootError::Io(p, e) => write!(f, "can't read {}: {e}", paths::display(p)),
        }
    }
}

impl std::error::Error for RootError {}

/// The set of folders Wit watches. Every entry is an existing directory,
/// stored canonicalised; the same folder spelled twice (symlink, case,
/// NFD, trailing separator) is stored once.
///
/// Building a `WatchedRoots` grants nothing: it is the input a
/// [`crate::clone::RestoresDir`] is validated against, and the
/// [`crate::watch::ProjectWatcher`] re-checks the Restores folder against
/// the exact set it is about to watch.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WatchedRoots {
    roots: Vec<WatchedRoot>,
}

impl WatchedRoots {
    pub fn new() -> WatchedRoots {
        WatchedRoots::default()
    }

    /// Add a folder. A folder already present keeps one entry; adding it as
    /// a [`RootKind::UserFolder`] upgrades a discovery root to the generic
    /// tier.
    pub fn add(&mut self, path: &Path, kind: RootKind) -> Result<(), RootError> {
        let canonical = std::fs::canonicalize(path).map_err(|e| match e.kind() {
            io::ErrorKind::NotFound => RootError::NotADirectory(path.to_path_buf()),
            _ => RootError::Io(path.to_path_buf(), e),
        })?;
        if !canonical.is_dir() {
            return Err(RootError::NotADirectory(path.to_path_buf()));
        }
        if let Some(existing) = self
            .roots
            .iter_mut()
            .find(|r| same_canonical(&r.path, &canonical))
        {
            if kind == RootKind::UserFolder {
                existing.kind = RootKind::UserFolder;
            }
            return Ok(());
        }
        self.roots.push(WatchedRoot {
            path: canonical,
            kind,
        });
        Ok(())
    }

    /// Remove a folder (however it is spelled). Returns whether it was present.
    pub fn remove(&mut self, path: &Path) -> bool {
        let Ok(canonical) = paths::canonicalize_lenient(path) else {
            return false;
        };
        let before = self.roots.len();
        self.roots.retain(|r| !same_canonical(&r.path, &canonical));
        before != self.roots.len()
    }

    pub fn iter(&self) -> impl Iterator<Item = &WatchedRoot> {
        self.roots.iter()
    }

    pub fn len(&self) -> usize {
        self.roots.len()
    }

    pub fn is_empty(&self) -> bool {
        self.roots.is_empty()
    }

    /// The most specific root containing an already-canonical `path`.
    pub fn root_for(&self, canonical_path: &Path) -> Option<&WatchedRoot> {
        self.roots
            .iter()
            .filter(|r| {
                paths::is_within_canonical(
                    canonical_path,
                    &r.path,
                    paths::CaseSensitivity::Insensitive,
                )
            })
            .max_by_key(|r| r.path.components().count())
    }

    /// Roots that aren't inside another root — what the watcher registers
    /// with the OS (a nested root is covered by its parent's recursive watch).
    pub fn outermost(&self) -> Vec<&WatchedRoot> {
        let strictly_inside = |inner: &Path, outer: &Path| {
            !same_canonical(inner, outer)
                && paths::is_within_canonical(inner, outer, paths::CaseSensitivity::Insensitive)
        };
        self.roots
            .iter()
            .filter(|r| {
                !self
                    .roots
                    .iter()
                    .any(|other| strictly_inside(&r.path, &other.path))
            })
            .collect()
    }
}

fn same_canonical(a: &Path, b: &Path) -> bool {
    paths::is_within_canonical(a, b, paths::CaseSensitivity::Insensitive)
        && paths::is_within_canonical(b, a, paths::CaseSensitivity::Insensitive)
}

/// The folders a user added with "Add folder", in the order they added
/// them. Stored as given so a folder on an unplugged drive survives a
/// restart; [`UserFolders::apply_to`] skips (and reports) ones that are
/// missing right now.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UserFolders {
    folders: Vec<PathBuf>,
}

impl UserFolders {
    pub fn new() -> UserFolders {
        UserFolders::default()
    }

    /// Restore a saved list without touching the filesystem.
    pub fn from_saved(folders: impl IntoIterator<Item = PathBuf>) -> UserFolders {
        let mut list = UserFolders::new();
        for folder in folders {
            if !list.folders.iter().any(|f| f == &folder) {
                list.folders.push(folder);
            }
        }
        list
    }

    /// Add a folder the user picked. It must exist and be a directory;
    /// returns `false` if it was already in the list (under any spelling).
    pub fn add(&mut self, path: &Path) -> Result<bool, RootError> {
        let canonical = std::fs::canonicalize(path)
            .map_err(|_| RootError::NotADirectory(path.to_path_buf()))?;
        if !canonical.is_dir() {
            return Err(RootError::NotADirectory(path.to_path_buf()));
        }
        let already = self
            .folders
            .iter()
            .any(|f| paths::canonicalize_lenient(f).is_ok_and(|c| same_canonical(&c, &canonical)));
        if already {
            return Ok(false);
        }
        self.folders.push(canonical);
        Ok(true)
    }

    /// Remove a folder (however it is spelled). Returns whether it was present.
    pub fn remove(&mut self, path: &Path) -> bool {
        let Ok(target) = paths::canonicalize_lenient(path) else {
            return false;
        };
        let before = self.folders.len();
        self.folders
            .retain(|f| !paths::canonicalize_lenient(f).is_ok_and(|c| same_canonical(&c, &target)));
        before != self.folders.len()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Path> {
        self.folders.iter().map(PathBuf::as_path)
    }

    /// Add every folder that exists right now to `roots` as a
    /// [`RootKind::UserFolder`]; returns the ones that were skipped.
    pub fn apply_to(&self, roots: &mut WatchedRoots) -> Vec<PathBuf> {
        let mut skipped = Vec::new();
        for folder in &self.folders {
            if roots.add(folder, RootKind::UserFolder).is_err() {
                skipped.push(folder.clone());
            }
        }
        skipped
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_OS: [TargetOs; 3] = [TargetOs::MacOs, TargetOs::Windows, TargetOs::Linux];

    fn fake_home() -> (tempfile::TempDir, HomeLayout) {
        let dir = tempfile::tempdir().unwrap();
        let layout = HomeLayout::from_home(dir.path().join("home"));
        (dir, layout)
    }

    #[test]
    fn candidates_match_the_documented_table() {
        let layout = HomeLayout::from_home("/h");
        let mac: Vec<PathBuf> = candidate_roots(&layout, TargetOs::MacOs)
            .into_iter()
            .map(|c| c.path)
            .collect();
        assert_eq!(
            mac,
            vec![
                PathBuf::from("/h/Music/Logic"),
                PathBuf::from("/h/Music/GarageBand"),
                PathBuf::from("/h/Music/Ableton"),
                PathBuf::from("/h/Documents/Image-Line/FL Studio/Projects"),
            ]
        );
        let win = candidate_roots(&layout, TargetOs::Windows);
        assert!(win.iter().any(|c| c.path == Path::new("/h/Music")));
        assert!(win
            .iter()
            .any(|c| c.path == Path::new("/h/OneDrive/Documents/Ableton")));
        let linux = candidate_roots(&layout, TargetOs::Linux);
        assert_eq!(linux[0].path, Path::new("/h/Music"));
        assert_eq!(linux[1].path, Path::new("/h/Documents"));
    }

    #[test]
    fn only_existing_folders_are_proposed() {
        let (_dir, layout) = fake_home();
        assert!(default_roots(&layout).is_empty(), "nothing exists yet");
        let first = candidate_roots(&layout, TargetOs::current()).remove(0).path;
        std::fs::create_dir_all(&first).unwrap();
        let roots = default_roots(&layout);
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].path, first);
    }

    #[test]
    fn nested_candidates_collapse_to_the_outer_one() {
        let (_dir, layout) = fake_home();
        let outer = layout.documents.clone();
        let inner = layout.documents.join("REAPER Media");
        std::fs::create_dir_all(&inner).unwrap();
        let kept = dedupe_nested(vec![
            candidate(inner.clone(), Some(Daw::Reaper), ""),
            candidate(outer.clone(), None, ""),
        ]);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].path, outer);
    }

    #[test]
    fn default_restores_dir_is_outside_every_default_root_on_every_os() {
        let layout = HomeLayout::from_home("/h");
        for os in ALL_OS {
            let restores = default_restores_dir(&layout, os);
            for root in candidate_roots(&layout, os) {
                assert!(
                    !paths::overlaps_canonical(&restores, &root.path),
                    "{os:?}: {} overlaps {}",
                    restores.display(),
                    root.path.display()
                );
            }
        }
    }

    #[test]
    fn xdg_user_dirs_relocate_music_and_documents() {
        let text = "# comment\nXDG_MUSIC_DIR=\"$HOME/Musik\"\nXDG_DOCUMENTS_DIR=\"/data/Dokumente\"\nXDG_DESKTOP_DIR=\"$HOME\"\n";
        let layout = HomeLayout::from_home("/h").with_xdg_user_dirs(text);
        assert_eq!(layout.music, Path::new("/h/Musik"));
        assert_eq!(layout.documents, Path::new("/data/Dokumente"));
        assert_eq!(
            parse_xdg_user_dir(text, "XDG_DESKTOP_DIR", Path::new("/h")),
            None
        );
        assert_eq!(
            parse_xdg_user_dir(text, "XDG_VIDEOS_DIR", Path::new("/h")),
            None
        );
    }

    #[test]
    fn watched_roots_dedupe_spellings_and_find_the_most_specific_root() {
        let dir = tempfile::tempdir().unwrap();
        let outer = dir.path().join("Music");
        let inner = outer.join("Projects");
        std::fs::create_dir_all(&inner).unwrap();
        let mut roots = WatchedRoots::new();
        roots.add(&outer, RootKind::Discovery).unwrap();
        roots.add(&outer.join("."), RootKind::Discovery).unwrap();
        roots.add(&inner, RootKind::UserFolder).unwrap();
        assert_eq!(roots.len(), 2);
        let canonical_inner = std::fs::canonicalize(&inner).unwrap();
        let found = roots.root_for(&canonical_inner.join("song.rpp")).unwrap();
        assert_eq!(found.kind(), RootKind::UserFolder);
        assert_eq!(roots.outermost().len(), 1);
        assert!(roots.remove(&inner));
        assert_eq!(roots.len(), 1);
        // Upgrading a discovery root to a user folder keeps one entry.
        roots.add(&outer, RootKind::UserFolder).unwrap();
        assert_eq!(roots.len(), 1);
        assert_eq!(roots.iter().next().unwrap().kind(), RootKind::UserFolder);
    }

    #[test]
    fn watched_roots_refuse_missing_folders_and_files() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("f");
        std::fs::write(&file, b"x").unwrap();
        let mut roots = WatchedRoots::new();
        assert!(matches!(
            roots.add(&dir.path().join("missing"), RootKind::Discovery),
            Err(RootError::NotADirectory(_))
        ));
        assert!(matches!(
            roots.add(&file, RootKind::Discovery),
            Err(RootError::NotADirectory(_))
        ));
    }

    #[test]
    fn user_folders_keep_order_skip_missing_and_dedupe() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a");
        std::fs::create_dir_all(&a).unwrap();
        let mut folders = UserFolders::new();
        assert!(folders.add(&a).unwrap());
        assert!(!folders.add(&a.join(".")).unwrap());
        let saved: Vec<PathBuf> = folders
            .iter()
            .map(Path::to_path_buf)
            .chain([dir.path().join("unplugged")])
            .collect();
        let restored = UserFolders::from_saved(saved);
        let mut roots = WatchedRoots::new();
        let skipped = restored.apply_to(&mut roots);
        assert_eq!(skipped, vec![dir.path().join("unplugged")]);
        assert_eq!(roots.len(), 1);
        assert!(folders.remove(&a));
        assert_eq!(folders.iter().count(), 0);
    }
}
