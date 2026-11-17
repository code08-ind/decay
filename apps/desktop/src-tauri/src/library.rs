//! The Library window's commands (P1.11, PRD F2/F7/F22, API Design §2.1–2.2): stat
//! cards, item details with their timeline, deleting archived items, the sweep history
//! with each sweep's items, and the Shadow review.
//!
//! Each builder takes the store (and what the engine knows) and is tested directly; the
//! Tauri commands only run them on the engine thread.

#![allow(clippy::needless_pass_by_value, reason = "Tauri commands take their arguments by value")]

use std::collections::{HashMap, HashSet};

use decay_core::{
    EventType, ItemKind, ItemState, Mode, ProtectedBy, Reason, SweepAction, SweepItemResult,
    SweepStatus, SweepTrigger,
};
use decay_store::{Item, Store, Sweep};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tauri::AppHandle;

use crate::commands::{Res, blocking};
use crate::core::AppError;
use crate::i18n::{t, tf};
use crate::search::{PanelItem, reason_text, row};

/// Shadow mode lasts this many days (PRD F2).
pub const SHADOW_DAYS: u32 = 7;

fn db(e: impl std::fmt::Display) -> AppError {
    AppError::db(&e)
}

// ── Stats ───────────────────────────────────────────────────────────────────────────

/// `library_stats` input: the local calendar's boundaries (the UI knows the user's
/// time zone and first day of the week).
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatsInput {
    pub today_start: i64,
    pub week_start: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KindCount {
    pub tabs: u32,
    pub windows: u32,
}

/// The Library's stat cards and shadow-mode banner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryStats {
    pub archived_total: u64,
    pub archived_today: KindCount,
    /// Items restored this week.
    pub restored_week: u32,
    pub open_tabs: u32,
    pub mode: Mode,
    /// Day of shadow mode (1–7; 8 and on once the week is over); `None` outside it.
    pub shadow_day: Option<u32>,
}

/// The calendar day of shadow mode at `now` in `tz` (day 1 is the day it started, day 2
/// starts at the next local midnight).
pub fn shadow_day(started_at: Option<i64>, now: i64, tz: &jiff::tz::TimeZone) -> u32 {
    let date =
        |ms: i64| jiff::Timestamp::from_millisecond(ms).map(|t| t.to_zoned(tz.clone()).date()).ok();
    let (Some(start), Some(today)) = (started_at.and_then(date), date(now)) else { return 1 };
    let days = start.until(today).map_or(0, |span| span.get_days());
    u32::try_from(days.max(0)).unwrap_or(u32::MAX).saturating_add(1)
}

pub fn stats_for(
    store: &Store,
    input: StatsInput,
    mode: Mode,
    open_tabs: u32,
    now: i64,
    tz: &jiff::tz::TimeZone,
) -> Res<LibraryStats> {
    let (tabs, windows) = store.archived_since(input.today_start).map_err(db)?;
    let started = store.setting::<i64>("shadow_started_at").map_err(db)?;
    Ok(LibraryStats {
        archived_total: store.archived_count().map_err(db)?,
        archived_today: KindCount { tabs, windows },
        restored_week: store
            .items_with_event_since(&[EventType::Restored, EventType::Undo], input.week_start)
            .map_err(db)?,
        open_tabs,
        mode,
        shadow_day: (mode == Mode::Shadow).then(|| shadow_day(started, now, tz)),
    })
}

#[tauri::command]
pub async fn library_stats(app: AppHandle, input: StatsInput) -> Res<LibraryStats> {
    blocking(app, move |core| {
        core.ask(move |e, now| {
            let status = e.status(now);
            let tz = jiff::tz::TimeZone::system();
            (stats_for(e.store(), input, status.mode, status.live_tabs, now, &tz), Vec::new())
        })?
    })
    .await
}

// ── Item details ────────────────────────────────────────────────────────────────────

/// One step of an item's timeline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineEvent {
    #[serde(rename = "type")]
    pub kind: EventType,
    pub at: i64,
    /// The reason in words, when there is one.
    pub label: Option<String>,
}

/// `get_item`: an item with what the drawer shows.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemDetail {
    #[serde(flatten)]
    pub row: PanelItem,
    pub state: ItemState,
    pub pinned: bool,
    pub group_title: Option<String>,
    pub first_seen_at: i64,
    pub last_used_at: i64,
    pub active_hours: u64,
    pub sweep_id: Option<String>,
    pub sweep_number: Option<u32>,
    pub restore_count: u32,
    /// Newest first.
    pub events: Vec<TimelineEvent>,
}

