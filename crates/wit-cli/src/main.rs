//! The `wit` command-line tool.
//!
//! M1 added `diff-als` (the M1 exit criterion: "`wit diff-als a.als b.als`
//! prints the golden"). M2 added `logic-probe` — the issue #20 CLI hook,
//! comparing two Logic/GarageBand saves at the Structure honesty tier
//! (census + extracted names; see `wit-logic`'s module docs for why byte
//! comparison is a diagnostic, never the verdict). M3 adds `scan` and
//! `dupes` (`wit-index`) — the first commands that persist anything, via
//! the one crate in the workspace allowed to write. M2.5 adds
//! `logic-report` — the issue #15 reality-gate tool, running `logic-probe`'s
//! comparison across an entire library instead of one pair. M5 adds
//! `demo-library` (`wit-demo`), which writes the synthetic library the app
//! is developed and demoed against, so neither needs a real Logic library
//! on the machine. Phase 4 (FL Studio) adds `flp-probe` (`wit-flp`) —
//! `logic-probe`'s counterpart for `.flp` projects, and `scan` now
//! discovers FL Studio projects and their `Backup/` autosave chain too.
//! `wit log`/`diff`/`report` land later; see `docs/ROADMAP.md`.

use clap::{Parser, Subcommand};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};
use wit_model::fmt_num;

#[derive(Parser)]
#[command(name = "wit", about = "Version control for music projects", version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Semantic diff between two Ableton Live sets — the Rust port of
    /// `experiments/als_semantic_diff.py`'s `report()`.
    DiffAls {
        old: PathBuf,
        new: PathBuf,
        /// Maximum number of changes to print.
        #[arg(long, default_value_t = 40)]
        limit: usize,
    },
    /// Structural comparison between two Logic Pro / GarageBand saves.
    /// Accepts either a raw `ProjectData` file or a `.logicx`/`.band`
    /// bundle directory (the current alternative's `ProjectData` is
    /// resolved automatically).
    LogicProbe { old: PathBuf, new: PathBuf },
    /// Print the names and tempo Wit can read out of an FL Studio project,
    /// and — when a second file is given — a plain-words comparison
    /// between the two. FL Studio 25+ projects print tempo as "can't read
    /// yet" and say which names are unverified on that version (see
    /// `wit-flp`'s module docs for why).
    FlpProbe { a: PathBuf, b: Option<PathBuf> },
    /// Discover Logic/GarageBand/Ableton/FL Studio projects under `path` and
    /// archive-before-recycle every version into Wit's local index.
    Scan {
        path: PathBuf,
        /// Override the index location (default: the platform app-data
        /// dir). Tests and anyone experimenting should always pass this —
        /// it's the same rule `wit-index`'s own tests follow.
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },
    /// Report byte-for-byte duplicate audio files under `path`. Read-only
    /// — Wit never deletes anything; this is just a map.
    Dupes { path: PathBuf },
    /// M2.5 (issue #15) reality-gate report: walk every Logic/GarageBand
    /// alternative's backup chain under `path`, run `logic-probe`'s
    /// comparison on every consecutive pair, and print the empty-verdict
    /// rate across the whole library. Read-only.
    LogicReport { path: PathBuf },
    /// Write a synthetic `~/Music`-shaped library to `dest` — two Logic
    /// projects, a GarageBand project, and an Ableton lineage — so the app
    /// is demoable on a machine with no real Logic library. Refuses to
    /// write into a directory that already has anything in it.
    DemoLibrary { dest: PathBuf },
    /// Tell the story of every song under `path`: sessions of saves, and in
    /// plain sentences what changed at each one. Read-only. `--json` prints
    /// the Story contract (`crates/wit-story`) the app reads.
    Story {
        path: PathBuf,
        #[arg(long)]
        json: bool,
        /// Your UTC offset for the time labels, e.g. 120 for UTC+2. Default:
        /// read from the system clock where possible, else UTC.
        #[arg(long, allow_hyphen_values = true)]
        utc_offset_minutes: Option<i32>,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::DiffAls { old, new, limit } => diff_als(&old, &new, limit),
        Command::LogicProbe { old, new } => logic_probe(&old, &new),
        Command::FlpProbe { a, b } => flp_probe(&a, b.as_deref()),
        Command::Scan { path, data_dir } => scan(&path, data_dir),
        Command::Dupes { path } => dupes(&path),
        Command::LogicReport { path } => logic_report(&path),
        Command::DemoLibrary { dest } => demo_library(&dest),
        Command::Story {
            path,
            json,
            utc_offset_minutes,
        } => story(&path, json, utc_offset_minutes),
    }
}

