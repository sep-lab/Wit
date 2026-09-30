//! The staging journal: a small append-only log **in Wit's own data
//! folder** of every staging entry a restore creates in a Restores folder.
//!
//! A crash mid-restore leaves a hidden `.wit-staging-…` entry behind. The
//! law says a restore never deletes anything that existed before it began,
//! so no restore ever cleans such leftovers up on its own. The journal lets
//! a later run *prove* an entry is Wit's own: only an entry that is
//! journaled as created, not journaled as finished, and still has the same
//! name **and file identity** in the same Restores folder may be removed —
//! and only through an explicit call ([`super::RestoresDir::remove_staging_leftover`]),
//! so the app can show the user what it found first.
//!
//! Format, one record per line (staging names are ASCII by construction):
//! `C <restores-id> <name> <entry-id> <unix-secs>` when created,
//! `F <restores-id> <name>` when committed, cleaned up or found gone.
//!
//! **Liveness.** A journaled, unfinished entry may belong to a restore that
//! is still running — in this process through another `RestoresDir`
//! handle, or in another process. So every restore also takes an exclusive
//! advisory lock (`flock` / `LockFileEx`, via std's `File::try_lock`) on
//! `restore-locks/<name>.lock` in the same data folder *before* creating
//! its staging entry, and holds it until it ends; the OS releases it when
//! the process exits, however it exits. An entry is a leftover only if its
//! lock file exists **and** its lock can be taken now. A missing lock file
//! means liveness can't be proven, so the entry is never listed. The lock
//! file is deleted only when the entry is journaled as finished, so a
//! restore whose own cleanup failed still leaves a provable leftover.

use super::fsops::FileId;
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const JOURNAL_FILE: &str = "restore-staging.journal";
pub const LOCKS_DIR: &str = "restore-locks";

#[derive(Debug)]
pub struct Journal {
    path: PathBuf,
    locks: PathBuf,
    lock: Mutex<()>,
}

/// Held for the whole life of one restore in progress. Dropping it (or the
/// process ending) releases the lock; the lock *file* is removed only when
/// the entry is journaled as finished.
#[derive(Debug)]
pub struct StagingLock {
    _file: File,
}

fn try_lock_file(file: &File) -> io::Result<()> {
    file.try_lock().map_err(|e| match e {
        TryLockError::WouldBlock => {
            io::Error::new(io::ErrorKind::WouldBlock, "a restore holds this lock")
        }
        TryLockError::Error(e) => e,
    })
}

/// A journaled staging entry that was never finished.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pending {
    pub name: String,
    pub entry: FileId,
    pub created: SystemTime,
}

impl Journal {
    /// Open (creating Wit's data folder and the journal if needed).
    pub fn open(data_dir: &Path) -> io::Result<Journal> {
        fs::create_dir_all(data_dir)?;
        let path = data_dir.join(JOURNAL_FILE);
        OpenOptions::new().create(true).append(true).open(&path)?;
        let locks = data_dir.join(LOCKS_DIR);
        fs::create_dir_all(&locks)?;
        Ok(Journal {
            path,
            locks,
            lock: Mutex::new(()),
        })
    }

    fn lock_path(&self, name: &str) -> PathBuf {
        self.locks.join(format!("{name}.lock"))
    }