/// Words for a protection ("Pinned", "Playing audio").
pub fn protected_text(by: ProtectedBy) -> String {
    t(match by {
        ProtectedBy::Pinned => "protected.pinned",
        ProtectedBy::ActiveTab => "protected.activeTab",
        ProtectedBy::Audible => "protected.audible",
        ProtectedBy::FormInput => "protected.formInput",
        ProtectedBy::Download => "protected.download",
        ProtectedBy::Call => "protected.call",
        ProtectedBy::Unsaved => "protected.unsaved",
        ProtectedBy::UnknownSaveState => "protected.unknownSaveState",
        ProtectedBy::Rule => "protected.rule",
        ProtectedBy::Hold => "protected.hold",
        ProtectedBy::Meeting => "protected.meeting",
        ProtectedBy::Grace => "protected.grace",
        ProtectedBy::Unrestorable => "protected.unrestorable",
    })
}

fn label(reason: Option<&Reason>) -> Option<String> {
    match reason? {
        Reason::Protected { by, .. } => {
            Some(tf("protected.reason", &[("why", protected_text(*by).into())]))
        }
        other => reason_text(other),
    }
}

pub fn item_detail(store: &Store, id: &str) -> Res<ItemDetail> {
    let item = store
        .item(id)
        .map_err(db)?
        .ok_or_else(|| AppError::new("NOT_FOUND", t("error.itemGone")))?;
    let events = store
        .events_for_item(id, 200)
        .map_err(db)?
        .into_iter()
        .map(|e| TimelineEvent { kind: e.kind, at: e.at, label: label(e.reason.as_ref()) })
        .collect();
    let sweep_number = match item.last_sweep_id.as_deref() {
        Some(sid) => match store.sweep(sid).map_err(db)? {
            Some(s) => Some(store.sweep_number(&s).map_err(db)?),
            None => None,
        },
        None => None,
    };
    Ok(ItemDetail {
        state: item.state,
        pinned: item.pinned,
        group_title: item.group_title.clone(),
        first_seen_at: item.first_seen_at,
        last_used_at: item.last_used_at,
        active_hours: item.active_age_s / 3600,
        sweep_id: item.last_sweep_id.clone(),
        sweep_number,
        restore_count: item.restore_count,
        events,
        row: row(store, item, &mut HashMap::new()),
    })
}

fn valid_id(id: &str) -> bool {
    (1..=64).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

#[tauri::command]
pub async fn get_item(app: AppHandle, id: String) -> Res<ItemDetail> {
    if !valid_id(&id) {
        return Err(AppError::new("INVALID_INPUT", t("error.notItem")));
    }
    blocking(app, move |core| core.ask(move |e, _| (item_detail(e.store(), &id), Vec::new()))?)
        .await
}

/// Deletes archived items for good. Live items, and items being restored right now
/// (`busy`: their tab is reopening), are left alone and not counted.
pub fn delete_archived(store: &mut Store, ids: &[String], busy: impl Fn(&str) -> bool) -> Res<u32> {
    store
        .atomically(true, |s| {
            let mut deleted = 0u32;
            for id in ids {
                if !busy(id)
                    && s.item(id)?.is_some_and(|i| i.state == ItemState::Archived)
                    && s.delete_item(id)?
                {
                    deleted += 1;
                }
            }
            Ok(deleted)
        })
        .map_err(db)
}

#[tauri::command]
pub async fn delete_items(app: AppHandle, ids: Vec<String>) -> Res<Value> {
    if ids.is_empty() || ids.len() > 500 || !ids.iter().all(|i| valid_id(i)) {
        return Err(AppError::new("INVALID_INPUT", t("error.delete")));
    }
    blocking(app, move |core| {
        core.ask(move |e, _| {
            let busy: HashSet<String> = ids.iter().filter(|i| e.is_restoring(i)).cloned().collect();
            let deleted = delete_archived(e.store_mut(), &ids, |id| busy.contains(id));
            (deleted.map(|n| json!({ "deleted": n })), Vec::new())
        })?
    })
    .await
}

// ── Sweeps ──────────────────────────────────────────────────────────────────────────

/// One sweep in the history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SweepRow {
    pub id: String,
    pub number: u32,
    pub started_at: i64,
    pub trigger: SweepTrigger,
    pub is_shadow: bool,
    pub status: SweepStatus,
    /// Items it archived (or would have): stays right after items are deleted or purged.
    pub item_count: u32,
    /// Of those still in the database: tabs and windows.
    pub tabs: u32,
    pub windows: u32,
    /// Of its items, how many are back (restored since).
    pub restored: u32,
    pub failed: u32,
    pub undone_at: Option<i64>,
    /// Undo would bring something back, and the engine accepts it (the last 20 real
    /// sweeps that archived something).
    pub undoable: bool,
}

