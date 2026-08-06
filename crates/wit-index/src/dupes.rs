//! Duplicate-audio report: byte-for-byte identical audio files under a
//! directory tree, grouped by BLAKE3 content hash.
//!
//! Read-only — this scans and hashes; it never touches, moves, or deletes
//! anything (PLAN.md: "No delete button exists. 'Wit never deletes — this
//! is just a map.'").

use crate::store::Hash;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const AUDIO_EXTENSIONS: &[&str] = &["wav", "aif", "aiff", "caf", "flac", "mp3", "m4a", "aac"];

#[derive(Debug, Clone, PartialEq)]
pub struct DuplicateGroup {
    pub hash: Hash,
    pub size_bytes: u64,
    /// Every file sharing this hash. Always ≥ 2 — a group of 1 isn't a
    /// duplicate and never appears in a [`DuplicateReport`].
    pub paths: Vec<PathBuf>,
}

impl DuplicateGroup {
    /// Bytes that would be freed by keeping one copy and removing the
    /// rest — `(count - 1) * size`. Wit never actually deletes anything;
    /// this is purely the number the report shows.
    pub fn wasted_bytes(&self) -> u64 {
        (self.paths.len() as u64 - 1) * self.size_bytes
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct DuplicateReport {
    pub groups: Vec<DuplicateGroup>,
    pub total_audio_bytes: u64,
    pub scanned_file_count: usize,
}

impl DuplicateReport {
    pub fn total_wasted_bytes(&self) -> u64 {
        self.groups.iter().map(|g| g.wasted_bytes()).sum()
    }

    /// The duplicate percentage the README's "found 5.4 GB of duplicate
    /// audio (24% of your library)" line means: wasted / total.
    pub fn duplicate_percent(&self) -> f64 {
        if self.total_audio_bytes == 0 {
            0.0
        } else {
            self.total_wasted_bytes() as f64 / self.total_audio_bytes as f64 * 100.0
        }
    }
}

/// Scan `root` for audio files (by extension) and group byte-identical
/// ones by BLAKE3 hash. A file that fails to read (permissions, a broken
/// symlink) is skipped, not a hard error — one bad file must not blank
/// the whole report.
pub fn duplicate_report(root: &Path) -> DuplicateReport {
    let mut by_hash: BTreeMap<Hash, (u64, Vec<PathBuf>)> = BTreeMap::new();
    let mut total_audio_bytes = 0u64;
    let mut scanned = 0usize;

    walk_audio_files(root, &mut |path| {
        let Ok(bytes) = std::fs::read(path) else {
            return;
        };
        let size = bytes.len() as u64;
        let hash = Hash::of(&bytes);
        total_audio_bytes += size;
        scanned += 1;
        let entry = by_hash.entry(hash).or_insert_with(|| (size, Vec::new()));
        entry.1.push(path.to_path_buf());
    });

    let groups = by_hash
        .into_iter()
        .filter(|(_, (_, paths))| paths.len() >= 2)
        .map(|(hash, (size_bytes, paths))| DuplicateGroup {
            hash,
            size_bytes,
            paths,
        })
        .collect();

    DuplicateReport {
        groups,
        total_audio_bytes,
        scanned_file_count: scanned,
    }
}

fn walk_audio_files(dir: &Path, visit: &mut impl FnMut(&Path)) {
    fn walk_inner(dir: &Path, depth: usize, visit: &mut impl FnMut(&Path)) {
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
                walk_inner(&path, depth + 1, visit);
            } else if path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| AUDIO_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
                .unwrap_or(false)
            {
                visit(&path);
            }
        }
    }
    walk_inner(dir, 0, visit);
}

/// Assert that `text` never leaks an absolute home-directory path — the
/// privacy report's machine-enforced guarantee (M3 issue: "regex-tested:
/// no `/Users/` ever in output"). Returns the offending substring on
/// failure so a test failure is actionable, not just "false".
pub fn assert_no_home_paths(text: &str) -> Result<(), String> {
    for marker in ["/Users/", "/home/", "C:\\Users\\"] {
        if let Some(idx) = text.find(marker) {
            let end = text[idx..]
                .find('\n')
                .map(|n| idx + n)
                .unwrap_or(text.len());
            return Err(format!(
                "found a home-directory path in report output: {:?}",
                &text[idx..end]
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, bytes: &[u8]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    #[test]
    fn identical_audio_files_form_a_duplicate_group() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join("a.wav"), b"same audio bytes");
        write(&dir.path().join("subdir/b.wav"), b"same audio bytes");
        write(&dir.path().join("c.wav"), b"different audio bytes");

        let report = duplicate_report(dir.path());
        assert_eq!(report.scanned_file_count, 3);
        assert_eq!(report.groups.len(), 1);
        assert_eq!(report.groups[0].paths.len(), 2);
        assert_eq!(
            report.groups[0].wasted_bytes(),
            b"same audio bytes".len() as u64
        );
    }

    #[test]
    fn non_audio_files_are_never_scanned() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join("notes.txt"), b"not audio");
        write(&dir.path().join("ProjectData"), b"not audio either");
        let report = duplicate_report(dir.path());
        assert_eq!(report.scanned_file_count, 0);
    }

    #[test]
    fn duplicate_percent_matches_the_readme_shape() {
        let dir = tempfile::tempdir().unwrap();
        // 3 copies of a 100-byte file: 300 bytes total, 200 wasted (2 of 3 copies), 66.67%.
        for i in 0..3 {
            write(&dir.path().join(format!("copy{i}.wav")), &[7u8; 100]);
        }
        let report = duplicate_report(dir.path());
        assert_eq!(report.total_audio_bytes, 300);
        assert_eq!(report.total_wasted_bytes(), 200);
        assert!((report.duplicate_percent() - 66.666).abs() < 0.01);
    }

    #[test]
    fn assert_no_home_paths_catches_a_leak() {
        assert!(assert_no_home_paths("clean report, no paths here").is_ok());
        assert!(assert_no_home_paths("found it at /Users/sepehr/Music/Song.logicx").is_err());
    }
}
