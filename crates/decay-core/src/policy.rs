//! `Policy::evaluate` (Software Design §4.1). Pure: no I/O. Principle: fail by doing
//! nothing; when in doubt, protect.
//!
//! Order: global hold/pause → hard protections → user rules → thresholds. Any matching
//! `protect` rule wins (safety first); otherwise the most specific valid `customThreshold`
//! replaces the global thresholds. `exclude` rules are applied before policy: excluded
//! items are never stored ([`is_excluded`]). Tabs behave exactly like the extension's
//! `lib/policy/policy.ts`; windows add hiding and Pro close rules (F4, F11).

use regex::Regex;
use std::sync::LazyLock;

use crate::matching::{MatchTarget, rule_matches};
use crate::types::{
    ItemKind, Mode, ProtectedBy, Reason, Rule, RuleAction, RuleScope, SaveState, Thresholds,
};

/// Archive-threshold multiplier for web pages whose form state can't be checked (PRD §7.3).
pub const UNKNOWN_FORM_FACTOR: f64 = 2.0;

/// Global inputs to a decision.
#[derive(Debug, Clone, Copy)]
pub struct PolicyContext<'a> {
    /// Unix epoch milliseconds.
    pub now: i64,
    pub mode: Mode,
    /// Manual hold until this time (epoch ms), if any.
    pub hold_until: Option<i64>,
    pub thresholds: Thresholds,
    pub rules: &'a [Rule],
    /// Pro features enabled (closing allowlisted windows, F11).
    pub pro: bool,
}

/// The item being decided on.
#[derive(Debug, Clone, Copy)]
pub struct PolicyTarget<'a> {
    pub kind: ItemKind,
    pub app_id: &'a str,
    pub url: Option<&'a str>,
    pub title: &'a str,
    pub pinned: bool,
    /// Active seconds since last use.
    pub active_age_s: u64,
    /// Protected until this time (epoch ms), e.g. 48 h after an Undo.
    pub grace_until: Option<i64>,
}

impl<'a> PolicyTarget<'a> {
    fn match_target(&self) -> MatchTarget<'a> {
        MatchTarget { app_id: self.app_id, url: self.url, title: self.title }
    }
}

/// Typed input / "Leave site?" state of a web page, from the form probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FormInput {
    /// Not a concern (non-web pages, windows).
    #[default]
    NotApplicable,
    Clean,
    Dirty,
    /// Couldn't be checked: web pages wait [`UNKNOWN_FORM_FACTOR`] × the archive threshold.
    Unknown,
}

/// Live signals for an item at decision time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Signals {
    /// The active tab of its window, or the focused window.
    pub in_use: bool,
    pub audible: bool,
    pub form_input: FormInput,
    /// Windows: whether the document is saved. Tabs: `NotApplicable`.
    pub save_state: SaveState,
}

/// What to do with an item now.
#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    /// Nothing to do yet.
    Keep { active_hours: f64, threshold: Thresholds },
    /// Never touched while this holds.
    Protect { by: ProtectedBy, rule_id: Option<String> },
    /// Old enough to show as stale (a visual hint only).
    Stale { active_hours: f64, threshold: Thresholds, rule_id: Option<String> },
    /// Windows: old enough to hide (minimise) at the next sweep. `kept_by` says why an
    /// allowlisted window past the archive threshold is hidden instead of closed (its
    /// document is unsaved, or its save state unknown).
    Hide {
        active_hours: f64,
        threshold: Thresholds,
        rule_id: Option<String>,
        kept_by: Option<ProtectedBy>,
    },
    /// Old enough to archive (tabs) or close (allowlisted saved windows) at the next sweep.
    /// The policy doesn't know lifecycle states: the engine closes a window only once it's
    /// Hidden (`BeginArchive` is refused otherwise), so a window that can't be minimised is
    /// never closed (when in doubt, keep it).
    Archive { reason: Reason },
}

impl Decision {
    /// The `Reason` to store for this decision, if it has one.
    pub fn reason(&self) -> Option<Reason> {
        match self {
            Self::Protect { by, rule_id } => {
                Some(Reason::Protected { by: *by, rule_id: rule_id.clone() })
            }
            Self::Archive { reason } => Some(reason.clone()),
            _ => None,
        }
    }

    pub const fn is_protected(&self) -> bool {
        matches!(self, Self::Protect { .. })
    }
}

