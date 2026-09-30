//! Reveal a file in the OS file manager, or open it with its default app
//! (the DAW).
//!
//! # Guarantees
//!
//! - **argv is built by pure functions** ([`reveal_plan`], [`open_plan`])
//!   that take a [`TargetOs`], so every OS's exact command line is
//!   unit-tested on every CI leg without spawning anything.
//! - **No argument injection from hostile file names.** Only absolute paths
//!   are accepted, so a name can never be parsed as an option (`-R`,
//!   `--args`, `/select`); control characters (newlines, NUL, …) are
//!   refused outright; nothing goes through a shell except the one Windows
//!   form below, whose quoting is explicit and whose unsafe characters are
//!   refused.
//! - **Nothing is spawned unless a caller asks** ([`reveal`],
//!   [`open_with_default_app`]); the child is reaped on a background thread
//!   so no zombie is left behind.
//! - **"Open" only opens projects** ([`check_openable`]). `open`,
//!   `cmd /c start` and `xdg-open` launch *anything* openable — an `.app`,
//!   `.command`, `.webloc`, `.exe`, `.bat`, `.lnk` or `.desktop` file. So
//!   Wit opens only what the path **resolves to** (a symlinked `.als`
//!   pointing at an app is judged as the app): a recognised project
//!   (`.logicx`/`.band` folders; `.als`, `.flp`, `.rpp` and the other DAW
//!   formats the watcher knows), a plain folder without an extension, or —
//!   inside a watched *user-added* folder only — a file with an allow-listed,
//!   non-executable extension (audio, MIDI, text, PDF, images).
//!
//! # Per-OS commands
//!
//! | OS | Reveal | Open with the DAW |
//! |---|---|---|
//! | macOS | `open -R <path>` | `open <path>` |
//! | Windows | `explorer.exe /select,"<path>"` | `cmd.exe /d /v:off /c start "" "<path>"`; `explorer.exe "<path>"` if the path contains `%` |
//! | Linux | `xdg-open <parent folder>` | `xdg-open <path>` |
//!
//! Windows command lines are passed verbatim (`raw_arg`) because neither
//! Explorer nor `cmd` parse arguments the MSVCRT way Rust quotes for.
//! Inside double quotes `cmd` treats `& | < > ( ) ^` literally; `!` is
//! neutralised by `/v:off`; `%` cannot be escaped inside quotes, so such
//! paths go to Explorer instead; `"` cannot occur in a Windows file name.
//! The Windows forms are **unit-tested but were not run on a Windows
//! machine** while writing this module. Linux's `org.freedesktop.FileManager1`
//! `ShowItems` (which would select the file) is not used: it needs a D-Bus
//! client, and the parent-folder fallback needs none.

use crate::paths;
use crate::roots::{RootKind, WatchedRoots};
use crate::TargetOs;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// A command to run, described as data so tests can assert on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchPlan {
    program: OsString,
    args: Vec<OsString>,
    /// Windows only: the command-line tail, passed through unmodified.
    raw_args: Option<String>,
}

impl LaunchPlan {
    pub fn program(&self) -> &OsStr {
        &self.program
    }

    /// Arguments passed as separate argv entries (macOS/Linux).
    pub fn args(&self) -> &[OsString] {
        &self.args
    }

    /// The exact Windows command-line tail, if this is a Windows plan.
    pub fn raw_args(&self) -> Option<&str> {
        self.raw_args.as_deref()
    }

    /// The `Command` this plan describes, for the OS this binary runs on.
    pub fn to_command(&self) -> Command {
        let mut command = Command::new(&self.program);
        command.args(&self.args);
        if let Some(raw) = &self.raw_args {
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                command.raw_arg(raw);
            }
            #[cfg(not(windows))]
            {
                // A Windows plan can't run elsewhere; keep it inert-but-visible.
                command.arg(raw);
            }
        }
        command
    }

    /// Spawn with no stdio and reap the child on a background thread.
    pub fn spawn(&self) -> io::Result<()> {
        let mut child = self
            .to_command()
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        Ok(())
    }
}

#[derive(Debug)]
pub enum RevealError {
    /// Only absolute paths are accepted — a relative one could start with `-`.
    NotAbsolute(String),
    /// Newlines, NUL and other control characters (and `"` in a Windows path) are refused.
    ForbiddenCharacter(String),
    /// A Windows path that isn't valid Unicode can't be quoted safely.
    NotUnicode,
    /// Not a project (or allow-listed file in a user folder); Wit won't
    /// launch it.
    NotOpenable(String),
    /// The path doesn't exist, or spawning failed.
    Io(io::Error),
}

