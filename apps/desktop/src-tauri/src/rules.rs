//! Rule commands (P1.12, API Design §2.4, PRD F8/F9): list, create, update, delete, test a
//! rule against open tabs and the archive, and preview what an exclude rule would delete.

#![allow(clippy::needless_pass_by_value, reason = "Tauri commands take their arguments by value")]

use decay_core::Rule;
use decay_engine::{Engine, RuleInput, RuleRefused, RuleTest};
use decay_store::Store;
use serde::Serialize;
use serde_json::{Value, json};
use tauri::AppHandle;

use crate::commands::{Res, blocking};
use crate::core::AppError;
use crate::i18n::{t, tf};
use crate::search::{PanelItem, row};

/// Tells every window the rules changed (Settings reloads its lists).
pub fn announce(app: &AppHandle) {
    let _ = tauri::Emitter::emit(app, "rules:changed", ());
}

pub fn refused(why: &RuleRefused) -> AppError {
    match why {
        RuleRefused::Invalid(text) => AppError::new("INVALID_INPUT", invalid_text(text)),
        RuleRefused::Duplicate => AppError::new("CONFLICT", t("refused.rule.duplicate")),
        RuleRefused::NotFound => AppError::new("NOT_FOUND", t("refused.rule.notFound")),
        RuleRefused::ProRequired => AppError::new("PRO_REQUIRED", t("refused.rule.proRequired")),
        RuleRefused::Store(_) => AppError::new("DB_ERROR", why.to_string()),
    }
}

/// The engine's rule checks (by their English text, decay-engine `rules.rs` and
/// decay-core `matching.rs`) as catalog keys; `None` for anything else.
fn invalid_key(text: &str) -> Option<&'static str> {
    Some(match text {
        "Not a valid domain" => "refused.rule.domain",
        "Match is required" => "refused.rule.matchRequired",
        "Match is too long" => "refused.rule.matchLong",
        "Use the global scope to match every URL" => "refused.rule.global",
        "Exclude rules need a domain, URL or title pattern" => "refused.rule.exclude",
        "Close-allowed rules apply to apps only" => "refused.rule.closeAllowed",
        "Custom threshold rules need thresholds" => "refused.rule.thresholds",
        "This exclude rule matches too many sites; make it more specific" => {
            "refused.rule.tooBroad"
        }
        "Pattern is too long" => "refused.rule.titleLong",
        "Inline flags are not supported" => "refused.rule.titleFlags",
        "Invalid title pattern" => "refused.rule.titleInvalid",
        "Back-references are not supported" => "refused.rule.titleBackrefs",
        "Lookarounds are not supported" => "refused.rule.titleLookarounds",
        "Repeated groups are not supported" => "refused.rule.titleRepeats",
        "Pattern is too complex" => "refused.rule.titleComplex",
        _ => return None,
    })
}

/// The engine's rule checks, in the user's language (anything unknown as written).
fn invalid_text(text: &str) -> String {
    if let Some(key) = invalid_key(text) {
        return t(key);
    }
    let suffix = text
        .strip_prefix('“')
        .and_then(|r| r.split_once("” is a public suffix"))
        .map(|(host, _)| host.to_owned());
    match suffix {
        Some(host) => tf("refused.rule.suffix", &[("host", host.into())]),
        None => text.to_owned(),
    }
}

fn valid_id(id: &str) -> bool {
    (1..=64).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// `create_rule` / `update_rule`: the rule, and how many stored items an exclude purged.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Saved {
    pub rule: Rule,
    pub purged: u32,
}

/// `test_rule`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TestOutput {
    pub matches_live: u32,
    pub matches_archived: u32,
    pub sample: Vec<PanelItem>,
}

pub fn test_output(store: &Store, t: RuleTest) -> TestOutput {
    let mut names = crate::search::RowCache::default();
    TestOutput {
        matches_live: t.matches_live,
        matches_archived: t.matches_archived,
        sample: t.sample.into_iter().map(|i| row(store, i, &mut names)).collect(),
    }
}

/// What a rule call answers through `Core::ask`.
type Answer<T> = (Res<T>, Vec<decay_engine::Output>);

fn saved(r: Result<(Rule, u32, Vec<decay_engine::Output>), RuleRefused>) -> Answer<Saved> {
    match r {
        Ok((rule, purged, out)) => (Ok(Saved { rule, purged }), out),
        Err(why) => (Err(refused(&why)), Vec::new()),
    }
}

fn listed(e: &Engine) -> Res<Vec<Rule>> {
    e.rules().map_err(|err| AppError::db(&err))
}

