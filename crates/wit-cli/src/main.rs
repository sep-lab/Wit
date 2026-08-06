//! The `wit` command-line tool.
//!
//! M1 added `diff-als` (the M1 exit criterion: "`wit diff-als a.als b.als`
//! prints the golden"). M2 adds `logic-probe` — the issue #20 CLI hook,
//! comparing two Logic/GarageBand saves at the Structure honesty tier
//! (census + extracted names; see `wit-logic`'s module docs for why byte
//! comparison is a diagnostic, never the verdict). `wit
//! scan`/`log`/`diff`/`dupes`/`report` land with M3 (`wit-index`); see
//! `docs/ROADMAP.md`.

use clap::{Parser, Subcommand};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::process::ExitCode;

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
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::DiffAls { old, new, limit } => diff_als(&old, &new, limit),
        Command::LogicProbe { old, new } => logic_probe(&old, &new),
    }
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

fn print_census_diff(a: &wit_logic::Census, b: &wit_logic::Census) {
    let tags: BTreeSet<&String> = a.keys().chain(b.keys()).collect();
    for tag in tags {
        let ca = a.get(tag).copied().unwrap_or(0);
        let cb = b.get(tag).copied().unwrap_or(0);
        if ca != cb {
            println!(
                "    {tag}: {ca} -> {cb} record(s) [internal count, not a musician-facing number]"
            );
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
