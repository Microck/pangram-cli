//! Database open, protection, and schema ownership for [`HistoryStore`].
//!
//! Protection is established before SQLite is ever opened (fail closed), and
//! re-verified on every open. WAL, foreign-key, and secure-delete pragmas are
//! applied per connection and read back through the same connection so the
//! runtime value is proven, matching the Packet A probe style.

use std::path::{Path, PathBuf};

use rusqlite::Connection;

use super::{HistoryError, HistoryErrorCode};

pub const DATABASE_DIRECTORY_NAME: &str = "history";
pub const DATABASE_FILE_NAME: &str = "pangram-history.db";
pub const SCHEMA_VERSION: u32 = 1;

/// The schema body docs/history-contract.md locks at `user_version = 1`.
pub(super) const SCHEMA_V1: &str = "
CREATE TABLE bulk_collections (
  id TEXT PRIMARY KEY,
  upstream_bulk_id TEXT UNIQUE,
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
  check_count INTEGER NOT NULL DEFAULT 1 CHECK (check_count BETWEEN 1 AND 2),
  result_json TEXT,
  error_json TEXT,
  upstream_version TEXT,
  retry_of TEXT REFERENCES analyses(id) ON DELETE SET NULL,
  rerun_of TEXT REFERENCES analyses(id) ON DELETE SET NULL,
  submitted_at TEXT,
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
  PRIMARY KEY (analysis_id, check_kind),
  UNIQUE (check_kind, upstream_task_id)
);

CREATE TABLE analysis_checks (
  analysis_id TEXT NOT NULL REFERENCES analyses(id) ON DELETE CASCADE,
  check_index INTEGER NOT NULL,
  check_kind TEXT NOT NULL,
  status TEXT NOT NULL,
  result_json TEXT,
  error_json TEXT,
  PRIMARY KEY (analysis_id, check_index),
  UNIQUE (analysis_id, check_kind)
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
    connection: super::sidecars::GuardedConnection<super::sidecars::SidecarGuards, Connection>,
}

impl HistoryStore {
    /// Opens an existing history database without creating missing storage.
    ///
    /// History list/search/show adapters use this probe so a read against a
    /// fresh data directory remains side-effect free. A present filesystem
    /// object still flows through the full protection and schema checks.
    pub fn open_existing(data_dir: &Path) -> Result<Option<Self>, HistoryError> {
        let database = data_dir
            .join(DATABASE_DIRECTORY_NAME)
            .join(DATABASE_FILE_NAME);
        match std::fs::symlink_metadata(&database) {
            Ok(_) => Self::open(data_dir).map(Some),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(_) => Err(HistoryError::new(
                HistoryErrorCode::HistoryUnavailable,
                "the history database path could not be inspected",
            )),
        }
    }

    /// Opens (creating if necessary) the history database under the
    /// platform data directory in effect. Fails closed: protection is
    /// established or verified before any SQLite handle exists.
    pub fn open(data_dir: &Path) -> Result<Self, HistoryError> {
        Self::open_in(data_dir.join(DATABASE_DIRECTORY_NAME))
    }

    /// Root-relative open for tests and tooling. The history directory is
    /// `directory_name` itself; `open` is the adapter-facing entry point.
    pub(crate) fn open_in(directory: PathBuf) -> Result<Self, HistoryError> {
        Self::open_in_after_sidecar_checks(directory, || {})
    }

