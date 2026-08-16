//! Build a synthetic, `~/Music`-shaped library so Wit's first-run, timeline
//! and watcher are demoable on any machine — not only one with a real Logic
//! library on it (issue #18 / `just demo-library`).
//!
//! **Why this is its own crate.** `wit-index` documents, as a safety
//! property enforced by construction, that its only write API takes bytes
//! rather than a path — so no caller can hand it a project path even by
//! mistake. A generator that writes a directory tree to a path the user
//! names is exactly the capability that property excludes, so it lives
//! here instead of eroding the claim. The guard rail moves with it:
//! [`build_demo_library`] refuses any destination that is not empty, so it
//! can never overwrite a real library even if pointed at one.
//!
//! **What the generated files are.** Real `ProjectData` and `.als`
//! *framing*, carrying only the whitelisted fields Wit extracts. Logic and
//! Live would not open them. They are fixtures for Wit's own readers, and
//! must never be described to a user as projects.
//!
//! The chain is shaped to match measured reality rather than to look good:
//! 3 of the 9 Logic save pairs are byte-different but structurally
//! identical, matching the 33% empty-verdict rate measured across 32 real
//! projects (`docs/EXPERIMENTS.md` §11). A demo where every save has
//! something to show would misrepresent the product.

pub mod ableton;
pub mod logic;

use ableton::{ClipSpec, SetSpec, TrackSpec};
use logic::{SongSpec, VERSION_GARAGEBAND, VERSION_LOGIC};
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub enum DemoError {
    /// The destination exists and has something in it. Never overwrite —
    /// the one thing this tool must not do is clobber a real library
    /// because someone typed `~/Music` instead of `/tmp/demo`.
    DestinationNotEmpty(PathBuf),
    Io(String),
}

impl std::fmt::Display for DemoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DemoError::DestinationNotEmpty(p) => write!(
                f,
                "{} already exists and is not empty — refusing to write a demo library over it",
                p.display()
            ),
            DemoError::Io(msg) => write!(f, "failed to write the demo library: {msg}"),
        }
    }
}

impl std::error::Error for DemoError {}

impl From<std::io::Error> for DemoError {
    fn from(e: std::io::Error) -> Self {
        DemoError::Io(e.to_string())
    }
}

/// What was written, for the CLI to report and tests to assert on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DemoLibrary {
    pub root: PathBuf,
    pub logic_projects: usize,
    pub garageband_projects: usize,
    pub ableton_lineages: usize,
    /// Every `ProjectData` plus every `.als` written.
    pub total_versions: usize,
}

fn write(path: &Path, bytes: &[u8]) -> Result<(), DemoError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, bytes)?;
    Ok(())
}

fn is_empty_dir(path: &Path) -> Result<bool, DemoError> {
    Ok(std::fs::read_dir(path)?.next().is_none())
}

/// The 10-save chain for the headline demo project: 9 consecutive pairs, of
/// which 3 show nothing Wit can see. Each entry is one save, oldest first.
///
/// The edits are the ones a producer actually makes, in an order that tells
/// a story when read top to bottom in the app's timeline — that is the
/// point of the demo, and it is why this is a hand-written list rather than
/// something generated from a seed.
fn coastline_chain() -> Vec<SongSpec> {
    let base = SongSpec {
        tempo_bpm: 120.0,
        track_names: vec!["Rhodes".into(), "Upright Bass".into()],
        region_names: vec!["Verse Rhodes".into()],
        audio_file_names: vec!["Upright Bass.caf".into()],
        churn: 0,
    };

    let mut chain = Vec::new();
    chain.push(base.clone()); // 0 — the first save

    // 1 — you nudged a fader and saved. Nothing Wit can see on Logic.
    chain.push(base.with_churn(1));

    // 2 — recorded the chorus.
    let mut v = base.with_churn(2);
    v.region_names.push("Chorus Rhodes".into());
    chain.push(v.clone());

    // 3 — named the bass track properly.
    v = v.with_churn(3);
    v.track_names[1] = "Upright Bass (DI)".into();
    chain.push(v.clone());

    // 4 — another invisible save.
    chain.push(v.with_churn(4));

    // 5 — pushed the tempo.
    v = v.with_churn(5);
    v.tempo_bpm = 124.0;
    chain.push(v.clone());

    // 6 — dragged in a drum loop.
    v = v.with_churn(6);
    v.audio_file_names.push("Brushed Kit 124.caf".into());
    v.region_names.push("Brushed Kit".into());
    chain.push(v.clone());

    // 7 — a third invisible save.
    chain.push(v.with_churn(7));

    // 8 — added a pad.
    v = v.with_churn(8);
    v.track_names.push("Wurli Pad".into());
    chain.push(v.clone());

    // 9 — the current save: doubled the chorus.
    v = v.with_churn(9);
    v.region_names.push("Chorus Rhodes 2".into());
    chain.push(v);

    chain
}

