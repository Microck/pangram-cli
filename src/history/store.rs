//! Database open, protection, and schema ownership for [`HistoryStore`].
//!
//! Protection is established before SQLite is ever opened (fail closed), and
//! re-verified on every open. WAL, foreign-key, and secure-delete pragmas are
//! applied per connection and read back through the same connection so the
//! runtime value is proven, matching the Packet A probe style.

use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::Connection;

use super::{HistoryError, HistoryErrorCode};

pub const DATABASE_DIRECTORY_NAME: &str = "history";
pub const DATABASE_FILE_NAME: &str = "pangram-history.db";
pub const SCHEMA_VERSION: u32 = 1;

/// The schema body docs/history-contract.md locks at `user_version = 1`.
const SCHEMA_V1: &str = "
CREATE TABLE bulk_collections (
  id TEXT PRIMARY KEY,
  upstream_bulk_id TEXT,
  status TEXT NOT NULL,
  submission_outcome TEXT NOT NULL,
  total_items INTEGER NOT NULL,
  accepted INTEGER NOT NULL,
  succeeded INTEGER NOT NULL,
  failed INTEGER NOT NULL,
  estimated_billable_units INTEGER NOT NULL,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  completed_at TEXT
);

CREATE TABLE analyses (
  id TEXT PRIMARY KEY,
  bulk_id TEXT REFERENCES bulk_collections(id),
  bulk_index INTEGER,
  caller_id TEXT,
  status TEXT NOT NULL,
  submission_outcome TEXT NOT NULL,
  save_state TEXT NOT NULL,
  input_type TEXT NOT NULL,
  input_sha256 TEXT NOT NULL,
  display_name TEXT,
  input_json TEXT NOT NULL,
  result_json TEXT,
  error_json TEXT,
  retry_of TEXT REFERENCES analyses(id),
  rerun_of TEXT REFERENCES analyses(id),
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  completed_at TEXT,
  UNIQUE (bulk_id, bulk_index)
);

CREATE TABLE upstream_tasks (
  analysis_id TEXT NOT NULL REFERENCES analyses(id) ON DELETE CASCADE,
  check_kind TEXT NOT NULL,
  upstream_task_id TEXT NOT NULL,
  last_stage TEXT,
  observed_at TEXT NOT NULL,
  PRIMARY KEY (analysis_id, check_kind)
);

CREATE VIRTUAL TABLE analysis_search USING fts5(
  analysis_id UNINDEXED,
  input_text,
  filename,
  headline,
  source_urls,
  tokenize = 'unicode61'
);

CREATE INDEX analyses_status_created
  ON analyses(status, created_at DESC);

CREATE INDEX analyses_bulk_index
  ON analyses(bulk_id, bulk_index);
";

/// The one concrete SQLite history store. Owns its single connection;
/// adapters hold the store itself rather than borrowing the connection.
#[derive(Debug)]
pub struct HistoryStore {
    connection: Connection,
}

impl HistoryStore {
    /// Opens (creating if necessary) the history database under the
    /// platform data directory in effect. Fails closed: protection is
    /// established or verified before any SQLite handle exists.
    pub fn open(data_dir: &Path) -> Result<Self, HistoryError> {
        Self::open_in(data_dir.join(DATABASE_DIRECTORY_NAME))
    }