/// M5 (issue #18): build the synthetic library `just demo-library` wraps.
fn demo_library(dest: &std::path::Path) -> ExitCode {
    let lib = match wit_demo::build_demo_library(dest) {
        Ok(lib) => lib,
        Err(e) => {
            eprintln!("wit: {e}");
            return ExitCode::FAILURE;
        }
    };
    println!(
        "  wrote {} Logic project(s), {} GarageBand project(s), {} Ableton lineage(s) — {} version(s) total",
        lib.logic_projects, lib.garageband_projects, lib.ableton_lineages, lib.total_versions
    );
    println!(
        "  these are synthetic fixtures for Wit's own readers — Logic and Live cannot open them"
    );
    println!("  point the app at: {}", lib.root.display());
    ExitCode::SUCCESS
}

/// The local UTC offset in minutes, from `date +%z` where that exists
/// (macOS, Linux). `None` elsewhere — the caller falls back to UTC and says
/// so rather than guessing.
fn system_utc_offset_minutes() -> Option<i32> {
    let out = std::process::Command::new("date")
        .arg("+%z")
        .output()
        .ok()?;
    let text = String::from_utf8(out.stdout).ok()?;
    let t = text.trim();
    if t.len() != 5 {
        return None;
    }
    let sign = match &t[..1] {
        "+" => 1,
        "-" => -1,
        _ => return None,
    };
    let h: i32 = t[1..3].parse().ok()?;
    let m: i32 = t[3..5].parse().ok()?;
    Some(sign * (h * 60 + m))
}

fn story(path: &std::path::Path, json: bool, utc_offset_minutes: Option<i32>) -> ExitCode {
    if !path.is_dir() {
        eprintln!("wit: {} is not a folder", path.display());
        return ExitCode::FAILURE;
    }
    let offset = utc_offset_minutes.or_else(system_utc_offset_minutes);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let clock = wit_story::Clock::fixed(wit_story::Timestamp(now), offset.unwrap_or(0));
    let label = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "this folder".to_string());
    let library = wit_story::build_library(path, &label, &clock);
    if json {
        print!("{}", wit_story::to_json(&library));
        return ExitCode::SUCCESS;
    }
    if library.stories.is_empty() {
        println!("  no Logic, GarageBand or Live projects found under {label}");
        return ExitCode::SUCCESS;
    }
    if offset.is_none() {
        println!("  (times are UTC — pass --utc-offset-minutes for local time)");
    }
    for story in &library.stories {
        let h = &story.header;
        let title = match &h.lineage {
            Some(l) => format!("{} · {l}", h.title),
            None => h.title.clone(),
        };
        println!("\n{title}\n  {}\n  {}", h.subtitle, h.kept.label);
        for session in &story.sessions {
            println!("\n  {}", session.label);
            for m in &session.moments {
                println!("    {}  ({})", m.label, m.source_label);
                if let Some(summary) = &m.summary {
                    println!("        {summary}");
                }
                for s in &m.sentences {
                    match &s.place_label {
                        Some(p) => println!("        {}  — {p}", s.text),
                        None => println!("        {}", s.text),
                    }
                }
                if let Some(note) = &m.note {
                    println!("        {note}");
                }
            }
        }
        if let Some(o) = &story.overview {
            println!("\n  {} ({})", o.heading, o.subheading);
            for s in &o.sentences {
                println!("        {}", s.text);
            }
        }
        let notes: Vec<&str> = story.capability.iter().map(|c| c.text.as_str()).collect();
        println!("\n  {}", notes.join(" "));
    }
    ExitCode::SUCCESS
}

/// The default index location: `~/Library/Application Support/Wit` on
/// macOS (the only platform the 0.0 pilot targets — ADR-0006). Falls back
/// to a `wit-data` directory under the current directory if `$HOME` isn't
/// set (a CI/test environment, not a real user's Mac), rather than
/// panicking.
fn default_data_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(|home| PathBuf::from(home).join("Library/Application Support/Wit"))
        .unwrap_or_else(|| PathBuf::from("wit-data"))
}

