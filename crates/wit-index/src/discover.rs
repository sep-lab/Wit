//! Discovery: find Logic/GarageBand packages and Ableton `.als` lineages
//! under a directory tree. Read-only — this module only ever returns
//! paths; nothing here writes (`store.rs`/`registry.rs` are the writers,
//! and neither takes a path as input, only bytes).

use std::path::{Path, PathBuf};
use std::time::SystemTime;

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
    /// `Alternatives/<name>/Project File Backups/NN/ProjectData`, sorted by
    /// **slot name** (`00`..`09`) — this is a raw directory listing, not a
    /// save order. Empty for GarageBand (probe-verified: GarageBand has no
    /// `Project File Backups/`) and for a Logic project that hasn't
    /// accumulated any yet — never assumed present.
    ///
    /// **Slot order is not save order.** The slots are a ring: Logic writes
    /// the next backup into the next slot and wraps from `09` back to `00`,
    /// so on any project old enough to have wrapped once, slot `00` holds a
    /// *newer* save than slot `09`. Anything that needs chronological
    /// order — [`LogicProject::all_versions`], `wit-index::report`'s
    /// `logic_report` — sorts by file modification time instead (tie-broken
    /// by slot name, so two saves in the same second stay deterministic).
    /// Found in review: `report.rs` originally paired backups in this
    /// field's slot order and so mis-paired a wrapped chain.
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

/// A path's modification time, or the Unix epoch when it can't be read
/// (permissions, a race with a deletion, ...) — never a panic, and a
/// deliberately *old* fallback so an unreadable file sorts first rather
/// than jumping the queue as if it were newest.
pub(crate) fn mtime_or_epoch(path: &Path) -> SystemTime {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH)
}

/// Sort a chain of save paths into real chronological order: by file
/// modification time, tie-broken by the path itself (which embeds the slot
/// name for a backup) so the order is deterministic even when two saves
/// land in the same mtime second. This is the one ordering every consumer
/// that cares about save order should use — see [`LogicAlternative::backups`]
/// for why slot order alone is not it.
pub(crate) fn sort_by_save_time(paths: &mut [&PathBuf]) {
    paths.sort_by(|a, b| {
        mtime_or_epoch(a)
            .cmp(&mtime_or_epoch(b))
            .then_with(|| a.cmp(b))
    });
}