    pub(super) fn open_in_after_sidecar_checks(
        directory: PathBuf,
        after_sidecar_checks: impl FnOnce(),
    ) -> Result<Self, HistoryError> {
        // Absolute lexical resolution prevents a relative `file:` component
        // from acquiring SQLite URI semantics.
        let directory = if directory.is_absolute() {
            directory
        } else {
            std::env::current_dir()
                .map_err(|_| {
                    HistoryError::new(
                        HistoryErrorCode::HistoryUnavailable,
                        "the history database path could not be resolved",
                    )
                })?
                .join(directory)
        };
        protection::establish_directory(&directory)?;
        let database = directory.join(DATABASE_FILE_NAME);
        protection::establish_file(&database)?;

        let sidecars = [
            database.with_extension("db-wal"),
            database.with_extension("db-shm"),
        ];
        // Validate existing sidecars before SQLite can consult them.
        for sidecar in &sidecars {
            protection::verify_file_if_present(sidecar)?;
        }
        let sidecar_guards = super::sidecars::SidecarGuards::pin(&sidecars)?;
        after_sidecar_checks();

        // `Connection::open` is lazy: SQLite identifies the file format on
        // first real I/O. That is the integrity probe below; classifying a
        // mismatched file as corruption happens there, not here.
        let open_flags = rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE
            | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX
            | rusqlite::OpenFlags::SQLITE_OPEN_NOFOLLOW;
        let connection = super::sidecars::GuardedConnection::open_and_verify(
            sidecar_guards,
            || {
                Connection::open_with_flags(&database, open_flags).map_err(|_| {
                    HistoryError::new(
                        HistoryErrorCode::HistoryUnavailable,
                        "SQLite could not open the history database",
                    )
                })
            },
            |_| {
                // Existing sidecars require the database's owner-only policy.
                for sidecar in &sidecars {
                    protection::verify_file_if_present(sidecar)?;
                }
                Ok(())
            },
        )?;

        let mut store = Self { connection };
        // Busy waiting precedes the schema lock for concurrent first use.
        store.set_busy_timeout()?;

        // Corruption surfaces before the schema transaction or any write
        // pragma. SQLite treats a protected zero-byte file as an empty
        // database, which is the one crash-left state initialization accepts.
        store.integrity_probe()?;

        // Classification and creation share one SQLite-owned write lock.
        store.initialize_or_validate_schema()?;
        super::read_validation::certify_foreign_keys(&store.connection)?;

        store.prepare_connection()?;

        // Newly created WAL/SHM sidecars receive owner-only protection.
        for sidecar in &sidecars {
            if protection::verify_file_if_present(sidecar)? {
                protection::restrict_file(sidecar)?;
                protection::verify_file(sidecar)?;
            }
        }
        store.connection.guards_mut().arm_cleanup();

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

    /// Runs `operation` against the borrowed store connection.
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

    /// Runs `operation` inside one `IMMEDIATE` transaction that commits on
    /// success and rolls back on any failure. `IMMEDIATE` takes the database
    /// write lock at `BEGIN`, so concurrent reconciliation processes
    /// serialize on the write path for the whole atomic unit (the lookup,
    /// the merge, and the insert-or-refresh) instead of racing a deferred
    /// read that upgrades only at write time. A deferred transaction can
    /// still surface `SQLITE_BUSY` at commit against a peer that already
    /// holds the write lock, so a busy or locked commit is retried a bounded
    /// number of times before failing closed as `history_write_failed`.
    pub(crate) fn in_immediate_transaction<T>(
        &mut self,
        mut operation: impl FnMut(&rusqlite::Transaction<'_>) -> Result<T, HistoryError>,
    ) -> Result<T, HistoryError> {
        const MAX_BUSY_RETRIES: u32 = 8;
        let mut attempt: u32 = 0;
        loop {
            let transaction = self
                .connection
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .map_err(|error| {
                    busy_or(
                        HistoryErrorCode::HistoryWriteFailed,
                        "begin transaction",
                        error,
                    )
                })?;
            let outcome = operation(&transaction)?;
            match transaction.commit() {
                Ok(()) => return Ok(outcome),
                Err(error) => {
                    if sqlite_is_busy_or_locked(&error) && attempt < MAX_BUSY_RETRIES {
                        attempt += 1;
                        std::thread::sleep(std::time::Duration::from_millis(
                            25 * u64::from(attempt),
                        ));
                        continue;
                    }
                    return Err(busy_or(
                        HistoryErrorCode::HistoryWriteFailed,
                        "commit transaction",
                        error,
                    ));
                }
            }
        }
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

    /// Checkpoints and truncates the WAL after a committed logical deletion.
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
        self.enable_wal()?;
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
        self.set_busy_timeout()?;
        Ok(())
    }

    fn enable_wal(&self) -> Result<(), HistoryError> {
        const MAX_BUSY_RETRIES: u32 = 8;
        let mut attempt = 0;
        loop {
            match self.connection.pragma_update(None, "journal_mode", "WAL") {
                Ok(()) => return Ok(()),
                Err(error) if sqlite_is_busy_or_locked(&error) && attempt < MAX_BUSY_RETRIES => {
                    // SQLite can bypass the busy handler while another opener
                    // activates or recovers WAL. Retry only that transient lock
                    // class; every other failure remains an immediate fail-close.
                    attempt += 1;
                    std::thread::sleep(std::time::Duration::from_millis(25 * u64::from(attempt)));
                }
                Err(_) => {
                    return Err(HistoryError::from_sqlite(
                        HistoryErrorCode::HistoryUnavailable,
                        "enable WAL",
                    ));
                }
            }
        }
    }

    fn set_busy_timeout(&self) -> Result<(), HistoryError> {
        self.connection
            .pragma_update(None, "busy_timeout", 5_000u32)
            .map_err(|_| {
                HistoryError::from_sqlite(HistoryErrorCode::HistoryUnavailable, "set busy timeout")
            })
    }

    fn initialize_or_validate_schema(&mut self) -> Result<(), HistoryError> {
        let transaction = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|error| {
                busy_or(
                    HistoryErrorCode::HistoryUnavailable,
                    "serialize schema initialization",
                    error,
                )
            })?;
        let user_version: u32 = transaction
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .map_err(|_| {
                HistoryError::from_sqlite(HistoryErrorCode::HistoryUnavailable, "schema probe")
            })?;

        match user_version {
            0 => {
                let has_schema_objects: bool = transaction
                    .query_row("SELECT EXISTS (SELECT 1 FROM sqlite_master)", [], |row| {
                        row.get(0)
                    })
                    .map_err(|_| {
                        HistoryError::from_sqlite(
                            HistoryErrorCode::HistoryUnavailable,
                            "empty schema probe",
                        )
                    })?;
                if has_schema_objects {
                    return Err(unknown_schema_version(0));
                }
                transaction.execute_batch(SCHEMA_V1).map_err(|_| {
                    HistoryError::from_sqlite(HistoryErrorCode::HistoryWriteFailed, "create schema")
                })?;
                transaction
                    .pragma_update(None, "user_version", SCHEMA_VERSION)
                    .map_err(|_| {
                        HistoryError::from_sqlite(
                            HistoryErrorCode::HistoryWriteFailed,
                            "record schema version",
                        )
                    })?;
            }
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
            unknown => return Err(unknown_schema_version(unknown)),
        }

        if !super::schema_v1::schema_structure_ok(&transaction)? {
            return Err(incompatible_schema_v1());
        }
        transaction.commit().map_err(|error| {
            busy_or(
                HistoryErrorCode::HistoryUnavailable,
                "commit schema initialization",
                error,
            )
        })
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

fn sqlite_is_busy_or_locked(error: &rusqlite::Error) -> bool {
    matches!(
        error,
        rusqlite::Error::SqliteFailure(inner, _)
            if matches!(
                inner.code,
                rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
            )
    )
}

fn unknown_schema_version(version: u32) -> HistoryError {
    HistoryError::new(
        HistoryErrorCode::HistoryCorrupt,
        format!(
            "the history database uses unknown schema user_version \
             {version}. Move the history directory aside and rerun \
             the command; the original file is preserved.",
        ),
    )
}

fn incompatible_schema_v1() -> HistoryError {
    HistoryError::new(
        HistoryErrorCode::HistoryCorrupt,
        "the history database claims schema user_version 1 but its \
         stored structure does not match the exact schema v1 contract \
         (missing or different tables, columns, indexes, uniqueness \
         rules, foreign keys, or the FTS5 search table). \
         Move the history directory aside and rerun the command; \
         the original file is preserved.",
    )
}

/// Maps a raw rusqlite failure onto the sanitized adapter error, keeping the
/// busy/locked classification consistent with the retry loop above: the raw
/// SQLite message is discarded (it can embed SQL text or values) and only the
/// operation name is reported.
fn busy_or(
    code: HistoryErrorCode,
    operation: &'static str,
    _error: rusqlite::Error,
) -> HistoryError {
    HistoryError::from_sqlite(code, operation)
}

/// Filesystem protection of the history directory and database file.
///
/// Unix enforces the exact `0700`/`0600` modes. Windows applies the Phase 1
/// owner-only ACL policy through `config::windows_acl`; the `windows-sys`
/// ACL types stay inside the existing crate module so this store adds no new
/// ACL policy.
pub(super) mod protection {
    use std::fs;
    use std::path::Path;

    use crate::history::{HistoryError, HistoryErrorCode};

    #[cfg(any(windows, test))]
    const WINDOWS_FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;

    #[cfg(any(windows, test))]
    const fn has_windows_reparse_attribute(attributes: u32) -> bool {
        attributes & WINDOWS_FILE_ATTRIBUTE_REPARSE_POINT != 0
    }

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

    fn unavailable() -> HistoryError {
        HistoryError::new(
            HistoryErrorCode::HistoryUnavailable,
            "history database path is not a regular file",
        )
    }

    #[cfg(unix)]
    pub(super) fn establish_directory(directory: &Path) -> Result<(), HistoryError> {
        use std::os::unix::fs::DirBuilderExt;

        match fs::symlink_metadata(directory) {
            Ok(_) => verify_directory(directory),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                // Build only the absent suffix, one component at a time. Each
                // directory is born with 0700 through mkdir(2); unlike
                // create_dir_all followed by chmod, no world-readable mode is
                // ever observable. A concurrent creator is safe: AlreadyExists
                // loses the race and then validates the exact object and mode.
                let mut missing = Vec::new();
                let mut cursor = directory;
                loop {
                    match fs::symlink_metadata(cursor) {
                        Ok(metadata) => {
                            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                                return Err(insecure());
                            }
                            break;
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                            missing.push(cursor.to_path_buf());
                            cursor = cursor.parent().ok_or_else(restriction_failed)?;
                        }
                        Err(_) => return Err(restriction_failed()),
                    }
                }

                for path in missing.iter().rev() {
                    let mut builder = fs::DirBuilder::new();
                    builder.mode(0o700);
                    match builder.create(path) {
                        Ok(()) => verify_directory(path)?,
                        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                            verify_directory(path)?;
                        }
                        Err(_) => return Err(restriction_failed()),
                    }
                }
                verify_directory(directory)
            }
            Err(_) => Err(restriction_failed()),
        }
    }

