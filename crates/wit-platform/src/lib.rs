//! The per-OS platform layer for Wit — and **the only code in Wit that writes
//! outside Wit's own data dir.**
//!
//! Everything here is a thin, testable seam between Wit's pure core (parsers,
//! diff, story) and the operating system:
//!
//! | Module | What it does |
//! |---|---|
//! | [`paths`] | NFC normalisation, case policy, Windows verbatim-path handling, symlink-resolving containment ([`paths::is_within`]), and the cross-platform [`paths::assert_no_home_paths`] privacy check |
//! | [`roots`] | Per-OS default discovery roots (only those that exist), the default Restores folder, the user's "Add folder" list, and [`roots::WatchedRoots`] |
//! | [`watch`] | A `notify`-based recursive watcher that emits **one** event per save, only after the project's files have stopped changing |
//! | [`daw`] | Which DAWs are running right now (`sysinfo`), and open/quit session bookkeeping |
//! | [`reveal`] | Reveal in Finder / Explorer / the file manager, and "open with the DAW" — argv built by pure functions |
//! | [`clone`] | Restore-as-copy **primitives**: [`clone::RestoresDir`], copy-on-write tree cloning, and write/remove helpers that can only address paths inside a Restores folder |
//!
//! # The write law (ADR-0007, restore-as-copy)
//!
//! Wit never modifies a user's DAW project. The only writes this crate can
//! perform land inside a [`clone::RestoresDir`], and that type cannot be
//! constructed for a folder that is inside, equal to, or a parent of any
//! watched root. None of the write functions accept a `Path` destination —
//! they accept only destinations minted by a `RestoresDir`, so a watched
//! project path cannot be passed to them even by mistake. See the
//! [`clone`] module docs for the full argument, and
//! `tests/no_write_in_watched_root.rs` for the property test that checks it
//! against generated trees full of symlinks, `..` components, trailing
//! separators, NFC/NFD spellings and case variants.
//!
//! # What this crate will never do
//!
//! - **Write into a watched root, or anywhere outside a validated Restores
//!   folder.** Not a file, not a directory, not an attribute. (Wit's own
//!   data dir is written by `wit-index`, not by this crate.)
//! - **Open a source file with write access.** Clone sources are opened
//!   read-only (or cloned by the kernel with `clonefile`/`FICLONE`/
//!   `FSCTL_DUPLICATE_EXTENTS`, which never modifies the source).
//! - **Overwrite anything that already exists in the Restores folder.** A
//!   restore always gets a fresh name (`<Song> — <date>`, then ` (2)`, …).
//! - **Leave a half-written copy behind.** Copies are assembled under a
//!   hidden staging name inside the Restores folder and renamed into place
//!   only when complete; a failed or abandoned copy removes its staging dir.
//! - **Delete anything except its own staging data.** The only removal APIs
//!   operate inside a staging dir this process created.
//! - **Reproduce a symlink in a copy.** A symlink inside a project could point
//!   back into the original project; a DAW opening the copy would then write
//!   through it. Symlinks are skipped and reported instead.
//! - **Run a shell over untrusted text.** Reveal/open commands are built as
//!   argv with absolute paths only; the one Windows `cmd /c start` form uses
//!   explicit quoting and refuses characters `cmd` would expand.
//! - **Launch a DAW on its own.** "Open with the DAW" runs only when a caller
//!   asks for it (a user click).
//! - **Touch the network or send telemetry.** There is no network code in
//!   this crate, and none of its dependencies are network clients.
//!
//! # Honest limits
//!
//! Only macOS was exercised by hand while writing this crate; Linux and
//! Windows are covered by the same unit/integration/property tests in CI.
//! Per-OS behaviour that could not be exercised end-to-end (Explorer's
//! `/select,` parsing, ReFS reflinks, Windows symlink creation without
//! Developer Mode) is marked in each module's docs.

#![forbid(unsafe_code)]

pub mod clone;
pub mod daw;
pub mod paths;
pub mod reveal;
pub mod roots;
pub mod watch;

/// The operating-system family a pure function should answer for.
///
/// Most functions in this crate that differ per OS take a `TargetOs`
/// argument instead of reading `cfg!(target_os)` directly, so every OS's
/// behaviour is unit-tested on every CI leg (a macOS runner still checks
/// the Windows argv, and vice versa). [`TargetOs::current`] is the value
/// production code passes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TargetOs {
    MacOs,
    Windows,
    /// Linux, and — conservatively — any other Unix (the BSDs get Linux's
    /// `xdg-open` behaviour and XDG directory layout).
    Linux,
}

impl TargetOs {
    /// The OS this binary was compiled for.
    pub const fn current() -> TargetOs {
        if cfg!(target_os = "macos") {
            TargetOs::MacOs
        } else if cfg!(windows) {
            TargetOs::Windows
        } else {
            TargetOs::Linux
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_os_matches_the_compile_target() {
        let os = TargetOs::current();
        if cfg!(target_os = "macos") {
            assert_eq!(os, TargetOs::MacOs);
        } else if cfg!(windows) {
            assert_eq!(os, TargetOs::Windows);
        } else {
            assert_eq!(os, TargetOs::Linux);
        }
    }
}