    /// Root-relative open for tests and tooling. The history directory is
    /// `directory_name` itself; `open` is the adapter-facing entry point.
    pub(crate) fn open_in(directory: PathBuf) -> Result<Self, HistoryError> {
        protection::establish_directory(&directory)?;
        let database = directory.join(DATABASE_FILE_NAME);

        let exists = match fs::metadata(&database) {
            Ok(metadata) => {
                if !metadata.is_file() {
                    return Err(HistoryError::new(
                        HistoryErrorCode::HistoryUnavailable,
                        "history database path is not a regular file",
                    ));
                }
                true
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(_) => {
                return Err(HistoryError::new(
                    HistoryErrorCode::HistoryUnavailable,
                    "cannot inspect the history database path",
                ));
            }
        };

        if exists {
            protection::verify_file(&database)?;
        }

        // `Connection::open` is lazy: SQLite identifies the file format on
        // first real I/O. That is the integrity probe below; classifying a
        // mismatched file as corruption happens there, not here.
        let connection = Connection::open(&database).map_err(|_| {
            HistoryError::new(
                HistoryErrorCode::HistoryUnavailable,
                "SQLite could not open the history database",
            )
        })?;

        // A brand-new file must become owner-only before any page is written.
        if !exists {
            protection::restrict_file(&database)?;
        }
        // WAL and SHM sidecars created by a prior run must carry the same
        // owner-only protection as the database itself; an insecure sidecar
        // fails closed the same way the database would.
        for sidecar in [
            database.with_extension("db-wal"),
            database.with_extension("db-shm"),
        ] {
            if sidecar.exists() {
                protection::verify_file(&sidecar)?;
            }
        }

        let store = Self { connection };
        // Corruption surfaces here, before any write pragma is tried.
        if exists {
            store.integrity_probe()?;
        }
        store.prepare_connection()?;

        // WAL/SHM sidecars are created by SQLite lazily, after the WAL
        // pragma. Restrict them now so they always carry the same exact
        // owner-only mode as the database itself; verification already ran
        // for pre-existing sidecars above.
        for sidecar in [
            store.database_path().with_extension("db-wal"),
            store.database_path().with_extension("db-shm"),
        ] {
            if sidecar.exists() {
                protection::restrict_file(&sidecar)?;
            }
        }

        let user_version: u32 = store
            .connection_ref()
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .map_err(|_| {
                HistoryError::from_sqlite(HistoryErrorCode::HistoryUnavailable, "schema probe")
            })?;

        match user_version {
            0 => store.initialize_schema()?,
            SCHEMA_VERSION => {}
            newer if newer > SCHEMA_VERSION => {
                return Err(HistoryError::new(
                    HistoryErrorCode::HistoryCorrupt,
                    format!(
                        "the history database uses schema user_version {newer}, \
                         which is newer than the supported version {SCHEMA_VERSION}. \
                         Move the history directory aside and rerun the command; \
                         the original file is preserved.",
                    ),
                ));
            }
            unknown => {
                return Err(HistoryError::new(
                    HistoryErrorCode::HistoryCorrupt,
                    format!(
                        "the history database uses unknown schema user_version \
                         {unknown}. Move the history directory aside and rerun \
                         the command; the original file is preserved.",
                    ),
                ));
            }
        }

        Ok(store)
    }

    /// The resolved history directory.
    #[must_use]
    pub fn directory(&self) -> PathBuf {
        self.database_path()
            .parent()
            .map_or_else(|| PathBuf::from(DATABASE_DIRECTORY_NAME), Path::to_path_buf)
    }

    /// The absolute path of the database file.
    #[must_use]
    pub fn database_path(&self) -> PathBuf {
        self.connection
            .path()
            .map_or_else(|| PathBuf::from(DATABASE_FILE_NAME), PathBuf::from)
    }

    /// The recorded schema version.
    pub fn user_version(&self) -> Result<u32, HistoryError> {
        self.connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .map_err(|_| {
                HistoryError::from_sqlite(HistoryErrorCode::HistoryUnavailable, "read schema")
            })
    }

    /// Runs `operation` against the store's connection. The closure receives
    /// a `&Connection` so tests can assert raw SQLite state (pragma values,
    /// `sqlite_master` rows) without the store handing out ownership of its
    /// handle.
    pub fn with_connection<T>(
        &self,
        operation: impl FnOnce(&Connection) -> T,
    ) -> Result<T, HistoryError> {
        Ok(operation(&self.connection))
    }

    /// The borrowed connection, for read helpers in the sibling module.
    pub(crate) fn connection_ref(&self) -> &Connection {
        &self.connection
    }

    /// Runs `operation` inside one transaction that commits on success and
    /// rolls back on any failure. `&mut self` is the honest ownership shape:
    /// every mutation owns its transaction until it commits.
    pub(crate) fn in_transaction<T>(
        &mut self,
        operation: impl FnOnce(&rusqlite::Transaction<'_>) -> Result<T, HistoryError>,
    ) -> Result<T, HistoryError> {
        let transaction = self.connection.transaction().map_err(|_| {
            HistoryError::from_sqlite(HistoryErrorCode::HistoryWriteFailed, "begin transaction")
        })?;
        let outcome = operation(&transaction)?;
        transaction.commit().map_err(|_| {
            HistoryError::from_sqlite(HistoryErrorCode::HistoryWriteFailed, "commit transaction")
        })?;
        Ok(outcome)
    }

    /// Checkpoint the WAL, truncating it. A failure is reported to the
    /// caller but must remain distinguishable from a committed logical
    /// delete; callers decide whether the operation reports success with a
    /// warning or the error itself.
    pub(crate) fn checkpoint_truncate(&self) -> Result<(), HistoryError> {
        self.connection
            .pragma_update(None, "wal_checkpoint", "TRUNCATE")
            .map_err(|_| {
                HistoryError::from_sqlite(HistoryErrorCode::HistoryWriteFailed, "wal checkpoint")
            })
    }

    fn prepare_connection(&self) -> Result<(), HistoryError> {
        // Apply and then verify the runtime value: a filesystem that cannot
        // honor WAL or a foreign-key build flag must fail closed here, not
        // midway through a transaction.
        self.connection
            .pragma_update(None, "journal_mode", "WAL")
            .map_err(|_| {
                HistoryError::from_sqlite(HistoryErrorCode::HistoryUnavailable, "enable WAL")
            })?;
        let applied: String = self
            .connection
            .pragma_query_value(None, "journal_mode", |row| row.get(0))
            .map_err(|_| {
                HistoryError::from_sqlite(HistoryErrorCode::HistoryUnavailable, "verify WAL")
            })?;
        if !applied.eq_ignore_ascii_case("wal") {
            return Err(HistoryError::new(
                HistoryErrorCode::HistoryUnavailable,
                "the filesystem or SQLite build cannot enable WAL journal mode",
            ));
        }
        self.connection
            .pragma_update(None, "foreign_keys", true)
            .map_err(|_| {
                HistoryError::from_sqlite(
                    HistoryErrorCode::HistoryUnavailable,
                    "enable foreign keys",
                )
            })?;
        self.connection
            .pragma_update(None, "secure_delete", true)
            .map_err(|_| {
                HistoryError::from_sqlite(
                    HistoryErrorCode::HistoryUnavailable,
                    "enable secure delete",
                )
            })?;
        self.connection
            .pragma_update(None, "busy_timeout", 5_000u32)
            .map_err(|_| {
                HistoryError::from_sqlite(HistoryErrorCode::HistoryUnavailable, "set busy timeout")
            })?;
        Ok(())
    }

    fn initialize_schema(&self) -> Result<(), HistoryError> {
        self.connection.execute_batch(SCHEMA_V1).map_err(|_| {
            HistoryError::from_sqlite(HistoryErrorCode::HistoryWriteFailed, "create schema")
        })?;
        self.connection
            .pragma_update(None, "user_version", SCHEMA_VERSION)
            .map_err(|_| {
                HistoryError::from_sqlite(
                    HistoryErrorCode::HistoryWriteFailed,
                    "record schema version",
                )
            })?;
        Ok(())
    }

    /// A short real read against the SQLite engine that fails malformed or
    /// unreadable files before any application statement runs. The original
    /// file is never replaced: the error surfaces as `history_corrupt`.
    fn integrity_probe(&self) -> Result<(), HistoryError> {
        match self
            .connection
            .pragma_query_value(None, "quick_check", |row| row.get::<_, String>(0))
        {
            Ok(quick_check) if quick_check == "ok" => Ok(()),
            Ok(other) => Err(HistoryError::new(
                HistoryErrorCode::HistoryCorrupt,
                format!(
                    "the history database failed its integrity check ({other}). \
                     Move the history directory aside and rerun the command; \
                     the original file is preserved."
                ),
            )),
            Err(_) => Err(HistoryError::new(
                HistoryErrorCode::HistoryCorrupt,
                "the history database could not be read as a SQLite database. \
                 Move the history directory aside and rerun the command; the \
                 original file is preserved.",
            )),
        }
    }
}

/// Filesystem protection of the history directory and database file.
///
/// Unix enforces the exact `0700`/`0600` modes. Windows applies the Phase 1
/// owner-only ACL policy through `config::windows_acl`; the `windows-sys`
/// ACL types stay inside the existing crate module so this store adds no new
/// ACL policy.
mod protection {
    use std::fs;
    use std::path::Path;

    use crate::history::{HistoryError, HistoryErrorCode};

    fn insecure() -> HistoryError {
        HistoryError::new(
            HistoryErrorCode::InsecureHistoryPermissions,
            "history storage is not protected by owner-only permissions",
        )
    }

    fn restriction_failed() -> HistoryError {
        HistoryError::new(
            HistoryErrorCode::InsecureHistoryPermissions,
            "owner-only permissions could not be established for history storage",
        )
    }

    #[cfg(unix)]
    pub(super) fn establish_directory(directory: &Path) -> Result<(), HistoryError> {
        use std::os::unix::fs::PermissionsExt;

        match fs::metadata(directory) {
            Ok(metadata) => {
                // An existing directory must already be owner-only. The contract
                // fails closed rather than silently downgrading a shared or
                // tampered-with directory.
                if !metadata.is_dir() || metadata.permissions().mode() & 0o7777 != 0o700 {
                    return Err(insecure());
                }
                Ok(())
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                // Fresh creation: set the exact mode on the open directory
                // before any content lands. `set_permissions` on the created
                // path is exact (no umask subtraction), matching the Phase 1
                // atomic credential write.
                fs::create_dir_all(directory).map_err(|_| restriction_failed())?;
                fs::set_permissions(directory, fs::Permissions::from_mode(0o700))
                    .map_err(|_| restriction_failed())?;
                verify_directory(directory)
            }
            Err(_) => Err(restriction_failed()),
        }
    }

    #[cfg(unix)]
    pub(super) fn verify_directory(directory: &Path) -> Result<(), HistoryError> {
        use std::os::unix::fs::PermissionsExt;

        let metadata = fs::metadata(directory).map_err(|_| restriction_failed())?;
        if metadata.permissions().mode() & 0o7777 != 0o700 {
            return Err(insecure());
        }
        Ok(())
    }

    #[cfg(unix)]
    pub(super) fn restrict_file(file: &Path) -> Result<(), HistoryError> {
        use std::os::unix::fs::PermissionsExt;

        fs::set_permissions(file, fs::Permissions::from_mode(0o600))
            .map_err(|_| restriction_failed())
    }

    #[cfg(unix)]
    pub(super) fn verify_file(file: &Path) -> Result<(), HistoryError> {
        use std::os::unix::fs::PermissionsExt;

        let metadata = fs::metadata(file).map_err(|_| restriction_failed())?;
        if metadata.permissions().mode() & 0o7777 != 0o600 {
            return Err(insecure());
        }
        Ok(())
    }

    #[cfg(windows)]
    pub(super) fn establish_directory(directory: &Path) -> Result<(), HistoryError> {
        // Directories on Windows inherit the ACL from their parent; the
        // platform data directory under `%LOCALAPPDATA%` is user-private by
        // construction, so the directory rule matches the credentials one:
        // create, then protect what we own. The ACL seam is tested by the
        // Windows-native CI gate that owns `set_owner_only_acl`.
        fs::create_dir_all(directory).map_err(|_| restriction_failed())?;
        crate::config::windows_acl::set_owner_only_acl(directory).map_err(|_| restriction_failed())
    }

    #[cfg(windows)]
    pub(super) fn verify_directory(directory: &Path) -> Result<(), HistoryError> {
        crate::config::windows_acl::enforce_owner_only(directory).map_err(|_| insecure())
    }

    #[cfg(windows)]
    pub(super) fn restrict_file(file: &Path) -> Result<(), HistoryError> {
        crate::config::windows_acl::set_owner_only_acl(file).map_err(|_| restriction_failed())
    }

    #[cfg(windows)]
    pub(super) fn verify_file(file: &Path) -> Result<(), HistoryError> {
        crate::config::windows_acl::enforce_owner_only(file).map_err(|_| insecure())
    }
}