fn diff_als(old: &std::path::Path, new: &std::path::Path, limit: usize) -> ExitCode {
    let model_a = match wit_als::parse_file(old) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("wit: failed to read {}: {e}", old.display());
            return ExitCode::FAILURE;
        }
    };
    let model_b = match wit_als::parse_file(new) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("wit: failed to read {}: {e}", new.display());
            return ExitCode::FAILURE;
        }
    };

    let records = wit_diff::diff(&model_a, &model_b);
    if records.is_empty() {
        println!("  no musical change detected (view / bookkeeping only)");
        return ExitCode::SUCCESS;
    }

    println!("  {} semantic change(s)", records.len());
    let text = wit_model::render_text(&records);
    let lines: Vec<&str> = text.lines().collect();
    for line in lines.iter().take(limit) {
        println!("    {line}");
    }
    if lines.len() > limit {
        println!("    ... and {} more", lines.len() - limit);
    }
    ExitCode::SUCCESS
}

/// If `path` is a directory (a `.logicx`/`.band` bundle), resolve to its
/// current alternative's `ProjectData`; if it's a file, use it as-is —
/// lets a user point `logic-probe` at the bundles Finder shows them, at a
/// specific `Project File Backups/NN` slot, or at a raw `ProjectData` file
/// directly — the three shapes a real Logic package actually has (a bundle
/// root's `ProjectData` sits under `Alternatives/000/`; a backup slot
/// directory holds `ProjectData` right inside it).
fn resolve_project_data(path: &std::path::Path) -> PathBuf {
    if path.is_file() {
        return path.to_path_buf();
    }
    let as_bundle = path.join("Alternatives/000/ProjectData");
    if as_bundle.is_file() {
        return as_bundle;
    }
    let as_slot = path.join("ProjectData");
    if as_slot.is_file() {
        return as_slot;
    }
    // Neither shape matched — return the bundle-root guess anyway so the
    // caller's read error names the path it actually tried, rather than
    // silently falling back to something else.
    as_bundle
}

fn logic_probe(old: &std::path::Path, new: &std::path::Path) -> ExitCode {
    let old_pd = resolve_project_data(old);
    let new_pd = resolve_project_data(new);

    let a = match wit_logic::walk_file(&old_pd) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("wit: failed to read {}: {e}", old_pd.display());
            return ExitCode::FAILURE;
        }
    };
    let b = match wit_logic::walk_file(&new_pd) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("wit: failed to read {}: {e}", new_pd.display());
            return ExitCode::FAILURE;
        }
    };

    match wit_logic::semantic_equal(&a, &b) {
        wit_logic::Verdict::NoStructuralChange => {
            println!(
                "  no structural change detected — Wit can't yet see knob and fader moves in Logic"
            );
        }
        wit_logic::Verdict::StructuralChange => {
            println!("  structural change detected:");
            print_census_diff(&a.census, &b.census);
            print_name_diff(
                "track/MIDI-sequence name",
                &a.extracted.possible_track_names,
                &b.extracted.possible_track_names,
            );
            print_name_diff(
                "region name",
                &a.extracted.region_names,
                &b.extracted.region_names,
            );
            print_name_diff(
                "audio file",
                &a.extracted.audio_file_names,
                &b.extracted.audio_file_names,
            );
            if a.extracted.tempo_bpm != b.extracted.tempo_bpm {
                println!(
                    "    tempo: {} -> {} BPM",
                    fmt_tempo(a.extracted.tempo_bpm),
                    fmt_tempo(b.extracted.tempo_bpm)
                );
            }
        }
    }

    // Diagnostic only, never part of the verdict above — see wit-logic's
    // module docs for why byte-identity and structural-identity are
    // deliberately different questions on this format.
    let bytes_identical = wit_logic::bytes_equal(
        &std::fs::read(&old_pd).unwrap_or_default(),
        &std::fs::read(&new_pd).unwrap_or_default(),
    );
    println!(
        "  (bytes identical: {bytes_identical} — diagnostic only, not part of the verdict above)"
    );

    ExitCode::SUCCESS
}

/// What kind of object a census tag's records cluster around, for display
/// only — plus whether the raw record count is known to line up 1:1 with a
/// real object count. Only `lFuA`/`AuFl` (audio files) has that evidence:
/// `docs/FORMATS.md` measured it matching `MetaData.plist`'s real count
/// exactly (35 -> 37). Every other tag here stays disclaimed: `wit-logic`'s
/// census module doc measured an ~8.4x record-to-track multiplier on a real
/// project (260 `karT` records against 31 actual tracks), so a tag graduates
/// to an actual object count only after issue #3's per-tag payload work, not
/// here. Deliberately excludes `gnoS` (the root/song record) — `wit-logic`'s
/// frame doc guarantees exactly one per valid file, so it can never differ
/// between two successfully-walked files and the diff branch below would
/// never fire for it.
struct TagInfo {
    noun: &'static str,
    verified_count: bool,
}

