//! Opt-in real-material checks. **Loudly skipped** unless `WIT_FIXTURES`
//! is set; never touches real material otherwise, and never reads a path
//! from anywhere but that env var (this crate's own source never hardcodes
//! a personal path). Two tests:
//!
//! - [`real_library_walks_clean_and_prints_the_documented_tallies`] walks
//!   every `.flp` under `WIT_FIXTURES`, asserts each one parses (a typed
//!   error, never a panic, and none expected), and prints the tallies
//!   `docs/FORMATS.md`'s FL section and `extract.rs`'s docs quote — this
//!   test *is* their reproduction command.
//! - [`bundled_fl_studio_20_files_read_the_pinned_names`] needs the 172
//!   files FL Studio 20 ships in its app bundle
//!   (`FL Studio 20.app/Contents/Resources/FL/Data/…`) somewhere under
//!   `WIT_FIXTURES`, and checks names two ways a count can't: pinned name
//!   lists for 8 of Image-Line's demo projects and templates, and the same
//!   song saved in both channel-name schemes (two demo songs ship twice:
//!   saved by FL 11.0/11.1, names on id 192, and re-saved by FL 12.3,
//!   names on each channel's header 203 — every older name must appear in
//!   the newer save).
//!
//! ```text
//! WIT_FIXTURES=/path/to/FL/material cargo test -p wit-flp --test real_fixtures -- --nocapture --ignored
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn fixtures_dir() -> Option<PathBuf> {
    let dir = std::env::var_os("WIT_FIXTURES").map(PathBuf::from);
    if dir.is_none() {
        eprintln!(
            "WIT_FIXTURES not set — skipped. To run: \
             WIT_FIXTURES=/path/to/FL/material cargo test -p wit-flp --test real_fixtures -- --nocapture --ignored"
        );
    }
    dir
}

/// Recursively collect every `.flp` file under `root`, depth-capped
/// defensively against a symlink cycle, sorted.
fn find_flps(root: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
        if depth > 16 {
            return;
        }
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, depth + 1, out);
            } else if path.extension().and_then(|e| e.to_str()) == Some("flp") {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    walk(root, 0, &mut out);
    out.sort();
    out
}

fn version_tuple(version: &str) -> Option<(u32, u32)> {
    let mut parts = version.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().and_then(|m| m.parse().ok()).unwrap_or(0);
    Some((major, minor))
}

/// A naive event walk that reads id 172 as an ordinary 4-byte dword, as
/// `experiments/flp_parse.py` does — to reproduce what the FL 25 framing
/// quirk does to a walker that doesn't know about it. `Err` if it runs off
/// the end or meets a varint longer than 5 bytes; else the event count.
fn naive_walk_event_count(bytes: &[u8]) -> Result<usize, &'static str> {
    let header_len = u32::from_le_bytes(bytes.get(4..8).ok_or("short")?.try_into().unwrap());
    let mut pos = 8 + header_len as usize + 4;
    let data_len = u32::from_le_bytes(bytes.get(pos..pos + 4).ok_or("short")?.try_into().unwrap());
    pos += 4;
    let end = pos + data_len as usize;
    let mut count = 0;
    while pos < end {
        let id = *bytes.get(pos).ok_or("past EOF")?;
        pos += 1;
        let size = match id {
            0..=63 => 1,
            64..=127 => 2,
            128..=191 => 4,
            _ => {
                let (mut value, mut shift, mut done) = (0usize, 0, false);
                for _ in 0..5 {
                    let b = *bytes.get(pos).ok_or("past EOF")?;
                    pos += 1;
                    value |= usize::from(b & 0x7f) << shift;
                    shift += 7;
                    if b & 0x80 == 0 {
                        done = true;
                        break;
                    }
                }
                if !done {
                    return Err("varint longer than 5 bytes");
                }
                value
            }
        };
        pos += size;
        if pos > end {
            return Err("payload past EOF");
        }
        count += 1;
    }
    Ok(count)
}

/// Every id `extract.rs` treats as the end of the channel section.
const CHANNEL_SECTION_END_IDS: [u8; 13] = [
    98, 99, 100, 147, 149, 154, 204, 233, 235, 236, 238, 239, 241,
];

