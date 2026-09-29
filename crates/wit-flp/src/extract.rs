//! Whitelist extraction: a walked event stream -> [`Extracted`]. A Rust
//! port of the *decisions* `experiments/flp_parse.py` makes about which
//! ids matter (`EVENT_NAMES`, `TEXT_EVENTS`, `decode_text`'s UTF-16LE-vs-
//! latin-1 heuristic), extended with the specific ids this crate's
//! whitelist asks for that the prototype only ever printed as raw text
//! rows, never named as semantic fields (tempo, mixer insert names,
//! arrangement names).
//!
//! **Every id below except `ARRANGEMENT_NAME` is independently verified
//! this session against real files spanning three FL major versions (10,
//! 20, 25) — not merely carried over from the prototype's `EVENT_NAMES`
//! table, which never decoded any of them as a semantic value.** See each
//! constant's doc comment for the specific evidence.

use crate::frame::{Header, RawEvent};
use std::fmt;

/// `ChanName` — a channel's display name. Verified: decodes real channel
/// names (`"Kick"`, `"808 Kick"`, `"reFX Nexus #3"`) across FL 10/20/25
/// fixtures this session.
const CHAN_NAME: u8 = 192;
/// `PatName` — a pattern's display name. Verified: decodes real pattern
/// names (e.g. `"Choir"`) on an FL 10 fixture this session.
const PAT_NAME: u8 = 193;
/// `Version` — the FL Studio build that wrote the file, e.g. `"25.2.5.5055"`.
/// This is also how [`is_v25_or_later`] decides whether the scalar keystream
/// (issue #7) applies.
const VERSION: u8 = 199;
/// `DefPluginName` — a plugin's default/generic name (e.g. `"Fruity
/// Wrapper"`, `"FLEX"`, `"Sytrus"`). Verified on real files this session.
const DEF_PLUGIN_NAME: u8 = 201;
/// `PluginName` — a plugin's specific instance/preset name (e.g.
/// `"808 Kick"`, `"FLEX Bass"`). Verified on real files this session.
/// [`Extracted::plugin_names`] merges this with [`DEF_PLUGIN_NAME`] — the
/// crate's whitelist asks for "plugin (generator/effect) names" as one
/// category, and real files showed both ids contributing genuinely
/// different plugin names depending on whether a channel wraps a VST
/// (`DefPluginName` = "Fruity Wrapper", the real plugin name is inside the
/// opaque, unread VST chunk) or is an FL-native plugin (`PluginName` =
/// the actual instrument, e.g. "808 Kick").
const PLUGIN_NAME: u8 = 203;
/// `InsertName` — a mixer insert's display name (only present when a user
/// renamed it from FL's default numbered name). Verified: decodes real
/// insert names (`"Dream bell"`, `"REC"`) across two real fixtures this
/// session — this is the "mixer insert names if the prototype has them"
/// the crate's whitelist asks for; `flp_parse.py` never named this id
/// (192-207 render generically as an untyped text row), but it is inside
/// that prototype's own `TEXT_EVENTS` range and decodes cleanly with its
/// same heuristic.
const INSERT_NAME: u8 = 204;
/// `Tempo` — dword, `round(BPM * 1000)`. **Not documented anywhere this
/// crate cites** (absent from `flp_parse.py`'s `EVENT_NAMES`); identified
/// this session by scanning every dword-class id that appears exactly
/// once per file for a value that divides evenly by 1000 into a plausible
/// BPM. On a real FL 20.8.3 fixture, id 156's single occurrence decoded
/// to exactly `130000` -> `130.0` BPM — an exact multiple of 1000 by
/// chance is a low-probability coincidence, which is the evidence this is
/// the right id, not just a plausible-looking number. See
/// [`is_v25_or_later`] for why this id is untrustworthy on v25+.
const TEMPO: u8 = 156;
/// An empirically observed, **undocumented and unverified-beyond-this-
/// session** id that decoded to the literal text `"Arrangement"` on two
/// of three real fixtures checked (present on FL 20/25 files, the two
/// with multi-arrangement support; absent — count zero — on the FL 10
/// fixture, which predates that feature). This is the closest candidate
/// found for the whitelist's "playlist/arrangement names if present";
/// kept separate from the other, better-evidenced ids above so a reader
/// can weigh the confidence differently. `Extracted::arrangement_names`
/// is empty, never wrong, if this guess is off for a given file — it is
/// filtered like any other text event, not assumed authoritative.
const ARRANGEMENT_NAME_GUESS: u8 = 241;

