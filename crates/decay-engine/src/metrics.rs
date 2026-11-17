//! On-device success metrics (P2.7, PRD F34 / §9): what the engine samples each tick,
//! the local day it stamps on each counted event, and what Insights reads. Storage and
//! definitions are in
//! `decay_store::metrics`; the views and goals are shared with the extension
//! (`@decay/ui/metrics`).

use decay_core::Mode;
use decay_store::MetricsDay;
use jiff::tz::TimeZone;
use serde::Serialize;

use crate::engine::{Engine, Res};

/// Days of metrics kept.
const KEPT_DAYS: i64 = 400;

/// The local day (`YYYY-MM-DD`) and hour of `now` (ms) in `tz`.
pub(crate) fn local_day(now: i64, tz: &TimeZone) -> Option<(String, u8)> {
    let zoned = jiff::Timestamp::from_millisecond(now).ok()?.to_zoned(tz.clone());
    let hour = u8::try_from(zoned.hour()).ok()?;
    Some((zoned.date().to_string(), hour))
}

/// What Insights reads (`get_metrics`).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricsReport {
    pub mode: Mode,
    /// Every stored day, oldest first.
    pub days: Vec<MetricsDay>,
    /// Sites archived most since the range's first day.
    pub top_archived: Vec<SiteCount>,
    /// Sites restored most (all time, Undo included).
    pub restored_most: Vec<SiteCount>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SiteCount {
    pub domain: String,
    pub count: u32,
}

fn sites(rows: Vec<(String, u32)>) -> Vec<SiteCount> {
    rows.into_iter().map(|(domain, count)| SiteCount { domain, count }).collect()
}

impl Engine {
    /// The zone metrics days are local to: the system's now (a change while running is
    /// picked up), unless a test set one.
    fn zone(&self) -> TimeZone {
        self.time_zone.clone().unwrap_or_else(TimeZone::system)
    }

    /// Each tick with active time (Phase 0's definition): the time, whether automatic mode
    /// is on, and the hourly sample of open tabs in normal windows and of the windows
    /// holding them (none while no browser is connected).
    pub(crate) fn note_metrics(&mut self, credit_secs: u64, now: i64) -> Res {
        self.date_metric_events()?;
        if credit_secs == 0 {
            return Ok(());
        }
        let Some((today, hour)) = local_day(now, &self.zone()) else { return Ok(()) };
        let automatic = self.policy.mode == Mode::Automatic;
        self.store.metrics_add_active(&today, credit_secs, automatic)?;
        if self.metrics_hour.as_ref() == Some(&(today.clone(), hour)) {
            return Ok(());
        }
        let (mut tabs, mut windows) = (0u32, 0u32);
        for conn in self.conns.values() {
            let Some(a) = Self::live_adapter(conn, now) else { continue };
            let normal = |w: i64| a.windows.get(&w).is_none_or(|w| !w.incognito);
            let mut seen = std::collections::HashSet::new();
            for t in a.tabs.values().filter(|t| normal(t.info.window_id)) {
                tabs = tabs.saturating_add(1);
                seen.insert(t.info.window_id);
            }
            windows = windows.saturating_add(u32::try_from(seen.len()).unwrap_or(u32::MAX));
        }
        if self.store.metrics_sample_hour(&today, hour, tabs, windows)? {
            self.metrics_hour = Some((today, hour));
        }
        Ok(())
    }

    /// Stamps counted events with their local day in the zone of now (each tick, so
    /// within seconds of the event): a later time-zone change doesn't move them.
    fn date_metric_events(&self) -> Res {
        let tz = self.zone();
        self.store.metrics_date_events(|at| local_day(at, &tz).map(|(day, _)| day))?;
        Ok(())
    }

    /// Hourly: forgets metrics older than [`KEPT_DAYS`].
    pub(crate) fn prune_metrics(&self, now: i64) -> Res {
        let before = now - KEPT_DAYS * 86_400_000;
        match local_day(before, &self.zone()) {
            Some((day, _)) => self.store.metrics_prune(before, &day),
            None => Ok(()),
        }
    }

    /// One search in the search panel or the extension's popup (once per opening).
    pub fn record_search(&self, now: i64) -> Res {
        match local_day(now, &self.zone()) {
            Some((today, _)) => self.store.metrics_record_search(&today),
            None => Ok(()),
        }
    }

    /// Search → restore time, once the restore reopened something.
    pub(crate) fn record_search_time(&self, ms: u64, now: i64) -> Res {
        match local_day(now, &self.zone()) {
            Some((today, _)) => self.store.metrics_record_search_time(&today, ms),
            None => Ok(()),
        }
    }

    /// Insights' data: every day (samples plus what happened, on the local day each
    /// event was stamped with), and the top sites since `since` (ms).
    pub fn metrics(&self, since: i64, now: i64) -> Res<MetricsReport> {
        self.date_metric_events()?;
        let tz = self.zone();
        let mut days: std::collections::BTreeMap<String, MetricsDay> =
            self.store.metrics_days()?.into_iter().map(|d| (d.day.clone(), d)).collect();
        for (kind, at, day) in self.store.metric_events_since(now - KEPT_DAYS * 86_400_000)? {
            // Undated only if more than a batch arrived at once: today's zone.
            let Some(day) = day.or_else(|| local_day(at, &tz).map(|(d, _)| d)) else { continue };
            let row = days
                .entry(day.clone())
                .or_insert_with(|| MetricsDay { day, ..MetricsDay::default() });
            let n = match kind.as_str() {
                "archived" => &mut row.archived_count,
                "restored_24h" => &mut row.restored_24h_count,
                "undo" => &mut row.undo_count,
                "shadow_flagged" => &mut row.shadow_flagged_count,
                "shadow_regret" => &mut row.shadow_regret_count,
                _ => continue,
            };
            *n = n.saturating_add(1);
        }
        let (archived, restored) = self.store.metrics_top_sites(since, 5)?;
        Ok(MetricsReport {
            mode: self.policy.mode,
            days: days.into_values().collect(),
            top_archived: sites(archived),
            restored_most: sites(restored),
        })
    }

    /// The time zone metrics days are counted in (tests; the app follows the system's).
    pub fn set_time_zone(&mut self, tz: TimeZone) {
        self.time_zone = Some(tz);
        self.metrics_hour = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn days_follow_the_zone_and_daylight_saving() {
        let ny = TimeZone::get("America/New_York").unwrap();
        // 2026-11-01: DST ends, a 25-hour day, 01:00 twice.
        let start = jiff::civil::date(2026, 11, 1).to_zoned(ny.clone()).unwrap();
        let ms = start.timestamp().as_millisecond();
        let h = 3_600_000;
        assert_eq!(local_day(ms + h, &ny).unwrap(), ("2026-11-01".into(), 1));
        assert_eq!(local_day(ms + 2 * h, &ny).unwrap(), ("2026-11-01".into(), 1), "again");
        assert_eq!(local_day(ms + 24 * h, &ny).unwrap(), ("2026-11-01".into(), 23));
        assert_eq!(local_day(ms + 25 * h, &ny).unwrap(), ("2026-11-02".into(), 0));
        assert_eq!(local_day(ms - 1, &ny).unwrap(), ("2026-10-31".into(), 23));
    }
}
