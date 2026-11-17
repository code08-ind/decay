//! Help and feedback (P2.7, PRD F32, Designs/09h and 16): the Help menu, links to the
//! help pages, the changelog and a prefilled GitHub issue for feedback, and "What's New"
//! once after an update.
//!
//! Only these fixed links can be opened (a window never passes an address). The feedback
//! issue carries the Decay version and OS, nothing from the archive or settings. The
//! version is the app's (`tauri.conf.json`, as About and the updater read it).

#![allow(clippy::needless_pass_by_value, reason = "Tauri commands take their arguments by value")]

use std::cmp::Ordering;
use std::sync::atomic::{AtomicBool, Ordering::SeqCst};

use decay_store::Store;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tauri::{AppHandle, Emitter as _};

use crate::commands::{Res, blocking};
use crate::core::AppError;
use crate::deeplink::Window;

/// The public releases repo (Docs/Release.md).
const RELEASES_REPO: &str = "https://github.com/code08-ind/decay-releases";
/// Help: the releases repo's README until a docs site exists (the one place to change).
pub const HELP_URL: &str = "https://github.com/code08-ind/decay-releases#readme";
/// Every release with its notes.
pub const CHANGELOG_URL: &str = "https://github.com/code08-ind/decay-releases/releases";

/// The setting holding the version whose "What's New" was last seen (or skipped).
const SEEN_KEY: &str = "whats_new_seen";

/// "Later" was chosen in this run: not shown again until Decay starts again (the Library
/// window is rebuilt each time it opens).
static LATER: AtomicBool = AtomicBool::new(false);

/// The running version, as the app (About, the updater) knows it.
pub fn app_version(app: &AppHandle) -> String {
    app.package_info().version.to_string()
}

/// A link a window may open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Link {
    Help,
    Changelog,
    Feedback,
}

/// The new-issue page, prefilled: a title to replace, questions, and version / OS.
pub fn feedback_url(version: &str, os: &str) -> String {
    let body = format!(
        "**What happened, or what would you like?**\n\n\n**What did you expect?**\n\n\n---\nDecay {version} · {os} · {}",
        std::env::consts::ARCH
    );
    let mut url = url::Url::parse(&format!("{RELEASES_REPO}/issues/new")).expect("a valid URL");
    url.query_pairs_mut().append_pair("title", "Feedback: ").append_pair("body", &body);
    url.into()
}

/// The address of `link`.
pub fn link_url(link: Link, version: &str, os: &str) -> String {
    match link {
        Link::Help => HELP_URL.to_owned(),
        Link::Changelog => CHANGELOG_URL.to_owned(),
        Link::Feedback => feedback_url(version, os),
    }
}

/// Opens an `https` address in the default browser (blocks while the opener starts).
pub fn open_url(url: &str) -> Res<()> {
    if !url.starts_with("https://") {
        return Err(AppError::new("INVALID_INPUT", "only https links open"));
    }
    #[cfg(windows)]
    let status = {
        let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
        std::process::Command::new(std::path::Path::new(&root).join("System32\\rundll32.exe"))
            .args(["url.dll,FileProtocolHandler", url])
            .status()
    };
    #[cfg(not(windows))]
    let status = std::process::Command::new("/usr/bin/open").arg(url).status();
    match status {
        Ok(s) if s.success() => Ok(()),
        Ok(s) => Err(AppError::new("IO_ERROR", format!("Couldn't open the link ({s})"))),
        Err(e) => Err(AppError::new("IO_ERROR", format!("Couldn't open the link: {e}"))),
    }
}

/// Opens a help link (blocking: `sw_vers` and the opener run).
fn open_link_blocking(link: Link, version: &str) -> Res<()> {
    open_url(&link_url(link, version, &crate::diagnostics::os_version()))
}

/// The Help menu: opens a link off the main thread.
pub fn open_link_from_menu(app: &AppHandle, link: Link) {
    let version = app_version(app);
    tauri::async_runtime::spawn_blocking(move || {
        if let Err(e) = open_link_blocking(link, &version) {
            tracing::warn!("help link: {}", e.message);
        }
    });
}

#[tauri::command]
pub async fn open_link(app: AppHandle, link: Link) -> Res<()> {
    let version = app_version(&app);
    tauri::async_runtime::spawn_blocking(move || open_link_blocking(link, &version))
        .await
        .map_err(|e| AppError::new("INTERNAL", e.to_string()))?
}

