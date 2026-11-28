//! First run (F21, Software Design §5.2, API Design §2.7): browser detection and the
//! native host, the extension's store page, Accessibility, and finishing onboarding.
//!
//! The engine runs during onboarding in shadow mode (the default), so nothing is closed
//! before the user has been through it; finishing it starts a fresh week of shadow mode.

#![allow(clippy::needless_pass_by_value, reason = "Tauri commands take their arguments by value")]

use std::sync::atomic::Ordering::SeqCst;

use decay_bridge::protocol::BrowserKind;
use decay_core::Preset;
use decay_engine::WindowManagement;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tauri::{AppHandle, Manager};

use crate::commands::{Res, blocking, broadcast_settings};
use crate::core::{AppError, Core, home_dir, install_host};
use crate::shell::Shell;

/// The extension's store page. Every supported browser installs from the Chrome Web
/// Store (Edge allows it too). The item's id is also in decay-host's
/// `BUILTIN_EXTENSION_IDS`.
pub const STORE_URL: &str =
    "https://chromewebstore.google.com/detail/jidljehgkacgjoefbancbihmdkkpcaaa";
/// Firefox installs from addons.mozilla.org (F14). A search until the listing exists.
pub const FIREFOX_STORE_URL: &str = "https://addons.mozilla.org/firefox/search/?q=Decay";

/// A browser onboarding lists: its host targets (decay-host's names), its kind as the
/// extension reports it, and how to open a page in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Known {
    /// Chrome Beta / Canary count as Chrome (their extension reports itself as Chrome).
    pub targets: &'static [&'static str],
    pub kind: BrowserKind,
    pub name: &'static str,
    /// macOS bundle id (`open -b`).
    pub mac_bundle: &'static str,
    /// Windows: its executable under App Paths, if it registers one.
    pub win_exe: Option<&'static str>,
    /// Where its extension comes from.
    pub store_url: &'static str,
}

/// The browsers onboarding shows, in order (Designs/03c).
pub const KNOWN: [Known; 5] = [
    Known {
        targets: &["Google Chrome", "Google Chrome Beta", "Google Chrome Canary"],
        kind: BrowserKind::Chrome,
        name: "Google Chrome",
        mac_bundle: "com.google.Chrome",
        win_exe: Some("chrome.exe"),
        store_url: STORE_URL,
    },
    Known {
        targets: &["Brave"],
        kind: BrowserKind::Brave,
        name: "Brave",
        mac_bundle: "com.brave.Browser",
        win_exe: Some("brave.exe"),
        store_url: STORE_URL,
    },
    Known {
        targets: &["Microsoft Edge"],
        kind: BrowserKind::Edge,
        name: "Microsoft Edge",
        mac_bundle: "com.microsoft.edgemac",
        win_exe: Some("msedge.exe"),
        store_url: STORE_URL,
    },
    Known {
        targets: &["Chromium"],
        kind: BrowserKind::Chromium,
        name: "Chromium",
        mac_bundle: "org.chromium.Chromium",
        win_exe: Some("chromium.exe"),
        store_url: STORE_URL,
    },
    Known {
        targets: &["Firefox"],
        kind: BrowserKind::Firefox,
        name: "Firefox",
        mac_bundle: "org.mozilla.firefox",
        win_exe: Some("firefox.exe"),
        store_url: FIREFOX_STORE_URL,
    },
];

fn kind_id(kind: BrowserKind) -> String {
    serde_json::to_value(kind).ok().and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_default()
}

/// One row of `detect_browsers`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectedBrowser {
    /// The extension's `browser` value (`chrome`, `edge`, …).
    pub id: String,
    pub name: String,
    pub installed: bool,
    pub host_installed: bool,
    pub extension_connected: bool,
    /// Connected profiles of it.
    pub profiles: u32,
    pub store_url: &'static str,
}

/// Puts detection and the connected browsers together. A browser that's connected counts
/// as installed and set up even if detection missed it.
pub fn rows(
    detected: &[decay_host::install::Detected],
    connected: &[BrowserKind],
) -> Vec<DetectedBrowser> {
    KNOWN
        .iter()
        .filter_map(|k| {
            let found: Vec<_> =
                detected.iter().filter(|d| k.targets.contains(&d.target.name)).collect();
            let profiles = connected.iter().filter(|c| **c == k.kind).count();
            if found.is_empty() && profiles == 0 {
                return None; // not a browser of this platform
            }
            let on = profiles > 0;
            Some(DetectedBrowser {
                id: kind_id(k.kind),
                name: k.name.to_owned(),
                installed: on || found.iter().any(|d| d.installed),
                host_installed: on || found.iter().any(|d| d.host_installed),
                extension_connected: on,
                profiles: u32::try_from(profiles).unwrap_or(u32::MAX),
                store_url: k.store_url,
            })
        })
        .collect()
}

