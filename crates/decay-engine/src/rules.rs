//! Rules from the app (P1.12, PRD F8/F9, API Design §2.4): validation like the
//! extension's (`lib/rules/rules.ts`), create / update / delete with the policy reloaded
//! at once, the excluded-site purge (F9: excluded sites are never stored, "even in
//! events"), and testing a rule against open tabs and the archive.

use decay_core::matching::title_regex_problem;
use decay_core::{
    ItemState, MatchTarget, Rule, RuleAction, RuleScope, RuleSource, Thresholds, rule_matches,
};
use decay_store::{Item, StoreError, new_id};
use serde::Deserialize;

use crate::engine::{Engine, Output, Res};

/// The longest pattern accepted (as the extension).
const MAX_MATCH_LENGTH: usize = 2048;
/// Open tabs a rule test shows.
const SAMPLE: usize = 5;

/// Unrelated pages used to catch exclude rules that would match (and purge) almost
/// everything, e.g. `*://*`, title `.*`. A real exclude matches at most one.
const BROADNESS_CANARIES: [(&str, &str); 5] = [
    ("https://example.com/", "Example Domain"),
    ("https://www.wikipedia.org/wiki/Tab", "Tab – Wikipedia"),
    ("https://news.ycombinator.com/item?id=1", "Hacker News"),
    ("http://localhost:3000/dashboard", "Dashboard · Dev"),
    ("https://mail.proton.me/u/0/inbox", "Inbox | Proton Mail"),
];

/// A rule as the user writes it (`create_rule`, `update_rule`, `test_rule`).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleInput {
    pub scope: RuleScope,
    #[serde(rename = "match", default)]
    pub pattern: String,
    pub action: RuleAction,
    #[serde(default)]
    pub threshold_hours: Option<Thresholds>,
    #[serde(default)]
    pub enabled: Option<bool>,
}

/// Why a rule change was refused.
#[derive(Debug, thiserror::Error)]
pub enum RuleRefused {
    #[error("{0}")]
    Invalid(String),
    #[error("an identical rule already exists")]
    Duplicate,
    #[error("this rule no longer exists")]
    NotFound,
    #[error(transparent)]
    Store(#[from] StoreError),
}

fn invalid(why: impl Into<String>) -> RuleRefused {
    RuleRefused::Invalid(why.into())
}

/// Accepts `github.com`, `*.github.com`, `GitHub.com`, `bücher.de` (→ punycode), an IP,
/// `localhost` or a pasted URL. Rejects bare public suffixes (`com`, `co.uk`,
/// `github.io`), which would match a huge part of the web.
pub fn normalize_domain(input: &str) -> Result<String, RuleRefused> {
    let mut value = input.trim().to_lowercase();
    if let Some(rest) = value.strip_prefix("*.") {
        value = rest.to_owned();
    }
    let has_scheme = value.find("://").is_some_and(|i| {
        value[..i].chars().enumerate().all(|(n, c)| {
            (n == 0 && c.is_ascii_lowercase())
                || (n > 0 && (c.is_ascii_alphanumeric() || "+.-".contains(c)))
        })
    });
    if !has_scheme {
        value = format!("http://{value}");
    }
    let bad = || invalid("Not a valid domain");
    let url = url::Url::parse(&value).map_err(|_| bad())?;
    if !url.username().is_empty() || url.password().is_some() {
        return Err(bad());
    }
    let host = match url.host().ok_or_else(bad)? {
        url::Host::Domain(d) => d.trim_end_matches('.').to_owned(),
        url::Host::Ipv4(ip) => return Ok(ip.to_string()),
        url::Host::Ipv6(ip) => return Ok(format!("[{ip}]")),
    };
    if host.is_empty() {
        return Err(bad());
    }
    if host != "localhost" && psl::domain_str(&host).is_none() {
        return Err(invalid(format!(
            "“{host}” is a public suffix; use a specific site such as example.{host}"
        )));
    }
    Ok(host)
}

/// Checks a rule and returns it normalized (scope, match, action, thresholds).
pub fn validate_rule(
    input: &RuleInput,
) -> Result<(RuleScope, String, RuleAction, Option<Thresholds>), RuleRefused> {
    let (scope, action) = (input.scope, input.action);
    let mut pattern = input.pattern.trim().to_owned();
    if scope == RuleScope::Global {
        pattern = "*".into();
    }
    if pattern.is_empty() {
        return Err(invalid("Match is required"));
    }
    if pattern.chars().count() > MAX_MATCH_LENGTH {
        return Err(invalid("Match is too long"));
    }
    match scope {
        RuleScope::Domain => pattern = normalize_domain(&pattern)?,
        RuleScope::TitleRegex => {
            if let Some(problem) = title_regex_problem(&pattern) {
                return Err(invalid(problem));
            }
        }
        RuleScope::UrlGlob if pattern.chars().all(|c| c == '*') => {
            return Err(invalid("Use the global scope to match every URL"));
        }
        _ => {}
    }
    if action == RuleAction::Exclude && matches!(scope, RuleScope::Global | RuleScope::App) {
        return Err(invalid("Exclude rules need a domain, URL or title pattern"));
    }
    if action == RuleAction::CloseAllowed && scope != RuleScope::App {
        return Err(invalid("Close-allowed rules apply to apps only"));
    }
    let thresholds = if action == RuleAction::CustomThreshold {
        let t = input
            .threshold_hours
            .ok_or_else(|| invalid("Custom threshold rules need thresholds"))?;
        t.validate().map_err(|e| invalid(e.to_string()))?;
        Some(t)
    } else {
        None
    };
    if action == RuleAction::Exclude {
        let probe = rule(scope, &pattern, action, None, true, 0);
        let hits = BROADNESS_CANARIES
            .iter()
            .filter(|(url, title)| {
                rule_matches(&probe, &MatchTarget { app_id: "canary", url: Some(url), title })
            })
            .count();
        if hits > 1 {
            return Err(invalid("This exclude rule matches too many sites; make it more specific"));
        }
    }
    Ok((scope, pattern, action, thresholds))
}

fn rule(
    scope: RuleScope,
    pattern: &str,
    action: RuleAction,
    threshold_hours: Option<Thresholds>,
    enabled: bool,
    now: i64,
) -> Rule {
    Rule {
        id: String::new(),
        scope,
        pattern: pattern.to_owned(),
        action,
        threshold_hours,
        enabled,
        source: RuleSource::User,
        created_at: now,
        hit_count: 0,
    }
}

/// What a rule test found (`test_rule`).
#[derive(Debug, Clone, PartialEq)]
pub struct RuleTest {
    pub matches_live: u32,
    pub matches_archived: u32,
    /// Open tabs it matches, the least recently used first (at most 5).
    pub sample: Vec<Item>,
}

fn store_err(e: StoreError) -> RuleRefused {
    match e {
        StoreError::DuplicateRule => RuleRefused::Duplicate,
        StoreError::NotFound => RuleRefused::NotFound,
        other => RuleRefused::Store(other),
    }
}

impl Engine {
    /// Every rule, oldest first.
    pub fn rules(&self) -> Res<Vec<Rule>> {
        self.store.rules()
    }

