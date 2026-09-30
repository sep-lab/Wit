//! `MetaData.plist` and `Resources/ProjectInformation.plist` — the facts
//! Logic already states about its own project, so Wit never has to guess
//! them from the `ProjectData` container.
//!
//! `MetaData.plist` sits beside `ProjectData` in every alternative **and in
//! every backup slot** (`Alternatives/NNN/Project File Backups/00../ProjectData`
//! has its own sibling `MetaData.plist`), so a caller reads whichever save it
//! is comparing, not just the current one. `ProjectInformation.plist` is a
//! single file at the bundle root (`Resources/ProjectInformation.plist`),
//! shared by every alternative.
//!
//! Field names below were read directly off a real project's plists this
//! session (`plutil -p`, a temp read-only copy — never `~/Music/Logic`, never
//! a real name in this file): `NumberOfTracks`, `SongKey`, `SongGenderKey`,
//! `SongSignatureNumerator`/`Denominator`, `SampleRate`, `BeatsPerMinute`,
//! `AudioFiles` in `MetaData.plist`; `LastSavedFrom` in
//! `ProjectInformation.plist`. This is a **single real project's spot
//! check**, the same caveat `extract.rs` already carries for its own
//! whitelisted fields — not the 30-fixture corpus issue #3 asks for.
//!
//! **Census-noun ban (ADR-0006).** `NumberOfTracks` here is the DAW's own
//! stated count — never a container record count (`karT`/`Trak`) — which is
//! exactly the distinction ADR-0006 requires before a track count can reach
//! a musician. `AuFl`/`karT` container tallies stay in `census.rs`.
//!
//! Both files are Apple property lists, binary on every real file measured
//! this session (`file(1)`: "Apple binary property list") but read through
//! the `plist` crate's format-sniffing reader, which also accepts XML — a
//! future Logic version or a hand-edited fixture may write either.
//!
//! **A plist is untrusted input, not a fixed-shape file Wit wrote itself.**
//! `plist::Value::from_reader` — this module's first version — recursively
//! builds a tree in memory with no depth or size bound, which a hostile or
//! merely corrupt plist can turn into a crash or a memory bomb: a plist
//! nested tens of thousands of arrays deep overflows the parsing thread's
//! call stack (recursive descent has no heap to spill into), and a binary
//! plist's object table lets one array be *referenced* by several parents,
//! so a few hundred bytes of nested shared references can expand into
//! hundreds of megabytes once flattened — a small file, a large tree, no
//! byte-size check catches it. Neither failure is hypothetical: both were
//! reproduced against this module's first version with a plist dropped into
//! a real backup slot's directory.
//!
//! The fix is to never build that tree at all. Every read here goes through
//! [`BoundedEvents`], a thin wrapper over `plist::stream::Reader` — the
//! crate's flat event-stream API, gated behind its own
//! `enable_unstable_features_that_may_break_with_minor_version_bumps`
//! feature (see `Cargo.toml`'s comment on the dependency) — which walks the
//! plist one `Event` at a time with **no recursion of its own** (its stack
//! lives on the heap, inside the reader), and which this module additionally
//! bounds on two axes as it iterates: [`MAX_DEPTH`] nested
//! arrays/dictionaries, and [`MAX_EVENTS`] total events read. Either bound
//! being hit aborts the read with a typed error — a metadata plist this
//! crate cares about is a shallow dictionary of scalars plus one string
//! array, so both bounds have wide headroom over anything real while still
//! being small enough to fail fast on something hostile. [`MAX_PLIST_LEN`]
//! additionally bounds the raw byte count *before* any of that parsing
//! starts, read via a capped [`std::io::Read::take`] rather than a
//! `metadata()`-then-`read()` pair (a metadata check and the read it gates
//! are two different syscalls; a file that grows between them is a benign
//! version of the same TOCTOU shape a hostile one could exploit on purpose).

use std::io::{Cursor, Read};
use std::path::Path;

use plist::stream::{Event, Reader as StreamReader};

