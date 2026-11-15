//! Backups (DB Design §1, §5): online `sqlite3_backup` copies, encrypted with the same key.
//!
//! - `decay-YYYYMMDD.db`: daily (UTC date), 7 kept.
//! - `decay-premigrate-v<NNNN>-<ms>.db`: taken before a migration, 3 kept.
//! - `decay-prerestore-<ms>.db`: the data replaced by a restore, 3 kept.
//! - `decay-preimport-<ms>.db`: taken before importing a file (P2.5), 3 kept.
//!
//! Each copy is written to a temporary file, synced and renamed into place, so a crash never
//! leaves a half-written backup. Ages come from the file names (copy and sync tools change
//! modification times), newest first.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use rusqlite::Connection;
use rusqlite::backup::Backup;

use crate::migrate::{self, SCHEMA_VERSION};
use crate::{DbKey, Result, Store, StoreError, StorePaths, check, open_keyed, utc_date};

const DAY_MS: i64 = 86_400_000;

/// What a backup was taken for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackupKind {
    Daily,
    PreMigration,
    PreRestore,
    PreImport,
}

impl BackupKind {
    const fn kept(self) -> usize {
        match self {
            Self::Daily => 7,
            Self::PreMigration | Self::PreRestore | Self::PreImport => 3,
        }
    }
}

/// One backup file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupInfo {
    pub path: PathBuf,
    pub kind: BackupKind,
    /// When it was taken (epoch ms): from the name; a daily backup's is the start of its
    /// UTC day.
    pub at: i64,
    pub bytes: u64,
}

/// Kind and time of a backup file name, or `None` for other files.
fn parse_name(name: &str) -> Option<(BackupKind, i64)> {
    let stem = name.strip_prefix("decay-")?.strip_suffix(".db")?;
    if let Some(rest) = stem.strip_prefix("premigrate-v") {
        let (_, ms) = rest.split_once('-')?;
        return Some((BackupKind::PreMigration, ms.parse().ok()?));
    }
    if let Some(ms) = stem.strip_prefix("prerestore-") {
        return Some((BackupKind::PreRestore, ms.parse().ok()?));
    }
    if let Some(ms) = stem.strip_prefix("preimport-") {
        return Some((BackupKind::PreImport, ms.parse().ok()?));
    }
    if stem.len() != 8 || !stem.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let num = |r: std::ops::Range<usize>| stem[r].parse::<i64>().ok();
    let days = crate::search::days_from_civil(num(0..4)?, num(4..6)?, num(6..8)?);
    Some((BackupKind::Daily, days * DAY_MS))
}

/// Every backup, newest first (by the time in its name, then modification time).
pub(crate) fn list(dir: &Path) -> Result<Vec<BackupInfo>> {
    let mut out = Vec::new();
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
        Err(e) => return Err(e.into()),
    };
    let mut keyed = Vec::new();
    for entry in entries {
        let entry = entry?;
        let Some((kind, at)) = parse_name(&entry.file_name().to_string_lossy()) else {
            continue;
        };
        let meta = entry.metadata()?;
        if !meta.is_file() {
            continue;
        }
        let modified = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_millis());
        keyed.push((
            at.div_euclid(DAY_MS),
            modified,
            BackupInfo { path: entry.path(), kind, at, bytes: meta.len() },
        ));
    }
    keyed.sort_by_key(|k| std::cmp::Reverse((k.0, k.1)));
    out.extend(keyed.into_iter().map(|(_, _, info)| info));
    Ok(out)
}

/// `fs::rename`, retried briefly: on Windows an antivirus or indexer holding the file
/// open makes it fail with "access denied" for a moment.
fn rename(from: &Path, to: &Path) -> std::io::Result<()> {
    let mut tries = 0;
    loop {
        match fs::rename(from, to) {
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied && tries < 20 => {
                tries += 1;
                std::thread::sleep(Duration::from_millis(50));
            }
            other => return other,
        }
    }
}

