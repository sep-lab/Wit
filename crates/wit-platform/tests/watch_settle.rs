//! The real OS watcher (FSEvents / inotify / ReadDirectoryChangesW) against
//! synthetic save sequences in a temp dir: chunked writes, rename-into-place,
//! Live-style `Backup/` moves, Logic-style backup rotation. Each save must
//! produce **exactly one** settled event, after the writing stops.
//!
//! No DAW is launched and nothing outside a temp dir is touched; the
//! "projects" are a few bytes of placeholder data with DAW-shaped names.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::thread::sleep;
use std::time::{Duration, Instant, SystemTime};
use wit_platform::clone::{write_file_in_restores, RestoresDir};
use wit_platform::roots::{RootKind, WatchedRoots};
use wit_platform::watch::{ProjectKind, ProjectWatcher, WatchConfig, WatchEvent};

const SETTLE: Duration = Duration::from_millis(750);
/// Gap between chunks of one save: shorter than SETTLE, so a save that
/// lasts several windows must still come out as one event.
const CHUNK_GAP: Duration = Duration::from_millis(100);

struct Rig {
    _tmp: tempfile::TempDir,
    base: PathBuf,
    restores: RestoresDir,
    watcher: ProjectWatcher,
}

/// Watch `base/watched` (as `kind`) with Restores at `base/Restores` and
/// `extra_ignore` ignored; `setup` populates the tree *before* watching.
fn rig(kind: RootKind, extra_ignore: Option<&str>, setup: impl FnOnce(&Path)) -> Rig {
    let tmp = tempfile::tempdir().unwrap();
    let base = fs::canonicalize(tmp.path()).unwrap();
    let watched = base.join("watched");
    fs::create_dir_all(&watched).unwrap();
    setup(&watched);
    let mut roots = WatchedRoots::new();
    roots.add(&watched, kind).unwrap();
    let restores = RestoresDir::new(&base.join("Restores"), &roots).unwrap();
    let mut config = WatchConfig::new(roots, &restores).unwrap().settle(SETTLE);
    if let Some(dir) = extra_ignore {
        config = config.ignore(&watched.join(dir));
    }
    // Let setup's own events age out before the stream starts.
    sleep(Duration::from_millis(300));
    let watcher = ProjectWatcher::start(config).unwrap();
    // Warm-up: drain anything the OS delivers late about the setup.
    let _ = collect(&watcher, SETTLE * 3);
    Rig {
        _tmp: tmp,
        base,
        restores,
        watcher,
    }
}

/// Every event that arrives within `window`.
fn collect(w: &ProjectWatcher, window: Duration) -> Vec<WatchEvent> {
    let deadline = Instant::now() + window;
    let mut out = Vec::new();
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        match w.recv_timeout(left) {
            Ok(e) => out.push(e),
            Err(_) => break,
        }
    }
    out
}

/// Wait (up to 15 s) for the first settled event, then keep listening for
/// a quiet window of four settle periods; return every settled event seen.
fn settled_after_save(w: &ProjectWatcher) -> Vec<(PathBuf, ProjectKind)> {
    let mut settled = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(15);
    while settled.is_empty() {
        let Some(left) = deadline.checked_duration_since(Instant::now()) else {
            break;
        };
        match w.recv_timeout(left) {
            Ok(WatchEvent::Settled { project, .. }) => settled.push((project.path, project.kind)),
            Ok(other) => eprintln!("(non-settled event: {other:?})"),
            Err(_) => break,
        }
    }
    for e in collect(w, SETTLE * 4) {
        match e {
            WatchEvent::Settled { project, .. } => settled.push((project.path, project.kind)),
            other => eprintln!("(non-settled event: {other:?})"),
        }
    }
    settled
}

/// Make every file under `dir` look last saved an hour ago. Setting a file
/// time needs a handle with write access on Windows, so open for writing
/// (without truncating) rather than with `File::open`.
fn age(dir: &Path) {
    let old = SystemTime::now() - Duration::from_secs(3_600);
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in fs::read_dir(&d).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                OpenOptions::new()
                    .write(true)
                    .open(&path)
                    .unwrap()
                    .set_modified(old)
                    .unwrap();
            }
        }
    }
}

