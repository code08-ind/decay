//! Tab tracking (Software Design §4.6–4.7): the extension's snapshot and events become live
//! items, like the Phase 0 tracker (`lib/tracker/reconcile.ts`, `tracker.ts`).
//!
//! - A snapshot is matched to the stored live items by tab id and URL, then by URL alone
//!   (browsers assign new tab ids after a restart and session restore reopens the same
//!   URLs, so ages carry over). Among several candidates the most recently used one wins.
//! - An item whose tab is gone (closed by the user, or not restored yet) is *parked*: its
//!   tab id is freed (`<profile>:~<item>`) so a new tab can take it, and it's deleted after
//!   a minute and a fresh snapshot, unless a tab with its URL comes back first. Decay keeps
//!   no history of tabs the user closed (DB Design §5).
//! - An item left `archiving` (a crash, or a close that was never answered) is settled
//!   against the snapshot: its tab is still open → back to stale; gone → archived.
//! - Excluded tabs (F9) are never stored.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use decay_bridge::protocol::{
    Snapshot, TabInfo, TabMoved, TabRemoved, TabStatus, TabUpdated, TabUsed, UseSignal,
    WindowFocus, WindowInfo, WindowState,
};
use decay_core::policy::is_excluded;
use decay_core::{EventType, ItemKind, ItemState, MatchTarget, SweepItemResult};
use decay_store::{Item, Source, Store, new_id};
use serde_json::Value;

use crate::domain::domain_of;
use crate::engine::{Adapter, ConnId, Engine, Pending, PolicySettings, Res, timing};
use crate::sweep::{SettleOutcome, commit, finish_sweep_if_settled, record_result, settle};

/// An open tab and the item backing it (`None`: excluded, or waiting for its title).
#[derive(Debug, Clone)]
pub(crate) struct Tab {
    pub info: TabInfo,
    pub item: Option<Item>,
}

/// Records the tab's window as a source (items reference it: batch restore).
fn ensure_source(s: &Store, a: &mut Adapter, window_id: i64, now: i64) -> Res<String> {
    let id = a.source_id(window_id);
    if !a.sources.contains(&id) {
        s.upsert_source(&Source {
            id: id.clone(),
            app_id: a.app_id.clone(),
            profile: Some(a.profile.clone()),
            external_id: Some(a.window_key(window_id)),
            title: None,
            first_seen_at: now,
            last_seen_at: now,
            closed_at: None,
        })?;
        a.sources.insert(id.clone());
    }
    Ok(id)
}

/// Writes the tab's current fields onto its item. Returns whether anything changed.
fn apply_fields(a: &Adapter, item: &mut Item, info: &TabInfo, source_id: &str) -> bool {
    let url = (!info.url.is_empty()).then(|| info.url.clone());
    let domain = url.as_deref().and_then(domain_of);
    let group = (info.group_id >= 0).then(|| a.groups.get(&info.group_id)).flatten();
    let group_title = group.map(|g| g.title.clone()).filter(|t| !t.is_empty());
    let group_color = group.map(|g| g.color.clone());
    let favicon = info.fav_icon_url.clone().filter(|f| !f.is_empty());
    // A tab that's still loading has no title yet: keep the one we know.
    let title = if info.title.is_empty() && !item.title.is_empty() {
        item.title.clone()
    } else {
        info.title.clone()
    };
    let ext = Some(a.ext_id(info.tab_id));
    let before = (
        item.external_id.clone(),
        item.source_id.clone(),
        item.title.clone(),
        item.url.clone(),
        item.domain.clone(),
        item.favicon_url.clone(),
        item.tab_index,
        item.pinned,
        item.group_title.clone(),
        item.group_color.clone(),
    );
    item.external_id = ext;
    item.source_id = Some(source_id.to_owned());
    item.title = title;
    item.url = url;
    item.domain = domain;
    if favicon.is_some() {
        item.favicon_url = favicon;
    }
    item.tab_index = Some(info.index);
    item.pinned = info.pinned;
    item.group_title = group_title;
    item.group_color = group_color;
    before
        != (
            item.external_id.clone(),
            item.source_id.clone(),
            item.title.clone(),
            item.url.clone(),
            item.domain.clone(),
            item.favicon_url.clone(),
            item.tab_index,
            item.pinned,
            item.group_title.clone(),
            item.group_color.clone(),
        )
}

