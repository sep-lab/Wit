//! Whitelist extraction: a walked event stream -> [`Extracted`]. A Rust
//! port of the *decisions* `experiments/flp_parse.py` makes about which
//! ids matter (`EVENT_NAMES`, `TEXT_EVENTS`, `decode_text`'s UTF-16LE-vs-
//! latin-1 heuristic), extended with the specific ids this crate's
//! whitelist asks for that the prototype only ever printed as a raw,
//! untyped text row (tempo, and a structured read of mixer insert /
//! arrangement names rather than a generic listing).
//!
//! # Where a name sits decides what it names
//!
//! FL's text events carry no owner: a `DefPluginName` (201) or
//! `PluginName` (203) means one thing inside a channel and another inside
//! the mixer. So extraction walks the stream as three regions, measured
//! on 178 real `.flp` files (FL 8.5.0 – 25.2.5: the 172 files FL Studio 20
//! ships in its app bundle, one FL 10.0.0 project and five FL 25.2.5 saves;
//! read-only copies, nothing committed):
//!
//! 1. **Channel blocks.** Each channel starts at a `NewChan` (64). Its
//!    *header* is the run of events right after it: `64, 21, 201, 212,
//!    203` in every one of the 3,694 channel blocks on FL >= 11.5 that
//!    carry a name, i.e. the channel's own name (203) is always exactly 4
//!    events after its 64. A block ends at the next 64 **or at the first
//!    event from [`CHANNEL_SECTION_END_IDS`]**. That second rule is the
//!    fix for a real bug: the last channel's block used to stay open to
//!    the end of the file, so a last channel with no name of its own
//!    borrowed the first 203 anywhere after it — on FL 20.7's bundled
//!    `Surround mix.flp` template that was the name of the Master insert's
//!    "Control Surface" plugin, 554 events later.
//! 2. **The arrangement/playlist section** that follows the channels
//!    (entered at the first [`CHANNEL_SECTION_END_IDS`] event). No 201,
//!    203 or 192 was ever observed here; nothing is read from them.
//! 3. **The mixer**, which starts at the first `InsertFlags`-like event
//!    ([`MIXER_INSERT_START`], 236) and has one such event per mixer
//!    position. Every 201 here is an effect plugin on that position.
//!
//! Every one of the 8,403 `DefPluginName` events in those 178 files sits
//! either in a channel header (4,004) or in the mixer (4,399) — none
//! anywhere else — on every FL version measured, which is why generator
//! and effect plugins can be read the same way on every version.
//!
//! # The channel-name id changed in FL 11.5
//!
//! A first pass of this module read every channel's name from id 192
//! (`ChanName`) on every version, because one real FL 10.0.0 file and
//! `flp_parse.py`'s `EVENT_NAMES` table suggested it. The wider check
//! showed that is only right before FL 11.5: from FL 11.5 on, id 192 is
//! absent (FL 11.5–20.8) or holds the app's own build string
//! (`"FL Studio 25.2.5.5055.5055"` on all 5 FL 25 saves), and the
//! channel's name is the 203 in its header — see
//! [`NEW_CHANNEL_SCHEME_MIN_VERSION`]. The strongest check that this reads
//! the *right* names, not just the right number of them: FL Studio 20's
//! bundle ships two demo songs twice, once saved by FL 11.0/11.1 (names on
//! id 192) and once re-saved by FL 12.3 (names on the header 203). Every
//! one of the older saves' channel names (164 of 164, and 64 of 64)
//! appears verbatim among the newer saves' names.

use crate::frame::{Header, RawEvent};
use std::fmt;