fn deleted(e: &mut Engine, id: &str, now: i64) -> Answer<Value> {
    match e.delete_rule(id, now) {
        Ok(out) => (Ok(json!({})), out),
        Err(why) => (Err(refused(&why)), Vec::new()),
    }
}

fn tested(e: &Engine, rule: &RuleInput, now: i64) -> Res<TestOutput> {
    e.test_rule(rule, now).map(|t| test_output(e.store(), t)).map_err(|why| refused(&why))
}

fn previewed(e: &Engine, rule: &RuleInput) -> Res<Value> {
    e.preview_exclude(rule).map(|n| json!({ "count": n })).map_err(|why| refused(&why))
}

/// A rule id the UI may pass (what `new_id` makes: letters, digits, dashes).
fn checked_id(id: &str) -> Res<()> {
    if valid_id(id) { Ok(()) } else { Err(AppError::new("INVALID_INPUT", t("error.notRule"))) }
}

/// Runs a change on the engine thread; every window hears about a saved one.
async fn change<T: Send + 'static>(
    app: AppHandle,
    f: impl FnOnce(&mut Engine, i64) -> Answer<T> + Send + 'static,
) -> Res<T> {
    let handle = app.clone();
    let out = blocking(app, move |core| core.ask(f)?).await;
    if out.is_ok() {
        announce(&handle);
    }
    out
}

#[tauri::command]
pub async fn list_rules(app: AppHandle) -> Res<Vec<Rule>> {
    blocking(app, |core| core.ask(|e, _| (listed(e), Vec::new()))?).await
}

#[tauri::command]
pub async fn create_rule(app: AppHandle, rule: RuleInput) -> Res<Saved> {
    change(app, move |e, now| saved(e.create_rule(&rule, now))).await
}

#[tauri::command]
pub async fn update_rule(app: AppHandle, id: String, rule: RuleInput) -> Res<Saved> {
    checked_id(&id)?;
    change(app, move |e, now| saved(e.update_rule(&id, &rule, now))).await
}

#[tauri::command]
pub async fn delete_rule(app: AppHandle, id: String) -> Res<Value> {
    checked_id(&id)?;
    change(app, move |e, now| deleted(e, &id, now)).await
}

/// Tests a rule as it's being written (live tabs, archived items, a few examples).
#[tauri::command]
pub async fn test_rule(app: AppHandle, rule: RuleInput) -> Res<TestOutput> {
    blocking(app, move |core| core.ask(move |e, now| (tested(e, &rule, now), Vec::new()))?).await
}

/// How many stored items saving this exclude rule would delete (to confirm first).
#[tauri::command]
pub async fn preview_exclude(app: AppHandle, rule: RuleInput) -> Res<Value> {
    blocking(app, move |core| core.ask(move |e, _| (previewed(e, &rule), Vec::new()))?).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refusals_map_to_app_errors() {
        assert_eq!(refused(&RuleRefused::Invalid("x".into())).code, "INVALID_INPUT");
        assert_eq!(refused(&RuleRefused::Duplicate).code, "CONFLICT");
        assert_eq!(refused(&RuleRefused::NotFound).code, "NOT_FOUND");
        assert_eq!(
            refused(&RuleRefused::Store(decay_store::StoreError::NotFound)).code,
            "DB_ERROR"
        );
        assert!(valid_id("0b7f3a52-6c1e") && !valid_id("a b") && !valid_id(""));
    }

    #[test]
    fn test_output_uses_panel_rows() {
        let dir = tempfile::tempdir().unwrap();
        let (store, _) = Store::open(
            decay_store::StorePaths::in_dir(dir.path()),
            decay_store::DbKey::from_bytes([7; 32]),
            1,
        )
        .unwrap();
        let mut item = decay_store::Item::new_live(
            "i".into(),
            decay_core::ItemKind::Tab,
            "com.google.Chrome",
            1,
        );
        item.title = "Docs".into();
        let out = test_output(
            &store,
            RuleTest { matches_live: 3, matches_archived: 1, sample: vec![item] },
        );
        let json = serde_json::to_value(&out).unwrap();
        assert_eq!(json["matchesLive"], 3);
        assert_eq!(json["sample"][0]["title"], "Docs");
    }
}

#[cfg(test)]
mod engine_tests {
    use decay_core::{RuleAction, RuleScope};

    use super::*;
    use crate::testing::{NOW, engine};