/// Marks an item used: age 0, stale → active, and a sampled `used` event.
pub(crate) fn mark_used(s: &Store, item: &mut Item, now: i64) -> Res {
    item.active_age_s = 0;
    item.last_used_at = now;
    if matches!(item.state, ItemState::Stale | ItemState::Hidden) {
        item.state = ItemState::Active;
        item.state_changed_at = now;
    }
    s.update_item(item)?;
    s.record_used(&item.id, now)?;
    Ok(())
}

/// The reopened tab of an archived item: linked to it, young, and protected for 48 h.
pub(crate) fn link_restored(s: &Store, item: &mut Item, archived_id: &str, now: i64) -> Res {
    let old = s.item(archived_id)?;
    item.restored_from_id = old.as_ref().map(|o| o.id.clone());
    item.grace_until = Some(now + timing::RESTORE_GRACE_MS);
    item.active_age_s = 0;
    item.last_used_at = now;
    if item.state != ItemState::Archiving {
        item.state = ItemState::Active;
    }
    if let Some(old) = old {
        // The tab may still be loading: keep the archived title and group until it updates.
        if item.title.is_empty() {
            item.title = old.title;
        }
        if item.group_title.is_none() {
            item.group_title = old.group_title;
            item.group_color = old.group_color;
        }
        if item.favicon_url.is_none() {
            item.favicon_url = old.favicon_url;
        }
    }
    s.update_item(item)?;
    Ok(())
}

/// A parked item (tab gone) with this URL, most recently used first.
fn reclaim(s: &Store, a: &mut Adapter, url: &str) -> Res<Option<Item>> {
    let mut best: Option<Item> = None;
    for id in a.missing.keys() {
        let Some(item) = s.item(id)? else { continue };
        let usable = item.state.is_live()
            && item.state != ItemState::Archiving
            && item.url.as_deref().unwrap_or("") == url;
        if usable && best.as_ref().is_none_or(|b| item.last_used_at > b.last_used_at) {
            best = Some(item);
        }
    }
    if let Some(b) = &best {
        a.missing.remove(&b.id);
    }
    Ok(best)
}

/// Brings a tab's item in line with the tab: creates or reclaims it, updates its fields,
/// or forgets it when the tab is excluded.
pub(crate) fn sync_tab(
    s: &Store,
    policy: &PolicySettings,
    a: &mut Adapter,
    tab_id: i64,
    now: i64,
) -> Res {
    let Some(tab) = a.tabs.get(&tab_id) else { return Ok(()) };
    let info = tab.info.clone();
    let url = (!info.url.is_empty()).then_some(info.url.as_str());
    let target = MatchTarget { app_id: &a.app_id, url, title: &info.title };
    let archiving = tab.item.as_ref().is_some_and(|i| i.state == ItemState::Archiving);
    if archiving {
        // The sweep executor owns it: its close re-checks the tab itself.
        return Ok(());
    }
    if is_excluded(&target, &policy.rules) {
        // e.g. navigated to an excluded site: forget the record (F9).
        if let Some(item) = a.tabs.get_mut(&tab_id).and_then(|t| t.item.take()) {
            s.delete_item(&item.id)?;
        }
        return Ok(());
    }
    let source_id = ensure_source(s, a, info.window_id, now)?;
    let existing = a.tabs.get_mut(&tab_id).and_then(|t| t.item.take());
    let mut item = if let Some(mut item) = existing {
        if apply_fields(a, &mut item, &info, &source_id) {
            s.update_item(&item)?;
        }
        item
    } else {
        // A loading tab has no real title yet: with title exclusions, wait for it.
        if info.status == TabStatus::Loading && policy.has_title_exclusions() {
            return Ok(());
        }
        if let Some(mut item) = reclaim(s, a, &info.url)? {
            apply_fields(a, &mut item, &info, &source_id);
            s.update_item(&item)?;
            item
        } else {
            let mut item = Item::new_live(new_id(now), ItemKind::Tab, a.app_id.clone(), now);
            apply_fields(a, &mut item, &info, &source_id);
            s.insert_item(&item)?;
            s.append_event(&item.id, EventType::Seen, None, None, now)?;
            item
        }
    };
    if let Some(archived_id) = a.restore_links.remove(&tab_id) {
        link_restored(s, &mut item, &archived_id, now)?;
    }
    if let Some(tab) = a.tabs.get_mut(&tab_id) {
        tab.item = Some(item);
    }
    Ok(())
}

