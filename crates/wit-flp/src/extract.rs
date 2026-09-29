//! Whitelist extraction: a walked event stream -> [`Extracted`]. A Rust
//! port of the *decisions* `experiments/flp_parse.py` makes about which
//! ids matter (`EVENT_NAMES`, `TEXT_EVENTS`, `decode_text`'s UTF-16LE-vs-
//! latin-1 heuristic), extended with the specific ids this crate's
//! whitelist asks for that the prototype only ever printed as a raw,
//! untyped text row (tempo, and a structured read of mixer insert /
//! arrangement names rather than a generic listing).
//!
//! **The channel-name scheme changed between FL Studio eras — measured,
//! not assumed.** A first pass of this module read every channel's name
//! from a single id (192, `ChanName`) on every version, because that is
//! what one real FL 10.0.0 file and `flp_parse.py`'s own `EVENT_NAMES`
//! table suggested. A wider real-material check (spanning FL 8.5 through
//! FL 25.2.5, including this crate's own `WIT_FIXTURES` corpus plus FL
//! Studio 20's bundled demo projects, read-only, off-repository) showed
//! that was wrong for every version at or after FL 12: id 192 either
//! never appears (FL 12–24, checked on dozens of real files) or holds
//! something that is not a channel name at all — the literal string
//! `"FL Studio 25.2.5.5055.5055"` on every one of 5 real v25 files
//! checked. [`extract`] therefore branches on the FL major version — see
//! [`NEW_CHANNEL_SCHEME_MIN_VERSION`] — and the two eras get two different,
//! independently verified extraction paths. See each constant's doc
//! comment for the specific evidence.

use crate::frame::{Header, RawEvent};
use std::fmt;

