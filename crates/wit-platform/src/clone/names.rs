//! Turning a song name into a file name that is safe, honest and fits on
//! every filesystem Wit targets.
//!
//! - **Safe:** no separators, `..`, control characters, leading dots,
//!   Windows-reserved names or characters, or trailing dots/spaces.
//! - **Honest:** line/paragraph separators (U+2028/U+2029), bidirectional
//!   overrides and isolates (U+202A–U+202E,
//!   U+2066–U+2069, U+200E/F, U+061C) and zero-width/format characters
//!   (U+200B–U+200D, U+2060–U+2064, U+FEFF, U+00AD, U+180E, U+FFF9–U+FFFB,
//!   tag characters) are removed, so a name can't display as something it
//!   isn't (`Song<U+202E>3pm.als` rendering as `Songsla.mp3`).
//! - **Fits:** at most 255 UTF-8 bytes (APFS, ext4) **and** 255 UTF-16 code
//!   units after canonical decomposition (HFS+ stores NFD; NTFS counts
//!   UTF-16), for the whole name including ` (999)` and the extension. The
//!   song part is shortened first, so the date survives.

use crate::paths;
use unicode_normalization::UnicodeNormalization;

/// The longest suffix a fresh name can get: ` (999)`.
pub const WORST_SUFFIX: &str = " (999)";
const MAX_BYTES: usize = 255;
const MAX_UTF16: usize = 255;
/// Cap for the date part, so a hostile "date" can't crowd out the song.
const MAX_DATE_BYTES: usize = 64;

fn is_invisible_or_bidi(c: char) -> bool {
    matches!(
        c,
        '\u{00AD}'
            | '\u{061C}'
            | '\u{180E}'
            | '\u{200B}'..='\u{200F}'
            | '\u{2028}'..='\u{202E}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{206F}'
            | '\u{FEFF}'
            | '\u{FFF9}'..='\u{FFFB}'
            | '\u{E0000}'..='\u{E007F}'
    )
}

/// Make `raw` safe as (part of) one file name. Returns `fallback` if nothing
/// usable is left.
pub fn sanitize_component(raw: &str, fallback: &str) -> String {
    let normalized = paths::nfc(raw);
    let mut out = String::with_capacity(normalized.len());
    for c in normalized.chars() {
        match c {
            '/' | '\\' | ':' | '|' => out.push('-'),
            '*' | '?' | '"' | '<' | '>' => out.push('_'),
            c if c.is_control() || is_invisible_or_bidi(c) => {}
            c => out.push(c),
        }
    }
    let mut name = out
        .trim()
        .trim_start_matches(['.', ' '])
        .trim_end_matches(['.', ' '])
        .to_string();
    if name.is_empty() {
        return fallback.to_string();
    }
    if paths::is_windows_reserved_name(&name) {
        name.insert(0, '_');
    }
    name
}

fn utf16_len_nfd(s: &str) -> usize {
    s.nfd().map(char::len_utf16).sum()
}

/// Whether `name` fits every target filesystem's limits.
#[cfg(test)]
pub fn fits(name: &str) -> bool {
    name.len() <= MAX_BYTES && utf16_len_nfd(name) <= MAX_UTF16
}

/// Longest prefix of `s` (on a `char` boundary) whose byte length and NFD
/// UTF-16 length stay within the given budgets.
fn truncate_to(s: &str, max_bytes: usize, max_utf16: usize) -> String {
    let (mut bytes, mut units) = (0usize, 0usize);
    let mut out = String::new();
    for c in s.chars() {
        let b = c.len_utf8();
        let u: usize = std::iter::once(c).nfd().map(char::len_utf16).sum();
        if bytes + b > max_bytes || units + u > max_utf16 {
            break;
        }
        bytes += b;
        units += u;
        out.push(c);
    }
    // Truncation must not leave a trailing dot/space (Windows strips them).
    out.trim_end_matches(['.', ' ']).to_string()
}

/// Build the base of a fresh name, `<song> — <date>`, from already
/// sanitised parts, shortening the song so that `base + " (999)" + .ext`
/// fits every filesystem.
pub fn fit_base(song: &str, date: &str, extension: Option<&str>) -> String {
    let date = truncate_to(date, MAX_DATE_BYTES, MAX_DATE_BYTES);
    let tail = match (date.is_empty(), extension) {
        (true, None) => WORST_SUFFIX.to_string(),
        (true, Some(ext)) => format!("{WORST_SUFFIX}.{ext}"),
        (false, None) => format!(" — {date}{WORST_SUFFIX}"),
        (false, Some(ext)) => format!(" — {date}{WORST_SUFFIX}.{ext}"),
    };
    let budget_bytes = MAX_BYTES.saturating_sub(tail.len());
    let budget_units = MAX_UTF16.saturating_sub(utf16_len_nfd(&tail));
    let mut song = truncate_to(song, budget_bytes, budget_units);
    if song.is_empty() {
        song = "Untitled".to_string();
    }
    if date.is_empty() {
        song
    } else {
        format!("{song} — {date}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    }

    #[test]
    fn bidi_and_invisible_characters_are_removed() {
        // "Song<RLO>3pm.als" would render as "Songsla.mp3".
        assert_eq!(
            sanitize_component("Song\u{202E}3pm.als", "U"),
            "Song3pm.als"
        );
        assert_eq!(
            sanitize_component("A\u{200B}B\u{2066}C\u{FEFF}D\u{00AD}E", "U"),
            "ABCDE"
        );
        assert_eq!(sanitize_component("\u{200B}\u{202E}", "U"), "U");
        assert_eq!(sanitize_component("A\u{2028}B\u{2029}C", "U"), "ABC");
    }

    #[test]
    fn long_names_fit_bytes_and_utf16_on_every_filesystem() {
        for song in [
            "x".repeat(500),
            "🎹".repeat(200),       // 4 bytes / 2 UTF-16 units each
            "音楽".repeat(150),     // 3 bytes each
            "e\u{301}".repeat(200), // NFC é = 2 bytes, NFD = 2 units
            "Å".repeat(300),        // NFC 2 bytes, NFD 2 units
        ] {
            let song = sanitize_component(&song, "U");
            let base = fit_base(&song, "2026-09-29", Some("logicx"));
            let worst = format!("{base}{WORST_SUFFIX}.logicx");
            assert!(
                fits(&worst),
                "{} bytes / {} units",
                worst.len(),
                utf16_len_nfd(&worst)
            );
            assert!(base.ends_with(" — 2026-09-29"), "the date survives: {base}");
            assert!(base.chars().count() > 10, "the song keeps what fits");
        }
        let hostile_date = "9".repeat(1_000);
        let base = fit_base("Song", &hostile_date, None);
        assert!(fits(&format!("{base}{WORST_SUFFIX}")));
        assert!(base.starts_with("Song — "));
    }
}