#[derive(Default)]
struct Tallies {
    files: usize,
    clean: usize,
    version_is_event_0: usize,
    pre25_files: usize,
    pre25_id172: usize,
    v25_files: usize,
    v25_first_172_index: BTreeMap<usize, usize>,
    v25_naive: Vec<Result<usize, &'static str>>,
    v25_gated_counts: Vec<usize>,
    rack_eq_header: usize,
    named_eq_header: usize,
    short: Vec<String>,
    // FL >= 11.5
    new_scheme_files: usize,
    new_blocks: usize,
    new_named_blocks: usize,
    new_header_shape: usize,
    non_last_blocks: usize,
    non_last_multi_201_203: usize,
    end_ids_inside_non_last_blocks: usize,
    first_end_id_after_last_channel: BTreeMap<u8, usize>,
    end_id_inside_last_header: usize,
    nameless_last_channel_next_203: Vec<(String, usize)>,
    new_files_with_pattern_events_in_a_block: usize,
    largest_rack: usize,
    // pre-11.5
    old_blocks: usize,
    old_blocks_with_one_192: usize,
    old_192_offsets: BTreeMap<usize, usize>,
    old_header_203: usize,
    // every version
    def_plugin_in_header: usize,
    def_plugin_in_mixer: usize,
    def_plugin_elsewhere: usize,
    mixer_positions: BTreeMap<usize, Vec<String>>,
    position0_files: usize,
    position0_limiter_files: usize,
    insert_names: usize,
    insert_name_then_236: usize,
    pattern_names: usize,
    pattern_name_right_after_65: usize,
    pattern_name_after_91_72: usize,
    pre25_files_with_unnamed_patterns: usize,
    pattern_numbers_with_two_names: usize,
    arrangement_starts: usize,
    arrangement_start_then_241: usize,
    pre25_newchan_payload_is_position: usize,
    with_tempo: usize,
}