/// Help → What's New, or Settings → About: an open Library shows it over the screen it's
/// on (`whats-new:open`); otherwise the Library opens with it.
pub fn show_whats_new(app: &AppHandle) {
    // Onboarded: the window exists only then (the gate opens onboarding instead).
    if crate::shell::focus_window(app, Window::Library) {
        let _ = app.emit_to(Window::Library.label(), "whats-new:open", json!({}));
    } else {
        crate::shell::show_window(app, Window::Library, "/?whats-new");
    }
}

#[tauri::command]
pub fn ui_show_whats_new(app: AppHandle) {
    show_whats_new(&app);
}

/// A version as `(major, minor, patch)` and its pre-release identifiers (empty for a
/// release); `None` if it isn't one.
fn parse(v: &str) -> Option<((u64, u64, u64), Vec<String>)> {
    let v = v.trim().trim_start_matches('v');
    let v = v.split('+').next()?;
    let (core, pre) = v.split_once('-').map_or((v, ""), |(c, p)| (c, p));
    let mut it = core.split('.').map(|p| p.parse::<u64>().ok());
    let n = (it.next()??, it.next()??, it.next().unwrap_or(Some(0))?);
    if it.next().is_some() {
        return None;
    }
    let pre = if pre.is_empty() { Vec::new() } else { pre.split('.').map(str::to_owned).collect() };
    Some((n, pre))
}

/// Semantic-version order: a release is newer than its pre-releases; numeric
/// identifiers compare as numbers (`beta.10` > `beta.9`).
fn compare(a: &str, b: &str) -> Option<Ordering> {
    let (an, ap) = parse(a)?;
    let (bn, bp) = parse(b)?;
    Some(an.cmp(&bn).then_with(|| match (ap.is_empty(), bp.is_empty()) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Greater,
        (false, true) => Ordering::Less,
        (false, false) => {
            for (x, y) in ap.iter().zip(&bp) {
                let o = match (x.parse::<u64>(), y.parse::<u64>()) {
                    (Ok(x), Ok(y)) => x.cmp(&y),
                    (Ok(_), Err(_)) => Ordering::Less,
                    (Err(_), Ok(_)) => Ordering::Greater,
                    (Err(_), Err(_)) => x.cmp(y),
                };
                if o != Ordering::Equal {
                    return o;
                }
            }
            ap.len().cmp(&bp.len())
        }
    }))
}

/// What to do at start-up: show "What's New" for `current`, and/or note `current` as seen
/// now (no earlier version known — a new install, or the first run with this feature —
/// or not an update: nothing new to show).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WhatsNewPlan {
    pub show: bool,
    pub mark_seen: bool,
}

pub fn plan(seen: Option<&str>, current: &str) -> WhatsNewPlan {
    let Some(seen) = seen.filter(|s| !s.is_empty()) else {
        return WhatsNewPlan { show: false, mark_seen: true };
    };
    if seen == current {
        return WhatsNewPlan { show: false, mark_seen: false };
    }
    match compare(seen, current) {
        Some(Ordering::Less) => WhatsNewPlan { show: true, mark_seen: false },
        _ => WhatsNewPlan { show: false, mark_seen: true },
    }
}

/// `whats_new` → the running version, and whether its notes are still to be shown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WhatsNew {
    pub version: String,
    pub show: bool,
}

/// Reads (and if needed stores) what was seen; `later`: "Later" chosen in this run.
pub fn whats_new_in(
    s: &Store,
    version: &str,
    later: bool,
    now: i64,
) -> Result<WhatsNew, decay_store::StoreError> {
    let seen: Option<String> = s.setting(SEEN_KEY)?;
    let p = plan(seen.as_deref(), version);
    if p.mark_seen {
        s.set_setting(SEEN_KEY, &version, now)?;
    }
    Ok(WhatsNew { version: version.to_owned(), show: p.show && !later })
}

#[tauri::command]
pub async fn whats_new(app: AppHandle) -> Res<WhatsNew> {
    let version = app_version(&app);
    blocking(app, move |core| {
        core.ask(move |e, now| {
            let r = whats_new_in(e.store(), &version, LATER.load(SeqCst), now);
            (r.map_err(|err| AppError::db(&err)), Vec::new())
        })?
    })
    .await
}

/// "Got it" (or this version has no notes): not shown again for this version. With
/// `later`, only for the rest of this run.
#[tauri::command]
pub async fn whats_new_seen(app: AppHandle, later: Option<bool>) -> Res<()> {
    if later == Some(true) {
        LATER.store(true, SeqCst);
        return Ok(());
    }
    let version = app_version(&app);
    blocking(app, move |core| {
        core.ask(move |e, now| {
            let r = e.store().set_setting(SEEN_KEY, &version, now);
            (r.map_err(|err| AppError::db(&err)), Vec::new())
        })?
    })
    .await
}

