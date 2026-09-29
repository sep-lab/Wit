//! Path handling that is honest about how three operating systems actually
//! name files.
//!
//! # Guarantees
//!
//! - **Containment is never a string prefix test.** [`is_within`] resolves
//!   both paths (symlinks, `..`, `.`, trailing separators) with
//!   [`canonicalize_lenient`], then compares them component by component
//!   using [`component_key`]. `/Music/Logic2` is *not* within `/Music/Logic`.
//! - **Comparison keys are Unicode-normalised.** macOS hands back names in
//!   whatever normal form they were created in (HFS+ forced NFD; APFS
//!   preserves the input), so `é` may arrive as one code point or two. Every
//!   key is NFC.
//! - **The safety checks are biased toward refusing.** [`is_within`] folds
//!   case on *every* OS, strips the trailing dots/spaces Win32 ignores, and
//!   answers `true` if a path cannot be resolved at all. Every caller uses it
//!   to *refuse* an operation, so a false "inside" costs a refusal, while a
//!   false "outside" could cost a user's project. [`CaseSensitivity`] exists
//!   for callers that want the precise per-volume answer instead.
//! - **Windows verbatim paths (`\\?\C:\…`) compare equal to their plain
//!   spelling** (`C:\…`), and `\\?\UNC\server\share` to `\\server\share`.
//!   [`simplified`] strips the verbatim prefix for display only when that is
//!   lossless ([`dunce`]); I/O keeps the verbatim form so long paths work.
//!
//! [`assert_no_home_paths`] is the privacy check for any text Wit prints or
//! copies (pilot report, `wit report`): it refuses macOS, Linux and Windows
//! home-directory paths in all the spellings a report could plausibly contain.

use std::borrow::Cow;
use std::ffi::{OsStr, OsString};
use std::io;
use std::path::{Component, Path, PathBuf, Prefix};
use unicode_normalization::UnicodeNormalization;

/// NFC-normalise a string (for display and comparison).
pub fn nfc(s: &str) -> String {
    s.nfc().collect()
}

/// NFC-normalise an `OsStr` when it is valid Unicode; return it unchanged
/// otherwise (a non-UTF-8 Linux filename has no normal form to speak of).
pub fn nfc_os(s: &OsStr) -> OsString {
    match s.to_str() {
        Some(text) => OsString::from(nfc(text)),
        None => s.to_os_string(),
    }
}

/// Strip a Windows verbatim prefix (`\\?\`) when doing so doesn't change what
/// the path means; identity on macOS and Linux. For **display and for
/// handing a path to another program** — keep the original for Wit's own I/O.
pub fn simplified(path: &Path) -> &Path {
    dunce::simplified(path)
}

/// The string a musician should see for `path`: verbatim prefix stripped when
/// lossless, NFC-normalised, lossy only for non-Unicode names.
pub fn display(path: &Path) -> String {
    nfc(&simplified(path).to_string_lossy())
}

/// Whether a volume distinguishes `Song` from `song`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CaseSensitivity {
    Sensitive,
    Insensitive,
}

impl CaseSensitivity {
    /// The default for the boot volume of each OS: macOS (APFS/HFS+ default
    /// formatting) and Windows (NTFS) are case-insensitive; Linux (ext4,
    /// btrfs, XFS) is case-sensitive. Individual volumes can differ in every
    /// direction — case-sensitive APFS, `chattr +F` casefolded ext4, exFAT
    /// USB sticks on Linux — use [`probe_case_sensitivity`] for a real answer.
    pub const fn platform_default() -> CaseSensitivity {
        if cfg!(any(target_os = "macos", windows)) {
            CaseSensitivity::Insensitive
        } else {
            CaseSensitivity::Sensitive
        }
    }
}