/// Real `MetaData.plist` files measured this session are 4–16 KB. 1 MiB is
/// generous headroom for a much larger `AudioFiles` list while still
/// refusing anything that looks like it isn't really a small metadata index
/// (AGENTS.md: "Untrusted input... bounded"). Lowered from an earlier 8 MiB
/// once bounded parsing made the real defense the event/depth caps below,
/// not the byte count — but the byte count still gates the cost of even
/// starting to parse.
pub const MAX_PLIST_LEN: u64 = 1024 * 1024;

/// However deeply nested a `MetaData.plist`/`ProjectInformation.plist` this
/// module cares about ever needs to be: a dictionary of scalars, one level
/// of which (`AudioFiles`) is an array of strings. 32 is generous headroom
/// over that shape while still failing fast on a plist nested far beyond
/// anything a real metadata index would be.
const MAX_DEPTH: usize = 32;

/// A bound on the total number of stream events one read may consume.
/// `MetaData.plist` today has on the order of 10 keys plus one
/// `AudioFiles` entry per audio file (a few dozen on the measured project);
/// this is generous over any real file while still aborting quickly against
/// a plist whose binary object table expands, once flattened into events,
/// into far more than a metadata index could ever legitimately need.
const MAX_EVENTS: usize = 20_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetadataError {
    /// The file is larger than [`MAX_PLIST_LEN`] — refused unread. `len` is
    /// only a lower bound when read via [`read_bounded`], which stops
    /// reading at `MAX_PLIST_LEN + 1` bytes rather than measuring the whole
    /// file first.
    TooLarge { len: u64, max: u64 },
    /// Reading the file itself failed (missing, permissions, ...).
    Io(String),
    /// The bytes are not a plist the `plist` crate can parse, or the root
    /// value isn't a dictionary.
    NotAPlist(String),
    /// [`MAX_DEPTH`] or [`MAX_EVENTS`] was exceeded while reading — refused
    /// rather than risk the unbounded recursion/expansion a hostile plist
    /// can otherwise force (see the module doc).
    TooComplex(&'static str),
}

impl std::fmt::Display for MetadataError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MetadataError::TooLarge { len, max } => {
                write!(
                    f,
                    "{len} bytes is over the {max}-byte plist bound — refusing to read"
                )
            }
            MetadataError::Io(msg) => write!(f, "failed to read plist: {msg}"),
            MetadataError::NotAPlist(msg) => write!(f, "not a readable plist: {msg}"),
            MetadataError::TooComplex(bound) => {
                write!(f, "plist exceeded Wit's {bound} bound — refusing to read")
            }
        }
    }
}

impl std::error::Error for MetadataError {}

/// A project's `SongSignatureNumerator`/`SongSignatureDenominator`, read
/// verbatim — never validated against a real time signature, since Wit
/// should surface whatever Logic wrote rather than silently substituting
/// 4/4.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeSignature {
    pub numerator: u16,
    pub denominator: u16,
}

/// What one save's `MetaData.plist` states about the project. Every field is
/// `None`/empty when the plist doesn't have it — never guessed, never
/// defaulted to a container count.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ProjectMetadata {
    /// `NumberOfTracks` — the DAW's own stated track count (ADR-0006).
    pub number_of_tracks: Option<u32>,
    /// `SongKey`, e.g. `"C"`. `None` when the project doesn't state one.
    pub key: Option<String>,
    /// `SongGenderKey`, e.g. `"major"`/`"minor"`.
    pub mode: Option<String>,
    pub time_signature: Option<TimeSignature>,
    /// `SampleRate`, in Hz.
    pub sample_rate: Option<u32>,
    /// `BeatsPerMinute`.
    pub bpm: Option<f64>,
    /// `AudioFiles`, basenames only (the plist stores
    /// `"Audio Files/<name>"` package-relative paths; Wit never carries a
    /// path past this function — see `wit-story`'s "no paths" rule).
    pub audio_files: Vec<String>,
}

