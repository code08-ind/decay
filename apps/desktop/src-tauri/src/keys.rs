//! The database key (Software Design §5.2, §6): 32 random bytes, kept in the macOS
//! Keychain / Windows Credential Manager under the app's identity, created on first run.
//!
//! The rule that matters: **a missing key never replaces existing data.** If the key is
//! gone but a database exists (a Keychain reset, a restore from another Mac), the app
//! stops with an error instead of creating a new key: a new key can't open the old data,
//! and the old data must stay untouched for a recovery path.
//!
//! Debug builds keep a key file next to their own data instead (`dev-app/`), so
//! rebuilding (which changes the code signature) doesn't trigger Keychain prompts.
//! `DECAY_USE_KEYCHAIN=1` makes a debug build use the Keychain, to test that path.

use std::path::{Path, PathBuf};

use decay_store::{DbKey, StorePaths};

/// The Keychain item: service and account.
pub const SERVICE: &str = "com.decay.app";
pub const ACCOUNT: &str = "database-key";

/// Where the key lives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeySource {
    /// Keychain / Credential Manager.
    Keychain,
    /// A file holding the key as hex (debug builds, tests).
    File(PathBuf),
}

#[derive(Debug, thiserror::Error)]
pub enum KeyError {
    #[error(
        "Decay's data is locked: its key is missing from the {0}. Your data hasn't been changed."
    )]
    Missing(&'static str),
    #[error("the stored key is damaged")]
    Damaged,
    #[error("the {store} can't be used: {why}")]
    Store { store: &'static str, why: String },
    #[error("no random numbers: {0}")]
    Random(String),
}

impl KeySource {
    fn name(&self) -> &'static str {
        match self {
            Self::Keychain if cfg!(windows) => "Credential Manager",
            Self::Keychain => "Keychain",
            Self::File(_) => "key file",
        }
    }

    fn read(&self) -> Result<Option<String>, KeyError> {
        let store = self.name();
        match self {
            Self::Keychain => {
                let entry = keychain_entry()?;
                match entry.get_password() {
                    Ok(hex) => Ok(Some(hex)),
                    Err(keyring::Error::NoEntry) => Ok(None),
                    Err(e) => Err(KeyError::Store { store, why: e.to_string() }),
                }
            }
            Self::File(path) => match std::fs::read_to_string(path) {
                Ok(hex) => Ok(Some(hex)),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(e) => Err(KeyError::Store { store, why: e.to_string() }),
            },
        }
    }

    fn write(&self, hex: &str) -> Result<(), KeyError> {
        let store = self.name();
        let failed = |why: String| KeyError::Store { store, why };
        match self {
            Self::Keychain => {
                keychain_entry()?.set_password(hex).map_err(|e| failed(e.to_string()))
            }
            Self::File(path) => {
                if let Some(dir) = path.parent() {
                    std::fs::create_dir_all(dir).map_err(|e| failed(e.to_string()))?;
                }
                write_private(path, hex).map_err(|e| failed(e.to_string()))
            }
        }
    }
}

fn keychain_entry() -> Result<keyring::Entry, KeyError> {
    keyring::Entry::new(SERVICE, ACCOUNT)
        .map_err(|e| KeyError::Store { store: "Keychain", why: e.to_string() })
}

/// Writes the file readable by its owner only.
fn write_private(path: &Path, contents: &str) -> std::io::Result<()> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(path)?.write_all(contents.as_bytes())
}

/// Data the key may belong to: the database, its WAL, or any backup (a missing
/// `decay.db` is recovered from the backups, which need the old key).
fn data_exists(paths: &StorePaths) -> bool {
    let mut wal = paths.db.as_os_str().to_owned();
    wal.push("-wal");
    let backups = std::fs::read_dir(&paths.backups).is_ok_and(|mut d| d.next().is_some());
    paths.db.exists() || Path::new(&wal).exists() || backups
}

/// The key for the database at `paths`: read from `source`, or created there when no
/// data exists yet.
pub fn load_or_create(source: &KeySource, paths: &StorePaths) -> Result<DbKey, KeyError> {
    if let Some(hex) = source.read()? {
        return DbKey::from_hex(hex.trim()).ok_or(KeyError::Damaged);
    }
    if data_exists(paths) {
        return Err(KeyError::Missing(source.name()));
    }
    let key = DbKey::generate().map_err(|e| KeyError::Random(e.to_string()))?;
    source.write(&key.to_hex())?;
    // Read back: a store that silently drops writes must not leave a database nobody can
    // open.
    match source.read()? {
        Some(hex) if hex.trim() == key.to_hex() => Ok(key),
        _ => Err(KeyError::Store { store: source.name(), why: "the key didn't read back".into() }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_a_key_once_and_reads_it_back() {
        let dir = tempfile::tempdir().unwrap();
        let source = KeySource::File(dir.path().join("keys/db.key"));
        let paths = StorePaths::in_dir(dir.path());
        let first = load_or_create(&source, &paths).unwrap();
        std::fs::write(&paths.db, b"data").unwrap();
        let again = load_or_create(&source, &paths).unwrap();
        assert_eq!(first.to_hex(), again.to_hex());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode =
                std::fs::metadata(dir.path().join("keys/db.key")).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn a_missing_key_never_replaces_existing_data() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db.key");
        let source = KeySource::File(path.clone());
        let paths = StorePaths::in_dir(dir.path());
        std::fs::write(&paths.db, b"data").unwrap();
        let err = load_or_create(&source, &paths).unwrap_err();
        assert!(matches!(err, KeyError::Missing("key file")), "{err}");
        assert!(!path.exists(), "no new key was written");
        assert_eq!(std::fs::read(&paths.db).unwrap(), b"data");
    }

    #[test]
    fn backups_or_a_wal_alone_are_data_too() {
        let dir = tempfile::tempdir().unwrap();
        let paths = StorePaths::in_dir(dir.path());
        let source = KeySource::File(dir.path().join("db.key"));
        std::fs::create_dir_all(&paths.backups).unwrap();
        // An empty backups folder is not data.
        assert!(!data_exists(&paths));
        std::fs::write(paths.backups.join("decay-1.db"), b"old").unwrap();
        assert!(matches!(load_or_create(&source, &paths), Err(KeyError::Missing(_))));
        std::fs::remove_file(paths.backups.join("decay-1.db")).unwrap();
        std::fs::write(dir.path().join("decay.db-wal"), b"wal").unwrap();
        assert!(matches!(load_or_create(&source, &paths), Err(KeyError::Missing(_))));
        assert!(!dir.path().join("db.key").exists());
    }

    #[test]
    fn a_damaged_key_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db.key");
        std::fs::write(&path, "not hex").unwrap();
        let err =
            load_or_create(&KeySource::File(path), &StorePaths::in_dir(dir.path())).unwrap_err();
        assert!(matches!(err, KeyError::Damaged));
    }
}