fn tally_one(t: &mut Tallies, file_name: &str, bytes: &[u8], e: &wit_flp::Extracted) {
    let (_, events) = wit_flp::parse_container(bytes).expect("parsed once already");
    let ids: Vec<u8> = events.iter().map(|ev| ev.id).collect();
    let word = |i: usize| u16::from_le_bytes(events[i].payload.try_into().unwrap_or([0, 0]));
    let version = e.fl_version.clone().unwrap_or_default();
    let vt = version_tuple(&version).unwrap_or((0, 0));
    let v25 = vt.0 >= 25;
    let new_scheme = vt >= (11, 5);

    if ids.first() == Some(&199) {
        t.version_is_event_0 += 1;
    }
    if v25 {
        t.v25_files += 1;
        if let Some(i) = ids.iter().position(|&id| id == 172) {
            *t.v25_first_172_index.entry(i).or_default() += 1;
        }
        t.v25_naive.push(naive_walk_event_count(bytes));
        t.v25_gated_counts.push(events.len());
    } else {
        t.pre25_files += 1;
        t.pre25_id172 += ids.iter().filter(|&&id| id == 172).count();
    }
    if matches!(e.tempo, wit_flp::Tempo::Known(_)) {
        t.with_tempo += 1;
    }
    let named = e.channel_names().len();
    if e.channel_rack.len() == usize::from(e.channels) {
        t.rack_eq_header += 1;
    }
    if named == usize::from(e.channels) {
        t.named_eq_header += 1;
    } else {
        t.short
            .push(format!("{file_name}: {named} of {} named", e.channels));
    }

    // Channel blocks.
    let starts: Vec<usize> = (0..ids.len()).filter(|&i| ids[i] == 64).collect();
    t.largest_rack = t.largest_rack.max(starts.len());
    let mut pattern_events_in_a_block = false;
    if !v25
        && starts
            .iter()
            .enumerate()
            .all(|(n, &i)| usize::from(word(i)) == n)
    {
        t.pre25_newchan_payload_is_position += 1;
    }
    for (n, &start) in starts.iter().enumerate() {
        let next = starts.get(n + 1).copied();
        let block_end = next.unwrap_or(ids.len());
        let block = &ids[start + 1..block_end];
        if new_scheme {
            t.new_blocks += 1;
            if block.starts_with(&[21, 201, 212, 203]) {
                t.new_header_shape += 1;
            }
        } else {
            t.old_blocks += 1;
            let first_end = block
                .iter()
                .position(|id| CHANNEL_SECTION_END_IDS.contains(id))
                .unwrap_or(block.len());
            if block[..first_end].iter().filter(|&&id| id == 192).count() == 1 {
                t.old_blocks_with_one_192 += 1;
            }
            if let Some(k) = block.iter().position(|&id| id == 192) {
                *t.old_192_offsets.entry(k + 1).or_default() += 1;
            }
            if block.starts_with(&[21, 201, 212, 203]) {
                t.old_header_203 += 1;
            }
        }
        if next.is_some() {
            if new_scheme && block.iter().any(|&id| id == 65 || id == 193) {
                pattern_events_in_a_block = true;
            }
            t.non_last_blocks += usize::from(new_scheme);
            if new_scheme
                && (block.iter().filter(|&&id| id == 201).count() > 1
                    || block.iter().filter(|&&id| id == 203).count() > 1)
            {
                t.non_last_multi_201_203 += 1;
            }
            t.end_ids_inside_non_last_blocks += block
                .iter()
                .filter(|id| CHANNEL_SECTION_END_IDS.contains(id))
                .count();
        } else if let Some(k) = block
            .iter()
            .position(|id| CHANNEL_SECTION_END_IDS.contains(id))
        {
            *t.first_end_id_after_last_channel
                .entry(block[k])
                .or_default() += 1;
            if k < 4 {
                t.end_id_inside_last_header += 1;
            }
            if new_scheme && !block.starts_with(&[21, 201, 212, 203]) {
                if let Some(d) = block.iter().position(|&id| id == 203) {
                    t.nameless_last_channel_next_203
                        .push((file_name.to_string(), d + 1));
                }
            }
        }
    }
    if new_scheme {
        t.new_scheme_files += 1;
        t.new_named_blocks += named;
        t.new_files_with_pattern_events_in_a_block += usize::from(pattern_events_in_a_block);
    }

    // DefPluginName placement.
    let first_236 = ids.iter().position(|&id| id == 236).unwrap_or(ids.len());
    for i in 0..ids.len() {
        if ids[i] != 201 {
            continue;
        }
        if i >= 2 && ids[i - 2] == 64 && ids[i - 1] == 21 {
            t.def_plugin_in_header += 1;
        } else if i > first_236 {
            t.def_plugin_in_mixer += 1;
        } else {
            t.def_plugin_elsewhere += 1;
        }
    }

    // Mixer.
    t.mixer_positions
        .entry(e.mixer_inserts.len())
        .or_default()
        .push(version.clone());
    let at0: Vec<&str> = e
        .mixer_effects
        .iter()
        .filter(|m| m.position == 0)
        .map(|m| m.name.as_str())
        .collect();
    if !at0.is_empty() {
        t.position0_files += 1;
        if at0
            .iter()
            .any(|n| *n == "Fruity Limiter" || *n == "Maximus")
        {
            t.position0_limiter_files += 1;
        }
    }
    for i in 0..ids.len() {
        if ids[i] == 204 {
            t.insert_names += 1;
            if ids[i + 1..].iter().find(|&&id| id == 204 || id == 236) == Some(&236) {
                t.insert_name_then_236 += 1;
            }
        }
    }

    // Patterns.
    let mut names_per_number: BTreeMap<u16, usize> = BTreeMap::new();
    let mut last_65: Option<usize> = None;
    for i in 0..ids.len() {
        match ids[i] {
            65 => last_65 = Some(i),
            193 => {
                t.pattern_names += 1;
                if let Some(j) = last_65 {
                    let between = &ids[j + 1..i];
                    if between.is_empty() {
                        t.pattern_name_right_after_65 += 1;
                    } else if between.iter().all(|&id| id == 91 || id == 72) {
                        t.pattern_name_after_91_72 += 1;
                    }
                    if !v25 {
                        *names_per_number.entry(word(j)).or_default() += 1;
                    }
                }
            }
            _ => {}
        }
    }
    t.pattern_numbers_with_two_names += names_per_number.values().filter(|&&n| n > 1).count();
    if !v25 && e.patterns.iter().any(|p| p.name.is_none()) {
        t.pre25_files_with_unnamed_patterns += 1;
    }

    // Arrangements.
    for i in 0..ids.len() {
        if ids[i] == 99 {
            t.arrangement_starts += 1;
            if ids[i + 1..].iter().find(|&&id| id == 99 || id == 241) == Some(&241) {
                t.arrangement_start_then_241 += 1;
            }
        }
    }
}