/// Detect, **without writing anything**, whether the volume holding `dir`
/// is case-insensitive: find an existing entry whose name has letters, look
/// it up with its case flipped, and check whether that lookup lands on the
/// same file. Returns `None` if `dir` has no entry with a flippable name (or
/// can't be read) — callers should fall back to
/// [`CaseSensitivity::platform_default`], or to `Insensitive` when the
/// question is a safety one.
pub fn probe_case_sensitivity(dir: &Path) -> Option<CaseSensitivity> {
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(text) = name.to_str() else { continue };
        let flipped: String = text
            .chars()
            .map(|c| {
                if c.is_ascii_lowercase() {
                    c.to_ascii_uppercase()
                } else {
                    c.to_ascii_lowercase()
                }
            })
            .collect();
        if flipped == text {
            continue;
        }
        let original = entry.path();
        let other = dir.join(&flipped);
        return match std::fs::symlink_metadata(&other) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Some(CaseSensitivity::Sensitive),
            Err(_) => None,
            Ok(_) if same_file(&original, &other) => Some(CaseSensitivity::Insensitive),
            // Both spellings exist as different files: case-sensitive.
            Ok(_) => Some(CaseSensitivity::Sensitive),
        };
    }
    None
}

#[cfg(unix)]
fn same_file(a: &Path, b: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (std::fs::symlink_metadata(a), std::fs::symlink_metadata(b)) {
        (Ok(x), Ok(y)) => x.dev() == y.dev() && x.ino() == y.ino(),
        _ => false,
    }
}

#[cfg(not(unix))]
fn same_file(a: &Path, b: &Path) -> bool {
    // `GetFinalPathNameByHandle` (what `canonicalize` uses on Windows)
    // returns the on-disk spelling, so two names for one file canonicalise
    // identically.
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => false,
    }
}

/// The comparison key for one path component.
///
/// - `Normal` names: NFC; trailing dots and spaces removed (Win32 ignores
///   them, so `Logic.` *is* `Logic` on Windows — folding them everywhere
///   only ever makes containment checks more cautious); on Windows, an
///   NTFS alternate-data-stream suffix (`name:stream`) is cut, because
///   `Logic:x` addresses the `Logic` file itself; with
///   [`CaseSensitivity::Insensitive`], full Unicode case folding (upper
///   then lower, so `ß`/`SS` and `ſ`/`s` fold together).
/// - Windows prefixes: `\\?\C:` and `C:` share a key, as do
///   `\\?\UNC\srv\share` and `\\srv\share`.
pub fn component_key(component: Component<'_>, case: CaseSensitivity) -> String {
    match component {
        Component::Prefix(p) => prefix_key(p.kind()),
        Component::RootDir => "/".to_string(),
        Component::CurDir => ".".to_string(),
        Component::ParentDir => "..".to_string(),
        Component::Normal(name) => normal_key(name, case),
    }
}

fn normal_key(name: &OsStr, case: CaseSensitivity) -> String {
    let mut key = nfc(&name.to_string_lossy());
    let trimmed = key.trim_end_matches(['.', ' ']);
    if !trimmed.is_empty() && trimmed.len() != key.len() {
        key = trimmed.to_string();
    }
    if cfg!(windows) {
        if let Some(i) = key.find(':') {
            if i > 0 {
                key.truncate(i);
            }
        }
    }
    if case == CaseSensitivity::Insensitive {
        key = nfc(&key.to_uppercase().to_lowercase());
    }
    key
}

fn prefix_key(prefix: Prefix<'_>) -> String {
    let fold = |s: &OsStr| nfc(&s.to_string_lossy()).to_lowercase();
    match prefix {
        Prefix::Verbatim(s) => format!("verbatim:{}", fold(s)),
        Prefix::VerbatimUNC(server, share) | Prefix::UNC(server, share) => {
            format!("unc:{}\\{}", fold(server), fold(share))
        }
        Prefix::VerbatimDisk(d) | Prefix::Disk(d) => {
            format!("disk:{}", (d as char).to_ascii_lowercase())
        }
        Prefix::DeviceNS(s) => format!("device:{}", fold(s)),
    }
}

fn keys(path: &Path, case: CaseSensitivity) -> Vec<String> {
    path.components().map(|c| component_key(c, case)).collect()
}