/// What `Resources/ProjectInformation.plist` states about the whole bundle
/// (shared by every alternative).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ProjectInformation {
    /// `LastSavedFrom`, e.g. `"Logic Pro Creator Studio 12.2 (6644)"`.
    pub last_saved_from: Option<String>,
}

/// Read `path`, capped at `MAX_PLIST_LEN + 1` bytes via [`Read::take`] —
/// never a `metadata()`-then-`read()` pair, which checks a size and reads
/// the file in two separate syscalls a file could grow between (a benign
/// version of the same TOCTOU shape a hostile one could exploit on
/// purpose). A file at or under the cap reads in full; anything longer
/// reads exactly `MAX_PLIST_LEN + 1` bytes and is then refused — Wit never
/// buffers more than one byte past its own limit to find that out.
fn read_bounded(path: &Path) -> Result<Vec<u8>, MetadataError> {
    let file = std::fs::File::open(path).map_err(|e| MetadataError::Io(e.to_string()))?;
    let mut buf = Vec::new();
    file.take(MAX_PLIST_LEN + 1)
        .read_to_end(&mut buf)
        .map_err(|e| MetadataError::Io(e.to_string()))?;
    check_len(buf.len() as u64)?;
    Ok(buf)
}

fn check_len(len: u64) -> Result<(), MetadataError> {
    if len > MAX_PLIST_LEN {
        return Err(MetadataError::TooLarge {
            len,
            max: MAX_PLIST_LEN,
        });
    }
    Ok(())
}

/// A `plist::stream::Reader` wrapped with the two bounds the module doc
/// describes: [`MAX_DEPTH`] nested arrays/dictionaries, and [`MAX_EVENTS`]
/// total events read. Every event this module consumes goes through
/// [`BoundedEvents::next`] — there is no other way to pull an event out of
/// the underlying reader from this module — so those two bounds hold for
/// every read regardless of which field-extraction function is walking.
struct BoundedEvents<R: Read + std::io::Seek> {
    reader: StreamReader<R>,
    events_seen: usize,
    depth: usize,
}

impl<R: Read + std::io::Seek> BoundedEvents<R> {
    fn new(reader: R) -> Self {
        BoundedEvents {
            reader: StreamReader::new(reader),
            events_seen: 0,
            depth: 0,
        }
    }

    /// The next event. `Ok(None)` at a clean end of stream; bounded errors
    /// before an unbounded one from the underlying reader ever could be.
    fn next(&mut self) -> Result<Option<Event<'static>>, MetadataError> {
        self.events_seen += 1;
        if self.events_seen > MAX_EVENTS {
            return Err(MetadataError::TooComplex("total event count"));
        }
        let Some(result) = self.reader.next() else {
            return Ok(None);
        };
        let event = result.map_err(|e| MetadataError::NotAPlist(e.to_string()))?;
        match &event {
            Event::StartArray(_) | Event::StartDictionary(_) => {
                self.depth += 1;
                if self.depth > MAX_DEPTH {
                    return Err(MetadataError::TooComplex("nesting depth"));
                }
            }
            Event::EndCollection => self.depth = self.depth.saturating_sub(1),
            _ => {}
        }
        Ok(Some(event))
    }

    /// Skip a value already begun by `first` — a no-op for a scalar
    /// (already fully consumed by reading it), or every event up to and
    /// including the matching [`Event::EndCollection`] for a collection.
    /// Used for every plist key this module doesn't care about, so the
    /// event stream stays correctly positioned for the key that follows.
    fn skip_value(&mut self, first: &Event<'static>) -> Result<(), MetadataError> {
        match first {
            Event::StartArray(_) | Event::StartDictionary(_) => self.skip_collection(),
            _ => Ok(()),
        }
    }

    fn skip_collection(&mut self) -> Result<(), MetadataError> {
        let mut open = 1usize;
        while open > 0 {
            match self.next()? {
                None => {
                    return Err(MetadataError::NotAPlist(
                        "plist ended inside a collection".to_string(),
                    ))
                }
                Some(Event::StartArray(_)) | Some(Event::StartDictionary(_)) => open += 1,
                Some(Event::EndCollection) => open -= 1,
                Some(_) => {}
            }
        }
        Ok(())
    }
}