/// FL Studio's own resync-breaking id (see `frame.rs`'s module doc) is
/// never itself a name we'd want to decode as text, but it is inside the
/// dword range and could in principle collide with a text-range id on a
/// future version. It doesn't today (172 < 192), so no special-casing is
/// needed here — noted for whoever adds the next id.
const _RESYNC_EVENT_ID_NOTE: u8 = 172;

/// Decode a NUL-terminated FL text payload — byte-for-byte port of
/// `flp_parse.py`'s `decode_text`. UTF-16LE text always ends with a
/// two-byte NUL terminator (`0x00 0x00`); latin-1 text always ends with a
/// single zero byte, which is why a short latin-1 name like `"Hat\0"`
/// (an even length) is never mistaken for UTF-16 — its last two bytes are
/// `('t', 0x00)`, never `(0x00, 0x00)`.
pub fn decode_text(payload: &[u8]) -> String {
    if payload.is_empty() {
        return String::new();
    }
    if payload.len().is_multiple_of(2) && payload.ends_with(&[0x00, 0x00]) {
        let units: Vec<u16> = payload
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        if let Ok(s) = String::from_utf16(&units) {
            return s.trim_end_matches('\0').to_string();
        }
        // Falls through to latin-1 on an invalid UTF-16 sequence, exactly
        // as the Python original's UnicodeDecodeError fallback does.
    }
    payload
        .iter()
        .map(|&b| b as char) // latin-1: byte value == code point
        .collect::<String>()
        .trim_end_matches('\0')
        .to_string()
}

/// Whether `version` (the raw `Version` text event, e.g. `"25.2.5.5055"`)
/// is FL Studio 25 or later — the boundary
/// [issue #7](https://github.com/sep-lab/Wit/issues/7) names for the
/// unsolved scalar keystream. Parses only the leading major-version
/// component; `None` (never treated as v25+) if it isn't a plain integer,
/// which is the conservative direction — an unparseable version string
/// should not silently suppress a real tempo reading.
pub fn major_version(version: &str) -> Option<u32> {
    version.split('.').next()?.parse().ok()
}

fn is_v25_or_later(version: &str) -> bool {
    major_version(version).is_some_and(|major| major >= 25)
}

/// A tempo reading, honest about the one case it cannot trust.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Tempo {
    /// No `Tempo` event was found at all.
    #[default]
    Unknown,
    Known(f64),
    /// **FL Studio v25+: the scalar (fixed-width) event stream is under an
    /// unsolved, offset-dependent obfuscation keystream**
    /// ([issue #7](https://github.com/sep-lab/Wit/issues/7)). Measured
    /// this session: a real v25.2.5 file's `Tempo` dword decoded to
    /// `252566982` (`252566.982` BPM) — obvious garbage, not a plausible
    /// misread. Rather than surface a wrong number, extraction refuses to
    /// interpret this field at all on v25+ and returns this marker
    /// instead. See [`Extracted::format_status`] for the same refusal
    /// applied to the format as a whole.
    PartialV25ScalarsUnreadable,
}

