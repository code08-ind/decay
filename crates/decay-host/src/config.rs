//! The host's settings, written by the app when it installs the host (first run, and
//! after updates): `<support dir>/host/host-config.json`.

use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Extension IDs always allowed (the published store builds). Development builds are
/// allowed through [`HostConfig::allowed_origins`].
// TODO(P0.9 publish): add the Chrome Web Store / Edge Add-ons IDs once assigned.
pub const BUILTIN_EXTENSION_IDS: &[&str] = &[];

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostConfig {
    /// `chrome-extension://<id>/` origins allowed besides the built-in ones.
    #[serde(default)]
    pub allowed_origins: Vec<String>,
    /// The app to start when it isn't running (Windows; macOS uses the bundle id).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_path: Option<PathBuf>,
}

/// `<support dir>/host/`.
pub fn host_dir(support_dir: &Path) -> PathBuf {
    support_dir.join("host")
}

impl HostConfig {
    pub fn path(support_dir: &Path) -> PathBuf {
        host_dir(support_dir).join("host-config.json")
    }

    /// Reads the config; a missing or unreadable file is an empty config (only the
    /// built-in IDs are allowed then: the safe direction).
    pub fn load(support_dir: &Path) -> Self {
        std::fs::read(Self::path(support_dir))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    /// Writes the config atomically.
    pub fn save(&self, support_dir: &Path) -> io::Result<()> {
        let json = serde_json::to_vec_pretty(self).map_err(io::Error::other)?;
        crate::install::write_atomic(&Self::path(support_dir), &json)
    }

    /// Every allowed origin: the built-in IDs and the configured ones.
    pub fn allowed(&self) -> Vec<String> {
        BUILTIN_EXTENSION_IDS
            .iter()
            .map(|id| origin_of(id))
            .chain(self.allowed_origins.iter().cloned())
            .collect()
    }
}

/// `chrome-extension://<id>/` for an extension id.
pub fn origin_of(id: &str) -> String {
    format!("chrome-extension://{id}/")
}

/// Whether `origin` is a well-formed extension origin (`chrome-extension://` + 32
/// letters a–p + `/`) that is in `allowed`.
pub fn origin_allowed(origin: &str, allowed: &[String]) -> bool {
    let well_formed = origin
        .strip_prefix("chrome-extension://")
        .and_then(|rest| rest.strip_suffix('/'))
        .is_some_and(|id| id.len() == 32 && id.bytes().all(|b| (b'a'..=b'p').contains(&b)));
    well_formed && allowed.iter().any(|a| a == origin)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "abcdefghijklmnopabcdefghijklmnop";

    #[test]
    fn only_listed_well_formed_origins_are_allowed() {
        let allowed = vec![origin_of(ID)];
        assert!(origin_allowed(&origin_of(ID), &allowed));
        assert!(!origin_allowed(&origin_of(&ID.replace('a', "b")), &allowed));
        assert!(!origin_allowed(
            &format!("chrome-extension://{ID}"),
            &[format!("chrome-extension://{ID}")]
        ));
        assert!(!origin_allowed(
            "chrome-extension://short/",
            &["chrome-extension://short/".into()]
        ));
        assert!(!origin_allowed(&format!("https://{ID}/"), &[format!("https://{ID}/")]));
        assert!(!origin_allowed("", &[String::new()]));
    }

    #[test]
    fn config_round_trips_and_defaults_to_nothing() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(HostConfig::load(dir.path()), HostConfig::default());
        let config =
            HostConfig { allowed_origins: vec![origin_of(ID)], app_path: Some("/x".into()) };
        config.save(dir.path()).unwrap();
        assert_eq!(HostConfig::load(dir.path()), config);
        std::fs::write(HostConfig::path(dir.path()), b"{oops").unwrap();
        assert_eq!(HostConfig::load(dir.path()), HostConfig::default());
    }
}