    /// Adds a rule; an enabled exclude rule purges what's stored for its sites in the
    /// same transaction. Returns the rule and how many items were purged.
    pub fn create_rule(
        &mut self,
        input: &RuleInput,
        now: i64,
    ) -> Result<(Rule, u32, Vec<Output>), RuleRefused> {
        let (scope, pattern, action, thresholds) = validate_rule(input)?;
        let mut r = rule(scope, &pattern, action, thresholds, input.enabled.unwrap_or(true), now);
        r.id = new_id(now);
        let purged = self.save_and_purge(&r, true)?;
        Ok((r, purged, self.rules_changed(now)?))
    }

    /// Changes a rule's scope, match, action, thresholds or on/off. Turning a rule off
    /// always works, even for one that today's checks would refuse (an imported or older
    /// rule), as in the extension.
    pub fn update_rule(
        &mut self,
        id: &str,
        input: &RuleInput,
        now: i64,
    ) -> Result<(Rule, u32, Vec<Output>), RuleRefused> {
        let mut r =
            self.store.rules()?.into_iter().find(|r| r.id == id).ok_or(RuleRefused::NotFound)?;
        let only_disabling = input.enabled == Some(false)
            && input.scope == r.scope
            && input.action == r.action
            && input.pattern == r.pattern
            && (input.threshold_hours.is_none() || input.threshold_hours == r.threshold_hours);
        if only_disabling {
            r.enabled = false;
        } else {
            let (scope, pattern, action, thresholds) = validate_rule(input)?;
            r.scope = scope;
            r.pattern = pattern;
            r.action = action;
            r.threshold_hours = thresholds;
            r.enabled = input.enabled.unwrap_or(r.enabled);
        }
        let purged = self.save_and_purge(&r, false)?;
        Ok((r, purged, self.rules_changed(now)?))
    }

    pub fn delete_rule(&mut self, id: &str, now: i64) -> Result<Vec<Output>, RuleRefused> {
        if !self.store.delete_rule(id)? {
            return Err(RuleRefused::NotFound);
        }
        self.rules_changed(now)
    }