fn write_in_chunks(path: &Path, chunks: usize) {
    let mut f = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(path)
        .unwrap();
    for i in 0..chunks {
        f.write_all(&[i as u8; 4096]).unwrap();
        f.flush().unwrap();
        f.sync_data().unwrap();
        sleep(CHUNK_GAP);
    }
}

#[test]
fn live_style_save_is_one_event_per_save() {
    let r = rig(RootKind::Discovery, None, |w| {
        fs::create_dir_all(w.join("Set Project/Backup")).unwrap();
        fs::write(w.join("Set Project/Set.als"), b"v0").unwrap();
    });
    let project = r.base.join("watched/Set Project");
    let main = fs::canonicalize(project.join("Set.als")).unwrap();

    for (n, stamp) in ["2026-09-29 101500", "2026-09-29 102000"]
        .iter()
        .enumerate()
    {
        // Write the new version to a hidden temp name in chunks, move the
        // old one into Backup/, then rename the new one into place.
        let temp = project.join(".Set.als.wit-test-tmp");
        write_in_chunks(&temp, 5);
        fs::rename(
            project.join("Set.als"),
            project.join(format!("Backup/Set [{stamp}].als")),
        )
        .unwrap();
        fs::rename(&temp, project.join("Set.als")).unwrap();
        let got = settled_after_save(&r.watcher);
        assert_eq!(
            got,
            vec![(main.clone(), ProjectKind::Ableton)],
            "save #{}",
            n + 1
        );
    }
}

#[test]
fn in_place_chunked_write_is_one_event() {
    let r = rig(RootKind::Discovery, None, |w| {
        fs::write(w.join("Beat.flp"), b"v0").unwrap();
    });
    let flp = r.base.join("watched/Beat.flp");
    // Nine chunks, 100 ms apart: the save spans ~0.9 s, longer than the
    // 0.75 s settle window, but never pauses that long.
    write_in_chunks(&flp, 9);
    let got = settled_after_save(&r.watcher);
    assert_eq!(
        got,
        vec![(fs::canonicalize(&flp).unwrap(), ProjectKind::FlStudio)]
    );
}

#[test]
fn logic_style_package_save_is_one_event() {
    let r = rig(RootKind::Discovery, None, |w| {
        let alt = w.join("Song.logicx/Alternatives/000");
        for slot in ["00", "01"] {
            fs::create_dir_all(alt.join("Project File Backups").join(slot)).unwrap();
            fs::write(
                alt.join("Project File Backups")
                    .join(slot)
                    .join("ProjectData"),
                b"old",
            )
            .unwrap();
        }
        fs::create_dir_all(w.join("Song.logicx/Resources")).unwrap();
        fs::write(alt.join("ProjectData"), b"current").unwrap();
        fs::write(alt.join("MetaData.plist"), b"meta").unwrap();
    });
    let package = r.base.join("watched/Song.logicx");
    let alt = package.join("Alternatives/000");
    // Rotate backups (01 -> 02, 00 -> 01, current -> 00) ...
    let backups = alt.join("Project File Backups");
    fs::rename(backups.join("01"), backups.join("02")).unwrap();
    fs::rename(backups.join("00"), backups.join("01")).unwrap();
    fs::create_dir(backups.join("00")).unwrap();
    fs::copy(alt.join("ProjectData"), backups.join("00/ProjectData")).unwrap();
    sleep(CHUNK_GAP);
    // ... then rewrite ProjectData in chunks and update the plists.
    write_in_chunks(&alt.join("ProjectData"), 4);
    fs::write(alt.join("MetaData.plist"), b"meta v2").unwrap();
    fs::write(package.join("Resources/ProjectInformation.plist"), b"info").unwrap();
    let got = settled_after_save(&r.watcher);
    assert_eq!(
        got,
        vec![(fs::canonicalize(&package).unwrap(), ProjectKind::Logic)]
    );

    // UI-state and media writes inside the package are not saves.
    fs::write(alt.join("DisplayState.plist"), b"ui").unwrap();
    fs::create_dir_all(package.join("Media/Audio Files")).unwrap();
    fs::write(package.join("Media/Audio Files/take.raw"), b"not a save").unwrap();
    let got = settled_after_save(&r.watcher);
    assert!(got.is_empty(), "UI/media writes produced {got:?}");
}

