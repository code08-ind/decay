//! Native messaging host manifests (Software Design §5.2 step 2): one
//! `com.decay.host.json` per detected Chromium browser, so it can launch `decay-host`.
//!
//! - macOS: `~/Library/Application Support/<browser>/NativeMessagingHosts/` (user level,
//!   no admin rights). A browser counts as detected when its support folder exists.
//! - Windows: the manifest lives in `<support dir>\host\`, and each detected browser
//!   (its `HKCU\Software\<vendor>` key exists) gets
//!   `HKCU\Software\<vendor>\NativeMessagingHosts\com.decay.host` pointing at it.
//! - Linux (development only): `~/.config/<browser>/NativeMessagingHosts/`.
//!
//! Arc's folder is from its documentation and not yet verified on a machine (Software
//! Design §10 spike 4).

use std::io;
use std::path::{Path, PathBuf};

use decay_bridge::HOST_NAME;
use serde_json::json;

#[cfg(windows)]
use crate::config::host_dir;
use crate::config::{HostConfig, origin_allowed};

/// A Chromium browser Decay can install its host for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BrowserTarget {
    pub name: &'static str,
    /// macOS: under `~/Library/Application Support/`.
    pub mac_dir: Option<&'static str>,
    /// Windows: under `HKCU\Software\`.
    pub win_key: Option<&'static str>,
    /// Linux: under `~/.config/`.
    pub linux_dir: Option<&'static str>,
}

pub const TARGETS: &[BrowserTarget] = &[
    BrowserTarget {
        name: "Google Chrome",
        mac_dir: Some("Google/Chrome"),
        win_key: Some(r"Google\Chrome"),
        linux_dir: Some("google-chrome"),
    },
    BrowserTarget {
        name: "Google Chrome Beta",
        mac_dir: Some("Google/Chrome Beta"),
        win_key: None,
        linux_dir: Some("google-chrome-beta"),
    },
    BrowserTarget {
        name: "Google Chrome Canary",
        mac_dir: Some("Google/Chrome Canary"),
        win_key: None,
        linux_dir: None,
    },
    BrowserTarget {
        name: "Chromium",
        mac_dir: Some("Chromium"),
        win_key: Some("Chromium"),
        linux_dir: Some("chromium"),
    },
    BrowserTarget {
        name: "Microsoft Edge",
        mac_dir: Some("Microsoft Edge"),
        win_key: Some(r"Microsoft\Edge"),
        linux_dir: Some("microsoft-edge"),
    },
    BrowserTarget {
        name: "Brave",
        mac_dir: Some("BraveSoftware/Brave-Browser"),
        win_key: Some(r"BraveSoftware\Brave-Browser"),
        linux_dir: Some("BraveSoftware/Brave-Browser"),
    },
    BrowserTarget { name: "Arc", mac_dir: Some("Arc/User Data"), win_key: None, linux_dir: None },
];

/// What [`install`] needs.
#[derive(Debug, Clone)]
pub struct InstallOptions {
    /// The user's home folder.
    pub home: PathBuf,
    /// The app's support folder (for the host config, and the Windows manifest).
    pub support_dir: PathBuf,
    /// The installed `decay-host` executable (absolute).
    pub host_path: PathBuf,
    /// Allowed `chrome-extension://<id>/` origins (besides the built-in store IDs).
    pub origins: Vec<String>,
    /// The app executable, for the host to start it on Windows.
    pub app_path: Option<PathBuf>,
    /// Windows: the registry path under `HKCU` holding the browser keys (`Software`;
    /// tests use a scratch key).
    pub registry_root: String,
}

/// A browser the host was installed for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    pub browser: &'static str,
    /// The manifest file (macOS / Linux) or registry key (Windows) written.
    pub location: String,
}