/// `NewChan` — marks the start of a new channel in the event stream.
/// Present in every FL version checked; only meaningful (as a block
/// boundary) under the new channel-name scheme — see
/// [`NEW_CHANNEL_SCHEME_MIN_VERSION`].
const NEW_CHAN: u8 = 64;
/// `ChanName` — a channel's display name, but **only on FL < 12**.
/// Verified on real files: decodes real names (`"Kick"`, `"Xtra Bass"`,
/// `"Clap"`, ...) on every real fixture checked from FL 8.5.0 through FL
/// 11.1.0. **Never read as a channel name on FL >= 12 or v25** — see
/// [`NEW_CHANNEL_SCHEME_MIN_VERSION`]'s doc for why: it is absent outright
/// on FL 12–24, and something else entirely (the FL Studio build string)
/// on v25.
const CHAN_NAME: u8 = 192;
/// `PatName` — a pattern's display name. Verified: decodes real pattern
/// names (e.g. `"Choir"`) on an FL 10 fixture this session. Unaffected by
/// the channel-name scheme change (patterns are a separate object from
/// channels).
const PAT_NAME: u8 = 193;
/// `Version` — the FL Studio build that wrote the file, e.g. `"25.2.5.5055"`.
/// This is also how [`major_version`] decides which channel-name scheme
/// applies and whether the v25 scalar keystream (issue #7) applies.
const VERSION: u8 = 199;
/// `DefPluginName` — the underlying plugin/generator *type*'s own name
/// (e.g. `"Fruity Wrapper"`, `"FPC"`, `"Harmor"`, `"Sytrus"`). Verified on
/// real files across every FL era checked (8.5 through 25.2.5): this
/// field's meaning did not change, only how it is scoped to a channel
/// did (see [`NEW_CHANNEL_SCHEME_MIN_VERSION`]). Empty for a plain Sampler
/// channel with no separate plugin/generator — measured on real FL 20/25
/// files: a channel using nothing but FL's built-in sampler carries a
/// zero-length `DefPluginName`, so an empty payload here is a real "no
/// plugin" fact, not a decode failure, and [`push_text`]'s blank-drop
/// behaviour is exactly what is wanted for it.
const DEF_PLUGIN_NAME: u8 = 201;
/// `PluginName` — **on FL >= 11.5, this is the channel's own display
/// name** (e.g. `"808 Kick"`, `"FLEX Bass"`, `"Drumpad"`, `"clapBuildup"`),
/// appearing once per channel immediately after that channel's
/// [`NEW_CHAN`]/[`DEF_PLUGIN_NAME`] pair — verified by cross-checking the
/// count of "first `PluginName` after each `NewChan`" against the file's
/// own declared channel count (`FLhd.channels`) on 46 real FL 11.5–20
/// files plus all 5 real v25 files in this crate's `WIT_FIXTURES` corpus:
/// an exact match on 50 of those 51 files. The one exception (a real FL
/// 20.0.3 file) is short by 2 of 74 channels — both apparently have a
/// genuinely empty `PluginName` in that file, which this crate's blank-
/// text drop (see [`push_text`]) correctly omits rather than reports as
/// an empty string; this is an absence, not a wrong attribution. **Below
/// FL 11.5 this id is never read at all** in the current extraction —
/// real files from that era do not show it appearing tightly bound to
/// `NewChan` the way FL >= 11.5 does, so treating it as a channel name
/// there is unverified; genuine pre-11.5 channel names come from
/// [`CHAN_NAME`] instead.
const PLUGIN_NAME: u8 = 203;
/// The FL (major, minor) version at and after which channel names are
/// read from [`PLUGIN_NAME`] inside a [`NEW_CHAN`] block, and
/// [`CHAN_NAME`] is never read as a channel name at all. **Measured
/// boundary, not a documented one, and narrower than an earlier pass of
/// this crate assumed.** A first check (against 42 real FL 12–20 files
/// plus 5 v25 files) set this at major version 12, reasoning that
/// FL 11.1.0 still had real channel names on id 192. A wider check (61
/// real files spanning FL 8.5–20.8) found the actual cutover is earlier:
/// FL 11.5.14 and 11.5.16 *already* use the `NewChan`-then-`PluginName`
/// pattern, with an exact match against their own declared channel
/// counts, and *zero* occurrences of id 192 — while FL 11.1.0 still has
/// real names on id 192 and no such pattern. **FL 11.2–11.4 was never
/// observed** (no fixture available in that exact range), so `(11, 5)` is
/// a boundary chosen to match that narrower gap conservatively — a file
/// reporting itself as FL 11.2–11.4 falls back to the pre-11.5
/// (`ChanName`) path here, which is untested for that specific range
/// rather than wrong by measurement.
const NEW_CHANNEL_SCHEME_MIN_VERSION: (u32, u32) = (11, 5);
/// `InsertName` — a mixer insert's display name (only present when a user
/// renamed it from FL's default numbered name). Verified: decodes real
/// insert names (`"Dream bell"`, `"REC"`) on two real pre-v25 fixtures
/// this session. `flp_parse.py`'s own `EVENT_NAMES` table already names
/// this id `"InsertName"` — the prototype never *extracts* it as a
/// structured field (192–207 all render as one generic, unlabelled text
/// row in its output), but the id itself is not new information here.
/// **Unverified on v25**: id 204 occurs zero times across all 5 real v25
/// fixtures in this crate's `WIT_FIXTURES` corpus (none of those projects
/// renamed an insert from its default name), so whether this id still
/// means the same thing on v25 has not actually been observed either way.
const INSERT_NAME: u8 = 204;
/// `Tempo` — dword, `round(BPM * 1000)`. **Not documented anywhere this
/// crate cites** (absent from `flp_parse.py`'s `EVENT_NAMES`); identified
/// this session by scanning every dword-class id that appears exactly
/// once per file for a value that divides evenly by 1000 into a plausible
/// BPM. On a real FL 20.8.3 fixture, id 156's single occurrence decoded
/// to exactly `130000` -> `130.0` BPM — an exact multiple of 1000 by
/// chance is a low-probability coincidence, which is the evidence this is
/// the right id, not just a plausible-looking number. **Absent on the
/// real FL 10.0.0 fixture checked** (id 156 occurs zero times there) —
/// tempo reads as [`Tempo::Unknown`] on that file, not a wrong number;
/// this crate does not have a fixture old enough to say exactly which
/// pre-11 versions have it, if any. See [`is_v25_or_later`] for why this
/// id is untrustworthy on v25+.
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

