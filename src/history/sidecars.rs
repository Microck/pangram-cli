//! Lifetime guards for SQLite WAL and shared-memory sidecars.
//!
//! The bundled Unix VFS opens WAL and SHM files with `O_NOFOLLOW`. Windows
//! SQLite does not request the corresponding reparse-point behavior, so the
//! store pins both terminal names before opening SQLite and excludes delete
//! sharing until immediately before the connection closes.

use std::ops::{Deref, DerefMut};

#[cfg(not(windows))]
use std::path::PathBuf;

use super::HistoryError;

/// Owns the sidecar identity pins and SQLite connection in their required
/// destruction order. Rust drops fields in declaration order, so the Pangram
/// guards release their no-delete handles immediately before SQLite closes.
/// SQLite's already-open Windows WAL/SHM handles preserve object identity
/// through that sequential handoff and then perform normal final cleanup.
#[derive(Debug)]
pub(super) struct GuardedConnection<Guards, Inner> {
    guards: Guards,
    connection: Inner,
}

impl<Guards, Inner> GuardedConnection<Guards, Inner> {
    pub(super) fn new(guards: Guards, connection: Inner) -> Self {
        Self { guards, connection }
    }

    pub(super) fn open_and_verify<Error>(
        guards: Guards,
        open: impl FnOnce() -> Result<Inner, Error>,
        verify: impl FnOnce(&Inner) -> Result<(), Error>,
    ) -> Result<Self, Error> {
        let guarded = Self::new(guards, open()?);
        verify(&guarded.connection)?;
        Ok(guarded)
    }

    pub(super) fn guards_mut(&mut self) -> &mut Guards {
        &mut self.guards
    }
}

impl<Guards, Inner> Deref for GuardedConnection<Guards, Inner> {
    type Target = Inner;

    fn deref(&self) -> &Self::Target {
        &self.connection
    }
}

impl<Guards, Inner> DerefMut for GuardedConnection<Guards, Inner> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.connection
    }
}

#[cfg(test)]
mod guarded_connection_tests {
    use std::cell::RefCell;

    use super::GuardedConnection;

    #[derive(Debug)]
    struct DropProbe<'a> {
        name: &'static str,
        drops: &'a RefCell<Vec<&'static str>>,
    }

    impl Drop for DropProbe<'_> {
        fn drop(&mut self) {
            self.drops.borrow_mut().push(self.name);
        }
    }

    #[test]
    fn sidecar_guards_drop_before_the_sqlite_connection() {
        let drops = RefCell::new(Vec::new());
        let guarded = GuardedConnection::new(
            DropProbe {
                name: "guards",
                drops: &drops,
            },
            DropProbe {
                name: "connection",
                drops: &drops,
            },
        );

        drop(guarded);

        assert_eq!(*drops.borrow(), ["guards", "connection"]);
    }

    #[test]
    fn post_open_verification_error_drops_guards_before_the_connection() {
        let drops = RefCell::new(Vec::new());
        let result = GuardedConnection::open_and_verify(
            DropProbe {
                name: "guards",
                drops: &drops,
            },
            || {
                Ok::<_, &'static str>(DropProbe {
                    name: "connection",
                    drops: &drops,
                })
            },
            |_| Err("post-open verification failed"),
        );

        assert_eq!(result.unwrap_err(), "post-open verification failed");
        assert_eq!(*drops.borrow(), ["guards", "connection"]);
    }
}

#[cfg(not(windows))]
#[derive(Debug)]
pub(super) struct SidecarGuards;

#[cfg(not(windows))]
impl SidecarGuards {
    pub(super) fn pin(_paths: &[PathBuf; 2]) -> Result<Self, HistoryError> {
        Ok(Self)
    }

    pub(super) fn arm_cleanup(&mut self) {}
}