/// What [`install`] / [`uninstall`] did: one browser failing doesn't stop the others.
#[derive(Debug, Default)]
pub struct Report {
    pub done: Vec<Installed>,
    pub failed: Vec<(&'static str, io::Error)>,
}

/// The manifest JSON.
pub fn manifest(host_path: &Path, origins: &[String]) -> serde_json::Value {
    json!({
        "name": HOST_NAME,
        "description": "Decay: connects the browser extension to the Decay app",
        "path": host_path,
        "type": "stdio",
        "allowed_origins": origins,
    })
}

/// Writes `bytes` to `path` via a new temporary file (unique name, never an existing file
/// or link) synced to disk, then a rename (creating the folder).
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::io::Write as _;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(format!(".tmp-{}-{}", std::process::id(), unique()));
    let tmp = PathBuf::from(tmp);
    let result = (|| {
        let mut f = std::fs::OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

fn unique() -> u128 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos())
}

/// A host path that won't last: a disk image, or macOS's quarantine copy of an app run
/// from Downloads ("App Translocation"). A manifest pointing there breaks later.
pub fn transient_path(path: &Path) -> bool {
    let s = path.to_string_lossy();
    s.contains("/AppTranslocation/") || s.starts_with("/Volumes/")
}

fn manifest_bytes(opts: &InstallOptions) -> io::Result<Vec<u8>> {
    let invalid = |why: String| io::Error::new(io::ErrorKind::InvalidInput, why);
    if !opts.host_path.is_absolute() {
        return Err(invalid("the host path must be absolute".into()));
    }
    if transient_path(&opts.host_path) {
        return Err(invalid(
            "Decay runs from a disk image or Downloads: move it to Applications first".into(),
        ));
    }
    if let Some(bad) = opts.origins.iter().find(|o| !origin_allowed(o, std::slice::from_ref(o))) {
        return Err(invalid(format!("not an extension origin: {bad:?}")));
    }
    let config =
        HostConfig { allowed_origins: opts.origins.clone(), app_path: opts.app_path.clone() };
    let mut origins = config.allowed();
    origins.sort();
    origins.dedup();
    if origins.is_empty() {
        return Err(invalid("no extension is allowed to connect".into()));
    }
    config.save(&opts.support_dir)?;
    serde_json::to_vec_pretty(&manifest(&opts.host_path, &origins)).map_err(io::Error::other)
}

/// Folder holding the browser's `NativeMessagingHosts` (macOS / Linux).
#[cfg(not(windows))]
fn browser_dir(home: &Path, target: &BrowserTarget) -> Option<PathBuf> {
    if cfg!(target_os = "macos") {
        target.mac_dir.map(|d| home.join("Library/Application Support").join(d))
    } else {
        target.linux_dir.map(|d| home.join(".config").join(d))
    }
}

/// Installs the host for every detected browser (and writes the host config). Cheap and
/// idempotent: the app runs it at every start, so a browser installed later, or an app
/// that moved, is picked up. Problems that concern every browser (a bad host path or
/// origin, the config can't be written) fail the whole call; one browser failing is
/// reported in [`Report::failed`] and the others still get the host.
pub fn install(opts: &InstallOptions) -> io::Result<Report> {
    let bytes = manifest_bytes(opts)?;
    let file = format!("{HOST_NAME}.json");
    let mut report = Report::default();
    #[cfg(not(windows))]
    for target in TARGETS {
        let Some(dir) = browser_dir(&opts.home, target).filter(|d| d.is_dir()) else { continue };
        let path = dir.join("NativeMessagingHosts").join(&file);
        match write_atomic(&path, &bytes) {
            Ok(()) => report
                .done
                .push(Installed { browser: target.name, location: path.display().to_string() }),
            Err(e) => report.failed.push((target.name, e)),
        }
    }
    #[cfg(windows)]
    {
        use winreg::RegKey;
        use winreg::enums::HKEY_CURRENT_USER;
        let manifest_path = host_dir(&opts.support_dir).join(&file);
        write_atomic(&manifest_path, &bytes)?;
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        for target in TARGETS {
            let Some(key) = target.win_key else { continue };
            let browser = format!(r"{}\{key}", opts.registry_root);
            if hkcu.open_subkey(&browser).is_err() {
                continue;
            }
            let host_key = format!(r"{browser}\NativeMessagingHosts\{HOST_NAME}");
            let written = hkcu
                .create_subkey(&host_key)
                .and_then(|(k, _)| k.set_value("", &manifest_path.display().to_string()));
            match written {
                Ok(()) => report.done.push(Installed {
                    browser: target.name,
                    location: format!(r"HKCU\{host_key}"),
                }),
                Err(e) => report.failed.push((target.name, e)),
            }
        }
    }
    Ok(report)
}

/// Removes every manifest / registry entry [`install`] may have written, and the host
/// config. Missing ones are fine; one that can't be removed is reported, and the rest are
/// still removed.
pub fn uninstall(opts: &InstallOptions) -> Report {
    let file = format!("{HOST_NAME}.json");
    let mut report = Report::default();
    let mut note = |what: &'static str, r: io::Result<()>| {
        if let Err(e) = r {
            report.failed.push((what, e));
        }
    };
    #[cfg(not(windows))]
    for target in TARGETS {
        if let Some(dir) = browser_dir(&opts.home, target) {
            note(target.name, remove_if_present(&dir.join("NativeMessagingHosts").join(&file)));
        }
    }
    #[cfg(windows)]
    {
        use winreg::RegKey;
        use winreg::enums::HKEY_CURRENT_USER;
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        for target in TARGETS {
            let Some(key) = target.win_key else { continue };
            let host_key =
                format!(r"{}\{key}\NativeMessagingHosts\{HOST_NAME}", opts.registry_root);
            match hkcu.delete_subkey_all(&host_key) {
                Err(e) if e.kind() != io::ErrorKind::NotFound => note(target.name, Err(e)),
                _ => {}
            }
        }
        note("manifest", remove_if_present(&host_dir(&opts.support_dir).join(&file)));
    }
    note("host config", remove_if_present(&HostConfig::path(&opts.support_dir)));
    report
}