impl fmt::Display for RevealError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RevealError::NotAbsolute(p) => write!(f, "refusing a relative path: {p:?}"),
            RevealError::ForbiddenCharacter(p) => {
                write!(
                    f,
                    "refusing a path with a control character or quote: {p:?}"
                )
            }
            RevealError::NotUnicode => write!(f, "refusing a non-Unicode Windows path"),
            RevealError::NotOpenable(p) => {
                write!(f, "Wit only opens projects and plain media files: {p:?}")
            }
            RevealError::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for RevealError {}

fn has_control_chars(s: &str) -> bool {
    s.chars().any(|c| c.is_control())
}

fn is_windows_absolute(s: &str) -> bool {
    let b = s.as_bytes();
    (b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/'))
        || s.starts_with(r"\\")
}

/// Validate and render `path` for `os`: returns the string form to embed.
fn checked(path: &Path, os: TargetOs) -> Result<String, RevealError> {
    let lossy = path.to_string_lossy();
    if has_control_chars(&lossy) {
        return Err(RevealError::ForbiddenCharacter(lossy.into_owned()));
    }
    match os {
        TargetOs::Windows => {
            let text = path.to_str().ok_or(RevealError::NotUnicode)?;
            if !is_windows_absolute(text) {
                return Err(RevealError::NotAbsolute(text.to_string()));
            }
            // A `"` can't be in a Windows file name; refuse rather than escape.
            if text.contains('"') {
                return Err(RevealError::ForbiddenCharacter(text.to_string()));
            }
            Ok(paths::simplify_windows_verbatim(text).into_owned())
        }
        TargetOs::MacOs | TargetOs::Linux => {
            if !lossy.starts_with('/') {
                return Err(RevealError::NotAbsolute(lossy.into_owned()));
            }
            Ok(lossy.into_owned())
        }
    }
}

/// The command that shows `path` selected in the file manager of `os`.
pub fn reveal_plan(path: &Path, os: TargetOs) -> Result<LaunchPlan, RevealError> {
    let text = checked(path, os)?;
    Ok(match os {
        TargetOs::MacOs => LaunchPlan {
            program: "open".into(),
            args: vec!["-R".into(), path.as_os_str().to_os_string()],
            raw_args: None,
        },
        TargetOs::Windows => LaunchPlan {
            program: "explorer.exe".into(),
            args: Vec::new(),
            raw_args: Some(format!("/select,\"{text}\"")),
        },
        TargetOs::Linux => {
            let folder = path
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(path);
            LaunchPlan {
                program: "xdg-open".into(),
                args: vec![folder.as_os_str().to_os_string()],
                raw_args: None,
            }
        }
    })
}

/// The command that opens `path` with its default application on `os` —
/// for a project, its DAW.
pub fn open_plan(path: &Path, os: TargetOs) -> Result<LaunchPlan, RevealError> {
    let text = checked(path, os)?;
    Ok(match os {
        TargetOs::MacOs => LaunchPlan {
            program: "open".into(),
            args: vec![path.as_os_str().to_os_string()],
            raw_args: None,
        },
        TargetOs::Windows if text.contains('%') => LaunchPlan {
            program: "explorer.exe".into(),
            args: Vec::new(),
            raw_args: Some(format!("\"{text}\"")),
        },
        TargetOs::Windows => LaunchPlan {
            program: "cmd.exe".into(),
            args: Vec::new(),
            raw_args: Some(format!("/d /v:off /c start \"\" \"{text}\"")),
        },
        TargetOs::Linux => LaunchPlan {
            program: "xdg-open".into(),
            args: vec![path.as_os_str().to_os_string()],
            raw_args: None,
        },
    })
}

/// Reveal an existing `path` in this OS's file manager.
pub fn reveal(path: &Path) -> Result<(), RevealError> {
    std::fs::symlink_metadata(path).map_err(RevealError::Io)?;
    reveal_plan(path, TargetOs::current())?
        .spawn()
        .map_err(RevealError::Io)
}

/// Project formats Wit will open anywhere (files).
const PROJECT_FILES: &[&str] = &[
    "als",
    "flp",
    "rpp",
    "bwproject",
    "ardour",
    "song",
    "cpr",
    "npr",
    "ptx",
    "dawproject",
    "mmpz",
    "mmp",
    "rns",
    "reason",
    "aup3",
];
/// Project packages Wit will open anywhere (folders).
const PROJECT_FOLDERS: &[&str] = &["logicx", "band"];
/// Non-executable files Wit will open inside a watched *user-added* folder.
const GENERIC_OPENABLE: &[&str] = &[
    "wav", "aif", "aiff", "aifc", "flac", "mp3", "m4a", "aac", "ogg", "opus", "caf", "mid", "midi",
    "txt", "md", "rtf", "pdf", "png", "jpg", "jpeg",
];

fn ext_of(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
}

/// Decide whether Wit may hand `path` to the OS "open" command, judging what
/// it **resolves to**. Returns the resolved path, which is what gets opened.
pub fn check_openable(path: &Path, watched: &WatchedRoots) -> Result<PathBuf, RevealError> {
    let resolved = std::fs::canonicalize(path).map_err(RevealError::Io)?;
    let meta = std::fs::metadata(&resolved).map_err(RevealError::Io)?;
    let ext = ext_of(&resolved);
    let refuse = || RevealError::NotOpenable(paths::display(&resolved));
    if meta.is_dir() {
        return match ext.as_deref() {
            None => Ok(resolved),
            Some(e) if PROJECT_FOLDERS.contains(&e) => Ok(resolved),
            Some(_) => Err(refuse()), // .app, .bundle, .framework, …
        };
    }
    if !meta.is_file() {
        return Err(refuse());
    }
    match ext.as_deref() {
        Some(e) if PROJECT_FILES.contains(&e) => Ok(resolved),
        Some(e) if GENERIC_OPENABLE.contains(&e) => {
            let in_user_folder = watched.iter().any(|r| {
                r.kind() == RootKind::UserFolder
                    && paths::is_within_canonical(
                        &resolved,
                        r.path(),
                        paths::CaseSensitivity::Insensitive,
                    )
            });
            if in_user_folder {
                Ok(resolved)
            } else {
                Err(refuse())
            }
        }
        _ => Err(refuse()),
    }
}

/// Open an existing project with its default application (its DAW), after
/// [`check_openable`]. Only ever call this from an explicit user action.
pub fn open_with_default_app(path: &Path, watched: &WatchedRoots) -> Result<(), RevealError> {
    let resolved = check_openable(path, watched)?;
    open_plan(&resolved, TargetOs::current())?
        .spawn()
        .map_err(RevealError::Io)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(plan: &LaunchPlan) -> Vec<String> {
        std::iter::once(plan.program())
            .chain(plan.args().iter().map(OsString::as_os_str))
            .map(|s| s.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn macos_reveal_and_open() {
        let p = Path::new("/Users/you/Music/Logic/My Song.logicx");
        let plan = reveal_plan(p, TargetOs::MacOs).unwrap();
        assert_eq!(
            argv(&plan),
            ["open", "-R", "/Users/you/Music/Logic/My Song.logicx"]
        );
        assert_eq!(plan.raw_args(), None);
        let plan = open_plan(p, TargetOs::MacOs).unwrap();
        assert_eq!(
            argv(&plan),
            ["open", "/Users/you/Music/Logic/My Song.logicx"]
        );
    }

    #[test]
    fn linux_reveal_opens_the_parent_folder() {
        let p = Path::new("/home/you/Music/song.rpp");
        let plan = reveal_plan(p, TargetOs::Linux).unwrap();
        assert_eq!(argv(&plan), ["xdg-open", "/home/you/Music"]);
        let plan = open_plan(p, TargetOs::Linux).unwrap();
        assert_eq!(argv(&plan), ["xdg-open", "/home/you/Music/song.rpp"]);
        let root = reveal_plan(Path::new("/"), TargetOs::Linux).unwrap();
        assert_eq!(argv(&root), ["xdg-open", "/"]);
    }

    #[test]
    fn windows_reveal_uses_raw_select_with_quotes() {
        let p = Path::new(r"C:\Users\you\Music\Song, final & mix.als");
        let plan = reveal_plan(p, TargetOs::Windows).unwrap();
        assert_eq!(plan.program(), "explorer.exe");
        assert!(plan.args().is_empty());
        assert_eq!(
            plan.raw_args(),
            Some(r#"/select,"C:\Users\you\Music\Song, final & mix.als""#)
        );
    }

    #[test]
    fn windows_open_quotes_for_cmd_and_avoids_percent_expansion() {
        let p = Path::new(r"C:\Music\Tom & Jerry (v2) ^ !x.flp");
        let plan = open_plan(p, TargetOs::Windows).unwrap();
        assert_eq!(plan.program(), "cmd.exe");
        assert_eq!(
            plan.raw_args(),
            Some(r#"/d /v:off /c start "" "C:\Music\Tom & Jerry (v2) ^ !x.flp""#)
        );
        let pct = Path::new(r"C:\Music\100%PATH%.flp");
        let plan = open_plan(pct, TargetOs::Windows).unwrap();
        assert_eq!(plan.program(), "explorer.exe");
        assert_eq!(plan.raw_args(), Some(r#""C:\Music\100%PATH%.flp""#));
    }

    #[test]
    fn windows_verbatim_paths_are_simplified_for_explorer() {
        let p = Path::new(r"\\?\C:\Music\Song.als");
        let plan = reveal_plan(p, TargetOs::Windows).unwrap();
        assert_eq!(plan.raw_args(), Some(r#"/select,"C:\Music\Song.als""#));
        let unc = Path::new(r"\\?\UNC\nas\share\Song.als");
        let plan = open_plan(unc, TargetOs::Windows).unwrap();
        assert_eq!(
            plan.raw_args(),
            Some(r#"/d /v:off /c start "" "\\nas\share\Song.als""#)
        );
    }

    #[test]
    fn hostile_names_are_refused() {
        for os in [TargetOs::MacOs, TargetOs::Linux, TargetOs::Windows] {
            // A relative path could be read as an option.
            assert!(matches!(
                reveal_plan(Path::new("-R"), os),
                Err(RevealError::NotAbsolute(_))
            ));
            assert!(matches!(
                open_plan(Path::new("--args"), os),
                Err(RevealError::NotAbsolute(_))
            ));
            assert!(matches!(
                open_plan(Path::new("/x/evil\nrm -rf ~"), os),
                Err(RevealError::ForbiddenCharacter(_)) | Err(RevealError::NotAbsolute(_))
            ));
        }
        assert!(matches!(
            open_plan(Path::new(r#"C:\a" & calc & ".als"#), TargetOs::Windows),
            Err(RevealError::ForbiddenCharacter(_))
        ));
        assert!(matches!(
            reveal_plan(Path::new("/unix/style"), TargetOs::Windows),
            Err(RevealError::NotAbsolute(_))
        ));
    }

    #[test]
    fn a_name_that_looks_like_an_option_is_only_ever_a_path_suffix() {
        let p = Path::new("/Music/-R --args -a Calculator");
        let plan = open_plan(p, TargetOs::MacOs).unwrap();
        assert_eq!(argv(&plan), ["open", "/Music/-R --args -a Calculator"]);
    }

    #[test]
    fn reveal_refuses_a_missing_path_without_spawning() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(
            reveal(&dir.path().join("missing")),
            Err(RevealError::Io(_))
        ));
        assert!(matches!(
            open_with_default_app(&dir.path().join("missing"), &WatchedRoots::new()),
            Err(RevealError::Io(_))
        ));
    }

    #[test]
    fn only_projects_and_allow_listed_files_in_user_folders_are_openable() {
        let dir = tempfile::tempdir().unwrap();
        let base = std::fs::canonicalize(dir.path()).unwrap();
        let user = base.join("user");
        let other = base.join("other");
        for d in [&user, &other] {
            std::fs::create_dir_all(d).unwrap();
        }
        let mut roots = WatchedRoots::new();
        roots.add(&user, RootKind::UserFolder).unwrap();
        let file = |p: &Path| {
            std::fs::write(p, b"x").unwrap();
            p.to_path_buf()
        };
        let dirp = |p: &Path| {
            std::fs::create_dir_all(p).unwrap();
            p.to_path_buf()
        };
        // Projects: anywhere.
        for ok in [
            file(&other.join("Set.als")),
            file(&other.join("Beat.FLP")),
            file(&other.join("song.rpp")),
            dirp(&other.join("Song.logicx")),
            dirp(&other.join("plain folder")),
        ] {
            assert!(check_openable(&ok, &roots).is_ok(), "{}", ok.display());
        }
        // Allow-listed media: only inside a user folder.
        assert!(check_openable(&file(&user.join("take.wav")), &roots).is_ok());
        assert!(check_openable(&file(&other.join("take.wav")), &roots).is_err());
        // Anything launchable: never.
        for bad in [
            dirp(&other.join("Evil.app")),
            file(&user.join("run.command")),
            file(&user.join("link.webloc")),
            file(&user.join("setup.exe")),
            file(&user.join("x.bat")),
            file(&user.join("x.lnk")),
            file(&user.join("x.desktop")),
            file(&user.join("script.sh")),
            file(&user.join("noext")),
        ] {
            assert!(
                matches!(
                    check_openable(&bad, &roots),
                    Err(RevealError::NotOpenable(_))
                ),
                "{}",
                bad.display()
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_project_name_symlinked_to_an_app_is_judged_as_the_app() {
        let dir = tempfile::tempdir().unwrap();
        let base = std::fs::canonicalize(dir.path()).unwrap();
        std::fs::create_dir_all(base.join("Evil.app")).unwrap();
        std::os::unix::fs::symlink(base.join("Evil.app"), base.join("Song.logicx")).unwrap();
        std::fs::write(base.join("run.command"), b"#!/bin/sh").unwrap();
        std::os::unix::fs::symlink(base.join("run.command"), base.join("Set.als")).unwrap();
        let roots = WatchedRoots::new();
        assert!(check_openable(&base.join("Song.logicx"), &roots).is_err());
        assert!(check_openable(&base.join("Set.als"), &roots).is_err());
    }
}