/// Whether extraction on this file is complete, or was restricted because
/// the file is FL Studio v25+.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormatStatus {
    /// Every whitelisted field this crate models was read normally.
    Complete,
    /// FL Studio v25+ ([issue #7](https://github.com/sep-lab/Wit/issues/7)):
    /// scalar (fixed-width) events are under an unsolved obfuscation
    /// keystream, so extraction is restricted to variable-length (text)
    /// events only — every name field on [`Extracted`] is still read
    /// normally (measured this session: real channel/plugin/insert names
    /// decode cleanly on v25 fixtures), but [`Extracted::tempo`] is
    /// [`Tempo::PartialV25ScalarsUnreadable`] rather than a number that
    /// looks plausible but was never verified.
    PartialV25ScalarsUnreadable,
}

/// Everything this crate's whitelist extracts from one `.flp`: the header
/// fields, the FL Studio version that wrote it, tempo, and every
/// whitelisted name category.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Extracted {
    pub header_format: i16,
    pub channels: u16,
    pub ppq: u16,
    /// The `Version` text event, verbatim (e.g. `"25.2.5.5055"`). `None`
    /// if the file has no `Version` event at all.
    pub fl_version: Option<String>,
    pub format_status: FormatStatus,
    pub tempo: Tempo,
    /// `ChanName` values, in event-stream order (not deduplicated — a
    /// project can genuinely have two channels with the same name, and
    /// that is a fact about the project, not noise to collapse).
    pub channel_names: Vec<String>,
    pub pattern_names: Vec<String>,
    /// `DefPluginName` and `PluginName` merged — see [`PLUGIN_NAME`]'s doc
    /// comment for why both matter.
    pub plugin_names: Vec<String>,
    pub mixer_insert_names: Vec<String>,
    /// Best-effort — see [`ARRANGEMENT_NAME_GUESS`]. Always empty rather
    /// than wrong if the guess doesn't hold for a given file.
    pub arrangement_names: Vec<String>,
}

/// `FormatStatus` has no natural "zero" value — defaulting to `Complete`
/// only exists so `Extracted`'s own `#[derive(Default)]` (used by tests'
/// `..Extracted::default()`) has something to land on. `extract()` always
/// sets this field explicitly from the real version check; nothing here
/// relies on the default outside of test scaffolding.
impl Default for FormatStatus {
    fn default() -> Self {
        FormatStatus::Complete
    }
}

impl fmt::Display for Tempo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Tempo::Unknown => write!(f, "unknown"),
            Tempo::Known(bpm) => write!(f, "{bpm:.3} BPM"),
            Tempo::PartialV25ScalarsUnreadable => {
                write!(f, "partial: v25 scalars unreadable")
            }
        }
    }
}