fn as_u32_event(event: &Event<'static>) -> Option<u32> {
    let Event::Integer(i) = event else {
        return None;
    };
    i.as_signed()
        .and_then(|v| u32::try_from(v).ok())
        .or_else(|| i.as_unsigned().and_then(|v| u32::try_from(v).ok()))
}

fn as_string_event(event: &Event<'static>) -> Option<String> {
    match event {
        Event::String(s) => Some(s.as_ref().to_string()),
        _ => None,
    }
}

fn as_real_event(event: &Event<'static>) -> Option<f64> {
    match event {
        Event::Real(r) if r.is_finite() => Some(*r),
        _ => None,
    }
}

/// The basename of a package-relative path string, e.g.
/// `"Audio Files/Deep Down Shaker.caf"` -> `"Deep Down Shaker.caf"`. Never a
/// path past this point (`wit-story`'s "no paths" rule).
fn basename(path_str: &str) -> Option<String> {
    let name = Path::new(path_str).file_name()?.to_str()?;
    if name.is_empty() {
        return None;
    }
    Some(name.to_string())
}

/// A generous bound on how many audio files one plist entry can list —
/// large enough for any real project (the measured fixture has 33). Mostly
/// redundant with [`MAX_EVENTS`] now (an `AudioFiles` array this long would
/// already have burned most of that budget), kept as an explicit,
/// self-documenting bound on this one field rather than relying only on the
/// shared event budget.
const MAX_AUDIO_FILES: usize = 10_000;

/// Read an already-opened `AudioFiles` array (the `StartArray` event has
/// been consumed by the caller) to its `EndCollection`, keeping only valid
/// string basenames, up to [`MAX_AUDIO_FILES`].
fn read_string_array<R: Read + std::io::Seek>(
    ev: &mut BoundedEvents<R>,
) -> Result<Vec<String>, MetadataError> {
    let mut out = Vec::new();
    loop {
        match ev.next()? {
            None => {
                return Err(MetadataError::NotAPlist(
                    "plist ended inside AudioFiles".to_string(),
                ))
            }
            Some(Event::EndCollection) => break,
            Some(Event::String(s)) => {
                if out.len() < MAX_AUDIO_FILES {
                    if let Some(name) = basename(s.as_ref()) {
                        out.push(name);
                    }
                }
            }
            Some(other) => ev.skip_value(&other)?,
        }
    }
    Ok(out)
}

/// Read one save's `MetaData.plist` (the alternative's own, or a numbered
/// backup slot's — both have the same shape).
pub fn read_metadata_plist(path: &Path) -> Result<ProjectMetadata, MetadataError> {
    read_metadata_plist_bytes(&read_bounded(path)?)
}

/// As [`read_metadata_plist`], from already-read bytes — for a caller that
/// built or fetched the plist in memory (`wit-demo`'s round-trip tests, the
/// proptests below) rather than reading it off disk. Bounded the same way
/// [`read_metadata_plist`] is regardless: a byte-length check up front, then
/// [`BoundedEvents`]' depth/event caps for everything after.
pub fn read_metadata_plist_bytes(bytes: &[u8]) -> Result<ProjectMetadata, MetadataError> {
    check_len(bytes.len() as u64)?;
    let mut ev = BoundedEvents::new(Cursor::new(bytes));
    match ev.next()? {
        Some(Event::StartDictionary(_)) => {}
        _ => {
            return Err(MetadataError::NotAPlist(
                "root value is not a dictionary".to_string(),
            ))
        }
    }

    let mut meta = ProjectMetadata::default();
    let mut numerator: Option<u16> = None;
    let mut denominator: Option<u16> = None;

    loop {
        let key = match ev.next()? {
            None | Some(Event::EndCollection) => break,
            Some(Event::String(s)) => s.into_owned(),
            Some(other) => {
                return Err(MetadataError::NotAPlist(format!(
                    "expected a dictionary key, found {other:?}"
                )))
            }
        };
        let value = ev
            .next()?
            .ok_or_else(|| MetadataError::NotAPlist("plist ended mid-dictionary".to_string()))?;
        match key.as_str() {
            "NumberOfTracks" => meta.number_of_tracks = as_u32_event(&value),
            "SongKey" => meta.key = as_string_event(&value),
            "SongGenderKey" => meta.mode = as_string_event(&value),
            "SongSignatureNumerator" => {
                numerator = as_u32_event(&value).and_then(|v| u16::try_from(v).ok())
            }
            "SongSignatureDenominator" => {
                denominator = as_u32_event(&value).and_then(|v| u16::try_from(v).ok())
            }
            "SampleRate" => meta.sample_rate = as_u32_event(&value),
            "BeatsPerMinute" => meta.bpm = as_real_event(&value),
            "AudioFiles" if matches!(value, Event::StartArray(_)) => {
                meta.audio_files = read_string_array(&mut ev)?;
            }
            _ => ev.skip_value(&value)?,
        }
    }

    meta.time_signature = match (numerator, denominator) {
        (Some(numerator), Some(denominator)) if numerator > 0 && denominator > 0 => {
            Some(TimeSignature {
                numerator,
                denominator,
            })
        }
        _ => None,
    };
    Ok(meta)
}