/// One item of a sweep.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SweepEntry {
    #[serde(flatten)]
    pub row: PanelItem,
    pub state: ItemState,
    /// Still archived from this sweep (can be restored from here).
    pub archived: bool,
}

/// The items a sweep touched: archived ones for a real sweep, the would-be ones (with
/// their reasons) for a shadow sweep.
fn sweep_items(store: &Store, sweep: &Sweep) -> Res<Vec<(Item, Option<Reason>)>> {
    if sweep.is_shadow {
        let events = store
            .events_since(EventType::ShadowWouldArchive, sweep.started_at, Some(&sweep.id), 1000)
            .map_err(db)?;
        let mut out = Vec::new();
        for e in events {
            if let Some(item) = store.item(&e.item_id).map_err(db)? {
                out.push((item, e.reason));
            }
        }
        return Ok(out);
    }
    let mut out = Vec::new();
    for si in store.sweep_items(&sweep.id).map_err(db)? {
        // Only what it really archived: a tab the browser kept open (skipped) or that
        // failed to close was never archived, and isn't "restored" either.
        if si.action != SweepAction::Archive
            || !matches!(si.result, SweepItemResult::Ok | SweepItemResult::Pending)
        {
            continue;
        }
        if let Some(item) = store.item(&si.item_id).map_err(db)? {
            let reason = item.archive_reason.clone();
            out.push((item, reason));
        }
    }
    Ok(out)
}

/// The item is still archived from this sweep (not restored since, and not archived again
/// by a later sweep: Undo of this sweep wouldn't bring that one back).
fn still_from(item: &Item, sweep: &Sweep) -> bool {
    item.state == ItemState::Archived && item.last_sweep_id.as_deref() == Some(&sweep.id)
}

fn sweep_row(
    store: &Store,
    sweep: &Sweep,
    items: &[(Item, Option<Reason>)],
    undo_ids: &HashSet<String>,
) -> Res<SweepRow> {
    let count = |k: ItemKind| {
        u32::try_from(items.iter().filter(|(i, _)| i.kind == k).count()).unwrap_or(u32::MAX)
    };
    let archived = items.iter().filter(|(i, _)| still_from(i, sweep)).count();
    let restored = if sweep.is_shadow {
        0
    } else {
        items
            .iter()
            .filter(|(i, _)| i.state != ItemState::Archiving && !still_from(i, sweep))
            .count()
    };
    Ok(SweepRow {
        id: sweep.id.clone(),
        number: store.sweep_number(sweep).map_err(db)?,
        started_at: sweep.started_at,
        trigger: sweep.trigger,
        is_shadow: sweep.is_shadow,
        status: sweep.status,
        item_count: sweep.item_count,
        tabs: count(ItemKind::Tab),
        windows: count(ItemKind::Window),
        restored: u32::try_from(restored).unwrap_or(u32::MAX),
        failed: sweep.failed_count,
        undone_at: sweep.undone_at,
        undoable: !sweep.is_shadow && archived > 0 && undo_ids.contains(&sweep.id),
    })
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListSweeps {
    pub limit: Option<usize>,
    #[serde(default)]
    pub include_shadow: bool,
}

pub fn sweeps_for(
    store: &Store,
    input: ListSweeps,
    undo_ids: &HashSet<String>,
) -> Res<Vec<SweepRow>> {
    let limit = input.limit.unwrap_or(50).clamp(1, 200);
    let sweeps = store.recent_sweeps_of(limit, input.include_shadow).map_err(db)?;
    sweeps.iter().map(|s| sweep_row(store, s, &sweep_items(store, s)?, undo_ids)).collect()
}

#[tauri::command]
pub async fn list_sweeps(app: AppHandle, input: Option<ListSweeps>) -> Res<Vec<SweepRow>> {
    let input = input.unwrap_or_default();
    blocking(app, move |core| {
        core.ask(move |e, _| {
            let rows =
                e.undoable_sweep_ids().map_err(db).and_then(|u| sweeps_for(e.store(), input, &u));
            (rows, Vec::new())
        })?
    })
    .await
}

/// `get_sweep`: a sweep with its items.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SweepDetail {
    #[serde(flatten)]
    pub sweep: SweepRow,
    pub items: Vec<SweepEntry>,
}