fn tag_info(tag: &str) -> Option<TagInfo> {
    match tag {
        "karT" => Some(TagInfo {
            noun: "tracks",
            verified_count: false,
        }),
        "gRuA" => Some(TagInfo {
            noun: "regions",
            verified_count: false,
        }),
        "lFuA" => Some(TagInfo {
            noun: "audio files",
            verified_count: true,
        }),
        "UCuA" => Some(TagInfo {
            noun: "plugins",
            verified_count: false,
        }),
        "qeSM" => Some(TagInfo {
            noun: "MIDI sequences",
            verified_count: false,
        }),
        "qSvE" => Some(TagInfo {
            noun: "event sequences",
            verified_count: false,
        }),
        _ => None,
    }
}

fn print_census_diff(a: &wit_logic::Census, b: &wit_logic::Census) {
    let tags: BTreeSet<&String> = a.keys().chain(b.keys()).collect();
    for tag in tags {
        let ca = a.get(tag).copied().unwrap_or(0);
        let cb = b.get(tag).copied().unwrap_or(0);
        if ca != cb {
            match tag_info(tag) {
                Some(info) if info.verified_count => println!(
                    "    {}: {ca} -> {cb} ({tag}) [verified against MetaData.plist on the one real project measured — see docs/FORMATS.md]",
                    info.noun
                ),
                Some(info) => println!(
                    "    {}-related records ({tag}): {ca} -> {cb} [internal record count, does NOT equal the number of {} — record-to-object ratio isn't 1:1, see wit-logic's census module doc]",
                    info.noun, info.noun
                ),
                None => println!(
                    "    {tag}: {ca} -> {cb} record(s) [internal count, unmapped tag — not a musician-facing number]"
                ),
            }
        }
    }
}

fn print_name_diff(label: &str, a: &[String], b: &[String]) {
    let sa: BTreeSet<&String> = a.iter().collect();
    let sb: BTreeSet<&String> = b.iter().collect();
    for added in sb.difference(&sa) {
        println!("    {label} added: '{added}'");
    }
    for removed in sa.difference(&sb) {
        println!("    {label} removed: '{removed}'");
    }
}

/// Render an optional tempo the way a musician reads it: a plain number
/// (never Rust's `Some(98.0)`/`None` debug spelling) via [`fmt_num`], or the
/// word "unknown" when Logic's three tempo slots didn't agree (see
/// `wit_logic::extract::tempo_bpm`).
fn fmt_tempo(bpm: Option<f64>) -> String {
    match bpm {
        Some(bpm) => fmt_num(bpm),
        None => "unknown".to_string(),
    }
}

// --------------------------------------------------------------------- //
// wit-flp: flp-probe
// --------------------------------------------------------------------- //

fn flp_probe(a: &std::path::Path, b: Option<&std::path::Path>) -> ExitCode {
    let extracted_a = match wit_flp::parse_file(a) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("wit: failed to read {}: {e}", a.display());
            return ExitCode::FAILURE;
        }
    };

    let Some(b) = b else {
        print_flp_summary(a, &extracted_a);
        return ExitCode::SUCCESS;
    };

    let extracted_b = match wit_flp::parse_file(b) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("wit: failed to read {}: {e}", b.display());
            return ExitCode::FAILURE;
        }
    };

    print_flp_summary(a, &extracted_a);
    println!();
    print_flp_summary(b, &extracted_b);
    println!();

    // Diagnostic only, mirroring logic_probe's own bytes_equal line: the
    // raw-byte fact is reported, but never used to decide the verdict
    // above it — see wit_flp::compare_with_bytes's doc comment.
    let bytes_equal = match (std::fs::read(a), std::fs::read(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => false,
    };
    let changes = wit_flp::compare_with_bytes(&extracted_a, &extracted_b, bytes_equal);
    if changes.is_empty() {
        println!("  nothing changed between these two projects");
    } else {
        println!("  {} thing(s) changed:", changes.len());
        for change in &changes {
            println!("    {}", render_flp_change(change));
        }
    }
    ExitCode::SUCCESS
}

/// Strip control characters (including bare `\r`/`\n` and terminal escape
/// sequences) and Unicode bidirectional-override/isolate/mark characters
/// from text before it ever reaches a `println!`. Every string `flp-probe`
/// prints from a project — names and the FL version text alike — comes
/// from an untrusted file's own bytes; a crafted (or merely corrupt)
/// payload could otherwise move the cursor, or reorder how the rest of the
/// line reads (a right-to-left override can make `'a' -> 'b'` display as
/// something else). Legitimate joiners such as U+200C (used in Persian
/// names) are kept.
fn sanitize_for_print(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_control() && !is_bidi_control(*c))
        .collect()
}

