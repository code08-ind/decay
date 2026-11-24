//! Batch labels (F15, Pro, off by default): a short name for each archived batch, made
//! on this computer only.
//!
//! - **Apple's on-device model** (Foundation Models, macOS 26+ with Apple Intelligence)
//!   through `decay-labeler`, a small helper bundled next to the app (built by
//!   `scripts/sidecar.mjs`): the batch's titles and sites in on stdin, one line out. Exit
//!   code 3 means the model isn't available on this Mac: the keyword labeller is used for
//!   the rest of the session.
//! - **Keywords** everywhere else ([`keywords`]).
//! - A worker labels new batches (archived in the last 30 days, 2+ items) every few
//!   minutes while the setting is on and Pro is active; turning it off forgets every
//!   label. Labels of batches that are no longer archived are forgotten too.

pub mod keywords;

use std::io::Write as _;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use decay_engine::Engine;
use decay_store::{Batch, BatchItem};
use tauri::{AppHandle, Manager};

use crate::core::Core;

/// The setting that turns labels on.
pub const SETTING: &str = "batch_labels";
/// How often new batches are looked for.
const EVERY: Duration = Duration::from_mins(5);
/// Batches archived longer ago than this aren't named any more.
const LOOKBACK_MS: i64 = 30 * 86_400_000;
/// Batches named per run (the model takes a second or two each).
const PER_RUN: usize = 20;
/// How long the helper may take for one batch.
const HELPER_TIMEOUT: Duration = Duration::from_secs(20);
/// The helper said the model isn't available: don't ask again this session.
static NO_MODEL: AtomicBool = AtomicBool::new(false);

/// Which labeller named a batch (stored with the label).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Model {
    Apple,
    Keywords,
}

impl Model {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Apple => "apple",
            Self::Keywords => "keywords",
        }
    }
}

/// The helper next to the app (`decay-labeler`), if this build has one.
fn helper() -> Option<std::path::PathBuf> {
    if !cfg!(target_os = "macos") || NO_MODEL.load(Ordering::Relaxed) {
        return None;
    }
    let exe = std::env::current_exe().ok()?;
    exe.parent().map(|d| d.join("decay-labeler")).filter(|p| p.is_file())
}

/// The helper's input: the titles and sites, nothing else.
fn helper_input(items: &[BatchItem]) -> String {
    let items: Vec<serde_json::Value> =
        items.iter().map(|i| serde_json::json!({ "title": i.title, "site": i.domain })).collect();
    serde_json::json!({ "items": items }).to_string()
}