/// Matches a snapshot to the adapter's stored live items (see the module docs). Returns
/// the sweeps whose items were settled.
#[allow(clippy::too_many_lines, reason = "the matching passes read best in one place")]
fn reconcile(
    s: &Store,
    policy: &PolicySettings,
    a: &mut Adapter,
    snap: Snapshot,
    in_flight: &HashSet<String>,
    now: i64,
) -> Res<Vec<String>> {
    a.windows.clear();
    a.groups.clear();
    let mut open: Vec<TabInfo> = Vec::new();
    for mut w in snap.windows {
        if w.incognito {
            continue;
        }
        open.append(&mut w.tabs);
        a.windows.insert(w.id, w);
    }
    for g in snap.groups {
        a.groups.insert(g.id, g);
    }
    let mut seen = HashSet::new();
    open.retain(|t| seen.insert(t.tab_id));

    let mut pool: HashMap<String, Item> =
        s.live_items()?.into_iter().filter(|i| a.owns(i)).map(|i| (i.id.clone(), i)).collect();
    let by_ext: HashMap<String, String> =
        pool.values().filter_map(|i| Some((i.external_id.clone()?, i.id.clone()))).collect();
    let mut matched: HashMap<i64, Item> = HashMap::new();

    // Pass 1: same tab id and same URL (an item being closed keeps its tab regardless).
    for t in &open {
        let Some(item_id) = by_ext.get(&a.ext_id(t.tab_id)) else { continue };
        let same = pool
            .get(item_id)
            .is_some_and(|i| in_flight.contains(&i.id) || i.url.as_deref().unwrap_or("") == t.url);
        if same && let Some(item) = pool.remove(item_id) {
            matched.insert(t.tab_id, item);
        }
    }
    // Pass 2: same URL, most recently used first (never an item whose close is running).
    let mut by_url: HashMap<String, Vec<(i64, String)>> = HashMap::new();
    for i in pool.values().filter(|i| !in_flight.contains(&i.id)) {
        by_url
            .entry(i.url.clone().unwrap_or_default())
            .or_default()
            .push((i.last_used_at, i.id.clone()));
    }
    for list in by_url.values_mut() {
        list.sort_by(|x, y| y.0.cmp(&x.0).then_with(|| x.1.cmp(&y.1)));
        list.reverse(); // pop() takes the most recent
    }
    for t in &open {
        if matched.contains_key(&t.tab_id) {
            continue;
        }
        let Some(list) = by_url.get_mut(&t.url) else { continue };
        while let Some((_, id)) = list.pop() {
            if let Some(item) = pool.remove(&id) {
                matched.insert(t.tab_id, item);
                break;
            }
        }
    }

    let mut settled = Vec::new();
    // Items whose tab is gone.
    let mut parked_until: Option<i64> = None;
    for (_, mut item) in pool.drain() {
        if in_flight.contains(&item.id) {
            continue; // the close's ack settles it
        }
        if item.state == ItemState::Archiving {
            if let SettleOutcome::Settled(Some(sweep)) = settle(s, &mut item, false, now)? {
                settled.push(sweep);
            }
            continue;
        }
        let since = a.missing.get(&item.id).copied().unwrap_or(now);
        if now - since >= timing::MISSING_GRACE_MS {
            s.delete_item(&item.id)?;
            a.missing.remove(&item.id);
            continue;
        }
        a.missing.insert(item.id.clone(), since);
        let due = since + timing::MISSING_GRACE_MS;
        parked_until = Some(parked_until.map_or(due, |t: i64| t.min(due)));
        let parked = a.parked_ext_id(&item.id);
        if item.external_id.as_deref() != Some(parked.as_str()) {
            item.external_id = Some(parked);
            s.update_item(&item)?;
        }
    }
    if let Some(due) = parked_until {
        a.resync_at = Some(a.resync_at.map_or(due, |t| t.min(due)));
    }
    // Matched items that change tab id are parked first: two items may swap ids, and an
    // id is unique among live items.
    for (tab_id, item) in &mut matched {
        if item.external_id.as_deref() != Some(a.ext_id(*tab_id).as_str()) {
            item.external_id = Some(a.parked_ext_id(&item.id));
            s.update_item(item)?;
        }
        a.missing.remove(&item.id);
    }

    let previous = std::mem::take(&mut a.tabs);
    let mut ids = Vec::with_capacity(open.len());
    for info in open {
        let item = matched.remove(&info.tab_id);
        ids.push(info.tab_id);
        a.tabs.insert(info.tab_id, Tab { info, item });
    }
    drop(previous);
    for tab_id in ids {
        // An item left archiving whose tab is still open: the close never happened.
        let stuck = a
            .tabs
            .get(&tab_id)
            .and_then(|t| t.item.as_ref())
            .is_some_and(|i| i.state == ItemState::Archiving && !in_flight.contains(&i.id));
        if stuck
            && let Some(item) = a.tabs.get_mut(&tab_id).and_then(|t| t.item.as_mut())
            && let SettleOutcome::Settled(Some(sweep)) = settle(s, item, true, now)?
        {
            settled.push(sweep);
        }
        sync_tab(s, policy, a, tab_id, now)?;
    }
    let tabs = &a.tabs;
    a.probes.retain(|id, _| tabs.contains_key(id));
    a.used_while_closing.retain(|id| tabs.contains_key(id));
    a.ready = true;
    a.last_snapshot_at = now;
    settled.sort();
    settled.dedup();
    Ok(settled)
}