    #[cfg(unix)]
    pub(super) fn verify_directory(directory: &Path) -> Result<(), HistoryError> {
        use std::os::unix::fs::PermissionsExt;

        let metadata = fs::symlink_metadata(directory).map_err(|_| restriction_failed())?;
        if metadata.file_type().is_symlink()
            || !metadata.is_dir()
            || metadata.permissions().mode() & 0o7777 != 0o700
        {
            return Err(insecure());
        }
        Ok(())
    }

    #[cfg(unix)]
    pub(super) fn establish_file(file: &Path) -> Result<(), HistoryError> {
        use std::os::unix::fs::OpenOptionsExt;

        match fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(file)
        {
            Ok(handle) => {
                drop(handle);
                restrict_file(file)
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => verify_file(file),
            Err(_) => Err(restriction_failed()),
        }
    }

    #[cfg(unix)]
    pub(in crate::history) fn restrict_file(file: &Path) -> Result<(), HistoryError> {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};

        let (handle, metadata) = open_verified_regular_file(file)?;
        handle
            .set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|_| restriction_failed())?;
        let restricted = handle.metadata().map_err(|_| restriction_failed())?;
        if restricted.dev() != metadata.dev()
            || restricted.ino() != metadata.ino()
            || restricted.permissions().mode() & 0o7777 != 0o600
        {
            return Err(restriction_failed());
        }
        Ok(())
    }

    pub(super) fn verify_file_if_present(file: &Path) -> Result<bool, HistoryError> {
        match fs::symlink_metadata(file) {
            Ok(_) => {
                verify_file(file)?;
                Ok(true)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(_) => Err(restriction_failed()),
        }
    }

    #[cfg(unix)]
    pub(in crate::history) fn verify_file(file: &Path) -> Result<(), HistoryError> {
        use std::os::unix::fs::PermissionsExt;

        let (_handle, metadata) = open_verified_regular_file(file)?;
        if metadata.permissions().mode() & 0o7777 != 0o600 {
            return Err(insecure());
        }
        Ok(())
    }

    #[cfg(unix)]
    fn open_verified_regular_file(file: &Path) -> Result<(fs::File, fs::Metadata), HistoryError> {
        use std::os::unix::fs::MetadataExt;

        let before = fs::symlink_metadata(file).map_err(|_| restriction_failed())?;
        if before.file_type().is_symlink() || !before.is_file() {
            return Err(unavailable());
        }
        let handle = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(file)
            .map_err(|_| restriction_failed())?;
        let opened = handle.metadata().map_err(|_| restriction_failed())?;
        let after = fs::symlink_metadata(file).map_err(|_| restriction_failed())?;
        if after.file_type().is_symlink()
            || !after.is_file()
            || !opened.is_file()
            || before.dev() != opened.dev()
            || before.ino() != opened.ino()
            || after.dev() != opened.dev()
            || after.ino() != opened.ino()
        {
            return Err(unavailable());
        }
        Ok((handle, opened))
    }

    #[cfg(windows)]
    pub(super) fn establish_directory(directory: &Path) -> Result<(), HistoryError> {
        // Directories on Windows inherit the ACL from their parent; the
        // platform data directory under `%LOCALAPPDATA%` is user-private by
        // construction, so the directory rule matches the credentials one:
        // create, then protect what we own. The ACL seam is tested by the
        // Windows-native CI gate that owns `set_owner_only_acl`.
        match fs::symlink_metadata(directory) {
            Ok(_) => verify_directory(directory),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir_all(directory).map_err(|_| restriction_failed())?;
                verify_windows_object(directory, true)?;
                crate::config::windows_acl::set_owner_only_acl(directory)
                    .map_err(|_| restriction_failed())?;
                verify_directory(directory)
            }
            Err(_) => Err(restriction_failed()),
        }
    }

    #[cfg(windows)]
    pub(super) fn verify_directory(directory: &Path) -> Result<(), HistoryError> {
        verify_windows_object(directory, true)?;
        crate::config::windows_acl::enforce_owner_only(directory).map_err(|_| insecure())
    }

    #[cfg(windows)]
    pub(super) fn establish_file(file: &Path) -> Result<(), HistoryError> {
        match fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(file)
        {
            Ok(handle) => {
                drop(handle);
                restrict_file(file)
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => verify_file(file),
            Err(_) => Err(restriction_failed()),
        }
    }

    #[cfg(windows)]
    pub(in crate::history) fn restrict_file(file: &Path) -> Result<(), HistoryError> {
        verify_windows_object(file, false)?;
        crate::config::windows_acl::set_owner_only_acl(file).map_err(|_| restriction_failed())?;
        verify_file(file)
    }

    #[cfg(windows)]
    pub(in crate::history) fn verify_file(file: &Path) -> Result<(), HistoryError> {
        verify_windows_object(file, false)?;
        crate::config::windows_acl::enforce_owner_only(file).map_err(|_| insecure())
    }

    #[cfg(windows)]
    fn verify_windows_object(path: &Path, directory: bool) -> Result<(), HistoryError> {
        use std::os::windows::fs::MetadataExt;

        let metadata = fs::symlink_metadata(path).map_err(|_| restriction_failed())?;
        if has_windows_reparse_attribute(metadata.file_attributes())
            || metadata.file_type().is_symlink()
            || (directory && !metadata.is_dir())
            || (!directory && !metadata.is_file())
        {
            return Err(unavailable());
        }
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::{WINDOWS_FILE_ATTRIBUTE_REPARSE_POINT, has_windows_reparse_attribute};

        #[test]
        fn every_windows_reparse_attribute_combination_is_rejected() {
            assert!(!has_windows_reparse_attribute(0));
            assert!(!has_windows_reparse_attribute(0x20));
            assert!(has_windows_reparse_attribute(
                WINDOWS_FILE_ATTRIBUTE_REPARSE_POINT
            ));
            assert!(has_windows_reparse_attribute(
                WINDOWS_FILE_ATTRIBUTE_REPARSE_POINT | 0x10 | 0x20
            ));
        }
    }
}