/// Extract every whitelisted field from an already-walked event stream.
pub fn extract(header: &Header, events: &[RawEvent<'_>]) -> Extracted {
    let fl_version = events
        .iter()
        .find(|e| e.id == VERSION)
        .map(|e| decode_text(e.payload));
    let v25_plus = fl_version.as_deref().is_some_and(is_v25_or_later);

    let mut out = Extracted {
        header_format: header.format,
        channels: header.channels,
        ppq: header.ppq,
        fl_version,
        format_status: if v25_plus {
            FormatStatus::PartialV25ScalarsUnreadable
        } else {
            FormatStatus::Complete
        },
        tempo: Tempo::Unknown,
        ..Extracted::default()
    };

    for event in events {
        match event.id {
            CHAN_NAME => push_text(&mut out.channel_names, event.payload),
            PAT_NAME => push_text(&mut out.pattern_names, event.payload),
            DEF_PLUGIN_NAME | PLUGIN_NAME => push_text(&mut out.plugin_names, event.payload),
            INSERT_NAME => push_text(&mut out.mixer_insert_names, event.payload),
            ARRANGEMENT_NAME_GUESS => push_text(&mut out.arrangement_names, event.payload),
            TEMPO if v25_plus => out.tempo = Tempo::PartialV25ScalarsUnreadable,
            TEMPO if event.payload.len() == 4 => {
                let raw = u32::from_le_bytes(event.payload.try_into().unwrap());
                out.tempo = Tempo::Known(raw as f64 / 1000.0);
            }
            _ => {}
        }
    }
    out
}

fn push_text(into: &mut Vec<String>, payload: &[u8]) {
    let text = decode_text(payload);
    if !text.trim().is_empty() {
        into.push(text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::parse_container;

    fn header_bytes(format: i16, channels: u16, ppq: u16) -> Vec<u8> {
        let mut body = Vec::new();
        body.extend_from_slice(&format.to_le_bytes());
        body.extend_from_slice(&channels.to_le_bytes());
        body.extend_from_slice(&ppq.to_le_bytes());
        let mut out = Vec::new();
        out.extend_from_slice(crate::frame::HEADER_MAGIC);
        out.extend_from_slice(&(body.len() as u32).to_le_bytes());
        out.extend_from_slice(&body);
        out
    }

    fn latin1_text(id: u8, s: &str) -> Vec<u8> {
        let mut payload = s.as_bytes().to_vec();
        payload.push(0);
        var_event(id, &payload)
    }

    fn utf16_text(id: u8, s: &str) -> Vec<u8> {
        let mut payload: Vec<u8> = s.encode_utf16().flat_map(u16::to_le_bytes).collect();
        payload.extend_from_slice(&[0, 0]);
        var_event(id, &payload)
    }

    fn var_event(id: u8, payload: &[u8]) -> Vec<u8> {
        let mut out = vec![id];
        out.extend_from_slice(&encode_varint(payload.len() as u64));
        out.extend_from_slice(payload);
        out
    }

    fn encode_varint(mut n: u64) -> Vec<u8> {
        let mut out = Vec::new();
        loop {
            let byte = (n & 0x7f) as u8;
            n >>= 7;
            if n != 0 {
                out.push(byte | 0x80);
            } else {
                out.push(byte);
                return out;
            }
        }
    }

    fn dword_event(id: u8, value: u32) -> Vec<u8> {
        let mut out = vec![id];
        out.extend_from_slice(&value.to_le_bytes());
        out
    }

    fn container(event_bytes: Vec<Vec<u8>>) -> Vec<u8> {
        let events: Vec<u8> = event_bytes.into_iter().flatten().collect();
        let mut out = header_bytes(0, 1, 96);
        out.extend_from_slice(crate::frame::DATA_MAGIC);
        out.extend_from_slice(&(events.len() as u32).to_le_bytes());
        out.extend_from_slice(&events);
        out
    }

    fn extracted_of(event_bytes: Vec<Vec<u8>>) -> Extracted {
        let data = container(event_bytes);
        let (header, events) = parse_container(&data).unwrap();
        extract(&header, &events)
    }

    #[test]
    fn channel_and_pattern_names_are_extracted() {
        let e = extracted_of(vec![
            latin1_text(CHAN_NAME, "Kick"),
            latin1_text(PAT_NAME, "Choir"),
        ]);
        assert_eq!(e.channel_names, vec!["Kick".to_string()]);
        assert_eq!(e.pattern_names, vec!["Choir".to_string()]);
    }

    #[test]
    fn def_plugin_name_and_plugin_name_both_feed_plugin_names() {
        let e = extracted_of(vec![
            latin1_text(DEF_PLUGIN_NAME, "Fruity Wrapper"),
            latin1_text(PLUGIN_NAME, "808 Kick"),
        ]);
        assert_eq!(
            e.plugin_names,
            vec!["Fruity Wrapper".to_string(), "808 Kick".to_string()]
        );
    }

    #[test]
    fn insert_names_are_extracted() {
        let e = extracted_of(vec![latin1_text(INSERT_NAME, "Dream bell")]);
        assert_eq!(e.mixer_insert_names, vec!["Dream bell".to_string()]);
    }

    #[test]
    fn arrangement_name_guess_is_extracted_but_kept_separate() {
        let e = extracted_of(vec![latin1_text(ARRANGEMENT_NAME_GUESS, "Arrangement")]);
        assert_eq!(e.arrangement_names, vec!["Arrangement".to_string()]);
    }

    #[test]
    fn blank_text_events_are_dropped() {
        let e = extracted_of(vec![
            latin1_text(CHAN_NAME, ""),
            latin1_text(CHAN_NAME, "   "),
            latin1_text(CHAN_NAME, "Real"),
        ]);
        assert_eq!(e.channel_names, vec!["Real".to_string()]);
    }

    #[test]
    fn tempo_decodes_as_bpm_times_1000() {
        let e = extracted_of(vec![
            latin1_text(VERSION, "20.8.3.2304"),
            dword_event(TEMPO, 130_000),
        ]);
        assert_eq!(e.tempo, Tempo::Known(130.0));
        assert_eq!(e.format_status, FormatStatus::Complete);
    }

    #[test]
    fn no_tempo_event_is_unknown_not_zero() {
        let e = extracted_of(vec![latin1_text(VERSION, "20.8.3.2304")]);
        assert_eq!(e.tempo, Tempo::Unknown);
    }

    #[test]
    fn v25_marks_tempo_partial_instead_of_a_wrong_number() {
        // Measured this session: a real v25 Tempo dword decoded to
        // 252566982 -- obvious garbage. Whatever value is present, v25
        // must never surface it as Tempo::Known.
        let e = extracted_of(vec![
            latin1_text(VERSION, "25.2.5.5055"),
            dword_event(TEMPO, 252_566_982),
        ]);
        assert_eq!(e.tempo, Tempo::PartialV25ScalarsUnreadable);
        assert_eq!(e.format_status, FormatStatus::PartialV25ScalarsUnreadable);
    }

    #[test]
    fn v25_still_extracts_names_normally() {
        // Measured this session: real channel/plugin names decode cleanly
        // on v25 fixtures -- only scalar (fixed-width) events are unsafe.
        let e = extracted_of(vec![
            latin1_text(VERSION, "25.2.5.5055"),
            latin1_text(CHAN_NAME, "808 Kick"),
            latin1_text(PLUGIN_NAME, "FLEX Bass"),
        ]);
        assert_eq!(e.channel_names, vec!["808 Kick".to_string()]);
        assert_eq!(e.plugin_names, vec!["FLEX Bass".to_string()]);
    }

    #[test]
    fn pre_v25_major_version_boundary() {
        assert!(!is_v25_or_later("24.9.9.9999"));
        assert!(is_v25_or_later("25.0.0.0"));
        assert!(is_v25_or_later("26.1.0.0"));
    }

    #[test]
    fn unparseable_version_is_never_treated_as_v25() {
        assert!(!is_v25_or_later("not-a-version"));
        assert_eq!(major_version("not-a-version"), None);
    }

    #[test]
    fn utf16_text_decodes_including_non_latin_scripts() {
        // AGENTS.md/task: a Persian name must survive the UTF-16LE path.
        let e = extracted_of(vec![utf16_text(CHAN_NAME, "آواز")]);
        assert_eq!(e.channel_names, vec!["آواز".to_string()]);
    }

    #[test]
    fn latin1_three_letter_names_are_not_mistaken_for_utf16() {
        for name in ["Hat", "Kik", "Bss"] {
            let e = extracted_of(vec![latin1_text(CHAN_NAME, name)]);
            assert_eq!(e.channel_names, vec![name.to_string()]);
        }
    }

    #[test]
    fn header_fields_are_carried_through() {
        let data = header_bytes(-1, 18, 96);
        let mut full = data;
        full.extend_from_slice(crate::frame::DATA_MAGIC);
        full.extend_from_slice(&0u32.to_le_bytes());
        let (header, events) = parse_container(&full).unwrap();
        let e = extract(&header, &events);
        assert_eq!((e.header_format, e.channels, e.ppq), (-1, 18, 96));
    }
}
