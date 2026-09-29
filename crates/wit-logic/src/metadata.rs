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
//! Untrusted input: every read here is bounded (a plist bigger than
//! [`MAX_PLIST_LEN`] is refused unread) and every field access is defensive
//! — a wrong type, a missing key or a malformed plist yields `None`/`Ok`
//! with fewer fields, never a panic.

use std::io::Cursor;
use std::path::Path;

/// Real `MetaData.plist` files measured this session are 4–16 KB. 8 MiB is
/// generous headroom for a much larger `AudioFiles` list while still
/// refusing anything that looks like it isn't really a small metadata index
/// (AGENTS.md: "Untrusted input... bounded").
pub const MAX_PLIST_LEN: u64 = 8 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetadataError {
    /// The file is larger than [`MAX_PLIST_LEN`] — refused unread.
    TooLarge { len: u64, max: u64 },
    /// Reading the file itself failed (missing, permissions, ...).
    Io(String),
    /// The bytes are not a plist the `plist` crate can parse, or the root
    /// value isn't a dictionary.
    NotAPlist(String),
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

fn read_bounded(path: &Path) -> Result<Vec<u8>, MetadataError> {
    let meta = std::fs::metadata(path).map_err(|e| MetadataError::Io(e.to_string()))?;
    if meta.len() > MAX_PLIST_LEN {
        return Err(MetadataError::TooLarge {
            len: meta.len(),
            max: MAX_PLIST_LEN,
        });
    }
    std::fs::read(path).map_err(|e| MetadataError::Io(e.to_string()))
}

fn parse_plist(bytes: &[u8]) -> Result<plist::Value, MetadataError> {
    plist::Value::from_reader(Cursor::new(bytes))
        .map_err(|e| MetadataError::NotAPlist(e.to_string()))
}

/// A plist integer as a `u32`, whichever signedness it was stored as.
/// `None` on a missing key, a wrong type, or a value that doesn't fit.
fn as_u32(dict: &plist::Dictionary, key: &str) -> Option<u32> {
    let v = dict.get(key)?;
    if let Some(i) = v.as_signed_integer() {
        return u32::try_from(i).ok();
    }
    v.as_unsigned_integer().and_then(|i| u32::try_from(i).ok())
}

fn as_u16(dict: &plist::Dictionary, key: &str) -> Option<u16> {
    as_u32(dict, key).and_then(|v| u16::try_from(v).ok())
}

fn as_string(dict: &plist::Dictionary, key: &str) -> Option<String> {
    dict.get(key)?.as_string().map(str::to_string)
}

fn as_real(dict: &plist::Dictionary, key: &str) -> Option<f64> {
    dict.get(key)?.as_real().filter(|x| x.is_finite())
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
/// large enough for any real project (the measured fixture has 33), small
/// enough that a corrupt or hostile plist can't force an unbounded
/// allocation from a file already under [`MAX_PLIST_LEN`].
const MAX_AUDIO_FILES: usize = 100_000;

fn audio_files(dict: &plist::Dictionary) -> Vec<String> {
    let Some(arr) = dict.get("AudioFiles").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(|v| v.as_string())
        .filter_map(basename)
        .take(MAX_AUDIO_FILES)
        .collect()
}

/// Read one save's `MetaData.plist` (the alternative's own, or a numbered
/// backup slot's — both have the same shape).
pub fn read_metadata_plist(path: &Path) -> Result<ProjectMetadata, MetadataError> {
    read_metadata_plist_bytes(&read_bounded(path)?)
}

/// As [`read_metadata_plist`], from already-read bytes — for a caller that
/// built or fetched the plist in memory (`wit-demo`'s round-trip tests; a
/// future in-memory fixture) rather than reading it off disk. `read_bounded`
/// already applied [`MAX_PLIST_LEN`] before this function ever sees the
/// bytes when called from [`read_metadata_plist`]; a caller passing its own
/// bytes is responsible for its own size bound.
pub fn read_metadata_plist_bytes(bytes: &[u8]) -> Result<ProjectMetadata, MetadataError> {
    let value = parse_plist(bytes)?;
    let dict = value
        .as_dictionary()
        .ok_or_else(|| MetadataError::NotAPlist("root value is not a dictionary".to_string()))?;

    let time_signature = match (
        as_u16(dict, "SongSignatureNumerator"),
        as_u16(dict, "SongSignatureDenominator"),
    ) {
        (Some(numerator), Some(denominator)) if numerator > 0 && denominator > 0 => {
            Some(TimeSignature {
                numerator,
                denominator,
            })
        }
        _ => None,
    };

    Ok(ProjectMetadata {
        number_of_tracks: as_u32(dict, "NumberOfTracks"),
        key: as_string(dict, "SongKey"),
        mode: as_string(dict, "SongGenderKey"),
        time_signature,
        sample_rate: as_u32(dict, "SampleRate"),
        bpm: as_real(dict, "BeatsPerMinute"),
        audio_files: audio_files(dict),
    })
}

/// Read the bundle-level `Resources/ProjectInformation.plist`.
pub fn read_project_information(path: &Path) -> Result<ProjectInformation, MetadataError> {
    let bytes = read_bounded(path)?;
    let value = parse_plist(&bytes)?;
    let dict = value
        .as_dictionary()
        .ok_or_else(|| MetadataError::NotAPlist("root value is not a dictionary".to_string()))?;
    Ok(ProjectInformation {
        last_saved_from: as_string(dict, "LastSavedFrom"),
    })
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