#[cfg(windows)]
mod windows {
    use std::fs::{self, File, OpenOptions};
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;
    use std::path::{Path, PathBuf};

    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_OPEN_REPARSE_POINT,
        FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_TYPE_DISK, GetFileInformationByHandle, GetFileType,
    };

    use super::HistoryError;
    use crate::history::HistoryErrorCode;

    const fn sidecar_share_mode() -> u32 {
        FILE_SHARE_READ | FILE_SHARE_WRITE
    }

    const fn sidecar_custom_flags() -> u32 {
        FILE_FLAG_OPEN_REPARSE_POINT
    }

    fn unavailable() -> HistoryError {
        HistoryError::new(
            HistoryErrorCode::HistoryUnavailable,
            "history database sidecar path is not a regular file",
        )
    }

    fn options(create_new: bool) -> OpenOptions {
        let mut options = OpenOptions::new();
        options
            .read(true)
            .write(true)
            .create_new(create_new)
            .share_mode(sidecar_share_mode())
            .custom_flags(sidecar_custom_flags());
        options
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    struct FileIdentity {
        volume: u32,
        index_high: u32,
        index_low: u32,
    }

    fn identity(file: &File) -> Result<FileIdentity, HistoryError> {
        let raw = file.as_raw_handle() as HANDLE;
        // SAFETY: `raw` is borrowed from a live `File`; the output points to
        // writable stack storage and is read only after the API succeeds.
        if unsafe { GetFileType(raw) } != FILE_TYPE_DISK {
            return Err(unavailable());
        }
        let mut information = std::mem::MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::uninit();
        // SAFETY: same live handle; the API initializes the complete output
        // structure on success.
        if unsafe { GetFileInformationByHandle(raw, information.as_mut_ptr()) } == 0 {
            return Err(unavailable());
        }
        // SAFETY: success above initialized the value.
        let information = unsafe { information.assume_init() };
        if information.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(unavailable());
        }
        let metadata = file.metadata().map_err(|_| unavailable())?;
        if !metadata.is_file() {
            return Err(unavailable());
        }
        Ok(FileIdentity {
            volume: information.dwVolumeSerialNumber,
            index_high: information.nFileIndexHigh,
            index_low: information.nFileIndexLow,
        })
    }

    fn open_existing(path: &Path) -> Result<(File, FileIdentity), HistoryError> {
        let file = options(false).open(path).map_err(|_| unavailable())?;
        let identity = identity(&file)?;
        Ok((file, identity))
    }

    #[derive(Debug)]
    struct SidecarGuard {
        path: PathBuf,
        file: Option<File>,
        identity: FileIdentity,
        created: bool,
        cleanup_armed: bool,
    }

    impl SidecarGuard {
        fn pin(path: PathBuf) -> Result<Self, HistoryError> {
            let (file, created) = match options(true).open(&path) {
                Ok(file) => (file, true),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => (
                    options(false).open(&path).map_err(|_| unavailable())?,
                    false,
                ),
                Err(_) => return Err(unavailable()),
            };
            let identity = identity(&file)?;

            if created {
                super::super::store::protection::restrict_file(&path)?;
            } else {
                super::super::store::protection::verify_file(&path)?;
            }

            let (verification, path_identity) = open_existing(&path)?;
            if path_identity != identity {
                return Err(unavailable());
            }
            drop(verification);

            Ok(Self {
                path,
                file: Some(file),
                identity,
                created,
                cleanup_armed: false,
            })
        }

        fn arm_cleanup(&mut self) {
            self.cleanup_armed = true;
        }
    }

    impl Drop for SidecarGuard {
        fn drop(&mut self) {
            drop(self.file.take());
            if !self.created && !self.cleanup_armed {
                return;
            }

            // Every accepted sidecar was a normal, owner-only disk file pinned
            // by identity. Reopen without following a reparse point and remove
            // only that same object. A concurrent SQLite guard excludes delete,
            // so its live sidecar survives and the last closer performs cleanup.
            let Ok((verification, current)) = open_existing(&self.path) else {
                return;
            };
            if current != self.identity
                || super::super::store::protection::verify_file(&self.path).is_err()
            {
                return;
            }
            drop(verification);
            let _ = fs::remove_file(&self.path);
        }
    }

    #[derive(Debug)]
    pub(crate) struct SidecarGuards {
        wal: SidecarGuard,
        shm: SidecarGuard,
    }

    impl SidecarGuards {
        pub(crate) fn pin(paths: &[PathBuf; 2]) -> Result<Self, HistoryError> {
            let wal = SidecarGuard::pin(paths[0].clone())?;
            let shm = SidecarGuard::pin(paths[1].clone())?;
            Ok(Self { wal, shm })
        }

        pub(crate) fn arm_cleanup(&mut self) {
            self.wal.arm_cleanup();
            self.shm.arm_cleanup();
        }
    }

    #[cfg(test)]
    mod tests {
        use std::time::{Duration, Instant};

        use super::*;
        use crate::history::HistoryStore;

        #[test]
        fn sidecar_open_flags_pin_reparse_names_without_delete_sharing() {
            assert_eq!(
                sidecar_custom_flags() & FILE_FLAG_OPEN_REPARSE_POINT,
                FILE_FLAG_OPEN_REPARSE_POINT
            );
            assert_eq!(
                sidecar_share_mode(),
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                "SQLite peers may share reads and writes"
            );
            assert_eq!(
                sidecar_share_mode() & windows_sys::Win32::Storage::FileSystem::FILE_SHARE_DELETE,
                0,
                "rename/delete replacement remains excluded"
            );
        }

        #[test]
        fn zero_length_pins_support_sqlite_wal_and_clean_up_after_last_close() {
            let root = tempfile::tempdir().expect("temporary history root");
            let first = HistoryStore::open(root.path()).expect("first pinned open");
            let database = first.database_path();
            let wal = database.with_extension("db-wal");
            let shm = database.with_extension("db-shm");
            assert!(wal.is_file(), "pinned WAL exists while SQLite is open");
            assert!(shm.is_file(), "pinned SHM exists while SQLite is open");

            let second = HistoryStore::open(root.path()).expect("concurrent pinned open");
            assert_eq!(second.user_version().expect("schema version"), 1);
            drop(first);
            assert!(wal.is_file(), "a peer still pins the WAL");
            assert!(shm.is_file(), "a peer still pins the SHM");
            assert_eq!(
                second.user_version().expect("peer remains valid"),
                1,
                "the surviving connection retains valid WAL state"
            );

            let close_started = Instant::now();
            drop(second);
            let close_elapsed = close_started.elapsed();

            assert!(!wal.exists(), "the last closer removes the guarded WAL");
            assert!(!shm.exists(), "the last closer removes the guarded SHM");
            assert!(
                close_elapsed < Duration::from_secs(2),
                "last close took {close_elapsed:?}; guard handles likely delayed SQLite cleanup"
            );
        }
    }
}