    fn input(pattern: &str, action: RuleAction) -> RuleInput {
        RuleInput {
            scope: RuleScope::Domain,
            pattern: pattern.into(),
            action,
            threshold_hours: None,
            enabled: None,
        }
    }

    #[test]
    fn rules_are_created_listed_tested_updated_and_deleted() {
        let (mut e, _dir) = engine();
        let before = listed(&e).unwrap().len();
        let (made, _) = saved(e.create_rule(&input("news.example", RuleAction::Protect), NOW));
        let made = made.unwrap();
        assert_eq!((made.rule.pattern.as_str(), made.purged), ("news.example", 0));
        assert_eq!(listed(&e).unwrap().len(), before + 1);
        let (dupe, out) = saved(e.create_rule(&input("news.example", RuleAction::Protect), NOW));
        assert_eq!((dupe.unwrap_err().code, out.len()), ("CONFLICT", 0));
        let (bad, _) = saved(e.create_rule(&input("", RuleAction::Protect), NOW));
        assert_eq!(bad.unwrap_err().code, "INVALID_INPUT");

        let test = tested(&e, &input("news.example", RuleAction::Protect), NOW).unwrap();
        assert_eq!((test.matches_live, test.matches_archived), (0, 0));
        assert_eq!(
            tested(&e, &input("", RuleAction::Protect), NOW).unwrap_err().code,
            "INVALID_INPUT"
        );
        assert_eq!(previewed(&e, &input("news.example", RuleAction::Exclude)).unwrap()["count"], 0);
        assert_eq!(previewed(&e, &input("news.example", RuleAction::Protect)).unwrap()["count"], 0);
        assert_eq!(
            previewed(&e, &input("", RuleAction::Exclude)).unwrap_err().code,
            "INVALID_INPUT"
        );

        let off = RuleInput { enabled: Some(false), ..input("news.example", RuleAction::Protect) };
        let (updated, _) = saved(e.update_rule(&made.rule.id, &off, NOW));
        assert!(!updated.unwrap().rule.enabled);
        let (missing, _) = saved(e.update_rule("nope", &off, NOW));
        assert_eq!(missing.unwrap_err().code, "NOT_FOUND");

        assert!(deleted(&mut e, &made.rule.id, NOW).0.is_ok());
        assert_eq!(deleted(&mut e, &made.rule.id, NOW).0.unwrap_err().code, "NOT_FOUND");
        assert_eq!(listed(&e).unwrap().len(), before);
        assert!(checked_id("0b7f-3a52").is_ok());
        assert_eq!(checked_id("a b").unwrap_err().code, "INVALID_INPUT");
    }
}

#[cfg(test)]
mod i18n_tests {
    use decay_core::{RuleAction, RuleScope};
    use decay_engine::{RuleInput, validate_rule};

    use super::*;

    /// Every refusal the engine's checks give has a key: a change of the engine's wording
    /// fails here instead of showing English in another language.
    #[test]
    fn engine_rule_refusals_are_all_translated() {
        let input = |scope, m: &str, action| RuleInput {
            scope,
            pattern: m.to_owned(),
            action,
            threshold_hours: None,
            enabled: None,
        };
        let long = "a".repeat(5000);
        let cases = [
            input(RuleScope::Domain, "not a domain!", RuleAction::Protect),
            input(RuleScope::Domain, "co.uk", RuleAction::Protect),
            input(RuleScope::Domain, "", RuleAction::Protect),
            input(RuleScope::Domain, &long, RuleAction::Protect),
            input(RuleScope::UrlGlob, "*", RuleAction::Protect),
            input(RuleScope::Global, "", RuleAction::Exclude),
            input(RuleScope::Domain, "example.com", RuleAction::CloseAllowed),
            input(RuleScope::Domain, "example.com", RuleAction::CustomThreshold),
            input(RuleScope::TitleRegex, "(?i)x", RuleAction::Protect),
            input(RuleScope::TitleRegex, "(a)\\1", RuleAction::Protect),
            input(RuleScope::TitleRegex, "a(?=b)", RuleAction::Protect),
            input(RuleScope::TitleRegex, "(", RuleAction::Protect),
        ];
        let mut refused = 0;
        for case in &cases {
            let Err(RuleRefused::Invalid(text)) = validate_rule(case) else { continue };
            refused += 1;
            assert_ne!(invalid_text(&text), "", "{text}");
            let known = invalid_key(&text).is_some() || text.contains("is a public suffix");
            assert!(known, "no key for the engine's {text:?}");
        }
        assert!(refused >= 8, "the cases still reach the checks ({refused})");
    }
}
