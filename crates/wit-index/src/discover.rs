//! Discovery: find Logic/GarageBand packages and Ableton `.als` lineages
//! under a directory tree. Read-only — this module only ever returns
//! paths; nothing here writes (`store.rs`/`registry.rs` are the writers,
//! and neither takes a path as input, only bytes).

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogicKind {
    Logic,
    GarageBand,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogicAlternative {
    pub name: String,
    /// `Alternatives/<name>/ProjectData` — always present if the
    /// alternative directory exists at all.
    pub current: PathBuf,
    /// `Alternatives/<name>/Project File Backups/NN/ProjectData`, sorted
    /// by slot name. Empty for GarageBand (probe-verified: GarageBand has
    /// no `Project File Backups/`) and for a Logic project that hasn't
    /// accumulated any yet — never assumed present.
    pub backups: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogicProject {
    /// The bundle's filename without its extension — e.g. `"You make my
    /// crazy!"` for `You make my crazy!.logicx`.
    pub name: String,
    pub bundle_path: PathBuf,
    pub kind: LogicKind,
    pub alternatives: Vec<LogicAlternative>,
}

impl LogicProject {
    /// Every `ProjectData` path across every alternative, current save
    /// first then backups, oldest-to-current within each alternative's
    /// backup slots — the order `wit scan`'s version count means.
    pub fn all_versions(&self) -> Vec<&PathBuf> {
        let mut v = Vec::new();
        for alt in &self.alternatives {
            v.extend(alt.backups.iter());
            v.push(&alt.current);
        }
        v
    }
}

/// Walk `root` for `.logicx`/`.band` bundles. Does not descend into a
/// matched bundle looking for more (bundles don't nest), and caps
/// recursion depth defensively against a symlink cycle.
pub fn discover_logic_projects(root: &Path) -> Vec<LogicProject> {
    let mut projects = Vec::new();
    walk(root, 0, &mut |path| {
        let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
            return true; // keep descending
        };
        let kind = match ext {
            "logicx" => Some(LogicKind::Logic),
            "band" => Some(LogicKind::GarageBand),
            _ => None,
        };
        let Some(kind) = kind else { return true };
        if let Some(project) = build_logic_project(path, kind) {
            projects.push(project);
        }
        false // don't descend into a matched bundle
    });
    projects
}

fn build_logic_project(bundle_path: &Path, kind: LogicKind) -> Option<LogicProject> {
    let name = bundle_path.file_stem()?.to_string_lossy().into_owned();
    let alternatives_dir = bundle_path.join("Alternatives");
    let mut alternatives = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&alternatives_dir) {
        let mut dirs: Vec<PathBuf> = entries
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        dirs.sort();
        for alt_dir in dirs {
            let current = alt_dir.join("ProjectData");
            if !current.is_file() {
                continue;
            }
            let alt_name = alt_dir.file_name()?.to_string_lossy().into_owned();
            let mut backups = Vec::new();
            let backups_dir = alt_dir.join("Project File Backups");
            if let Ok(backup_entries) = std::fs::read_dir(&backups_dir) {
                let mut backup_dirs: Vec<PathBuf> = backup_entries
                    .filter_map(|e| e.ok())
                    .map(|e| e.path())
                    .filter(|p| p.is_dir())
                    .collect();
                backup_dirs.sort();
                for slot in backup_dirs {
                    let pd = slot.join("ProjectData");
                    if pd.is_file() {
                        backups.push(pd);
                    }
                }
            }
            alternatives.push(LogicAlternative {
                name: alt_name,
                current,
                backups,
            });
        }
    }
    if alternatives.is_empty() {
        return None; // not a real bundle — e.g. a stray directory that happens to end in .logicx
    }
    Some(LogicProject {
        name,
        bundle_path: bundle_path.to_path_buf(),
        kind,
        alternatives,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbletonLineage {
    pub name: String,
    /// Sorted chronologically — Live's autosave filenames sort
    /// lexicographically in timestamp order (`YYYY-MM-DD HHMMSS`), and a
    /// singleton (non-autosave-named) lineage always has exactly one.
    pub saves: Vec<PathBuf>,
}

/// Walk `root` for `.als` files and group them into lineages. Mirrors
/// `experiments/als_semantic_diff.py`'s `AUTOSAVE_NAME` regex
/// (` \[YYYY-MM-DD HHMMSS\]\.als$`) without adding a `regex` dependency —
/// the pattern is fixed-width and simple enough to match by hand. A file
/// matching the pattern joins the lineage named by its prefix; a file that
/// doesn't (a deliberately-named save like `v1.als`) becomes its own
/// singleton lineage.
pub fn discover_ableton_lineages(root: &Path) -> Vec<AbletonLineage> {
    let mut by_name: std::collections::BTreeMap<String, Vec<PathBuf>> =
        std::collections::BTreeMap::new();
    walk(root, 0, &mut |path| {
        if path.extension().and_then(|e| e.to_str()) == Some("als") {
            if let Some(filename) = path.file_name().and_then(|f| f.to_str()) {
                let lineage_name = autosave_lineage_name(filename).unwrap_or_else(|| {
                    filename
                        .strip_suffix(".als")
                        .unwrap_or(filename)
                        .to_string()
                });
                by_name
                    .entry(lineage_name)
                    .or_default()
                    .push(path.to_path_buf());
            }
        }
        true
    });
    by_name
        .into_iter()
        .map(|(name, mut saves)| {
            saves.sort();
            AbletonLineage { name, saves }
        })
        .collect()
}

/// If `filename` matches `"<name> [YYYY-MM-DD HHMMSS].als"`, return
/// `<name>`. Otherwise `None`.
fn autosave_lineage_name(filename: &str) -> Option<String> {
    let base = filename.strip_suffix(".als")?;
    if base.len() < 20 {
        return None;
    }
    let tail = &base[base.len() - 20..];
    let bytes = tail.as_bytes();
    if bytes[0] != b' ' || bytes[1] != b'[' || bytes[19] != b']' || bytes[12] != b' ' {
        return None;
    }
    let date = &tail[2..12];
    let time = &tail[13..19];
    let date_ok = date.as_bytes().iter().enumerate().all(|(i, &c)| {
        if i == 4 || i == 7 {
            c == b'-'
        } else {
            c.is_ascii_digit()
        }
    });
    let time_ok = time.bytes().all(|c| c.is_ascii_digit());
    if !date_ok || !time_ok {
        return None;
    }
    Some(base[..base.len() - 20].to_string())
}

/// Recursive directory walk with a depth cap (defends against a symlink
/// cycle without needing an inode-visited set). `visit` returns `true` to
/// keep descending into a directory, `false` to stop there.
fn walk(dir: &Path, depth: usize, visit: &mut impl FnMut(&Path) -> bool) {
    const MAX_DEPTH: usize = 16;
    if depth > MAX_DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.is_dir() {
            if visit(&path) {
                walk(&path, depth + 1, visit);
            }
        } else {
            visit(&path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"x").unwrap();
    }

    #[test]
    fn discovers_a_logic_project_with_current_and_backups() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = dir.path().join("Song.logicx");
        touch(&bundle.join("Alternatives/000/ProjectData"));
        touch(&bundle.join("Alternatives/000/Project File Backups/00/ProjectData"));
        touch(&bundle.join("Alternatives/000/Project File Backups/01/ProjectData"));

        let projects = discover_logic_projects(dir.path());
        assert_eq!(projects.len(), 1);
        let p = &projects[0];
        assert_eq!(p.name, "Song");
        assert_eq!(p.kind, LogicKind::Logic);
        assert_eq!(p.alternatives.len(), 1);
        assert_eq!(p.alternatives[0].backups.len(), 2);
        assert_eq!(p.all_versions().len(), 3);
    }

    #[test]
    fn discovers_a_garageband_project_with_no_backups() {
        let dir = tempfile::tempdir().unwrap();
        let bundle = dir.path().join("Jam.band");
        touch(&bundle.join("Alternatives/000/ProjectData"));
        // No Project File Backups/ at all — probe-verified real GarageBand shape.

        let projects = discover_logic_projects(dir.path());
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].kind, LogicKind::GarageBand);
        assert!(projects[0].alternatives[0].backups.is_empty());
    }

    #[test]
    fn a_directory_ending_in_logicx_with_no_alternatives_is_not_a_project() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("Empty.logicx")).unwrap();
        assert!(discover_logic_projects(dir.path()).is_empty());
    }

    #[test]
    fn discovers_multiple_projects_at_different_depths() {
        let dir = tempfile::tempdir().unwrap();
        touch(&dir.path().join("A.logicx/Alternatives/000/ProjectData"));
        touch(
            &dir.path()
                .join("nested/deeper/B.logicx/Alternatives/000/ProjectData"),
        );
        let projects = discover_logic_projects(dir.path());
        assert_eq!(projects.len(), 2);
    }

    #[test]
    fn autosave_lineage_name_parses_lives_naming_convention() {
        assert_eq!(
            autosave_lineage_name("Undertow [2026-05-05 095412].als"),
            Some("Undertow".to_string())
        );
        assert_eq!(autosave_lineage_name("v1.als"), None);
        assert_eq!(autosave_lineage_name("mix_final.als"), None);
    }

    #[test]
    fn ableton_lineages_group_autosaves_and_isolate_deliberate_names() {
        let dir = tempfile::tempdir().unwrap();
        touch(&dir.path().join("Song [2026-05-05 095412].als"));
        touch(&dir.path().join("Song [2026-05-05 095508].als"));
        touch(&dir.path().join("v1.als"));

        let mut lineages = discover_ableton_lineages(dir.path());
        lineages.sort_by(|a, b| a.name.cmp(&b.name));

        assert_eq!(lineages.len(), 2);
        assert_eq!(lineages[0].name, "Song");
        assert_eq!(lineages[0].saves.len(), 2);
        assert_eq!(lineages[1].name, "v1");
        assert_eq!(lineages[1].saves.len(), 1);
    }
}