#[cfg(windows)]
pub(super) use windows::SidecarGuards;

#[cfg(all(test, unix))]
mod unix_tests {
    use std::fs;
    use std::os::unix::fs::{PermissionsExt, symlink};

    use crate::history::{HistoryErrorCode, HistoryStore};

    #[test]
    fn raced_terminal_wal_and_shm_symlinks_fail_without_mutating_the_target() {
        for suffix in ["db-wal", "db-shm"] {
            let root = tempfile::tempdir().expect("temporary history root");
            let initialized = HistoryStore::open(root.path()).expect("initialize database");
            let directory = initialized.directory();
            let database = initialized.database_path();
            drop(initialized);

            let target = root.path().join(format!("{suffix}-target"));
            let sentinel = b"raced sidecar target sentinel";
            fs::write(&target, sentinel).expect("write target");
            fs::set_permissions(&target, fs::Permissions::from_mode(0o600))
                .expect("protect target");
            let sidecar = database.with_extension(suffix);

            let error = HistoryStore::open_in_after_sidecar_checks(directory, || {
                symlink(&target, &sidecar).expect("race in terminal sidecar alias");
            })
            .expect_err("bundled SQLite must reject the raced terminal alias");

            assert_eq!(error.code(), HistoryErrorCode::HistoryUnavailable);
            assert_eq!(
                fs::read(&target).expect("target survives"),
                sentinel,
                "{suffix}: SQLite followed and mutated the raced alias"
            );
            assert!(
                fs::symlink_metadata(&sidecar)
                    .expect("alias survives")
                    .file_type()
                    .is_symlink(),
                "{suffix}: rejected alias was replaced"
            );
        }
    }
}