#[tauri::command]
pub async fn detect_browsers(app: AppHandle) -> Res<Vec<DetectedBrowser>> {
    let support = app.state::<Shell>().support.clone();
    blocking(app, move |core| {
        let connected: Vec<BrowserKind> = core
            .ask(|e, now| (e.status(now).browsers, Vec::new()))
            .map(|b| b.into_iter().map(|b| b.browser).collect())
            .unwrap_or_default();
        let home = home_dir().ok_or_else(|| AppError::new("INTERNAL", "no home folder"))?;
        let detected = decay_host::install::detect(&home, &support, "Software");
        Ok(rows(&detected, &connected))
    })
    .await
}

/// Installs the host for every detected browser now (it's also done at every start).
/// `NOT_FOUND`: this build has no host; `PERMISSION_DENIED`: installing failed.
#[tauri::command]
pub async fn install_native_hosts(app: AppHandle) -> Res<Value> {
    let support = app.state::<Shell>().support.clone();
    let installed = tauri::async_runtime::spawn_blocking(move || install_host(&support))
        .await
        .map_err(|e| AppError::new("INTERNAL", e.to_string()))??;
    Ok(json!({ "installed": installed }))
}

/// Opens the extension's store page in that browser (installing it there is what
/// connects it). Only the browsers onboarding lists. Runs off the main thread (a browser
/// may take a while to start).
#[tauri::command]
pub async fn open_extension_store(browser: String) -> Res<Value> {
    let known = *KNOWN.iter().find(|k| kind_id(k.kind) == browser).ok_or_else(|| {
        AppError::new(
            "INVALID_INPUT",
            crate::i18n::tf("error.unknownBrowser", &[("browser", browser.as_str().into())]),
        )
    })?;
    let opened = tauri::async_runtime::spawn_blocking(move || open_in(&known))
        .await
        .map_err(|e| AppError::new("INTERNAL", e.to_string()))?;
    opened.map_err(|e| {
        tracing::warn!(browser = %known.name, "store page: {e}");
        AppError::new(
            "NOT_FOUND",
            crate::i18n::tf("error.store", &[("browser", known.name.into())]),
        )
    })?;
    Ok(json!({}))
}

#[cfg(target_os = "macos")]
fn open_in(known: &Known) -> std::io::Result<()> {
    let status = std::process::Command::new("/usr/bin/open")
        .args(["-b", known.mac_bundle, known.store_url])
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other(format!("open -b {}: {status}", known.mac_bundle)))
    }
}

/// Starts the browser's own executable (from App Paths) with the page: no shell, no
/// console window, and never the wrong browser.
#[cfg(windows)]
fn open_in(known: &Known) -> std::io::Result<()> {
    use std::os::windows::process::CommandExt;
    use winreg::RegKey;
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let exe = known.win_exe.ok_or_else(|| std::io::Error::other("no App Paths entry"))?;
    let key = format!(r"Software\Microsoft\Windows\CurrentVersion\App Paths\{exe}");
    let path: String = [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE]
        .into_iter()
        .find_map(|root| RegKey::predef(root).open_subkey(&key).ok()?.get_value("").ok())
        .ok_or_else(|| std::io::Error::other(format!("{exe} isn't registered")))?;
    std::process::Command::new(path.trim_matches('"'))
        .arg(known.store_url)
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map(|_| ())
}

#[cfg(not(any(target_os = "macos", windows)))]
fn open_in(_known: &Known) -> std::io::Result<()> {
    std::process::Command::new("xdg-open").arg(known.store_url).spawn().map(|_| ())
}

/// What an Accessibility request does: the system prompt the first time in a run (it also
/// adds Decay to the list in System Settings), then the System Settings pane (macOS shows
/// the prompt only once).
fn ax_request_kind(already_prompted: &std::sync::atomic::AtomicBool) -> &'static str {
    if already_prompted.swap(true, SeqCst) { "settings" } else { "prompt" }
}

