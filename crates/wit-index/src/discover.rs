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

/// One FL Studio project: its own `.flp`, plus FL's own `Backup/` autosave
/// chain of it, if one was found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlpProject {
    pub name: String,
    /// The project's own current save, wherever it lives. `None` for a
    /// **backup-only lineage** — FL's default autosave layout is one
    /// shared `Backup/` folder per "Projects" root (probe-verified this
    /// session: a real autosave chain lived at `<Projects
    /// root>/Backup/<name> (autosaved at <time>).flp`, a sibling of the
    /// project's *own* subfolder, not of the `.flp` file itself), and a
    /// project renamed via Save As leaves its old-named autosaves with no
    /// current file to match. Never dropped — archive-before-recycle
    /// applies here the same way it does to every other DAW this crate
    /// discovers.
    pub current: Option<PathBuf>,
    /// `Backup/<name> (autosaved at <time>).flp`, oldest first **by
    /// filesystem modification time — not by filename.** This is a
    /// deliberate divergence from [`discover_ableton_lineages`]: Ableton's
    /// autosave names embed a full date (`[YYYY-MM-DD HHMMSS]`), so
    /// lexicographic order is chronological order. FL's autosave names
    /// carry only a time of day (`"... (autosaved at 5h56)"`), no date —
    /// measured this session on 4 real autosaves of one project: two were
    /// named with hours that sort "later" (`"16h34"`) than a file that was
    /// actually written a full day *earlier* than a `"5h36"`/`"7h59"`
    /// pair. Sorting these by name would silently misorder the lineage.
    pub backups: Vec<PathBuf>,
}

impl FlpProject {
    /// Every version, oldest backup first, current save last — the same
    /// convention [`LogicProject::all_versions`] uses.
    pub fn all_versions(&self) -> Vec<&PathBuf> {
        let mut v: Vec<&PathBuf> = self.backups.iter().collect();
        v.extend(self.current.iter());
        v
    }
}

/// Walk `root` for `.flp` files and FL Studio's own `Backup/<name>
/// (autosaved at <time>).flp` autosave chain, grouped into one lineage
/// per project name — the same grouping strategy
/// [`discover_ableton_lineages`] uses, adapted to FL's current-file-plus-
/// shared-Backup-folder shape rather than Ableton's flat lineage-of-equals
/// one (see [`FlpProject`]'s doc for why).
///
/// **Known limitation, inherited from the same design
/// [`discover_ableton_lineages`] already accepts:** grouping is by name
/// alone, globally across `root`, so two unrelated projects that happen to
/// share a filename in different folders would incorrectly merge into one
/// lineage. Scoping backups to a per-project directory instead would avoid
/// that, but would also miss the real, measured shape above (one shared
/// `Backup/` folder per Projects root) — this crate matches what FL
/// Studio actually does on disk over what would be safest in the
/// abstract, and says so here rather than silently.
pub fn discover_flp_projects(root: &Path) -> Vec<FlpProject> {
    let mut current_by_name: std::collections::BTreeMap<String, Vec<PathBuf>> =
        std::collections::BTreeMap::new();
    let mut backups_by_name: std::collections::BTreeMap<String, Vec<PathBuf>> =
        std::collections::BTreeMap::new();

    walk(root, 0, &mut |path| {
        if path.extension().and_then(|e| e.to_str()) != Some("flp") {
            return true;
        }
        let Some(filename) = path.file_name().and_then(|f| f.to_str()) else {
            return true;
        };
        let in_backup_dir = path
            .parent()
            .and_then(|p| p.file_name())
            .is_some_and(|n| n == "Backup");
        if in_backup_dir {
            if let Some(name) = flp_autosave_lineage_name(filename) {
                backups_by_name
                    .entry(name)
                    .or_default()
                    .push(path.to_path_buf());
                return true;
            }
            // A file inside a Backup/ dir that doesn't match FL's own
            // autosave naming (a manual copy someone dropped there) falls
            // through to the ordinary-current-file branch below instead
            // of being silently ignored.
        }
        let name = filename
            .strip_suffix(".flp")
            .unwrap_or(filename)
            .to_string();
        current_by_name
            .entry(name)
            .or_default()
            .push(path.to_path_buf());
        true
    });

    let mut names: std::collections::BTreeSet<String> = current_by_name.keys().cloned().collect();
    names.extend(backups_by_name.keys().cloned());

    names
        .into_iter()
        .map(|name| {
            let mut backups = backups_by_name.remove(&name).unwrap_or_default();
            backups.sort_by_key(|p| {
                std::fs::metadata(p)
                    .and_then(|m| m.modified())
                    .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
            });
            let mut currents = current_by_name.remove(&name).unwrap_or_default();
            currents.sort(); // deterministic pick if the same name somehow occurs twice
            let current = currents.into_iter().next();
            FlpProject {
                name,
                current,
                backups,
            }
        })
        .collect()
}