/// The Unicode bidi formatting characters: ALM, LRM/RLM, the embedding and
/// override controls (LRE, RLE, PDF, LRO, RLO) and the isolates (LRI, RLI,
/// FSI, PDI).
fn is_bidi_control(c: char) -> bool {
    matches!(c, '\u{061C}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

fn render_names_for_print(names: &[String]) -> String {
    names
        .iter()
        .map(|n| sanitize_for_print(n))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Plain words for a tempo reading — matches `flp-probe --help`'s own
/// description of the v25 case ("can't read yet"), rather than
/// `Tempo`'s `Display` impl, which is worded for engineering diagnostics
/// ("partial: v25 scalars unreadable") rather than a musician-facing CLI.
fn render_tempo(tempo: wit_flp::Tempo) -> String {
    match tempo {
        wit_flp::Tempo::Unknown => "unknown".to_string(),
        wit_flp::Tempo::Known(bpm) => format!("{bpm:.3} BPM"),
        wit_flp::Tempo::PartialV25ScalarsUnreadable => "can't read yet".to_string(),
    }
}

fn print_flp_summary(path: &std::path::Path, e: &wit_flp::Extracted) {
    println!("  {}", path.display());
    println!(
        "    FL Studio version: {}  channels: {}  tempo: {}",
        e.fl_version
            .as_deref()
            .map(sanitize_for_print)
            .unwrap_or_else(|| "unknown".to_string()),
        e.channels,
        render_tempo(e.tempo)
    );
    let channel_names = e.channel_names();
    if !channel_names.is_empty() {
        println!(
            "    channel names: {}",
            render_names_for_print(&channel_names)
        );
    }
    let unnamed = e.channel_rack.len() - channel_names.len();
    if unnamed > 0 {
        println!("    channels with no name saved: {unnamed}");
    }
    if !e.pattern_names.is_empty() {
        println!(
            "    pattern names: {}",
            render_names_for_print(&e.pattern_names)
        );
    }
    let generators = e.generator_names();
    if !generators.is_empty() {
        println!(
            "    generator plugins: {}",
            render_names_for_print(&generators)
        );
    }
    if !e.mixer_effects.is_empty() {
        let effects: Vec<String> = e
            .mixer_effects
            .iter()
            .map(|m| format!("{}{}", sanitize_for_print(&m.name), render_insert(m.insert)))
            .collect();
        println!("    effect plugins: {}", effects.join(", "));
    }
    if !e.mixer_insert_names.is_empty() {
        println!(
            "    mixer insert names: {}",
            render_names_for_print(&e.mixer_insert_names)
        );
    }
    if !e.arrangement_names.is_empty() {
        println!(
            "    arrangement names: {}",
            render_names_for_print(&e.arrangement_names)
        );
    }
    if e.format_status == wit_flp::FormatStatus::PartialV25ScalarsUnreadable {
        println!("{}", V25_NOTE);
    }
}

/// Printed under every FL 25+ summary. What it calls checked vs unverified
/// is exactly `wit_flp::FormatStatus::PartialV25ScalarsUnreadable`'s doc.
const V25_NOTE: &str = "    note: this FL Studio version scrambles some numeric settings \
     Wit can't unscramble yet, which is why tempo can't be read. Channel and plugin names read \
     cleanly on the real projects from this version Wit was checked against; pattern \
     names and mixer insert names could not be checked (none of those projects had any), \
     and neither could which mixer insert an effect sits on, so treat those as unverified.";

/// `" (Master)"`, `" (insert 3)"`, or nothing when the position is not
/// labelled — see `wit_flp::MixerEffect::insert`.
fn render_insert(insert: Option<u16>) -> String {
    match insert {
        Some(0) => " (Master)".to_string(),
        Some(n) => format!(" (insert {n})"),
        None => String::new(),
    }
}

fn render_flp_change(change: &wit_flp::FlChange) -> String {
    let n = |name: &str| sanitize_for_print(name);
    let generator = |g: &Option<String>| match g {
        Some(g) => format!(" (generator plugin '{}')", n(g)),
        None => String::new(),
    };
    match change {
        wit_flp::FlChange::ChannelAdded { name, generator: g } => {
            format!("channel added: '{}'{}", n(name), generator(g))
        }
        wit_flp::FlChange::ChannelRemoved { name, generator: g } => {
            format!("channel removed: '{}'{}", n(name), generator(g))
        }
        wit_flp::FlChange::ChannelRenamed { old, new } => {
            format!("channel renamed: '{}' -> '{}'", n(old), n(new))
        }
        wit_flp::FlChange::PatternAdded { name } => format!("pattern added: '{}'", n(name)),
        wit_flp::FlChange::PatternRemoved { name } => format!("pattern removed: '{}'", n(name)),
        wit_flp::FlChange::PatternRenamed { old, new } => {
            format!("pattern renamed: '{}' -> '{}'", n(old), n(new))
        }
        wit_flp::FlChange::GeneratorAdded { name } => {
            format!("generator plugin added: '{}'", n(name))
        }
        wit_flp::FlChange::GeneratorRemoved { name } => {
            format!("generator plugin removed: '{}'", n(name))
        }
        wit_flp::FlChange::EffectAdded { name, insert } => {
            format!(
                "effect plugin added: '{}'{}",
                n(name),
                render_insert(*insert)
            )
        }
        wit_flp::FlChange::EffectRemoved { name, insert } => {
            format!(
                "effect plugin removed: '{}'{}",
                n(name),
                render_insert(*insert)
            )
        }
        wit_flp::FlChange::MixerInsertAdded { name } => {
            format!("mixer insert added: '{}'", n(name))
        }
        wit_flp::FlChange::MixerInsertRemoved { name } => {
            format!("mixer insert removed: '{}'", n(name))
        }
        wit_flp::FlChange::MixerInsertRenamed { old, new } => {
            format!("mixer insert renamed: '{}' -> '{}'", n(old), n(new))
        }
        wit_flp::FlChange::ArrangementAdded { name } => {
            format!("arrangement added: '{}'", n(name))
        }
        wit_flp::FlChange::ArrangementRemoved { name } => {
            format!("arrangement removed: '{}'", n(name))
        }
        wit_flp::FlChange::ArrangementRenamed { old, new } => {
            format!("arrangement renamed: '{}' -> '{}'", n(old), n(new))
        }
        wit_flp::FlChange::TempoChanged { from_bpm, to_bpm } => {
            format!("tempo: {from_bpm} -> {to_bpm} BPM")
        }
        wit_flp::FlChange::BytesChangedNothingReadable => {
            "something changed that Wit can't read yet".to_string()
        }
    }
}

// --------------------------------------------------------------------- //
// M3: scan / dupes (wit-index)
// --------------------------------------------------------------------- //

fn scan(path: &std::path::Path, data_dir: Option<PathBuf>) -> ExitCode {
    let data_dir = data_dir.unwrap_or_else(default_data_dir);
    let store = match wit_index::Store::open(data_dir.join("objects")) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("wit: failed to open the store: {e}");
            return ExitCode::FAILURE;
        }
    };
    let registry = match wit_index::Registry::open(data_dir.join("wit.db")) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("wit: failed to open the index: {e}");
            return ExitCode::FAILURE;
        }
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    let result = wit_index::scan(path, &store, &registry, now);
    println!(
        "  found {} Logic/GarageBand project(s), {} Ableton lineage(s), {} FL Studio project(s) \
         — {} new version(s) archived",
        result.logic_projects_found,
        result.ableton_lineages_found,
        result.flp_projects_found,
        result.new_versions_ingested
    );
    if result.read_errors > 0 {
        println!(
            "  ({} file(s) could not be read and were skipped)",
            result.read_errors
        );
    }

    let projects = match registry.list_projects() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("wit: failed to list the index: {e}");
            return ExitCode::FAILURE;
        }
    };
    // Names only, never the full bundle_path (which contains an absolute
    // home-directory path) — the same "no path leaves this machine's
    // report output" discipline `wit dupes` follows below.
    for project in &projects {
        println!(
            "    {} ({}): {} version(s)",
            project.name, project.kind, project.version_count
        );
    }
    ExitCode::SUCCESS
}

