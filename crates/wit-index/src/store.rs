//! A content-addressed blob store: BLAKE3 hash of the *uncompressed*
//! content addresses a zstd-compressed file on disk.
//!
//! **`ingest_bytes` never takes a project path — only bytes.** This is the
//! architectural half of "`wit-index` is the only crate that writes, and no
//! write API takes a project path" (M3 tracking issue): a caller physically
//! cannot ingest-by-path, because the method doesn't accept one. Reading the
//! original project file and deciding what to ingest is the caller's job
//! (`ingest.rs`); this module only ever sees bytes already in memory.

use std::fmt;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Hash(pub [u8; 32]);

impl Hash {
    pub fn of(bytes: &[u8]) -> Hash {
        Hash(*blake3::hash(bytes).as_bytes())
    }

    pub fn to_hex(self) -> String {
        self.0.iter().map(|b| format!("{b:02x}")).collect()
    }
}

impl fmt::Display for Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_hex())
    }
}

#[derive(Debug)]
pub enum StoreError {
    Io(String),
    Compression(String),
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StoreError::Io(msg) => write!(f, "store I/O error: {msg}"),
            StoreError::Compression(msg) => write!(f, "store compression error: {msg}"),
        }
    }
}

impl std::error::Error for StoreError {}

/// A BLAKE3+zstd content-addressed store rooted at a directory. Layout:
/// `<root>/objects/<first 2 hex chars>/<full hex hash>.zst` — the
/// two-level split keeps any one directory from holding tens of thousands
/// of entries on a large library.
pub struct Store {
    root: PathBuf,
}

impl Store {
    /// Open (creating if needed) a store rooted at `root`. Tests always
    /// pass a `tempfile::tempdir()` path — never the real
    /// `~/Library/Application Support/Wit` (M3 test-hygiene rule).
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, StoreError> {
        let root = root.into();
        std::fs::create_dir_all(root.join("objects")).map_err(|e| StoreError::Io(e.to_string()))?;
        Ok(Store { root })
    }

    fn object_path(&self, hash: Hash) -> PathBuf {
        let hex = hash.to_hex();
        self.root
            .join("objects")
            .join(&hex[0..2])
            .join(format!("{hex}.zst"))
    }

    /// Store `bytes`, addressed by the BLAKE3 hash of their *uncompressed*
    /// content. Idempotent: ingesting the same bytes twice is a no-op the
    /// second time (same hash, file already exists) — this is what makes
    /// re-scan idempotent at the store layer.
    pub fn ingest_bytes(&self, bytes: &[u8]) -> Result<Hash, StoreError> {
        let hash = Hash::of(bytes);
        let path = self.object_path(hash);
        if path.exists() {
            return Ok(hash);
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| StoreError::Io(e.to_string()))?;
        }
        let compressed = zstd::stream::encode_all(bytes, 0)
            .map_err(|e| StoreError::Compression(e.to_string()))?;
        // Write to a temp file then rename — concurrent scans (or a crash
        // mid-write) must never leave a partially-written, hash-addressed
        // object that would silently corrupt every future read of it.
        let tmp_path = path.with_extension("zst.tmp");
        {
            let mut f =
                std::fs::File::create(&tmp_path).map_err(|e| StoreError::Io(e.to_string()))?;
            f.write_all(&compressed)
                .map_err(|e| StoreError::Io(e.to_string()))?;
        }
        std::fs::rename(&tmp_path, &path).map_err(|e| StoreError::Io(e.to_string()))?;
        Ok(hash)
    }

    /// Read back the original (decompressed) bytes for `hash`.
    pub fn read(&self, hash: Hash) -> Result<Vec<u8>, StoreError> {
        let path = self.object_path(hash);
        let compressed = std::fs::read(&path).map_err(|e| StoreError::Io(e.to_string()))?;
        let mut out = Vec::new();
        zstd::stream::read::Decoder::new(compressed.as_slice())
            .map_err(|e| StoreError::Compression(e.to_string()))?
            .read_to_end(&mut out)
            .map_err(|e| StoreError::Io(e.to_string()))?;
        Ok(out)
    }

    pub fn contains(&self, hash: Hash) -> bool {
        self.object_path(hash).exists()
    }

    /// Total bytes actually on disk in the store (compressed) — used by
    /// diagnostics, never by the verdict logic anywhere else in the crate.
    pub fn disk_usage(&self) -> u64 {
        fn walk(dir: &Path) -> u64 {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return 0;
            };
            entries
                .filter_map(|e| e.ok())
                .map(|e| {
                    let p = e.path();
                    if p.is_dir() {
                        walk(&p)
                    } else {
                        e.metadata().map(|m| m.len()).unwrap_or(0)
                    }
                })
                .sum()
        }
        walk(&self.root.join("objects"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ingest_and_read_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let hash = store.ingest_bytes(b"hello world").unwrap();
        assert_eq!(store.read(hash).unwrap(), b"hello world");
    }

    #[test]
    fn identical_bytes_hash_the_same_and_ingest_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let h1 = store.ingest_bytes(b"same content").unwrap();
        let h2 = store.ingest_bytes(b"same content").unwrap();
        assert_eq!(h1, h2);
        assert!(store.contains(h1));
    }

    #[test]
    fn different_bytes_hash_differently() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let h1 = store.ingest_bytes(b"content A").unwrap();
        let h2 = store.ingest_bytes(b"content B").unwrap();
        assert_ne!(h1, h2);
    }

    #[test]
    fn contains_is_false_for_unstored_hash() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        assert!(!store.contains(Hash::of(b"never stored")));
    }

    #[test]
    fn store_compresses_highly_repetitive_content() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let bytes = vec![0u8; 100_000];
        store.ingest_bytes(&bytes).unwrap();
        assert!(
            store.disk_usage() < 1000,
            "100 KB of zeros should compress to well under 1 KB"
        );
    }
}
