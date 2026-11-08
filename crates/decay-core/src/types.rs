//! Domain types shared by every Decay crate (Software Design §4.1, API Design §1).
//!
//! JSON uses the API's camelCase names (so the UI, the extension and stored `Reason`
//! JSON agree); SQL `CHECK` values use `snake_case`, via [`SqlEnum`].

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// A `TEXT` enum column with a `CHECK IN (…)` constraint (DB Design §3).
pub trait SqlEnum: Sized + Copy + 'static {
    /// Every value, in declaration order.
    const ALL: &'static [Self];
    /// The value stored in SQL.
    fn as_sql(self) -> &'static str;
    /// Parses a stored SQL value.
    fn from_sql(value: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|v| v.as_sql() == value)
    }
}

macro_rules! sql_enum {
    ($(#[$meta:meta])* $name:ident { $($(#[$vmeta:meta])* $variant:ident => $sql:literal),+ $(,)? }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        $(#[$meta])*
        pub enum $name {
            $($(#[$vmeta])* $variant),+
        }

        impl SqlEnum for $name {
            const ALL: &'static [Self] = &[$(Self::$variant),+];
            fn as_sql(self) -> &'static str {
                match self {
                    $(Self::$variant => $sql),+
                }
            }
        }
    };
}

sql_enum! {
    /// A browser tab or an app window.
    #[serde(rename_all = "camelCase")]
    ItemKind { Tab => "tab", Window => "window" }
}

sql_enum! {
    /// Lifecycle state of an item (see [`crate::lifecycle`]).
    #[serde(rename_all = "camelCase")]
    ItemState {
        Active => "active",
        Stale => "stale",
        Hidden => "hidden",
        /// Write-before-close in progress (Software Design §4.4).
        Archiving => "archiving",
        Archived => "archived",
        /// An archived record that was reopened (the reopened item is a new live record).
        Restored => "restored",
        Purged => "purged",
    }
}

impl ItemState {
    /// Open in the browser or on screen right now.
    pub const fn is_live(self) -> bool {
        matches!(self, Self::Active | Self::Stale | Self::Hidden | Self::Archiving)
    }
}

sql_enum! {
    /// Whether a window's document is saved (windows only; tabs are `n/a`).
    #[derive(Default)]
    SaveState {
        #[serde(rename = "saved")] Saved => "saved",
        #[serde(rename = "unsaved")] Unsaved => "unsaved",
        #[serde(rename = "unknown")] Unknown => "unknown",
        #[default] #[serde(rename = "n/a")] NotApplicable => "n/a",
    }
}

sql_enum! {
    /// Global mode. Shadow mode simulates sweeps; paused does nothing.
    #[serde(rename_all = "camelCase")]
    Mode { Shadow => "shadow", Automatic => "automatic", Paused => "paused" }
}

sql_enum! {
    /// Why an item is protected (API Design §1 `Reason.by`).
    #[serde(rename_all = "camelCase")]
    ProtectedBy {
        Pinned => "pinned",
        ActiveTab => "active_tab",
        Audible => "audible",
        FormInput => "form_input",
        Download => "download",
        Call => "call",
        Unsaved => "unsaved",
        UnknownSaveState => "unknown_save_state",
        Rule => "rule",
        Hold => "hold",
        Meeting => "meeting",
        Grace => "grace",
        /// The URL can't be reopened exactly (data:, blob:, file:, …).
        Unrestorable => "unrestorable",
    }
}

/// Machine-readable reason for every state change; stored as JSON in `items.archive_reason`
/// and `events.reason`. JSON has the extension's shape and values, e.g.
/// `{"type":"unused","activeHours":61,"threshold":60}` (Rust may print `60.0`; both
/// parse to the same value).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Reason {
    /// Unused for `active_hours` (whole hours) against an archive threshold in hours.
    Unused { active_hours: u64, threshold: f64 },
    /// Archived under a rule's custom threshold.
    CustomRule { rule_id: String, active_hours: u64 },
    /// Kept because of a protection.
    Protected {
        by: ProtectedBy,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rule_id: Option<String>,
    },
}

/// Allowed range of each threshold, in active hours (API Design `update_settings`).
pub const THRESHOLD_MIN_H: f64 = 1.0;
/// Upper bound of each threshold, in active hours (30 days).
pub const THRESHOLD_MAX_H: f64 = 720.0;

/// Active hours until an item turns stale, is hidden (windows), and is archived.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Thresholds {
    pub stale: f64,
    pub hidden: f64,
    pub archive: f64,
}

/// Invalid thresholds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("thresholds must be 1–720 h and ordered stale < hidden < archive")]
pub struct ThresholdsError;