    /// How many items saving an exclude rule would delete (to confirm before saving);
    /// 0 for any other action, as in the extension.
    pub fn preview_exclude(&self, input: &RuleInput) -> Result<u32, RuleRefused> {
        let (scope, pattern, action, t) = validate_rule(input)?;
        if action != RuleAction::Exclude {
            return Ok(0);
        }
        let probe = rule(scope, &pattern, action, t, true, 0);
        let (now, later) = self.matching_items(&probe)?;
        Ok((now.len() + later.len()).try_into().unwrap_or(u32::MAX))
    }

    /// Tests a rule (valid or not yet complete: an invalid one is reported).
    pub fn test_rule(&self, input: &RuleInput, now: i64) -> Result<RuleTest, RuleRefused> {
        let (scope, pattern, action, t) = validate_rule(input)?;
        let probe = rule(scope, &pattern, action, t, true, 0);
        let mut live: Vec<Item> = Vec::new();
        for conn in self.conns.values() {
            let Some(a) = Self::live_adapter(conn, now) else { continue };
            for item in a.tabs.values().filter_map(|t| t.item.as_ref()) {
                let target = MatchTarget {
                    app_id: &item.app_id,
                    url: item.url.as_deref(),
                    title: &item.title,
                };
                if rule_matches(&probe, &target) {
                    live.push(item.clone());
                }
            }
        }
        live.sort_by(|x, y| x.last_used_at.cmp(&y.last_used_at).then_with(|| x.id.cmp(&y.id)));
        let archived = self
            .store
            .item_targets()?
            .into_iter()
            .filter(|t| t.state == ItemState::Archived)
            .filter(|t| {
                rule_matches(
                    &probe,
                    &MatchTarget { app_id: &t.app_id, url: t.url.as_deref(), title: &t.title },
                )
            })
            .count();
        Ok(RuleTest {
            matches_live: live.len().try_into().unwrap_or(u32::MAX),
            matches_archived: archived.try_into().unwrap_or(u32::MAX),
            sample: live.into_iter().take(SAMPLE).collect(),
        })
    }

    /// Stored items an exclude-style `probe` matches: those to delete now, and those a
    /// sweep is closing (the sweep owns them until the close settles; then they go too).
    pub(crate) fn matching_items(&self, probe: &Rule) -> Res<(Vec<String>, Vec<String>)> {
        let (mut now, mut later) = (Vec::new(), Vec::new());
        for t in self.store.item_targets()? {
            let target = MatchTarget { app_id: &t.app_id, url: t.url.as_deref(), title: &t.title };
            if rule_matches(probe, &target) {
                if t.state == ItemState::Archiving { later.push(t.id) } else { now.push(t.id) }
            }
        }
        Ok((now, later))
    }

    /// Saves `r` (insert or update) and, when it's an enabled exclude rule, deletes what's
    /// stored for its sites, all in one transaction: a failure leaves neither.
    fn save_and_purge(&mut self, r: &Rule, insert: bool) -> Result<u32, RuleRefused> {
        let (ids, later) = if r.enabled && r.action == RuleAction::Exclude {
            self.matching_items(r)?
        } else {
            (Vec::new(), Vec::new())
        };
        self.store
            .atomically(true, |s| {
                if insert {
                    s.insert_rule(r)?;
                } else {
                    s.update_rule(r)?;
                }
                for id in &ids {
                    s.delete_item(id)?;
                }
                Ok(())
            })
            .map_err(store_err)?;
        self.forget_items(&ids);
        self.exclude_pending.extend(later);
        Ok(ids.len().try_into().unwrap_or(u32::MAX))
    }

    /// Drops deleted items from the open tabs' records.
    pub(crate) fn forget_items(&mut self, ids: &[String]) {
        if ids.is_empty() {
            return;
        }
        for conn in self.conns.values_mut() {
            if let Some(a) = conn.adapter.as_mut() {
                for tab in a.tabs.values_mut() {
                    if tab.item.as_ref().is_some_and(|i| ids.contains(&i.id)) {
                        tab.item = None;
                    }
                }
            }
        }
    }

    /// Deletes the items excluded mid-archive whose close has settled (archived, or back
    /// to stale), if an enabled exclude rule still matches them.
    pub(crate) fn purge_settled_excludes(&mut self) {
        if self.exclude_pending.is_empty() {
            return;
        }
        let excludes: Vec<Rule> = self
            .policy
            .rules
            .iter()
            .filter(|r| r.enabled && r.action == RuleAction::Exclude)
            .cloned()
            .collect();
        let mut done = Vec::new();
        let mut delete = Vec::new();
        for id in &self.exclude_pending {
            match self.store.item(id) {
                Ok(Some(item)) if item.state == ItemState::Archiving => {}
                Ok(Some(item)) => {
                    done.push(id.clone());
                    let target = MatchTarget {
                        app_id: &item.app_id,
                        url: item.url.as_deref(),
                        title: &item.title,
                    };
                    if excludes.iter().any(|r| rule_matches(r, &target)) {
                        delete.push(id.clone());
                    }
                }
                Ok(None) => done.push(id.clone()),
                Err(e) => {
                    tracing::warn!(error = %e, "exclude purge");
                    return;
                }
            }
        }
        let deleted = self.store.atomically(true, |s| {
            for id in &delete {
                s.delete_item(id)?;
            }
            Ok(())
        });
        if let Err(e) = deleted {
            tracing::warn!(error = %e, "exclude purge");
            return;
        }
        self.forget_items(&delete);
        for id in done {
            self.exclude_pending.remove(&id);
        }
    }