/// Resolve `path` to an absolute, symlink-free spelling, the way the OS
/// would — **even if its tail doesn't exist yet** (a Restores folder that is
/// about to be created, an event path for a file that was just deleted).
///
/// The longest existing ancestor is resolved with [`std::fs::canonicalize`]
/// (so symlinks and `..` get the OS's own semantics: physical on Unix,
/// lexical-then-physical on Windows via [`std::path::absolute`]); the
/// missing tail is appended with `..` removed lexically (it names nothing
/// on disk, so there is no symlink for `..` to traverse). Trailing
/// separators and `.` components disappear. On Windows the result keeps the
/// verbatim `\\?\` prefix `canonicalize` returns.
pub fn canonicalize_lenient(path: &Path) -> io::Result<PathBuf> {
    canonicalize_lenient_depth(path, 0)
}

/// Same bound as Linux's `MAXSYMLINKS`.
const MAX_SYMLINK_HOPS: usize = 40;

fn canonicalize_lenient_depth(path: &Path, hops: usize) -> io::Result<PathBuf> {
    if hops > MAX_SYMLINK_HOPS {
        return Err(io::Error::other("too many levels of symbolic links"));
    }
    let absolute = std::path::absolute(path)?;
    let components: Vec<Component<'_>> = absolute.components().collect();
    for split in (1..=components.len()).rev() {
        if matches!(components[split - 1], Component::Prefix(_)) {
            // A bare `C:` means "the current directory on drive C", not the
            // drive's root; never resolve it on its own.
            continue;
        }
        let head: PathBuf = components[..split].iter().collect();
        let Ok(mut resolved) = std::fs::canonicalize(&head) else {
            // A *dangling* symlink can't be canonicalised, but it still
            // decides where a later write would land: follow it by hand.
            let is_link =
                std::fs::symlink_metadata(&head).is_ok_and(|m| m.file_type().is_symlink());
            if is_link {
                let target = std::fs::read_link(&head)?;
                let mut redirected = if target.is_absolute() {
                    target
                } else {
                    head.parent().unwrap_or(Path::new("")).join(target)
                };
                for component in &components[split..] {
                    redirected.push(component.as_os_str());
                }
                return canonicalize_lenient_depth(&redirected, hops + 1);
            }
            continue;
        };
        let mut pushed = 0usize;
        for component in &components[split..] {
            match component {
                Component::Normal(name) => {
                    resolved.push(name);
                    pushed += 1;
                }
                Component::ParentDir => {
                    if pushed > 0 {
                        resolved.pop();
                        pushed -= 1;
                    } else {
                        // Popping above the canonical base: that base is
                        // symlink-free, so its lexical parent is its real one.
                        resolved.pop();
                    }
                }
                Component::CurDir | Component::RootDir | Component::Prefix(_) => {}
            }
        }
        return Ok(resolved);
    }
    // Nothing on the path exists (not even the root — e.g. an unplugged
    // drive letter). Fall back to a lexical normalisation.
    Ok(lexical_normalize(&absolute))
}

fn lexical_normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// `true` if `child` is `root` or lies beneath it, comparing **already
/// canonical** paths component-by-component with [`component_key`]. No I/O.
/// Use this on paths that came out of [`canonicalize_lenient`] or
/// [`std::fs::canonicalize`], or from a watcher rooted at a canonical path.
pub fn is_within_canonical(child: &Path, root: &Path, case: CaseSensitivity) -> bool {
    let child = keys(child, case);
    let root = keys(root, case);
    !root.is_empty() && root.len() <= child.len() && child[..root.len()] == root[..]
}

/// Conservative containment: `true` if `child` is `root` or lies beneath it
/// once both are resolved (symlinks, `..`, trailing separators, Unicode
/// normal form, verbatim prefixes) — comparing case-insensitively on every
/// OS, and answering `true` if either path can't be resolved at all.
///
/// Use this to **refuse** things. It can say "inside" for two folders a
/// case-sensitive volume keeps apart (`~/Music` vs `~/music`); it cannot
/// say "outside" for a path that really is inside.
pub fn is_within(child: &Path, root: &Path) -> bool {
    is_within_with(child, root, CaseSensitivity::Insensitive)
}

