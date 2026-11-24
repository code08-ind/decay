//! Decay's storage (Software Design §4, DB Design): one SQLCipher-encrypted SQLite file with
//! FTS5 search, forward-only migrations, daily backups, an integrity check at open with
//! automatic restore from the latest good backup, and "Erase all data".
//!
//! The API is synchronous (rusqlite); the engine owns the [`Store`] on one thread.
//! Timestamps are Unix epoch milliseconds, passed in by the caller (no clocks here).

mod backup;
mod diagnostics;
mod events;
mod export;
mod import;
mod items;
mod key;
mod labels;
mod library;
mod license;
mod metrics;
pub use library::ItemTarget;
mod migrate;
mod profiles;
mod rules;
mod search;
mod settings;
mod sources;
mod sweeps;
mod words;

use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, ErrorCode, Transaction};
use thiserror::Error;

pub use backup::{BackupInfo, BackupKind};
pub use diagnostics::{BackupSummary, StoreDiagnostics};
pub use events::EventRecord;
pub use export::{MAX_BOOKMARKS, bookmarks_to_items};
pub use import::{FileOptions, ImportReport, catalog_app_id};
pub use items::{App, Item, Platform, RestoreKind};
pub use key::DbKey;
pub use labels::{Batch, BatchItem, MIN_BATCH_ITEMS};
pub use license::LicenseRow;
pub use metrics::{MetricsDay, SourceCount};
pub use migrate::SCHEMA_VERSION;
pub use profiles::{MAX_PROFILE_NAME, Profile, ProfileLabel, clean_profile_name};
pub use search::{SearchFilters, SearchHit, SearchPage, SearchQuery, parse_query};
pub use settings::default_setting;
pub use sources::Source;
pub use sweeps::{Sweep, SweepItem};
pub use words::{fts_query, url_words};

/// Storage failures.
#[derive(Debug, Error)]
pub enum StoreError {
    /// The database can't be read with this key and no backup can either: a wrong key,
    /// or the file and every backup are damaged. Nothing was changed. (The two can't be
    /// told apart: the file is encrypted.)
    #[error(
        "the database can't be read with this key (wrong key, or it and all backups are damaged)"
    )]
    Unreadable,
    /// Another process (or another `Store`) has the database open.
    #[error("the database is already open in another Decay process")]
    InUse,
    #[error("database error: {0}")]
    Sql(#[from] rusqlite::Error),
    #[error("file error: {0}")]
    Io(#[from] std::io::Error),
    #[error("migration V{version} changed after it was applied")]
    MigrationChanged { version: u32 },
    #[error("migration V{version} failed: {reason}")]
    Migration { version: u32, reason: String },
    #[error(
        "the database is from a newer version of Decay (schema V{found}, this build knows V{known})"
    )]
    TooNew { found: u32, known: u32 },
    #[error("integrity check failed: {0}")]
    Corrupt(String),
    #[error("stored value is invalid: {0}")]
    InvalidData(String),
    #[error("not found")]
    NotFound,
    #[error("a rule with this scope, match and action already exists")]
    DuplicateRule,
}

pub type Result<T, E = StoreError> = std::result::Result<T, E>;

/// [`Store::erase`] failed. If the database couldn't be closed, the store is handed back
/// (nothing was deleted) so the caller can retry.
#[derive(Debug)]
pub struct EraseError {
    pub store: Option<Box<Store>>,
    pub error: StoreError,
}

/// Where the database and its backups live.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorePaths {
    pub db: PathBuf,
    pub backups: PathBuf,
}

impl StorePaths {
    /// `<dir>/decay.db` and `<dir>/backups/` (DB Design §1: the app's support folder).
    pub fn in_dir(dir: &Path) -> Self {
        Self { db: dir.join("decay.db"), backups: dir.join("backups") }
    }