/// A shorter second project, so the Shelf has more than one card and the
/// "two alternatives" shape (Logic's own branching) is exercised.
fn night_bus_chain() -> Vec<SongSpec> {
    let base = SongSpec {
        tempo_bpm: 88.0,
        track_names: vec!["Tape Drums".into()],
        region_names: vec!["Intro".into()],
        audio_file_names: vec!["Tape Drums.caf".into()],
        churn: 0,
    };
    let mut second = base.with_churn(1);
    second.region_names.push("Verse".into());
    let mut third = second.with_churn(2);
    third.track_names.push("Sub".into());
    vec![base, second, third]
}

/// Write one Logic/GarageBand bundle: `Alternatives/<alt>/ProjectData` for
/// the newest save, and `Project File Backups/NN/ProjectData` for the older
/// ones — the exact layout `wit-index::discover` walks, and the real
/// on-disk shape (backups oldest-first in slots `00`..`09`, current save
/// outside them).
fn write_logic_bundle(
    bundle: &Path,
    alternative: &str,
    chain: &[SongSpec],
    version: [u8; 2],
    with_backups: bool,
) -> Result<usize, DemoError> {
    let alt_dir = bundle.join("Alternatives").join(alternative);
    let (backups, current) = chain.split_at(chain.len() - 1);

    let mut written = 0;
    if with_backups {
        for (slot, spec) in backups.iter().enumerate() {
            let path = alt_dir
                .join("Project File Backups")
                .join(format!("{slot:02}"))
                .join("ProjectData");
            write(&path, &logic::build_project_data(spec, version))?;
            written += 1;
        }
    }
    write(
        &alt_dir.join("ProjectData"),
        &logic::build_project_data(&current[0], version),
    )?;
    written += 1;

    Ok(written)
}

fn coastline_als_chain() -> Vec<SetSpec> {
    let base = SetSpec {
        creator: "Ableton Live 12.4.2".into(),
        tempo_bpm: 120.0,
        tracks: vec![
            TrackSpec {
                id: 8,
                name: "Rhodes".into(),
                volume: 0.7943282127,
                pan: 0.0,
                devices: vec!["Eq8".into()],
                device_knob: 1.0,
                clips: vec![ClipSpec {
                    id: 3,
                    name: "verse rhodes".into(),
                    start: 0.0,
                    end: 16.0,
                    sample: "rhodes take 3.wav".into(),
                    disabled: false,
                }],
            },
            TrackSpec {
                id: 9,
                name: "Upright Bass".into(),
                volume: 0.6606934,
                pan: -0.15,
                devices: vec!["Compressor2".into()],
                device_knob: 0.5,
                clips: vec![],
            },
        ],
    };

    // 1 — pure bookkeeping: nothing in the whitelist moves, so the app must
    // say "no musical change detected" rather than inventing something.
    let bookkeeping = base.clone();

    // 2 — turned the Rhodes down.
    let mut quieter = base.clone();
    quieter.tracks[0].volume = 0.5248075;

    // 3 — added a filter and muted the clip under it.
    let mut filtered = quieter.clone();
    filtered.tracks[0].devices.push("AutoFilter".into());
    filtered.tracks[0].clips[0].disabled = true;

    // 4 — renamed the sample in Finder, and pushed the tempo.
    let mut renamed = filtered.clone();
    renamed.tracks[0].clips[0].sample = "rhodes FINAL.wav".into();
    renamed.tempo_bpm = 124.0;

    vec![base, bookkeeping, quieter, filtered, renamed]
}

