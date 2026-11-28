//! The note before the Keychain prompt after an update (macOS).
//!
//! Until Decay is signed with an Apple Developer ID, every build is new to the Keychain:
//! the database key's item lists allowed builds by their `cdhash` (an app without a Team
//! ID gets a `cdhash:` partition), so macOS asks for the login password after each
//! update. Decay says so first, once per new version, so the prompt isn't a surprise.
//! A fresh install doesn't ask (Decay creates the item), so it gets no note.

use std::path::{Path, PathBuf};

use decay_store::StorePaths;

use crate::core::DataDir;
use crate::keys::KeySource;

/// The last version that started with this data, next to the database.
const FILE: &str = "last-version";

fn file(data: &Path) -> PathBuf {
    data.join(FILE)
}

/// Whether to show the note before opening the data.
pub fn due(dir: &DataDir, version: &str) -> bool {
    // Debug builds can force it, to see the note (`DECAY_SHOW_UPDATE_NOTE=1`).
    if cfg!(debug_assertions)
        && std::env::var_os("DECAY_SHOW_UPDATE_NOTE").is_some_and(|v| v == "1")
    {
        return true;
    }
    should_show(
        cfg!(target_os = "macos"),
        dir.key == KeySource::Keychain,
        StorePaths::in_dir(&dir.data).db.exists(),
        last(&dir.data).as_deref(),
        version,
    )
}

fn should_show(mac: bool, keychain: bool, has_data: bool, last: Option<&str>, now: &str) -> bool {
    mac && keychain && has_data && last != Some(now)
}

fn last(data: &Path) -> Option<String> {
    let text = std::fs::read_to_string(file(data)).ok()?;
    let v = text.trim();
    (!v.is_empty()).then(|| v.to_owned())
}

/// Records `version` once the data opened (so a failed start shows the note again).
pub fn remember(data: &Path, version: &str) {
    if let Err(e) = std::fs::write(file(data), version) {
        tracing::warn!("last version not saved: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_after_an_update_of_existing_keychain_data_on_a_mac() {
        // An update (or the first version that records itself) with data: the note.
        assert!(should_show(true, true, true, Some("0.1.5"), "0.1.6"));
        assert!(should_show(true, true, true, None, "0.1.6"));
        // Same version again, a fresh install, a key file, Windows: no note.
        assert!(!should_show(true, true, true, Some("0.1.6"), "0.1.6"));
        assert!(!should_show(true, true, false, None, "0.1.6"));
        assert!(!should_show(true, false, true, None, "0.1.6"));
        assert!(!should_show(false, true, true, None, "0.1.6"));
    }

    #[test]
    fn remembers_the_version_next_to_the_data() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(last(dir.path()), None);
        remember(dir.path(), "0.1.6");
        assert_eq!(last(dir.path()).as_deref(), Some("0.1.6"));
        std::fs::write(file(dir.path()), "  \n").unwrap();
        assert_eq!(last(dir.path()), None);
    }

    #[test]
    fn due_reads_the_data_folder() {
        let support = tempfile::tempdir().unwrap();
        let dir = DataDir {
            support: support.path().to_owned(),
            data: support.path().to_owned(),
            key: KeySource::Keychain,
        };
        // No database yet: a fresh install.
        assert!(!due(&dir, "0.1.6"));
        std::fs::write(StorePaths::in_dir(&dir.data).db, b"").unwrap();
        assert_eq!(due(&dir, "0.1.6"), cfg!(target_os = "macos"));
        remember(&dir.data, "0.1.6");
        assert!(!due(&dir, "0.1.6"));
    }
}