impl LogicProject {
    /// Every `ProjectData` path across every alternative, in real
    /// chronological order **within each alternative** (oldest backup to
    /// the current save, by file modification time — see
    /// [`LogicAlternative::backups`]); alternatives themselves keep their
    /// existing relative order, since Logic's alternatives are siblings
    /// with no shared timeline to interleave them into.
    pub fn all_versions(&self) -> Vec<&PathBuf> {
        let mut v = Vec::new();
        for alt in &self.alternatives {
            let mut chain: Vec<&PathBuf> = alt
                .backups
                .iter()
                .chain(std::iter::once(&alt.current))
                .collect();
            sort_by_save_time(&mut chain);
            v.extend(chain);
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
    /// The project's own current save, wherever it lives. Every current
    /// `.flp` file discovery finds becomes its own [`FlpProject`] — never
    /// merged with another one just because it shares a name, and never
    /// dropped (see [`discover_flp_projects`]'s doc for the bug this
    /// closed). `None` only for a **backup-only lineage**: a `Backup/`
    /// autosave chain with no unambiguous current file under its Projects
    /// root (the project was renamed via Save As, moved out of the Projects
    /// root, or never saved — FL's `"untitled"`). Never dropped either —
    /// archive-before-recycle applies here the same way it does to every
    /// other DAW this crate discovers.
    pub current: Option<PathBuf>,
    /// The `Backup/` folder [`FlpProject::backups`] came from, if any. An
    /// autosave chain is one lineage name *in one `Backup/` folder* —
    /// never merged across folders — so this is a stable identity for a
    /// backup-only lineage however many of its autosaves FL has rotated
    /// away.
    pub backup_folder: Option<PathBuf>,
    /// `Backup/<name> (autosaved at <time>).flp`, oldest first **by
    /// filesystem modification time, not by filename, with the path as a
    /// tie-break** when two backups report the same (or an unreadable)
    /// modification time. This is a deliberate divergence from
    /// [`discover_ableton_lineages`]: Ableton's autosave names embed a
    /// full date (`[YYYY-MM-DD HHMMSS]`), so lexicographic order is
    /// chronological order. FL's autosave names carry only a time of day
    /// (`"... (autosaved at 5h56)"`), no date — measured this session on 4
    /// real autosaves of one project: two were named with hours that sort
    /// "later" (`"16h34"`) than a file that was actually written a full
    /// day *earlier* than a `"5h36"`/`"7h59"` pair. Sorting these by name
    /// would silently misorder the lineage.
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
/// (autosaved at <time>).flp` autosave chains.
///
/// **One project per current file, always — never dropped, never merged
/// with another current file just because they share a name.** (An
/// earlier version kept only one of several same-named current files.)
///
/// **One autosave chain per (`Backup/` folder, lineage name)** — FL
/// shares one `Backup/` folder across a whole Projects root, so a folder
/// holds many chains, but two folders' `"untitled (autosaved at …)"` files
/// are two different projects and are never merged. A chain is *attached*
/// to a current file only when that is unambiguous, with the Projects
/// root being the `Backup/` folder's parent:
///
/// 1. Exactly one same-named current file under the Projects root ->
///    attach to it.
/// 2. Several -> attach to the one at FL's own save location for that
///    name (`<root>/<name>/<name>.flp` or `<root>/<name>.flp`) if exactly
///    one of those exists. So copying a project folder inside the
///    Projects root (`<root>/Song copy/Song.flp`) leaves the original's
///    chain where it was, instead of detaching it and re-archiving it as
///    a new backup-only project.
///
/// A same-named file *outside* the Projects root never receives the chain:
/// `"untitled"` is FL's own name for every unsaved project, so an
/// unrelated `untitled.flp` in, say, Downloads would otherwise collect a
/// Projects root's untitled autosaves. The accepted cost: a project moved
/// out of its Projects root keeps its autosaves as a separate backup-only
/// project. When nested Projects roots both claim one file, the nearest
/// (deepest) root's chain wins. Every chain left unattached becomes its
/// own backup-only [`FlpProject`] (`current: None`) — never dropped.
pub fn discover_flp_projects(root: &Path) -> Vec<FlpProject> {
    use std::collections::BTreeMap;

    let mut currents: Vec<PathBuf> = Vec::new();
    // (Backup folder, lineage name) -> that chain's autosaves.
    let mut chains: BTreeMap<ChainKey, Vec<PathBuf>> = BTreeMap::new();

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
            if let (Some(name), Some(folder)) = (flp_autosave_lineage_name(filename), path.parent())
            {
                chains
                    .entry((folder.to_path_buf(), name))
                    .or_default()
                    .push(path.to_path_buf());
                return true;
            }
            // A file inside a Backup/ dir that doesn't match FL's own
            // autosave naming (a manual copy someone dropped there) falls
            // through to the ordinary-current-file branch below instead
            // of being silently ignored.
        }
        currents.push(path.to_path_buf());
        true
    });

    let mut currents_by_name: BTreeMap<String, Vec<&PathBuf>> = BTreeMap::new();
    for current in &currents {
        currents_by_name
            .entry(flp_lineage_name(current))
            .or_default()
            .push(current);
    }

    // Each chain's candidate current file (rules 1–2 in the doc above),
    // with how deep the chain's Projects root is (nearest root wins).
    let mut claims: BTreeMap<&PathBuf, Vec<(&ChainKey, usize)>> = BTreeMap::new();
    for key in chains.keys() {
        let (folder, name) = key;
        let Some(projects_root) = folder.parent() else {
            continue;
        };
        let near: Vec<&PathBuf> = currents_by_name
            .get(name)
            .map(Vec::as_slice)
            .unwrap_or(&[])
            .iter()
            .copied()
            .filter(|c| c.starts_with(projects_root))
            .collect();
        let claim = match near.as_slice() {
            [] => None,
            [only] => Some(*only),
            several => {
                let r = projects_root;
                let own = [
                    r.join(name).join(format!("{name}.flp")),
                    r.join(format!("{name}.flp")),
                ];
                let at_own: Vec<&PathBuf> = several
                    .iter()
                    .copied()
                    .filter(|c| own.iter().any(|o| o == *c))
                    .collect();
                match at_own.as_slice() {
                    [only] => Some(*only),
                    _ => None,
                }
            }
        };
        if let Some(current) = claim {
            let depth = projects_root.components().count();
            claims.entry(current).or_default().push((key, depth));
        }
    }
    // A current file claimed by several (nested) Projects roots' chains
    // keeps the nearest root's, if exactly one is nearest.
    let mut attached: BTreeMap<&PathBuf, &ChainKey> = BTreeMap::new();
    for (current, chain_keys) in claims {
        let deepest = chain_keys.iter().map(|(_, depth)| *depth).max();
        let nearest: Vec<&ChainKey> = chain_keys
            .iter()
            .filter(|(_, depth)| Some(*depth) == deepest)
            .map(|(key, _)| *key)
            .collect();
        if let [key] = nearest.as_slice() {
            attached.insert(current, *key);
        }
    }