fn dupes(path: &std::path::Path) -> ExitCode {
    let report = wit_index::duplicate_report(path);
    if report.groups.is_empty() {
        println!(
            "  no duplicate audio found ({} file(s) scanned)",
            report.scanned_file_count
        );
        return ExitCode::SUCCESS;
    }

    let mut out = String::new();
    out.push_str(&format!(
        "  found {} of duplicate audio ({:.1}% of {} scanned)\n",
        human_bytes(report.total_wasted_bytes()),
        report.duplicate_percent(),
        human_bytes(report.total_audio_bytes)
    ));
    let mut groups = report.groups.clone();
    groups.sort_by_key(|g| std::cmp::Reverse(g.wasted_bytes()));
    for group in &groups {
        // Basenames only — never a full path, per the same privacy
        // discipline the M3 issue asks of the (future) `report` command.
        let names: Vec<String> = group
            .paths
            .iter()
            .map(|p| {
                p.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default()
            })
            .collect();
        out.push_str(&format!(
            "    {} ({} copies, {} each): {}\n",
            human_bytes(group.wasted_bytes()),
            group.paths.len(),
            human_bytes(group.size_bytes),
            names.join(", ")
        ));
    }
    out.push_str("  no delete button exists — this is just a map\n");

    if let Err(msg) = wit_index::assert_no_home_paths(&out) {
        // This must never happen — it's a bug in this function, not a
        // recoverable runtime condition, so fail loudly rather than print
        // a path that was supposed to be impossible to print.
        eprintln!("wit: internal error — {msg}");
        return ExitCode::FAILURE;
    }
    print!("{out}");
    ExitCode::SUCCESS
}