/// The leading major-version component of `version` (e.g. `"25.2.5.5055"`
/// -> `Some(25)`), or `None` if it isn't a plain integer. `None` is the
/// conservative direction everywhere this is used: an unparseable version
/// string falls back to the pre-FL-12 extraction path and never silently
/// suppresses a real tempo reading.
pub fn major_version(version: &str) -> Option<u32> {
    version.split('.').next()?.parse().ok()
}

/// Whether `version` is FL Studio 25 or later — the boundary
/// [issue #7](https://github.com/sep-lab/Wit/issues/7) names for the
/// unsolved scalar keystream.
fn is_v25_or_later(version: &str) -> bool {
    major_version(version).is_some_and(|major| major >= 25)
}

/// The leading `(major, minor)` version components of `version` (e.g.
/// `"11.5.14"` -> `Some((11, 5))`), or `None` if the major component isn't
/// a plain integer. A missing or unparseable minor component defaults to
/// `0` rather than failing the whole parse — real FL version strings
/// always have one, but a truncated or hand-written one shouldn't be
/// treated worse than "assume the oldest sub-version of that major".
fn major_minor_version(version: &str) -> Option<(u32, u32)> {
    let mut parts = version.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().and_then(|m| m.parse().ok()).unwrap_or(0);
    Some((major, minor))
}

/// Whether `version` uses the new channel-name scheme — see
/// [`NEW_CHANNEL_SCHEME_MIN_VERSION`].
fn uses_new_channel_scheme(version: &str) -> bool {
    major_minor_version(version).is_some_and(|v| v >= NEW_CHANNEL_SCHEME_MIN_VERSION)
}

/// A tempo reading, honest about the one case it cannot trust.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Tempo {
    /// No `Tempo` event was found at all — either a version old enough
    /// not to have one (measured: the real FL 10.0.0 fixture checked has
    /// none), or a genuinely unreadable file.
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
    /// events only. Channel and plugin names are still read normally
    /// (measured this session: real channel/plugin names decode cleanly
    /// and match the file's own declared channel count on all 5 v25
    /// fixtures checked) — but [`Extracted::mixer_insert_names`] is
    /// **unverified rather than confirmed** on v25 (id 204 never occurred
    /// on any of those 5 fixtures, so this is an absence of evidence, not
    /// evidence of correctness), and [`Extracted::tempo`] is
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
    /// One name per channel, in channel order. Sourced from [`CHAN_NAME`]
    /// on FL < 12, or from the first [`PLUGIN_NAME`] inside each
    /// [`NEW_CHAN`] block on FL >= 12 — see
    /// [`NEW_CHANNEL_SCHEME_MIN_VERSION`]. Not deduplicated — a project can
    /// genuinely have two channels with the same name, and that is a fact
    /// about the project, not noise to collapse.
    pub channel_names: Vec<String>,
    pub pattern_names: Vec<String>,
    /// The underlying plugin/generator type's own name
    /// ([`DEF_PLUGIN_NAME`]) for every channel that has one — empty
    /// (Sampler-only) channels contribute nothing here, per
    /// [`DEF_PLUGIN_NAME`]'s doc comment. **No longer merged with
    /// [`PLUGIN_NAME`]** (a change from this crate's first pass): on FL
    /// major version 12 and above, [`PLUGIN_NAME`] is the channel's own
    /// display name, not a second plugin name, and folding it in here
    /// doubled every channel-name change into a spurious pair of "plugin
    /// added/removed" lines.
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

/// Per-channel extraction state, reset at every [`NEW_CHAN`] event. Only
/// meaningful under the new (FL >= 12) channel-name scheme; unused
/// otherwise.
#[derive(Default)]
struct ChannelBlockState {
    in_block: bool,
    def_plugin_name_taken: bool,
    plugin_name_taken: bool,
}

