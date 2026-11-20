//! On-device success metrics (P2.7, PRD F34 / §9, DB Design §3.10).
//!
//! - **`metrics_daily`, one row per local day, from the engine and the search UIs:** active
//!   seconds and whether automatic mode was on (every tick with active time), once per
//!   local hour of active use the open tabs and windows (their medians), one search per
//!   search panel / popup opening and the time from its first keystroke to a restore.
//! - **`metric_events`, copied as things happen (triggers on `events`, see migration V3):**
//!   archived, restored within 24 h, sweeps undone, shadow flags, and shadow regret
//!   ([`Store::metrics_note_deliberate_use`]). Not tied to items, so a deleted item never
//!   changes a count. The engine stamps each with its local day within a tick
//!   ([`Store::metrics_date_events`]), so a later time-zone change doesn't move it.
//!
//! Counts and medians only: no item, address, title or site.

use rusqlite::{OptionalExtension, params};
use serde::Serialize;

use crate::{Result, Store};

/// Samples kept per day (hourly counts, search times).
pub const MAX_SAMPLES: usize = 48;
/// Longer search → restore times are an idle panel, not a search.
pub const MAX_SEARCH_TO_RESTORE_MS: u64 = 120_000;
const DAY_MS: i64 = 86_400_000;
/// Events dated per call (a backfilled database catches up over a few ticks).
const DATE_BATCH: u32 = 5_000;

/// One day as Insights reads it (the extension's `MetricsDay` shape, camelCase).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricsDay {
    pub day: String,
    pub active_seconds: u64,
    pub open_tabs_p50: u32,
    pub open_windows_p50: u32,
    pub archived_count: u32,
    #[serde(rename = "restored24hCount")]
    pub restored_24h_count: u32,
    pub undo_count: u32,
    pub search_count: u32,
    pub search_to_restore_ms_p50: u32,
    pub auto_mode_on: u8,
    pub shadow_flagged_count: u32,
    pub shadow_regret_count: u32,
    pub sampled_hours: Vec<u8>,
    pub open_tabs_samples: Vec<u32>,
    pub open_windows_samples: Vec<u32>,
    pub search_to_restore_samples: Vec<u32>,
}

/// `(site, count)` rows, most first.
pub type SiteCounts = Vec<(String, u32)>;

/// Archived items of one app and browser profile ([`Store::metrics_top_sources`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceCount {
    pub app_id: String,
    pub profile: Option<String>,
    pub tabs: u32,
    pub windows: u32,
}

/// Median of the samples, rounded; 0 for none.
pub fn median(samples: &[u32]) -> u32 {
    let mut s = samples.to_vec();
    s.sort_unstable();
    match s.len() {
        0 => 0,
        n if n % 2 == 1 => s[n / 2],
        n => (s[n / 2 - 1] + s[n / 2]).div_ceil(2),
    }
}

fn list<T: serde::de::DeserializeOwned>(json: &str) -> Vec<T> {
    serde_json::from_str(json).unwrap_or_default()
}

fn push<T: Serialize>(mut samples: Vec<T>, value: T) -> String {
    samples.push(value);
    let skip = samples.len().saturating_sub(MAX_SAMPLES);
    serde_json::to_string(&samples[skip..]).unwrap_or_else(|_| "[]".into())
}

/// A stored count as `u32`: capped, never negative.
fn count(n: i64) -> u32 {
    u32::try_from(n.max(0)).unwrap_or(u32::MAX)
}

impl Store {
    fn ensure_metrics_day(&self, day: &str) -> Result<()> {
        self.conn.execute("INSERT OR IGNORE INTO metrics_daily (day) VALUES (?1)", [day])?;
        Ok(())
    }

    /// Adds active time to `day`, and notes whether automatic mode was on (a day of use
    /// in automatic mode keeps its run going, with a browser or not).
    pub fn metrics_add_active(&self, day: &str, seconds: u64, automatic: bool) -> Result<()> {
        if seconds == 0 {
            return Ok(());
        }
        self.ensure_metrics_day(day)?;
        self.conn.execute(
            "UPDATE metrics_daily SET active_seconds = active_seconds + ?2,
               auto_mode_on = max(auto_mode_on, ?3)
             WHERE day = ?1",
            params![day, i64::try_from(seconds).unwrap_or(i64::MAX), i64::from(automatic)],
        )?;
        Ok(())
    }