fn remove_if_present(path: &Path) -> io::Result<()> {
    match std::fs::remove_file(path) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::origin_of;

    const ID: &str = "abcdefghijklmnopabcdefghijklmnop";

    fn options(dir: &Path) -> InstallOptions {
        InstallOptions {
            home: dir.join("home"),
            support_dir: dir.join("support"),
            host_path: dir.join("Decay.app/Contents/MacOS/decay-host"),
            origins: vec![origin_of(ID)],
            app_path: None,
            registry_root: format!(r"Software\DecayTest-{}", std::process::id()),
        }
    }

    #[test]
    fn writes_a_valid_manifest() {
        let m = manifest(
            Path::new("/Applications/Decay.app/Contents/MacOS/decay-host"),
            &[origin_of(ID)],
        );
        assert_eq!(m["name"], "com.decay.host");
        assert_eq!(m["type"], "stdio");
        assert_eq!(m["allowed_origins"][0], format!("chrome-extension://{ID}/"));
    }

    #[test]
    fn refuses_bad_origins_no_origins_and_transient_host_paths() {
        let dir = tempfile::tempdir().unwrap();
        let mut opts = options(dir.path());
        opts.origins = vec!["garbage".into()];
        assert_eq!(install(&opts).unwrap_err().kind(), io::ErrorKind::InvalidInput);
        opts.origins = vec![];
        assert_eq!(install(&opts).unwrap_err().kind(), io::ErrorKind::InvalidInput);
        assert!(transient_path(Path::new(
            "/private/var/folders/x/AppTranslocation/ABC/d/Decay.app/Contents/MacOS/decay-host"
        )));
        assert!(transient_path(Path::new("/Volumes/Decay/Decay.app/Contents/MacOS/decay-host")));
        assert!(!transient_path(Path::new("/Applications/Decay.app/Contents/MacOS/decay-host")));
    }

    #[cfg(unix)]
    #[test]
    fn one_unwritable_browser_does_not_stop_the_others() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let opts = options(dir.path());
        for name in ["Google Chrome", "Chromium", "Brave"] {
            let t = TARGETS.iter().find(|t| t.name == name).unwrap();
            std::fs::create_dir_all(
                browser_dir(&opts.home, t).unwrap().join("NativeMessagingHosts"),
            )
            .unwrap();
        }
        let chromium = TARGETS.iter().find(|t| t.name == "Chromium").unwrap();
        let locked = browser_dir(&opts.home, chromium).unwrap().join("NativeMessagingHosts");
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555)).unwrap();
        let report = install(&opts).unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
        let names: Vec<&str> = report.done.iter().map(|i| i.browser).collect();
        assert_eq!(names, ["Google Chrome", "Brave"]);
        assert_eq!(report.failed.iter().map(|f| f.0).collect::<Vec<_>>(), ["Chromium"]);
        // No temporary files left behind.
        let leftovers = std::fs::read_dir(&locked).unwrap().count();
        assert_eq!(leftovers, 0);
    }

    #[test]
    fn refuses_a_relative_host_path() {
        let dir = tempfile::tempdir().unwrap();
        let mut opts = options(dir.path());
        opts.host_path = "decay-host".into();
        assert_eq!(install(&opts).unwrap_err().kind(), io::ErrorKind::InvalidInput);
    }

    #[cfg(not(windows))]
    #[test]
    fn installs_for_detected_browsers_only_and_uninstalls() {
        let dir = tempfile::tempdir().unwrap();
        let opts = options(dir.path());
        let detected: Vec<&BrowserTarget> =
            TARGETS.iter().filter(|t| ["Google Chrome", "Brave"].contains(&t.name)).collect();
        for t in &detected {
            std::fs::create_dir_all(browser_dir(&opts.home, t).unwrap()).unwrap();
        }
        let report = install(&opts).unwrap();
        assert_eq!(report.failed.len(), 0);
        let done = report.done;
        let names: Vec<&str> = done.iter().map(|i| i.browser).collect();
        assert_eq!(names, ["Google Chrome", "Brave"]);
        let written: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&done[0].location).unwrap()).unwrap();
        assert_eq!(written["path"], opts.host_path.display().to_string());
        assert_eq!(HostConfig::load(&opts.support_dir).allowed_origins, opts.origins);
        // Edge isn't installed: no folder was created for it.
        let edge = TARGETS.iter().find(|t| t.name == "Microsoft Edge").unwrap();
        assert!(!browser_dir(&opts.home, edge).unwrap().exists());

        assert_eq!(uninstall(&opts).failed.len(), 0);
        assert!(!Path::new(&done[0].location).exists());
        assert!(!HostConfig::path(&opts.support_dir).exists());
        assert_eq!(uninstall(&opts).failed.len(), 0, "twice is fine");
    }

    #[cfg(windows)]
    #[test]
    fn installs_registry_entries_for_detected_browsers_only() {
        use winreg::RegKey;
        use winreg::enums::HKEY_CURRENT_USER;
        /// Removes the scratch registry tree even if the test fails.
        struct Cleanup(String);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = RegKey::predef(HKEY_CURRENT_USER).delete_subkey_all(&self.0);
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let opts = options(dir.path());
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let _cleanup = Cleanup(opts.registry_root.clone());
        hkcu.create_subkey(format!(r"{}\Google\Chrome", opts.registry_root)).unwrap();
        let report = install(&opts).unwrap();
        let names: Vec<&str> = report.done.iter().map(|i| i.browser).collect();
        assert_eq!(names, ["Google Chrome"]);
        let key = hkcu
            .open_subkey(format!(
                r"{}\Google\Chrome\NativeMessagingHosts\{HOST_NAME}",
                opts.registry_root
            ))
            .unwrap();
        let manifest_path: String = key.get_value("").unwrap();
        assert!(Path::new(&manifest_path).is_file());
        assert_eq!(uninstall(&opts).failed.len(), 0);
        assert!(!Path::new(&manifest_path).exists());
    }
}