/// If `filename` matches FL Studio's own autosave naming,
/// `"<name> (autosaved at <time>).flp"`, return `<name>`. `<time>` is
/// deliberately never parsed as a clock time — see [`FlpProject::backups`]
/// for why it cannot be used to order the chain.
fn flp_autosave_lineage_name(filename: &str) -> Option<String> {
    let base = filename.strip_suffix(".flp")?;
    const MARKER: &str = " (autosaved at ";
    let marker_start = base.find(MARKER)?;
    if !base.ends_with(')') {
        return None;
    }
    Some(base[..marker_start].to_string())
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

    fn touch_at(path: &Path, epoch_secs: u64) {
        touch(path);
        let mtime = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(epoch_secs);
        // Windows refuses to set times through a read-only handle.
        std::fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(mtime)
            .unwrap();
    }

    #[test]
    fn flp_autosave_lineage_name_parses_fls_own_naming_convention() {
        assert_eq!(
            flp_autosave_lineage_name("untitled (autosaved at 5h56).flp"),
            Some("untitled".to_string())
        );
        assert_eq!(
            flp_autosave_lineage_name("My Song (autosaved at 16h34).flp"),
            Some("My Song".to_string())
        );
        assert_eq!(flp_autosave_lineage_name("v1.flp"), None);
        assert_eq!(flp_autosave_lineage_name("Project_1.flp"), None);
    }

    #[test]
    fn discovers_an_flp_project_with_current_and_backups() {
        let dir = tempfile::tempdir().unwrap();
        touch(&dir.path().join("Song.flp"));
        touch(&dir.path().join("Backup/Song (autosaved at 1h00).flp"));
        touch(&dir.path().join("Backup/Song (autosaved at 2h00).flp"));

        let projects = discover_flp_projects(dir.path());
        assert_eq!(projects.len(), 1);
        let p = &projects[0];
        assert_eq!(p.name, "Song");
        assert_eq!(p.current, Some(dir.path().join("Song.flp")));
        assert_eq!(p.backups.len(), 2);
        assert_eq!(p.all_versions().len(), 3);
    }

    #[test]
    fn flp_backups_are_ordered_by_modification_time_not_by_filename() {
        // FL's own autosave names carry only a time of day, no date
        // (measured this session — see FlpProject::backups). Name "5h36"
        // is written LATER (mtime 2000) than name "7h59" (mtime 1000),
        // which sorting by filename would get backwards.
        let dir = tempfile::tempdir().unwrap();
        let later_by_name_earlier_by_time = dir.path().join("Backup/Song (autosaved at 7h59).flp");
        let earlier_by_name_later_by_time = dir.path().join("Backup/Song (autosaved at 5h36).flp");
        touch_at(&later_by_name_earlier_by_time, 1000);
        touch_at(&earlier_by_name_later_by_time, 2000);

        let projects = discover_flp_projects(dir.path());
        assert_eq!(projects.len(), 1);
        let backups = &projects[0].backups;
        assert_eq!(backups.len(), 2);
        assert!(
            backups[0].to_string_lossy().contains("7h59"),
            "the older-by-mtime file must sort first regardless of its name: {backups:?}"
        );
        assert!(backups[1].to_string_lossy().contains("5h36"));
    }

    #[test]
    fn a_backup_only_lineage_is_never_dropped() {
        // The project was renamed via Save As, so its old-named autosaves
        // have no current file to match — archive-before-recycle: still
        // discovered, current is None, nothing is lost.
        let dir = tempfile::tempdir().unwrap();
        touch(&dir.path().join("Backup/untitled (autosaved at 5h56).flp"));
        touch(&dir.path().join("Renamed.flp")); // a different, unrelated project

        let mut projects = discover_flp_projects(dir.path());
        projects.sort_by(|a, b| a.name.cmp(&b.name));

        assert_eq!(projects.len(), 2);
        assert_eq!(projects[0].name, "Renamed");
        assert_eq!(projects[0].current, Some(dir.path().join("Renamed.flp")));
        assert!(projects[0].backups.is_empty());
        assert_eq!(projects[1].name, "untitled");
        assert_eq!(projects[1].current, None);
        assert_eq!(projects[1].backups.len(), 1);
        assert_eq!(projects[1].all_versions().len(), 1);
    }

    #[test]
    fn discovers_multiple_flp_projects_at_different_depths() {
        let dir = tempfile::tempdir().unwrap();
        touch(&dir.path().join("A.flp"));
        touch(&dir.path().join("nested/deeper/B.flp"));
        let projects = discover_flp_projects(dir.path());
        assert_eq!(projects.len(), 2);
    }
}
