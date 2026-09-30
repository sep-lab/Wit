//! Directory handles that **pin** a folder, and every file operation a
//! restore performs, done *relative to* such a handle.
//!
//! Why: a path is re-resolved by the OS on every syscall, so a check on a
//! path says nothing about where the next syscall lands — swap a folder for
//! a symlink in between and the write goes elsewhere (measured in the
//! safety review of #55: ~140 staging entries created inside a Logic package
//! in 4 s). A handle names one directory for as long as it is open.
//!
//! - **Unix (macOS, Linux):** directories are opened with
//!   `O_DIRECTORY | O_NOFOLLOW`, children with `openat(…, O_NOFOLLOW)`;
//!   files are created with `openat(O_CREAT | O_EXCL | O_NOFOLLOW)`; renames
//!   use `renameatx_np(RENAME_EXCL)` (macOS) / `renameat2(RENAME_NOREPLACE)`
//!   (Linux) so nothing is ever replaced; clones use `fclonefileat` (macOS)
//!   and `FICLONE` (Linux). All via `rustix`'s safe wrappers — this crate
//!   has no `unsafe`.
//! - **Windows:** std has no `*at` calls, so the folder is opened (as a
//!   directory, never through a reparse point) **without `FILE_SHARE_DELETE`**
//!   and held for the whole operation: Windows then refuses to rename or
//!   delete that folder, so its path keeps naming the same folder while
//!   Wit works inside it. Operations are path-based under that held folder.

use std::ffi::OsStr;
use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};

/// A file's identity: `(st_dev, st_ino)` on Unix, `(volume serial, file
/// index)` on Windows. Two paths name the same file iff their ids match —
/// whatever their spelling (case, normal form, firmlinks, bind mounts).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FileId {
    pub(crate) volume: u64,
    pub(crate) index: u64,
}

impl FileId {
    /// The id of what `path` resolves to (symlinks followed).
    pub fn of_path(path: &Path) -> io::Result<FileId> {
        imp::id_of_path(path)
    }

    /// The id of an open file or directory.
    pub fn of_file(file: &File) -> io::Result<FileId> {
        imp::id_of_file(file)
    }

    pub(crate) fn encode(self) -> String {
        format!("{}:{}", self.volume, self.index)
    }

    pub(crate) fn decode(s: &str) -> Option<FileId> {
        let (v, i) = s.split_once(':')?;
        Some(FileId {
            volume: v.parse().ok()?,
            index: i.parse().ok()?,
        })
    }
}

/// What a directory entry is, without following it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    File,
    Dir,
    Symlink,
    Other,
}

/// How a no-replace rename ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Renamed {
    /// The entry now has its new name, and only that.
    Moved,
    /// Filesystems without an atomic no-replace rename: the new name was
    /// hard-linked (so it exists, complete) but the old name couldn't be
    /// removed. The caller reports the old name as a leftover.
    LinkedLeftover,
}

/// Result of trying a copy-on-write clone.
pub enum Cloned {
    /// `name` now exists and shares blocks with the source.
    Yes,
    /// No clone was made. On Linux the empty destination file was already
    /// created (it is handed back for the byte copy); elsewhere nothing was
    /// created.
    No(Option<File>),
}

/// An open, pinned directory.
#[derive(Debug)]
pub struct Dir {
    file: File,
    path: PathBuf,
}

impl Dir {
    /// Open a real directory at a canonical `path` (its last component must
    /// not be a symlink or reparse point).
    pub fn open(path: &Path) -> io::Result<Dir> {
        imp::open(path)
    }

    /// The path this directory was opened at (for display, errors, and on
    /// Windows the base of path-relative operations).
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn id(&self) -> io::Result<FileId> {
        FileId::of_file(&self.file)
    }

    /// Flush the directory's own metadata (a completed rename) to disk.
    /// Best effort; a no-op where directories can't be synced.
    pub fn sync(&self) {
        if cfg!(unix) {
            let _ = self.file.sync_all();
        }
    }

