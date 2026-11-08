//! Rules (DB Design §3.8), as `decay_core::Rule`.

use decay_core::{Rule, SqlEnum, Thresholds};
use rusqlite::{ErrorCode, params};

use crate::items::sql_enum;
use crate::{Result, Store, StoreError};

/// `SQLITE_CONSTRAINT_UNIQUE`: the only UNIQUE constraint on `rules` is
/// `(scope, match, action)` (a duplicate id is `SQLITE_CONSTRAINT_PRIMARYKEY`).
const SQLITE_CONSTRAINT_UNIQUE: i32 = 2067;

fn map_unique(err: rusqlite::Error) -> StoreError {
    match &err {
        rusqlite::Error::SqliteFailure(e, _)
            if e.code == ErrorCode::ConstraintViolation
                && e.extended_code == SQLITE_CONSTRAINT_UNIQUE =>
        {
            StoreError::DuplicateRule
        }
        _ => err.into(),
    }
}

fn thresholds_json(rule: &Rule) -> Result<Option<String>> {
    rule.threshold_hours
        .map(|t| serde_json::to_string(&t))
        .transpose()
        .map_err(|e| StoreError::InvalidData(e.to_string()))
}

impl Store {
    /// Every rule, oldest first (the policy's tie-break order).
    pub fn rules(&self) -> Result<Vec<Rule>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT id, scope, match, action, threshold_hours, enabled, source, created_at, hit_count
             FROM rules ORDER BY created_at, id",
        )?;
        let mut rows = stmt.query([])?;
        let mut out = Vec::new();
        while let Some(r) = rows.next()? {
            let thresholds: Option<String> = r.get(4)?;
            out.push(Rule {
                id: r.get(0)?,
                scope: sql_enum(&r.get::<_, String>(1)?)?,
                pattern: r.get(2)?,
                action: sql_enum(&r.get::<_, String>(3)?)?,
                // Malformed thresholds read as none: the policy then ignores the rule's
                // custom threshold, as the extension does.
                threshold_hours: thresholds
                    .and_then(|t| serde_json::from_str::<Thresholds>(&t).ok()),
                enabled: r.get(5)?,
                source: sql_enum(&r.get::<_, String>(6)?)?,
                created_at: r.get(7)?,
                hit_count: r.get(8)?,
            });
        }
        Ok(out)
    }

    /// Adds a rule; [`StoreError::DuplicateRule`] if one with the same scope, match and
    /// action exists.
    pub fn insert_rule(&self, rule: &Rule) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO rules (id, scope, match, action, threshold_hours, enabled, source,
                   created_at, hit_count) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    rule.id,
                    rule.scope.as_sql(),
                    rule.pattern,
                    rule.action.as_sql(),
                    thresholds_json(rule)?,
                    rule.enabled,
                    rule.source.as_sql(),
                    rule.created_at,
                    rule.hit_count
                ],
            )
            .map_err(map_unique)?;
        Ok(())
    }

    /// Saves a rule's editable fields (scope, match, action, thresholds, enabled).
    pub fn update_rule(&self, rule: &Rule) -> Result<()> {
        let n = self
            .conn
            .execute(
                "UPDATE rules SET scope = ?2, match = ?3, action = ?4, threshold_hours = ?5,
                   enabled = ?6 WHERE id = ?1",
                params![
                    rule.id,
                    rule.scope.as_sql(),
                    rule.pattern,
                    rule.action.as_sql(),
                    thresholds_json(rule)?,
                    rule.enabled
                ],
            )
            .map_err(map_unique)?;
        if n == 0 { Err(StoreError::NotFound) } else { Ok(()) }
    }

    pub fn delete_rule(&self, id: &str) -> Result<bool> {
        Ok(self.conn.execute("DELETE FROM rules WHERE id = ?1", [id])? > 0)
    }

    /// Counts matches shown in the rules UI.
    pub fn add_rule_hits(&self, id: &str, hits: u64) -> Result<()> {
        self.conn.execute(
            "UPDATE rules SET hit_count = hit_count + min(?2, 9223372036854775807 - hit_count)
             WHERE id = ?1",
            params![id, hits.min(i64::MAX as u64)],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::*;
    use decay_core::{RuleAction, RuleScope, RuleSource};

    fn rule(id: &str) -> Rule {
        Rule {
            id: id.into(),
            scope: RuleScope::UrlGlob,
            pattern: "https://*.notion.so/*".into(),
            action: RuleAction::CustomThreshold,
            threshold_hours: Some(Thresholds { stale: 2.0, hidden: 4.0, archive: 8.0 }),
            enabled: true,
            source: RuleSource::QuickAction,
            created_at: NOW,
            hit_count: 0,
        }
    }

    #[test]
    fn ships_the_default_rules() {
        let (store, _dir) = store();
        let rules = store.rules().unwrap();
        let protected: Vec<&str> = rules
            .iter()
            .filter(|r| r.action == RuleAction::Protect)
            .map(|r| r.pattern.as_str())
            .collect();
        assert_eq!(
            protected,
            ["accounts.google.com", "meet.google.com", "teams.microsoft.com", "zoom.us"]
        );
        assert!(rules.iter().any(|r| r.action == RuleAction::CloseAllowed
            && r.scope == RuleScope::App
            && r.pattern == "com.apple.finder"));
    }

    #[test]
    fn rules_round_trip_and_stay_unique() {
        let (store, _dir) = store();
        let mut r = rule("r1");
        store.insert_rule(&r).unwrap();
        assert!(matches!(store.insert_rule(&rule("r2")), Err(StoreError::DuplicateRule)));
        r.enabled = false;
        store.update_rule(&r).unwrap();
        store.add_rule_hits("r1", 3).unwrap();
        let stored = store.rules().unwrap().into_iter().find(|x| x.id == "r1").unwrap();
        assert_eq!(stored, Rule { hit_count: 3, ..r });
        assert!(store.delete_rule("r1").unwrap());
        assert!(matches!(store.update_rule(&rule("r1")), Err(StoreError::NotFound)));
    }
}
