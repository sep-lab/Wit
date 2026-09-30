//! `ChangeRecord` → [`Sentence`]: the only place Wit's engine words become
//! words a musician reads. `render_text` in `wit-model` stays the terse,
//! golden-pinned CLI format; this is the studio-couch version.
//!
//! Every sentence is built from spans, so a name the musician typed is
//! always its own span and never mixed into Wit's wording — the UI can
//! bold it, and the vocabulary lint (`vocab.rs`) checks only Wit's words.
//! Names other than track and plugin names are quoted in the plain text
//! ("Added clip 'verse rhodes' on Rhodes") so copy-as-text reads
//! unambiguously.
//!
//! Honesty rules enforced here: an [`Confidence::Inferred`] sentence always
//! says "Probably"; a position Wit had to assume something to compute says
//! "about"; a number that isn't finite (a malformed file) never becomes a
//! place or a value.

use crate::types::{Confidence, Icon, Place, Sentence, Span, SpanKind, Tier};
use wit_model::{BarPos, ChangeRecord, MixField, MixValue, TrackKind};

/// What a sentence needs to know beyond the record itself.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SentenceContext {
    pub tier: Tier,
    /// Quarter-note beats per bar (numerator × 4 / denominator), for
    /// converting Ableton's beat positions to bars. `None` = the time
    /// signature wasn't read: assume 4 and say "about".
    pub beats_per_bar: Option<f64>,
}

struct Builder {
    spans: Vec<Span>,
}

impl Builder {
    fn new() -> Self {
        Builder { spans: Vec::new() }
    }

    fn plain(mut self, text: &str) -> Self {
        self.push(SpanKind::Plain, text);
        self
    }

    fn value(mut self, text: &str) -> Self {
        self.push(SpanKind::Value, text);
        self
    }

    /// A name the musician chose. Track and plugin names stand alone (the
    /// UI styles them); every other kind is quoted in the plain text.
    fn name(mut self, kind: SpanKind, text: &str) -> Self {
        let quoted = !matches!(kind, SpanKind::Track | SpanKind::Plugin | SpanKind::Value);
        if quoted {
            self.push(SpanKind::Plain, "'");
        }
        self.push(kind, text);
        if quoted {
            self.push(SpanKind::Plain, "'");
        }
        self
    }

    fn push(&mut self, kind: SpanKind, text: &str) {
        if text.is_empty() {
            return;
        }
        if let Some(last) = self.spans.last_mut() {
            if last.kind == kind && kind == SpanKind::Plain {
                last.text.push_str(text);
                return;
            }
        }
        self.spans.push(Span {
            kind,
            text: text.to_string(),
        });
    }

    fn finish(self, icon: Icon, tier: Tier) -> Sentence {
        let text = self.spans.iter().map(|s| s.text.as_str()).collect();
        Sentence {
            icon,
            text,
            spans: self.spans,
            track: None,
            lane: None,
            place: None,
            place_label: None,
            confidence: Confidence::Exact,
            tier,
        }
    }
}

impl Sentence {
    fn on(mut self, track: &str) -> Self {
        self.track = Some(track.to_string());
        self
    }

    fn on_opt(mut self, track: &Option<String>) -> Self {
        self.track = track.clone();
        self
    }

    fn at(mut self, place: Option<Place>) -> Self {
        self.place_label = place.as_ref().map(place_label);
        if let Some(Place::Bars {
            approximate: true, ..
        }) = place
        {
            if self.confidence == Confidence::Exact {
                self.confidence = Confidence::Approximate;
            }
        }
        self.place = place;
        self
    }

    fn with_label(mut self, label: String) -> Self {
        self.place_label = Some(label);
        self
    }
}

/// `Some(x)` only for a finite number.
fn finite(x: f64) -> Option<f64> {
    x.is_finite().then_some(x)
}