#[test]
#[ignore = "opt-in: set WIT_FIXTURES to real FL Studio material and pass --ignored"]
fn real_library_walks_clean_and_prints_the_documented_tallies() {
    let Some(dir) = fixtures_dir() else {
        return;
    };
    let flps = find_flps(&dir);
    assert!(!flps.is_empty(), "no .flp files found under {dir:?}");

    let mut t = Tallies::default();
    let mut errors: Vec<(PathBuf, wit_flp::FlpError)> = Vec::new();
    for path in &flps {
        let Ok(bytes) = std::fs::read(path) else {
            eprintln!("  SKIP (unreadable): {}", path.display());
            continue;
        };
        t.files += 1;
        let file_name = path.file_name().unwrap().to_string_lossy().into_owned();
        match wit_flp::parse(&bytes) {
            Ok(e) => {
                t.clean += 1;
                tally_one(&mut t, &file_name, &bytes, &e);
            }
            Err(err) => {
                eprintln!("  ERROR {file_name}: {err}");
                errors.push((path.clone(), err));
            }
        }
    }

    let naive_fail = t.v25_naive.iter().filter(|r| r.is_err()).count();
    let naive_counts: Vec<usize> = t.v25_naive.iter().filter_map(|r| r.ok()).collect();
    eprintln!("\n{} file(s), {} parsed clean", t.files, t.clean);
    eprintln!(
        "Version (199) is event #0 in {} of {}",
        t.version_is_event_0, t.clean
    );
    eprintln!(
        "id 172 events in the {} pre-FL-25 files: {}; FL 25 files: {}, first 172 at event index {:?}",
        t.pre25_files, t.pre25_id172, t.v25_files, t.v25_first_172_index
    );
    eprintln!(
        "FL 25 walked with 172 as 4 bytes: {naive_fail} fail, the rest reach EOF with {:?} events; \
         with 172 as 3 bytes: {:?} events",
        naive_counts, t.v25_gated_counts
    );
    eprintln!(
        "channel blocks == FLhd.channels on {} of {}; named channels == FLhd.channels on {} of {}",
        t.rack_eq_header, t.clean, t.named_eq_header, t.clean
    );
    for s in &t.short {
        eprintln!("  fewer names than channels: {s}");
    }
    eprintln!(
        "NewChan payload == rack position in {} of {} pre-FL-25 files",
        t.pre25_newchan_payload_is_position, t.pre25_files
    );
    eprintln!(
        "FL >= 11.5: {} files, {} channel blocks, {} named, {} with header 64,21,201,212,203; \
         {} non-last blocks, {} holding >1 id-201 or >1 id-203",
        t.new_scheme_files,
        t.new_blocks,
        t.new_named_blocks,
        t.new_header_shape,
        t.non_last_blocks,
        t.non_last_multi_201_203
    );
    eprintln!(
        "section-end ids inside a non-last channel block: {}; first one after the last channel: {:?} \
         ({} inside that channel's header)",
        t.end_ids_inside_non_last_blocks,
        t.first_end_id_after_last_channel,
        t.end_id_inside_last_header
    );
    eprintln!(
        "last channel with no header name -> next id-203 this many events after its NewChan: {:?}",
        t.nameless_last_channel_next_203
    );
    eprintln!(
        "FL >= 11.5 files with pattern events (65/193) inside a non-last channel block: {} of {}; \
         largest rack: {} channels",
        t.new_files_with_pattern_events_in_a_block, t.new_scheme_files, t.largest_rack
    );
    eprintln!(
        "pre-11.5: {} channel blocks, {} with exactly one id-192 before the section ends; \
         192 this many events after its NewChan: {:?}; {} blocks with a header 203",
        t.old_blocks, t.old_blocks_with_one_192, t.old_192_offsets, t.old_header_203
    );
    eprintln!(
        "id-201 events: {} in a channel header, {} in the mixer, {} elsewhere ({} total)",
        t.def_plugin_in_header,
        t.def_plugin_in_mixer,
        t.def_plugin_elsewhere,
        t.def_plugin_in_header + t.def_plugin_in_mixer + t.def_plugin_elsewhere
    );
    for (positions, versions) in &t.mixer_positions {
        let mut vs: Vec<&String> = versions.iter().collect();
        vs.sort_by_key(|v| version_tuple(v));
        vs.dedup();
        eprintln!(
            "mixer positions {positions}: {} files, FL {} – {}",
            versions.len(),
            vs.first().map(|s| s.as_str()).unwrap_or("?"),
            vs.last().map(|s| s.as_str()).unwrap_or("?")
        );
    }
    eprintln!(
        "files with an effect at position 0: {}, with a Fruity Limiter or Maximus there: {}",
        t.position0_files, t.position0_limiter_files
    );
    eprintln!(
        "id-204 events: {}, followed by a 236 before another 204: {}",
        t.insert_names, t.insert_name_then_236
    );
    eprintln!(
        "id-193 events: {}; right after their 65: {}; after only 91/72 events: {}; \
         pattern numbers given two names: {}; pre-FL-25 files with an unnamed pattern: {} of {}",
        t.pattern_names,
        t.pattern_name_right_after_65,
        t.pattern_name_after_91_72,
        t.pattern_numbers_with_two_names,
        t.pre25_files_with_unnamed_patterns,
        t.pre25_files
    );
    eprintln!(
        "id-99 events: {}, followed by a 241 before the next 99: {}",
        t.arrangement_starts, t.arrangement_start_then_241
    );
    eprintln!(
        "{} file(s) with a readable tempo, {} FL 25 (tempo partial)",
        t.with_tempo, t.v25_files
    );

    assert!(
        errors.is_empty(),
        "every real .flp should walk clean; failures: {errors:?}"
    );
}

