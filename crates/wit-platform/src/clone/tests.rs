use super::*;
use crate::roots::RootKind;

struct World {
    _dir: tempfile::TempDir,
    base: PathBuf,
    data: PathBuf,
    roots: WatchedRoots,
}

/// A temp dir with one watched root holding a fake package.
fn world() -> World {
    let dir = tempfile::tempdir().unwrap();
    let base = fs::canonicalize(dir.path()).unwrap();
    let watched = base.join("Music/Logic");
    let package = watched.join("Song.logicx/Alternatives/000");
    fs::create_dir_all(package.join("Project File Backups/00")).unwrap();
    fs::write(package.join("ProjectData"), b"current").unwrap();
    fs::write(
        package.join("Project File Backups/00/ProjectData"),
        b"older",
    )
    .unwrap();
    let mut roots = WatchedRoots::new();
    roots.add(&watched, RootKind::Discovery).unwrap();
    World {
        _dir: dir,
        data: base.join("WitData"),
        base,
        roots,
    }
}

impl World {
    fn restores(&self, rel: &str) -> Result<RestoresDir, CloneError> {
        RestoresDir::new(&self.base.join(rel), &self.roots, &self.data)
    }
}

fn snapshot(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in fs::read_dir(&d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                stack.push(p);
            } else {
                out.push((p.clone(), fs::read(&p).unwrap()));
            }
        }
    }
    out.sort();
    out
}