/// `NewChan` — starts a channel block. Present in every FL version
/// checked (one per channel: the count equals `FLhd.channels` in every
/// file measured).
const NEW_CHAN: u8 = 64;
/// `ChanName` — a channel's display name, **only before FL 11.5**, and only
/// read inside that channel's own block. Verified: decodes real names
/// (`"Kick"`, `"Xtra Bass"`, `"Clap"`, ...) on real files from FL 8.5.0
/// through 11.1.1, exactly one per channel block (943 blocks). **Never
/// read on FL >= 11.5** — see [`NEW_CHANNEL_SCHEME_MIN_VERSION`].
const CHAN_NAME: u8 = 192;
/// `PatName` — a pattern's display name. Verified on real pre-v25 files
/// (e.g. `"Choir"` on an FL 10 file). **Unverified on FL 25**: id 193
/// occurs zero times in all 5 real FL 25 saves checked, so whether it
/// still names a pattern there has not been observed either way.
const PAT_NAME: u8 = 193;
/// `Version` — the FL Studio build that wrote the file, e.g.
/// `"25.2.5.5055"`. Event #0 in all 178 real files checked.
const VERSION: u8 = 199;
/// `DefPluginName` — the plugin's own type name (`"FPC"`, `"Harmor"`,
/// `"Fruity Reeverb 2"`, `"Fruity Wrapper"` for a hosted VST/AU). In a
/// channel header it is that channel's generator; in the mixer it is an
/// effect (see the module doc for the measured split). Empty for a plain
/// Sampler channel — a real "no plugin" fact, dropped rather than
/// reported as an empty name.
const DEF_PLUGIN_NAME: u8 = 201;
/// `PluginName` — **on FL >= 11.5, the 203 in a channel's header is that
/// channel's own display name** (`"808 Kick"`, `"Drumpad"`, ...). Every
/// other 203 (a renamed effect in the mixer, most often) is never read.
/// Below FL 11.5 a header 203 exists on some channels (18 blocks
/// measured) but holds the generator instance's own name (`"Lead"` on a
/// `"Sytrus"` channel, say), while the channel's name is on 192 — so it
/// is never read there either.
const PLUGIN_NAME: u8 = 203;
/// The FL (major, minor) version from which channel names are read from
/// the header 203 instead of 192. **Measured, not documented:** FL 11.0.x
/// and 11.1.x files carry names on 192 (one per channel); FL 11.5.14 and
/// 11.5.16 files have zero 192 events and names on the header 203. FL
/// 11.2–11.4 was never observed (no file in that range), so a file
/// declaring one of those versions takes the pre-11.5 path — untested for
/// that range rather than wrong by measurement.
const NEW_CHANNEL_SCHEME_MIN_VERSION: (u32, u32) = (11, 5);
/// `InsertName` — a mixer insert's display name (present only when the
/// insert was renamed). Verified: decodes real insert names (`"Dream
/// bell"`, `"REC"`) on real pre-v25 files. **Unverified on FL 25**: id 204
/// occurs zero times in all 5 real FL 25 saves checked.
const INSERT_NAME: u8 = 204;
/// One per mixer position, in mixer order; the first one starts the
/// mixer. Measured: 105 per file on FL 8.5–12.5, 127 on FL 12.9–20.8, 18
/// on the FL 25 saves checked — and never inside a channel block. The
/// position (0 = Master, counted from the first 236) is what
/// [`MixerEffect::insert`] reports.
const MIXER_INSERT_START: u8 = 236;
/// Mixer positions above this are not labelled (see
/// [`MixerEffect::insert`]): 1–99 are "Insert 1".."Insert 99" on every FL
/// version with 105 or 127 positions, but what FL calls positions 100 and
/// up differs by version (sends and the "current" track on 105-position
/// files, more inserts on 127-position ones) and was not checked here.
const MAX_LABELLED_INSERT: u32 = 99;
/// Events that end the channel section. **Derived from the data:** none of
/// these ids occurs inside any channel block (before the next `NewChan`)
/// in any of the 178 real files, and the first event after every file's
/// last channel header that is one of them is 238 (69 files), 99 (67) or
/// 233 (42) — never within that channel's header. 99/241 open an
/// arrangement (241 is the `"Arrangement"` name), 233/238/239 are
/// playlist/track data, 100 follows the arrangements, and
/// 98/147/149/154/204/235/236 are mixer events.
const CHANNEL_SECTION_END_IDS: [u8; 13] = [
    98, 99, 100, 147, 149, 154, 204, 233, 235, 236, 238, 239, 241,
];
/// Ids that may sit between a `NewChan` and the channel's own 201/203 —
/// the header shape measured on every channel block (`64, 21, 201, 212,
/// 203`). Anything else ends the header, so a 201 or 203 further down a
/// block is never read as the channel's generator or name.
const CHANNEL_HEADER_IDS: [u8; 3] = [21, DEF_PLUGIN_NAME, 212];
/// `Tempo` — dword, `round(BPM * 1000)`. **Not documented anywhere this
/// crate cites** (absent from `flp_parse.py`'s `EVENT_NAMES`); identified
/// by scanning every dword-class id that appears exactly once per file
/// for a value that divides evenly by 1000 into a plausible BPM. On a real
/// FL 20.8.3 file, id 156's single occurrence decoded to exactly `130000`
/// -> `130.0` BPM. **Absent on the real FL 10.0.0 file checked** — tempo
/// reads as [`Tempo::Unknown`] there, not a wrong number. See
/// [`is_v25_or_later`] for why it is untrustworthy on FL 25.
const TEMPO: u8 = 156;
/// An empirically observed, **undocumented** id that decoded to the
/// literal text `"Arrangement"` on the FL 20/25 files checked (absent on
/// FL 10, which predates arrangements). The closest candidate found for
/// the whitelist's "playlist/arrangement names if present"; kept separate
/// from the better-evidenced ids above. `Extracted::arrangement_names` is
/// empty, never wrong, if this guess is off for a given file.
const ARRANGEMENT_NAME_GUESS: u8 = 241;

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
/// string takes the pre-FL-11.5 extraction path, never suppresses a real
/// tempo reading, and never switches on `frame.rs`'s FL 25 framing rule.
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
/// `0` rather than failing the whole parse.
fn major_minor_version(version: &str) -> Option<(u32, u32)> {
    let mut parts = version.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().and_then(|m| m.parse().ok()).unwrap_or(0);
    Some((major, minor))
}

