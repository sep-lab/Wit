//! SQLite registry: which projects and versions Wit has indexed, plus
//! local-only counters. **No telemetry, zero network code** — this is a
//! `rusqlite` connection to a file on disk, nothing else.
//!
//! Semantic verdict labeling (bytes identical / routine save / "something
//! changed Wit can't see yet") is deliberately **not** computed here — it
//! needs a per-DAW differ (`wit-diff`, `wit-logic`) wired in per version,
//! which is UI-facing work for a later milestone. What this registry
//! guarantees is the M3 exit criteria's actual bar: every byte-different
//! save gets an ingested version row (archive-before-recycle), and
//! re-scanning is idempotent.

use crate::store::Hash;
use rusqlite::Connection;
use std::fmt;
use std::path::Path;

#[derive(Debug)]
pub enum RegistryError {
    Sqlite(String),
}

impl fmt::Display for RegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RegistryError::Sqlite(msg) => write!(f, "registry error: {msg}"),
        }
    }
}

impl std::error::Error for RegistryError {}

impl From<rusqlite::Error> for RegistryError {
    fn from(e: rusqlite::Error) -> Self {
        RegistryError::Sqlite(e.to_string())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProjectRow {
    pub id: i64,
    pub name: String,
    pub bundle_path: String,
    pub kind: String,
    pub version_count: usize,
}

pub struct Registry {
    conn: Connection,
}

impl Registry {
    /// Open (creating and migrating if needed) a registry at `db_path`.
    /// Tests always pass a `tempfile::tempdir()` path — never the real
    /// `~/Library/Application Support/Wit` (M3 test-hygiene rule).
    pub fn open(db_path: impl AsRef<Path>) -> Result<Self, RegistryError> {
        let conn = Connection::open(db_path)?;
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS projects (
                id INTEGER PRIMARY KEY,
                name TEXT NOT NULL,
                bundle_path TEXT NOT NULL UNIQUE,
                kind TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS versions (
                id INTEGER PRIMARY KEY,
                project_id INTEGER NOT NULL REFERENCES projects(id),
                slot_key TEXT NOT NULL,
                content_hash TEXT NOT NULL,
                ingested_at INTEGER NOT NULL,
                UNIQUE(project_id, slot_key, content_hash)
            );
            CREATE TABLE IF NOT EXISTS counters (
                event TEXT PRIMARY KEY,
                count INTEGER NOT NULL DEFAULT 0
            );
            ",
        )?;
        Ok(Registry { conn })
    }

    /// Insert the project if it isn't already known (keyed by
    /// `bundle_path`), and return its id either way.
    pub fn upsert_project(
        &self,
        name: &str,
        bundle_path: &str,
        kind: &str,
    ) -> Result<i64, RegistryError> {
        self.conn.execute(
            "INSERT INTO projects (name, bundle_path, kind) VALUES (?1, ?2, ?3)
             ON CONFLICT(bundle_path) DO UPDATE SET name = excluded.name",
            (name, bundle_path, kind),
        )?;
        let id = self.conn.query_row(
            "SELECT id FROM projects WHERE bundle_path = ?1",
            [bundle_path],
            |row| row.get(0),
        )?;
        Ok(id)
    }

    /// Record a version for `project_id` at `slot_key` (a stable per-project
    /// identifier for *where* this save lives — e.g. `"000/backup/03"` or
    /// `"lineage-save/2"` — not a filesystem path, since backup slot
    /// numbers and lineage positions are what stays stable across a rescan,
    /// not necessarily the exact file each one happens to be at). Returns
    /// `true` if this was a genuinely new row (the content hash at this
    /// slot hasn't been seen before — **archive-before-recycle: this fires
    /// for every byte-different save, never gated on a semantic verdict**),
    /// `false` if it was already known (a rescan no-op — what makes
    /// rescanning idempotent).
    pub fn record_version_if_new(
        &self,
        project_id: i64,
        slot_key: &str,
        content_hash: Hash,
        ingested_at: i64,
    ) -> Result<bool, RegistryError> {
        let changed = self.conn.execute(
            "INSERT OR IGNORE INTO versions (project_id, slot_key, content_hash, ingested_at)
             VALUES (?1, ?2, ?3, ?4)",
            (project_id, slot_key, content_hash.to_hex(), ingested_at),
        )?;
        Ok(changed > 0)
    }

    pub fn version_count(&self, project_id: i64) -> Result<usize, RegistryError> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM versions WHERE project_id = ?1",
            [project_id],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    pub fn list_projects(&self) -> Result<Vec<ProjectRow>, RegistryError> {
        let mut stmt = self.conn.prepare(
            "SELECT p.id, p.name, p.bundle_path, p.kind,
                    (SELECT COUNT(*) FROM versions v WHERE v.project_id = p.id) AS version_count
             FROM projects p ORDER BY p.name",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok(ProjectRow {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    bundle_path: row.get(2)?,
                    kind: row.get(3)?,
                    version_count: row.get::<_, i64>(4)? as usize,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Local-only, opt-in-surfaced counters (compares run, shares created,
    /// ...) — no telemetry, no network code anywhere in this crate.
    pub fn increment_counter(&self, event: &str) -> Result<(), RegistryError> {
        self.conn.execute(
            "INSERT INTO counters (event, count) VALUES (?1, 1)
             ON CONFLICT(event) DO UPDATE SET count = count + 1",
            [event],
        )?;
        Ok(())
    }

    pub fn counter(&self, event: &str) -> Result<i64, RegistryError> {
        let count = self
            .conn
            .query_row(
                "SELECT count FROM counters WHERE event = ?1",
                [event],
                |row| row.get(0),
            )
            .unwrap_or(0);
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open_test_registry() -> (tempfile::TempDir, Registry) {
        let dir = tempfile::tempdir().unwrap();
        let reg = Registry::open(dir.path().join("wit.db")).unwrap();
        (dir, reg)
    }

    #[test]
    fn upsert_project_is_idempotent_and_returns_the_same_id() {
        let (_dir, reg) = open_test_registry();
        let id1 = reg
            .upsert_project("Song", "/tmp/Song.logicx", "logic")
            .unwrap();
        let id2 = reg
            .upsert_project("Song", "/tmp/Song.logicx", "logic")
            .unwrap();
        assert_eq!(id1, id2);
    }

    #[test]
    fn record_version_if_new_returns_true_once_then_false() {
        let (_dir, reg) = open_test_registry();
        let pid = reg
            .upsert_project("Song", "/tmp/Song.logicx", "logic")
            .unwrap();
        let hash = Hash::of(b"version 1 bytes");
        assert!(reg
            .record_version_if_new(pid, "000/current", hash, 1000)
            .unwrap());
        // Re-scanning the same bytes at the same slot: idempotent no-op.
        assert!(!reg
            .record_version_if_new(pid, "000/current", hash, 2000)
            .unwrap());
        assert_eq!(reg.version_count(pid).unwrap(), 1);
    }

    #[test]
    fn a_byte_different_save_at_the_same_slot_is_always_a_new_version() {
        // Archive-before-recycle: never gated on a semantic verdict.
        let (_dir, reg) = open_test_registry();
        let pid = reg
            .upsert_project("Song", "/tmp/Song.logicx", "logic")
            .unwrap();
        let h1 = Hash::of(b"save A");
        let h2 = Hash::of(b"save B - only a fader moved, semantically invisible to wit-logic");
        assert!(reg
            .record_version_if_new(pid, "000/current", h1, 1000)
            .unwrap());
        assert!(reg
            .record_version_if_new(pid, "000/current", h2, 2000)
            .unwrap());
        assert_eq!(reg.version_count(pid).unwrap(), 2);
    }

    #[test]
    fn list_projects_reports_correct_version_counts() {
        let (_dir, reg) = open_test_registry();
        let pid = reg
            .upsert_project("Song", "/tmp/Song.logicx", "logic")
            .unwrap();
        reg.record_version_if_new(pid, "a", Hash::of(b"1"), 1)
            .unwrap();
        reg.record_version_if_new(pid, "b", Hash::of(b"2"), 2)
            .unwrap();
        let rows = reg.list_projects().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].version_count, 2);
    }

    #[test]
    fn counters_increment_and_persist_locally_only() {
        let (_dir, reg) = open_test_registry();
        assert_eq!(reg.counter("compares_opened").unwrap(), 0);
        reg.increment_counter("compares_opened").unwrap();
        reg.increment_counter("compares_opened").unwrap();
        assert_eq!(reg.counter("compares_opened").unwrap(), 2);
    }
}