fn names_in(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

#[test]
fn restores_may_be_inside_or_equal_to_a_watched_root() {
    let w = world();
    let inside = w.base.join("Music/Logic/Wit Restores");
    w.restores("Music/Logic/Wit Restores").unwrap();
    assert!(inside.is_dir());
    w.restores("Music/Logic").unwrap();
}

#[test]
fn refuses_restores_above_another_watched_root() {
    let w = world();
    assert!(matches!(
        w.restores("Music"),
        Err(CloneError::ContainsWatchedRoot { .. })
    ));
}

#[test]
fn refuses_restores_inside_a_project_folder_of_any_daw_and_creates_nothing() {
    let w = world();
    // Logic package, spelled with `..`, a trailing separator and odd case.
    assert!(matches!(
        w.restores("Music/Logic/Song.logicx/Wit Restores"),
        Err(CloneError::InsideProject { .. })
    ));
    assert!(!w.base.join("Music/Logic/Song.logicx/Wit Restores").exists());
    assert!(w.restores("Other/../MUSIC/logic/Song.LOGICX/x/").is_err());
    assert!(!w.base.join("Other").exists());

    // Ableton: a folder holding `Ableton Project Info`.
    fs::create_dir_all(w.base.join("Live/Set Project/Ableton Project Info")).unwrap();
    assert!(matches!(
        w.restores("Live/Set Project/Restores"),
        Err(CloneError::InsideProject { .. })
    ));

    // FL: a folder whose .flp shares its name — refused however deep below.
    fs::create_dir_all(w.base.join("FLProj/new")).unwrap();
    fs::write(w.base.join("FLProj/FLProj.flp"), b"flp").unwrap();
    assert!(matches!(
        w.restores("FLProj/new/Wit Restores"),
        Err(CloneError::InsideProject { .. })
    ));
    assert!(!w.base.join("FLProj/new/Wit Restores").exists());

    // FL: a folder holding Backup/ with .flp autosaves.
    fs::create_dir_all(w.base.join("Beats/Backup")).unwrap();
    fs::write(w.base.join("Beats/Backup/Beat (autosave).flp"), b"flp").unwrap();
    assert!(matches!(
        w.restores("Beats/Wit Restores"),
        Err(CloneError::InsideProject { .. })
    ));
}

#[test]
fn a_loose_project_file_in_a_parent_does_not_make_it_a_project() {
    let w = world();
    fs::write(w.base.join("Music/stray set.als"), b"als").unwrap();
    fs::write(w.base.join("Music/stray beat.flp"), b"flp").unwrap();
    w.restores("Music/Wit Restores").unwrap();
}

#[cfg(unix)]
#[test]
fn placement_checks_fail_closed_on_unreadable_folders() {
    use std::os::unix::fs::PermissionsExt;
    let w = world();
    let locked = w.base.join("locked");
    fs::create_dir(&locked).unwrap();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
    // Running as root would ignore the permission bits; nothing to test then.
    let denied = fs::symlink_metadata(locked.join("x"))
        .is_err_and(|e| e.kind() == io::ErrorKind::PermissionDenied);
    let result = w.restores("locked/sub/Restores");
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
    if denied {
        assert!(
            matches!(result, Err(CloneError::CannotVerifyPlacement { .. })),
            "{result:?}"
        );
        assert!(!locked.join("sub").exists());
    }
}

#[test]
fn a_restore_inside_the_watched_root_leaves_every_existing_file_alone() {
    let w = world();
    let restores = w.restores("Music/Logic/Wit Restores").unwrap();
    let before = snapshot(&w.base.join("Music"));
    let dest = restores
        .fresh_destination("Song", "d", Some("logicx"))
        .unwrap();
    let restored = clone_tree(&w.base.join("Music/Logic/Song.logicx"), dest).unwrap();
    let after = snapshot(&w.base.join("Music"));
    let new: Vec<&PathBuf> = after
        .iter()
        .filter(|e| !before.contains(e))
        .map(|(p, _)| p)
        .collect();
    assert!(
        before.iter().all(|e| after.contains(e)),
        "an existing file changed"
    );
    assert!(new.iter().all(|p| p.starts_with(&restored.path)), "{new:?}");
}

#[test]
fn creates_a_valid_restores_folder_and_its_parents() {
    let w = world();
    let path = w.base.join("Music/Wit Restores/nested");
    let restores = w.restores("Music/Wit Restores/nested").unwrap();
    assert!(path.is_dir());
    assert_eq!(restores.path(), fs::canonicalize(&path).unwrap());
    assert_eq!(restores.data_dir(), fs::canonicalize(&w.data).unwrap());
    restores.revalidate().unwrap();
}

#[test]
fn the_data_folder_may_not_be_inside_the_restores_folder() {
    let w = world();
    let r = RestoresDir::new(&w.base.join("R"), &w.roots, &w.base.join("R/data"));
    assert!(matches!(r, Err(CloneError::InvalidDataDir { .. })), "{r:?}");
}

// POSIX only: on Windows, `..` is resolved lexically before a path ever
// reaches Wit (`PathBuf::join` onto a verbatim path and `GetFullPathName`
// both collapse it), so there is no `..` left to refuse.
#[cfg(unix)]
#[test]
fn refuses_dot_dot_after_a_missing_folder() {
    let w = world();
    assert!(w.restores("missing/../Restores").is_err());
}

#[test]
fn clone_tree_copies_a_package_and_never_touches_the_source() {
    let w = world();
    let restores = w.restores("Restores").unwrap();
    let src = w.base.join("Music/Logic/Song.logicx");
    let before = snapshot(&w.base.join("Music"));
    let dest = restores
        .fresh_destination("Song", "2026-09-29", Some("logicx"))
        .unwrap();
    assert_eq!(dest.file_name(), "Song — 2026-09-29.logicx");
    let restored = clone_tree(&src, dest).unwrap();
    assert_eq!(snapshot(&w.base.join("Music")), before);
    assert_eq!(
        fs::read(restored.path.join("Alternatives/000/ProjectData")).unwrap(),
        b"current"
    );
    assert_eq!(restored.report.total_bytes, 12);
    assert_eq!(
        restored.report.reflinked_files + restored.report.copied_files,
        2
    );
    assert_eq!(restored.leftover_staging, None);
    assert_eq!(
        names_in(restores.path()),
        vec!["Song — 2026-09-29.logicx".to_string()]
    );
    assert!(restores.staging_leftovers().unwrap().is_empty());
}

#[test]
fn a_second_restore_gets_a_fresh_name_and_never_overwrites() {
    let w = world();
    let restores = w.restores("Restores").unwrap();
    let a = write_file_in_restores(
        restores
            .fresh_destination("Song", "d", Some("als"))
            .unwrap(),
        b"first",
    )
    .unwrap()
    .path;
    let b = write_file_in_restores(
        restores
            .fresh_destination("Song", "d", Some("als"))
            .unwrap(),
        b"second",
    )
    .unwrap()
    .path;
    assert_ne!(a, b);
    assert!(b.ends_with("Song — d (2).als"));
    assert_eq!(fs::read(&a).unwrap(), b"first");
    assert_eq!(fs::read(&b).unwrap(), b"second");
}

#[test]
fn a_name_taken_after_minting_is_bumped_at_commit() {
    let w = world();
    let restores = w.restores("Restores").unwrap();
    let dest = restores
        .fresh_destination("Song", "d", Some("als"))
        .unwrap();
    fs::write(dest.path(), b"theirs").unwrap(); // someone else takes the name first
    let ours = write_file_in_restores(dest, b"ours").unwrap().path;
    assert!(ours.ends_with("Song — d (2).als"));
    assert_eq!(
        fs::read(restores.path().join("Song — d.als")).unwrap(),
        b"theirs"
    );
}

#[test]
fn staged_restore_swaps_a_file_and_clears_backups_atomically() {
    let w = world();
    let restores = w.restores("Restores").unwrap();
    let src = w.base.join("Music/Logic/Song.logicx");
    let before = snapshot(&w.base.join("Music"));
    let dest = restores
        .fresh_destination("Song", "old", Some("logicx"))
        .unwrap();
    let final_path = dest.path();
    let mut new = NewRestore::begin(dest).unwrap();
    new.clone_tree_from(&src).unwrap();
    assert!(!final_path.exists(), "nothing appears before commit");
    new.write_file(Path::new("Alternatives/000/ProjectData"), b"older")
        .unwrap();
    let removed = remove_dir_contents_in_restores(
        &mut new,
        Path::new("Alternatives/000/Project File Backups"),
    )
    .unwrap();
    assert_eq!(removed, 1);
    let landed = new.commit().unwrap();
    assert_eq!(landed.path, final_path);
    assert_eq!(
        landed.report.total_bytes, 12,
        "the clone report is carried to commit"
    );
    assert_eq!(
        fs::read(landed.path.join("Alternatives/000/ProjectData")).unwrap(),
        b"older"
    );
    assert!(
        fs::read_dir(landed.path.join("Alternatives/000/Project File Backups"))
            .unwrap()
            .next()
            .is_none()
    );
    assert_eq!(
        snapshot(&w.base.join("Music")),
        before,
        "the source is untouched"
    );
}

#[test]
fn dropping_a_new_restore_leaves_nothing_behind() {
    let w = world();
    let restores = w.restores("Restores").unwrap();
    let mut new = restores.begin_restore("Song", "x", None).unwrap();
    new.write_file(Path::new("a/b"), b"x").unwrap();
    drop(new);
    assert!(names_in(restores.path()).is_empty());
    assert!(restores.staging_leftovers().unwrap().is_empty());
}

#[test]
fn staged_paths_refuse_escapes() {
    let w = world();
    let restores = w.restores("Restores").unwrap();
    let mut new = restores.begin_restore("Song", "x", None).unwrap();
    let absolute = w
        .base
        .join("Music/Logic/Song.logicx/Alternatives/000/ProjectData");
    for bad in [
        Path::new("../escape"),
        Path::new("a/../../escape"),
        Path::new("./a"),
        absolute.as_path(),
        Path::new("a:stream"),
        Path::new("CON"),
        Path::new("trailing."),
        Path::new(""),
    ] {
        assert!(
            new.write_file(bad, b"x").is_err(),
            "should refuse {}",
            bad.display()
        );
    }
    assert!(new.remove_dir_contents(Path::new("..")).is_err());
    assert_eq!(
        fs::read(&absolute).unwrap(),
        b"current",
        "the watched project is untouched"
    );
}

#[cfg(unix)]
#[test]
fn symlinks_in_the_source_are_skipped_not_reproduced() {
    let w = world();
    let restores = w.restores("Restores").unwrap();
    let src = w.base.join("Music/Logic/Song.logicx");
    std::os::unix::fs::symlink(w.base.join("Music/Logic"), src.join("Media")).unwrap();
    let dest = restores
        .fresh_destination("Song", "x", Some("logicx"))
        .unwrap();
    let restored = clone_tree(&src, dest).unwrap();
    assert_eq!(
        restored.report.skipped_symlinks,
        vec![PathBuf::from("Media")]
    );
    assert!(fs::symlink_metadata(restored.path.join("Media")).is_err());
}

#[cfg(unix)]
#[test]
fn a_symlink_planted_inside_a_new_restore_is_not_followed() {
    let w = world();
    let restores = w.restores("Restores").unwrap();
    let mut new = restores.begin_restore("Song", "x", None).unwrap();
    let project = w.base.join("Music/Logic/Song.logicx/Alternatives/000");
    std::os::unix::fs::symlink(&project, new.staging_path().join("evil")).unwrap();
    assert!(matches!(
        new.write_file(Path::new("evil/ProjectData"), b"pwned"),
        Err(CloneError::NotARealEntry { .. })
    ));
    assert!(new.remove_dir_contents(Path::new("evil")).is_err());
    assert_eq!(fs::read(project.join("ProjectData")).unwrap(), b"current");
    // A tampered tree is never published ...
    assert!(matches!(
        new.commit(),
        Err(CloneError::NotARealEntry { .. })
    ));
    // ... and its cleanup removed the link, not the target.
    assert!(project.join("ProjectData").is_file());
    assert!(names_in(restores.path()).is_empty());
}

#[cfg(unix)]
#[test]
fn a_hard_link_planted_inside_a_new_restore_blocks_the_commit() {
    let w = world();
    let restores = w.restores("Restores").unwrap();
    let mut new = restores.begin_restore("Song", "x", None).unwrap();
    new.write_file(Path::new("own"), b"mine").unwrap();
    let project = w
        .base
        .join("Music/Logic/Song.logicx/Alternatives/000/ProjectData");
    fs::hard_link(&project, new.staging_path().join("stolen")).unwrap();
    assert!(matches!(
        new.commit(),
        Err(CloneError::NotARealEntry { .. })
    ));
    assert_eq!(fs::read(&project).unwrap(), b"current");
    assert!(names_in(restores.path()).is_empty());
}

#[cfg(unix)]
#[test]
fn a_restores_folder_swapped_for_a_symlink_is_caught_before_writing() {
    let w = world();
    let restores = w.restores("Restores").unwrap();
    let path = w.base.join("Restores");
    fs::remove_dir(&path).unwrap();
    std::os::unix::fs::symlink(w.base.join("Music/Logic"), &path).unwrap();
    assert!(matches!(
        restores.fresh_destination("Song", "x", Some("als")),
        Err(CloneError::RestoresMoved { .. })
    ));
}

#[cfg(unix)]
#[test]
fn a_restores_folder_moved_into_a_project_mid_restore_stops_the_restore() {
    let w = world();
    let restores = w.restores("Restores").unwrap();
    let mut new = restores.begin_restore("Song", "x", None).unwrap();
    let inside = w.base.join("Music/Logic/Song.logicx/Moved");
    fs::rename(w.base.join("Restores"), &inside).unwrap();
    assert!(matches!(
        new.write_file(Path::new("f"), b"x"),
        Err(CloneError::RestoresMoved { .. })
    ));
    assert!(matches!(
        new.commit(),
        Err(CloneError::RestoresMoved { .. })
    ));
}

#[test]
fn bad_extensions_are_refused() {
    let w = world();
    let restores = w.restores("Restores").unwrap();
    for ext in ["", "a/b", "../x", "logicx ", &"a".repeat(17)] {
        assert!(
            restores.fresh_destination("S", "d", Some(ext)).is_err(),
            "{ext}"
        );
    }
}

#[test]
fn hostile_long_names_fit_and_land() {
    let w = world();
    let restores = w.restores("Restores").unwrap();
    for song in [
        "🎹".repeat(200),
        "音楽".repeat(150),
        format!("Song\u{202E}{}", "x".repeat(300)),
    ] {
        let dest = restores
            .fresh_destination(&song, "2026-09-29", Some("als"))
            .unwrap();
        assert!(!dest.file_name().contains('\u{202E}'));
        let landed = write_file_in_restores(dest, b"x").unwrap();
        assert!(landed.path.is_file());
    }
}

#[test]
fn a_crash_leftover_is_journaled_listed_and_removable_only_while_unchanged() {
    let w = world();
    let restores = w.restores("Restores").unwrap();
    // A crash mid-restore: a staging folder created and journaled exactly as
    // `NewRestore::begin` does, half-written, then the process is gone
    // (no Drop, no commit, no handles left open).
    let staging = {
        let root = restores.pin().unwrap();
        let name = staging_name();
        root.mkdir(OsStr::new(&name)).unwrap();
        journal_created(&restores, &root, &name).unwrap();
        restores.path().join(name)
    };
    fs::write(staging.join("half"), b"half").unwrap();
    // A later "run": a fresh RestoresDir on the same folders.
    let later = w.restores("Restores").unwrap();
    let leftovers = later.staging_leftovers().unwrap();
    assert_eq!(leftovers.len(), 1);
    assert_eq!(leftovers[0].path, staging);
    // A new restore never touches it.
    write_file_in_restores(later.fresh_destination("Other", "x", None).unwrap(), b"y").unwrap();
    assert!(staging.join("half").is_file());
    // Explicit removal works while it is unchanged ...
    later.remove_staging_leftover(&leftovers[0]).unwrap();
    assert!(!staging.exists());
    assert!(later.staging_leftovers().unwrap().is_empty());
    // ... and is refused once gone (or replaced by something else).
    fs::create_dir(&staging).unwrap();
    assert!(matches!(
        later.remove_staging_leftover(&leftovers[0]),
        Err(CloneError::NotAStagingLeftover { .. })
    ));
    assert!(staging.is_dir(), "a non-journaled entry is never removed");
}

#[test]
fn unjournaled_staging_look_alikes_are_never_listed() {
    let w = world();
    let restores = w.restores("Restores").unwrap();
    fs::create_dir(restores.path().join(format!("{STAGING_PREFIX}not-ours"))).unwrap();
    assert!(restores.staging_leftovers().unwrap().is_empty());
}

#[test]
fn check_against_catches_a_root_added_later() {
    let w = world();
    let restores = w.restores("Restores").unwrap();
    let mut around = w.roots.clone();
    around.add(&w.base, RootKind::UserFolder).unwrap();
    restores.check_against(&around).unwrap();
    let inner = restores.path().join("inner");
    fs::create_dir(&inner).unwrap();
    let mut inside = w.roots.clone();
    inside.add(&inner, RootKind::UserFolder).unwrap();
    assert!(matches!(
        restores.check_against(&inside),
        Err(CloneError::ContainsWatchedRoot { .. })
    ));
}

#[test]
fn a_source_containing_the_restores_folder_is_refused() {
    let w = world();
    let restores = w.restores("Other/Restores").unwrap();
    let dest = restores.fresh_destination("All", "x", None).unwrap();
    assert!(matches!(
        clone_tree(&w.base.join("Other"), dest),
        Err(CloneError::SourceContainsRestores { .. })
    ));
    assert!(names_in(restores.path()).is_empty());
}

/// Review finding 3: `/System/Volumes/Data/…` is a firmlink alias that
/// `canonicalize` keeps, so spelling alone saw no relation.
#[cfg(target_os = "macos")]
#[test]
fn a_firmlink_alias_is_caught_by_identity() {
    let w = world();
    let alias = Path::new("/System/Volumes/Data").join(w.base.strip_prefix("/").unwrap());
    if !alias.is_dir() {
        return; // no firmlinked data volume on this machine
    }
    let r = RestoresDir::new(&alias.join("Music"), &w.roots, &w.data);
    assert!(
        matches!(r, Err(CloneError::ContainsWatchedRoot { .. })),
        "{r:?}"
    );
    // And a source spelled through the alias can't swallow the Restores folder.
    let restores = w.restores("Music/Wit Restores").unwrap();
    let dest = restores.fresh_destination("All", "x", None).unwrap();
    assert!(matches!(
        clone_tree(&alias.join("Music"), dest),
        Err(CloneError::SourceContainsRestores { .. })
    ));
}

/// Review finding 5: a Finder-locked (`uchg`) source.
#[cfg(target_os = "macos")]
#[test]
fn finder_locked_sources_restore_as_unlocked_copies() {
    use std::os::macos::fs::MetadataExt;
    let chflags = |flag: &str, p: &Path| {
        std::process::Command::new("chflags")
            .arg(flag)
            .arg(p)
            .status()
            .unwrap()
    };
    let w = world();
    let restores = w.restores("Restores").unwrap();
    let set = w.base.join("Music/Logic/Locked.als");
    fs::write(&set, b"locked set").unwrap();
    let pd = w
        .base
        .join("Music/Logic/Song.logicx/Alternatives/000/ProjectData");
    assert!(chflags("uchg", &set).success() && chflags("uchg", &pd).success());

    let single = clone_tree(
        &set,
        restores
            .fresh_destination("Locked", "x", Some("als"))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        fs::metadata(&single.path).unwrap().st_flags() & 0x2,
        0,
        "copy is not locked"
    );
    let mut new = restores.begin_restore("Song", "x", Some("logicx")).unwrap();
    new.clone_tree_from(&w.base.join("Music/Logic/Song.logicx"))
        .unwrap();
    new.write_file(Path::new("Alternatives/000/ProjectData"), b"swapped")
        .unwrap();
    drop(new); // cleanup must succeed too
    let leftover = names_in(restores.path())
        .into_iter()
        .filter(|n| n.starts_with(STAGING_PREFIX))
        .count();
    chflags("nouchg", &set);
    chflags("nouchg", &pd);
    assert_eq!(leftover, 0, "no staging leaked");
}

/// Review finding 6: a clone into a name that already exists passes the
/// error up and removes nothing.
#[test]
fn an_existing_destination_is_never_removed_on_clone_failure() {
    let w = world();
    let restores = w.restores("Restores").unwrap();
    let root = restores.pin().unwrap();
    fs::write(restores.path().join("taken"), b"someone else's").unwrap();
    let src = w
        .base
        .join("Music/Logic/Song.logicx/Alternatives/000/ProjectData");
    let meta = fs::symlink_metadata(&src).unwrap();
    let mut ctx = CloneCtx {
        mode: CopyMode::TryReflink,
        remaining: meta.len(),
        space_checked: false,
        space_probe: restores.path().to_path_buf(),
        restores_id: restores.inner.id,
        report: CloneReport::default(),
    };
    assert!(clone_file(&src, &meta, &root, OsStr::new("taken"), &mut ctx).is_err());
    assert_eq!(
        fs::read(restores.path().join("taken")).unwrap(),
        b"someone else's"
    );
    ctx.mode = CopyMode::Copy;
    assert!(clone_file(&src, &meta, &root, OsStr::new("taken"), &mut ctx).is_err());
    assert_eq!(
        fs::read(restores.path().join("taken")).unwrap(),
        b"someone else's"
    );
}

#[test]
fn utc_date_labels() {
    use std::time::Duration;
    assert_eq!(date_label_utc(UNIX_EPOCH), "1970-01-01");
    assert_eq!(
        date_label_utc(UNIX_EPOCH + Duration::from_secs(1_790_640_000)),
        "2026-09-29"
    );
    assert_eq!(
        date_label_utc(UNIX_EPOCH + Duration::from_secs(951_782_400)),
        "2000-02-29"
    );
}
