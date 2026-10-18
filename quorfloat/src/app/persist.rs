//! Atomic writes for the shell's small local state files.
//!
//! The preferences, pinned pair and window position are tiny JSON documents, but they are
//! also the only state that survives a restart. A direct `fs::write` can leave an empty or
//! truncated file when the process is interrupted between create and write; the next start
//! then treats that as "the user chose nothing" and silently loses a setting.
//!
//! The rule here is the usual one: write a complete temporary file in the **same directory**
//! (so `rename` stays on one filesystem), flush it, then replace the target atomically.
//! `fs::rename` replaces an existing destination on both Unix (`rename(2)`) and Windows
//! (`MoveFileExW` with `MOVEFILE_REPLACE_EXISTING`), so the target is either the complete old
//! file or the complete new one — never a partial write.
//!
//! Failure is **returned**, not swallowed: callers keep the in-memory state and write a
//! marker. A failed save must not make the panel unusable.

use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Distinguishes temporary names created by one process; the pid distinguishes processes.
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Write `bytes` to `path` atomically.
///
/// The temporary file is created next to the target, so the final rename is same-filesystem.
/// On success no temporary file remains. On failure the temporary file is removed and the
/// previous target (if any) is left untouched.
///
/// @param path - the file to replace or create.
/// @param bytes - the complete new contents.
/// @returns `Ok(())` when the target now contains `bytes`; an error with the path in its
///   message otherwise.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    write_atomic_with(path, bytes, |from, to| fs::rename(from, to))
}

/// The body of [`write_atomic`], with the replace step injectable for tests.
///
/// Production always passes [`fs::rename`]. The test seam exists so a test can force the
/// replace to fail after the complete temporary file and the old target both exist — the one
/// state that is otherwise hard to create portably without permissions or platform tricks.
///
/// @param path - the file to replace or create.
/// @param bytes - the complete new contents.
/// @param replace - the operation that installs the temporary file at the target.
/// @returns `Ok(())` when the target now contains `bytes`; an error with the path in its
///   message otherwise.
fn write_atomic_with(
    path: &Path,
    bytes: &[u8],
    replace: impl FnOnce(&Path, &Path) -> io::Result<()>,
) -> io::Result<()> {
    let target = resolve_target(path);
    let parent = target
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|error| with_path(&target, error))?;

    let (temporary, file) = create_temporary(parent, &target)?;
    let result = write_and_replace(&temporary, &target, bytes, file, replace);
    if result.is_err() {
        // The target is untouched; the temp file is the only thing to clean up.
        let _ = fs::remove_file(&temporary);
    }
    result.map_err(|error| with_path(&target, error))
}

/// Remove `path` if it exists.
///
/// A missing file is success: "the user chose nothing" is represented by absence, and
/// removing it twice is not an error.
///
/// @param path - the file to remove.
/// @returns `Ok(())` when the file is gone; an error with the path in its message otherwise.
pub fn remove_if_present(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(with_path(path, error)),
    }
}

/// Whether a save target is an existing symlink, and if so where it really points.
///
/// The old `fs::write` followed a symlink and changed its target. Replacing `path` directly
/// would instead replace the link with a regular file, silently breaking a dotfile setup.
/// Resolving an existing link preserves the old behaviour; a dangling link falls back to the
/// path itself, where there is no target to preserve.
fn resolve_target(path: &Path) -> PathBuf {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
        }
        _ => path.to_path_buf(),
    }
}