/// Read the bundle-level `Resources/ProjectInformation.plist`.
pub fn read_project_information(path: &Path) -> Result<ProjectInformation, MetadataError> {
    read_project_information_bytes(&read_bounded(path)?)
}

/// As [`read_project_information`], from already-read bytes — see
/// [`read_metadata_plist_bytes`]'s doc for why this exists and how it stays
/// bounded.
pub fn read_project_information_bytes(bytes: &[u8]) -> Result<ProjectInformation, MetadataError> {
    check_len(bytes.len() as u64)?;
    let mut ev = BoundedEvents::new(Cursor::new(bytes));
    match ev.next()? {
        Some(Event::StartDictionary(_)) => {}
        _ => {
            return Err(MetadataError::NotAPlist(
                "root value is not a dictionary".to_string(),
            ))
        }
    }

    let mut info = ProjectInformation::default();
    loop {
        let key = match ev.next()? {
            None | Some(Event::EndCollection) => break,
            Some(Event::String(s)) => s.into_owned(),
            Some(other) => {
                return Err(MetadataError::NotAPlist(format!(
                    "expected a dictionary key, found {other:?}"
                )))
            }
        };
        let value = ev
            .next()?
            .ok_or_else(|| MetadataError::NotAPlist("plist ended mid-dictionary".to_string()))?;
        match key.as_str() {
            "LastSavedFrom" => info.last_saved_from = as_string_event(&value),
            _ => ev.skip_value(&value)?,
        }
    }
    Ok(info)
}

/// The newest Logic major version this codebase's Logic mappings have been
/// verified against — **[cited]** `docs/EXPERIMENTS.md` §12: "n = 1 library,
/// one person's projects, all Logic 12." A `LastSavedFrom` naming a higher
/// major version is Wit's one early-warning signal that a save may use a
/// container or payload shape nothing here has seen.
pub const KNOWN_MAX_MAJOR_VERSION: u32 = 12;

/// Parse the leading `major` version number out of a `LastSavedFrom` string
/// such as `"Logic Pro Creator Studio 12.2 (6644)"` — the first
/// whitespace-delimited token that looks like `<digits>.<digits>...`.
/// Returns `None` rather than guessing when no token has that shape.
pub fn parse_major_version(last_saved_from: &str) -> Option<u32> {
    last_saved_from.split_whitespace().find_map(|token| {
        let major = token.split('.').next()?;
        if !major.is_empty() && major.bytes().all(|b| b.is_ascii_digit()) {
            major.parse::<u32>().ok()
        } else {
            None
        }
    })
}