    /// After any rule change: the policy reloads and the extensions hear about changed
    /// exclusions at once.
    fn rules_changed(&mut self, now: i64) -> Result<Vec<Output>, RuleRefused> {
        let before = self.policy.excluded_domains();
        self.reload_policy()?;
        if self.policy.excluded_domains() != before {
            self.broadcast_config(now);
        }
        self.publish_status(now);
        Ok(std::mem::take(&mut self.out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(scope: RuleScope, pattern: &str, action: RuleAction) -> RuleInput {
        RuleInput { scope, pattern: pattern.into(), action, threshold_hours: None, enabled: None }
    }

    fn ok(i: &RuleInput) -> String {
        validate_rule(i).unwrap().1
    }

    fn err(i: &RuleInput) -> String {
        validate_rule(i).unwrap_err().to_string()
    }

    #[test]
    fn domains_normalize_like_the_extension() {
        let d = |s: &str| normalize_domain(s);
        assert_eq!(d("GitHub.com").unwrap(), "github.com");
        assert_eq!(d("*.github.com").unwrap(), "github.com");
        assert_eq!(d("https://docs.github.com/en?x=1").unwrap(), "docs.github.com");
        assert_eq!(d("bücher.de").unwrap(), "xn--bcher-kva.de");
        assert_eq!(d("localhost").unwrap(), "localhost");
        assert_eq!(d("127.0.0.1").unwrap(), "127.0.0.1");
        assert_eq!(d("alice.github.io").unwrap(), "alice.github.io");
        for bad in ["com", "co.uk", "github.io", "", "user:pw@site.com", "http://"] {
            assert!(d(bad).is_err(), "{bad}");
        }
        assert!(d("com").unwrap_err().to_string().contains("public suffix"));
    }

    #[test]
    fn rules_are_checked() {
        use RuleAction::*;
        use RuleScope::*;
        assert_eq!(ok(&input(Domain, " Mail.Google.com ", Protect)), "mail.google.com");
        assert_eq!(ok(&input(Global, "anything", Protect)), "*");
        assert!(err(&input(Domain, "  ", Protect)).contains("required"));
        assert!(err(&input(UrlGlob, "***", Protect)).contains("global scope"));
        assert!(err(&input(App, "com.apple.finder", Exclude)).contains("Exclude rules"));
        assert!(err(&input(Domain, "a.com", CloseAllowed)).contains("apps only"));
        assert!(err(&input(Domain, "a.com", CustomThreshold)).contains("need thresholds"));
        let mut custom = input(Domain, "a.com", CustomThreshold);
        custom.threshold_hours = Some(Thresholds { stale: 5.0, hidden: 4.0, archive: 9.0 });
        assert!(validate_rule(&custom).is_err());
        custom.threshold_hours = Some(Thresholds { stale: 2.0, hidden: 4.0, archive: 9.0 });
        assert_eq!(validate_rule(&custom).unwrap().3, custom.threshold_hours);
        assert!(err(&input(TitleRegex, "(a+)+", Protect)).len() > 3);
        assert!(err(&input(Domain, &"a".repeat(3000), Protect)).contains("too long"));
        // Too-broad excludes.
        assert!(err(&input(UrlGlob, "http*", Exclude)).contains("too many sites"));
        assert!(err(&input(TitleRegex, ".", Exclude)).contains("too many sites"));
        assert_eq!(
            ok(&input(UrlGlob, "https://bank.example/*", Exclude)),
            "https://bank.example/*"
        );
    }

    #[test]
    fn rule_input_reads_the_api_shape() {
        let i: RuleInput = serde_json::from_value(serde_json::json!({
            "scope": "urlGlob", "match": "github.com/*/pull/*", "action": "customThreshold",
            "thresholdHours": {"stale": 24, "hidden": 72, "archive": 120}, "enabled": false
        }))
        .unwrap();
        assert_eq!(
            (i.scope, i.action, i.enabled),
            (RuleScope::UrlGlob, RuleAction::CustomThreshold, Some(false))
        );
        assert_eq!(i.threshold_hours.unwrap().archive, 120.0);
    }
}
