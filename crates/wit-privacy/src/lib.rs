//! Privacy checks for any text Wit prints or copies (pilot report,
//! `wit report`, the share page): **no absolute home-directory path may leave
//! the machine.**
//!
//! A zero-dependency leaf crate on purpose: read-only crates (`wit-index`,
//! `wit-story`, the CLI) use it without pulling in `wit-platform`, the crate
//! that can write.

#![forbid(unsafe_code)]

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