/// Whether `version` uses the FL 11.5+ channel-name scheme — see
/// [`NEW_CHANNEL_SCHEME_MIN_VERSION`].
fn uses_new_channel_scheme(version: &str) -> bool {
    major_minor_version(version).is_some_and(|v| v >= NEW_CHANNEL_SCHEME_MIN_VERSION)
}

/// A tempo reading, honest about the one case it cannot trust.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Tempo {
    /// No `Tempo` event was found at all — either a version old enough
    /// not to have one (measured: the real FL 10.0.0 file checked has
    /// none), or a genuinely unreadable file.
    #[default]
    Unknown,
    Known(f64),
    /// **FL Studio v25+: the scalar (fixed-width) event stream is under an
    /// unsolved, offset-dependent obfuscation keystream**
    /// ([issue #7](https://github.com/sep-lab/Wit/issues/7)). Measured: a
    /// real v25.2.5 file's `Tempo` dword decoded to `252566982`
    /// (`252566.982` BPM) — obvious garbage. Rather than surface a wrong
    /// number, extraction refuses to interpret this field on v25+.
    PartialV25ScalarsUnreadable,
}

/// Whether extraction on this file is complete, or was restricted because
/// the file is FL Studio v25+.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FormatStatus {
    /// Every whitelisted field this crate models was read normally.
    #[default]
    Complete,
    /// FL Studio v25+ ([issue #7](https://github.com/sep-lab/Wit/issues/7)):
    /// scalar (fixed-width) events are under an unsolved obfuscation
    /// keystream, so only variable-length (text/data) events are read.
    /// Measured on all 5 real FL 25 saves checked: channel names decode
    /// cleanly and number exactly `FLhd.channels`, and generator/effect
    /// plugin names decode cleanly. **Unverified rather than confirmed**:
    /// pattern names (id 193) and mixer insert names (id 204) — neither id
    /// occurs in any of those 5 files — and the mixer position an effect is
    /// attributed to (FL 25 saves only 18 mixer positions, and which FL
    /// label each carries was not checked). [`Extracted::tempo`] is
    /// [`Tempo::PartialV25ScalarsUnreadable`].
    PartialV25ScalarsUnreadable,
}

/// One channel in the Channel rack, in rack order.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RackChannel {
    /// The channel's own display name, or `None` when the file carries no
    /// non-blank one for it (4 of 3,698 channel blocks on FL >= 11.5, e.g.
    /// the single default Sampler channel in FL 20.7's `Surround mix.flp`
    /// and `Empty with 4 sends.flp` templates). Never borrowed from
    /// anything outside the channel's own block.
    pub name: Option<String>,
    /// The generator plugin the channel plays (`"FPC"`, `"Sytrus"`), from
    /// the 201 in its header. `None` for a plain Sampler/audio channel.
    pub generator: Option<String>,
}

/// One effect plugin in the mixer.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct MixerEffect {
    /// The plugin's own type name (201), e.g. `"Fruity Reeverb 2"`. A
    /// hosted VST/AU reads as FL's own `"Fruity Wrapper"`: the hosted
    /// plugin's name lives in its opaque state (ADR-0003), not in a
    /// whitelisted text event.
    pub name: String,
    /// Which mixer insert it sits on: `Some(0)` is the Master, `Some(n)` is
    /// FL's "Insert n". `None` for positions above 99, whose FL label
    /// depends on the version and was not checked. Counted from the first
    /// [`MIXER_INSERT_START`] event. **Inferred, not observed in FL
    /// itself:** 63 of the 85 real files with an effect at position 0 have
    /// a Fruity Limiter or Maximus there (the bundled `Basic with
    /// limiter.flp` template's only effect is a Fruity Limiter at position
    /// 0), and FL 20.7's `Surround mix.flp` template names insert *n* (204)
    /// `"n"` right before position *n*'s 236 — both consistent with this
    /// numbering. Unverified on FL 25.
    pub insert: Option<u16>,
}