impl Engine {
    /// Item ids in `tabs.close` commands still waiting for their ack on this connection.
    fn closing_items(&self, conn: ConnId) -> HashSet<String> {
        self.pending
            .values()
            .filter(|p| p.conn == conn)
            .filter_map(|p| match &p.what {
                Pending::Close { tabs, .. } => Some(tabs.iter().map(|(_, id)| id.clone())),
                _ => None,
            })
            .flatten()
            .collect()
    }

    pub(crate) fn on_snapshot(&mut self, conn: ConnId, snap: Snapshot, now: i64) -> Res {
        let in_flight = self.closing_items(conn);
        let policy = Arc::clone(&self.policy);
        let Some(a) = self.conns.get_mut(&conn).and_then(|c| c.adapter.as_mut()) else {
            return Ok(());
        };
        let settled =
            self.store.atomically(false, |s| reconcile(s, &policy, a, snap, &in_flight, now));
        if settled.is_err() {
            self.recover_cache(conn, now);
        }
        let settled = settled?;
        for sweep in settled {
            if let Some(event) = finish_sweep_if_settled(&self.store, &sweep, now)? {
                self.out.push(crate::Output::Event(event));
            }
        }
        Ok(())
    }

    fn ready_adapter(&mut self, conn: ConnId) -> Option<&mut Adapter> {
        self.conns.get_mut(&conn).and_then(|c| c.adapter.as_mut()).filter(|a| a.ready)
    }