    /// `decay.db` + `suffix` (`-wal`, `.restore.tmp`, …).
    pub(crate) fn db_with(&self, suffix: &str) -> PathBuf {
        let mut s = self.db.as_os_str().to_owned();
        s.push(suffix);
        PathBuf::from(s)
    }
}

/// What happened while opening.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OpenReport {
    /// A new database was created.
    pub created: bool,
    /// The database was damaged or missing and was replaced by this backup. A damaged file
    /// is kept next to it as `decay.db.damaged-<time>` (with its WAL).
    pub recovered_from: Option<PathBuf>,
    /// Migrations applied now.
    pub applied_migrations: Vec<u32>,
    /// The backup taken before migrating, if any.
    pub pre_migration_backup: Option<PathBuf>,
}

/// The open database. It holds an exclusive lock: one owner per file.
#[derive(Debug)]
pub struct Store {
    conn: Connection,
    paths: StorePaths,
    key: DbKey,
}

fn busy(err: &rusqlite::Error) -> bool {
    matches!(err.sqlite_error_code(), Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked))
}

/// Opens an encrypted file (creating it if missing) and registers Decay's SQL functions.
fn open_keyed(path: &Path, key: &DbKey) -> Result<Connection> {
    let conn = Connection::open(path)?;
    conn.execute_batch(&key.pragma())?;
    words::register(&conn)?;
    Ok(conn)
}

/// Checks a file: the key reads it and `PRAGMA quick_check` passes.
pub(crate) fn check(path: &Path, key: &DbKey) -> Result<()> {
    let conn = open_keyed(path, key)?;
    let read = conn.query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get::<_, i64>(0));
    if let Err(e) = read {
        return Err(if busy(&e) { StoreError::InUse } else { e.into() });
    }
    let result: String = conn.query_row("PRAGMA quick_check", [], |r| r.get(0))?;
    if result != "ok" {
        return Err(StoreError::Corrupt(result));
    }
    Ok(())
}

/// A failure that means the file can't be used as it is (damaged, or a wrong key).
pub(crate) fn unreadable(err: &StoreError) -> bool {
    match err {
        StoreError::Corrupt(_) => true,
        StoreError::Sql(e) => matches!(
            e.sqlite_error_code(),
            Some(ErrorCode::NotADatabase | ErrorCode::DatabaseCorrupt)
        ),
        _ => false,
    }
}

fn configure(conn: &Connection, created: bool) -> Result<()> {
    if created {
        // Only takes effect before the first table exists.
        conn.execute_batch("PRAGMA auto_vacuum = INCREMENTAL;")?;
    }
    // Exclusive locking: one owner per file, so a second instance can't write to (or
    // erase) data another one is using; WAL then needs no shared-memory file.
    // `fullfsync` makes macOS flush the drive cache on sync (plain fsync doesn't), so
    // durable commits survive a power loss (Software Design §7).
    conn.execute_batch(
        "PRAGMA locking_mode = EXCLUSIVE;
         PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;
         PRAGMA fullfsync = ON;
         PRAGMA checkpoint_fullfsync = ON;
         PRAGMA foreign_keys = ON;
         PRAGMA busy_timeout = 3000;
         PRAGMA temp_store = MEMORY;",
    )?;
    conn.execute_batch("BEGIN EXCLUSIVE; COMMIT;")
        .map_err(|e| if busy(&e) { StoreError::InUse } else { e.into() })?;
    Ok(())
}