pub fn sweep_detail(store: &Store, id: &str, undo_ids: &HashSet<String>) -> Res<SweepDetail> {
    let sweep = store
        .sweep(id)
        .map_err(db)?
        .ok_or_else(|| AppError::new("NOT_FOUND", t("error.sweepGone")))?;
    let items = sweep_items(store, &sweep)?;
    let row_info = sweep_row(store, &sweep, &items, undo_ids)?;
    let mut names = HashMap::new();
    let entries = items
        .into_iter()
        .map(|(item, reason)| {
            let state = item.state;
            let archived = still_from(&item, &sweep);
            let mut r = row(store, item, &mut names);
            r.reason = label(reason.as_ref());
            SweepEntry { row: r, state, archived }
        })
        .collect();
    Ok(SweepDetail { sweep: row_info, items: entries })
}

#[tauri::command]
pub async fn get_sweep(app: AppHandle, id: String) -> Res<SweepDetail> {
    if !valid_id(&id) {
        return Err(AppError::new("INVALID_INPUT", t("error.notSweep")));
    }
    blocking(app, move |core| {
        core.ask(move |e, _| {
            let d =
                e.undoable_sweep_ids().map_err(db).and_then(|u| sweep_detail(e.store(), &id, &u));
            (d, Vec::new())
        })?
    })
    .await
}

// ── Shadow review ───────────────────────────────────────────────────────────────────

/// An item shadow mode would have archived.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShadowItem {
    #[serde(flatten)]
    pub row: PanelItem,
    /// The first time it would have been archived.
    pub would_archive_at: i64,
    /// Used again after that: archiving it would have been a regret.
    pub used_after: bool,
    /// Still open (not closed or archived since).
    pub live: bool,
}

/// A live item a protection keeps.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeptItem {
    #[serde(flatten)]
    pub row: PanelItem,
    pub by: ProtectedBy,
    pub label: String,
    pub active_hours: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeptCount {
    pub by: ProtectedBy,
    pub label: String,
    pub count: u32,
}

/// `get_shadow_report`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShadowReport {
    pub mode: Mode,
    pub started_at: Option<i64>,
    pub day: u32,
    pub days: u32,
    pub tabs: u32,
    pub windows: u32,
    /// Would-be-archived items used again afterwards.
    pub reopened: u32,
    pub would_archive: Vec<ShadowItem>,
    pub kept: Vec<KeptItem>,
    /// Kept items by protection, most first.
    pub kept_by: Vec<KeptCount>,
}