/// [`is_within`] with an explicit case policy. Still resolves both paths
/// and still answers `true` when resolution fails.
pub fn is_within_with(child: &Path, root: &Path, case: CaseSensitivity) -> bool {
    match (canonicalize_lenient(child), canonicalize_lenient(root)) {
        (Ok(child), Ok(root)) => is_within_canonical(&child, &root, case),
        _ => true,
    }
}

/// `true` if either path contains the other (or they're the same) — the
/// relation the Restores folder must never have with a watched root.
/// Conservative in the same way as [`is_within`].
pub fn overlaps(a: &Path, b: &Path) -> bool {
    match (canonicalize_lenient(a), canonicalize_lenient(b)) {
        (Ok(a), Ok(b)) => overlaps_canonical(&a, &b),
        _ => true,
    }
}

/// [`overlaps`] for already-canonical paths (no I/O), case-folded.
pub fn overlaps_canonical(a: &Path, b: &Path) -> bool {
    is_within_canonical(a, b, CaseSensitivity::Insensitive)
        || is_within_canonical(b, a, CaseSensitivity::Insensitive)
}

/// Windows device names that can't be used as a file name stem on Windows
/// (`CON`, `NUL.txt`, `com1.als` …), case-insensitively. Checked on every
/// OS so a restore created on a Mac can still be copied to a PC.
pub fn is_windows_reserved_name(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or(name).trim_end_matches(' ');
    let upper = stem.to_ascii_uppercase();
    let chars: Vec<char> = upper.chars().collect();
    matches!(
        upper.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || ((upper.starts_with("COM") || upper.starts_with("LPT"))
        && chars.len() == 4
        && (chars[3].is_ascii_digit() || matches!(chars[3], '¹' | '²' | '³')))
}

/// Strip a Windows verbatim prefix **from a string**, on any host OS, when
/// that is lossless (the same rules [`dunce`] applies on Windows: under
/// `MAX_PATH`, no reserved names, no trailing dots/spaces, no characters
/// Win32 would reinterpret). `\\?\C:\x` → `C:\x`, `\\?\UNC\srv\share\x` →
/// `\\srv\share\x`. Anything else is returned unchanged. Used by
/// [`crate::reveal`] to build Windows argv on any CI leg.
pub fn simplify_windows_verbatim(path: &str) -> Cow<'_, str> {
    let (candidate, rest) = if let Some(rest) = path.strip_prefix(r"\\?\UNC\") {
        (format!(r"\\{rest}"), rest)
    } else if let Some(rest) = path.strip_prefix(r"\\?\") {
        let bytes = rest.as_bytes();
        if bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && &rest[1..3] == r":\" {
            (rest.to_string(), &rest[3..])
        } else {
            return Cow::Borrowed(path);
        }
    } else {
        return Cow::Borrowed(path);
    };
    let safe = candidate.len() < 260
        && rest.split('\\').all(|part| {
            !part.is_empty()
                && part != "."
                && part != ".."
                && !part.ends_with(['.', ' '])
                && !is_windows_reserved_name(part)
                && !part
                    .chars()
                    .any(|c| c < ' ' || matches!(c, '<' | '>' | ':' | '"' | '/' | '|' | '?' | '*'))
        });
    if safe {
        Cow::Owned(candidate)
    } else {
        Cow::Borrowed(path)
    }
}

/// Refuse `text` if it contains an absolute home-directory path, in any of
/// the spellings Wit's reports could plausibly carry:
///
/// - macOS `/Users/<name>/…` (also inside `file:///Users/…` and
///   `/Volumes/<disk>/Users/…`),
/// - Linux `/home/<name>/…` (also Fedora Atomic's `/var/home/…`),
/// - Windows `C:\Users\<name>\…` on any drive letter, with `\`, `/` or
///   JSON-escaped `\\` separators, verbatim `\\?\C:\Users\…`, admin shares
///   `\\host\c$\Users\…`, and XP-era `C:\Documents and Settings\<name>\…`,
/// - percent-encoded `%2FUsers%2F` / `%2Fhome%2F`.
///
/// Matching is ASCII-case-insensitive (`/users/` on a case-insensitive Mac
/// is still someone's home). Returns the offending text, up to the end of
/// its line, so a failure is actionable.
pub fn assert_no_home_paths(text: &str) -> Result<(), String> {
    match find_home_path(text) {
        None => Ok(()),
        Some(start) => {
            let end = text[start..]
                .find('\n')
                .map(|n| start + n)
                .unwrap_or(text.len());
            Err(format!(
                "found a home-directory path in report output: {:?}",
                &text[start..end]
            ))
        }
    }
}