impl Thresholds {
    /// Gentle preset.
    pub const GENTLE: Self = Self { stale: 24.0, hidden: 60.0, archive: 96.0 };
    /// Balanced preset (the default, PRD §1.1).
    pub const BALANCED: Self = Self { stale: 12.0, hidden: 36.0, archive: 60.0 };
    /// Aggressive preset.
    pub const AGGRESSIVE: Self = Self { stale: 8.0, hidden: 24.0, archive: 40.0 };

    /// Each value finite and in range, and strictly ordered.
    pub fn validate(&self) -> Result<(), ThresholdsError> {
        let in_range = |h: f64| h.is_finite() && (THRESHOLD_MIN_H..=THRESHOLD_MAX_H).contains(&h);
        let ok = [self.stale, self.hidden, self.archive].into_iter().all(in_range)
            && self.stale < self.hidden
            && self.hidden < self.archive;
        if ok { Ok(()) } else { Err(ThresholdsError) }
    }

    pub fn is_valid(&self) -> bool {
        self.validate().is_ok()
    }
}

impl Default for Thresholds {
    fn default() -> Self {
        Self::BALANCED
    }
}

sql_enum! {
    /// What a rule matches against.
    #[serde(rename_all = "camelCase")]
    RuleScope {
        App => "app",
        Domain => "domain",
        UrlGlob => "url_glob",
        TitleRegex => "title_regex",
        Global => "global",
    }
}

impl RuleScope {
    /// Most specific first (Software Design §4.1): URL glob > title > domain > app > global.
    pub const fn specificity(self) -> u8 {
        match self {
            Self::UrlGlob => 0,
            Self::TitleRegex => 1,
            Self::Domain => 2,
            Self::App => 3,
            Self::Global => 4,
        }
    }
}

sql_enum! {
    /// What a matching rule does. `Exclude` items are never stored (F9).
    #[serde(rename_all = "camelCase")]
    RuleAction {
        Protect => "protect",
        Exclude => "exclude",
        CustomThreshold => "custom_threshold",
        CloseAllowed => "close_allowed",
    }
}

sql_enum! {
    /// Who created a rule.
    #[derive(Default)]
    RuleSource {
        #[default] #[serde(rename = "user")] User => "user",
        #[serde(rename = "default")] Default => "default",
        /// Created from a one-click suggestion (Undo, "Always keep").
        #[serde(rename = "quick_action")] QuickAction => "quick_action",
    }
}

fn lenient<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Thresholds>, D::Error> {
    let value = Option::<serde_json::Value>::deserialize(d)?;
    Ok(value.and_then(|v| serde_json::from_value(v).ok()))
}

/// Reads a JSON array of rules (an import, the extension's data), skipping the ones that
/// can't be understood (unknown scope or action, wrong types), as the extension ignores
/// them: one bad rule never drops the others. `None` if it isn't an array.
pub fn rules_from_json(json: &str) -> Option<Vec<Rule>> {
    let values: Vec<serde_json::Value> = serde_json::from_str(json).ok()?;
    Some(values.into_iter().filter_map(|v| serde_json::from_value(v).ok()).collect())
}

/// A protection / exclusion / custom-threshold rule (F8, F9).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Rule {
    pub id: String,
    pub scope: RuleScope,
    /// App id, domain, URL glob or title pattern, depending on `scope`.
    #[serde(rename = "match")]
    pub pattern: String,
    pub action: RuleAction,
    /// Malformed thresholds read as none (the extension ignores them the same way).
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "lenient")]
    pub threshold_hours: Option<Thresholds>,
    /// Missing reads as disabled, like the extension's `!rule.enabled`.
    #[serde(default)]
    pub enabled: bool,
    /// Optional in API Design's `Rule`; older records may lack it.
    #[serde(default)]
    pub source: RuleSource,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub hit_count: u64,
}

sql_enum! {
    /// Event log types (DB Design `events.type`).
    EventType {
        #[serde(rename = "seen")] Seen => "seen",
        #[serde(rename = "used")] Used => "used",
        #[serde(rename = "stale")] Stale => "stale",
        #[serde(rename = "hidden")] Hidden => "hidden",
        #[serde(rename = "archived")] Archived => "archived",
        #[serde(rename = "close_failed")] CloseFailed => "close_failed",
        #[serde(rename = "restored")] Restored => "restored",
        #[serde(rename = "undo")] Undo => "undo",
        #[serde(rename = "protected")] Protected => "protected",
        #[serde(rename = "purged")] Purged => "purged",
        #[serde(rename = "shadow_would_archive")] ShadowWouldArchive => "shadow_would_archive",
    }
}

sql_enum! {
    /// What started a sweep.
    #[serde(rename_all = "camelCase")]
    SweepTrigger { Scheduled => "scheduled", Wake => "wake", Manual => "manual" }
}