/// Extract every whitelisted field from an already-walked event stream.
pub fn extract(header: &Header, events: &[RawEvent<'_>]) -> Extracted {
    let fl_version = events
        .iter()
        .find(|e| e.id == VERSION)
        .map(|e| decode_text(e.payload));
    let v25_plus = fl_version.as_deref().is_some_and(is_v25_or_later);
    let new_channel_scheme = fl_version.as_deref().is_some_and(uses_new_channel_scheme);

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

    let mut block = ChannelBlockState::default();

    for event in events {
        match event.id {
            NEW_CHAN if new_channel_scheme => {
                block = ChannelBlockState {
                    in_block: true,
                    ..ChannelBlockState::default()
                };
            }
            CHAN_NAME if !new_channel_scheme => {
                push_text(&mut out.channel_names, event.payload);
            }
            DEF_PLUGIN_NAME if new_channel_scheme => {
                if block.in_block && !block.def_plugin_name_taken {
                    block.def_plugin_name_taken = true;
                    push_text(&mut out.plugin_names, event.payload);
                }
            }
            DEF_PLUGIN_NAME => {
                // Pre-FL-12: DefPluginName always names a plugin, with no
                // channel-display-name role to disambiguate from.
                push_text(&mut out.plugin_names, event.payload);
            }
            PLUGIN_NAME if new_channel_scheme => {
                if block.in_block && !block.plugin_name_taken {
                    block.plugin_name_taken = true;
                    push_text(&mut out.channel_names, event.payload);
                }
            }
            // Pre-FL-12: PluginName's real-file behaviour was never
            // verified as either a channel name or a plugin name (see
            // PLUGIN_NAME's doc comment) — read nothing from it rather
            // than guess.
            PAT_NAME => push_text(&mut out.pattern_names, event.payload),
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

    // Literal byte ids throughout this test module, not the crate's own
    // constants (192, 193, 199, 201, 203, 204, 241, 64) — pinning the
    // wire format independently of any future rename/typo in extract.rs
    // itself, matching the reviewer note that these must be pinned by
    // literal bytes.

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

    fn word_event(id: u8, value: u16) -> Vec<u8> {
        let mut out = vec![id];
        out.extend_from_slice(&value.to_le_bytes());
        out
    }

    /// A real FL >= 12 channel block, byte for byte as measured on real
    /// files this session: `NewChan` (64, word), then `DefPluginName`
    /// (201, empty for a plain Sampler channel), then `PluginName` (203,
    /// the channel's own display name).
    fn new_scheme_channel(display_name: &str) -> Vec<u8> {
        let mut out = word_event(64, 0);
        out.extend(latin1_text(201, ""));
        out.extend(latin1_text(203, display_name));
        out
    }

    fn new_scheme_channel_with_plugin(display_name: &str, plugin: &str) -> Vec<u8> {
        let mut out = word_event(64, 0);
        out.extend(latin1_text(201, plugin));
        out.extend(latin1_text(203, display_name));
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

    // ---- pre-FL-12 (id 192) path ------------------------------------- //

    #[test]
    fn pre_fl12_channel_and_pattern_names_are_extracted_from_192_and_193() {
        let e = extracted_of(vec![
            latin1_text(199, "10.0.0"),
            latin1_text(192, "Kick"),
            latin1_text(193, "Choir"),
        ]);
        assert_eq!(e.channel_names, vec!["Kick".to_string()]);
        assert_eq!(e.pattern_names, vec!["Choir".to_string()]);
    }

    #[test]
    fn pre_fl12_def_plugin_name_feeds_plugin_names() {
        let e = extracted_of(vec![
            latin1_text(199, "10.0.0"),
            latin1_text(201, "Fruity Wrapper"),
        ]);
        assert_eq!(e.plugin_names, vec!["Fruity Wrapper".to_string()]);
    }

    #[test]
    fn pre_fl12_plugin_name_203_is_never_read_as_a_channel_or_plugin_name() {
        // Real pre-FL-12 files do not show 203 tightly bound to NewChan
        // the way FL >= 12 does, and its actual role there was never
        // verified -- read nothing rather than guess.
        let e = extracted_of(vec![latin1_text(199, "10.0.0"), latin1_text(203, "Sytrus")]);
        assert!(e.channel_names.is_empty());
        assert!(e.plugin_names.is_empty());
    }

    #[test]
    fn no_version_event_falls_back_to_the_pre_fl12_path() {
        let e = extracted_of(vec![latin1_text(192, "Kick")]);
        assert_eq!(e.channel_names, vec!["Kick".to_string()]);
    }

    // ---- FL >= 12 (NewChan/201/203 block) path ----------------------- //

    #[test]
    fn fl12_plus_reads_channel_names_from_203_inside_a_newchan_block() {
        let mut events = vec![latin1_text(199, "20.8.3.2304")];
        events.push(new_scheme_channel("808 Kick"));
        events.push(new_scheme_channel("FLEX Bass"));
        let e = extracted_of(events);
        assert_eq!(
            e.channel_names,
            vec!["808 Kick".to_string(), "FLEX Bass".to_string()]
        );
    }

    #[test]
    fn fl_11_5_also_uses_the_new_channel_scheme() {
        // Regression: real FL 11.5.14/11.5.16 fixtures use the new
        // scheme too, not just FL 12+ -- this must not regress to zero
        // channel names for that version.
        let mut events = vec![latin1_text(199, "11.5.14")];
        events.push(new_scheme_channel("Ghost Kick"));
        let e = extracted_of(events);
        assert_eq!(e.channel_names, vec!["Ghost Kick".to_string()]);
    }

    #[test]
    fn fl12_plus_never_reads_192_as_a_channel_name() {
        // Measured this session: on v25, id 192 holds the FL Studio build
        // string, not a channel name; on FL 12-24 it does not appear at
        // all. Either way it must never contribute to channel_names here.
        let mut events = vec![latin1_text(199, "25.2.5.5055")];
        events.push(latin1_text(192, "FL Studio 25.2.5.5055.5055"));
        events.push(new_scheme_channel("808 Kick"));
        let e = extracted_of(events);
        assert_eq!(e.channel_names, vec!["808 Kick".to_string()]);
    }

    #[test]
    fn fl12_plus_takes_plugin_name_only_from_non_empty_def_plugin_name() {
        let mut events = vec![latin1_text(199, "20.8.3.2304")];
        events.push(new_scheme_channel("clapBuildup")); // Sampler channel: empty 201
        events.push(new_scheme_channel_with_plugin("Drums", "FPC"));
        let e = extracted_of(events);
        assert_eq!(
            e.channel_names,
            vec!["clapBuildup".to_string(), "Drums".to_string()]
        );
        assert_eq!(e.plugin_names, vec!["FPC".to_string()]);
    }

    #[test]
    fn fl12_plus_only_takes_the_first_201_and_203_per_channel_block() {
        // A device chain can carry more than one 201/203 per channel (an
        // effect's own name, for instance) -- only the first of each,
        // right after NewChan, is the channel's own identity.
        let mut events = vec![latin1_text(199, "20.8.3.2304"), word_event(64, 0)];
        events.push(latin1_text(201, "Harmor"));
        events.push(latin1_text(203, "Pluck 1"));
        events.push(latin1_text(201, "Fruity Reeverb")); // a device further down the chain
        events.push(latin1_text(203, "Reverb Slot"));
        let e = extracted_of(events);
        assert_eq!(e.channel_names, vec!["Pluck 1".to_string()]);
        assert_eq!(e.plugin_names, vec!["Harmor".to_string()]);
    }

    #[test]
    fn a_channel_add_reports_one_channel_not_a_doubled_plugin_pair() {
        // The bug this fix closes: comparing two extractions where one
        // channel was added must show exactly one new channel name, not
        // two "plugin" entries (one for 201, one for 203).
        let mut old_events = vec![latin1_text(199, "25.2.5.5055")];
        old_events.push(new_scheme_channel("808 Kick"));
        let mut new_events = vec![latin1_text(199, "25.2.5.5055")];
        new_events.push(new_scheme_channel("808 Kick"));
        new_events.push(new_scheme_channel_with_plugin("Drumpad", "FPC"));
        let old = extracted_of(old_events);
        let new = extracted_of(new_events);
        assert_eq!(old.channel_names, vec!["808 Kick".to_string()]);
        assert_eq!(
            new.channel_names,
            vec!["808 Kick".to_string(), "Drumpad".to_string()]
        );
        assert_eq!(new.plugin_names, vec!["FPC".to_string()]);
    }

    #[test]
    fn insert_names_are_extracted() {
        let e = extracted_of(vec![latin1_text(204, "Dream bell")]);
        assert_eq!(e.mixer_insert_names, vec!["Dream bell".to_string()]);
    }

    #[test]
    fn arrangement_name_guess_is_extracted_but_kept_separate() {
        let e = extracted_of(vec![latin1_text(241, "Arrangement")]);
        assert_eq!(e.arrangement_names, vec!["Arrangement".to_string()]);
    }

    #[test]
    fn blank_text_events_are_dropped() {
        let e = extracted_of(vec![
            latin1_text(199, "10.0.0"),
            latin1_text(192, ""),
            latin1_text(192, "   "),
            latin1_text(192, "Real"),
        ]);
        assert_eq!(e.channel_names, vec!["Real".to_string()]);
    }

    #[test]
    fn tempo_decodes_as_bpm_times_1000() {
        let e = extracted_of(vec![
            latin1_text(199, "20.8.3.2304"),
            dword_event(156, 130_000),
        ]);
        assert_eq!(e.tempo, Tempo::Known(130.0));
        assert_eq!(e.format_status, FormatStatus::Complete);
    }

    #[test]
    fn no_tempo_event_is_unknown_not_zero() {
        // Measured: the real FL 10.0.0 fixture in WIT_FIXTURES has no
        // Tempo (156) event at all.
        let e = extracted_of(vec![latin1_text(199, "10.0.0")]);
        assert_eq!(e.tempo, Tempo::Unknown);
    }

    #[test]
    fn v25_marks_tempo_partial_instead_of_a_wrong_number() {
        // Measured this session: a real v25 Tempo dword decoded to
        // 252566982 -- obvious garbage. Whatever value is present, v25
        // must never surface it as Tempo::Known.
        let e = extracted_of(vec![
            latin1_text(199, "25.2.5.5055"),
            dword_event(156, 252_566_982),
        ]);
        assert_eq!(e.tempo, Tempo::PartialV25ScalarsUnreadable);
        assert_eq!(e.format_status, FormatStatus::PartialV25ScalarsUnreadable);
    }

    #[test]
    fn pre_v25_major_version_boundary() {
        assert!(!is_v25_or_later("24.9.9.9999"));
        assert!(is_v25_or_later("25.0.0.0"));
        assert!(is_v25_or_later("26.1.0.0"));
    }

    #[test]
    fn new_channel_scheme_boundary_is_11_5() {
        // Measured this session: real FL 11.1.0 fixtures use the old
        // (ChanName) scheme; real FL 11.5.14/11.5.16 fixtures already
        // use the new (NewChan/PluginName) one -- an earlier pass of this
        // constant was set at major version 12, which silently produced
        // zero channel names on those two real 11.5.x files.
        assert!(!uses_new_channel_scheme("11.1.0"));
        assert!(!uses_new_channel_scheme("11.4.9.9999"));
        assert!(uses_new_channel_scheme("11.5.14"));
        assert!(uses_new_channel_scheme("11.5.16"));
        assert!(uses_new_channel_scheme("12.0.0.0"));
        assert!(uses_new_channel_scheme("25.2.5.5055"));
    }

    #[test]
    fn major_minor_version_defaults_a_missing_minor_to_zero() {
        assert_eq!(major_minor_version("11"), Some((11, 0)));
        assert_eq!(major_minor_version("11.5.14"), Some((11, 5)));
        assert_eq!(major_minor_version("not-a-version"), None);
    }

    #[test]
    fn unparseable_version_is_never_treated_as_v25_or_new_scheme() {
        assert!(!is_v25_or_later("not-a-version"));
        assert!(!uses_new_channel_scheme("not-a-version"));
        assert_eq!(major_version("not-a-version"), None);
    }

    #[test]
    fn utf16_text_decodes_including_non_latin_scripts() {
        // AGENTS.md/task: a Persian name must survive the UTF-16LE path.
        let e = extracted_of(vec![latin1_text(199, "10.0.0"), utf16_text(192, "آواز")]);
        assert_eq!(e.channel_names, vec!["آواز".to_string()]);
    }

    #[test]
    fn latin1_three_letter_names_are_not_mistaken_for_utf16() {
        for name in ["Hat", "Kik", "Bss"] {
            let e = extracted_of(vec![latin1_text(199, "10.0.0"), latin1_text(192, name)]);
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