/// Format a byte count in **decimal** units — 1 GB = 1,000,000,000 bytes.
///
/// This is deliberately not the 1024-based convention. Every published
/// figure in `docs/EXPERIMENTS.md` is decimal GB (§9 says so explicitly),
/// and `wit dupes` output is meant to be directly comparable to it — a
/// user pasting this tool's number into a Measurement issue (the ask in
/// [#4]) must be quoting the same unit the docs quote. Dividing by 1024
/// while printing "GB" understated the library total by 7.4% and made the
/// two numbers silently incomparable.
fn human_bytes(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1000.0 && unit < UNITS.len() - 1 {
        size /= 1000.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{size:.1} {}", UNITS[unit])
    }
}

// --------------------------------------------------------------------- //
// M2.5: logic-report (issue #15 reality gate)
// --------------------------------------------------------------------- //

fn logic_report(path: &std::path::Path) -> ExitCode {
    let report = wit_index::logic_report(path);

    if report.projects_scanned == 0 {
        println!("  no Logic/GarageBand project found under this path — nothing to report");
        return ExitCode::SUCCESS;
    }

    // Project/alternative *names* only, never a full path — same privacy
    // discipline `wit scan`/`wit dupes` already follow (no home-directory
    // path leaves this machine's report output).
    let mut out = String::new();
    out.push_str(&format!(
        "  scanned {} project(s), {} alternative(s), {} consecutive save pair(s)\n",
        report.projects_scanned,
        report.alternatives_scanned,
        report.total_pairs()
    ));

    if report.total_pairs() == 0 {
        out.push_str("  no consecutive save pairs found (every alternative has 0 or 1 version) — nothing to compare\n");
    } else {
        out.push_str(&format!(
            "  {:.1}% of save pairs show a structural change Wit can see ({} of {})\n",
            report.structural_change_percent(),
            report.pairs_with_structural_change(),
            report.total_pairs()
        ));
        out.push_str(
            "  distribution of change counts per save pair (0 = no visible structural change):\n",
        );
        for (count, n) in report.change_count_distribution() {
            out.push_str(&format!("    {count} change(s): {n} pair(s)\n"));
        }
        let byte_different_but_same = report.byte_different_structurally_identical();
        out.push_str(&format!(
            "  {byte_different_but_same} pair(s) ({:.1}%) are byte-different but structurally identical\n",
            byte_different_but_same as f64 / report.total_pairs() as f64 * 100.0
        ));
    }
    if !report.read_errors.is_empty() {
        out.push_str(&format!(
            "  ({} ProjectData file(s) could not be read or walked and were skipped)\n",
            report.read_errors.len()
        ));
    }

    if let Err(msg) = wit_index::assert_no_home_paths(&out) {
        // Must never happen — a bug in this function, not a recoverable
        // runtime condition, so fail loudly rather than print a path that
        // was supposed to be impossible to print (mirrors `dupes` above).
        eprintln!("wit: internal error — {msg}");
        return ExitCode::FAILURE;
    }
    print!("{out}");
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::{human_bytes, render_flp_change, sanitize_for_print, tag_info, V25_NOTE};

    #[test]
    fn sanitize_strips_control_and_bidi_characters_but_keeps_joiners() {
        // A right-to-left override would make the rest of the line display
        // reversed; ESC would start a terminal escape sequence.
        assert_eq!(sanitize_for_print("Kick\u{202E}kcans"), "Kickkcans");
        assert_eq!(sanitize_for_print("\u{1b}[2J25.2.5"), "[2J25.2.5");
        assert_eq!(
            sanitize_for_print("a\u{2066}b\u{2069}c\u{200F}d\u{061C}e\r\n"),
            "abcde"
        );
        // ZWNJ is part of ordinary Persian spelling and must survive.
        assert_eq!(sanitize_for_print("می\u{200C}خواهم"), "می\u{200C}خواهم");
    }

    #[test]
    fn flp_probe_wording_never_uses_banned_vocabulary() {
        // Every line flp-probe can print about a change, rendered with
        // neutral names, plus the FL 25 note. Musicians' own names are
        // theirs; Wit's own words must pass the Story contract's lint.
        let name = || "x".to_string();
        let changes = [
            wit_flp::FlChange::ChannelAdded {
                name: name(),
                generator: Some(name()),
            },
            wit_flp::FlChange::ChannelRemoved {
                name: name(),
                generator: None,
            },
            wit_flp::FlChange::ChannelRenamed {
                old: name(),
                new: name(),
            },
            wit_flp::FlChange::PatternAdded { name: name() },
            wit_flp::FlChange::PatternRemoved { name: name() },
            wit_flp::FlChange::PatternRenamed {
                old: name(),
                new: name(),
            },
            wit_flp::FlChange::GeneratorAdded { name: name() },
            wit_flp::FlChange::GeneratorRemoved { name: name() },
            wit_flp::FlChange::EffectAdded {
                name: name(),
                insert: Some(0),
            },
            wit_flp::FlChange::EffectRemoved {
                name: name(),
                insert: Some(3),
            },
            wit_flp::FlChange::MixerInsertAdded { name: name() },
            wit_flp::FlChange::MixerInsertRemoved { name: name() },
            wit_flp::FlChange::MixerInsertRenamed {
                old: name(),
                new: name(),
            },
            wit_flp::FlChange::ArrangementAdded { name: name() },
            wit_flp::FlChange::ArrangementRemoved { name: name() },
            wit_flp::FlChange::ArrangementRenamed {
                old: name(),
                new: name(),
            },
            wit_flp::FlChange::TempoChanged {
                from_bpm: 120.0,
                to_bpm: 128.0,
            },
            wit_flp::FlChange::BytesChangedNothingReadable,
        ];
        for change in &changes {
            let line = render_flp_change(change);
            assert!(wit_story::vocab::banned_words(&line).is_empty(), "{line}");
        }
        assert!(wit_story::vocab::banned_words(V25_NOTE).is_empty());
    }

    #[test]
    fn byte_counts_are_formatted_in_decimal_units_not_binary() {
        // The boundary that matters: 1000 B is 1.0 KB, and 1024 B is also
        // 1.0 KB rather than the binary convention's "1.0 KiB".
        assert_eq!(human_bytes(999), "999 B");
        assert_eq!(human_bytes(1_000), "1.0 KB");
        assert_eq!(human_bytes(1_024), "1.0 KB");
        // The unit the published numbers are actually quoted in. A 1024-based
        // divisor would render this as "20.7 GB" — a 7.4% understatement, and
        // the exact discrepancy that made `wit dupes` output incomparable to
        // EXPERIMENTS.md §9.
        assert_eq!(human_bytes(22_200_000_000), "22.2 GB");
        assert_eq!(human_bytes(0), "0 B");
    }

    #[test]
    fn only_audio_files_are_marked_as_a_verified_count() {
        // lFuA is the only tag docs/FORMATS.md measured matching
        // MetaData.plist exactly — every other mapped tag showed a
        // record-to-object multiplier and must stay disclaimed.
        let audio_files = tag_info("lFuA").expect("lFuA is a mapped tag");
        assert_eq!(audio_files.noun, "audio files");
        assert!(audio_files.verified_count);

        for (tag, noun) in [
            ("karT", "tracks"),
            ("gRuA", "regions"),
            ("UCuA", "plugins"),
            ("qeSM", "MIDI sequences"),
            ("qSvE", "event sequences"),
        ] {
            let info = tag_info(tag).unwrap_or_else(|| panic!("{tag} should be mapped"));
            assert_eq!(info.noun, noun);
            assert!(!info.verified_count, "{tag} has no verified 1:1 count");
        }
    }

    #[test]
    fn unmapped_and_root_tags_have_no_category() {
        assert!(tag_info("MneG").is_none());
        // gnoS (the root/song record) is deliberately excluded: wit-logic's
        // frame doc guarantees exactly one per valid file, so its count can
        // never differ between two successfully-walked files.
        assert!(tag_info("gnoS").is_none());
    }
}
