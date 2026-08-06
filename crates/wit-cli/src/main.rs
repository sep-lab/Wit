//! The `wit` command-line tool.
//!
//! M1 ships exactly one subcommand — `diff-als` — because it is the M1
//! exit criterion (PLAN.md: "`wit diff-als a.als b.als` prints the
//! golden"). `wit scan`/`log`/`diff`/`dupes`/`report`/`logic-probe` land
//! with M3 (`wit-index`); see `docs/ROADMAP.md`.

use clap::{Parser, Subcommand};
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
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::DiffAls { old, new, limit } => diff_als(&old, &new, limit),
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