/// Everything this crate's whitelist extracts from one `.flp`: the header
/// fields, the FL Studio version that wrote it, tempo, and every
/// whitelisted name category.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Extracted {
    pub header_format: i16,
    /// The header's own declared channel count (`FLhd.channels`).
    pub channels: u16,
    pub ppq: u16,
    /// The `Version` text event, verbatim (e.g. `"25.2.5.5055"`). `None`
    /// if the file has no `Version` event at all. Untrusted text: sanitize
    /// before printing.
    pub fl_version: Option<String>,
    pub format_status: FormatStatus,
    pub tempo: Tempo,
    /// One entry per channel block (`NewChan`), in rack order. See
    /// [`Extracted::channel_names`] and [`Extracted::generator_names`].
    pub channel_rack: Vec<RackChannel>,
    pub pattern_names: Vec<String>,
    /// Every effect plugin in the mixer, in mixer order — on every FL
    /// version (FL < 11.5 used to list these mixed in with generators;
    /// FL >= 11.5 used to drop them).
    pub mixer_effects: Vec<MixerEffect>,
    pub mixer_insert_names: Vec<String>,
    /// Best-effort — see [`ARRANGEMENT_NAME_GUESS`]. Always empty rather
    /// than wrong if the guess doesn't hold for a given file.
    pub arrangement_names: Vec<String>,
}

impl Extracted {
    /// Every channel's own name, in rack order, skipping channels without
    /// one. Not deduplicated — two channels can genuinely share a name.
    pub fn channel_names(&self) -> Vec<String> {
        self.channel_rack
            .iter()
            .filter_map(|c| c.name.clone())
            .collect()
    }