sql_enum! {
    /// Sweep progress.
    #[serde(rename_all = "camelCase")]
    SweepStatus {
        Pending => "pending",
        Running => "running",
        Done => "done",
        Failed => "failed",
        Undone => "undone",
    }
}

sql_enum! {
    /// What a sweep does to one item.
    #[serde(rename_all = "camelCase")]
    SweepAction { Archive => "archive", Hide => "hide", Close => "close" }
}

sql_enum! {
    /// Outcome for one item of a sweep.
    #[serde(rename_all = "camelCase")]
    SweepItemResult { Pending => "pending", Ok => "ok", Failed => "failed", Skipped => "skipped" }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reason_json_matches_the_extension() {
        let unused = Reason::Unused { active_hours: 61, threshold: 60.0 };
        assert_eq!(
            serde_json::to_value(&unused).unwrap(),
            json!({"type":"unused","activeHours":61,"threshold":60.0})
        );
        let rule = Reason::CustomRule { rule_id: "r1".into(), active_hours: 4 };
        assert_eq!(
            serde_json::to_value(&rule).unwrap(),
            json!({"type":"customRule","ruleId":"r1","activeHours":4})
        );
        let by = Reason::Protected { by: ProtectedBy::ActiveTab, rule_id: None };
        assert_eq!(
            serde_json::to_value(&by).unwrap(),
            json!({"type":"protected","by":"activeTab"})
        );
        // Stored by the extension (integers for hours) and read back here.
        let stored: Reason =
            serde_json::from_str(r#"{"type":"unused","activeHours":62,"threshold":120}"#).unwrap();
        assert_eq!(stored, Reason::Unused { active_hours: 62, threshold: 120.0 });
    }

    #[test]
    fn one_bad_rule_does_not_drop_the_others() {
        let json = r#"[
            {"id":"a","scope":"domain","match":"x.com","action":"protect","enabled":true},
            {"id":"b","scope":"someday","match":"x","action":"protect","enabled":true},
            {"id":"c","scope":"domain","match":"y.com","action":"protect"},
            {"id":"d","scope":"global","match":"*","action":"customThreshold","enabled":true,
             "thresholdHours":{"stale":1}}
        ]"#;
        let rules = rules_from_json(json).unwrap();
        let ids: Vec<&str> = rules.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ["a", "c", "d"]);
        assert!(!rules[1].enabled);
        assert_eq!(rules[2].threshold_hours, None);
        assert_eq!(rules_from_json("{}"), None);
    }

    #[test]
    fn rule_json_round_trips_with_the_extension_shape() {
        let json = r#"{"id":"r1","scope":"urlGlob","match":"https://a.com/*","action":"customThreshold","thresholdHours":{"stale":1,"hidden":2,"archive":3},"enabled":true,"source":"quick_action","createdAt":5,"hitCount":0}"#;
        let rule: Rule = serde_json::from_str(json).unwrap();
        assert_eq!(rule.scope, RuleScope::UrlGlob);
        assert_eq!(rule.action, RuleAction::CustomThreshold);
        assert_eq!(rule.source, RuleSource::QuickAction);
        assert_eq!(rule.pattern, "https://a.com/*");
        let back: Rule = serde_json::from_value(serde_json::to_value(&rule).unwrap()).unwrap();
        assert_eq!(back, rule);
    }

    #[test]
    fn sql_values_match_the_check_constraints() {
        assert_eq!(RuleScope::UrlGlob.as_sql(), "url_glob");
        assert_eq!(RuleAction::CustomThreshold.as_sql(), "custom_threshold");
        assert_eq!(SaveState::NotApplicable.as_sql(), "n/a");
        assert_eq!(EventType::ShadowWouldArchive.as_sql(), "shadow_would_archive");
        for s in ItemState::ALL {
            assert_eq!(ItemState::from_sql(s.as_sql()), Some(*s));
        }
        assert_eq!(ItemState::from_sql("nope"), None);
    }

    #[test]
    fn thresholds_validation() {
        assert!(Thresholds::BALANCED.is_valid());
        assert!(Thresholds::GENTLE.is_valid() && Thresholds::AGGRESSIVE.is_valid());
        for bad in [
            Thresholds { stale: 0.0, hidden: 0.0, archive: 0.0 },
            Thresholds { stale: 12.0, hidden: 12.0, archive: 60.0 },
            Thresholds { stale: 12.0, hidden: 36.0, archive: 721.0 },
            Thresholds { stale: f64::NAN, hidden: 36.0, archive: 60.0 },
        ] {
            assert_eq!(bad.validate(), Err(ThresholdsError), "{bad:?}");
        }
    }
}