    pub(crate) fn on_tab_created(&mut self, conn: ConnId, info: TabInfo, now: i64) -> Res {
        let policy = Arc::clone(&self.policy);
        let Some(a) =
            self.conns.get_mut(&conn).and_then(|c| c.adapter.as_mut()).filter(|a| a.ready)
        else {
            return Ok(());
        };
        a.windows.entry(info.window_id).or_insert_with(|| WindowInfo {
            id: info.window_id,
            focused: false,
            state: WindowState::Normal,
            incognito: false,
            tabs: Vec::new(),
        });
        let tab_id = info.tab_id;
        match a.tabs.get_mut(&tab_id) {
            Some(tab) => tab.info = info,
            None => {
                a.tabs.insert(tab_id, Tab { info, item: None });
            }
        }
        sync_tab(&self.store, &policy, a, tab_id, now)
    }

    pub(crate) fn on_tab_updated(&mut self, conn: ConnId, update: &TabUpdated, now: i64) -> Res {
        let policy = Arc::clone(&self.policy);
        let Some(a) =
            self.conns.get_mut(&conn).and_then(|c| c.adapter.as_mut()).filter(|a| a.ready)
        else {
            return Ok(());
        };
        let Some(tab) = a.tabs.get_mut(&update.tab_id) else { return Ok(()) };
        let info = &mut tab.info;
        let mut navigated = false;
        for (key, value) in &update.changes {
            match (key.as_str(), value) {
                ("url", Value::String(url)) => {
                    navigated |= *url != info.url;
                    info.url.clone_from(url);
                }
                ("title", Value::String(title)) => info.title.clone_from(title),
                ("favIconUrl", Value::String(f)) => info.fav_icon_url = Some(f.clone()),
                ("pinned", Value::Bool(b)) => info.pinned = *b,
                ("audible", Value::Bool(b)) => info.audible = *b,
                ("discarded", Value::Bool(b)) => info.discarded = *b,
                ("groupId", v) => info.group_id = v.as_i64().unwrap_or(-1),
                ("status", v) => {
                    info.status = serde_json::from_value(v.clone()).unwrap_or(TabStatus::Unknown);
                }
                _ => {}
            }
        }
        if navigated {
            // A verdict is about one page.
            a.probes.remove(&update.tab_id);
        }
        sync_tab(&self.store, &policy, a, update.tab_id, now)
    }

    pub(crate) fn on_tab_removed(&mut self, conn: ConnId, removed: TabRemoved, now: i64) -> Res {
        let Some(a) = self.ready_adapter(conn) else { return Ok(()) };
        let Some(tab) = a.tabs.remove(&removed.tab_id) else { return Ok(()) };
        a.probes.remove(&removed.tab_id);
        let Some(mut item) = tab.item else { return Ok(()) };
        if item.state == ItemState::Archiving {
            if removed.by_user {
                // Closed by the user while the sweep was closing it: theirs, not archived.
                self.store.delete_item(&item.id)?;
            }
            // Closed by Decay: the close's ack (or the next snapshot) settles it.
            return Ok(());
        }
        if !removed.by_user {
            // Decay closed it after all (a close that was late, or ran on across a
            // reconnect, after the item went back to stale): it must not be forgotten.
            let sweep = item.last_sweep_id.clone();
            self.store.atomically(false, |s| {
                commit(s, &mut item, sweep.as_deref(), now)?;
                if let Some(id) = &sweep
                    && s.sweep(id)?.is_some()
                {
                    record_result(s, &item, id, SweepItemResult::Ok)?;
                }
                Ok(())
            })?;
            if let Some(id) = &sweep
                && let Some(event) = finish_sweep_if_settled(&self.store, id, now)?
            {
                self.out.push(crate::Output::Event(event));
            }
            return Ok(());
        }
        // Gone: parked for a minute (a reopened window reclaims it), then forgotten.
        let parked = a.parked_ext_id(&item.id);
        a.missing.insert(item.id.clone(), now);
        let due = now + timing::MISSING_GRACE_MS;
        a.resync_at = Some(a.resync_at.map_or(due, |t| t.min(due)));
        item.external_id = Some(parked);
        self.store.update_item(&item)
    }