    /// Remove `name` and, if it is a real directory, everything in it —
    /// never following a symlink (a symlink is removed as a link).
    pub fn remove_tree(&self, name: &OsStr) -> io::Result<()> {
        match self.kind(name)? {
            None => Ok(()),
            Some(EntryKind::Dir) => {
                {
                    let child = self.open_dir(name)?;
                    for entry in child.entries()? {
                        child.remove_tree(&entry)?;
                    }
                } // closed before removal (Windows can't remove an open folder)
                self.remove_empty_dir(name)
            }
            Some(_) => self.remove_file(name),
        }
    }
}

fn check_name(name: &OsStr) -> io::Result<()> {
    let ok = !name.is_empty()
        && name != "."
        && name != ".."
        && !name.to_string_lossy().contains(['/', '\\', '\0']);
    if ok {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not a single plain file name",
        ))
    }
}

#[cfg(unix)]
mod imp {
    use super::{check_name, Cloned, Dir, EntryKind, FileId, Renamed};
    use rustix::fs::{self as rfs, AtFlags, FileType, Mode, OFlags, RenameFlags};
    use rustix::io::Errno;
    use std::ffi::{OsStr, OsString};
    use std::fs::File;
    use std::io;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::MetadataExt;
    use std::path::Path;

    const DIR_FLAGS: OFlags = OFlags::RDONLY
        .union(OFlags::DIRECTORY)
        .union(OFlags::NOFOLLOW)
        .union(OFlags::CLOEXEC);

    pub fn id_of_path(path: &Path) -> io::Result<FileId> {
        let m = std::fs::metadata(path)?;
        Ok(FileId {
            volume: m.dev(),
            index: m.ino(),
        })
    }

    pub fn id_of_file(file: &File) -> io::Result<FileId> {
        let m = file.metadata()?;
        Ok(FileId {
            volume: m.dev(),
            index: m.ino(),
        })
    }

    pub fn open(path: &Path) -> io::Result<Dir> {
        let fd = rfs::open(path, DIR_FLAGS, Mode::empty())?;
        Ok(Dir {
            file: File::from(fd),
            path: path.to_path_buf(),
        })
    }

    fn unsupported(e: Errno) -> bool {
        e == Errno::INVAL || e == Errno::NOSYS || e == Errno::NOTSUP || e == Errno::OPNOTSUPP
    }

    impl Dir {
        pub fn open_dir(&self, name: &OsStr) -> io::Result<Dir> {
            check_name(name)?;
            let fd = rfs::openat(&self.file, name, DIR_FLAGS, Mode::empty())?;
            Ok(Dir {
                file: File::from(fd),
                path: self.path.join(name),
            })
        }

        pub fn mkdir(&self, name: &OsStr) -> io::Result<()> {
            check_name(name)?;
            rfs::mkdirat(&self.file, name, Mode::from_raw_mode(0o755))?;
            Ok(())
        }

        pub fn create_file(&self, name: &OsStr) -> io::Result<File> {
            check_name(name)?;
            let flags =
                OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC;
            let fd = rfs::openat(&self.file, name, flags, Mode::from_raw_mode(0o644))?;
            Ok(File::from(fd))
        }

        pub fn kind(&self, name: &OsStr) -> io::Result<Option<EntryKind>> {
            check_name(name)?;
            match rfs::statat(&self.file, name, AtFlags::SYMLINK_NOFOLLOW) {
                Ok(st) => Ok(Some(match FileType::from_raw_mode(st.st_mode as _) {
                    FileType::RegularFile => EntryKind::File,
                    FileType::Directory => EntryKind::Dir,
                    FileType::Symlink => EntryKind::Symlink,
                    _ => EntryKind::Other,
                })),
                Err(Errno::NOENT) => Ok(None),
                Err(e) => Err(e.into()),
            }
        }

        /// How many hard links the entry `name` has (not followed).
        pub fn links_of(&self, name: &OsStr) -> io::Result<u64> {
            check_name(name)?;
            let st = rfs::statat(&self.file, name, AtFlags::SYMLINK_NOFOLLOW)?;
            #[allow(clippy::unnecessary_cast)]
            Ok(st.st_nlink as u64)
        }

        /// Identity of the entry `name` itself (not followed).
        pub fn id_of(&self, name: &OsStr) -> io::Result<FileId> {
            check_name(name)?;
            let st = rfs::statat(&self.file, name, AtFlags::SYMLINK_NOFOLLOW)?;
            #[allow(clippy::unnecessary_cast)]
            Ok(FileId {
                volume: st.st_dev as u64,
                index: st.st_ino as u64,
            })
        }