/// A bundled FL Studio 20 file whose names are pinned.
struct Pin {
    /// Path below FL Studio 20's `Contents/Resources/FL` folder.
    path: &'static str,
    fl_version: &'static str,
    /// Every channel block, named or not.
    rack_len: usize,
    named: usize,
    /// The first channel names, in rack order (all of them for small
    /// projects).
    first_names: &'static [&'static str],
    /// The first effects, in mixer order: (plugin, mixer position).
    first_effects: &'static [(&'static str, u16)],
    /// The first named mixer inserts: (position, name).
    first_inserts: &'static [(usize, &'static str)],
}

const PINS: &[Pin] = &[
    // The review's reproduction: one default Sampler channel with no name
    // of its own; the first 203 after it is the Master insert's "Control
    // Surface" plugin, renamed "Mix". No channel name may be read.
    Pin {
        path: "Data/Templates/Utility/Surround mix/Surround mix.flp",
        fl_version: "20.7.0.1702",
        rack_len: 1,
        named: 0,
        first_names: &[],
        first_effects: &[("Control Surface", 0), ("Fruity X-Y Controller", 1)],
        first_inserts: &[(1, "1"), (2, "2"), (3, "3")],
    },
    Pin {
        path: "Data/Templates/Minimal/Empty with 4 sends/Empty with 4 sends.flp",
        fl_version: "20.7.1.1773",
        rack_len: 1,
        named: 0,
        first_names: &[],
        first_effects: &[],
        first_inserts: &[],
    },
    Pin {
        path: "Data/Templates/Minimal/Basic 808/Basic 808.flp",
        fl_version: "20.7.0.1702",
        rack_len: 4,
        named: 4,
        first_names: &["808 Kick", "808 Clap", "808 HiHat", "808 Snare"],
        first_effects: &[],
        first_inserts: &[],
    },
    Pin {
        path: "Data/Templates/Minimal/Basic with limiter/Basic with limiter.flp",
        fl_version: "20.7.0.1702",
        rack_len: 4,
        named: 4,
        first_names: &["Kick", "Clap", "Hat", "Snare"],
        first_effects: &[("Fruity Limiter", 0)],
        first_inserts: &[],
    },
    Pin {
        path: "Data/Demo projects/NewStuff.flp",
        fl_version: "20.8.0.1377",
        rack_len: 144,
        named: 144,
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
        first_effects: &[
            ("Fruity Parametric EQ 2", 0),
            ("Fruity Parametric EQ 2", 0),
            ("Maximus", 0),
        ],
        first_inserts: &[],
    },
    // The first FL 11.5 file seen: the new scheme's lower boundary.
    Pin {
        path: "Data/Demo projects/Demo songs/Raubana - LIFE.flp",
        fl_version: "11.5.14",
        rack_len: 24,
        named: 24,
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
        first_effects: &[("Maximus", 0), ("Fruity Reeverb 2", 1)],
        first_inserts: &[],
    },
    // Pre-11.5 (id 192) files.
    Pin {
        path: "Data/Demo projects/Product demos/Vocodex demo.flp",
        fl_version: "8.5.0",
        rack_len: 7,
        named: 7,
        first_names: &[
            "Modulator #2",
            "Modulator",
            "PWM",
            "Supersaw",
            "Saw",
            "Ensemble",
            "Phased 2",
        ],
        first_effects: &[("Vocodex", 2)],
        first_inserts: &[],
    },
    Pin {
        path: "Data/Demo projects/Demo songs/AuraQualic - DATA (FL Studio Remix).flp",
        fl_version: "10.0.0",
        rack_len: 43,
        named: 43,
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
        first_effects: &[("Fruity Wrapper", 0), ("Maximus", 0)],
        first_inserts: &[],
    },
];