/// Byte offset of the first home-directory path in `text`, if any. See
/// [`assert_no_home_paths`] for what counts.
pub fn find_home_path(text: &str) -> Option<usize> {
    // ASCII lowercasing keeps every byte offset identical to `text`'s.
    let lower = text.to_ascii_lowercase();
    let mut hits: Vec<usize> = Vec::new();
    for marker in ["/users/", "/home/", "%2fusers%2f", "%2fhome%2f"] {
        if let Some(i) = lower.find(marker) {
            hits.push(i);
        }
    }
    for word in ["users", "documents and settings"] {
        let mut from = 0;
        while let Some(rel) = lower[from..].find(word) {
            let at = from + rel;
            from = at + word.len();
            if let Some(start) = windows_home_start(&lower, at, word.len()) {
                hits.push(start);
                break;
            }
        }
    }
    hits.into_iter().min()
}

/// If `lower[at..at+len]` is the `Users` segment of a Windows home path
/// (`X:<seps>Users<sep>` or `\\host\x$<seps>Users<sep>`), return where the
/// path starts.
fn windows_home_start(lower: &str, at: usize, len: usize) -> Option<usize> {
    let bytes = lower.as_bytes();
    let is_sep = |b: u8| b == b'\\' || b == b'/';
    // After the word: at least one separator.
    if !bytes.get(at + len).copied().is_some_and(is_sep) {
        return None;
    }
    // Before the word: one or more separators …
    let mut i = at;
    let mut seps = 0;
    while i > 0 && is_sep(bytes[i - 1]) {
        i -= 1;
        seps += 1;
    }
    if seps == 0 || i < 2 {
        return None;
    }
    // … preceded by `<letter>:` (a drive) or `<letter>$` (an admin share).
    let (marker, letter) = (bytes[i - 1], bytes[i - 2]);
    if (marker == b':' || marker == b'$') && letter.is_ascii_alphabetic() {
        let mut start = i - 2;
        // Include a verbatim `\\?\` or UNC host prefix in the reported span.
        while start > 0 && !bytes[start - 1].is_ascii_whitespace() && bytes[start - 1] != b'"' {
            start -= 1;
        }
        Some(start)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NFC_E: &str = "\u{e9}"; // é as one code point
    const NFD_E: &str = "e\u{301}"; // é as e + combining acute

    #[test]
    fn nfc_and_nfd_spellings_share_a_key() {
        let a = Path::new("/x").join(format!("Caf{NFC_E}"));
        let b = Path::new("/x").join(format!("Caf{NFD_E}"));
        assert_ne!(a, b, "the two spellings really are different byte strings");
        assert!(is_within_canonical(&a, &b, CaseSensitivity::Sensitive));
        assert!(is_within_canonical(&b, &a, CaseSensitivity::Sensitive));
    }

    #[test]
    fn case_folding_is_only_applied_when_asked() {
        let a = Path::new("/Music/Logic/Song");
        let b = Path::new("/music/logic");
        assert!(is_within_canonical(a, b, CaseSensitivity::Insensitive));
        assert!(!is_within_canonical(a, b, CaseSensitivity::Sensitive));
    }

    #[test]
    fn full_unicode_case_folding() {
        let a = Path::new("/STRASSE");
        let b = Path::new("/straße");
        assert!(is_within_canonical(a, b, CaseSensitivity::Insensitive));
    }

    #[test]
    fn containment_is_by_component_not_string_prefix() {
        let root = Path::new("/Music/Logic");
        assert!(!is_within_canonical(
            Path::new("/Music/Logic2/Song"),
            root,
            CaseSensitivity::Insensitive
        ));
        assert!(is_within_canonical(
            Path::new("/Music/Logic"),
            root,
            CaseSensitivity::Insensitive
        ));
        assert!(!is_within_canonical(
            Path::new("/Music"),
            root,
            CaseSensitivity::Insensitive
        ));
    }

    #[test]
    fn trailing_dots_and_spaces_fold_away() {
        assert!(is_within_canonical(
            Path::new("/Music/Logic. /x"),
            Path::new("/Music/Logic"),
            CaseSensitivity::Insensitive
        ));
    }

    #[test]
    fn empty_root_contains_nothing() {
        assert!(!is_within_canonical(
            Path::new("/a"),
            Path::new(""),
            CaseSensitivity::Insensitive
        ));
    }

    #[test]
    fn lenient_canonicalize_resolves_dot_dot_and_trailing_separators() {
        let dir = tempfile::tempdir().unwrap();
        let real = std::fs::canonicalize(dir.path()).unwrap();
        std::fs::create_dir_all(real.join("a/b")).unwrap();
        let messy = real.join("a").join("b").join("..").join(".").join("b");
        let with_slash = PathBuf::from(format!("{}{}", messy.display(), std::path::MAIN_SEPARATOR));
        assert_eq!(canonicalize_lenient(&with_slash).unwrap(), real.join("a/b"));
        // A missing tail is kept, with its own `..` removed lexically.
        let missing = real.join("a").join("nope").join("..").join("new");
        assert_eq!(
            canonicalize_lenient(&missing).unwrap(),
            real.join("a").join("new")
        );
    }

    #[cfg(unix)]
    #[test]
    fn lenient_canonicalize_follows_symlinks_then_dot_dot_physically() {
        let dir = tempfile::tempdir().unwrap();
        let real = std::fs::canonicalize(dir.path()).unwrap();
        std::fs::create_dir_all(real.join("watched/deep")).unwrap();
        std::fs::create_dir_all(real.join("elsewhere")).unwrap();
        std::os::unix::fs::symlink(real.join("watched/deep"), real.join("elsewhere/link")).unwrap();
        // POSIX: `link/..` is the parent of the link's *target*.
        let sneaky = real.join("elsewhere/link/../x");
        assert_eq!(
            canonicalize_lenient(&sneaky).unwrap(),
            real.join("watched/x")
        );
        assert!(is_within(&sneaky, &real.join("watched")));
    }

    #[cfg(unix)]
    #[test]
    fn lenient_canonicalize_follows_dangling_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        let real = std::fs::canonicalize(dir.path()).unwrap();
        std::fs::create_dir_all(real.join("watched")).unwrap();
        // `restores` points at a folder inside the watched root that doesn't exist yet.
        std::os::unix::fs::symlink(real.join("watched/not-yet"), real.join("restores")).unwrap();
        let resolved = canonicalize_lenient(&real.join("restores/Song")).unwrap();
        assert_eq!(resolved, real.join("watched/not-yet/Song"));
        assert!(is_within(&real.join("restores"), &real.join("watched")));
        // A symlink loop is an error, which the conservative callers treat as "inside".
        std::os::unix::fs::symlink(real.join("loop-b"), real.join("loop-a")).unwrap();
        std::os::unix::fs::symlink(real.join("loop-a"), real.join("loop-b")).unwrap();
        assert!(canonicalize_lenient(&real.join("loop-a/x")).is_err());
        assert!(is_within(&real.join("loop-a/x"), &real.join("watched")));
    }

    #[test]
    fn unresolvable_paths_are_treated_as_inside() {
        assert!(is_within(Path::new(""), Path::new("/")));
        assert!(overlaps(Path::new(""), Path::new("/")));
    }

    #[cfg(windows)]
    #[test]
    fn verbatim_and_plain_windows_spellings_share_a_key() {
        assert!(is_within_canonical(
            Path::new(r"\\?\C:\Users\you\Music\Song"),
            Path::new(r"c:\users\YOU\music"),
            CaseSensitivity::Insensitive
        ));
        assert!(is_within_canonical(
            Path::new(r"\\?\UNC\srv\share\a\b"),
            Path::new(r"\\SRV\share\a"),
            CaseSensitivity::Insensitive
        ));
        // An alternate data stream addresses the file itself.
        assert!(is_within_canonical(
            Path::new(r"C:\Music\Logic:evil"),
            Path::new(r"C:\Music\Logic"),
            CaseSensitivity::Insensitive
        ));
    }

    #[test]
    fn simplify_windows_verbatim_only_when_lossless() {
        assert_eq!(
            simplify_windows_verbatim(r"\\?\C:\Music\a.als"),
            r"C:\Music\a.als"
        );
        assert_eq!(
            simplify_windows_verbatim(r"\\?\UNC\srv\share\a.als"),
            r"\\srv\share\a.als"
        );
        assert_eq!(simplify_windows_verbatim(r"C:\plain"), r"C:\plain");
        // Reserved names and trailing dots only exist under the verbatim prefix.
        assert_eq!(simplify_windows_verbatim(r"\\?\C:\x\CON"), r"\\?\C:\x\CON");
        assert_eq!(simplify_windows_verbatim(r"\\?\C:\x\a."), r"\\?\C:\x\a.");
        let long = format!(r"\\?\C:\{}", "a".repeat(300));
        assert_eq!(simplify_windows_verbatim(&long), long);
    }

    #[test]
    fn windows_reserved_names() {
        for name in ["CON", "con", "Nul.txt", "COM1", "lpt9.als", "AUX ", "COM¹"] {
            assert!(is_windows_reserved_name(name), "{name}");
        }
        for name in ["CONSOLE", "Song", "COM10", "nul-mix", "LPT"] {
            assert!(!is_windows_reserved_name(name), "{name}");
        }
    }

    #[test]
    fn case_probe_matches_the_platform_on_the_temp_volume() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Probe"), b"x").unwrap();
        let probed = probe_case_sensitivity(dir.path()).expect("an entry with letters exists");
        // CI temp volumes use their OS's default formatting.
        assert_eq!(probed, CaseSensitivity::platform_default());
        let empty = tempfile::tempdir().unwrap();
        assert_eq!(probe_case_sensitivity(empty.path()), None);
    }

    #[test]
    fn display_is_nfc() {
        let p = PathBuf::from(format!("/x/Caf{NFD_E}"));
        assert_eq!(display(&p), format!("/x/Caf{NFC_E}"));
    }

    #[test]
    fn home_paths_are_caught_in_every_spelling() {
        let leaks = [
            "found it at /Users/alice/Music/Song.logicx",
            "file:///Users/alice/Music",
            "/users/alice/music (lowercase)",
            "/Volumes/Backup/Users/alice/x",
            "/home/alice/Music",
            "/var/home/alice/Music",
            r"C:\Users\alice\Music",
            r"d:\users\alice\Music",
            "C:/Users/alice/Music",
            r#"{"path": "C:\\Users\\alice\\Music"}"#,
            r"\\?\C:\Users\alice\Music",
            r"\\host\c$\Users\alice\Music",
            r"C:\Documents and Settings\alice\My Documents",
            "%2FUsers%2Falice%2FMusic",
        ];
        for text in leaks {
            assert!(assert_no_home_paths(text).is_err(), "missed: {text}");
        }
    }

    #[test]
    fn clean_text_passes() {
        for text in [
            "clean report, no paths here",
            "~/Music/Logic is the default",
            "3 users, 2 songs",
            r"Users\ on its own",
            "a users/ folder",
            r"C:\Music\Wit Restores",
        ] {
            assert!(assert_no_home_paths(text).is_ok(), "false positive: {text}");
        }
    }

    #[test]
    fn home_path_error_reports_the_line() {
        let err = assert_no_home_paths("ok\nat /home/bob/x y\nmore").unwrap_err();
        assert!(err.contains("/home/bob/x y"), "{err}");
        assert!(!err.contains("more"), "{err}");
    }
}