/// Create a fresh, uniquely named temporary file next to `target`.
///
/// `create_new(true)` is what makes the name safe: two writers — or two tests — cannot both
/// create the same temporary. A collision retries with the next counter rather than truncating
/// the other writer's file.
fn create_temporary(parent: &Path, target: &Path) -> io::Result<(PathBuf, File)> {
    let name = target.file_name().unwrap_or_default();
    for _ in 0..16 {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let mut temporary_name = OsString::from(".");
        temporary_name.push(name);
        temporary_name.push(format!(".{}.{}.tmp", std::process::id(), sequence));
        let temporary = parent.join(temporary_name);
        match OpenOptions::new().write(true).create_new(true).open(&temporary) {
            Ok(file) => return Ok((temporary, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(with_path(&temporary, error)),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        format!("{}: could not create a unique temporary file", target.display()),
    ))
}

/// Write, flush and replace, consuming the temporary file handle before the rename.
///
/// Dropping the handle before `rename` matters on Windows: an open handle can keep the file
/// from being moved or replaced.
fn write_and_replace(
    temporary: &Path,
    target: &Path,
    bytes: &[u8],
    mut file: File,
    replace: impl FnOnce(&Path, &Path) -> io::Result<()>,
) -> io::Result<()> {
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    // Preserve the old file's Unix mode (a user who made the state file private should not get
    // a world-readable replacement). Windows has no equivalent mode; its ACL follows the
    // directory, and a failure to inherit anything meaningful is not worth failing the save.
    #[cfg(unix)]
    if let Ok(metadata) = fs::metadata(target) {
        let _ = fs::set_permissions(temporary, metadata.permissions());
    }
    replace(temporary, target)
}

/// Add the path to an I/O error so a marker line says which save failed.
fn with_path(path: &Path, error: io::Error) -> io::Error {
    io::Error::new(error.kind(), format!("{}: {error}", path.display()))
}

#[cfg(test)]
pub(crate) mod test_support {
    //! A tiny temporary-directory guard for the state-file tests.

    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);

    /// A directory removed when the test drops it; no external test dependency.
    pub(crate) struct TempDir(PathBuf);

    impl TempDir {
        /// Create a uniquely named directory under the platform's temporary root.
        pub(crate) fn new() -> Self {
            let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "dsh-quorfloat-state-{}-{sequence}",
                std::process::id()
            ));
            std::fs::create_dir_all(&path).expect("create test temporary directory");
            Self(path)
        }

        /// The directory's path.
        pub(crate) fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::TempDir;
    use super::*;

    fn temporary_entries(directory: &Path) -> Vec<String> {
        fs::read_dir(directory)
            .expect("read test directory")
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".tmp"))
            .collect()
    }

    #[test]
    fn write_atomic_replaces_an_existing_file() {
        let directory = TempDir::new();
        let target = directory.path().join("state.json");
        fs::write(&target, "old").expect("seed old file");

        write_atomic(&target, b"new").expect("atomic write");

        assert_eq!(fs::read_to_string(&target).unwrap(), "new");
        assert!(temporary_entries(directory.path()).is_empty());
    }

    #[test]
    fn failure_keeps_the_target_and_removes_the_temporary() {
        let directory = TempDir::new();
        let target = directory.path().join("state.json");
        // A directory cannot be replaced by a file rename on either platform.
        fs::create_dir(&target).expect("seed target directory");

        let error = write_atomic(&target, b"new").expect_err("rename onto a directory must fail");

        assert!(error.to_string().contains("state.json"));
        assert!(target.is_dir());
        assert!(temporary_entries(directory.path()).is_empty());
    }

    #[test]
    fn forced_replace_failure_keeps_the_old_file_contents() {
        let directory = TempDir::new();
        let target = directory.path().join("state.json");
        fs::write(&target, "old").expect("seed old file");

        let error = write_atomic_with(&target, b"new", |_, _| {
            Err(io::Error::other("forced replace failure"))
        })
        .expect_err("the injected replace must fail");

        assert!(error.to_string().contains("state.json"));
        assert_eq!(fs::read_to_string(&target).unwrap(), "old");
        assert!(temporary_entries(directory.path()).is_empty());
    }

    #[test]
    fn remove_if_present_accepts_a_missing_file() {
        let directory = TempDir::new();
        remove_if_present(&directory.path().join("absent.json")).expect("missing is success");
    }

    #[test]
    fn remove_if_present_removes_a_real_file() {
        let directory = TempDir::new();
        let target = directory.path().join("state.json");
        fs::write(&target, "old").expect("seed file");

        remove_if_present(&target).expect("remove");

        assert!(!target.exists());
    }

    #[cfg(unix)]
    #[test]
    fn write_atomic_preserves_a_symlink_and_updates_its_target() {
        use std::os::unix::fs::symlink;

        let directory = TempDir::new();
        let target = directory.path().join("real.json");
        let link = directory.path().join("link.json");
        fs::write(&target, "old").expect("seed target");
        symlink(&target, &link).expect("create symlink");

        write_atomic(&link, b"new").expect("write through link");

        assert!(fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
        assert_eq!(fs::read_to_string(&target).unwrap(), "new");
        assert!(temporary_entries(directory.path()).is_empty());
    }
}