#[test]
fn ignored_folders_and_scratch_files_are_silent_but_user_files_count() {
    let r = rig(RootKind::UserFolder, Some("WitData"), |w| {
        fs::create_dir_all(w.join("WitData")).unwrap();
    });
    let watched = r.base.join("watched");
    // Wit's own data dir (ignored), hidden and temp names: silence.
    fs::write(watched.join("WitData/index.sqlite"), b"wit's own").unwrap();
    fs::write(watched.join(".DS_Store"), b"finder").unwrap();
    fs::write(watched.join("mix.wav.part"), b"download").unwrap();
    let got = settled_after_save(&r.watcher);
    assert!(got.is_empty(), "ignored writes produced {got:?}");
    // A plain file in a user-added folder is the generic History tier.
    let rpp = watched.join("song.rpp");
    fs::write(&rpp, b"<REAPER_PROJECT>").unwrap();
    let got = settled_after_save(&r.watcher);
    assert_eq!(
        got,
        vec![(fs::canonicalize(&rpp).unwrap(), ProjectKind::OtherDaw)]
    );
    let notes = watched.join("lyrics.txt");
    fs::write(&notes, b"verse").unwrap();
    let got = settled_after_save(&r.watcher);
    assert_eq!(
        got,
        vec![(fs::canonicalize(&notes).unwrap(), ProjectKind::Generic)]
    );
}

#[test]
fn a_restore_is_watched_and_appears_as_exactly_one_new_project() {
    let r = rig(RootKind::Discovery, None, |_| {});
    // The staging entry is hidden; only the renamed-into-place restore counts.
    let dest = r
        .restores
        .fresh_destination("Song", "2026-09-29", Some("als"))
        .unwrap();
    let landed = write_file_in_restores(dest, b"restored set").unwrap();
    let got = settled_after_save(&r.watcher);
    assert_eq!(got, vec![(landed.clone(), ProjectKind::Ableton)]);
    // A musician keeps working in the restored copy: that's history too.
    write_in_chunks(&landed, 3);
    let got = settled_after_save(&r.watcher);
    assert_eq!(got, vec![(landed, ProjectKind::Ableton)]);
}

#[test]
fn a_restore_being_staged_is_invisible_until_renamed_into_place() {
    let r = rig(RootKind::Discovery, None, |w| {
        let alt = w.join("Song.logicx/Alternatives/000");
        fs::create_dir_all(alt.join("Project File Backups/00")).unwrap();
        fs::write(alt.join("ProjectData"), b"current").unwrap();
        fs::write(alt.join("Project File Backups/00/ProjectData"), b"older").unwrap();
        // A real source was saved long before anyone restores from it. (On
        // APFS, cloning it makes FSEvents report it as modified anyway.)
        age(&w.join("Song.logicx"));
    });
    let mut new = r
        .restores
        .begin_restore("Song", "2026-09-29", Some("logicx"))
        .unwrap();
    new.clone_tree_from(&r.base.join("watched/Song.logicx"))
        .unwrap();
    new.write_file(Path::new("Alternatives/000/ProjectData"), b"older")
        .unwrap();
    // Linger in staging for three settle windows: nothing may surface, and
    // reading the source package must not look like a save either.
    let during: Vec<WatchEvent> = collect(&r.watcher, SETTLE * 3)
        .into_iter()
        .filter(|e| matches!(e, WatchEvent::Settled { .. }))
        .collect();
    assert!(during.is_empty(), "staging surfaced: {during:?}");
    let landed = new.commit().unwrap();
    let got = settled_after_save(&r.watcher);
    assert_eq!(got, vec![(landed, ProjectKind::Logic)]);
}

#[test]
fn dropping_the_watcher_stops_it() {
    let r = rig(RootKind::Discovery, None, |_| {});
    let Rig { watcher, _tmp, .. } = r;
    let started = Instant::now();
    drop(watcher);
    assert!(started.elapsed() < Duration::from_secs(5), "drop hung");
}