    pub(crate) fn on_tab_moved(&mut self, conn: ConnId, moved: TabMoved, now: i64) -> Res {
        let policy = Arc::clone(&self.policy);
        let Some(a) =
            self.conns.get_mut(&conn).and_then(|c| c.adapter.as_mut()).filter(|a| a.ready)
        else {
            return Ok(());
        };
        a.windows.entry(moved.window_id).or_insert_with(|| WindowInfo {
            id: moved.window_id,
            focused: false,
            state: WindowState::Normal,
            incognito: false,
            tabs: Vec::new(),
        });
        let Some(tab) = a.tabs.get_mut(&moved.tab_id) else { return Ok(()) };
        tab.info.window_id = moved.window_id;
        tab.info.index = moved.index;
        sync_tab(&self.store, &policy, a, moved.tab_id, now)
    }

    pub(crate) fn on_tab_used(&mut self, conn: ConnId, used: &TabUsed, now: i64) -> Res {
        let deliberate = matches!(used.signal, UseSignal::Activated | UseSignal::Typed);
        if deliberate {
            self.last_activity_at = Some(now);
        }
        let Some(a) =
            self.conns.get_mut(&conn).and_then(|c| c.adapter.as_mut()).filter(|a| a.ready)
        else {
            return Ok(());
        };
        if !a.tabs.contains_key(&used.tab_id) {
            return Ok(());
        }
        match used.signal {
            UseSignal::Activated => {
                for tab in a.tabs.values_mut().filter(|t| t.info.window_id == used.window_id) {
                    tab.info.active = tab.info.tab_id == used.tab_id;
                }
            }
            UseSignal::Audible => {
                if let Some(tab) = a.tabs.get_mut(&used.tab_id) {
                    tab.info.audible = true;
                }
            }
            _ => {}
        }
        if deliberate {
            a.probes.remove(&used.tab_id);
        }
        let Some(item) = a.tabs.get_mut(&used.tab_id).and_then(|t| t.item.as_mut()) else {
            return Ok(());
        };
        if item.state == ItemState::Archiving {
            // Being closed: the extension skips it (used within the last minute), and its
            // age is reset once the close is settled.
            a.used_while_closing.insert(used.tab_id);
            return Ok(());
        }
        mark_used(&self.store, item, now)
    }

    pub(crate) fn on_window_focus(&mut self, conn: ConnId, focus: WindowFocus, now: i64) -> Res {
        if focus.window_id < 0 {
            return Ok(()); // the browser lost focus
        }
        self.last_activity_at = Some(now);
        let Some(a) =
            self.conns.get_mut(&conn).and_then(|c| c.adapter.as_mut()).filter(|a| a.ready)
        else {
            return Ok(());
        };
        for w in a.windows.values_mut() {
            w.focused = w.id == focus.window_id;
            if w.focused && w.state == WindowState::Minimized {
                w.state = WindowState::Normal;
            }
        }
        // The focused window's active tab is in use.
        let Some(item) = a
            .tabs
            .values_mut()
            .find(|t| t.info.window_id == focus.window_id && t.info.active)
            .and_then(|t| t.item.as_mut())
        else {
            return Ok(());
        };
        if item.state == ItemState::Archiving {
            return Ok(());
        }
        mark_used(&self.store, item, now)
    }
}
