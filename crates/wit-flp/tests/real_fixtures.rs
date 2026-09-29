//! Opt-in real-material check: walk every `.flp` under `WIT_FIXTURES` and
//! assert `wit_flp::parse` never panics and reports a clean parse (or a
//! typed error, never a crash). Mirrors the `WIT_FIXTURES` discipline
//! `tests/conftest.py` and `wit-diff`/`wit-logic`'s own real-fixture tests
//! already follow: **loudly skipped** by default, never touches real
//! material unless asked, never reads a path from anywhere but this env
//! var (this crate's own source never hardcodes a personal path).
//!
//! Beyond "parses clean", it checks the channel names two ways, because a
//! count that matches `FLhd.channels` cannot catch a wrong name:
//!
//! - **Pinned name lists** for a few of the demo projects and templates
//!   Image-Line ships inside FL Studio 20's app bundle
//!   (`FL Studio 20.app/Contents/Resources/FL/Data`), checked whenever one
//!   of them (matched by file name *and* declared FL version) is under
//!   `WIT_FIXTURES`.
//! - **The same song saved in both channel-name schemes.** The bundle
//!   ships two demo songs twice: once saved by FL 11.0/11.1 (names on id
//!   192) and once re-saved by FL 12.3 (names on each channel's header
//!   203). Every name read from the older save must appear among the newer
//!   save's names.
//!
//! Run recursively against a real FL Studio library:
//!
//! ```text
//! WIT_FIXTURES=/path/to/FL/library cargo test -p wit-flp --test real_fixtures -- --nocapture --ignored
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn fixtures_dir() -> Option<PathBuf> {
    std::env::var_os("WIT_FIXTURES").map(PathBuf::from)
}

/// Recursively collect every `.flp` file under `root` — real FL libraries
/// nest project files and `Backup/` autosave folders arbitrarily deep.
/// Depth-capped defensively against a symlink cycle, matching
/// `wit-index::discover`'s own walk.
fn find_flps(root: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    const MAX_DEPTH: usize = 16;
    if depth > MAX_DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.is_dir() {
            find_flps(&path, depth + 1, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("flp") {
            out.push(path);
        }
    }
}

/// A bundled FL Studio 20 file whose channel names are pinned.
struct Pin {
    file_name: &'static str,
    fl_version: &'static str,
    /// Every channel block, named or not.
    rack_len: usize,
    /// The first names, in rack order (all of them for small projects).
    first_names: &'static [&'static str],
    named: usize,
    /// The first effects, in mixer order, when pinned.
    first_effects: &'static [(&'static str, Option<u16>)],
}

