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
//! on the machine. `wit log`/`diff`/`report` land later; see
//! `docs/ROADMAP.md`.

use clap::{Parser, Subcommand};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

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
    /// Discover Logic/GarageBand/Ableton projects under `path` and
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
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::DiffAls { old, new, limit } => diff_als(&old, &new, limit),
        Command::LogicProbe { old, new } => logic_probe(&old, &new),
        Command::Scan { path, data_dir } => scan(&path, data_dir),
        Command::Dupes { path } => dupes(&path),
        Command::LogicReport { path } => logic_report(&path),
        Command::DemoLibrary { dest } => demo_library(&dest),
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
                    "    tempo: {:?} -> {:?} BPM",
                    a.extracted.tempo_bpm, b.extracted.tempo_bpm
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
        "  found {} Logic/GarageBand project(s), {} Ableton lineage(s) — {} new version(s) archived",
        result.logic_projects_found, result.ableton_lineages_found, result.new_versions_ingested
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
    use super::{human_bytes, tag_info};

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