        pub fn rename_noreplace(&self, from: &OsStr, to: &OsStr) -> io::Result<Renamed> {
            check_name(from)?;
            check_name(to)?;
            match rfs::renameat_with(&self.file, from, &self.file, to, RenameFlags::NOREPLACE) {
                Ok(()) => Ok(Renamed::Moved),
                Err(Errno::EXIST) | Err(Errno::NOTEMPTY) => {
                    Err(io::ErrorKind::AlreadyExists.into())
                }
                Err(e) if unsupported(e) => self.rename_noreplace_fallback(from, to),
                Err(e) => Err(e.into()),
            }
        }

        /// For filesystems without an atomic no-replace rename.
        fn rename_noreplace_fallback(&self, from: &OsStr, to: &OsStr) -> io::Result<Renamed> {
            if self.kind(from)? == Some(EntryKind::Dir) {
                if self.kind(to)?.is_some() {
                    return Err(io::ErrorKind::AlreadyExists.into());
                }
                // POSIX rename can at most replace an *empty* directory that
                // appeared in between; it fails on a non-empty one.
                rfs::renameat(&self.file, from, &self.file, to)?;
                return Ok(Renamed::Moved);
            }
            match rfs::linkat(&self.file, from, &self.file, to, AtFlags::empty()) {
                Ok(()) => match rfs::unlinkat(&self.file, from, AtFlags::empty()) {
                    Ok(()) => Ok(Renamed::Moved),
                    Err(_) => Ok(Renamed::LinkedLeftover),
                },
                Err(Errno::EXIST) => Err(io::ErrorKind::AlreadyExists.into()),
                Err(e) => Err(e.into()),
            }
        }

        /// Rename within this directory, replacing `to` if it exists. Only
        /// ever used inside a staging folder the current restore created.
        pub fn rename_replace(&self, from: &OsStr, to: &OsStr) -> io::Result<()> {
            check_name(from)?;
            check_name(to)?;
            rfs::renameat(&self.file, from, &self.file, to)?;
            Ok(())
        }

        pub fn remove_file(&self, name: &OsStr) -> io::Result<()> {
            check_name(name)?;
            rfs::unlinkat(&self.file, name, AtFlags::empty())?;
            Ok(())
        }

        pub fn remove_empty_dir(&self, name: &OsStr) -> io::Result<()> {
            check_name(name)?;
            rfs::unlinkat(&self.file, name, AtFlags::REMOVEDIR)?;
            Ok(())
        }

        pub fn entries(&self) -> io::Result<Vec<OsString>> {
            let mut out = Vec::new();
            for entry in rfs::Dir::read_from(&self.file)? {
                let entry = entry?;
                let name = OsStr::from_bytes(entry.file_name().to_bytes());
                if name != "." && name != ".." {
                    out.push(name.to_os_string());
                }
            }
            Ok(out)
        }

        /// Try a copy-on-write clone of `src` as a new entry `name`.
        pub fn clone_file(&self, src: &File, _src_path: &Path, name: &OsStr) -> io::Result<Cloned> {
            check_name(name)?;
            clone_into(self, src, name)
        }
    }

    #[cfg(target_os = "macos")]
    fn clone_into(dir: &Dir, src: &File, name: &OsStr) -> io::Result<Cloned> {
        match rfs::fclonefileat(src, &dir.file, name, rfs::CloneFlags::NOOWNERCOPY) {
            Ok(()) => Ok(Cloned::Yes),
            Err(Errno::EXIST) => Err(io::ErrorKind::AlreadyExists.into()),
            // clonefile is atomic: on failure nothing was created.
            Err(_) => Ok(Cloned::No(None)),
        }
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    fn clone_into(dir: &Dir, src: &File, name: &OsStr) -> io::Result<Cloned> {
        let dst = dir.create_file(name)?;
        match rfs::ioctl_ficlone(&dst, src) {
            Ok(()) => Ok(Cloned::Yes),
            Err(_) => Ok(Cloned::No(Some(dst))),
        }
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "android")))]
    fn clone_into(_dir: &Dir, _src: &File, _name: &OsStr) -> io::Result<Cloned> {
        Ok(Cloned::No(None))
    }
}