/// A number the way a musician writes it: `124`, not `124.0`; at most two
/// decimals.
pub fn friendly_num(x: f64) -> String {
    if !x.is_finite() {
        return "?".to_string();
    }
    if x.fract() == 0.0 && x.abs() < 1e15 {
        return format!("{}", x as i64);
    }
    let s = format!("{x:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// Linear gain (Live's volume: 1.0 = 0 dB) to a dB string with a real
/// minus sign.
pub fn gain_to_db(linear: f64) -> String {
    if !linear.is_finite() {
        return "? dB".to_string();
    }
    if linear <= 0.0 {
        return "−∞ dB".to_string();
    }
    let db = 20.0 * linear.log10();
    let rounded = (db * 10.0).round() / 10.0;
    let body = format!("{:.1}", rounded.abs());
    if rounded < 0.0 {
        format!("−{body} dB")
    } else if rounded > 0.0 {
        format!("+{body} dB")
    } else {
        "0.0 dB".to_string()
    }
}

/// Live's pan (−1 … 1) the way Live's pan knob shows it: "50L" … "C" …
/// "50R".
pub fn pan_label(pan: f64) -> String {
    if !pan.is_finite() {
        return "?".to_string();
    }
    let v = (pan * 50.0).round() as i64;
    match v {
        0 => "C".to_string(),
        v if v < 0 => format!("{}L", -v),
        v => format!("{v}R"),
    }
}

fn place_label(place: &Place) -> String {
    match place {
        Place::WholeSong => "whole song".to_string(),
        Place::Bars {
            start,
            end,
            approximate,
            section,
        } => {
            let about = if *approximate { "about " } else { "" };
            let first = start.floor();
            let bars = match end {
                Some(e) if e.floor() > first => {
                    format!(
                        "{about}bars {}–{}",
                        friendly_num(first),
                        friendly_num(e.floor())
                    )
                }
                _ => format!("{about}bar {}", friendly_num(first)),
            };
            match section {
                Some(s) => format!("{bars} · {s}"),
                None => bars,
            }
        }
    }
}

fn bars_from_pos(pos: &BarPos) -> Option<Place> {
    Some(Place::Bars {
        start: finite(pos.bar)?,
        end: None,
        approximate: pos.approximate,
        section: None,
    })
}

fn track_kind_word(kind: TrackKind) -> &'static str {
    match kind {
        TrackKind::Audio => "audio track ",
        TrackKind::Midi => "MIDI track ",
        TrackKind::Group => "group track ",
        TrackKind::Return => "return track ",
    }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}

/// Ableton's `CurrentStart`/`CurrentEnd` are quarter-note beats. Converted
/// to 1-based bars; approximate when the meter wasn't read. `None` for a
/// non-finite position.
fn beats_to_place(
    ctx: &SentenceContext,
    start_beats: f64,
    end_beats: Option<f64>,
) -> Option<Place> {
    let start_beats = finite(start_beats)?;
    let (bpb, approximate) = match ctx.beats_per_bar {
        Some(b) if b.is_finite() && b > 0.0 => (b, false),
        _ => (4.0, true),
    };
    let start = (start_beats / bpb).floor() + 1.0;
    // `end` is exclusive in beats; the last bar touched is inclusive.
    let end = end_beats
        .and_then(finite)
        .map(|e| (e / bpb).ceil().max(start));
    Some(Place::Bars {
        start,
        end,
        approximate,
        section: None,
    })
}

/// True when every number in the record is finite. A record carrying NaN
/// or infinity (a malformed file) is dropped rather than rendered.
pub fn record_is_finite(record: &ChangeRecord) -> bool {
    let nums: Vec<f64> = match record {
        ChangeRecord::TempoChanged { from_bpm, to_bpm } => vec![*from_bpm, *to_bpm],
        ChangeRecord::MixChanged { from, to, .. } => [from, to]
            .iter()
            .filter_map(|v| match v {
                MixValue::Num(x) => Some(*x),
                MixValue::Text(_) => None,
            })
            .collect(),
        ChangeRecord::ClipAdded { start_bar, .. } | ChangeRecord::ClipRemoved { start_bar, .. } => {
            vec![*start_bar]
        }
        ChangeRecord::ClipRangeChanged {
            from_start,
            from_end,
            to_start,
            to_end,
            ..
        } => vec![*from_start, *from_end, *to_start, *to_end],
        ChangeRecord::RegionMoved { from, to, .. } => vec![from.bar, to.bar],
        ChangeRecord::RegionTrimmed {
            from_bars, to_bars, ..
        } => vec![*from_bars, *to_bars],
        _ => vec![],
    };
    nums.iter().all(|x| x.is_finite())
}

/// The sentence for one change record. Callers drop records that fail
/// [`record_is_finite`] first.
pub fn sentence(record: &ChangeRecord, ctx: &SentenceContext) -> Sentence {
    let tier = ctx.tier;
    match record {
        ChangeRecord::TempoChanged { from_bpm, to_bpm } => Builder::new()
            .plain("Tempo ")
            .value(&format!(
                "{} → {} BPM",
                friendly_num(*from_bpm),
                friendly_num(*to_bpm)
            ))
            .finish(Icon::Tempo, tier),
        ChangeRecord::SampleRenamed { old, new, count } => {
            let s = Builder::new()
                .plain("Sample ")
                .name(SpanKind::File, old)
                .plain(" renamed to ")
                .name(SpanKind::File, new);
            let s = if *count > 1 {
                s.plain(" (")
                    .value(&plural(*count, "clip", "clips"))
                    .plain(")")
            } else {
                s
            };
            s.finish(Icon::Rename, tier)
        }
        ChangeRecord::TrackAdded { name, kind } => Builder::new()
            .plain("Added ")
            .plain(track_kind_word(*kind))
            .name(SpanKind::Track, name)
            .finish(Icon::Add, tier)
            .on(name),
        ChangeRecord::TrackRemoved { name, .. } => Builder::new()
            .plain("Removed track ")
            .name(SpanKind::Track, name)
            .finish(Icon::Remove, tier)
            .on(name),
        ChangeRecord::TrackRenamed { old_name, new_name } => Builder::new()
            .plain("Renamed ")
            .name(SpanKind::Track, old_name)
            .plain(" to ")
            .name(SpanKind::Track, new_name)
            .finish(Icon::Rename, tier)
            .on(new_name),
        ChangeRecord::MixChanged {
            track,
            field,
            from,
            to,
        } => mix_sentence(track, *field, from, to, tier),
        ChangeRecord::FxAdded { track, devices } => Builder::new()
            .plain("Added ")
            .name(SpanKind::Plugin, &devices.join(", "))
            .plain(" on ")
            .name(SpanKind::Track, track)
            .finish(Icon::Plugin, tier)
            .on(track),
        ChangeRecord::FxRemoved { track, devices } => Builder::new()
            .plain("Removed ")
            .name(SpanKind::Plugin, &devices.join(", "))
            .plain(" from ")
            .name(SpanKind::Track, track)
            .finish(Icon::Plugin, tier)
            .on(track),
        ChangeRecord::FxReordered { track } => Builder::new()
            .plain("Reordered the effects on ")
            .name(SpanKind::Track, track)
            .finish(Icon::Plugin, tier)
            .on(track),
        ChangeRecord::FxSettingsChanged { track, device } => Builder::new()
            .plain("Changed ")
            .name(SpanKind::Plugin, device)
            .plain(" settings on ")
            .name(SpanKind::Track, track)
            .finish(Icon::Plugin, tier)
            .on(track),
        ChangeRecord::AutomationLanesChanged { track, from, to } => {
            let (verb, n) = if to > from {
                ("Added ", to - from)
            } else {
                ("Removed ", from - to)
            };
            Builder::new()
                .plain(verb)
                .value(&plural(n, "automation lane", "automation lanes"))
                .plain(if to > from { " on " } else { " from " })
                .name(SpanKind::Track, track)
                .finish(Icon::Automation, tier)
                .on(track)
        }
        ChangeRecord::NoteCountChanged { track, from, to } => {
            let (verb, n) = if to > from {
                ("Added ", to - from)
            } else {
                ("Removed ", from - to)
            };
            Builder::new()
                .plain(verb)
                .value(&plural(n, "note", "notes"))
                .plain(if to > from { " on " } else { " from " })
                .name(SpanKind::Track, track)
                .finish(Icon::Midi, tier)
                .on(track)
        }
        ChangeRecord::ClipAdded {
            track,
            label,
            start_bar,
        } => Builder::new()
            .plain("Added clip ")
            .name(SpanKind::Region, label)
            .plain(" on ")
            .name(SpanKind::Track, track)
            .finish(Icon::Add, tier)
            .on(track)
            .at(beats_to_place(ctx, *start_bar, None)),
        ChangeRecord::ClipRemoved {
            track,
            label,
            start_bar,
        } => Builder::new()
            .plain("Removed clip ")
            .name(SpanKind::Region, label)
            .plain(" from ")
            .name(SpanKind::Track, track)
            .finish(Icon::Remove, tier)
            .on(track)
            .at(beats_to_place(ctx, *start_bar, None)),
        ChangeRecord::ClipRangeChanged {
            track,
            label,
            from_start,
            from_end,
            to_start,
            to_end,
        } => clip_range_sentence(
            ctx,
            track,
            label,
            (*from_start, *from_end),
            (*to_start, *to_end),
        ),
        ChangeRecord::ClipMuteChanged {
            track,
            label,
            muted,
        } => Builder::new()
            .plain(if *muted {
                "Muted clip "
            } else {
                "Unmuted clip "
            })
            .name(SpanKind::Region, label)
            .plain(" on ")
            .name(SpanKind::Track, track)
            .finish(Icon::Mute, tier)
            .on(track),
        ChangeRecord::ClipSampleReplaced {
            track,
            label,
            old,
            new,
        } => Builder::new()
            .plain("Swapped the sample in ")
            .name(SpanKind::Region, label)
            .plain(" on ")
            .name(SpanKind::Track, track)
            .plain(": ")
            .name(SpanKind::File, old)
            .plain(" → ")
            .name(SpanKind::File, new)
            .finish(Icon::Record, tier)
            .on(track),
        ChangeRecord::KeyChanged { from, to } => match (from, to) {
            (None, Some(t)) => Builder::new()
                .plain("Set the key to ")
                .value(t)
                .finish(Icon::Key, tier),
            (Some(f), None) => Builder::new()
                .plain("Cleared the key (was ")
                .value(f)
                .plain(")")
                .finish(Icon::Key, tier),
            (f, t) => Builder::new()
                .plain("Key ")
                .value(&format!(
                    "{} → {}",
                    f.as_deref().unwrap_or("none"),
                    t.as_deref().unwrap_or("none")
                ))
                .finish(Icon::Key, tier),
        },
        ChangeRecord::TimeSignatureChanged { from, to } => Builder::new()
            .plain("Time signature ")
            .value(&format!("{from} → {to}"))
            .finish(Icon::Meter, tier),
        ChangeRecord::TrackCountChanged { from, to } => Builder::new()
            .plain("Track count ")
            .value(&format!("{from} → {to}"))
            .finish(Icon::Tracks, tier),
        ChangeRecord::RegionAdded { track, name, at } => {
            region_with_track("Added region ", name, track, " on ", Icon::Add, tier)
                .at(at.as_ref().and_then(bars_from_pos))
        }
        ChangeRecord::RegionRemoved { track, name, at } => {
            region_with_track("Removed region ", name, track, " from ", Icon::Remove, tier)
                .at(at.as_ref().and_then(bars_from_pos))
        }
        ChangeRecord::RegionMoved {
            track,
            name,
            from,
            to,
        } => {
            let delta = to.bar - from.bar;
            let direction = if delta >= 0.0 { "later" } else { "earlier" };
            let amount = delta.abs();
            let unit = if (amount - 1.0).abs() < f64::EPSILON {
                "bar"
            } else {
                "bars"
            };
            let about = if to.approximate || from.approximate {
                "about "
            } else {
                ""
            };
            let s = region_builder("Moved ", name, track, " on ")
                .plain(" ")
                .value(&format!(
                    "{about}{} {unit} {direction}",
                    friendly_num(amount)
                ))
                .finish(Icon::Move, tier)
                .on_opt(track);
            let label = format!("now {about}bar {}", friendly_num(to.bar.floor()));
            s.at(bars_from_pos(to)).with_label(label)
        }
        ChangeRecord::RegionTrimmed {
            track,
            name,
            from_bars,
            to_bars,
        } => {
            let verb = if to_bars < from_bars {
                "Shortened "
            } else {
                "Lengthened "
            };
            region_builder(verb, name, track, " on ")
                .plain(" to ")
                .value(&format!("{} bars", friendly_num(*to_bars)))
                .plain(" (was ")
                .value(&friendly_num(*from_bars))
                .plain(")")
                .finish(Icon::Trim, tier)
                .on_opt(track)
        }
        ChangeRecord::RegionDuplicated { track, name, at } => {
            region_with_track("Duplicated ", name, track, " on ", Icon::Duplicate, tier)
                .at(at.as_ref().and_then(bars_from_pos))
        }
        ChangeRecord::AudioFileAdded { name } => Builder::new()
            .plain("New audio file ")
            .name(SpanKind::File, name)
            .finish(Icon::AudioFile, tier),
        ChangeRecord::AudioFileRemoved { name } => Builder::new()
            .plain("Audio file ")
            .name(SpanKind::File, name)
            .plain(" is no longer in the project")
            .finish(Icon::AudioFile, tier),
        ChangeRecord::MarkerAdded { name, at } => Builder::new()
            .plain("Added marker ")
            .name(SpanKind::Marker, name)
            .finish(Icon::Marker, tier)
            .at(at.as_ref().and_then(bars_from_pos)),
        ChangeRecord::MarkerRemoved { name } => Builder::new()
            .plain("Removed marker ")
            .name(SpanKind::Marker, name)
            .finish(Icon::Marker, tier),
        ChangeRecord::MarkerRenamed { old, new } => Builder::new()
            .plain("Renamed marker ")
            .name(SpanKind::Marker, old)
            .plain(" to ")
            .name(SpanKind::Marker, new)
            .finish(Icon::Marker, tier),
        ChangeRecord::PluginAdded { track, plugin } => {
            let s = Builder::new()
                .plain("Added ")
                .name(SpanKind::Plugin, plugin);
            let s = match track {
                Some(t) => s.plain(" on ").name(SpanKind::Track, t),
                None => s,
            };
            s.finish(Icon::Plugin, tier).on_opt(track)
        }
        ChangeRecord::PluginRemoved { track, plugin } => {
            let s = Builder::new()
                .plain("Removed ")
                .name(SpanKind::Plugin, plugin);
            let s = match track {
                Some(t) => s.plain(" from ").name(SpanKind::Track, t),
                None => s,
            };
            s.finish(Icon::Plugin, tier).on_opt(track)
        }
    }
}

/// Logic's track list mixes track names with MIDI region names until the
/// Logic lane pairs them (`karT` ↔ `qeSM`), so these say "track or MIDI
/// region", use the neutral [`SpanKind::Name`], and claim no track.
pub fn logic_name_added(name: &str, tier: Tier) -> Sentence {
    Builder::new()
        .plain("New track or MIDI region ")
        .name(SpanKind::Name, name)
        .finish(Icon::Add, tier)
}

pub fn logic_name_removed(name: &str, tier: Tier) -> Sentence {
    Builder::new()
        .plain("Removed track or MIDI region ")
        .name(SpanKind::Name, name)
        .finish(Icon::Remove, tier)
}

/// One name gone and one new name in the same save, read as a rename. That
/// reading is Wit's inference, so the words say "Probably" and the
/// confidence says `Inferred`.
pub fn logic_name_renamed(old: &str, new: &str, tier: Tier) -> Sentence {
    let mut s = Builder::new()
        .plain("Probably renamed ")
        .name(SpanKind::Name, old)
        .plain(" to ")
        .name(SpanKind::Name, new)
        .finish(Icon::Rename, tier);
    s.confidence = Confidence::Inferred;
    s
}

/// How a Logic region change honestly names its subject: the bare stem
/// when it's the family's only region object, or "a 'stem' region" when it
/// isn't — matching `experiments/logic_region_map.py`'s own
/// `family_subject` wording, never a copy count (a family can hold region
/// objects that were never placed at all, so a count next to "which copy"
/// would overstate what's actually ambiguous — see
/// `wit_logic::regions::RegionSubject`'s doc). Built with the `Builder`
/// directly, the same way [`logic_name_added`] above bypasses a generic
/// `ChangeRecord` template: a fixed one-name-per-sentence template can't
/// put "a "/" region" in [`SpanKind::Plain`] around just the stem.
fn logic_region_subject(b: Builder, stem: &str, ambiguous: bool) -> Builder {
    if ambiguous {
        b.plain("a ").name(SpanKind::Region, stem).plain(" region")
    } else {
        b.name(SpanKind::Region, stem)
    }
}

/// "on track 3" / "from track 3" — Wit's own words about a raw, 1-based
/// track *number* from `wit_logic::regions`' placement diff, in
/// [`SpanKind::Plain`]. Never [`SpanKind::Track`] (reserved for a name a
/// musician typed) and never set on [`Sentence::track`] either (documented
/// as a track's display *name* — Logic's `karT`↔`qeSM` pairing that would
/// give one is still unsolved).
fn on_track_number(preposition: &str, track: u8) -> String {
    format!("{preposition}track {track}")
}

/// A region appeared on the timeline. `at` is always [`BarPos::about`] for
/// Logic (see `wit-story::build::logic_bar_pos`); this only renders it, it
/// doesn't decide that.
pub fn logic_region_added(
    stem: &str,
    ambiguous: bool,
    track: u8,
    at: BarPos,
    tier: Tier,
) -> Sentence {
    logic_region_subject(Builder::new().plain("Added region "), stem, ambiguous)
        .plain(&on_track_number(" on ", track))
        .finish(Icon::Add, tier)
        .at(bars_from_pos(&at))
}

pub fn logic_region_removed(
    stem: &str,
    ambiguous: bool,
    track: u8,
    at: BarPos,
    tier: Tier,
) -> Sentence {
    logic_region_subject(Builder::new().plain("Removed region "), stem, ambiguous)
        .plain(&on_track_number(" from ", track))
        .finish(Icon::Remove, tier)
        .at(bars_from_pos(&at))
}

/// A region moved on one track — never called for a cross-track move
/// (`wit-story::build::logic_sentences` reports that as a removal plus an
/// addition instead, since one sentence can't honestly name both tracks).
pub fn logic_region_moved(
    stem: &str,
    ambiguous: bool,
    track: u8,
    from: BarPos,
    to: BarPos,
    tier: Tier,
) -> Sentence {
    let delta = to.bar - from.bar;
    let direction = if delta >= 0.0 { "later" } else { "earlier" };
    let amount = delta.abs();
    let unit = if (amount - 1.0).abs() < f64::EPSILON {
        "bar"
    } else {
        "bars"
    };
    let about = if to.approximate || from.approximate {
        "about "
    } else {
        ""
    };
    let s = logic_region_subject(Builder::new().plain("Moved "), stem, ambiguous)
        .plain(&on_track_number(" on ", track))
        .plain(" ")
        .value(&format!(
            "{about}{} {unit} {direction}",
            friendly_num(amount)
        ))
        .finish(Icon::Move, tier);
    let place_label = format!("now {about}bar {}", friendly_num(to.bar.floor()));
    s.at(bars_from_pos(&to)).with_label(place_label)
}

fn region_builder(verb: &str, name: &str, track: &Option<String>, preposition: &str) -> Builder {
    let b = Builder::new().plain(verb).name(SpanKind::Region, name);
    match track {
        Some(t) => b.plain(preposition).name(SpanKind::Track, t),
        None => b,
    }
}

fn region_with_track(
    verb: &str,
    name: &str,
    track: &Option<String>,
    preposition: &str,
    icon: Icon,
    tier: Tier,
) -> Sentence {
    region_builder(verb, name, track, preposition)
        .finish(icon, tier)
        .on_opt(track)
}

fn mix_sentence(
    track: &str,
    field: MixField,
    from: &MixValue,
    to: &MixValue,
    tier: Tier,
) -> Sentence {
    match (field, from, to) {
        (MixField::Volume, MixValue::Num(a), MixValue::Num(b)) => Builder::new()
            .plain("Volume on ")
            .name(SpanKind::Track, track)
            .plain(": ")
            .value(&format!(
                "{} → {}",
                gain_to_db(*a).trim_end_matches(" dB"),
                gain_to_db(*b)
            ))
            .finish(Icon::Mix, tier)
            .on(track),
        (MixField::Pan, MixValue::Num(a), MixValue::Num(b)) => Builder::new()
            .plain("Pan on ")
            .name(SpanKind::Track, track)
            .plain(": ")
            .value(&format!("{} → {}", pan_label(*a), pan_label(*b)))
            .finish(Icon::Mix, tier)
            .on(track),
        (MixField::OutputEnabled, _, MixValue::Text(t)) => {
            let verb = if t == "false" {
                "Muted track "
            } else {
                "Unmuted track "
            };
            Builder::new()
                .plain(verb)
                .name(SpanKind::Track, track)
                .finish(Icon::Mute, tier)
                .on(track)
        }
        (MixField::Color, _, _) => Builder::new()
            .plain("Changed the color of ")
            .name(SpanKind::Track, track)
            .finish(Icon::Mix, tier)
            .on(track),
        (MixField::Volume, _, _) => Builder::new()
            .plain("Changed the volume on ")
            .name(SpanKind::Track, track)
            .finish(Icon::Mix, tier)
            .on(track),
        (MixField::Pan, _, _) => Builder::new()
            .plain("Changed the pan on ")
            .name(SpanKind::Track, track)
            .finish(Icon::Mix, tier)
            .on(track),
        (MixField::OutputEnabled, _, _) => Builder::new()
            .plain("Changed whether ")
            .name(SpanKind::Track, track)
            .plain(" plays")
            .finish(Icon::Mute, tier)
            .on(track),
    }
}

fn clip_range_sentence(
    ctx: &SentenceContext,
    track: &str,
    label: &str,
    from: (f64, f64),
    to: (f64, f64),
) -> Sentence {
    let tier = ctx.tier;
    let from_len = from.1 - from.0;
    let to_len = to.1 - to.0;
    let moved = (to.0 - from.0).abs() > f64::EPSILON;
    let resized = (to_len - from_len).abs() > f64::EPSILON;
    let place = beats_to_place(ctx, to.0, Some(to.1));
    let (verb, icon) = match (moved, resized) {
        (true, false) => ("Moved clip ", Icon::Move),
        (false, _) if to_len < from_len => ("Shortened clip ", Icon::Trim),
        (false, _) => ("Lengthened clip ", Icon::Trim),
        (true, true) => ("Moved and resized clip ", Icon::Move),
    };
    Builder::new()
        .plain(verb)
        .name(SpanKind::Region, label)
        .plain(" on ")
        .name(SpanKind::Track, track)
        .finish(icon, tier)
        .on(track)
        .at(place)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIVE: SentenceContext = SentenceContext {
        tier: Tier::Semantic,
        beats_per_bar: Some(4.0),
    };

    fn text(r: ChangeRecord) -> String {
        sentence(&r, &LIVE).text
    }

    #[test]
    fn spans_always_concatenate_to_the_text() {
        let s = sentence(
            &ChangeRecord::RegionMoved {
                track: Some("Bass".into()),
                name: "havoc bass".into(),
                from: BarPos::exact(7.0),
                to: BarPos::exact(9.0),
            },
            &LIVE,
        );
        let joined: String = s.spans.iter().map(|x| x.text.as_str()).collect();
        assert_eq!(joined, s.text);
        assert_eq!(s.text, "Moved 'havoc bass' on Bass 2 bars later");
        assert_eq!(s.place_label.as_deref(), Some("now bar 9"));
        assert!(s
            .spans
            .iter()
            .any(|x| x.kind == SpanKind::Value && x.text == "2 bars later"));
    }

    #[test]
    fn tempo_reads_like_a_musician_writes_it() {
        assert_eq!(
            text(ChangeRecord::TempoChanged {
                from_bpm: 120.0,
                to_bpm: 124.0
            }),
            "Tempo 120 → 124 BPM"
        );
    }

    #[test]
    fn volume_is_in_db_and_pan_as_live_shows_it() {
        assert_eq!(gain_to_db(1.0), "0.0 dB");
        assert_eq!(gain_to_db(0.7943282127), "−2.0 dB");
        assert_eq!(gain_to_db(0.0), "−∞ dB");
        assert_eq!(pan_label(-1.0), "50L");
        assert_eq!(pan_label(-0.15), "8L");
        assert_eq!(pan_label(0.0), "C");
        assert_eq!(pan_label(1.0), "50R");
        assert_eq!(
            text(ChangeRecord::MixChanged {
                track: "Rhodes".into(),
                field: MixField::Volume,
                from: MixValue::Num(0.794),
                to: MixValue::Num(0.525),
            }),
            "Volume on Rhodes: −2.0 → −5.6 dB"
        );
    }

    #[test]
    fn ableton_beats_become_one_based_bars() {
        let s = sentence(
            &ChangeRecord::ClipAdded {
                track: "Rhodes".into(),
                label: "verse".into(),
                start_bar: 16.0, // beats
            },
            &LIVE,
        );
        assert_eq!(s.text, "Added clip 'verse' on Rhodes");
        assert_eq!(s.place_label.as_deref(), Some("bar 5"));
        assert_eq!(s.confidence, Confidence::Exact);

        let unknown_meter = SentenceContext {
            tier: Tier::Semantic,
            beats_per_bar: None,
        };
        let s = sentence(
            &ChangeRecord::ClipAdded {
                track: "Rhodes".into(),
                label: "verse".into(),
                start_bar: 16.0,
            },
            &unknown_meter,
        );
        assert_eq!(s.place_label.as_deref(), Some("about bar 5"));
        assert_eq!(s.confidence, Confidence::Approximate);
    }

    #[test]
    fn fractional_bars_label_the_bar_they_fall_in() {
        let s = sentence(
            &ChangeRecord::MarkerAdded {
                name: "Chorus".into(),
                at: Some(BarPos::exact(17.5)),
            },
            &LIVE,
        );
        assert_eq!(s.place_label.as_deref(), Some("bar 17"));
    }

    #[test]
    fn non_finite_numbers_never_become_places() {
        let r = ChangeRecord::ClipAdded {
            track: "Rhodes".into(),
            label: "verse".into(),
            start_bar: f64::NAN,
        };
        assert!(!record_is_finite(&r));
        let s = sentence(&r, &LIVE);
        assert_eq!(s.place, None);
        assert!(!record_is_finite(&ChangeRecord::TempoChanged {
            from_bpm: 120.0,
            to_bpm: f64::INFINITY
        }));
    }

    #[test]
    fn inferred_sentences_say_probably() {
        let s = logic_name_renamed("Audio 7", "Synth arp", Tier::Structure);
        assert_eq!(s.confidence, Confidence::Inferred);
        assert_eq!(s.text, "Probably renamed 'Audio 7' to 'Synth arp'");
        assert_eq!(
            s.track, None,
            "a Logic list name is not known to be a track"
        );
    }

    #[test]
    fn user_names_are_their_own_spans() {
        let s = sentence(
            &ChangeRecord::TrackAdded {
                name: "</b><script>".into(),
                kind: TrackKind::Audio,
            },
            &LIVE,
        );
        assert!(s
            .spans
            .iter()
            .any(|x| x.kind == SpanKind::Track && x.text == "</b><script>"));
        assert!(s
            .spans
            .iter()
            .filter(|x| x.kind == SpanKind::Plain)
            .all(|x| !x.text.contains('<')));
    }
}
