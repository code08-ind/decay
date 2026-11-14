//! Rule commands (P1.12, API Design §2.4, PRD F8/F9): list, create, update, delete, test a
//! rule against open tabs and the archive, and preview what an exclude rule would delete.

#![allow(clippy::needless_pass_by_value, reason = "Tauri commands take their arguments by value")]

use std::collections::HashMap;

use decay_core::Rule;
use decay_engine::{RuleInput, RuleRefused, RuleTest};
use decay_store::Store;
use serde::Serialize;
use serde_json::{Value, json};
use tauri::AppHandle;

use crate::commands::{Res, blocking};
use crate::core::AppError;
use crate::search::{PanelItem, row};

/// Tells every window the rules changed (Settings reloads its lists).
pub fn announce(app: &AppHandle) {
    let _ = tauri::Emitter::emit(app, "rules:changed", ());
}

pub fn refused(why: &RuleRefused) -> AppError {
    let code = match why {
        RuleRefused::Invalid(_) => "INVALID_INPUT",
        RuleRefused::Duplicate => "CONFLICT",
        RuleRefused::NotFound => "NOT_FOUND",
        RuleRefused::Store(_) => "DB_ERROR",
    };
    AppError::new(code, why.to_string())
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
    let mut names = HashMap::new();
    TestOutput {
        matches_live: t.matches_live,
        matches_archived: t.matches_archived,
        sample: t.sample.into_iter().map(|i| row(store, i, &mut names)).collect(),
    }
}

#[tauri::command]
pub async fn list_rules(app: AppHandle) -> Res<Vec<Rule>> {
    blocking(app, |core| {
        core.ask(|e, _| (e.rules().map_err(|err| AppError::db(&err)), Vec::new()))?
    })
    .await
}

#[tauri::command]
pub async fn create_rule(app: AppHandle, rule: RuleInput) -> Res<Saved> {
    let handle = app.clone();
    let out = blocking(app, move |core| {
        core.ask(move |e, now| match e.create_rule(&rule, now) {
            Ok((rule, purged, out)) => (Ok(Saved { rule, purged }), out),
            Err(why) => (Err(refused(&why)), Vec::new()),
        })?
    })
    .await;
    if out.is_ok() {
        announce(&handle);
    }
    out
}

#[tauri::command]
pub async fn update_rule(app: AppHandle, id: String, rule: RuleInput) -> Res<Saved> {
    if !valid_id(&id) {
        return Err(AppError::new("INVALID_INPUT", "not a rule id"));
    }
    let handle = app.clone();
    let out = blocking(app, move |core| {
        core.ask(move |e, now| match e.update_rule(&id, &rule, now) {
            Ok((rule, purged, out)) => (Ok(Saved { rule, purged }), out),
            Err(why) => (Err(refused(&why)), Vec::new()),
        })?
    })
    .await;
    if out.is_ok() {
        announce(&handle);
    }
    out
}

#[tauri::command]
pub async fn delete_rule(app: AppHandle, id: String) -> Res<Value> {
    if !valid_id(&id) {
        return Err(AppError::new("INVALID_INPUT", "not a rule id"));
    }
    let handle = app.clone();
    let out = blocking(app, move |core| {
        core.ask(move |e, now| match e.delete_rule(&id, now) {
            Ok(out) => (Ok(json!({})), out),
            Err(why) => (Err(refused(&why)), Vec::new()),
        })?
    })
    .await;
    if out.is_ok() {
        announce(&handle);
    }
    out
}

/// Tests a rule as it's being written (live tabs, archived items, a few examples).
#[tauri::command]
pub async fn test_rule(app: AppHandle, rule: RuleInput) -> Res<TestOutput> {
    blocking(app, move |core| {
        core.ask(move |e, now| match e.test_rule(&rule, now) {
            Ok(t) => (Ok(test_output(e.store(), t)), Vec::new()),
            Err(why) => (Err(refused(&why)), Vec::new()),
        })?
    })
    .await
}

/// How many stored items saving this exclude rule would delete (to confirm first).
#[tauri::command]
pub async fn preview_exclude(app: AppHandle, rule: RuleInput) -> Res<Value> {
    blocking(app, move |core| {
        core.ask(move |e, _| match e.preview_exclude(&rule) {
            Ok(n) => (Ok(json!({ "count": n })), Vec::new()),
            Err(why) => (Err(refused(&why)), Vec::new()),
        })?
    })
    .await
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
