//! Opt-in real-material check: walk every `.flp` under `WIT_FIXTURES` and
//! assert `wit_flp::parse` never panics and reports a clean parse (or a
//! typed error, never a crash). Mirrors the `WIT_FIXTURES` discipline
//! `tests/conftest.py` and `wit-diff`/`wit-logic`'s own real-fixture tests
//! already follow: **loudly skipped** by default, never touches real
//! material unless asked, never reads a path from anywhere but this env
//! var (this crate's own source never hardcodes a personal path).
//!
//! Run recursively against a real FL Studio library:
//!
//! ```text
//! WIT_FIXTURES=/path/to/FL/library cargo test -p wit-flp --test real_fixtures -- --nocapture --ignored
//! ```

use std::path::{Path, PathBuf};

fn fixtures_dir() -> Option<PathBuf> {
    std::env::var_os("WIT_FIXTURES").map(PathBuf::from)
}

/// Recursively collect every `.flp` file under `root` — real FL libraries
/// nest project files and `Backup/` autosave folders arbitrarily deep, so
/// a flat `read_dir` (as `wit-diff`'s `.als`-flat real_fixtures test uses)
/// would miss most of them. Depth-capped defensively against a symlink
/// cycle, matching `wit-index::discover`'s own walk.
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

#[test]
#[ignore = "opt-in: set WIT_FIXTURES to a real FL Studio library and pass --ignored"]
fn real_library_walks_clean_and_reports_versions_seen() {
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
    let mut versions: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    let mut v25_plus = 0usize;
    let mut with_tempo = 0usize;

    for path in &flps {
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("  SKIP (unreadable): {}: {e}", path.display());
                continue;
            }
        };
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
                eprintln!(
                    "  OK    {}: version={:?} channels={} {} channel name(s), {} pattern name(s), \
                     {} plugin name(s), tempo={}",
                    path.file_name().unwrap().to_string_lossy(),
                    extracted.fl_version,
                    extracted.channels,
                    extracted.channel_names.len(),
                    extracted.pattern_names.len(),
                    extracted.plugin_names.len(),
                    extracted.tempo,
                );
            }
            Err(e) => {
                eprintln!(
                    "  ERROR {}: {e}",
                    path.file_name().unwrap().to_string_lossy()
                );
                errors.push((path.clone(), e));
            }
        }
    }

    eprintln!(
        "\n{} file(s) found, {clean} parsed clean, {} error(s)",
        flps.len(),
        errors.len()
    );
    eprintln!("versions seen: {versions:?}");
    eprintln!("{v25_plus} file(s) flagged v25+ (scalars partial), {with_tempo} file(s) with a readable tempo");

    assert!(
        errors.is_empty(),
        "every real .flp should walk clean; failures: {errors:?}"
    );
}