/// Makes a rename durable (Unix; on Windows the rename is already).
fn sync_dir(dir: &Path) {
    #[cfg(unix)]
    if let Ok(d) = fs::File::open(dir) {
        let _ = d.sync_all();
    }
    #[cfg(not(unix))]
    let _ = dir;
}

/// Copies `src` into a new encrypted file at `dest` (atomically replacing it). The copy
/// uses a rollback journal, so checking or reading it never creates WAL files.
fn copy_to(src: &Connection, dest: &Path, key: &DbKey) -> Result<()> {
    let mut tmp = dest.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    let _ = fs::remove_file(&tmp);
    {
        let mut copy = Connection::open(&tmp)?;
        copy.execute_batch(&key.pragma())?;
        Backup::new(src, &mut copy)?.run_to_completion(512, Duration::ZERO, None)?;
        copy.execute_batch("PRAGMA journal_mode = DELETE;")?;
        copy.close().map_err(|(_, e)| e)?;
    }
    // Write access: Windows' FlushFileBuffers needs it (a read-only handle is refused).
    fs::OpenOptions::new().write(true).open(&tmp)?.sync_all()?;
    rename(&tmp, dest)?;
    if let Some(dir) = dest.parent() {
        sync_dir(dir);
    }
    Ok(())
}

/// Deletes the oldest backups of `kind` beyond what's kept.
fn prune(dir: &Path, kind: BackupKind) -> Result<()> {
    let all: Vec<BackupInfo> = list(dir)?.into_iter().filter(|b| b.kind == kind).collect();
    for old in all.into_iter().skip(kind.kept()) {
        fs::remove_file(old.path)?;
    }
    Ok(())
}

/// Checks the database, keeping a copy of its WAL until the check passes: opening a
/// damaged database can drop the WAL, and with it the newest writes, which belong in the
/// damaged copy kept for recovery tools.
pub(crate) fn check_keeping_wal(paths: &StorePaths, key: &DbKey) -> Result<()> {
    let wal = paths.db_with("-wal");
    let kept = paths.db_with("-wal.precheck");
    let has_wal = fs::metadata(&wal).is_ok_and(|m| m.len() > 0);
    if has_wal {
        fs::copy(&wal, &kept)?;
    }
    let result = check(&paths.db, key);
    let damaged = matches!(&result, Err(e) if crate::unreadable(e));
    if !damaged {
        let _ = fs::remove_file(&kept);
    }
    result
}

/// Replaces a damaged or missing database with `backup`. The backup is copied next to the
/// database first; only then is the damaged file (and its WAL) moved aside as
/// `decay.db.damaged-<now>` and the copy renamed into place. A crash at any point leaves
/// either the old files or a missing `decay.db` with the backup intact, which the next
/// open recovers again.
pub(crate) fn replace_damaged(
    paths: &StorePaths,
    key: &DbKey,
    backup: &Path,
    now: i64,
) -> Result<()> {
    let staged = paths.db_with(".restore.tmp");
    {
        let src = open_keyed(backup, key)?;
        copy_to(&src, &staged, key)?;
    }
    let aside = |p: &Path, suffix: &str| {
        let mut name = p.as_os_str().to_owned();
        name.push(format!("{suffix}.damaged-{now}"));
        PathBuf::from(name)
    };
    if paths.db.exists() {
        rename(&paths.db, &aside(&paths.db, ""))?;
    }
    for side in [paths.db_with("-wal"), paths.db_with("-shm")] {
        if side.exists() {
            rename(&side, &aside(&side, ""))?;
        }
    }
    let kept_wal = paths.db_with("-wal.precheck");
    if kept_wal.exists() {
        rename(&kept_wal, &aside(&paths.db, "-wal"))?;
    }
    rename(&staged, &paths.db)?;
    if let Some(dir) = paths.db.parent() {
        sync_dir(dir);
    }
    Ok(())
}