#[cfg(test)]
mod tests {
    use decay_store::{DbKey, StorePaths};

    use super::*;

    #[test]
    fn whats_new_shows_once_after_an_update() {
        let p = |seen: Option<&str>, cur: &str| {
            let p = plan(seen, cur);
            (p.show, p.mark_seen)
        };
        assert_eq!(p(None, "1.5.0"), (false, true), "new install: nothing to show");
        assert_eq!(p(Some(""), "1.5.0"), (false, true));
        assert_eq!(p(Some("1.5.0"), "1.5.0"), (false, false), "seen");
        assert_eq!(p(Some("1.4.2"), "1.5.0"), (true, false), "updated: until Got it");
        assert_eq!(p(Some("1.9.0"), "1.10.0"), (true, false), "numbers, not text");
        assert_eq!(p(Some("1.5.0-beta.2"), "1.5.0"), (true, false), "beta → release");
        assert_eq!(p(Some("1.5.0-beta.9"), "1.5.0-beta.10"), (true, false));
        assert_eq!(p(Some("1.5.0"), "1.6.0-beta.1"), (true, false), "next beta");
        assert_eq!(p(Some("1.5.0"), "1.5.0-beta.1"), (false, true), "back to its beta");
        assert_eq!(p(Some("1.5.0-beta.2"), "1.5.0-beta.1"), (false, true), "older beta");
        assert_eq!(p(Some("1.6.0"), "1.5.0"), (false, true), "downgrade");
        assert_eq!(p(Some("junk"), "1.5.0"), (false, true));
    }

    #[test]
    fn versions_compare() {
        assert_eq!(compare("1.4.2", "1.4.2"), Some(Ordering::Equal));
        assert_eq!(compare("v2.0", "2.0.0"), Some(Ordering::Equal));
        assert_eq!(compare("1.5.0-beta.1+abc", "1.5.0-beta.1"), Some(Ordering::Equal));
        assert_eq!(compare("1.5.0-alpha", "1.5.0-beta"), Some(Ordering::Less));
        assert_eq!(compare("1.5.0-beta", "1.5.0-beta.1"), Some(Ordering::Less));
        assert_eq!(compare("1.2.3.4", "1.0.0"), None);
        assert_eq!(compare("x", "1.0.0"), None);
    }

    #[test]
    fn what_was_seen_is_stored_and_later_lasts_the_run() {
        let dir = tempfile::tempdir().unwrap();
        let key = DbKey::generate().unwrap();
        let (s, _) = Store::open(StorePaths::in_dir(dir.path()), key, 1).unwrap();
        let show = |v: &str, later: bool| whats_new_in(&s, v, later, 2).unwrap().show;
        assert!(!show("0.1.0", false), "a new install shows nothing");
        assert_eq!(s.setting::<String>(SEEN_KEY).unwrap().as_deref(), Some("0.1.0"));
        assert!(!show("0.1.0", false));
        assert!(show("0.2.0", false), "after the update");
        assert!(show("0.2.0", false), "until Got it");
        assert!(!show("0.2.0", true), "not again in a run where Later was chosen");
        s.set_setting(SEEN_KEY, &"0.2.0", 3).unwrap(); // Got it
        assert!(!show("0.2.0", false));
    }

    #[test]
    fn feedback_is_a_prefilled_issue_with_version_and_os_only() {
        let url = feedback_url("1.4.2", "macOS 15.1");
        assert!(url.starts_with("https://github.com/code08-ind/decay-releases/issues/new?"));
        let parsed = url::Url::parse(&url).unwrap();
        let q: std::collections::HashMap<_, _> = parsed.query_pairs().collect();
        assert_eq!(q["title"], "Feedback: ");
        assert!(
            q["body"].ends_with(&format!("Decay 1.4.2 · macOS 15.1 · {}", std::env::consts::ARCH))
        );
        assert_eq!(q.len(), 2);
        assert_eq!(link_url(Link::Help, "1", "x"), HELP_URL);
        assert_eq!(link_url(Link::Changelog, "1", "x"), CHANGELOG_URL);
    }

    #[test]
    fn only_https_opens() {
        assert!(open_url("file:///etc/passwd").is_err());
        assert!(open_url("javascript:alert(1)").is_err());
    }
}