pub fn shadow_report(
    store: &Store,
    mode: Mode,
    kept: Vec<(Item, ProtectedBy)>,
    now: i64,
    tz: &jiff::tz::TimeZone,
) -> Res<ShadowReport> {
    let started = store.setting::<i64>("shadow_started_at").map_err(db)?;
    // Per item: the first time it would have been archived, and the latest reason
    // (aggregated in SQL: every item, however many events).
    let first = store
        .first_events_since(EventType::ShadowWouldArchive, started.unwrap_or(0))
        .map_err(db)?;
    let mut names = HashMap::new();
    let mut would = Vec::new();
    for (id, at, reason) in first {
        let Some(item) = store.item(&id).map_err(db)? else { continue };
        let used_after = item.last_used_at > at;
        let live = matches!(item.state, ItemState::Active | ItemState::Stale | ItemState::Hidden);
        let mut r = row(store, item, &mut names);
        r.reason = label(reason.as_ref());
        would.push(ShadowItem { row: r, would_archive_at: at, used_after, live });
    }
    // Newest decisions first.
    would.sort_by(|a, b| b.would_archive_at.cmp(&a.would_archive_at).then(a.row.id.cmp(&b.row.id)));
    let count = |k: ItemKind| {
        u32::try_from(would.iter().filter(|w| w.row.kind == k).count()).unwrap_or(u32::MAX)
    };
    // In order of first appearance (then sorted by count, stable).
    let mut by: Vec<(ProtectedBy, u32)> = Vec::new();
    let kept: Vec<KeptItem> = kept
        .into_iter()
        .map(|(item, protected)| {
            match by.iter_mut().find(|(b, _)| *b == protected) {
                Some((_, n)) => *n += 1,
                None => by.push((protected, 1)),
            }
            let active_hours = item.active_age_s / 3600;
            KeptItem {
                row: row(store, item, &mut names),
                by: protected,
                label: protected_text(protected),
                active_hours,
            }
        })
        .collect();
    let mut kept_by: Vec<KeptCount> = by
        .into_iter()
        .map(|(by, count)| KeptCount { by, label: protected_text(by), count })
        .collect();
    kept_by.sort_by_key(|k| std::cmp::Reverse(k.count));
    Ok(ShadowReport {
        mode,
        started_at: started,
        day: shadow_day(started, now, tz),
        days: SHADOW_DAYS,
        tabs: count(ItemKind::Tab),
        windows: count(ItemKind::Window),
        reopened: u32::try_from(would.iter().filter(|w| w.used_after).count()).unwrap_or(u32::MAX),
        would_archive: would,
        kept,
        kept_by,
    })
}

#[tauri::command]
pub async fn get_shadow_report(app: AppHandle) -> Res<ShadowReport> {
    blocking(app, move |core| {
        core.ask(move |e, now| {
            let mode = e.status(now).mode;
            let kept = e.kept_live(now);
            let tz = jiff::tz::TimeZone::system();
            (shadow_report(e.store(), mode, kept, now, &tz), Vec::new())
        })?
    })
    .await
}

#[cfg(test)]
mod tests {
    use decay_core::{SweepItemResult, Thresholds};
    use decay_store::{DbKey, StorePaths, SweepItem};

    use super::*;