/// Live's autosave filenames, which `wit-index::discover` parses to group a
/// lineage: `<name> [YYYY-MM-DD HHMMSS].als`. Fixed timestamps, never
/// `now()` — the generated tree has to be byte-identical run to run.
const ALS_TIMESTAMPS: [&str; 5] = [
    "2026-01-04 101500",
    "2026-01-04 103012",
    "2026-01-04 111845",
    "2026-01-05 200133",
    "2026-01-05 204417",
];

/// Build the whole demo library under `dest`.
///
/// Refuses unless `dest` is missing or an empty directory. Deterministic:
/// the same `dest` twice produces byte-identical files, with no clock or
/// RNG anywhere in the generator.
pub fn build_demo_library(dest: &Path) -> Result<DemoLibrary, DemoError> {
    if dest.exists() && !is_empty_dir(dest)? {
        return Err(DemoError::DestinationNotEmpty(dest.to_path_buf()));
    }
    std::fs::create_dir_all(dest)?;

    let mut total_versions = 0;

    // Logic — the headline project, 10 saves in one alternative.
    total_versions += write_logic_bundle(
        &dest.join("Logic/Coastline.logicx"),
        "000",
        &coastline_chain(),
        VERSION_LOGIC,
        true,
    )?;

    // Logic — a second project with two alternatives, so the Shelf shows
    // more than one card and Logic's own branching is represented.
    let night_bus = dest.join("Logic/Night Bus.logicx");
    let night_bus_chain = night_bus_chain();
    total_versions += write_logic_bundle(&night_bus, "000", &night_bus_chain, VERSION_LOGIC, true)?;
    total_versions += write_logic_bundle(
        &night_bus,
        "001",
        &night_bus_chain[..2],
        VERSION_LOGIC,
        true,
    )?;

    // GarageBand — one alternative, no backups. That is the real shape:
    // the probe confirmed GarageBand keeps no `Project File Backups`, so a
    // demo that gave it some would teach the wrong thing.
    total_versions += write_logic_bundle(
        &dest.join("GarageBand/Kitchen Jam.band"),
        "000",
        &night_bus_chain[..1],
        VERSION_GARAGEBAND,
        false,
    )?;

    // Ableton — one lineage of 5 autosaves, in Live's own Backup/ layout.
    let backup_dir = dest.join("Ableton/Coastline Project/Backup");
    for (spec, stamp) in coastline_als_chain().iter().zip(ALS_TIMESTAMPS) {
        let path = backup_dir.join(format!("Coastline [{stamp}].als"));
        write(&path, &ableton::build_als(spec)?)?;
        total_versions += 1;
    }

    Ok(DemoLibrary {
        root: dest.to_path_buf(),
        logic_projects: 2,
        garageband_projects: 1,
        ableton_lineages: 1,
        total_versions,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn built() -> (tempfile::TempDir, DemoLibrary) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("demo");
        let lib = build_demo_library(&root).unwrap();
        (dir, lib)
    }

    #[test]
    fn reports_what_it_wrote() {
        let (_dir, lib) = built();
        assert_eq!(lib.logic_projects, 2);
        assert_eq!(lib.garageband_projects, 1);
        assert_eq!(lib.ableton_lineages, 1);
        // 10 (Coastline) + 3 + 2 (Night Bus, two alternatives) + 1
        // (GarageBand) + 5 (.als) = 21.
        assert_eq!(lib.total_versions, 21);
    }

    #[test]
    fn wit_index_discovers_every_project_it_writes() {
        // The property that actually matters: the app finds these through
        // the same discovery path it uses on a real library.
        let (_dir, lib) = built();
        let projects = wit_index::discover_logic_projects(&lib.root);
        assert_eq!(projects.len(), 3, "2 Logic + 1 GarageBand");

        let coastline = projects.iter().find(|p| p.name == "Coastline").unwrap();
        assert_eq!(coastline.kind, wit_index::LogicKind::Logic);
        assert_eq!(coastline.all_versions().len(), 10);

        let night_bus = projects.iter().find(|p| p.name == "Night Bus").unwrap();
        assert_eq!(night_bus.alternatives.len(), 2);

        let jam = projects.iter().find(|p| p.name == "Kitchen Jam").unwrap();
        assert_eq!(jam.kind, wit_index::LogicKind::GarageBand);
        assert!(
            jam.alternatives[0].backups.is_empty(),
            "GarageBand keeps no Project File Backups — the demo must not invent any"
        );
    }

    #[test]
    fn wit_index_groups_the_ableton_saves_into_one_lineage() {
        let (_dir, lib) = built();
        let lineages = wit_index::discover_ableton_lineages(&lib.root);
        assert_eq!(lineages.len(), 1);
        assert_eq!(lineages[0].name, "Coastline");
        assert_eq!(lineages[0].saves.len(), 5);
    }

    #[test]
    fn the_logic_chain_reproduces_the_measured_empty_verdict_rate() {
        // EXPERIMENTS.md §11 measured 33% of real save pairs as showing no
        // structural change. If the demo drifts away from that, first-run
        // stops representing what a pilot user will actually see.
        let (_dir, lib) = built();
        let projects = wit_index::discover_logic_projects(&lib.root);
        let coastline = projects.iter().find(|p| p.name == "Coastline").unwrap();
        let versions = coastline.all_versions();

        let mut empty = 0;
        let mut byte_different_but_empty = 0;
        for pair in versions.windows(2) {
            let (a_bytes, b_bytes) = (
                std::fs::read(pair[0]).unwrap(),
                std::fs::read(pair[1]).unwrap(),
            );
            let a = wit_logic::walk(&a_bytes).unwrap();
            let b = wit_logic::walk(&b_bytes).unwrap();
            if wit_logic::semantic_equal(&a, &b) == wit_logic::Verdict::NoStructuralChange {
                empty += 1;
                if a_bytes != b_bytes {
                    byte_different_but_empty += 1;
                }
            }
        }
        assert_eq!(versions.len() - 1, 9, "9 consecutive pairs");
        assert_eq!(empty, 3, "3 of 9 pairs empty = 33%, matching §11");
        assert_eq!(
            byte_different_but_empty, 3,
            "and all 3 are byte-different — §11's 28% case"
        );
    }

    #[test]
    fn the_ableton_lineage_opens_with_a_bookkeeping_only_save() {
        // The demo's first sentence is "no musical change detected", which
        // is the single most distinctive thing Wit says. Assert it holds.
        let (_dir, lib) = built();
        let lineage = &wit_index::discover_ableton_lineages(&lib.root)[0];
        let a = wit_als::parse_file(&lineage.saves[0]).unwrap();
        let b = wit_als::parse_file(&lineage.saves[1]).unwrap();
        assert!(wit_diff::diff(&a, &b).is_empty());

        // ...and the next pair does have something to say.
        let c = wit_als::parse_file(&lineage.saves[2]).unwrap();
        assert!(!wit_diff::diff(&b, &c).is_empty());
    }

    #[test]
    fn refuses_to_write_into_a_non_empty_directory() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("MyRealSong.logicx"), b"precious").unwrap();
        let err = build_demo_library(dir.path()).unwrap_err();
        assert!(matches!(err, DemoError::DestinationNotEmpty(_)));
        // And it wrote nothing.
        assert!(!dir.path().join("Logic").exists());
        assert_eq!(
            std::fs::read(dir.path().join("MyRealSong.logicx")).unwrap(),
            b"precious"
        );
    }

    #[test]
    fn an_existing_empty_directory_is_fine() {
        let dir = tempfile::tempdir().unwrap();
        assert!(build_demo_library(dir.path()).is_ok());
    }

    #[test]
    fn two_builds_produce_byte_identical_trees() {
        let (_a_dir, a) = built();
        let (_b_dir, b) = built();
        for project in wit_index::discover_logic_projects(&a.root) {
            for version in project.all_versions() {
                let relative = version.strip_prefix(&a.root).unwrap();
                assert_eq!(
                    std::fs::read(version).unwrap(),
                    std::fs::read(b.root.join(relative)).unwrap(),
                    "{} differs between runs",
                    relative.display()
                );
            }
        }
    }
}