const PINS: &[Pin] = &[
    // The review's reproduction: one default Sampler channel with no name
    // of its own; the first 203 after it is the Master insert's "Control
    // Surface" plugin, renamed "Mix". No channel name may be read.
    Pin {
        file_name: "Surround mix.flp",
        fl_version: "20.7.0.1702",
        rack_len: 1,
        first_names: &[],
        named: 0,
        first_effects: &[
            ("Control Surface", Some(0)),
            ("Fruity X-Y Controller", Some(1)),
        ],
    },
    Pin {
        file_name: "Empty with 4 sends.flp",
        fl_version: "20.7.1.1773",
        rack_len: 1,
        first_names: &[],
        named: 0,
        first_effects: &[],
    },
    Pin {
        file_name: "Basic 808.flp",
        fl_version: "20.7.0.1702",
        rack_len: 4,
        first_names: &["808 Kick", "808 Clap", "808 HiHat", "808 Snare"],
        named: 4,
        first_effects: &[],
    },
    Pin {
        file_name: "Basic with limiter.flp",
        fl_version: "20.7.0.1702",
        rack_len: 4,
        first_names: &["Kick", "Clap", "Hat", "Snare"],
        named: 4,
        first_effects: &[("Fruity Limiter", Some(0))],
    },
    Pin {
        file_name: "NewStuff.flp",
        fl_version: "20.8.0.1377",
        rack_len: 144,
        first_names: &[
            "Sytrus Juno Bass",
            "BassDrum",
            "BassDrum .2",
            "SynthFX",
            "SynthRexv2",
            "WNoise",
            "SynthFX #3",
            "SynthFX #2",
            "SynthFX #2",
            "UPFx",
            "SynthFX #3",
            "SynthFX",
        ],
        named: 144,
        first_effects: &[
            ("Fruity Parametric EQ 2", Some(0)),
            ("Fruity Parametric EQ 2", Some(0)),
            ("Maximus", Some(0)),
        ],
    },
    // The first FL 11.5 file seen: the new scheme's lower boundary.
    Pin {
        file_name: "Raubana - LIFE.flp",
        fl_version: "11.5.14",
        rack_len: 24,
        first_names: &[
            "Master Volume",
            "Main Break",
            "Break Break",
            "Shaker",
            "Drum Low Pass",
            "Drum High Pass",
            "Drum Fill",
            "Rev Drum Fill",
            "Tappy Hats",
            "Tappy Hats Stereo",
            "Tappy Hats Pan",
            "Tappy Hats EQ",
            "Soft Pads",
            "Soft Pads Volume",
            "Sub",
            "Sub Volume",
            "Phased Sub-Bass",
            "Phased Bass",
            "Piano (Layer)",
            "Piano",
            "Tiny Piano",
            "Tiny Piano Volume",
            "Plucked",
            "Raubana",
        ],
        named: 24,
        first_effects: &[("Maximus", Some(0)), ("Fruity Reeverb 2", Some(1))],
    },
    // Pre-11.5 (id 192) files.
    Pin {
        file_name: "Vocodex demo.flp",
        fl_version: "8.5.0",
        rack_len: 7,
        first_names: &[
            "Modulator #2",
            "Modulator",
            "PWM",
            "Supersaw",
            "Saw",
            "Ensemble",
            "Phased 2",
        ],
        named: 7,
        first_effects: &[("Vocodex", Some(2))],
    },
    Pin {
        file_name: "AuraQualic - DATA (FL Studio Remix).flp",
        fl_version: "10.0.0",
        rack_len: 43,
        first_names: &[
            "Layer #2",
            "DM-BD 0021",
            "DM-BD 0014",
            "DNC_Kick #4",
            "kick_puch #2",
            "DNC_Kick",
            "DNC_Kick #2",
            "Percloop",
        ],
        named: 43,
        first_effects: &[("Fruity Wrapper", Some(0)), ("Maximus", Some(0))],
    },
];

/// (older save, names on 192; newer re-save, names on the header 203).
const SAME_SONG_BOTH_SCHEMES: &[(&str, &str)] = &[
    (
        "SeamlessR - Menagerie.flp",
        "JuiceBass - SeamlessR - Menagerie.flp",
    ),
    (
        "Gimbal & Sinan + Futorial - RawFL.flp",
        "Treben - Gimbal and Sinan - RawFL.flp",
    ),
];

