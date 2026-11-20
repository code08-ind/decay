//! The daily summary (P2.4, PRD F10, Designs/11): what moved yesterday (tabs archived,
//! windows hidden, items restored), the archived items, and a few hints "worth a look":
//! a site restored again and again (offer "Always keep"), a site archived often (offer
//! "Keep longer"). Shown in the Library (`get_daily_summary`) and, as a morning
//! notification, by `notify`.

use decay_core::{EventType, RuleAction, RuleScope};
use decay_store::Store;
use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use crate::commands::{Res, blocking};
use crate::core::AppError;
use crate::search::{PanelItem, row};

/// Hints look back this far.
const HINT_WINDOW_MS: i64 = 7 * 24 * 3_600_000;
/// A site restored this often in a week is worth keeping.
const RESTORED_OFTEN: u32 = 3;
/// A site archived this often in a week may deserve longer.
const ARCHIVED_OFTEN: u32 = 5;
/// The list shows at most this many items.
const ITEMS: usize = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum HintKind {
    /// Restored often: "Always keep" (a protect rule).
    AlwaysKeep,
    /// Archived often: "Keep longer" (a custom-timing rule).
    KeepLonger,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Hint {
    pub kind: HintKind,
    pub domain: String,
    /// How many times in the last 7 days.
    pub count: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DailySummary {
    pub archived_tabs: u32,
    pub hidden_windows: u32,
    pub restored: u32,
    pub hints: Vec<Hint>,
    /// Still archived from that day, newest first.
    pub items: Vec<PanelItem>,
}

impl DailySummary {
    /// Whether anything happened (no notification for an empty day).
    pub fn is_empty(&self) -> bool {
        self.archived_tabs == 0 && self.hidden_windows == 0 && self.restored == 0
    }

    /// The notification's text ("Yesterday Decay archived 14 tabs and hid 3 windows. 2
    /// things are worth a look.").
    pub fn sentence(&self) -> String {
        use crate::i18n::tf;
        let counts = [("tabs", self.archived_tabs.into()), ("windows", self.hidden_windows.into())];
        let key = match (self.archived_tabs, self.hidden_windows) {
            (0, 0) => "summary.nothing",
            (_, 0) => "summary.tabs",
            (0, _) => "summary.windows",
            _ => "summary.both",
        };
        let mut out = tf(key, &counts);
        if self.restored > 0 {
            out = tf("summary.reopened", &[("text", out.into()), ("n", self.restored.into())]);
        }
        if !self.hints.is_empty() {
            out = tf("summary.hints", &[("text", out.into()), ("n", self.hints.len().into())]);
        }
        out
    }
}

/// The summary of `[from, to)` (a local day) as of `now`.
pub fn summary_for(store: &Store, from: i64, to: i64, now: i64) -> Res<DailySummary> {
    let db = |e: decay_store::StoreError| AppError::db(&e);
    let (archived_tabs, _) =
        store.items_with_event_between(EventType::Archived, from, to).map_err(db)?;
    let (_, hidden_windows) =
        store.items_with_event_between(EventType::Hidden, from, to).map_err(db)?;
    // Reopened: restored from the archive, or brought back by Undo (tabs and hidden
    // windows), each item once.
    let restored = store
        .items_with_any_event_between(&[EventType::Restored, EventType::Undo], from, to)
        .map_err(db)?;
    let since = now - HINT_WINDOW_MS;
    let rules = store.rules().map_err(db)?;
    let has_rule = |domain: &str, action: RuleAction| {
        rules.iter().any(|r| {
            r.enabled && r.scope == RuleScope::Domain && r.pattern == domain && r.action == action
        })
    };
    let mut hints = Vec::new();
    for (domain, count) in store
        .domains_with_events_since(EventType::Restored, since, RESTORED_OFTEN, 3)
        .map_err(db)?
    {
        if !has_rule(&domain, RuleAction::Protect) {
            hints.push(Hint { kind: HintKind::AlwaysKeep, domain, count });
        }
    }
    for (domain, count) in store
        .domains_with_events_since(EventType::Archived, since, ARCHIVED_OFTEN, 3)
        .map_err(db)?
    {
        let known = hints.iter().any(|h| h.domain == domain)
            || has_rule(&domain, RuleAction::Protect)
            || has_rule(&domain, RuleAction::CustomThreshold);
        if !known {
            hints.push(Hint { kind: HintKind::KeepLonger, domain, count });
        }
    }
    let mut names = crate::search::RowCache::default();
    let items = store
        .archived_between(from, to, ITEMS)
        .map_err(db)?
        .into_iter()
        .map(|i| row(store, i, &mut names))
        .collect();
    Ok(DailySummary { archived_tabs, hidden_windows, restored, hints, items })
}

/// `[start, end)` of the local day before the one containing `now`, in epoch ms.
pub fn yesterday(now: i64, tz: &jiff::tz::TimeZone) -> Option<(i64, i64)> {
    let today = jiff::Timestamp::from_millisecond(now).ok()?.to_zoned(tz.clone()).date();
    let start_of =
        |d: jiff::civil::Date| d.to_zoned(tz.clone()).ok().map(|z| z.timestamp().as_millisecond());
    let end = start_of(today)?;
    let start = start_of(today.yesterday().ok()?)?;
    Some((start, end))
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SummaryInput {
    /// The local day to summarise (the UI knows its time zone).
    pub day_start: i64,
    pub day_end: i64,
}

#[tauri::command]
pub async fn get_daily_summary(app: AppHandle, input: SummaryInput) -> Res<DailySummary> {
    if input.day_end <= input.day_start || input.day_end - input.day_start > 26 * 3_600_000 {
        return Err(AppError::new("INVALID_INPUT", "not a day"));
    }
    blocking(app, move |core| {
        core.ask(move |e, now| {
            (summary_for(e.store(), input.day_start, input.day_end, now), Vec::new())
        })?
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(tabs: u32, windows: u32, restored: u32, hints: usize) -> DailySummary {
        DailySummary {
            archived_tabs: tabs,
            hidden_windows: windows,
            restored,
            hints: (0..hints)
                .map(|n| Hint { kind: HintKind::AlwaysKeep, domain: format!("s{n}.com"), count: 3 })
                .collect(),
            items: Vec::new(),
        }
    }

    #[test]
    fn the_sentence_says_what_moved() {
        assert_eq!(
            summary(14, 3, 0, 2).sentence(),
            "Yesterday Decay archived 14 tabs and hid 3 windows. 2 things are worth a look."
        );
        assert_eq!(
            summary(1, 0, 1, 1).sentence(),
            "Yesterday Decay archived 1 tab. You reopened 1 item. 1 thing is worth a look."
        );
        assert_eq!(summary(0, 1, 0, 0).sentence(), "Yesterday Decay hid 1 window.");
        assert!(summary(0, 0, 0, 0).is_empty());
        assert!(!summary(0, 0, 2, 0).is_empty());
    }

    #[test]
    fn yesterday_is_the_previous_local_day() {
        let tz = jiff::tz::TimeZone::fixed(jiff::tz::offset(2));
        // 2026-11-14 09:00 at +02:00.
        let now = jiff::civil::date(2026, 11, 14).at(9, 0, 0, 0).to_zoned(tz.clone()).unwrap();
        let (start, end) = yesterday(now.timestamp().as_millisecond(), &tz).unwrap();
        assert_eq!(end - start, 24 * 3_600_000);
        let start_local = jiff::Timestamp::from_millisecond(start).unwrap().to_zoned(tz).datetime();
        assert_eq!(start_local, jiff::civil::date(2026, 11, 13).at(0, 0, 0, 0));
    }

    #[test]
    fn a_day_with_archives_hidden_windows_restores_and_hints() {
        use decay_core::{ItemKind, ItemState};
        let dir = tempfile::tempdir().unwrap();
        let (store, _) = Store::open(
            decay_store::StorePaths::in_dir(dir.path()),
            decay_store::DbKey::from_bytes([7; 32]),
            1,
        )
        .unwrap();
        let day = 1_000_000_000;
        let mk = |id: &str, kind: ItemKind, domain: &str, state: ItemState| {
            let mut item = decay_store::Item::new_live(id.into(), kind, "com.google.Chrome", day);
            item.domain = Some(domain.into());
            item.url = Some(format!("https://{domain}/"));
            item.title = id.into();
            item.state = state;
            if state == ItemState::Archived {
                item.archived_at = Some(day + 10);
            }
            store.insert_item(&item).unwrap();
        };
        mk("a", ItemKind::Tab, "news.example", ItemState::Archived);
        mk("w", ItemKind::Window, "", ItemState::Hidden);
        mk("r", ItemKind::Tab, "stripe.com", ItemState::Restored);
        store.append_event("a", EventType::Archived, None, None, day + 10).unwrap();
        store.append_event("w", EventType::Hidden, None, None, day + 20).unwrap();
        for n in 0..3 {
            store.append_event("r", EventType::Restored, None, None, day + 30 + n).unwrap();
        }
        // Before the day: not counted.
        store.append_event("a", EventType::Archived, None, None, day - 10).unwrap();
        let s = summary_for(&store, day, day + 86_400_000, day + 86_400_000).unwrap();
        assert_eq!((s.archived_tabs, s.hidden_windows, s.restored), (1, 1, 1));
        assert_eq!(
            s.hints,
            vec![Hint { kind: HintKind::AlwaysKeep, domain: "stripe.com".into(), count: 3 }]
        );
        assert_eq!(s.items.len(), 1);
        assert_eq!(s.items[0].id, "a");
    }

    #[test]
    fn hints_skip_sites_with_a_rule_and_never_repeat_a_site() {
        use decay_core::{ItemKind, ItemState, Rule, RuleSource};
        let dir = tempfile::tempdir().unwrap();
        let (store, _) = Store::open(
            decay_store::StorePaths::in_dir(dir.path()),
            decay_store::DbKey::from_bytes([7; 32]),
            1,
        )
        .unwrap();
        let day = 1_000_000_000;
        let add = |id: &str, domain: &str, kind: EventType, n: i64| {
            let mut item =
                decay_store::Item::new_live(id.into(), ItemKind::Tab, "com.google.Chrome", day);
            item.domain = Some(domain.into());
            item.state = ItemState::Restored;
            store.insert_item(&item).unwrap();
            for k in 0..n {
                store.append_event(id, kind, None, None, day + k).unwrap();
            }
        };
        add("kept", "kept.com", EventType::Restored, 3); // has a protect rule
        add("both", "both.com", EventType::Restored, 3); // restored often…
        add("both2", "both.com", EventType::Archived, 5); // …and archived often: one hint
        add("timed", "timed.com", EventType::Archived, 5); // has a custom timing rule
        add("off", "off.com", EventType::Restored, 3); // its rule is turned off: hinted
        let rule = |id: &str, pattern: &str, action: RuleAction, enabled: bool| Rule {
            id: id.into(),
            scope: RuleScope::Domain,
            pattern: pattern.into(),
            action,
            threshold_hours: (action == RuleAction::CustomThreshold)
                .then_some(decay_core::Thresholds { stale: 2.0, hidden: 4.0, archive: 8.0 }),
            enabled,
            source: RuleSource::User,
            created_at: 0,
            hit_count: 0,
        };
        store.insert_rule(&rule("r1", "kept.com", RuleAction::Protect, true)).unwrap();
        store.insert_rule(&rule("r2", "timed.com", RuleAction::CustomThreshold, true)).unwrap();
        store.insert_rule(&rule("r3", "off.com", RuleAction::Protect, false)).unwrap();
        let s = summary_for(&store, day, day + 86_400_000, day + 86_400_000).unwrap();
        let mut got: Vec<(String, HintKind)> =
            s.hints.into_iter().map(|h| (h.domain, h.kind)).collect();
        got.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(
            got,
            vec![
                ("both.com".into(), HintKind::AlwaysKeep),
                ("off.com".into(), HintKind::AlwaysKeep)
            ]
        );
    }
}