/// True when `last_saved_from` names a Logic major version newer than
/// [`KNOWN_MAX_MAJOR_VERSION`]. `false` when the string doesn't parse — an
/// unparseable string is not evidence of a newer version, only of a shape
/// Wit hasn't catalogued, and this function makes no claim either way about
/// that.
pub fn is_newer_than_known(last_saved_from: &str) -> bool {
    parse_major_version(last_saved_from).is_some_and(|major| major > KNOWN_MAX_MAJOR_VERSION)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_plist(path: &Path, dict: plist::Dictionary) {
        plist::Value::Dictionary(dict)
            .to_writer_binary(std::fs::File::create(path).unwrap())
            .unwrap();
    }

    fn sample_metadata_dict() -> plist::Dictionary {
        let mut d = plist::Dictionary::new();
        d.insert("NumberOfTracks".into(), plist::Value::Integer(31.into()));
        d.insert("SongKey".into(), plist::Value::String("C".into()));
        d.insert("SongGenderKey".into(), plist::Value::String("major".into()));
        d.insert(
            "SongSignatureNumerator".into(),
            plist::Value::Integer(4.into()),
        );
        d.insert(
            "SongSignatureDenominator".into(),
            plist::Value::Integer(4.into()),
        );
        d.insert("SampleRate".into(), plist::Value::Integer(48000.into()));
        d.insert("BeatsPerMinute".into(), plist::Value::Real(122.0028));
        d.insert(
            "AudioFiles".into(),
            plist::Value::Array(vec![
                plist::Value::String("Audio Files/Deep Down Shaker.caf".into()),
                plist::Value::String("Audio Files/Windmill Synth.caf".into()),
            ]),
        );
        d
    }

    #[test]
    fn reads_every_whitelisted_field_from_a_binary_plist() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("MetaData.plist");
        write_plist(&path, sample_metadata_dict());

        let meta = read_metadata_plist(&path).unwrap();
        assert_eq!(meta.number_of_tracks, Some(31));
        assert_eq!(meta.key.as_deref(), Some("C"));
        assert_eq!(meta.mode.as_deref(), Some("major"));
        assert_eq!(
            meta.time_signature,
            Some(TimeSignature {
                numerator: 4,
                denominator: 4
            })
        );
        assert_eq!(meta.sample_rate, Some(48000));
        assert_eq!(meta.bpm, Some(122.0028));
        assert_eq!(
            meta.audio_files,
            vec![
                "Deep Down Shaker.caf".to_string(),
                "Windmill Synth.caf".to_string()
            ]
        );
    }

    #[test]
    fn reads_an_xml_plist_the_same_way() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("MetaData.plist");
        plist::Value::Dictionary(sample_metadata_dict())
            .to_writer_xml(std::fs::File::create(&path).unwrap())
            .unwrap();

        let meta = read_metadata_plist(&path).unwrap();
        assert_eq!(meta.number_of_tracks, Some(31));
        assert_eq!(meta.bpm, Some(122.0028));
    }

    #[test]
    fn missing_fields_are_none_not_a_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("MetaData.plist");
        write_plist(&path, plist::Dictionary::new());

        let meta = read_metadata_plist(&path).unwrap();
        assert_eq!(meta, ProjectMetadata::default());
    }

    #[test]
    fn a_wrong_typed_field_is_none_not_a_panic() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("MetaData.plist");
        let mut d = plist::Dictionary::new();
        d.insert(
            "NumberOfTracks".into(),
            plist::Value::String("not a number".into()),
        );
        write_plist(&path, d);

        let meta = read_metadata_plist(&path).unwrap();
        assert_eq!(meta.number_of_tracks, None);
    }

    #[test]
    fn a_zero_time_signature_field_is_rejected_not_kept() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("MetaData.plist");
        let mut d = plist::Dictionary::new();
        d.insert(
            "SongSignatureNumerator".into(),
            plist::Value::Integer(0.into()),
        );
        d.insert(
            "SongSignatureDenominator".into(),
            plist::Value::Integer(4.into()),
        );
        write_plist(&path, d);

        assert_eq!(read_metadata_plist(&path).unwrap().time_signature, None);
    }

    #[test]
    fn an_oversized_plist_is_refused_unread() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("MetaData.plist");
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(&vec![0u8; (MAX_PLIST_LEN + 1) as usize])
            .unwrap();

        assert!(matches!(
            read_metadata_plist(&path),
            Err(MetadataError::TooLarge { .. })
        ));
    }

    #[test]
    fn an_oversized_byte_slice_is_refused_even_off_a_file() {
        let bytes = vec![0u8; (MAX_PLIST_LEN + 1) as usize];
        assert!(matches!(
            read_metadata_plist_bytes(&bytes),
            Err(MetadataError::TooLarge { .. })
        ));
    }

    // ---------------------------------------------------------------- //
    // hostile plists — bounded parsing, never a crash or a memory bomb
    // ---------------------------------------------------------------- //

    const XML_PLIST_HEADER: &str = concat!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
        "<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" ",
        "\"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n",
        "<plist version=\"1.0\">",
    );

    fn nested_xml_dict(depth: usize) -> String {
        let mut s = String::from(XML_PLIST_HEADER);
        s.push_str("<dict><key>Bomb</key>");
        for _ in 0..depth {
            s.push_str("<array>");
        }
        s.push_str("<true/>");
        for _ in 0..depth {
            s.push_str("</array>");
        }
        s.push_str("</dict></plist>");
        s
    }

    fn wide_xml_array(count: usize) -> String {
        let mut s = String::from(XML_PLIST_HEADER);
        s.push_str("<dict><key>Bomb</key><array>");
        for _ in 0..count {
            s.push_str("<string>x</string>");
        }
        s.push_str("</array></dict></plist>");
        s
    }

    #[test]
    fn a_plist_nested_past_the_depth_bound_is_refused_not_a_stack_overflow() {
        // 40 levels of plain nesting is nowhere near the 100k-level input
        // that overflowed a 2 MiB thread stack against this module's first
        // version (which parsed into a recursively-built `Value` tree) —
        // it only needs to clear `MAX_DEPTH` (32) to prove the bound, not
        // reproduce the original crash at its original scale.
        let xml = nested_xml_dict(40);
        assert!(matches!(
            read_metadata_plist_bytes(xml.as_bytes()),
            Err(MetadataError::TooComplex("nesting depth"))
        ));
    }

    #[test]
    fn a_plist_within_the_depth_bound_still_reads() {
        let xml = nested_xml_dict(MAX_DEPTH - 5);
        // "Bomb" isn't a key this module reads, so this should read clean
        // through to an otherwise-empty, successfully-parsed result — the
        // point is that a legitimately shallow structure isn't punished by
        // the same bound that refuses a hostile one.
        assert_eq!(
            read_metadata_plist_bytes(xml.as_bytes()).unwrap(),
            ProjectMetadata::default()
        );
    }

    #[test]
    fn a_wide_flat_array_is_refused_by_total_event_count() {
        // No nesting at all here (every `<string>` is a sibling, so depth
        // never grows) — only the total-event bound can catch this, which
        // is exactly the point: a depth cap alone would let it through.
        let xml = wide_xml_array(30_000);
        assert!(matches!(
            read_metadata_plist_bytes(xml.as_bytes()),
            Err(MetadataError::TooComplex("total event count"))
        ));
    }

    /// A hostile **binary** plist whose object table shares one small array
    /// across `LEVELS` levels of doubling (`array[k] = [array[k-1],
    /// array[k-1]]`) — the same "small file, huge tree" shape a 572-byte
    /// real-world plist was measured to expand into 634 MB against this
    /// module's first version, which read into a `Value` tree with no
    /// bound on how many times a shared object could be re-expanded.
    /// Fully expanding `LEVELS` levels produces on the order of `3 *
    /// 2^LEVELS` events; at `LEVELS = 15` that's ~98,000, comfortably over
    /// [`MAX_EVENTS`] — while every array is only ever 2 elements deep, so
    /// [`MAX_DEPTH`] is never what actually catches this one, proving the
    /// event-count bound is doing independent work, not just riding behind
    /// the depth bound.
    fn hostile_shared_reference_bomb() -> Vec<u8> {
        const LEVELS: usize = 15;
        let mut objects: Vec<Vec<u8>> = Vec::new();
        objects.push(vec![0x51, b'x']); // object 0: ASCII string "x"
        for level in 1..=LEVELS {
            let child = (level - 1) as u8;
            objects.push(vec![0xA2, child, child]); // object `level`: [child, child]
        }
        let bomb_idx = LEVELS as u8;
        objects.push(vec![0x54, b'B', b'o', b'm', b'b']); // the dict key "Bomb"
        let key_idx = (LEVELS + 1) as u8;
        objects.push(vec![0xD1, key_idx, bomb_idx]); // {"Bomb": array[LEVELS]}
        let root_idx = (LEVELS + 2) as u64;

        let mut out = Vec::new();
        out.extend_from_slice(b"bplist00");
        let mut offsets = Vec::new();
        for obj in &objects {
            offsets.push(out.len() as u8);
            out.extend_from_slice(obj);
        }
        let offset_table_start = out.len() as u64;
        for &off in &offsets {
            out.push(off);
        }
        // Trailer (32 bytes): 6 unused, offset-int-size, object-ref-size,
        // object count (BE u64), root object index (BE u64), offset table
        // start (BE u64). Every offset/ref here fits in one byte (the
        // whole file is under 256 bytes), matching the `1, 1` sizes below.
        out.extend_from_slice(&[0u8; 6]);
        out.push(1);
        out.push(1);
        out.extend_from_slice(&(objects.len() as u64).to_be_bytes());
        out.extend_from_slice(&root_idx.to_be_bytes());
        out.extend_from_slice(&offset_table_start.to_be_bytes());
        out
    }

    #[test]
    fn a_shared_reference_bomb_is_capped_by_event_count_not_by_expanding_it() {
        let bytes = hostile_shared_reference_bomb();
        assert!(
            bytes.len() < 200,
            "the hostile file itself must stay tiny: {} bytes",
            bytes.len()
        );
        assert!(matches!(
            read_metadata_plist_bytes(&bytes),
            Err(MetadataError::TooComplex("total event count"))
        ));
    }

    #[test]
    fn a_missing_file_is_a_typed_io_error_not_a_panic() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(
            read_metadata_plist(&dir.path().join("nope.plist")),
            Err(MetadataError::Io(_))
        ));
    }

    #[test]
    fn garbage_bytes_are_a_typed_error_not_a_panic() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("MetaData.plist");
        std::fs::write(&path, b"not a plist at all").unwrap();
        assert!(matches!(
            read_metadata_plist(&path),
            Err(MetadataError::NotAPlist(_))
        ));
    }

    #[test]
    fn reads_last_saved_from_out_of_project_information() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ProjectInformation.plist");
        let mut d = plist::Dictionary::new();
        d.insert(
            "LastSavedFrom".into(),
            plist::Value::String("Logic Pro Creator Studio 12.2 (6644)".into()),
        );
        write_plist(&path, d);

        let info = read_project_information(&path).unwrap();
        assert_eq!(
            info.last_saved_from.as_deref(),
            Some("Logic Pro Creator Studio 12.2 (6644)")
        );
    }

    #[test]
    fn parses_the_major_version_out_of_last_saved_from() {
        assert_eq!(
            parse_major_version("Logic Pro Creator Studio 12.2 (6644)"),
            Some(12)
        );
        assert_eq!(parse_major_version("Logic Pro 10.7.9"), Some(10));
        assert_eq!(parse_major_version("GarageBand"), None);
        assert_eq!(parse_major_version(""), None);
    }

    #[test]
    fn flags_only_a_major_version_above_the_known_one() {
        assert!(!is_newer_than_known("Logic Pro Creator Studio 12.2 (6644)"));
        assert!(!is_newer_than_known("Logic Pro 10.7.9"));
        assert!(is_newer_than_known("Logic Pro Creator Studio 13.0 (7000)"));
        assert!(!is_newer_than_known("a future Logic with no version word"));
    }
}