    let mut projects = Vec::new();
    let mut used: std::collections::BTreeSet<&ChainKey> = Default::default();
    for current in &currents {
        let chain = attached.get(current).copied();
        if let Some(key) = chain {
            used.insert(key);
        }
        projects.push(FlpProject {
            name: flp_lineage_name(current),
            current: Some(current.clone()),
            backup_folder: chain.map(|(folder, _)| folder.clone()),
            backups: chain
                .map(|key| sort_backups(chains[key].clone()))
                .unwrap_or_default(),
        });
    }
    // Every chain not attached to a current file becomes its own
    // backup-only project. Archive-before-recycle: never dropped.
    for (key, paths) in &chains {
        if used.contains(key) {
            continue;
        }
        let (folder, name) = key;
        projects.push(FlpProject {
            name: name.clone(),
            current: None,
            backup_folder: Some(folder.clone()),
            backups: sort_backups(paths.clone()),
        });
    }

    projects
}

/// One autosave chain: its `Backup/` folder and lineage name.
type ChainKey = (PathBuf, String);

fn flp_lineage_name(path: &Path) -> String {
    path.file_name()
        .and_then(|f| f.to_str())
        .and_then(|f| f.strip_suffix(".flp"))
        .unwrap_or("")
        .to_string()
}

/// Oldest first by filesystem modification time, with the path itself as
/// a deterministic tie-break when two backups report the same (or an
/// unreadable) modification time — see [`FlpProject::backups`].
fn sort_backups(mut backups: Vec<PathBuf>) -> Vec<PathBuf> {
    backups.sort_by(|a, b| {
        let mtime = |p: &Path| {
            std::fs::metadata(p)
                .and_then(|m| m.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
        };
        mtime(a).cmp(&mtime(b)).then_with(|| a.cmp(b))
    });
    backups
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
    fn all_versions_orders_a_wrapped_backup_ring_by_mtime_not_by_slot() {
        // The ring already wrapped: slots 08/09 hold the oldest two saves,
        // slots 00/01 the next two (written after wrapping past 09), and
        // the current save is newest. Slot-alphabetical order would read
        // 00, 01, 08, 09, current — real chronological order is
        // 08, 09, 00, 01, current.
        let dir = tempfile::tempdir().unwrap();
        let bundle = dir.path().join("Song.logicx");
        let alt = bundle.join("Alternatives/000");
        let chronological = [
            (alt.join("Project File Backups/08/ProjectData"), 1_000),
            (alt.join("Project File Backups/09/ProjectData"), 2_000),
            (alt.join("Project File Backups/00/ProjectData"), 3_000),
            (alt.join("Project File Backups/01/ProjectData"), 4_000),
            (alt.join("ProjectData"), 5_000),
        ];
        for (path, secs) in &chronological {
            touch_at(path, *secs);
        }

        let projects = discover_logic_projects(dir.path());
        let versions = projects[0].all_versions();
        let expected: Vec<&PathBuf> = chronological.iter().map(|(p, _)| p).collect();
        assert_eq!(
            versions, expected,
            "must follow mtime order, not slot-name order"
        );
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
    fn backups_with_the_same_modification_time_are_ordered_by_path() {
        // The sort's tie-break: equal mtimes (or unreadable ones, which
        // read as the epoch) fall back to path order, so the chain's order
        // never depends on directory-listing order.
        let dir = tempfile::tempdir().unwrap();
        let written_first = dir.path().join("Backup/Song (autosaved at 9h00).flp");
        let written_second = dir.path().join("Backup/Song (autosaved at 10h00).flp");
        let written_third = dir.path().join("Backup/Song (autosaved at 1h00).flp");
        for p in [&written_first, &written_second, &written_third] {
            touch_at(p, 1000);
        }
        let sorted = sort_backups(vec![
            written_first.clone(),
            written_second.clone(),
            written_third.clone(),
        ]);
        // Byte-wise path order: "10h00" < "1h00" ('0' < 'h') < "9h00" —
        // deterministic, not chronological (see FlpProject::backups).
        assert_eq!(sorted, vec![written_second, written_third, written_first]);
        // And the tie-break is total: shuffled input, same output.
        let again = sort_backups(sorted.iter().rev().cloned().collect());
        assert_eq!(again, sorted);
    }

    #[test]
    fn same_named_chains_in_different_backup_folders_stay_separate() {
        // Two Projects roots, each with FL's default "untitled" autosaves:
        // two different projects, never one merged chain.
        let dir = tempfile::tempdir().unwrap();
        touch(&dir.path().join("A/Backup/untitled (autosaved at 1h00).flp"));
        touch(&dir.path().join("A/Backup/untitled (autosaved at 2h00).flp"));
        touch(&dir.path().join("B/Backup/untitled (autosaved at 1h00).flp"));

        let mut projects = discover_flp_projects(dir.path());
        projects.sort_by(|a, b| a.backup_folder.cmp(&b.backup_folder));
        assert_eq!(projects.len(), 2, "{projects:?}");
        assert_eq!(projects[0].backup_folder, Some(dir.path().join("A/Backup")));
        assert_eq!(projects[0].backups.len(), 2);
        assert_eq!(projects[1].backup_folder, Some(dir.path().join("B/Backup")));
        assert_eq!(projects[1].backups.len(), 1);
        assert!(projects.iter().all(|p| p.current.is_none()));
    }

    #[test]
    fn each_chain_attaches_to_the_project_under_its_own_projects_root() {
        let dir = tempfile::tempdir().unwrap();
        touch(&dir.path().join("A/Song/Song.flp"));
        touch(&dir.path().join("A/Backup/Song (autosaved at 1h00).flp"));
        touch(&dir.path().join("B/Song.flp"));
        touch(&dir.path().join("B/Backup/Song (autosaved at 1h00).flp"));
        touch(&dir.path().join("B/Backup/Song (autosaved at 2h00).flp"));

        let projects = discover_flp_projects(dir.path());
        assert_eq!(projects.len(), 2, "{projects:?}");
        let a = projects
            .iter()
            .find(|p| p.current == Some(dir.path().join("A/Song/Song.flp")))
            .unwrap();
        assert_eq!(a.backup_folder, Some(dir.path().join("A/Backup")));
        assert_eq!(a.backups.len(), 1);
        let b = projects
            .iter()
            .find(|p| p.current == Some(dir.path().join("B/Song.flp")))
            .unwrap();
        assert_eq!(b.backup_folder, Some(dir.path().join("B/Backup")));
        assert_eq!(b.backups.len(), 2);
    }

    #[test]
    fn copying_a_project_does_not_detach_its_backups() {
        // FL's own layout: Projects/Song/Song.flp with the chain in
        // Projects/Backup. A copy of the project folder inside the same
        // Projects root, and another copy elsewhere, must leave the chain
        // on the original — detaching it would re-archive every autosave
        // as a new backup-only project.
        let dir = tempfile::tempdir().unwrap();
        let original = dir.path().join("Projects/Song/Song.flp");
        touch(&original);
        touch(
            &dir.path()
                .join("Projects/Backup/Song (autosaved at 1h00).flp"),
        );
        touch(&dir.path().join("Projects/Song copy/Song.flp"));
        touch(&dir.path().join("Desktop/Song.flp"));

        let projects = discover_flp_projects(dir.path());
        assert_eq!(projects.len(), 3, "{projects:?}");
        let with_backups: Vec<&FlpProject> =
            projects.iter().filter(|p| !p.backups.is_empty()).collect();
        assert_eq!(with_backups.len(), 1, "{projects:?}");
        assert_eq!(with_backups[0].current, Some(original));
        assert!(projects.iter().all(|p| p.current.is_some()));
    }

    #[test]
    fn two_copies_at_neither_fl_location_leave_the_chain_on_its_own() {
        // Nothing marks either copy as the original: attach to neither.
        let dir = tempfile::tempdir().unwrap();
        touch(&dir.path().join("Projects/Old/Song.flp"));
        touch(&dir.path().join("Projects/New/Song.flp"));
        touch(
            &dir.path()
                .join("Projects/Backup/Song (autosaved at 1h00).flp"),
        );
        let projects = discover_flp_projects(dir.path());
        let backup_only: Vec<&FlpProject> =
            projects.iter().filter(|p| p.current.is_none()).collect();
        assert_eq!(backup_only.len(), 1, "{projects:?}");
        assert_eq!(backup_only[0].backups.len(), 1);
    }

    #[test]
    fn an_untitled_file_elsewhere_never_receives_a_projects_roots_autosaves() {
        // "untitled" is FL's own name for any unsaved project: a stray
        // untitled.flp outside the Projects root is not that chain's save.
        let dir = tempfile::tempdir().unwrap();
        touch(
            &dir.path()
                .join("Projects/Backup/untitled (autosaved at 1h00).flp"),
        );
        touch(&dir.path().join("Downloads/untitled.flp"));

        let projects = discover_flp_projects(dir.path());
        assert_eq!(projects.len(), 2, "{projects:?}");
        let downloaded = projects
            .iter()
            .find(|p| p.current == Some(dir.path().join("Downloads/untitled.flp")))
            .unwrap();
        assert!(downloaded.backups.is_empty(), "{projects:?}");
        let chain = projects.iter().find(|p| p.current.is_none()).unwrap();
        assert_eq!(chain.backups.len(), 1);
    }

    #[test]
    fn nested_projects_roots_give_a_file_the_nearest_roots_chain() {
        let dir = tempfile::tempdir().unwrap();
        touch(&dir.path().join("A/B/Song/Song.flp"));
        touch(&dir.path().join("A/Backup/Song (autosaved at 1h00).flp"));
        touch(&dir.path().join("A/B/Backup/Song (autosaved at 2h00).flp"));

        let projects = discover_flp_projects(dir.path());
        let song = projects.iter().find(|p| p.current.is_some()).unwrap();
        assert_eq!(song.backup_folder, Some(dir.path().join("A/B/Backup")));
        assert_eq!(
            projects.len(),
            2,
            "the outer chain stays its own: {projects:?}"
        );
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

    #[test]
    fn same_named_current_files_in_different_folders_are_never_dropped() {
        // Reproduces the bug this fix closes: two unrelated projects that
        // happen to share a filename used to be bucketed into one lineage
        // and only one of them ever got archived. SongA/beat.flp and
        // SongB/beat.flp are unrelated projects; Backup/beat.flp is a
        // third, unrelated file that happens to live in the Backup/
        // folder without matching FL's autosave naming (so it is an
        // ordinary current file, not an autosave); Backup/beat (autosaved
        // at 1h00).flp is a real autosave. All four must be discovered
        // and none silently merged away.
        let dir = tempfile::tempdir().unwrap();
        touch(&dir.path().join("SongA/beat.flp"));
        touch(&dir.path().join("SongB/beat.flp"));
        touch(&dir.path().join("Backup/beat.flp"));
        touch(&dir.path().join("Backup/beat (autosaved at 1h00).flp"));

        let projects = discover_flp_projects(dir.path());

        let all_currents: Vec<&PathBuf> =
            projects.iter().filter_map(|p| p.current.as_ref()).collect();
        assert_eq!(
            all_currents.len(),
            3,
            "all three current files sharing the name 'beat' must be discovered: {projects:?}"
        );
        assert!(all_currents.contains(&&dir.path().join("SongA/beat.flp")));
        assert!(all_currents.contains(&&dir.path().join("SongB/beat.flp")));
        assert!(all_currents.contains(&&dir.path().join("Backup/beat.flp")));

        // The name is ambiguous (three current files share it), so the
        // one real autosave must not be silently guessed onto any of
        // them -- it survives as its own backup-only lineage instead.
        let total_backups: usize = projects.iter().map(|p| p.backups.len()).sum();
        assert_eq!(
            total_backups, 1,
            "the one real autosave must not be dropped: {projects:?}"
        );
        let backup_only: Vec<&FlpProject> =
            projects.iter().filter(|p| p.current.is_none()).collect();
        assert_eq!(backup_only.len(), 1);
        assert_eq!(backup_only[0].backups.len(), 1);
    }

    #[test]
    fn an_unambiguous_current_far_from_the_backup_dir_is_still_preferred_over_a_nearer_ambiguity() {
        // Two current files share the name "Song": one lives under the
        // same Projects root as the Backup/ folder, the other lives
        // entirely elsewhere in the tree. The nearby one is the
        // unambiguous preferred match.
        let dir = tempfile::tempdir().unwrap();
        touch(&dir.path().join("Projects/MySong/Song.flp"));
        touch(
            &dir.path()
                .join("Projects/Backup/Song (autosaved at 1h00).flp"),
        );
        touch(&dir.path().join("Elsewhere/Song.flp"));

        let projects = discover_flp_projects(dir.path());
        let with_backups: Vec<&FlpProject> =
            projects.iter().filter(|p| !p.backups.is_empty()).collect();
        assert_eq!(with_backups.len(), 1, "{projects:?}");
        assert_eq!(
            with_backups[0].current,
            Some(dir.path().join("Projects/MySong/Song.flp"))
        );
    }
}