impl Store {
    /// Writes today's daily backup (replacing an earlier one from the same UTC day) and
    /// keeps the newest 7.
    pub fn backup_now(&self, now: i64) -> Result<PathBuf> {
        let dest = self.paths.backups.join(format!("decay-{}.db", utc_date(now)));
        copy_to(&self.conn, &dest, &self.key)?;
        prune(&self.paths.backups, BackupKind::Daily)?;
        Ok(dest)
    }

    /// A copy of the data before a file import (`decay-preimport-<now>.db`, 3 kept).
    pub fn backup_before_import(&self, now: i64) -> Result<PathBuf> {
        let dest = self.paths.backups.join(format!("decay-preimport-{now}.db"));
        copy_to(&self.conn, &dest, &self.key)?;
        prune(&self.paths.backups, BackupKind::PreImport)?;
        Ok(dest)
    }

    /// Whether today's (UTC) daily backup is still missing.
    pub fn backup_due(&self, now: i64) -> bool {
        !self.paths.backups.join(format!("decay-{}.db", utc_date(now))).exists()
    }

    pub(crate) fn backup_before_migration(&self, now: i64) -> Result<PathBuf> {
        let version = migrate::current_version(&self.conn)?;
        let dest = self.paths.backups.join(format!("decay-premigrate-v{version:04}-{now}.db"));
        copy_to(&self.conn, &dest, &self.key)?;
        prune(&self.paths.backups, BackupKind::PreMigration)?;
        Ok(dest)
    }

    /// Every backup, newest first (`get_storage_info`).
    pub fn backups(&self) -> Result<Vec<BackupInfo>> {
        list(&self.paths.backups)
    }