impl Store {
    /// Opens (or creates) the database.
    ///
    /// When the file can't be read or fails its integrity check, or is missing / empty
    /// while backups exist, the newest backup that passes is restored (a damaged file is
    /// moved aside, never deleted). If none passes, [`StoreError::Unreadable`] is returned
    /// and no file is touched: a wrong key looks the same, and starting empty would
    /// silently rotate the good backups out. Pending migrations run after a backup.
    pub fn open(paths: StorePaths, key: DbKey, now: i64) -> Result<(Self, OpenReport)> {
        if let Some(parent) = paths.db.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::create_dir_all(&paths.backups)?;
        let mut report = OpenReport::default();

        let has_data = |p: &Path| fs::metadata(p).is_ok_and(|m| m.len() > 0);
        let needs_backup = if has_data(&paths.db) {
            match backup::check_keeping_wal(&paths, &key) {
                Ok(()) => false,
                Err(err) if unreadable(&err) => true,
                Err(err) => return Err(err),
            }
        } else {
            !backup::list(&paths.backups)?.is_empty()
        };
        if needs_backup {
            let good = backup::list(&paths.backups)?
                .into_iter()
                .find(|b| check(&b.path, &key).is_ok())
                .ok_or(StoreError::Unreadable)?;
            backup::replace_damaged(&paths, &key, &good.path, now)?;
            report.recovered_from = Some(good.path);
        }

        report.created = !has_data(&paths.db);
        let conn = open_keyed(&paths.db, &key)?;
        configure(&conn, report.created)?;
        let mut store = Self { conn, paths, key };
        let backup_first = !report.created;
        report.applied_migrations =
            store.migrate(now, backup_first, &mut report.pre_migration_backup)?;
        Ok((store, report))
    }

    /// Applies pending migrations (after a backup when `backup_first`) and seeds
    /// first-run settings.
    pub(crate) fn migrate(
        &mut self,
        now: i64,
        backup_first: bool,
        backup: &mut Option<PathBuf>,
    ) -> Result<Vec<u32>> {
        let pending = migrate::pending(&self.conn)?;
        let mut applied = Vec::new();
        if !pending.is_empty() {
            if backup_first {
                *backup = Some(self.backup_before_migration(now)?);
            }
            applied = migrate::apply(&mut self.conn, &pending, now)?;
        }
        settings::seed(&self.conn, now)?;
        Ok(applied)
    }

    pub fn paths(&self) -> &StorePaths {
        &self.paths
    }

    /// Runs `f` in a transaction (committed when it returns `Ok`).
    pub fn transaction<T>(&mut self, f: impl FnOnce(&Transaction<'_>) -> Result<T>) -> Result<T> {
        let tx = self.conn.transaction()?;
        let out = f(&tx)?;
        tx.commit()?;
        Ok(out)
    }

    /// Like [`Store::transaction`], committed with `synchronous = FULL` (and `fullfsync`
    /// on macOS): the sweep executor's write-before-close records must survive a power
    /// loss (Software Design §7).
    pub fn transaction_durable<T>(
        &mut self,
        f: impl FnOnce(&Transaction<'_>) -> Result<T>,
    ) -> Result<T> {
        self.conn.execute_batch("PRAGMA synchronous = FULL;")?;
        let out = self.transaction(f);
        let reset = self.conn.execute_batch("PRAGMA synchronous = NORMAL;");
        let out = out?;
        reset?;
        Ok(out)
    }

    /// Runs `f` with this store in one transaction, committed when it returns `Ok` (rolled
    /// back otherwise), so the repository methods can be used inside. `durable` commits with
    /// `synchronous = FULL` like [`Store::transaction_durable`].
    pub fn atomically<T>(
        &mut self,
        durable: bool,
        f: impl FnOnce(&Self) -> Result<T>,
    ) -> Result<T> {
        if durable {
            self.conn.execute_batch("PRAGMA synchronous = FULL;")?;
        }
        let out = (|| {
            let tx = self.conn.unchecked_transaction()?;
            let out = f(self)?;
            tx.commit()?;
            Ok(out)
        })();
        if durable {
            let reset = self.conn.execute_batch("PRAGMA synchronous = NORMAL;");
            let out = out?;
            reset?;
            return Ok(out);
        }
        out
    }

    /// Returns freed pages to the file system (monthly, at idle: DB Design §5).
    pub fn incremental_vacuum(&self) -> Result<()> {
        self.conn.execute_batch("PRAGMA incremental_vacuum;")?;
        Ok(())
    }

    /// Size of the database on disk (file plus WAL), in bytes.
    pub fn db_bytes(&self) -> Result<u64> {
        let wal = fs::metadata(self.paths.db_with("-wal")).map_or(0, |m| m.len());
        Ok(fs::metadata(&self.paths.db)?.len() + wal)
    }

    /// "Erase all data": closes the database and deletes it, every file next to it whose
    /// name starts with `decay.db` (WAL, damaged copies, temporary files) and every
    /// backup. The caller then deletes the key from the Keychain / Credential Manager
    /// (without it nothing left on disk can be read: the files are encrypted, so this
    /// doesn't rely on overwriting SSD blocks).
    pub fn erase(self) -> std::result::Result<(), EraseError> {
        let Self { conn, paths, key } = self;
        if let Err((conn, error)) = conn.close() {
            let store = Self { conn, paths, key };
            return Err(EraseError { store: Some(Box::new(store)), error: error.into() });
        }
        let fail = |e: std::io::Error| EraseError { store: None, error: e.into() };
        let remove = |p: &Path| match fs::remove_file(p) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        };
        if let (Some(dir), Some(name)) = (paths.db.parent(), paths.db.file_name()) {
            let name = name.to_string_lossy();
            for entry in fs::read_dir(dir).map_err(fail)? {
                let entry = entry.map_err(fail)?;
                let is_file = entry.file_type().is_ok_and(|t| t.is_file());
                if is_file && entry.file_name().to_string_lossy().starts_with(name.as_ref()) {
                    remove(&entry.path()).map_err(fail)?;
                }
            }
        }
        match fs::remove_dir_all(&paths.backups) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(fail(e)),
            _ => Ok(()),
        }
    }
}