/// Asks for Accessibility (macOS); the answer arrives as `permission:changed`.
#[tauri::command]
pub async fn request_accessibility(app: AppHandle) -> Res<Value> {
    let kind = ax_request_kind(&app.state::<Shell>().ax_prompted);
    if kind == "prompt" {
        if !app.state::<Core>().request_accessibility() {
            app.state::<Shell>().ax_prompted.store(false, SeqCst);
            return Err(AppError::new("ADAPTER_UNAVAILABLE", crate::i18n::t("error.engine")));
        }
    } else {
        #[cfg(target_os = "macos")]
        tauri::async_runtime::spawn_blocking(|| {
            let pane =
                "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility";
            if let Err(e) = std::process::Command::new("/usr/bin/open").arg(pane).status() {
                tracing::warn!("System Settings: {e}");
            }
        })
        .await
        .map_err(|e| AppError::new("INTERNAL", e.to_string()))?;
    }
    Ok(json!({ "requested": true, "opened": kind }))
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Completion {
    preset: Preset,
    window_management: WindowManagement,
    launch_at_login: bool,
}

/// Finishes onboarding in one transaction: pace, window handling, launch at login, and a
/// fresh week of shadow mode from now. Every window hears `settings:changed`; the login
/// item is applied before this returns (so the Done step's switch can't race it).
#[tauri::command]
pub async fn complete_onboarding(app: AppHandle, input: Completion) -> Res<Value> {
    let handle = app.clone();
    blocking(app.clone(), move |core| {
        core.ask(move |e, now| {
            let done = e
                .complete_onboarding(
                    input.preset,
                    input.window_management,
                    input.launch_at_login,
                    now,
                )
                .and_then(|out| Ok((e.store().settings_snapshot()?, out)));
            match done {
                Ok((settings, out)) => {
                    broadcast_settings(&handle, &settings);
                    (Ok(()), out)
                }
                Err(err) => (Err(AppError::db(&err)), Vec::new()),
            }
        })?
    })
    .await?;
    crate::shell::set_onboarded(&app, true);
    let handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || crate::apply_launch_at_login(&handle))
        .await
        .map_err(|e| AppError::new("INTERNAL", e.to_string()))?;
    Ok(json!({}))
}

#[cfg(test)]
mod tests {
    use decay_host::install::{Detected, TARGETS};

    use super::*;

    fn found(name: &str, installed: bool, host: bool) -> Detected {
        let target = TARGETS.iter().find(|t| t.name == name).unwrap();
        Detected { target, installed, host_installed: host }
    }

    #[test]
    fn every_listed_browser_is_a_host_target() {
        for k in KNOWN {
            for t in k.targets {
                assert!(TARGETS.iter().any(|x| x.name == *t), "{t}");
            }
        }
        assert_eq!(kind_id(BrowserKind::Edge), "edge");
    }

    #[test]
    fn rows_combine_detection_and_connections() {
        let detected = [
            found("Google Chrome", true, true),
            found("Brave", true, false),
            found("Microsoft Edge", false, false),
        ];
        let rows = rows(&detected, &[BrowserKind::Chrome, BrowserKind::Chrome, BrowserKind::Chromium]);
        let view: Vec<(&str, bool, bool, bool, u32)> = rows
            .iter()
            .map(|r| {
                (r.id.as_str(), r.installed, r.host_installed, r.extension_connected, r.profiles)
            })
            .collect();
        assert_eq!(
            view,
            [
                ("chrome", true, true, true, 2),
                ("brave", true, false, false, 0),
                ("edge", false, false, false, 0),
                // Connected though detection didn't see it.
                ("chromium", true, true, true, 1),
            ]
        );
        assert!(rows.iter().all(|r| r.store_url == STORE_URL));
        // Firefox installs its extension from Mozilla's site.
        let ff = super::rows(&[found("Firefox", true, false)], &[]);
        assert_eq!((ff[0].id.as_str(), ff[0].store_url), ("firefox", FIREFOX_STORE_URL));
    }

    #[test]
    fn completion_input_is_validated() {
        let ok: Completion = serde_json::from_value(
            json!({"preset": "gentle", "windowManagement": "off", "launchAtLogin": false}),
        )
        .unwrap();
        assert_eq!((ok.preset, ok.window_management), (Preset::Gentle, WindowManagement::Off));
        for bad in [
            json!({"preset": "turbo", "windowManagement": "off", "launchAtLogin": true}),
            json!({"preset": "gentle", "windowManagement": "close", "launchAtLogin": true}),
            json!({"preset": "gentle", "windowManagement": "off"}),
        ] {
            assert!(serde_json::from_value::<Completion>(bad).is_err());
        }
    }

    #[test]
    fn chrome_beta_and_canary_count_as_chrome() {
        let rows = rows(
            &[found("Google Chrome", false, false), found("Google Chrome Canary", true, true)],
            &[],
        );
        let chrome = rows.iter().find(|r| r.id == "chrome").unwrap();
        assert!(chrome.installed && chrome.host_installed && !chrome.extension_connected);
    }

    #[test]
    fn accessibility_prompts_once_then_opens_settings() {
        let flag = std::sync::atomic::AtomicBool::new(false);
        assert_eq!(ax_request_kind(&flag), "prompt");
        assert_eq!(ax_request_kind(&flag), "settings");
        assert_eq!(ax_request_kind(&flag), "settings");
    }

    #[test]
    fn only_listed_browsers_open_the_store() {
        let err = tauri::async_runtime::block_on(open_extension_store("netscape".into()));
        assert_eq!(err.unwrap_err().code, "INVALID_INPUT");
    }
}
