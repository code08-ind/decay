//! Importing a file the user picked (P2.5, PRD F20): a Decay JSON export, the extension's
//! archive, or the items made from a bookmarks file.
//!
//! Safety, in order:
//! 1. Anything that isn't an archive (another app's JSON, a newer format) is refused
//!    before anything changes.
//! 2. Every rule in the file is checked as if added by hand; refused ones are counted
//!    as invalid and never stored or applied.
//! 3. A `decay-preimport-<ms>.db` backup is written first.
//! 4. "Never record" sites stay unrecorded: incoming items matching an enabled exclude
//!    rule — this installation's or one the file brings — are dropped, and stored items
//!    matching an exclude rule the file brings are deleted in the import's transaction,
//!    as adding the rule by hand does. Like adding it by hand, that needs a confirmation:
//!    [`Engine::import_purge_count`] says how many first, and the import is refused
//!    ([`ImportRefused::NeedsConfirm`]) unless the caller allows that many.
//! 5. The file never sets the mode, the shadow week, a hold or the retention, and items
//!    older than this installation keeps aren't imported; times are clamped to now, text
//!    is capped, tabs need an address they can reopen, a live record is never changed,
//!    and bookmarks never update a record ([`decay_store::Store::import_file`]).
//! 6. Every item's site comes from its address, not the file.

use decay_core::{MatchTarget, Rule, RuleAction, RuleSource, rule_matches};
use decay_store::{FileOptions, ImportReport, StoreError, catalog_app_id};
use serde::Serialize;
use serde_json::Value;

use crate::domain::domain_of;
use crate::engine::{Engine, Output};
use crate::rules::{RuleInput, validate_rule};

/// Why a file wasn't imported (nothing changed).
#[derive(Debug, thiserror::Error)]
pub enum ImportRefused {
    #[error("This file isn't a Decay archive.")]
    NotAnArchive,
    #[error("This archive was made by a newer version of Decay (format {0}). Update Decay first.")]
    NewerFormat(u64),
    /// The file's "never record" rules would delete this many stored items, more than the
    /// caller allowed.
    #[error("This file's never-record rules would delete {0} stored items.")]
    NeedsConfirm(usize),
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// How to import a file.
#[derive(Debug, Clone, Copy, Default)]
pub struct ImportOptions {
    /// The items were made from a bookmarks file.
    pub bookmarks: bool,
    /// Stored items the file's exclude rules may delete (what the user confirmed).
    pub allow_purge: usize,
}

/// What importing a file did.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileImport {
    #[serde(flatten)]
    pub report: ImportReport,
    /// Incoming items on sites Decay never records (not imported).
    pub items_excluded: u32,
}

/// The JSON format versions this build reads.
const FORMAT_VERSION: u64 = 1;

/// Checks the document is an archive this build can read.
fn check_format(doc: &Value) -> Result<(), ImportRefused> {
    let Some(obj) = doc.as_object() else { return Err(ImportRefused::NotAnArchive) };
    match obj.get("format") {
        Some(Value::String(f)) if f == "decay-archive" => {
            match obj.get("version").and_then(Value::as_u64) {
                Some(v) if v > FORMAT_VERSION => Err(ImportRefused::NewerFormat(v)),
                Some(_) if obj.get("items").is_some_and(Value::is_array) => Ok(()),
                _ => Err(ImportRefused::NotAnArchive),
            }
        }
        // The extension's archive (and bookmark items) carry no header: items are enough.
        None if obj.get("items").is_some_and(Value::is_array) => Ok(()),
        Some(_) | None => Err(ImportRefused::NotAnArchive),
    }
}

fn enabled_excludes(rules: impl IntoIterator<Item = Rule>) -> Vec<Rule> {
    rules.into_iter().filter(|r| r.enabled && r.action == RuleAction::Exclude).collect()
}

/// The file's rules, each checked as if added by hand ([`validate_rule`]) and stored
/// normalized: a crafted file mustn't bring a rule the app refuses (an exclude matching
/// every page or a whole browser would delete the archive). Defaults pass unchanged (the
/// store only uses them to turn a default off). Returns the rules and how many were
/// refused.
fn checked_rules(raw: &[Value]) -> (Vec<Value>, Vec<Rule>, u32) {
    let (mut keep, mut rules, mut refused) = (Vec::new(), Vec::new(), 0_u32);
    for value in raw {
        let Ok(mut rule) = serde_json::from_value::<Rule>(value.clone()) else {
            keep.push(value.clone()); // the store counts what it can't read
            continue;
        };
        if rule.source != RuleSource::Default {
            let input = RuleInput {
                scope: rule.scope,
                pattern: rule.pattern.clone(),
                action: rule.action,
                threshold_hours: rule.threshold_hours,
                enabled: Some(rule.enabled),
            };
            let Ok((scope, pattern, action, thresholds)) = validate_rule(&input) else {
                refused += 1;
                continue;
            };
            (rule.scope, rule.pattern, rule.action, rule.threshold_hours) =
                (scope, pattern, action, thresholds);
        }
        keep.push(serde_json::to_value(&rule).unwrap_or(Value::Null));
        rules.push(rule);
    }
    (keep, rules, refused)
}