/// A label from the on-device model, or `None` (no helper, no model, a failure, a
/// timeout, or an unusable answer).
fn model_label(items: &[BatchItem]) -> Option<String> {
    let path = helper()?;
    let mut child = Command::new(path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let input = helper_input(items);
    child.stdin.take()?.write_all(input.as_bytes()).ok()?;
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() < HELPER_TIMEOUT => {
                std::thread::sleep(Duration::from_millis(50));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };
    if status.code() == Some(3) {
        tracing::info!("on-device model not available: keyword labels");
        NO_MODEL.store(true, Ordering::Relaxed);
        return None;
    }
    let mut out = String::new();
    std::io::Read::read_to_string(&mut child.stdout.take()?, &mut out).ok()?;
    status.success().then(|| clean(&out)).flatten()
}

/// A model's answer as a label: its first line, without quotes or a trailing period;
/// `None` when that leaves nothing.
pub fn clean(answer: &str) -> Option<String> {
    let line = answer.lines().map(str::trim).find(|l| !l.is_empty())?;
    let line = line.trim_matches(|c: char| matches!(c, '"' | '\'' | '“' | '”' | '«' | '»'));
    let line = line.strip_suffix('.').unwrap_or(line).trim();
    (!line.is_empty() && !line.chars().any(char::is_control)).then(|| keywords::cut(line))
}

/// The best label for a batch: the model's, else the keywords'.
pub fn label(items: &[BatchItem]) -> Option<(String, Model)> {
    if let Some(l) = model_label(items) {
        return Some((l, Model::Apple));
    }
    keywords::label(items).map(|l| (l, Model::Keywords))
}

/// What one run should do, read on the engine thread: label these batches, or (labels
/// off, or no Pro) forget every label.
fn todo(e: &Engine, now: i64) -> Result<Option<Vec<Batch>>, decay_store::StoreError> {
    let on = e.store().setting::<bool>(SETTING)?.unwrap_or(false) && e.is_pro();
    if !on {
        e.store().clear_batch_labels()?;
        return Ok(None);
    }
    e.store().prune_batch_labels()?;
    e.store().unlabeled_batches(now - LOOKBACK_MS, PER_RUN).map(Some)
}

/// One run: name what needs a name (off the engine thread), then save the names.
fn run(core: &Core) -> Result<usize, crate::core::AppError> {
    let batches = core
        .ask(|e, now| (todo(e, now), Vec::new()))?
        .map_err(|err| crate::core::AppError::db(&err))?
        .unwrap_or_default();
    let named: Vec<(Batch, String, Model)> = batches
        .into_iter()
        .filter_map(|b| {
            let (label, model) = label(&b.items)?;
            Some((b, label, model))
        })
        .collect();
    let n = named.len();
    if n == 0 {
        return Ok(0);
    }
    core.ask(move |e, now| {
        let saved = e.store_mut().atomically(false, |s| {
            for (b, label, model) in &named {
                s.set_batch_label(&b.sweep_id, b.source_id.as_deref(), label, model.as_str(), now)?;
            }
            Ok(())
        });
        (saved.map_err(|err| crate::core::AppError::db(&err)), Vec::new())
    })??;
    Ok(n)
}

/// Starts the worker (it sleeps first: the engine and the license settle).
pub fn start(app: &AppHandle) {
    let handle = app.clone();
    let spawned = std::thread::Builder::new().name("decay-labels".into()).spawn(move || {
        loop {
            std::thread::sleep(EVERY);
            match run(&handle.state::<Core>()) {
                Ok(0) => {}
                Ok(n) => {
                    tracing::info!(n, "batches named");
                    let _ = tauri::Emitter::emit(&handle, "archive:changed", ());
                }
                Err(e) => tracing::warn!("batch labels: {}", e.message),
            }
        }
    });
    if let Err(e) = spawned {
        tracing::warn!("batch labels not started: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_answers_are_cleaned_up() {
        assert_eq!(
            clean("\n  \"Lisbon trip: flights, hotels.\"\nextra").as_deref(),
            Some("Lisbon trip: flights, hotels")
        );
        assert_eq!(clean("“Rust async”").as_deref(), Some("Rust async"));
        assert_eq!(clean("   \n\n"), None);
        assert_eq!(clean("\"\""), None);
        assert_eq!(clean(&"x".repeat(200)).map(|l| l.chars().count()), Some(keywords::MAX_LABEL));
    }

    #[test]
    fn the_helper_sees_titles_and_sites_only() {
        let input = helper_input(&[BatchItem {
            title: "A \"quoted\" title".into(),
            domain: Some("a.com".into()),
        }]);
        let v: serde_json::Value = serde_json::from_str(&input).unwrap();
        assert_eq!(v["items"][0]["title"], "A \"quoted\" title");
        assert_eq!(v["items"][0]["site"], "a.com");
        assert_eq!(v["items"][0].as_object().unwrap().len(), 2);
    }

    #[test]
    fn a_run_labels_only_with_the_setting_and_pro_and_forgets_when_off() {
        use crate::testing::{NOW, engine};
        let (mut e, _dir) = engine();
        e.store().set_setting(SETTING, &true, NOW).unwrap();
        // No Pro: nothing to do, and nothing kept.
        assert_eq!(todo(&e, NOW).unwrap(), None);
        e.set_license(true, None, NOW).unwrap();
        assert_eq!(todo(&e, NOW).unwrap(), Some(Vec::new()), "on, nothing archived yet");
        e.store().set_setting(SETTING, &false, NOW).unwrap();
        assert_eq!(todo(&e, NOW).unwrap(), None);
    }

    #[test]
    fn without_a_helper_labels_come_from_keywords() {
        let items = [
            BatchItem { title: "Lisbon flights".into(), domain: Some("a.com".into()) },
            BatchItem { title: "Lisbon hotels".into(), domain: Some("b.com".into()) },
        ];
        // Tests run without the bundled helper.
        assert_eq!(label(&items), Some(("Lisbon".to_owned(), Model::Keywords)));
    }
}