    const NOW: i64 = 1_793_750_400_000;
    const H: i64 = 3_600_000;
    const DAY_MS: i64 = 24 * H;

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let (s, _) =
            Store::open(StorePaths::in_dir(dir.path()), DbKey::from_bytes([7; 32]), NOW).unwrap();
        (dir, s)
    }

    fn item(store: &Store, id: &str, kind: ItemKind, state: ItemState) -> Item {
        let mut i = Item::new_live(id.into(), kind, "com.google.Chrome", NOW - 10 * H);
        i.title = format!("Item {id}");
        i.url = Some(format!("https://{id}.example/"));
        i.domain = Some(format!("{id}.example"));
        i.state = state;
        i.active_age_s = 61 * 3600;
        if state == ItemState::Archived {
            i.archived_at = Some(NOW);
            i.archive_reason = Some(Reason::Unused { active_hours: 61, threshold: 60.0 });
        }
        store.insert_item(&i).unwrap();
        i
    }

    fn sweep(store: &Store, id: &str, at: i64, shadow: bool) -> Sweep {
        let s = Sweep {
            id: id.into(),
            started_at: at,
            finished_at: Some(at),
            trigger: SweepTrigger::Scheduled,
            is_shadow: shadow,
            status: SweepStatus::Done,
            item_count: 0,
            failed_count: 0,
            reason: None,
            undone_at: None,
        };
        store.insert_sweep(&s).unwrap();
        s
    }

    /// `item` in `sweep` with `result`; an archived item is recorded as archived by it.
    fn in_sweep_as(store: &Store, sweep: &str, item: &Item, result: SweepItemResult) {
        store
            .put_sweep_item(&SweepItem {
                sweep_id: sweep.into(),
                item_id: item.id.clone(),
                action: SweepAction::Archive,
                prev_state: ItemState::Stale,
                result,
            })
            .unwrap();
        let mut i = store.item(&item.id).unwrap().unwrap();
        i.last_sweep_id = Some(sweep.into());
        store.update_item(&i).unwrap();
    }

    fn in_sweep(store: &Store, sweep: &str, item: &Item) {
        in_sweep_as(store, sweep, item, SweepItemResult::Ok);
    }

    fn tz() -> jiff::tz::TimeZone {
        jiff::tz::TimeZone::get("Europe/Paris").unwrap()
    }

    fn all_ids(store: &Store) -> HashSet<String> {
        store.recent_sweeps(1000).unwrap().into_iter().map(|s| s.id).collect()
    }

    #[test]
    fn shadow_days_are_calendar_days() {
        let at = |s: &str| s.parse::<jiff::Zoned>().unwrap().timestamp().as_millisecond();
        let start = at("2026-11-02T23:00:00+01:00[Europe/Paris]");
        // Day 2 starts at the next local midnight, not 24 h later.
        assert_eq!(shadow_day(Some(start), start, &tz()), 1);
        assert_eq!(
            shadow_day(Some(start), at("2026-11-03T09:00:00+01:00[Europe/Paris]"), &tz()),
            2
        );
        assert_eq!(
            shadow_day(Some(start), at("2026-11-08T23:59:00+01:00[Europe/Paris]"), &tz()),
            7
        );
        assert_eq!(
            shadow_day(Some(start), at("2026-11-09T00:01:00+01:00[Europe/Paris]"), &tz()),
            8
        );
        assert_eq!(shadow_day(Some(start), start - 5_000, &tz()), 1, "a clock moved back");
        assert_eq!(shadow_day(None, NOW, &tz()), 1);
    }

    #[test]
    fn stats_count_today_and_the_week() {
        let (_d, store) = store();
        item(&store, "a", ItemKind::Tab, ItemState::Archived);
        item(&store, "b", ItemKind::Window, ItemState::Archived);
        let r = item(&store, "c", ItemKind::Tab, ItemState::Active);
        store.append_event(&r.id, EventType::Restored, None, None, NOW + 1).unwrap();
        // Brought back by Undo counts as restored too.
        let u = item(&store, "u", ItemKind::Tab, ItemState::Active);
        store.append_event(&u.id, EventType::Undo, None, None, NOW + 1).unwrap();
        let input = StatsInput { today_start: NOW - 1, week_start: NOW - DAY_MS };
        let s = stats_for(&store, input, Mode::Shadow, 12, NOW + 2 * DAY_MS, &tz()).unwrap();
        assert_eq!(s.archived_total, 2);
        assert_eq!(s.archived_today, KindCount { tabs: 1, windows: 1 });
        assert_eq!(s.restored_week, 2);
        assert_eq!((s.open_tabs, s.shadow_day), (12, Some(3)));
        let auto = stats_for(&store, input, Mode::Automatic, 0, NOW, &tz()).unwrap();
        assert_eq!(auto.shadow_day, None);
    }

    #[test]
    fn item_details_have_a_timeline_and_their_sweep() {
        let (_d, store) = store();
        sweep(&store, "s1", NOW - H, false);
        let s2 = sweep(&store, "s2", NOW, false);
        let mut a = item(&store, "a", ItemKind::Tab, ItemState::Archived);
        a.last_sweep_id = Some(s2.id.clone());
        a.pinned = true;
        store.update_item(&a).unwrap();
        let reason = Reason::Unused { active_hours: 61, threshold: 60.0 };
        store.append_event(&a.id, EventType::Stale, None, None, NOW - 2 * H).unwrap();
        store.append_event(&a.id, EventType::Archived, Some("s2"), Some(&reason), NOW).unwrap();
        let d = item_detail(&store, "a").unwrap();
        assert_eq!(d.sweep_number, Some(2));
        assert_eq!((d.active_hours, d.pinned, d.state), (61, true, ItemState::Archived));
        assert_eq!(
            d.events.iter().map(|e| e.kind).collect::<Vec<_>>(),
            [EventType::Archived, EventType::Stale]
        );
        assert_eq!(d.events[0].label.as_deref(), Some("Unused 61 active h"));
        assert_eq!(d.row.reason.as_deref(), Some("Unused 61 active h"));
        assert_eq!(item_detail(&store, "nope").unwrap_err().code, "NOT_FOUND");
        let json = serde_json::to_value(&d).unwrap();
        assert_eq!(json["title"], "Item a");
        assert_eq!(json["events"][0]["type"], "archived");
    }

    #[test]
    fn only_archived_items_are_deleted() {
        let (_d, mut store) = store();
        item(&store, "a", ItemKind::Tab, ItemState::Archived);
        item(&store, "b", ItemKind::Tab, ItemState::Active);
        item(&store, "r", ItemKind::Tab, ItemState::Archived);
        let ids = ["a", "b", "r", "missing"].map(String::from);
        // "r" is being restored right now: its tab is reopening, so it stays.
        assert_eq!(delete_archived(&mut store, &ids, |id| id == "r").unwrap(), 1);
        assert!(store.item("a").unwrap().is_none());
        assert!(store.item("b").unwrap().is_some());
        assert!(store.item("r").unwrap().is_some());
    }

    #[test]
    #[allow(clippy::many_single_char_names, reason = "short item ids read best here")]
    fn sweeps_list_their_counts_restores_and_undo() {
        let (_d, store) = store();
        let real = sweep(&store, "real", NOW, false);
        let a = item(&store, "a", ItemKind::Tab, ItemState::Archived);
        let b = item(&store, "b", ItemKind::Tab, ItemState::Restored);
        let w = item(&store, "w", ItemKind::Window, ItemState::Archived);
        for i in [&a, &b, &w] {
            in_sweep(&store, &real.id, i);
        }
        let shadow = sweep(&store, "shadow", NOW + H, true);
        let s = item(&store, "s", ItemKind::Tab, ItemState::Stale);
        let reason = Reason::Unused { active_hours: 61, threshold: 60.0 };
        store
            .append_event(
                &s.id,
                EventType::ShadowWouldArchive,
                Some(&shadow.id),
                Some(&reason),
                NOW + H,
            )
            .unwrap();

        let rows = sweeps_for(&store, ListSweeps::default(), &all_ids(&store)).unwrap();
        assert_eq!(rows.len(), 1, "shadow sweeps only on request");
        let r = &rows[0];
        assert_eq!((r.number, r.tabs, r.windows, r.restored, r.undoable), (1, 2, 1, 1, true));

        let all =
            sweeps_for(&store, ListSweeps { limit: None, include_shadow: true }, &all_ids(&store))
                .unwrap();
        assert_eq!(all.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), ["shadow", "real"]);
        assert_eq!((all[0].tabs, all[0].undoable), (1, false));

        let d = sweep_detail(&store, "real", &all_ids(&store)).unwrap();
        assert_eq!(d.items.len(), 3);
        assert_eq!(d.items.iter().filter(|i| i.archived).count(), 2);
        let sd = sweep_detail(&store, "shadow", &all_ids(&store)).unwrap();
        assert_eq!(sd.items[0].row.reason.as_deref(), Some("Unused 61 active h"));
        assert_eq!(sweep_detail(&store, "nope", &all_ids(&store)).unwrap_err().code, "NOT_FOUND");

        // Everything restored: nothing left to undo.
        for i in [&a, &w] {
            store.set_item_state(&i.id, ItemState::Restored, NOW + 2 * H).unwrap();
        }
        let rows = sweeps_for(&store, ListSweeps::default(), &all_ids(&store)).unwrap();
        assert_eq!((rows[0].restored, rows[0].undoable), (3, false));
    }

    #[test]
    fn sweeps_count_only_what_they_archived_and_undo_follows_the_engine() {
        let (_d, store) = store();
        let mut s1 = sweep(&store, "s1", NOW, false);
        s1.item_count = 3;
        store.update_sweep(&s1).unwrap();
        let a = item(&store, "a", ItemKind::Tab, ItemState::Archived);
        let b = item(&store, "b", ItemKind::Tab, ItemState::Archived);
        // The browser kept "k" open (skipped): never archived, so not restored either.
        let k = item(&store, "k", ItemKind::Tab, ItemState::Active);
        in_sweep(&store, "s1", &a);
        in_sweep(&store, "s1", &b);
        in_sweep_as(&store, "s1", &k, SweepItemResult::Skipped);
        // b was restored, then archived again by a later sweep.
        sweep(&store, "s2", NOW + H, false);
        in_sweep(&store, "s2", &b);

        let ids = all_ids(&store);
        let rows = sweeps_for(&store, ListSweeps::default(), &ids).unwrap();
        let r1 = rows.iter().find(|r| r.id == "s1").unwrap();
        assert_eq!((r1.item_count, r1.tabs, r1.restored, r1.undoable), (3, 2, 1, true));
        let d = sweep_detail(&store, "s1", &ids).unwrap();
        let archived: Vec<&str> =
            d.items.iter().filter(|i| i.archived).map(|i| i.row.id.as_str()).collect();
        assert_eq!(archived, ["a"], "b now belongs to s2");
        // Outside the engine's undo history: no Undo, though something is archived.
        let none = HashSet::new();
        let r1 = sweeps_for(&store, ListSweeps::default(), &none).unwrap();
        assert!(r1.iter().all(|r| !r.undoable));
        // Deleting an item keeps the sweep's total.
        store.delete_item("a").unwrap();
        let rows = sweeps_for(&store, ListSweeps::default(), &ids).unwrap();
        assert_eq!(rows.iter().find(|r| r.id == "s1").unwrap().item_count, 3);
    }

    #[test]
    fn real_sweeps_are_found_past_many_shadow_ones() {
        let (_d, store) = store();
        sweep(&store, "real", NOW, false);
        for n in 0..300 {
            sweep(&store, &format!("sh{n}"), NOW + 1 + n, true);
        }
        let rows = sweeps_for(
            &store,
            ListSweeps { limit: Some(10), include_shadow: false },
            &HashSet::new(),
        )
        .unwrap();
        assert_eq!(rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), ["real"]);
    }

    #[test]
    fn the_shadow_report_counts_would_be_archives_regrets_and_protections() {
        let (_d, store) = store();
        store.set_setting("shadow_started_at", &(NOW - DAY_MS), NOW).unwrap();
        let s1 = sweep(&store, "s1", NOW, true);
        let s2 = sweep(&store, "s2", NOW + H, true);
        let a = item(&store, "a", ItemKind::Tab, ItemState::Stale);
        let mut b = item(&store, "b", ItemKind::Window, ItemState::Stale);
        let old = item(&store, "old", ItemKind::Tab, ItemState::Stale);
        let reason = Reason::Unused { active_hours: 61, threshold: 60.0 };
        let ev = EventType::ShadowWouldArchive;
        store.append_event(&a.id, ev, Some(&s1.id), Some(&reason), NOW).unwrap();
        store.append_event(&a.id, ev, Some(&s2.id), Some(&reason), NOW + H).unwrap();
        store.append_event(&b.id, ev, Some(&s2.id), Some(&reason), NOW + H).unwrap();
        // Before this shadow week: not counted.
        store.append_event(&old.id, ev, Some(&s1.id), None, NOW - 2 * DAY_MS).unwrap();
        // b was used again after it would have been archived.
        b.last_used_at = NOW + 2 * H;
        store.update_item(&b).unwrap();

        let mut pinned = item(&store, "p", ItemKind::Tab, ItemState::Stale);
        pinned.pinned = true;
        let audio = item(&store, "m", ItemKind::Tab, ItemState::Stale);
        let pinned2 = item(&store, "p2", ItemKind::Tab, ItemState::Stale);
        let kept = vec![
            (pinned, ProtectedBy::Pinned),
            (audio, ProtectedBy::Audible),
            (pinned2, ProtectedBy::Pinned),
        ];
        let r = shadow_report(&store, Mode::Shadow, kept, NOW + 2 * H, &tz()).unwrap();
        assert_eq!((r.day, r.days, r.tabs, r.windows, r.reopened), (2, 7, 1, 1, 1));
        assert_eq!(r.would_archive.len(), 2);
        let first_a = r.would_archive.iter().find(|w| w.row.id == "a").unwrap();
        assert_eq!(first_a.would_archive_at, NOW, "the first time counts");
        assert!(first_a.live && !first_a.used_after);
        assert_eq!(first_a.row.reason.as_deref(), Some("Unused 61 active h"));
        assert_eq!(r.kept.len(), 3);
        assert_eq!(
            r.kept_by[0],
            KeptCount { by: ProtectedBy::Pinned, label: "Pinned".into(), count: 2 }
        );
        assert_eq!(r.kept[1].label, "Playing audio");
        let json = serde_json::to_value(&r).unwrap();
        assert_eq!(json["keptBy"][1]["by"], "audible");
        let _ = Thresholds::BALANCED;
    }

    #[test]
    fn every_protection_has_words() {
        for by in [ProtectedBy::Pinned, ProtectedBy::Grace, ProtectedBy::Unrestorable] {
            assert_ne!(protected_text(by), "");
        }
        assert_eq!(
            label(Some(&Reason::Protected { by: ProtectedBy::Audible, rule_id: None })).as_deref(),
            Some("Protected: Playing audio")
        );
    }
}