/// A new `UUIDv7` id (sortable by time), as stored in `TEXT` id columns.
pub fn new_id(now: i64) -> String {
    let ms = u64::try_from(now).unwrap_or(0);
    let ts =
        uuid::Timestamp::from_unix(uuid::NoContext, ms / 1000, ((ms % 1000) * 1_000_000) as u32);
    uuid::Uuid::new_v7(ts).to_string()
}

/// `YYYYMMDD` of a UTC epoch-ms time (backup file names).
pub(crate) fn utc_date(ms: i64) -> String {
    // Howard Hinnant's civil-from-days.
    let days = ms.div_euclid(86_400_000);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}{month:02}{day:02}")
}

#[cfg(test)]
pub(crate) mod test_util {
    use super::*;

    pub const NOW: i64 = 1_793_750_400_000; // 2026-11-04T00:00:00Z

    pub fn key() -> DbKey {
        DbKey::from_bytes([7; 32])
    }

    /// A fresh store in a temp dir (kept alive by the returned guard).
    pub fn store() -> (Store, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let (store, _) = Store::open(StorePaths::in_dir(dir.path()), key(), NOW).unwrap();
        (store, dir)
    }
}

#[cfg(test)]
mod tests {
    use super::test_util::*;
    use super::*;

    #[test]
    fn atomically_commits_or_rolls_back_everything() {
        let (mut store, _dir) = store();
        let failed: Result<()> = store.atomically(true, |s| {
            s.set_setting("idle_cutoff_s", &120, NOW)?;
            Err(StoreError::NotFound)
        });
        assert!(failed.is_err());
        assert_eq!(store.setting::<u64>("idle_cutoff_s").unwrap(), Some(300));
        store.atomically(false, |s| s.set_setting("idle_cutoff_s", &120, NOW)).unwrap();
        assert_eq!(store.setting::<u64>("idle_cutoff_s").unwrap(), Some(120));
    }