// JS `/i` on these ASCII patterns is ASCII-only case-insensitive (`ſ` is not `s`), so the
// input is ASCII-lowercased and matched case-sensitively; Unicode `(?i)` would differ.
static RESTORABLE: LazyLock<Regex> = LazyLock::new(|| {
    // `[^\n\r\u{2028}\u{2029}]` is JS's `.` (the extension's regex).
    Regex::new(
        r"^(https?:|chrome:|edge:|brave:|chrome-extension:|about:blank([?#][^\n\r\u{2028}\u{2029}]*)?$)",
    )
    .expect("valid regex")
});
static BLANK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^(about:blank|chrome://newtab/?|edge://newtab/?|brave://newtab/?|chrome://new-tab-page/?)$",
    )
    .expect("valid regex")
});

fn is_web(url: &str) -> bool {
    let lower = url.get(..6).unwrap_or(url).to_ascii_lowercase();
    lower.starts_with("http:") || lower.starts_with("https:")
}

/// URLs that reopen exactly with `tabs.create({ url })`.
pub fn is_restorable_url(url: Option<&str>) -> bool {
    url.is_some_and(|u| RESTORABLE.is_match(&u.to_ascii_lowercase()))
}

/// Empty new-tab pages: archivable, but not worth showing in search.
pub fn is_blank_page(url: Option<&str>) -> bool {
    url.is_some_and(|u| BLANK.is_match(&u.to_ascii_lowercase()))
}

/// Excluded (F9): never stored, never touched.
pub fn is_excluded(target: &MatchTarget<'_>, rules: &[Rule]) -> bool {
    rules.iter().any(|r| r.action == RuleAction::Exclude && rule_matches(r, target))
}

fn protect(by: ProtectedBy) -> Decision {
    Decision::Protect { by, rule_id: None }
}