/// What a file would do to the stored archive, worked out before anything changes.
struct Plan {
    /// The file's rules, checked (what the store gets).
    rules: Vec<Value>,
    rules_refused: u32,
    /// Enabled exclude rules: this installation's and the file's new ones.
    excludes: Vec<Rule>,
    /// Stored items the file's new exclude rules cover: deleted now / once a close settles.
    purge: Vec<String>,
    later: Vec<String>,
}

impl Engine {
    /// How many stored items importing `doc` would delete (its new "never record" rules
    /// cover them), for a confirmation first. Changes nothing.
    pub fn import_purge_count(&self, doc: &Value) -> Result<usize, ImportRefused> {
        check_format(doc)?;
        Ok(self.plan_import(doc)?.purge.len())
    }

    fn plan_import(&self, doc: &Value) -> Result<Plan, ImportRefused> {
        let empty = Vec::new();
        let raw_rules = doc.get("rules").and_then(Value::as_array).unwrap_or(&empty);
        let (checked, rules, rules_refused) = checked_rules(raw_rules);

        // Exclude rules the file brings (new ones only: an id or pattern already here
        // changes nothing; defaults are never added).
        let known = self.store.rules()?;
        let new_excludes = enabled_excludes(rules.into_iter().filter(|r| {
            r.source != RuleSource::Default
                && !known.iter().any(|k| {
                    k.id == r.id
                        || (k.scope == r.scope && k.pattern == r.pattern && k.action == r.action)
                })
        }));
        let mut excludes = enabled_excludes(known);
        excludes.extend(new_excludes.iter().cloned());

        // Stored items those new rules cover (before the import adds more).
        let (mut purge, mut later) = (Vec::new(), Vec::new());
        for rule in &new_excludes {
            let (now_ids, later_ids) = self.matching_items(rule)?;
            purge.extend(now_ids);
            later.extend(later_ids);
        }
        purge.sort();
        purge.dedup();
        Ok(Plan { rules: checked, rules_refused, excludes, purge, later })
    }

    /// Imports a file's content (see the module docs).
    pub fn import_from_app(
        &mut self,
        mut doc: Value,
        now: i64,
        options: ImportOptions,
    ) -> Result<(FileImport, Vec<Output>), ImportRefused> {
        check_format(&doc)?;
        let platform = self.config.platform;
        let Plan { rules, rules_refused, excludes, purge, later } = self.plan_import(&doc)?;
        if purge.len() > options.allow_purge {
            return Err(ImportRefused::NeedsConfirm(purge.len()));
        }
        if doc.get("rules").is_some_and(Value::is_array) {
            doc["rules"] = Value::Array(rules);
        }

        // Incoming items on never-recorded sites are dropped; the others get their site
        // from their address (bookmarks carry none, and a file's own could be anything)
        // so `site:` search and grouping find them.
        let mut dropped = 0_u32;
        if let Some(items) = doc.get_mut("items").and_then(Value::as_array_mut) {
            items.retain_mut(|raw| {
                let app = catalog_app_id(
                    raw.get("appId").and_then(Value::as_str).unwrap_or_default(),
                    platform,
                );
                let url = raw.get("url").and_then(Value::as_str);
                let target = MatchTarget {
                    app_id: &app,
                    url,
                    title: raw.get("title").and_then(Value::as_str).unwrap_or_default(),
                };
                if excludes.iter().any(|r| rule_matches(r, &target)) {
                    dropped += 1;
                    return false;
                }
                let domain = url.and_then(domain_of).map_or(Value::Null, Value::from);
                if let Some(obj) = raw.as_object_mut() {
                    obj.insert("domain".into(), domain);
                }
                true
            });
        }

        // A copy of the data first: an import can be rolled back from the backups.
        self.store.backup_before_import(now)?;
        let before = self.policy.excluded_domains();
        let days = i64::try_from(self.policy.retention_days).unwrap_or(i64::MAX / 86_400_001);
        let opts = FileOptions {
            retention_cutoff: now.saturating_sub(days.saturating_mul(86_400_000)),
            purge: purge.clone(),
            bookmarks: options.bookmarks,
        };
        let mut report = self.store.import_file(&doc, platform, now, &opts)?;
        report.rules_invalid += rules_refused;
        self.forget_items(&purge);
        self.exclude_pending.extend(later);
        self.after_import(&report, &before, now);
        self.publish_status(now);
        let result = FileImport { report, items_excluded: dropped };
        Ok((result, std::mem::take(&mut self.out)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn only_archives_this_build_reads_are_accepted() {
        assert!(
            check_format(&json!({"format": "decay-archive", "version": 1, "items": []})).is_ok()
        );
        assert!(check_format(&json!({"items": []})).is_ok(), "the extension's archive");
        assert!(matches!(
            check_format(&json!({"format": "decay-archive", "version": 2})),
            Err(ImportRefused::NewerFormat(2))
        ));
        for odd in [
            json!({"format": "decay-beta-metrics", "version": 1}),
            json!({"format": "decay-archive"}),
            json!({"format": "decay-archive", "version": 1}),
            json!({"rules": []}),
            json!([1, 2]),
            json!(null),
        ] {
            assert!(matches!(check_format(&odd), Err(ImportRefused::NotAnArchive)), "{odd}");
        }
    }
}