    /// The hourly sample of open tabs and windows (their medians), at most once per local
    /// `hour` of `day`; none without open tabs. Returns whether it was taken.
    pub fn metrics_sample_hour(
        &self,
        day: &str,
        hour: u8,
        tabs: u32,
        windows: u32,
    ) -> Result<bool> {
        if tabs == 0 {
            return Ok(false);
        }
        self.ensure_metrics_day(day)?;
        let (hours, tab_json, win_json): (String, String, String) = self.conn.query_row(
            "SELECT sampled_hours, open_tabs_samples, open_windows_samples FROM metrics_daily
             WHERE day = ?1",
            [day],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        let hours: Vec<u8> = list(&hours);
        if hours.contains(&hour) {
            return Ok(false);
        }
        let tab_samples = push(list::<u32>(&tab_json), tabs);
        let win_samples = push(list::<u32>(&win_json), windows);
        self.conn.execute(
            "UPDATE metrics_daily SET sampled_hours = ?2, open_tabs_samples = ?3,
               open_windows_samples = ?4, open_tabs_p50 = ?5, open_windows_p50 = ?6
             WHERE day = ?1",
            params![
                day,
                push(hours, hour),
                tab_samples,
                win_samples,
                median(&list::<u32>(&tab_samples)),
                median(&list::<u32>(&win_samples)),
            ],
        )?;
        Ok(true)
    }

    /// One search (a search panel or popup opening with a query).
    pub fn metrics_record_search(&self, day: &str) -> Result<()> {
        self.ensure_metrics_day(day)?;
        self.conn.execute(
            "UPDATE metrics_daily SET search_count = search_count + 1 WHERE day = ?1",
            [day],
        )?;
        Ok(())
    }

    /// Time from a search's first keystroke to restoring one of its results (dropped
    /// over [`MAX_SEARCH_TO_RESTORE_MS`]).
    pub fn metrics_record_search_time(&self, day: &str, ms: u64) -> Result<()> {
        if ms > MAX_SEARCH_TO_RESTORE_MS {
            return Ok(());
        }
        self.ensure_metrics_day(day)?;
        let samples: String = self.conn.query_row(
            "SELECT search_samples FROM metrics_daily WHERE day = ?1",
            [day],
            |r| r.get(0),
        )?;
        let samples = push(list::<u32>(&samples), u32::try_from(ms).unwrap_or(u32::MAX));
        self.conn.execute(
            "UPDATE metrics_daily SET search_samples = ?2, search_to_restore_ms_p50 = ?3
             WHERE day = ?1",
            params![day, samples, median(&list::<u32>(&samples))],
        )?;
        Ok(())
    }

    /// A deliberate use of an item (activated, typed in, its window focused): each
    /// shadow flag of it from the last 24 h becomes a regret, once, on the flag's day.
    pub fn metrics_note_deliberate_use(&self, item_id: &str, at: i64) -> Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO metric_events (kind, at, ref)
             SELECT 'shadow_regret', f.at, CAST(f.id AS TEXT) FROM events f
             WHERE f.item_id = ?1 AND f.type = 'shadow_would_archive'
               AND f.at <= ?2 AND f.at > ?2 - ?3",
            params![item_id, at, DAY_MS],
        )?;
        Ok(())
    }

    /// Stamps events not dated yet with their local day (`day_of(at)`; `None` leaves one
    /// for later). Returns how many were dated.
    pub fn metrics_date_events(&self, day_of: impl Fn(i64) -> Option<String>) -> Result<usize> {
        let undated: Vec<(i64, i64)> = {
            let mut stmt = self.conn.prepare(
                "SELECT id, at FROM metric_events WHERE day IS NULL ORDER BY id LIMIT ?1",
            )?;
            let rows = stmt.query_map([DATE_BATCH], |r| Ok((r.get(0)?, r.get(1)?)))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        let mut dated = 0;
        for (id, at) in undated {
            if let Some(day) = day_of(at) {
                dated += self
                    .conn
                    .execute("UPDATE metric_events SET day = ?2 WHERE id = ?1", params![id, day])?;
            }
        }
        Ok(dated)
    }

    /// Everything counted since `since` (ms): `(kind, at, local day if dated)`, oldest
    /// first.
    pub fn metric_events_since(&self, since: i64) -> Result<Vec<(String, i64, Option<String>)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT kind, at, day FROM metric_events WHERE at >= ?1 ORDER BY at")?;
        let rows = stmt.query_map([since], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Forgets counts and days older than `before` (ms / `YYYY-MM-DD`).
    pub fn metrics_prune(&self, before: i64, before_day: &str) -> Result<()> {
        self.conn.execute("DELETE FROM metric_events WHERE at < ?1", [before])?;
        self.conn.execute("DELETE FROM metrics_daily WHERE day < ?1", [before_day])?;
        Ok(())
    }

    /// Every stored day (oldest first, at most the last 400), without event counts (the
    /// caller adds them from [`Self::metric_events_since`]).
    pub fn metrics_days(&self) -> Result<Vec<MetricsDay>> {
        let mut stmt = self.conn.prepare(
            "SELECT day, active_seconds, open_tabs_p50, open_windows_p50, search_count,
               search_to_restore_ms_p50, auto_mode_on, sampled_hours, open_tabs_samples,
               open_windows_samples, search_samples
             FROM (SELECT * FROM metrics_daily ORDER BY day DESC LIMIT 400) ORDER BY day",
        )?;
        let rows = stmt.query_map([], |r| {
            let n = |i: usize| -> rusqlite::Result<u32> {
                Ok(count(r.get::<_, Option<i64>>(i)?.unwrap_or(0)))
            };
            Ok(MetricsDay {
                day: r.get(0)?,
                active_seconds: u64::try_from(r.get::<_, Option<i64>>(1)?.unwrap_or(0).max(0))
                    .unwrap_or(0),
                open_tabs_p50: n(2)?,
                open_windows_p50: n(3)?,
                search_count: n(4)?,
                search_to_restore_ms_p50: n(5)?,
                auto_mode_on: u8::from(n(6)? > 0),
                sampled_hours: list(&r.get::<_, String>(7)?),
                open_tabs_samples: list(&r.get::<_, String>(8)?),
                open_windows_samples: list(&r.get::<_, String>(9)?),
                search_to_restore_samples: list(&r.get::<_, String>(10)?),
                ..MetricsDay::default()
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Sites archived most since `since` (ms), and restored most (all time, Undo
    /// included): `(site, count)`, at most `limit` each. For Insights only; the beta
    /// export never carries sites.
    pub fn metrics_top_sites(&self, since: i64, limit: u32) -> Result<(SiteCounts, SiteCounts)> {
        let rows = |sql: &str, args: &[&dyn rusqlite::ToSql]| -> Result<SiteCounts> {
            let mut stmt = self.conn.prepare(sql)?;
            let out = stmt
                .query_map(args, |r| Ok((r.get(0)?, count(r.get(1)?))))?
                .collect::<rusqlite::Result<_>>()?;
            Ok(out)
        };
        let archived = rows(
            "SELECT domain, count(*) AS n FROM items
             WHERE domain IS NOT NULL AND archived_at >= ?1
             GROUP BY domain ORDER BY n DESC, domain LIMIT ?2",
            params![since, limit],
        )?;
        let restored = rows(
            "SELECT domain, sum(restore_count) AS n FROM items
             WHERE domain IS NOT NULL AND restore_count > 0
             GROUP BY domain ORDER BY n DESC, domain LIMIT ?1",
            params![limit],
        )?;
        Ok((archived, restored))
    }

    /// What was archived since `since` by app and browser profile (P2.8, Insights' top
    /// sources): `(app id, profile id, tabs, windows)`, most first. Items without a known
    /// profile count under their app (`None`).
    pub fn metrics_top_sources(&self, since: i64, limit: u32) -> Result<Vec<SourceCount>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT i.app_id, s.profile, sum(i.kind = 'tab'), sum(i.kind = 'window'), count(*) AS n
             FROM items i LEFT JOIN sources s ON s.id = i.source_id
             WHERE i.archived_at >= ?1
             GROUP BY i.app_id, s.profile ORDER BY n DESC, i.app_id, s.profile LIMIT ?2",
        )?;
        let out = stmt
            .query_map(params![since, limit], |r| {
                Ok(SourceCount {
                    app_id: r.get(0)?,
                    profile: r.get(1)?,
                    tabs: count(r.get(2)?),
                    windows: count(r.get(3)?),
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(out)
    }

    /// Everything archived since `since` (Insights' top-source shares are of this).
    pub fn metrics_archived_since(&self, since: i64) -> Result<u32> {
        let n: i64 = self.conn.query_row(
            "SELECT count(*) FROM items WHERE archived_at >= ?1",
            [since],
            |r| r.get(0),
        )?;
        Ok(count(n))
    }

    /// A random id for the beta export, made once on this installation.
    pub fn beta_install_id(&self, now: i64) -> Result<String> {
        let stored: Option<String> = self
            .conn
            .query_row("SELECT value FROM settings WHERE key = 'beta_install_id'", [], |r| r.get(0))
            .optional()?
            .and_then(|v: String| serde_json::from_str(&v).ok());
        if let Some(id) = stored {
            return Ok(id);
        }
        // Random, not time-based: it mustn't say when Decay was installed.
        let mut bytes = [0u8; 16];
        getrandom::fill(&mut bytes)
            .map_err(|e| crate::StoreError::Io(std::io::Error::other(e.to_string())))?;
        let id = bytes.iter().fold(String::with_capacity(32), |mut s, b| {
            use std::fmt::Write as _;
            let _ = write!(s, "{b:02x}");
            s
        });
        self.set_setting("beta_install_id", &id, now)?;
        Ok(id)
    }
}

#[cfg(test)]
mod tests {
    use decay_core::{EventType, ItemKind};

    use super::*;
    use crate::items::Item;
    use crate::test_util::*;

    const H: i64 = 3_600_000;

    #[test]
    fn hourly_samples_once_per_hour_with_medians() {
        let (store, _dir) = store();
        assert!(!store.metrics_sample_hour("2026-11-04", 9, 0, 0).unwrap(), "no tabs");
        assert!(store.metrics_sample_hour("2026-11-04", 9, 10, 2).unwrap());
        assert!(!store.metrics_sample_hour("2026-11-04", 9, 99, 9).unwrap(), "same hour");
        assert!(store.metrics_sample_hour("2026-11-04", 10, 20, 3).unwrap());
        assert!(store.metrics_sample_hour("2026-11-04", 11, 31, 3).unwrap());
        store.metrics_add_active("2026-11-04", 120, false).unwrap();
        store.metrics_add_active("2026-11-04", 0, true).unwrap(); // no time: no mode either
        store.metrics_add_active("2026-11-04", 30, true).unwrap();
        store.metrics_record_search("2026-11-04").unwrap();
        store.metrics_record_search_time("2026-11-04", 3_000).unwrap();
        store.metrics_record_search_time("2026-11-04", 5_000).unwrap();
        store.metrics_record_search_time("2026-11-04", 999_999).unwrap();
        let days = store.metrics_days().unwrap();
        let d = &days[0];
        assert_eq!(
            (d.open_tabs_p50, d.open_windows_p50, d.auto_mode_on, d.active_seconds),
            (20, 3, 1, 150)
        );
        assert_eq!(d.sampled_hours, [9, 10, 11]);
        assert_eq!((d.search_count, d.search_to_restore_ms_p50), (1, 4_000));
        assert_eq!(d.search_to_restore_samples, [3_000, 5_000], "idle times dropped");
    }

    #[test]
    fn counts_are_copied_as_they_happen_and_outlive_the_items() {
        let (store, _dir) = store();
        let t0 = NOW - 12 * H;
        let mut ids = Vec::new();
        for i in 0..4 {
            let item = Item::new_live(format!("i{i}"), ItemKind::Tab, "com.google.Chrome", NOW);
            store.insert_item(&item).unwrap();
            ids.push(item.id);
        }
        let ev =
            |id: &str, t: EventType, at: i64| store.append_event(id, t, None, None, at).unwrap();
        ev(&ids[0], EventType::Archived, t0 + H);
        ev(&ids[0], EventType::Restored, t0 + 2 * H); // within a day: regret
        ev(&ids[1], EventType::Archived, t0 - 3 * DAY_MS);
        ev(&ids[1], EventType::Restored, t0 + 3 * H); // days later: not regret
        ev(&ids[2], EventType::ShadowWouldArchive, t0 + H);
        store.metrics_note_deliberate_use(&ids[2], t0 + 5 * H).unwrap(); // shadow regret
        store.metrics_note_deliberate_use(&ids[2], t0 + 6 * H).unwrap(); // the same flag
        ev(&ids[3], EventType::ShadowWouldArchive, t0 + H);
        store.metrics_note_deliberate_use(&ids[3], t0 + 2 * DAY_MS).unwrap(); // too late
        let kinds = |store: &Store| {
            let mut k: Vec<String> =
                store.metric_events_since(0).unwrap().into_iter().map(|(k, ..)| k).collect();
            k.sort();
            k
        };
        let expect = [
            "archived",
            "archived",
            "restored_24h",
            "shadow_flagged",
            "shadow_flagged",
            "shadow_regret",
        ];
        assert_eq!(kinds(&store), expect);
        // The user closes the flagged tabs, deletes the archived ones: nothing changes.
        for id in &ids {
            store.delete_item(id).unwrap();
        }
        assert_eq!(kinds(&store), expect);
        let regret = store.metric_events_since(0).unwrap();
        let flag_at = regret.iter().find(|(k, ..)| k == "shadow_regret").unwrap().1;
        assert_eq!(flag_at, t0 + H, "on the flag's day");
        store.metrics_prune(NOW + 1, "9999-01-01").unwrap();
        assert_eq!(store.metric_events_since(0).unwrap(), vec![]);
    }

    #[test]
    fn top_sources_by_app_and_profile() {
        let (store, _dir) = store();
        let chrome = "com.google.Chrome";
        for (src, profile) in [("s1", "pa"), ("s2", "pb")] {
            store
                .upsert_source(&crate::Source {
                    id: src.into(),
                    app_id: chrome.into(),
                    profile: Some(profile.into()),
                    external_id: None,
                    title: None,
                    first_seen_at: NOW,
                    last_seen_at: NOW,
                    closed_at: None,
                })
                .unwrap();
        }
        let rows = [("s1", NOW), ("s1", NOW), ("s2", NOW), ("", NOW), ("s2", NOW - 10)];
        for (i, (src, at)) in rows.iter().enumerate() {
            let mut item = Item::new_live(format!("t{i}"), ItemKind::Tab, chrome, NOW);
            item.source_id = (!src.is_empty()).then(|| (*src).to_owned());
            item.state = decay_core::ItemState::Archived;
            item.archived_at = Some(*at);
            store.insert_item(&item).unwrap();
        }
        let got = store.metrics_top_sources(NOW - 1, 5).unwrap();
        let got: Vec<_> = got.iter().map(|c| (c.profile.as_deref(), c.tabs, c.windows)).collect();
        assert_eq!(got, [(Some("pa"), 2, 0), (None, 1, 0), (Some("pb"), 1, 0)]);
    }

    #[test]
    fn top_sites_and_install_id() {
        let (store, _dir) = store();
        for (i, (domain, restores)) in [("a.com", 0), ("a.com", 2), ("b.com", 5)].iter().enumerate()
        {
            let mut item = Item::new_live(format!("t{i}"), ItemKind::Tab, "com.google.Chrome", NOW);
            item.domain = Some((*domain).into());
            item.state = decay_core::ItemState::Archived;
            item.archived_at = Some(NOW);
            item.restore_count = *restores;
            store.insert_item(&item).unwrap();
        }
        let (archived, restored) = store.metrics_top_sites(NOW - 1, 5).unwrap();
        assert_eq!(archived, [("a.com".to_owned(), 2), ("b.com".to_owned(), 1)]);
        assert_eq!(restored, [("b.com".to_owned(), 5), ("a.com".to_owned(), 2)]);
        let id = store.beta_install_id(NOW).unwrap();
        assert_eq!(store.beta_install_id(NOW + 1).unwrap(), id, "made once");
    }

    #[test]
    fn events_keep_the_day_they_were_dated_with() {
        let (store, _dir) = store();
        let item = Item::new_live("i".into(), ItemKind::Tab, "com.google.Chrome", NOW);
        store.insert_item(&item).unwrap();
        store.append_event(&item.id, EventType::Archived, None, None, NOW).unwrap();
        assert_eq!(store.metric_events_since(0).unwrap()[0].2, None, "not dated yet");
        assert_eq!(store.metrics_date_events(|_| None).unwrap(), 0, "left for later");
        assert_eq!(store.metrics_date_events(|_| Some("2026-11-04".into())).unwrap(), 1);
        // Travelling later (another zone would say another day) changes nothing.
        assert_eq!(store.metrics_date_events(|_| Some("2026-11-05".into())).unwrap(), 0);
        assert_eq!(store.metric_events_since(0).unwrap()[0].2.as_deref(), Some("2026-11-04"));
    }

    #[test]
    fn counts_are_capped_not_zeroed() {
        assert_eq!(count(-1), 0);
        assert_eq!(count(7), 7);
        assert_eq!(count(i64::from(u32::MAX) + 1), u32::MAX);
        let (store, _dir) = store();
        store.metrics_add_active("2026-11-04", 5_000_000_000, false).unwrap();
        assert_eq!(store.metrics_days().unwrap()[0].active_seconds, 5_000_000_000);
    }

    #[test]
    fn medians() {
        assert_eq!(median(&[]), 0);
        assert_eq!(median(&[3, 1, 2]), 2);
        assert_eq!(median(&[1, 2]), 2, "rounded");
        assert_eq!(median(&[1, 4]), 3);
    }
}