/// Decides what to do with one item.
pub fn evaluate(item: &PolicyTarget<'_>, signals: &Signals, ctx: &PolicyContext<'_>) -> Decision {
    // 1. Global hold / pause.
    if ctx.mode == Mode::Paused || ctx.hold_until.is_some_and(|t| t > ctx.now) {
        return protect(ProtectedBy::Hold);
    }

    // 2. Hard protections.
    if item.pinned {
        return protect(ProtectedBy::Pinned);
    }
    if signals.in_use {
        return protect(ProtectedBy::ActiveTab);
    }
    if signals.audible {
        return protect(ProtectedBy::Audible);
    }
    if signals.form_input == FormInput::Dirty {
        return protect(ProtectedBy::FormInput);
    }
    if item.grace_until.is_some_and(|t| t > ctx.now) {
        return protect(ProtectedBy::Grace);
    }
    if item.kind == ItemKind::Tab && !is_restorable_url(item.url) {
        return protect(ProtectedBy::Unrestorable);
    }

    // 3. User rules: any protect rule wins; else the most specific valid custom threshold.
    let target = item.match_target();
    let mut matching: Vec<&Rule> = ctx
        .rules
        .iter()
        .filter(|r| matches!(r.action, RuleAction::Protect | RuleAction::CustomThreshold))
        .filter(|r| rule_matches(r, &target))
        .collect();
    matching.sort_by_key(|r| r.scope.specificity()); // stable: ties keep rule order
    if let Some(rule) = matching.iter().find(|r| r.action == RuleAction::Protect) {
        // Like the extension, an empty id is left out of the reason.
        let rule_id = (!rule.id.is_empty()).then(|| rule.id.clone());
        return Decision::Protect { by: ProtectedBy::Rule, rule_id };
    }
    // A custom rule with invalid thresholds (old data, hand edits) is ignored, not obeyed.
    let custom = matching.iter().find_map(|r| {
        (r.action == RuleAction::CustomThreshold)
            .then_some(r)
            .and_then(|r| r.threshold_hours.filter(Thresholds::is_valid).map(|t| (r, t)))
    });
    let rule_id = custom.map(|(r, _)| r.id.clone());

    // 4. Thresholds.
    let threshold = custom.map_or(ctx.thresholds, |(_, t)| t);
    let active_hours = item.active_age_s as f64 / 3600.0;
    let shown_hours = active_hours.floor() as u64;
    let archive_reason = |archive_at: f64| match &rule_id {
        Some(id) => Reason::CustomRule { rule_id: id.clone(), active_hours: shown_hours },
        None => Reason::Unused { active_hours: shown_hours, threshold: archive_at },
    };

    match item.kind {
        ItemKind::Tab => {
            // A web page might hold typed input we can't see: archive only at 2×.
            let archive_at =
                if signals.form_input == FormInput::Unknown && item.url.is_some_and(is_web) {
                    threshold.archive * UNKNOWN_FORM_FACTOR
                } else {
                    threshold.archive
                };
            if active_hours >= archive_at {
                return Decision::Archive { reason: archive_reason(archive_at) };
            }
        }
        ItemKind::Window => {
            // Closing is Pro and allowlisted *apps* only (PRD F11: an app-scope
            // `closeAllowed` rule; other scopes are ignored defensively), and only with the
            // document saved.
            let allowlisted = ctx.pro
                && ctx.rules.iter().any(|r| {
                    r.action == RuleAction::CloseAllowed
                        && r.scope == RuleScope::App
                        && rule_matches(r, &target)
                });
            let past_archive = active_hours >= threshold.archive;
            if allowlisted && past_archive && signals.save_state == SaveState::Saved {
                return Decision::Archive { reason: archive_reason(threshold.archive) };
            }
            if active_hours >= threshold.hidden {
                let kept_by = (allowlisted && past_archive).then_some(match signals.save_state {
                    SaveState::Unsaved => ProtectedBy::Unsaved,
                    _ => ProtectedBy::UnknownSaveState,
                });
                return Decision::Hide { active_hours, threshold, rule_id, kept_by };
            }
        }
    }
    if active_hours >= threshold.stale {
        return Decision::Stale { active_hours, threshold, rule_id };
    }
    Decision::Keep { active_hours, threshold }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{RuleScope, RuleSource};

    const H: u64 = 3600;
    const NOW: i64 = 1_000_000;

    fn ctx(rules: &[Rule]) -> PolicyContext<'_> {
        PolicyContext {
            now: NOW,
            mode: Mode::Automatic,
            hold_until: None,
            thresholds: Thresholds::BALANCED,
            rules,
            pro: false,
        }
    }
    fn tab(age: u64) -> PolicyTarget<'static> {
        PolicyTarget {
            kind: ItemKind::Tab,
            app_id: "com.google.Chrome",
            url: Some("https://example.com/a"),
            title: "A",
            pinned: false,
            active_age_s: age,
            grace_until: None,
        }
    }
    fn window(age: u64) -> PolicyTarget<'static> {
        PolicyTarget {
            kind: ItemKind::Window,
            app_id: "com.apple.Preview",
            url: None,
            title: "report.pdf",
            ..tab(age)
        }
    }
    fn rule(id: &str, scope: RuleScope, pattern: &str, action: RuleAction) -> Rule {
        Rule {
            id: id.into(),
            scope,
            pattern: pattern.into(),
            action,
            threshold_hours: None,
            enabled: true,
            source: RuleSource::User,
            created_at: 0,
            hit_count: 0,
        }
    }
    const IDLE: Signals = Signals {
        in_use: false,
        audible: false,
        form_input: FormInput::NotApplicable,
        save_state: SaveState::NotApplicable,
    };
    fn kind(d: &Decision) -> &'static str {
        match d {
            Decision::Keep { .. } => "keep",
            Decision::Protect { .. } => "protect",
            Decision::Stale { .. } => "stale",
            Decision::Hide { .. } => "hide",
            Decision::Archive { .. } => "archive",
        }
    }

    #[test]
    fn applies_the_thresholds() {
        assert_eq!(kind(&evaluate(&tab(11 * H), &IDLE, &ctx(&[]))), "keep");
        assert_eq!(kind(&evaluate(&tab(12 * H), &IDLE, &ctx(&[]))), "stale");
        assert_eq!(
            evaluate(&tab(61 * H + H / 2), &IDLE, &ctx(&[])),
            Decision::Archive { reason: Reason::Unused { active_hours: 61, threshold: 60.0 } }
        );
    }

    #[test]
    fn holds_and_pause_protect_everything() {
        let old = tab(100 * H);
        let paused = PolicyContext { mode: Mode::Paused, ..ctx(&[]) };
        assert_eq!(evaluate(&old, &IDLE, &paused), protect(ProtectedBy::Hold));
        let held = PolicyContext { hold_until: Some(2_000_000), ..ctx(&[]) };
        assert!(evaluate(&old, &IDLE, &held).is_protected());
        let expired = PolicyContext { hold_until: Some(500), ..ctx(&[]) };
        assert_eq!(kind(&evaluate(&old, &IDLE, &expired)), "archive");
    }

    #[test]
    fn doubles_the_archive_threshold_for_web_pages_whose_form_state_is_unknown() {
        let unknown = Signals { form_input: FormInput::Unknown, ..IDLE };
        assert_eq!(kind(&evaluate(&tab(100 * H), &unknown, &ctx(&[]))), "stale");
        assert_eq!(
            evaluate(&tab(120 * H), &unknown, &ctx(&[])),
            Decision::Archive { reason: Reason::Unused { active_hours: 120, threshold: 120.0 } }
        );
        let new_tab = PolicyTarget { url: Some("chrome://newtab/"), ..tab(61 * H) };
        assert_eq!(kind(&evaluate(&new_tab, &unknown, &ctx(&[]))), "archive");
        let clean = Signals { form_input: FormInput::Clean, ..IDLE };
        assert_eq!(kind(&evaluate(&tab(61 * H), &clean, &ctx(&[]))), "archive");
    }

    #[test]
    fn applies_hard_protections_in_order() {
        let by = |t: PolicyTarget<'_>, s: Signals| match evaluate(&t, &s, &ctx(&[])) {
            Decision::Protect { by, .. } => format!("{by:?}"),
            d => kind(&d).to_owned(),
        };
        let old = tab(100 * H);
        let busy = Signals { in_use: true, audible: true, ..IDLE };
        assert_eq!(by(PolicyTarget { pinned: true, ..old }, busy), "Pinned");
        assert_eq!(by(old, Signals { in_use: true, ..IDLE }), "ActiveTab");
        assert_eq!(by(old, Signals { audible: true, ..IDLE }), "Audible");
        assert_eq!(by(old, Signals { form_input: FormInput::Dirty, ..IDLE }), "FormInput");
        assert_eq!(by(PolicyTarget { grace_until: Some(2_000_000), ..old }, IDLE), "Grace");
        assert_eq!(
            by(PolicyTarget { url: Some("data:text/html,hi"), ..old }, IDLE),
            "Unrestorable"
        );
        assert_eq!(by(PolicyTarget { url: None, ..old }, IDLE), "Unrestorable");
        assert_eq!(by(PolicyTarget { url: Some("chrome://newtab/"), ..old }, IDLE), "archive");
    }

    #[test]
    fn lets_any_protect_rule_win_over_custom_thresholds() {
        let mut custom =
            rule("c", RuleScope::UrlGlob, "https://example.com/*", RuleAction::CustomThreshold);
        custom.threshold_hours = Some(Thresholds { stale: 1.0, hidden: 2.0, archive: 3.0 });
        let protect_rule = rule("p", RuleScope::Domain, "example.com", RuleAction::Protect);
        let rules = [custom, protect_rule];
        assert_eq!(
            evaluate(&tab(100 * H), &IDLE, &ctx(&rules)),
            Decision::Protect { by: ProtectedBy::Rule, rule_id: Some("p".into()) }
        );
    }

    #[test]
    fn uses_the_most_specific_custom_threshold() {
        let mut domain = rule("d", RuleScope::Domain, "example.com", RuleAction::CustomThreshold);
        domain.threshold_hours = Some(Thresholds { stale: 20.0, hidden: 30.0, archive: 40.0 });
        let mut glob =
            rule("g", RuleScope::UrlGlob, "https://example.com/*", RuleAction::CustomThreshold);
        glob.threshold_hours = Some(Thresholds { stale: 1.0, hidden: 2.0, archive: 3.0 });
        let rules = [domain, glob];
        assert_eq!(
            evaluate(&tab(4 * H), &IDLE, &ctx(&rules)),
            Decision::Archive {
                reason: Reason::CustomRule { rule_id: "g".into(), active_hours: 4 }
            }
        );
    }

    #[test]
    fn ignores_custom_rules_with_invalid_thresholds() {
        let mut legacy = rule("l", RuleScope::Domain, "example.com", RuleAction::CustomThreshold);
        legacy.threshold_hours = Some(Thresholds { stale: 0.0, hidden: 0.0, archive: 0.0 });
        assert_eq!(kind(&evaluate(&tab(1), &IDLE, &ctx(&[legacy]))), "keep");
    }

    #[test]
    fn ignores_disabled_exclude_and_close_allowed_rules_for_tabs() {
        let mut disabled = rule("x", RuleScope::Domain, "example.com", RuleAction::Protect);
        disabled.enabled = false;
        let rules = [
            disabled,
            rule("e", RuleScope::Domain, "example.com", RuleAction::Exclude),
            rule("a", RuleScope::App, "com.google.Chrome", RuleAction::CloseAllowed),
        ];
        assert_eq!(kind(&evaluate(&tab(100 * H), &IDLE, &ctx(&rules))), "archive");
    }

    #[test]
    fn windows_are_hidden_and_closed_only_when_allowlisted_saved_and_pro() {
        let allow = [rule("a", RuleScope::App, "com.apple.Preview", RuleAction::CloseAllowed)];
        let saved = Signals { save_state: SaveState::Saved, ..IDLE };
        let pro = PolicyContext { pro: true, ..ctx(&allow) };
        assert_eq!(kind(&evaluate(&window(13 * H), &saved, &pro)), "stale");
        assert_eq!(kind(&evaluate(&window(40 * H), &saved, &pro)), "hide");
        assert_eq!(kind(&evaluate(&window(61 * H), &saved, &pro)), "archive");
        // Without Pro, the allowlist, or a saved document: hidden, never closed.
        assert_eq!(kind(&evaluate(&window(61 * H), &saved, &ctx(&allow))), "hide");
        assert_eq!(
            kind(&evaluate(&window(61 * H), &saved, &PolicyContext { pro: true, ..ctx(&[]) })),
            "hide"
        );
        for (state, why) in [
            (SaveState::Unsaved, ProtectedBy::Unsaved),
            (SaveState::Unknown, ProtectedBy::UnknownSaveState),
        ] {
            let s = Signals { save_state: state, ..IDLE };
            let d = evaluate(&window(500 * H), &s, &pro);
            assert!(matches!(d, Decision::Hide { kept_by: Some(by), .. } if by == why), "{d:?}");
        }
        // Below the archive threshold there's nothing to explain.
        let d =
            evaluate(&window(40 * H), &Signals { save_state: SaveState::Unsaved, ..IDLE }, &pro);
        assert!(matches!(d, Decision::Hide { kept_by: None, .. }));
        // Only app-scope allowlist rules count: a global closeAllowed rule closes nothing.
        let global = [rule("g", RuleScope::Global, "*", RuleAction::CloseAllowed)];
        let pro_global = PolicyContext { pro: true, ..ctx(&global) };
        assert_eq!(kind(&evaluate(&window(61 * H), &saved, &pro_global)), "hide");
        // Windows have no URL: not "unrestorable".
        assert_eq!(kind(&evaluate(&window(H), &IDLE, &ctx(&[]))), "keep");
    }

    #[test]
    fn helpers() {
        for url in
            ["https://a.com", "http://x", "chrome://settings", "about:blank", "about:blank#x"]
        {
            assert!(is_restorable_url(Some(url)), "{url}");
        }
        for url in ["data:,x", "blob:https://a/1", "file:///a.pdf", "javascript:1", "about:srcdoc"]
        {
            assert!(!is_restorable_url(Some(url)), "{url}");
        }
        assert!(is_blank_page(Some("chrome://newtab/")));
        assert!(is_blank_page(Some("about:blank")));
        assert!(!is_blank_page(Some("chrome://settings/")));
        assert!(!is_blank_page(None));
        let rules = [rule("e", RuleScope::Domain, "bank.com", RuleAction::Exclude)];
        let t = |url| MatchTarget { app_id: "x", url: Some(url), title: "" };
        assert!(is_excluded(&t("https://my.bank.com/"), &rules));
        assert!(!is_excluded(&t("https://other.com/"), &rules));
    }

    #[test]
    fn decision_reason_round_trips() {
        let d = Decision::Protect { by: ProtectedBy::Rule, rule_id: Some("r".into()) };
        assert_eq!(
            d.reason(),
            Some(Reason::Protected { by: ProtectedBy::Rule, rule_id: Some("r".into()) })
        );
        assert_eq!(
            Decision::Keep { active_hours: 0.0, threshold: Thresholds::BALANCED }.reason(),
            None
        );
    }
}