    #[test]
    fn creates_an_encrypted_database_with_the_v1_schema() {
        let dir = tempfile::tempdir().unwrap();
        let paths = StorePaths::in_dir(dir.path());
        let (store, report) = Store::open(paths.clone(), key(), NOW).unwrap();
        assert!(report.created);
        assert_eq!(report.applied_migrations, vec![1, 2, 3, 4, 5, 6]);
        assert_eq!(report.pre_migration_backup, None);
        let journal: String =
            store.conn.query_row("PRAGMA journal_mode", [], |r| r.get(0)).unwrap();
        assert_eq!(journal, "wal");
        drop(store);

        // Encrypted: plain SQLite can't read it, and neither can another key.
        let header = fs::read(&paths.db).unwrap();
        assert!(!header.starts_with(b"SQLite format 3"));
        let other = DbKey::from_bytes([8; 32]);
        assert!(matches!(Store::open(paths.clone(), other, NOW), Err(StoreError::Unreadable)));

        // Reopening applies nothing.
        let (_, report) = Store::open(paths, key(), NOW).unwrap();
        assert!(!report.created);
        assert_eq!(report.applied_migrations.len(), 0);
    }

    #[test]
    fn a_wrong_key_changes_no_file() {
        let dir = tempfile::tempdir().unwrap();
        let paths = StorePaths::in_dir(dir.path());
        let (store, _) = Store::open(paths.clone(), key(), NOW).unwrap();
        store.backup_now(NOW).unwrap();
        drop(store);
        let before = fs::read(&paths.db).unwrap();
        let err = Store::open(paths.clone(), DbKey::from_bytes([9; 32]), NOW).unwrap_err();
        assert!(matches!(err, StoreError::Unreadable));
        assert_eq!(fs::read(&paths.db).unwrap(), before);
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2, "only decay.db and backups/");
    }

    #[test]
    fn refuses_a_database_from_a_newer_version() {
        let (store, dir) = store();
        store
            .conn
            .execute("INSERT INTO schema_migrations VALUES (99, 'future', 0, 'x')", [])
            .unwrap();
        drop(store);
        let err = Store::open(StorePaths::in_dir(dir.path()), key(), NOW).unwrap_err();
        assert!(matches!(err, StoreError::TooNew { found: 99, known: SCHEMA_VERSION }));
    }

    #[test]
    fn a_second_store_on_the_same_file_is_refused() {
        let (store, dir) = store();
        let second = Store::open(StorePaths::in_dir(dir.path()), key(), NOW);
        assert!(matches!(second, Err(StoreError::InUse)), "{second:?}");
        drop(store);
        Store::open(StorePaths::in_dir(dir.path()), key(), NOW).unwrap();
    }

    #[test]
    fn a_failed_migration_reports_briefly() {
        let (store, dir) = store();
        store.conn.execute("DELETE FROM schema_migrations", []).unwrap();
        drop(store);
        let err = Store::open(StorePaths::in_dir(dir.path()), key(), NOW).unwrap_err();
        let StoreError::Migration { version: 1, reason } = &err else { panic!("{err:?}") };
        assert!(reason.contains("already exists") && reason.len() < 120, "{reason}");
    }

    #[test]
    fn erase_removes_every_file() {
        let (store, dir) = store();
        store.backup_now(NOW).unwrap();
        // Leftovers of an interrupted recovery.
        for name in ["decay.db.restore.tmp", "decay.db-wal.damaged-1", "decay.db.tmp"] {
            fs::write(dir.path().join(name), b"x").unwrap();
        }
        store.erase().unwrap();
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn formats_utc_dates() {
        assert_eq!(utc_date(0), "19700101");
        assert_eq!(utc_date(NOW), "20261104");
        assert_eq!(utc_date(951_782_400_000), "20000229");
    }

    #[test]
    fn ids_are_time_ordered_uuids() {
        let (a, b) = (new_id(NOW), new_id(NOW + 1));
        assert_eq!(a.len(), 36);
        assert!(a < b);
    }
}