    /// Every channel's generator plugin, in rack order, skipping plain
    /// Sampler/audio channels.
    pub fn generator_names(&self) -> Vec<String> {
        self.channel_rack
            .iter()
            .filter_map(|c| c.generator.clone())
            .collect()
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

/// Which part of the event stream the walk is in — see the module doc.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Region {
    /// Project-level events before the first `NewChan`.
    BeforeChannels,
    /// Inside a channel block. `header_open` stays true only while the
    /// events since its `NewChan` are all [`CHANNEL_HEADER_IDS`].
    Channel { header_open: bool },
    /// After the channel section, before the mixer.
    AfterChannels,
    /// In the mixer, at this position (0 = Master).
    Mixer { position: u32 },
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

    let mut region = Region::BeforeChannels;
    // Whether the current channel's header 201 was already seen (it may be
    // blank, which leaves `generator` at None but must still not let a
    // second header 201 overwrite it).
    let mut generator_seen = false;

    for event in events {
        let id = event.id;

        // Region transitions first.
        if id == NEW_CHAN && !matches!(region, Region::Mixer { .. }) {
            // Measured: no NewChan ever follows the mixer's start; if one
            // did, it must not turn mixer effects into channel generators.
            out.channel_rack.push(RackChannel::default());
            region = Region::Channel { header_open: true };
            generator_seen = false;
            continue;
        }
        if id == MIXER_INSERT_START {
            region = Region::Mixer {
                position: match region {
                    Region::Mixer { position } => position.saturating_add(1),
                    _ => 0,
                },
            };
        } else if matches!(region, Region::Channel { .. }) && CHANNEL_SECTION_END_IDS.contains(&id)
        {
            region = Region::AfterChannels;
        }

        // Is this event part of the current channel's header?
        let in_header = match region {
            Region::Channel { header_open: true } => {
                CHANNEL_HEADER_IDS.contains(&id) || (new_channel_scheme && id == PLUGIN_NAME)
            }
            _ => false,
        };
        if let Region::Channel { header_open } = &mut region {
            if !in_header {
                *header_open = false;
            }
        }

        match id {
            DEF_PLUGIN_NAME => match region {
                Region::Channel { .. } if in_header && !generator_seen => {
                    generator_seen = true;
                    if let Some(channel) = out.channel_rack.last_mut() {
                        channel.generator = non_blank(event.payload);
                    }
                }
                Region::Mixer { position } => {
                    if let Some(name) = non_blank(event.payload) {
                        out.mixer_effects.push(MixerEffect {
                            name,
                            insert: u16::try_from(position)
                                .ok()
                                .filter(|_| position <= MAX_LABELLED_INSERT),
                        });
                    }
                }
                // Measured never: a 201 before the channels, after the
                // channel header, or between the channels and the mixer.
                // Its role would be unknown, so it is not read.
                _ => {}
            },
            PLUGIN_NAME if in_header => {
                // Only reachable on FL >= 11.5 (see `in_header`). The name
                // closes the header: nothing after it is the channel's own.
                if let Some(channel) = out.channel_rack.last_mut() {
                    channel.name = non_blank(event.payload);
                }
                region = Region::Channel { header_open: false };
            }
            CHAN_NAME if !new_channel_scheme => {
                if let (Region::Channel { .. }, Some(channel)) =
                    (region, out.channel_rack.last_mut())
                {
                    if channel.name.is_none() {
                        channel.name = non_blank(event.payload);
                    }
                }
            }
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

fn non_blank(payload: &[u8]) -> Option<String> {
    let text = decode_text(payload);
    (!text.trim().is_empty()).then_some(text)
}

fn push_text(into: &mut Vec<String>, payload: &[u8]) {
    if let Some(text) = non_blank(payload) {
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

    // Literal byte ids throughout this test module, never the crate's own
    // constants — pinning the wire format independently of any future
    // rename/typo in extract.rs itself.

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

    fn byte_event(id: u8, value: u8) -> Vec<u8> {
        vec![id, value]
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

    /// A real FL >= 11.5 channel, byte for byte in the shape measured on
    /// every named channel block: `NewChan` (64), 21, `DefPluginName`
    /// (201, the generator; empty for a plain Sampler), 212 (opaque plugin
    /// data), `PluginName` (203, the channel's own name), then a couple of
    /// the ordinary channel events that follow (155, 128).
    fn channel(display_name: &str, generator: &str) -> Vec<u8> {
        let mut out = word_event(64, 0);
        out.extend(byte_event(21, 0));
        out.extend(latin1_text(201, generator));
        out.extend(var_event(212, &[0; 4]));
        out.extend(latin1_text(203, display_name));
        out.extend(dword_event(155, 0));
        out.extend(dword_event(128, 0));
        out
    }

    /// A channel block with no name of its own: FL 20.7's `Surround mix`
    /// template's single Sampler channel has exactly this header (21, 201
    /// empty, 212, then straight on to 155).
    fn unnamed_sampler_channel() -> Vec<u8> {
        let mut out = word_event(64, 0);
        out.extend(byte_event(21, 0));
        out.extend(latin1_text(201, ""));
        out.extend(var_event(212, &[0; 4]));
        out.extend(dword_event(155, 0));
        out.extend(dword_event(128, 0));
        out.extend(byte_event(0, 1));
        out
    }

    /// One mixer position: [204 name,] 236, then each slot's plugin events
    /// (201 type, 212, optional 203 instance name, 213) closed by a 98 slot
    /// index, then 235/154/147 — the shape measured on FL 12.9–20.8 files.
    fn mixer_insert(name: Option<&str>, effects: &[(&str, Option<&str>)]) -> Vec<u8> {
        let mut out = Vec::new();
        if let Some(name) = name {
            out.extend(dword_event(149, 0));
            out.extend(latin1_text(204, name));
        }
        out.extend(var_event(236, &[0; 4]));
        for (slot, (plugin, instance_name)) in effects.iter().enumerate() {
            out.extend(latin1_text(201, plugin));
            out.extend(var_event(212, &[0; 4]));
            if let Some(instance_name) = instance_name {
                out.extend(latin1_text(203, instance_name));
            }
            out.extend(var_event(213, &[0; 4]));
            out.extend(word_event(98, slot as u16));
        }
        out.extend(var_event(235, &[0; 4]));
        out.extend(dword_event(154, 0));
        out.extend(dword_event(147, 0));
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

    fn strings(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    // ---- the last channel's block must close ------------------------- //

    #[test]
    fn a_nameless_last_channel_never_borrows_a_mixer_effects_name() {
        // The review's reproduction, in the exact shape of FL 20.7's
        // `Surround mix.flp` template: one plain Sampler channel with no
        // 203 of its own, then the arrangement (99, 241 "Arrangement"),
        // then the mixer, whose Master insert holds a "Control Surface"
        // plugin renamed "Mix" (201 then 203). The old reader took "Mix"
        // as the channel's name; the block must close at the arrangement.
        let e = extracted_of(vec![
            latin1_text(199, "20.7.0.1702"),
            word_event(64, 0),
            latin1_text(201, ""),
            dword_event(155, 0),
            dword_event(128, 0),
            byte_event(0, 1),
            word_event(99, 0),
            latin1_text(241, "Arrangement"),
            var_event(233, &[0; 8]),
            latin1_text(204, " "),
            var_event(236, &[0; 4]),
            latin1_text(201, "Control Surface"),
            var_event(212, &[0; 4]),
            latin1_text(203, "Mix"),
        ]);
        assert!(e.channel_names().is_empty(), "{:?}", e.channel_names());
        assert_eq!(e.channel_rack.len(), 1);
        assert_eq!(e.channel_rack[0], RackChannel::default());
        assert_eq!(
            e.mixer_effects,
            vec![MixerEffect {
                name: "Control Surface".to_string(),
                insert: Some(0),
            }]
        );
    }

    #[test]
    fn the_review_byte_shape_reads_no_channel_names() {
        // Literally `64, 201 "", …, 241 "Arrangement", 204, 236,
        // 201 "Control Surface", 203 "Mix"` -> no channel names.
        let e = extracted_of(vec![
            latin1_text(199, "20.7.0.1702"),
            word_event(64, 0),
            latin1_text(201, ""),
            dword_event(128, 0),
            latin1_text(241, "Arrangement"),
            latin1_text(204, "1"),
            var_event(236, &[0; 4]),
            latin1_text(201, "Control Surface"),
            latin1_text(203, "Mix"),
        ]);
        assert!(e.channel_names().is_empty());
    }

    #[test]
    fn each_section_end_id_closes_the_last_channel_block() {
        // Every id in the derived closing set must end the block on its
        // own, so a nameless last channel never reaches a later 201/203.
        for end_id in [
            98u8, 99, 100, 147, 149, 154, 204, 233, 235, 236, 238, 239, 241,
        ] {
            let closer = match end_id {
                98..=100 => word_event(end_id, 0),
                147..=154 => dword_event(end_id, 0),
                _ => latin1_text(end_id, "x"),
            };
            let e = extracted_of(vec![
                latin1_text(199, "20.8.0.1377"),
                unnamed_sampler_channel(),
                closer,
                latin1_text(201, "Maximus"),
                latin1_text(203, "Maximus"),
            ]);
            assert!(e.channel_names().is_empty(), "closer {end_id}");
            assert_eq!(
                e.channel_rack,
                vec![RackChannel::default()],
                "closer {end_id}"
            );
        }
    }

    #[test]
    fn a_203_further_down_a_channel_block_is_never_its_name() {
        // The name is only read from the header (`64, 21, 201, 212, 203`,
        // measured on every named block). A 203 after other channel events
        // is not the channel's own name, whatever it is.
        let mut unnamed = unnamed_sampler_channel();
        unnamed.extend(latin1_text(203, "Not a channel name"));
        let e = extracted_of(vec![latin1_text(199, "20.8.0.1377"), unnamed]);
        assert!(e.channel_names().is_empty());
    }

    #[test]
    fn only_the_header_201_and_203_of_a_block_are_read() {
        // Defensive, not a measured shape: 0 of the 3,605 non-last channel
        // blocks checked on FL >= 11.5 hold more than one 201 or 203. If
        // one ever did, only the header pair is the channel's own.
        let mut events = vec![latin1_text(199, "20.8.3.2304")];
        events.push(channel("Pluck 1", "Harmor"));
        events.push(latin1_text(201, "Fruity Reeverb 2"));
        events.push(latin1_text(203, "Reverb Slot"));
        let e = extracted_of(events);
        assert_eq!(e.channel_names(), strings(&["Pluck 1"]));
        assert_eq!(e.generator_names(), strings(&["Harmor"]));
        assert!(e.mixer_effects.is_empty());
    }

    // ---- FL >= 11.5 (NewChan header) path ----------------------------- //

    #[test]
    fn fl_11_5_plus_reads_channel_names_from_the_header_203() {
        let e = extracted_of(vec![
            latin1_text(199, "20.8.3.2304"),
            channel("808 Kick", ""),
            channel("FLEX Bass", "FLEX"),
        ]);
        assert_eq!(e.channel_names(), strings(&["808 Kick", "FLEX Bass"]));
        assert_eq!(e.generator_names(), strings(&["FLEX"]));
        assert_eq!(
            e.channel_rack[1],
            RackChannel {
                name: Some("FLEX Bass".to_string()),
                generator: Some("FLEX".to_string()),
            }
        );
    }

    #[test]
    fn fl_11_5_also_uses_the_new_channel_scheme() {
        // Regression: real FL 11.5.14/11.5.16 files use the new scheme.
        let e = extracted_of(vec![latin1_text(199, "11.5.14"), channel("Ghost Kick", "")]);
        assert_eq!(e.channel_names(), strings(&["Ghost Kick"]));
    }

    #[test]
    fn fl_11_5_plus_never_reads_192_as_a_channel_name() {
        // On FL 25 id 192 holds the build string (event #5, before any
        // channel); on FL 11.5–20.8 it does not occur. Never a name.
        let e = extracted_of(vec![
            latin1_text(199, "25.2.5.5055"),
            latin1_text(192, "FL Studio 25.2.5.5055.5055"),
            channel("808 Kick", ""),
            latin1_text(192, "inside a block"),
        ]);
        assert_eq!(e.channel_names(), strings(&["808 Kick"]));
    }

    #[test]
    fn channels_interleaved_with_pattern_events_keep_their_names() {
        // Measured: pattern events (65, 193) sit between channel blocks in
        // 54 of 93 FL >= 11.5 files. They neither close a block nor get
        // mistaken for a channel's name.
        let mut first = channel("Kick", "");
        first.extend(word_event(65, 1));
        first.extend(latin1_text(193, "Intro"));
        let e = extracted_of(vec![
            latin1_text(199, "20.8.0.1377"),
            first,
            channel("Snare", ""),
        ]);
        assert_eq!(e.channel_names(), strings(&["Kick", "Snare"]));
        assert_eq!(e.pattern_names, strings(&["Intro"]));
    }

    #[test]
    fn a_channel_add_reports_one_channel_and_its_generator_not_a_plugin_pair() {
        let old = extracted_of(vec![
            latin1_text(199, "25.2.5.5055"),
            channel("808 Kick", ""),
        ]);
        let new = extracted_of(vec![
            latin1_text(199, "25.2.5.5055"),
            channel("808 Kick", ""),
            channel("Drumpad", "Drumpad"),
        ]);
        assert_eq!(old.channel_names(), strings(&["808 Kick"]));
        assert_eq!(new.channel_names(), strings(&["808 Kick", "Drumpad"]));
        assert_eq!(new.generator_names(), strings(&["Drumpad"]));
    }

    // ---- pre-FL-11.5 (id 192) path ------------------------------------ //

    /// A pre-11.5 channel block: the name is on 192, some 30+ events into
    /// the block (positions 34–39 measured), not in the header.
    fn old_scheme_channel(name: &str, generator: &str) -> Vec<u8> {
        let mut out = word_event(64, 0);
        out.extend(byte_event(21, 0));
        out.extend(latin1_text(201, generator));
        out.extend(var_event(212, &[0; 4]));
        out.extend(var_event(213, &[0; 4]));
        for _ in 0..30 {
            out.extend(byte_event(0, 1));
        }
        out.extend(latin1_text(192, name));
        out
    }

    #[test]
    fn pre_11_5_reads_names_from_192_and_generators_from_the_header() {
        let e = extracted_of(vec![
            latin1_text(199, "10.0.0"),
            old_scheme_channel("Kick", ""),
            old_scheme_channel("Xtra Bass", "Sytrus"),
            latin1_text(193, "Choir"),
        ]);
        assert_eq!(e.channel_names(), strings(&["Kick", "Xtra Bass"]));
        assert_eq!(e.generator_names(), strings(&["Sytrus"]));
        assert_eq!(e.pattern_names, strings(&["Choir"]));
    }

    #[test]
    fn pre_11_5_header_203_is_never_read_as_a_channel_name() {
        // Measured on 18 real pre-11.5 blocks: a header 203 there holds the
        // generator instance's name ("Lead" on a Sytrus channel); the
        // channel's own name is the 192.
        let mut block = word_event(64, 0);
        block.extend(byte_event(21, 0));
        block.extend(latin1_text(201, "Sytrus"));
        block.extend(var_event(212, &[0; 4]));
        block.extend(latin1_text(203, "Lead"));
        block.extend(latin1_text(192, "Synth Lead"));
        let e = extracted_of(vec![latin1_text(199, "11.1.1"), block]);
        assert_eq!(e.channel_names(), strings(&["Synth Lead"]));
    }

    #[test]
    fn pre_11_5_192_outside_a_channel_block_is_not_a_name() {
        let e = extracted_of(vec![
            latin1_text(199, "10.0.0"),
            latin1_text(192, "before any channel"),
            old_scheme_channel("Kick", ""),
            word_event(99, 0),
            latin1_text(192, "after the channels"),
        ]);
        assert_eq!(e.channel_names(), strings(&["Kick"]));
    }

    #[test]
    fn no_version_event_falls_back_to_the_pre_11_5_path() {
        let e = extracted_of(vec![old_scheme_channel("Kick", "")]);
        assert_eq!(e.channel_names(), strings(&["Kick"]));
    }

    // ---- mixer effects, on every version ------------------------------ //

    #[test]
    fn mixer_effects_are_read_with_their_insert_on_fl_11_5_plus() {
        let e = extracted_of(vec![
            latin1_text(199, "20.8.0.1377"),
            channel("Kick", ""),
            word_event(99, 0),
            latin1_text(241, "Arrangement"),
            mixer_insert(None, &[("Maximus", None)]),
            mixer_insert(Some("Drums"), &[]),
            mixer_insert(
                Some("Vox"),
                &[("Fruity Reeverb 2", Some("Hall")), ("Fruity Limiter", None)],
            ),
        ]);
        assert_eq!(e.channel_names(), strings(&["Kick"]));
        assert_eq!(
            e.mixer_effects,
            vec![
                MixerEffect {
                    name: "Maximus".to_string(),
                    insert: Some(0)
                },
                MixerEffect {
                    name: "Fruity Reeverb 2".to_string(),
                    insert: Some(2)
                },
                MixerEffect {
                    name: "Fruity Limiter".to_string(),
                    insert: Some(2)
                },
            ]
        );
        assert_eq!(e.mixer_insert_names, strings(&["Drums", "Vox"]));
    }

    #[test]
    fn mixer_effects_are_read_the_same_way_before_fl_11_5() {
        // FL 10 has no 98 slot events: plugins follow each other directly
        // inside a 236 position (measured on the real FL 10.0.0 file).
        let mut master = var_event(236, &[0; 4]);
        master.extend(latin1_text(201, "Fruity Limiter"));
        master.extend(var_event(212, &[0; 4]));
        master.extend(var_event(213, &[0; 4]));
        let mut insert1 = var_event(236, &[0; 4]);
        insert1.extend(latin1_text(201, "Fruity Wrapper"));
        insert1.extend(latin1_text(203, "Fruity Reeverb"));
        insert1.extend(latin1_text(201, "Maximus"));
        let e = extracted_of(vec![
            latin1_text(199, "10.0.0"),
            old_scheme_channel("Kick", "Sytrus"),
            var_event(233, &[0; 8]),
            master,
            insert1,
        ]);
        assert_eq!(e.channel_names(), strings(&["Kick"]));
        assert_eq!(e.generator_names(), strings(&["Sytrus"]));
        let effects: Vec<(&str, Option<u16>)> = e
            .mixer_effects
            .iter()
            .map(|m| (m.name.as_str(), m.insert))
            .collect();
        assert_eq!(
            effects,
            vec![
                ("Fruity Limiter", Some(0)),
                ("Fruity Wrapper", Some(1)),
                ("Maximus", Some(1)),
            ]
        );
    }

    #[test]
    fn mixer_positions_above_99_are_not_labelled() {
        let mut events = vec![latin1_text(199, "20.8.0.1377")];
        for _ in 0..100 {
            events.push(var_event(236, &[0; 4]));
        }
        events.push(latin1_text(201, "at position 99"));
        events.push(var_event(236, &[0; 4]));
        events.push(latin1_text(201, "at position 100"));
        let e = extracted_of(events);
        assert_eq!(e.mixer_effects[0].insert, Some(99));
        assert_eq!(e.mixer_effects[1].insert, None);
    }

    #[test]
    fn a_new_chan_inside_the_mixer_never_turns_effects_into_generators() {
        let e = extracted_of(vec![
            latin1_text(199, "20.8.0.1377"),
            var_event(236, &[0; 4]),
            word_event(64, 0),
            byte_event(21, 0),
            latin1_text(201, "Fruity Limiter"),
        ]);
        assert!(e.channel_rack.is_empty());
        assert_eq!(e.mixer_effects.len(), 1);
    }

    // ---- other fields ------------------------------------------------- //

    #[test]
    fn insert_names_are_extracted() {
        let e = extracted_of(vec![latin1_text(204, "Dream bell")]);
        assert_eq!(e.mixer_insert_names, strings(&["Dream bell"]));
    }

    #[test]
    fn arrangement_name_guess_is_extracted_but_kept_separate() {
        let e = extracted_of(vec![latin1_text(241, "Arrangement")]);
        assert_eq!(e.arrangement_names, strings(&["Arrangement"]));
    }

    #[test]
    fn blank_names_are_dropped() {
        let e = extracted_of(vec![
            latin1_text(199, "20.8.0.1377"),
            channel("   ", ""),
            channel("Real", ""),
            latin1_text(193, ""),
        ]);
        assert_eq!(e.channel_names(), strings(&["Real"]));
        assert_eq!(e.channel_rack[0].name, None);
        assert!(e.pattern_names.is_empty());
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
        let e = extracted_of(vec![latin1_text(199, "10.0.0")]);
        assert_eq!(e.tempo, Tempo::Unknown);
    }

    #[test]
    fn v25_marks_tempo_partial_instead_of_a_wrong_number() {
        // Measured: a real v25 Tempo dword decoded to 252566982 — garbage.
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
        assert!(!uses_new_channel_scheme("11.1.0"));
        assert!(!uses_new_channel_scheme("11.1.1"));
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
        // A Persian name must survive the UTF-16LE path, on either scheme.
        let mut block = word_event(64, 0);
        block.extend(latin1_text(201, ""));
        block.extend(utf16_text(203, "آواز"));
        let e = extracted_of(vec![latin1_text(199, "20.8.0.1377"), block]);
        assert_eq!(e.channel_names(), strings(&["آواز"]));
        let mut old = word_event(64, 0);
        old.extend(utf16_text(192, "آواز"));
        let e = extracted_of(vec![latin1_text(199, "10.0.0"), old]);
        assert_eq!(e.channel_names(), strings(&["آواز"]));
    }

    #[test]
    fn latin1_three_letter_names_are_not_mistaken_for_utf16() {
        for name in ["Hat", "Kik", "Bss"] {
            let e = extracted_of(vec![latin1_text(199, "20.8.0.1377"), channel(name, "")]);
            assert_eq!(e.channel_names(), strings(&[name]));
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