    /// Replaces the current data with a backup from the backups folder, given by file name
    /// (`restore_backup`). The backup is checked first and must not be from a newer
    /// version; the current data is saved as a `decay-prerestore-<now>.db` backup (whose
    /// path is returned), and an older backup is migrated after the restore. On failure
    /// before the copy, the current data is unchanged.
    pub fn restore_backup(&mut self, file_name: &str, now: i64) -> Result<PathBuf> {
        let name = Path::new(file_name).file_name().ok_or(StoreError::NotFound)?;
        let path = self.paths.backups.join(name);
        if parse_name(&name.to_string_lossy()).is_none() || !path.is_file() {
            return Err(StoreError::NotFound);
        }
        check(&path, &self.key)?;
        let src = open_keyed(&path, &self.key)?;
        let version = migrate::current_version(&src)?;
        if version > SCHEMA_VERSION {
            return Err(StoreError::TooNew { found: version, known: SCHEMA_VERSION });
        }
        let safety = self.paths.backups.join(format!("decay-prerestore-{now}.db"));
        copy_to(&self.conn, &safety, &self.key)?;
        prune(&self.paths.backups, BackupKind::PreRestore)?;

        Backup::new(&src, &mut self.conn)?.run_to_completion(512, Duration::ZERO, None)?;
        drop(src);
        self.conn.execute_batch("PRAGMA journal_mode = WAL;")?;
        self.migrate(now, false, &mut None)?;
        Ok(safety)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::*;

    fn count_rules(store: &Store) -> i64 {
        store.conn.query_row("SELECT count(*) FROM rules", [], |r| r.get(0)).unwrap()
    }

    fn names(store: &Store) -> Vec<String> {
        store
            .backups()
            .unwrap()
            .iter()
            .map(|b| b.path.file_name().unwrap().to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn keeps_seven_daily_backups() {
        let (store, _dir) = store();
        assert!(store.backup_due(NOW));
        for day in 0..10 {
            store.backup_now(NOW + day * DAY_MS).unwrap();
        }
        assert!(!store.backup_due(NOW + 9 * DAY_MS));
        let names = names(&store);
        assert_eq!(names.len(), 7);
        assert_eq!(names[0], "decay-20261113.db", "newest first");
        assert!(!names.contains(&"decay-20261106.db".to_owned()));
        assert!(!store.paths.backups.join("decay-20261113.db.tmp").exists());
    }

    #[test]
    fn orders_and_prunes_by_the_time_in_the_name() {
        let (store, _dir) = store();
        for (version, at) in [(2, 1), (9, 2), (10, 3), (11, 4)] {
            let name = format!("decay-premigrate-v{version:04}-{}.db", NOW + at);
            store.backup_now(NOW).unwrap();
            fs::rename(
                store.paths.backups.join("decay-20261104.db"),
                store.paths.backups.join(name),
            )
            .unwrap();
        }
        prune(&store.paths.backups, BackupKind::PreMigration).unwrap();
        let names = names(&store);
        assert_eq!(names.len(), 3);
        assert!(names[0].starts_with("decay-premigrate-v0011-"), "{names:?}");
        assert!(!names.iter().any(|n| n.contains("v0002")));
        assert_eq!(parse_name("decay-20261104.db"), Some((BackupKind::Daily, NOW)));
        assert_eq!(parse_name("decay-preimport-5.db"), Some((BackupKind::PreImport, 5)));
        assert_eq!(parse_name("decay-2026110.db"), None);
        assert_eq!(parse_name("notes.db"), None);
    }

    #[test]
    fn restore_saves_the_current_data_first() {
        let (mut store, _dir) = store();
        let before = count_rules(&store);
        let backup = store.backup_now(NOW).unwrap();
        store.conn.execute("DELETE FROM rules", []).unwrap();
        let name = backup.file_name().unwrap().to_string_lossy().into_owned();
        let safety = store.restore_backup(&name, NOW + 1).unwrap();
        assert_eq!(count_rules(&store), before);
        assert!(safety.ends_with(format!("decay-prerestore-{}.db", NOW + 1)));

        // The safety copy holds the replaced data: restoring it undoes the restore.
        let safety_name = safety.file_name().unwrap().to_string_lossy().into_owned();
        store.restore_backup(&safety_name, NOW + 2).unwrap();
        assert_eq!(count_rules(&store), 0);

        // Only files in the backups folder, by name.
        for bad in ["/etc/hosts", "../decay.db", "decay-nope.db"] {
            assert!(matches!(store.restore_backup(bad, NOW), Err(StoreError::NotFound)), "{bad}");
        }
        // And the store stays usable.
        store.backup_now(NOW + 3).unwrap();
    }

    #[test]
    fn restoring_an_older_schema_migrates_it() {
        let (mut store, _dir) = store();
        // A pre-V1 backup: only the migrations table.
        let old = store.paths.backups.join(format!("decay-premigrate-v0000-{NOW}.db"));
        {
            let conn = open_keyed(&old, &key()).unwrap();
            conn.execute_batch(
                "CREATE TABLE schema_migrations (version INTEGER PRIMARY KEY, name TEXT NOT NULL,
                   applied_on INTEGER NOT NULL, checksum TEXT NOT NULL);",
            )
            .unwrap();
        }
        let name = old.file_name().unwrap().to_string_lossy().into_owned();
        store.restore_backup(&name, NOW + 1).unwrap();
        assert!(count_rules(&store) > 0, "V1 applied again, with its default rules");
        assert_eq!(store.mode().unwrap(), decay_core::Mode::Shadow);
    }

    #[test]
    fn refuses_to_restore_a_backup_from_a_newer_version() {
        let (mut store, _dir) = store();
        let backup = store.backup_now(NOW).unwrap();
        {
            let conn = open_keyed(&backup, &key()).unwrap();
            conn.execute("INSERT INTO schema_migrations VALUES (99, 'future', 0, 'x')", [])
                .unwrap();
        }
        let name = backup.file_name().unwrap().to_string_lossy().into_owned();
        assert!(matches!(
            store.restore_backup(&name, NOW),
            Err(StoreError::TooNew { found: 99, .. })
        ));
        assert!(count_rules(&store) > 0);
    }

    #[test]
    fn a_damaged_database_is_replaced_by_the_newest_good_backup() {
        let dir = tempfile::tempdir().unwrap();
        let paths = StorePaths::in_dir(dir.path());
        let (store, _) = Store::open(paths.clone(), key(), NOW).unwrap();
        store.conn.execute("DELETE FROM rules WHERE source = 'default'", []).unwrap();
        store.backup_now(NOW).unwrap();
        drop(store);
        // A newer but broken backup is skipped.
        fs::write(paths.backups.join("decay-20991231.db"), b"garbage").unwrap();
        // Damage the main file's pages (keep its size).
        let mut bytes = fs::read(&paths.db).unwrap();
        for b in bytes.iter_mut().skip(100) {
            *b ^= 0x5a;
        }
        fs::write(&paths.db, &bytes).unwrap();

        let (store, report) = Store::open(paths.clone(), key(), NOW + 1).unwrap();
        assert_eq!(report.recovered_from, Some(paths.backups.join("decay-20261104.db")));
        assert_eq!(count_rules(&store), 0, "the backup's data");
        let damaged = dir.path().join(format!("decay.db.damaged-{}", NOW + 1));
        assert_eq!(fs::read(damaged).unwrap(), bytes, "the damaged file is kept");
        assert!(!paths.db_with(".restore.tmp").exists());
    }

    #[test]
    fn a_missing_or_empty_database_is_restored_not_recreated() {
        for damage in ["delete", "empty", "crash before the final rename"] {
            let dir = tempfile::tempdir().unwrap();
            let paths = StorePaths::in_dir(dir.path());
            let (store, _) = Store::open(paths.clone(), key(), NOW).unwrap();
            store.conn.execute("DELETE FROM rules WHERE id = 'default-zoom'", []).unwrap();
            store.backup_now(NOW).unwrap();
            drop(store);
            match damage {
                "delete" => fs::remove_file(&paths.db).unwrap(),
                "empty" => fs::write(&paths.db, b"").unwrap(),
                _ => {
                    // replace_damaged moved the file aside but never renamed the copy in.
                    fs::rename(&paths.db, paths.db_with(".damaged-1")).unwrap();
                    fs::write(paths.db_with(".restore.tmp"), b"partial").unwrap();
                }
            }
            let (store, report) = Store::open(paths.clone(), key(), NOW + 1).unwrap();
            assert!(!report.created, "{damage}");
            assert!(report.recovered_from.is_some(), "{damage}");
            let zoom: i64 = store
                .conn
                .query_row("SELECT count(*) FROM rules WHERE id = 'default-zoom'", [], |r| r.get(0))
                .unwrap();
            assert_eq!(zoom, 0, "{damage}: the backup's data, not a fresh database");
        }
    }

    #[test]
    fn missing_database_and_unreadable_backups_is_an_error_not_a_fresh_start() {
        let dir = tempfile::tempdir().unwrap();
        let paths = StorePaths::in_dir(dir.path());
        let (store, _) = Store::open(paths.clone(), key(), NOW).unwrap();
        store.backup_now(NOW).unwrap();
        drop(store);
        fs::remove_file(&paths.db).unwrap();
        let other = DbKey::from_bytes([3; 32]);
        assert!(matches!(Store::open(paths.clone(), other, NOW), Err(StoreError::Unreadable)));
        assert!(!paths.db.exists());
    }

    #[test]
    fn migrating_an_existing_database_backs_it_up_first() {
        // A database from before V1: only the migrations table exists.
        let dir = tempfile::tempdir().unwrap();
        let paths = StorePaths::in_dir(dir.path());
        {
            let conn = open_keyed(&paths.db, &key()).unwrap();
            conn.execute_batch(
                "CREATE TABLE schema_migrations (version INTEGER PRIMARY KEY, name TEXT NOT NULL,
                   applied_on INTEGER NOT NULL, checksum TEXT NOT NULL);",
            )
            .unwrap();
        }
        let (_, report) = Store::open(paths, key(), NOW).unwrap();
        assert!(!report.created);
        assert_eq!(report.applied_migrations, vec![1, 2]);
        let backup = report.pre_migration_backup.unwrap();
        assert_eq!(
            backup.file_name().unwrap().to_string_lossy(),
            format!("decay-premigrate-v0000-{NOW}.db")
        );
    }
}