#[test]
#[ignore = "opt-in: set WIT_FIXTURES to a real FL Studio library and pass --ignored"]
fn real_library_walks_clean_and_reads_the_right_names() {
    let Some(dir) = fixtures_dir() else {
        eprintln!(
            "WIT_FIXTURES not set — skipped. To run: \
             WIT_FIXTURES=/path/to/FL/library cargo test -p wit-flp --test real_fixtures -- --nocapture --ignored"
        );
        return;
    };

    let mut flps = Vec::new();
    find_flps(&dir, 0, &mut flps);
    flps.sort();

    assert!(!flps.is_empty(), "no .flp files found under {dir:?}");

    let mut clean = 0usize;
    let mut errors: Vec<(PathBuf, wit_flp::FlpError)> = Vec::new();
    let mut versions: BTreeMap<String, usize> = BTreeMap::new();
    let mut v25_plus = 0usize;
    let mut with_tempo = 0usize;
    let mut rack_matches_header = 0usize;
    let mut names_match_header = 0usize;
    let mut short: Vec<String> = Vec::new();
    let mut by_name: BTreeMap<String, wit_flp::Extracted> = BTreeMap::new();

    for path in &flps {
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("  SKIP (unreadable): {}: {e}", path.display());
                continue;
            }
        };
        let file_name = path.file_name().unwrap().to_string_lossy().into_owned();
        // Never panic on real material -- a parse failure is a finding
        // (recorded and reported), not a test-harness crash.
        match wit_flp::parse(&bytes) {
            Ok(extracted) => {
                clean += 1;
                let version = extracted
                    .fl_version
                    .clone()
                    .unwrap_or_else(|| "?".to_string());
                *versions.entry(version).or_default() += 1;
                if extracted.format_status == wit_flp::FormatStatus::PartialV25ScalarsUnreadable {
                    v25_plus += 1;
                }
                if matches!(extracted.tempo, wit_flp::Tempo::Known(_)) {
                    with_tempo += 1;
                }
                let named = extracted.channel_names().len();
                if extracted.channel_rack.len() == usize::from(extracted.channels) {
                    rack_matches_header += 1;
                }
                if named == usize::from(extracted.channels) {
                    names_match_header += 1;
                } else {
                    short.push(format!(
                        "{file_name}: {named} of {} named",
                        extracted.channels
                    ));
                }
                eprintln!(
                    "  OK    {file_name}: version={:?} channels={} rack={} named={} \
                     generators={} effects={} patterns={} tempo={}",
                    extracted.fl_version,
                    extracted.channels,
                    extracted.channel_rack.len(),
                    named,
                    extracted.generator_names().len(),
                    extracted.mixer_effects.len(),
                    extracted.pattern_names.len(),
                    extracted.tempo,
                );
                by_name.insert(file_name, extracted);
            }
            Err(e) => {
                eprintln!("  ERROR {file_name}: {e}");
                errors.push((path.clone(), e));
            }
        }
    }

    // Pinned name lists.
    let mut pins_checked = 0usize;
    for pin in PINS {
        let Some(e) = by_name
            .get(pin.file_name)
            .filter(|e| e.fl_version.as_deref() == Some(pin.fl_version))
        else {
            eprintln!("  PIN not under WIT_FIXTURES: {}", pin.file_name);
            continue;
        };
        pins_checked += 1;
        let names = e.channel_names();
        assert_eq!(e.channel_rack.len(), pin.rack_len, "{}", pin.file_name);
        assert_eq!(names.len(), pin.named, "{}", pin.file_name);
        assert_eq!(
            &names[..pin.first_names.len()],
            pin.first_names,
            "{}",
            pin.file_name
        );
        let effects: Vec<(&str, Option<u16>)> = e
            .mixer_effects
            .iter()
            .take(pin.first_effects.len())
            .map(|m| (m.name.as_str(), m.insert))
            .collect();
        assert_eq!(effects, pin.first_effects, "{}", pin.file_name);
    }

    // Same song, both schemes.
    let mut pairs_checked = 0usize;
    for (older, newer) in SAME_SONG_BOTH_SCHEMES {
        let (Some(old), Some(new)) = (by_name.get(*older), by_name.get(*newer)) else {
            eprintln!("  PAIR not under WIT_FIXTURES: {older}");
            continue;
        };
        pairs_checked += 1;
        let mut remaining = new.channel_names();
        let old_names = old.channel_names();
        assert!(!old_names.is_empty(), "{older}");
        for name in &old_names {
            let i = remaining
                .iter()
                .position(|n| n == name)
                .unwrap_or_else(|| panic!("{older}'s channel {name:?} is not in {newer}"));
            remaining.remove(i);
        }
        eprintln!(
            "  PAIR  {older} ({:?}) -> {newer} ({:?}): all {} older names found",
            old.fl_version,
            new.fl_version,
            old_names.len()
        );
    }

    eprintln!(
        "\n{} file(s) found, {clean} parsed clean, {} error(s)",
        flps.len(),
        errors.len()
    );
    eprintln!("versions seen: {versions:?}");
    eprintln!(
        "{v25_plus} file(s) flagged v25+ (scalars partial), {with_tempo} file(s) with a readable tempo"
    );
    eprintln!(
        "channel blocks == FLhd.channels on {rack_matches_header} of {clean}; \
         named channels == FLhd.channels on {names_match_header} of {clean}"
    );
    for s in &short {
        eprintln!("  fewer names than channels: {s}");
    }
    eprintln!(
        "pinned files checked: {pins_checked} of {}; same-song pairs checked: {pairs_checked} of {}",
        PINS.len(),
        SAME_SONG_BOTH_SCHEMES.len()
    );

    assert!(
        errors.is_empty(),
        "every real .flp should walk clean; failures: {errors:?}"
    );
}