#[cfg(windows)]
mod imp {
    use super::{check_name, Cloned, Dir, EntryKind, FileId, Renamed};
    use std::ffi::{OsStr, OsString};
    use std::fs::{self, File, OpenOptions};
    use std::io;
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
    use std::path::Path;

    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    const FILE_SHARE_READ: u32 = 0x1;
    const FILE_SHARE_WRITE: u32 = 0x2;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;

    pub fn id_of_path(path: &Path) -> io::Result<FileId> {
        let handle = winapi_util::Handle::from_path_any(path)?;
        let info = winapi_util::file::information(&handle)?;
        Ok(FileId {
            volume: info.volume_serial_number(),
            index: info.file_index(),
        })
    }

    pub fn id_of_file(file: &File) -> io::Result<FileId> {
        let info = winapi_util::file::information(file)?;
        Ok(FileId {
            volume: info.volume_serial_number(),
            index: info.file_index(),
        })
    }

    /// Open a directory itself (not through a reparse point), sharing read
    /// and write but **not delete**: while this handle is open, Windows
    /// refuses to rename or delete the folder.
    pub fn open(path: &Path) -> io::Result<Dir> {
        let file = OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)?;
        let meta = file.metadata()?;
        if !meta.is_dir() || meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(io::Error::new(
                io::ErrorKind::NotADirectory,
                "not a real directory (a file, symlink or junction)",
            ));
        }
        Ok(Dir {
            file,
            path: path.to_path_buf(),
        })
    }

    fn kind_of(meta: &fs::Metadata) -> EntryKind {
        if meta.file_type().is_symlink()
            || meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
        {
            EntryKind::Symlink
        } else if meta.is_dir() {
            EntryKind::Dir
        } else if meta.is_file() {
            EntryKind::File
        } else {
            EntryKind::Other
        }
    }

    /// Clear the read-only attribute on an entry Wit itself created.
    pub(crate) fn make_writable(path: &Path) {
        if let Ok(meta) = fs::symlink_metadata(path) {
            let mut perms = meta.permissions();
            if perms.readonly() {
                #[allow(clippy::permissions_set_readonly_false)]
                perms.set_readonly(false);
                let _ = fs::set_permissions(path, perms);
            }
        }
    }

    impl Dir {
        pub fn open_dir(&self, name: &OsStr) -> io::Result<Dir> {
            check_name(name)?;
            open(&self.path.join(name))
        }

        pub fn mkdir(&self, name: &OsStr) -> io::Result<()> {
            check_name(name)?;
            fs::create_dir(self.path.join(name))
        }

        pub fn create_file(&self, name: &OsStr) -> io::Result<File> {
            check_name(name)?;
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
                .open(self.path.join(name))
        }

        pub fn kind(&self, name: &OsStr) -> io::Result<Option<EntryKind>> {
            check_name(name)?;
            match fs::symlink_metadata(self.path.join(name)) {
                Ok(m) => Ok(Some(kind_of(&m))),
                Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
                Err(e) => Err(e),
            }
        }

        pub fn id_of(&self, name: &OsStr) -> io::Result<FileId> {
            check_name(name)?;
            id_of_path(&self.path.join(name))
        }

        /// How many hard links the file `name` has
        /// (`BY_HANDLE_FILE_INFORMATION.nNumberOfLinks`).
        pub fn links_of(&self, name: &OsStr) -> io::Result<u64> {
            check_name(name)?;
            let handle = winapi_util::Handle::from_path_any(self.path.join(name))?;
            Ok(winapi_util::file::information(&handle)?.number_of_links())
        }

        pub fn rename_noreplace(&self, from: &OsStr, to: &OsStr) -> io::Result<Renamed> {
            check_name(from)?;
            check_name(to)?;
            let (src, dst) = (self.path.join(from), self.path.join(to));
            if self.kind(from)? != Some(EntryKind::Dir) {
                match fs::hard_link(&src, &dst) {
                    Ok(()) => {
                        return match fs::remove_file(&src) {
                            Ok(()) => Ok(Renamed::Moved),
                            Err(_) => Ok(Renamed::LinkedLeftover),
                        }
                    }
                    Err(e) if e.kind() == io::ErrorKind::AlreadyExists => return Err(e),
                    Err(_) => {} // no hard links here (FAT/exFAT): check-then-rename
                }
            }
            if self.kind(to)?.is_some() {
                return Err(io::ErrorKind::AlreadyExists.into());
            }
            fs::rename(&src, &dst)?;
            Ok(Renamed::Moved)
        }

        pub fn rename_replace(&self, from: &OsStr, to: &OsStr) -> io::Result<()> {
            check_name(from)?;
            check_name(to)?;
            fs::rename(self.path.join(from), self.path.join(to))
        }

        pub fn remove_file(&self, name: &OsStr) -> io::Result<()> {
            check_name(name)?;
            let path = self.path.join(name);
            let first = if self.kind(name)? == Some(EntryKind::Symlink) {
                // A directory symlink/junction is removed with RemoveDirectory.
                fs::remove_file(&path).or_else(|_| fs::remove_dir(&path))
            } else {
                fs::remove_file(&path)
            };
            match first {
                Err(e) if e.kind() == io::ErrorKind::PermissionDenied => {
                    // Clear read-only only on a single-link file: every link
                    // shares one set of attributes, so on a planted hard link
                    // to a read-only project file this would unprotect the
                    // project file itself.
                    if self.links_of(name).ok() == Some(1) {
                        make_writable(&path);
                        fs::remove_file(&path)
                    } else {
                        Err(e)
                    }
                }
                other => other,
            }
        }

        pub fn remove_empty_dir(&self, name: &OsStr) -> io::Result<()> {
            check_name(name)?;
            fs::remove_dir(self.path.join(name))
        }

        pub fn entries(&self) -> io::Result<Vec<OsString>> {
            fs::read_dir(&self.path)?
                .map(|e| e.map(|e| e.file_name()))
                .collect()
        }

        pub fn clone_file(&self, _src: &File, src_path: &Path, name: &OsStr) -> io::Result<Cloned> {
            check_name(name)?;
            let dst = self.path.join(name);
            match reflink_copy::reflink(src_path, &dst) {
                Ok(()) => {
                    if self.links_of(name).ok() == Some(1) {
                        make_writable(&dst);
                    }
                    Ok(Cloned::Yes)
                }
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Err(e),
                // reflink-copy removes its own partial file on failure.
                Err(_) => Ok(Cloned::No(None)),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    #[test]
    fn ids_see_through_spellings() {
        let dir = tempfile::tempdir().unwrap();
        let real = std::fs::canonicalize(dir.path()).unwrap();
        std::fs::create_dir(real.join("a")).unwrap();
        let a = FileId::of_path(&real.join("a")).unwrap();
        assert_eq!(FileId::of_path(&real.join("a").join(".")).unwrap(), a);
        assert_ne!(FileId::of_path(&real).unwrap(), a);
        assert_eq!(FileId::decode(&a.encode()), Some(a));
        assert_eq!(FileId::decode("x"), None);
    }

    #[test]
    fn handle_ops_create_never_replace_and_remove_trees() {
        let dir = tempfile::tempdir().unwrap();
        let real = std::fs::canonicalize(dir.path()).unwrap();
        let d = Dir::open(&real).unwrap();
        d.mkdir(OsStr::new("sub")).unwrap();
        assert!(d.mkdir(OsStr::new("sub")).is_err());
        let sub = d.open_dir(OsStr::new("sub")).unwrap();
        std::io::Write::write_all(&mut sub.create_file(OsStr::new("f")).unwrap(), b"x").unwrap();
        assert_eq!(
            sub.create_file(OsStr::new("f")).unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
        std::io::Write::write_all(&mut sub.create_file(OsStr::new("g")).unwrap(), b"y").unwrap();
        assert_eq!(
            sub.rename_noreplace(OsStr::new("g"), OsStr::new("f"))
                .unwrap_err()
                .kind(),
            io::ErrorKind::AlreadyExists
        );
        assert_eq!(std::fs::read(real.join("sub/f")).unwrap(), b"x");
        assert!(matches!(
            sub.rename_noreplace(OsStr::new("g"), OsStr::new("h"))
                .unwrap(),
            Renamed::Moved
        ));
        assert_eq!(sub.kind(OsStr::new("h")).unwrap(), Some(EntryKind::File));
        assert_eq!(sub.kind(OsStr::new("g")).unwrap(), None);
        let mut names = sub.entries().unwrap();
        names.sort();
        assert_eq!(names, vec![OsString::from("f"), OsString::from("h")]);
        drop(sub);
        d.remove_tree(OsStr::new("sub")).unwrap();
        assert!(!real.join("sub").exists());
        for bad in ["", ".", "..", "a/b", "a\\b"] {
            assert!(d.mkdir(OsStr::new(bad)).is_err(), "{bad:?}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn handles_never_follow_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        let real = std::fs::canonicalize(dir.path()).unwrap();
        std::fs::create_dir_all(real.join("target")).unwrap();
        std::os::unix::fs::symlink(real.join("target"), real.join("link")).unwrap();
        assert!(Dir::open(&real.join("link")).is_err());
        let d = Dir::open(&real).unwrap();
        assert!(d.open_dir(OsStr::new("link")).is_err());
        assert_eq!(
            d.kind(OsStr::new("link")).unwrap(),
            Some(EntryKind::Symlink)
        );
        std::os::unix::fs::symlink(real.join("target/x"), real.join("dangling")).unwrap();
        assert!(d.create_file(OsStr::new("dangling")).is_err());
        assert!(
            !real.join("target/x").exists(),
            "never created through a symlink"
        );
        d.remove_tree(OsStr::new("link")).unwrap();
        assert!(
            real.join("target").is_dir(),
            "a symlink is removed as a link"
        );
    }

    /// Re-review: on Windows a planted hard link was invisible (link count
    /// always 1) and removing it could clear the project file's read-only
    /// attribute.
    #[cfg(windows)]
    #[test]
    fn a_planted_hard_link_is_counted_and_never_made_writable() {
        let dir = tempfile::tempdir().unwrap();
        let real = std::fs::canonicalize(dir.path()).unwrap();
        let project = real.join("project.als");
        std::fs::write(&project, b"project").unwrap();
        let mut perms = std::fs::metadata(&project).unwrap().permissions();
        perms.set_readonly(true);
        std::fs::set_permissions(&project, perms).unwrap();
        std::fs::create_dir(real.join("R")).unwrap();
        std::fs::hard_link(&project, real.join("R/planted")).unwrap();
        let d = Dir::open(&real.join("R")).unwrap();
        assert_eq!(d.links_of(OsStr::new("planted")).unwrap(), 2);
        // Current std may delete a read-only file outright (then only the
        // planted *name* goes); where it refuses, Wit must not fall back to
        // clearing the attribute. Either way the project is untouched.
        let _ = d.remove_file(OsStr::new("planted"));
        assert_eq!(std::fs::read(&project).unwrap(), b"project");
        assert!(
            std::fs::metadata(&project)
                .unwrap()
                .permissions()
                .readonly(),
            "the project file's read-only attribute is untouched"
        );
        drop(d);
        imp::make_writable(&project); // let the temp dir clean up
    }

    #[test]
    fn a_pinned_folder_keeps_naming_the_same_folder() {
        let dir = tempfile::tempdir().unwrap();
        let real = std::fs::canonicalize(dir.path()).unwrap();
        std::fs::create_dir(real.join("R")).unwrap();
        let pinned = Dir::open(&real.join("R")).unwrap();
        let id = pinned.id().unwrap();
        let moved = std::fs::rename(real.join("R"), real.join("R2"));
        if cfg!(windows) {
            assert!(
                moved.is_err(),
                "Windows must refuse to move a pinned folder"
            );
        } else {
            moved.unwrap();
            std::fs::create_dir(real.join("R")).unwrap();
            // The handle still names the original folder, wherever it went.
            pinned.mkdir(OsStr::new("x")).unwrap();
            assert!(real.join("R2/x").is_dir());
            assert!(!real.join("R/x").exists());
            assert_eq!(pinned.id().unwrap(), id);
        }
    }
}