/// (older save, names on 192; newer re-save, names on the header 203).
const SAME_SONG_BOTH_SCHEMES: &[(&str, &str)] = &[
    (
        "Data/Demo projects/Demo songs/SeamlessR - Menagerie.flp",
        "Data/Demo projects/Visual/ZGameEditor Visualizer/JuiceBass - SeamlessR - Menagerie.flp",
    ),
    (
        "Data/Demo projects/Demo songs/Gimbal & Sinan + Futorial - RawFL.flp",
        "Data/Demo projects/Visual/ZGameEditor Visualizer/Treben - Gimbal and Sinan - RawFL.flp",
    ),
];

#[test]
#[ignore = "opt-in: set WIT_FIXTURES to a folder holding FL Studio 20's bundled files and pass --ignored"]
fn bundled_fl_studio_20_files_read_the_pinned_names() {
    let Some(dir) = fixtures_dir() else {
        return;
    };
    let flps = find_flps(&dir);
    let find = |relative: &str| -> Option<wit_flp::Extracted> {
        let path = flps.iter().find(|p| p.ends_with(relative))?;
        wit_flp::parse(&std::fs::read(path).ok()?).ok()
    };

    let mut pins_checked = 0usize;
    for pin in PINS {
        let e = find(pin.path).unwrap_or_else(|| {
            panic!(
                "{} is not under WIT_FIXTURES — point it at a folder holding FL Studio 20's \
                 bundled files (FL Studio 20.app/Contents/Resources/FL)",
                pin.path
            )
        });
        assert_eq!(
            e.fl_version.as_deref(),
            Some(pin.fl_version),
            "{}",
            pin.path
        );
        let names = e.channel_names();
        assert_eq!(e.channel_rack.len(), pin.rack_len, "{}", pin.path);
        assert_eq!(names.len(), pin.named, "{}", pin.path);
        assert_eq!(
            &names[..pin.first_names.len()],
            pin.first_names,
            "{}",
            pin.path
        );
        let effects: Vec<(&str, u16)> = e
            .mixer_effects
            .iter()
            .take(pin.first_effects.len())
            .map(|m| (m.name.as_str(), m.position))
            .collect();
        assert_eq!(effects, pin.first_effects, "{}", pin.path);
        let inserts: Vec<(usize, &str)> = e
            .mixer_inserts
            .iter()
            .enumerate()
            .filter_map(|(i, n)| Some((i, n.as_deref()?)))
            .take(pin.first_inserts.len())
            .collect();
        assert_eq!(inserts, pin.first_inserts, "{}", pin.path);
        pins_checked += 1;
    }
    assert_eq!(pins_checked, PINS.len());

    let mut pairs_checked = 0usize;
    for (older, newer) in SAME_SONG_BOTH_SCHEMES {
        let (Some(old), Some(new)) = (find(older), find(newer)) else {
            panic!("{older} or {newer} is not under WIT_FIXTURES");
        };
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
        pairs_checked += 1;
    }
    assert_eq!(pairs_checked, SAME_SONG_BOTH_SCHEMES.len());
    eprintln!(
        "pinned files checked: {pins_checked} of {}; same-song pairs checked: {pairs_checked} of {}",
        PINS.len(),
        SAME_SONG_BOTH_SCHEMES.len()
    );
}