    /// Take the liveness lock for a staging entry about to be created.
    pub fn lock_new(&self, name: &str) -> io::Result<StagingLock> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(self.lock_path(name))?;
        try_lock_file(&file)?;
        Ok(StagingLock { _file: file })
    }

    /// Take the liveness lock of an existing entry — succeeds only if no
    /// live restore holds it (the entry is a crash leftover). A missing
    /// lock file is an error: liveness can't be proven.
    pub fn lock_existing(&self, name: &str) -> io::Result<StagingLock> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(self.lock_path(name))?;
        try_lock_file(&file)?;
        Ok(StagingLock { _file: file })
    }

    fn append(&self, line: &str) -> io::Result<()> {
        let _guard = self.lock.lock().unwrap_or_else(|p| p.into_inner());
        let mut f = OpenOptions::new().append(true).open(&self.path)?;
        f.write_all(line.as_bytes())?;
        f.sync_data()
    }

    pub fn created(&self, restores: FileId, name: &str, entry: FileId) -> io::Result<()> {
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        self.append(&format!(
            "C\t{}\t{name}\t{}\t{secs}\n",
            restores.encode(),
            entry.encode()
        ))
    }

    /// Record that `name` is done with (committed, cleaned up, or gone),
    /// and delete its lock file.
    pub fn finished(&self, restores: FileId, name: &str) -> io::Result<()> {
        self.append(&format!("F\t{}\t{name}\n", restores.encode()))?;
        let _ = fs::remove_file(self.lock_path(name));
        Ok(())
    }

    /// Unfinished entries for the Restores folder `restores`.
    pub fn pending(&self, restores: FileId) -> io::Result<Vec<Pending>> {
        let text = {
            let _guard = self.lock.lock().unwrap_or_else(|p| p.into_inner());
            fs::read_to_string(&self.path)?
        };
        let mut open: BTreeMap<String, Pending> = BTreeMap::new();
        for line in text.lines() {
            let fields: Vec<&str> = line.split('\t').collect();
            match fields.as_slice() {
                ["C", r, name, entry, secs] if FileId::decode(r) == Some(restores) => {
                    if let (Some(entry), Ok(secs)) = (FileId::decode(entry), secs.parse::<u64>()) {
                        open.insert(
                            (*name).to_string(),
                            Pending {
                                name: (*name).to_string(),
                                entry,
                                created: UNIX_EPOCH + Duration::from_secs(secs),
                            },
                        );
                    }
                }
                ["F", r, name] if FileId::decode(r) == Some(restores) => {
                    open.remove(*name);
                }
                _ => {} // another Restores folder, or a torn/foreign line
            }
        }
        Ok(open.into_values().collect())
    }

    /// Truncate the journal when *nothing* in it is unfinished, for any
    /// Restores folder — so it never grows without bound.
    pub fn compact_if_idle(&self) -> io::Result<()> {
        let _guard = self.lock.lock().unwrap_or_else(|p| p.into_inner());
        let text = fs::read_to_string(&self.path)?;
        let mut open: BTreeMap<(String, String), ()> = BTreeMap::new();
        for line in text.lines() {
            let fields: Vec<&str> = line.split('\t').collect();
            match fields.as_slice() {
                ["C", r, name, ..] => {
                    open.insert(((*r).to_string(), (*name).to_string()), ());
                }
                ["F", r, name] => {
                    open.remove(&((*r).to_string(), (*name).to_string()));
                }
                _ => {}
            }
        }
        if open.is_empty() && !text.is_empty() {
            OpenOptions::new()
                .write(true)
                .truncate(true)
                .open(&self.path)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(v: u64, i: u64) -> FileId {
        FileId {
            volume: v,
            index: i,
        }
    }

    #[test]
    fn pending_is_created_minus_finished_per_restores_folder() {
        let dir = tempfile::tempdir().unwrap();
        let j = Journal::open(&dir.path().join("data")).unwrap();
        j.created(id(1, 1), ".wit-staging-a", id(1, 10)).unwrap();
        j.created(id(1, 1), ".wit-staging-b", id(1, 11)).unwrap();
        j.created(id(2, 2), ".wit-staging-c", id(2, 12)).unwrap();
        j.finished(id(1, 1), ".wit-staging-a").unwrap();
        let p = j.pending(id(1, 1)).unwrap();
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].name, ".wit-staging-b");
        assert_eq!(p[0].entry, id(1, 11));
        j.compact_if_idle().unwrap();
        assert_eq!(
            j.pending(id(2, 2)).unwrap().len(),
            1,
            "not idle: nothing truncated"
        );
        j.finished(id(1, 1), ".wit-staging-b").unwrap();
        j.finished(id(2, 2), ".wit-staging-c").unwrap();
        j.compact_if_idle().unwrap();
        assert_eq!(
            fs::read_to_string(dir.path().join("data").join(JOURNAL_FILE)).unwrap(),
            ""
        );
    }

    #[test]
    fn torn_and_foreign_lines_are_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let j = Journal::open(dir.path()).unwrap();
        fs::write(
            dir.path().join(JOURNAL_FILE),
            "garbage\nC\t1:1\t.wit-staging-x\tnot-an-id\t5\nC\t1:1\t.wit-sta",
        )
        .unwrap();
        assert!(j.pending(id(1, 1)).unwrap().is_empty());
    }
}
