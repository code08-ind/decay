//! Items (DB Design §3.3) and the app catalog (§3.1).

use decay_core::{ItemKind, ItemState, Reason, SaveState, SqlEnum};
use rusqlite::{Connection, OptionalExtension, Row, params};

use crate::{Result, Store, StoreError};

/// Largest favicon kept (DB Design §3.3); bigger ones are dropped, not stored.
pub const MAX_FAVICON_BYTES: usize = 4096;

/// A tab or window, live or archived.
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub id: String,
    pub kind: ItemKind,
    pub app_id: String,
    pub source_id: Option<String>,
    /// Browser `tabId` / window id while live.
    pub external_id: Option<String>,
    pub title: String,
    pub url: Option<String>,
    /// eTLD+1, lowercase (private suffixes included).
    pub domain: Option<String>,
    /// Folder / document path (windows).
    pub path: Option<String>,
    pub favicon: Option<Vec<u8>>,
    /// Where the favicon can be fetched from (imported items have no image yet).
    pub favicon_url: Option<String>,
    pub tab_index: Option<i64>,
    pub group_title: Option<String>,
    pub group_color: Option<String>,
    pub pinned: bool,
    pub state: ItemState,
    pub save_state: SaveState,
    pub active_age_s: u64,
    pub first_seen_at: i64,
    pub last_used_at: i64,
    pub state_changed_at: i64,
    pub archived_at: Option<i64>,
    pub archive_reason: Option<Reason>,
    pub last_sweep_id: Option<String>,
    pub restored_from_id: Option<String>,
    pub grace_until: Option<i64>,
    pub restore_count: u32,
}

impl Item {
    /// A new live item seen now.
    pub fn new_live(id: String, kind: ItemKind, app_id: impl Into<String>, now: i64) -> Self {
        Self {
            id,
            kind,
            app_id: app_id.into(),
            source_id: None,
            external_id: None,
            title: String::new(),
            url: None,
            domain: None,
            path: None,
            favicon: None,
            favicon_url: None,
            tab_index: None,
            group_title: None,
            group_color: None,
            pinned: false,
            state: ItemState::Active,
            save_state: match kind {
                ItemKind::Tab => SaveState::NotApplicable,
                ItemKind::Window => SaveState::Unknown,
            },
            active_age_s: 0,
            first_seen_at: now,
            last_used_at: now,
            state_changed_at: now,
            archived_at: None,
            archive_reason: None,
            last_sweep_id: None,
            restored_from_id: None,
            grace_until: None,
            restore_count: 0,
        }
    }
}

pub(crate) const COLUMNS: &str =
    "id, kind, app_id, source_id, external_id, title, url, domain, path, favicon,
    tab_index, group_title, group_color, pinned, state, save_state, active_age_s, first_seen_at,
    last_used_at, state_changed_at, archived_at, archive_reason, last_sweep_id, restored_from_id,
    grace_until, restore_count, favicon_url";

/// Columns prefixed with `i.` (for joins).
pub(crate) fn columns_as(alias: &str) -> String {
    COLUMNS.split(',').map(|c| format!("{alias}.{}", c.trim())).collect::<Vec<_>>().join(", ")
}

pub(crate) fn sql_enum<T: SqlEnum>(value: &str) -> Result<T> {
    T::from_sql(value).ok_or_else(|| StoreError::InvalidData(format!("unknown value {value:?}")))
}

fn json<T: serde::de::DeserializeOwned>(value: Option<String>) -> Result<Option<T>> {
    value
        .map(|v| serde_json::from_str(&v).map_err(|e| StoreError::InvalidData(e.to_string())))
        .transpose()
}

/// Reads an item from a row whose first columns are [`COLUMNS`].
pub(crate) fn from_row(row: &Row<'_>) -> Result<Item> {
    Ok(Item {
        id: row.get(0)?,
        kind: sql_enum(&row.get::<_, String>(1)?)?,
        app_id: row.get(2)?,
        source_id: row.get(3)?,
        external_id: row.get(4)?,
        title: row.get(5)?,
        url: row.get(6)?,
        domain: row.get(7)?,
        path: row.get(8)?,
        favicon: row.get(9)?,
        tab_index: row.get(10)?,
        group_title: row.get(11)?,
        group_color: row.get(12)?,
        pinned: row.get(13)?,
        state: sql_enum(&row.get::<_, String>(14)?)?,
        save_state: sql_enum(&row.get::<_, String>(15)?)?,
        active_age_s: row.get(16)?,
        first_seen_at: row.get(17)?,
        last_used_at: row.get(18)?,
        state_changed_at: row.get(19)?,
        archived_at: row.get(20)?,
        archive_reason: json(row.get(21)?)?,
        last_sweep_id: row.get(22)?,
        restored_from_id: row.get(23)?,
        grace_until: row.get(24)?,
        restore_count: row.get(25)?,
        favicon_url: row.get(26)?,
    })
}

pub(crate) fn query_items(
    conn: &Connection,
    sql: &str,
    args: impl rusqlite::Params,
) -> Result<Vec<Item>> {
    let mut stmt = conn.prepare_cached(sql)?;
    let mut rows = stmt.query(args)?;
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        out.push(from_row(row)?);
    }
    Ok(out)
}

fn reason_json(reason: Option<&Reason>) -> Result<Option<String>> {
    reason
        .map(|r| serde_json::to_string(r).map_err(|e| StoreError::InvalidData(e.to_string())))
        .transpose()
}

/// Writes every column of `item` (insert, or update of the row with its id).
fn write(conn: &Connection, item: &Item, insert: bool) -> Result<usize> {
    let favicon = item.favicon.as_deref().filter(|f| f.len() <= MAX_FAVICON_BYTES);
    let sql = if insert {
        format!(
            "INSERT INTO items ({COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10,
                 ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27)"
        )
    } else {
        "UPDATE items SET kind = ?2, app_id = ?3, source_id = ?4, external_id = ?5, title = ?6,
           url = ?7, domain = ?8, path = ?9, favicon = ?10, tab_index = ?11, group_title = ?12,
           group_color = ?13, pinned = ?14, state = ?15, save_state = ?16, active_age_s = ?17,
           first_seen_at = ?18, last_used_at = ?19, state_changed_at = ?20, archived_at = ?21,
           archive_reason = ?22, last_sweep_id = ?23, restored_from_id = ?24, grace_until = ?25,
           restore_count = ?26, favicon_url = ?27
         WHERE id = ?1"
            .to_owned()
    };
    Ok(conn.prepare_cached(&sql)?.execute(params![
        item.id,
        item.kind.as_sql(),
        item.app_id,
        item.source_id,
        item.external_id,
        item.title,
        item.url,
        item.domain,
        item.path,
        favicon,
        item.tab_index,
        item.group_title,
        item.group_color,
        item.pinned,
        item.state.as_sql(),
        item.save_state.as_sql(),
        item.active_age_s,
        item.first_seen_at,
        item.last_used_at,
        item.state_changed_at,
        item.archived_at,
        reason_json(item.archive_reason.as_ref())?,
        item.last_sweep_id,
        item.restored_from_id,
        item.grace_until,
        item.restore_count,
        item.favicon_url,
    ])?)
}

/// Inserts on any connection or transaction (bulk test data).
#[cfg(test)]
pub(crate) fn insert_in(conn: &Connection, item: &Item) -> Result<()> {
    write(conn, item, true)?;
    Ok(())
}

/// Where an app runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    MacOs,
    Windows,
}

/// How an app's windows are reopened (Product Spec restore table).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestoreKind {
    Url,
    Folder,
    Document,
    AppOnly,
    None,
}

/// A known application or browser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct App {
    pub id: String,
    pub name: String,
    pub platform: Platform,
    pub is_browser: bool,
    pub close_allowed: bool,
    pub restore_kind: RestoreKind,
    pub icon_png: Option<Vec<u8>>,
}

const RESTORE_KINDS: [(RestoreKind, &str); 5] = [
    (RestoreKind::Url, "url"),
    (RestoreKind::Folder, "folder"),
    (RestoreKind::Document, "document"),
    (RestoreKind::AppOnly, "app_only"),
    (RestoreKind::None, "none"),
];

impl Store {
    /// Adds a new item. Its `(app_id, external_id)` must be unique among live items.
    pub fn insert_item(&self, item: &Item) -> Result<()> {
        write(&self.conn, item, true)?;
        Ok(())
    }

    /// Saves every field of an existing item.
    pub fn update_item(&self, item: &Item) -> Result<()> {
        match write(&self.conn, item, false)? {
            0 => Err(StoreError::NotFound),
            _ => Ok(()),
        }
    }

    pub fn item(&self, id: &str) -> Result<Option<Item>> {
        let sql = format!("SELECT {COLUMNS} FROM items WHERE id = ?1");
        Ok(query_items(&self.conn, &sql, [id])?.pop())
    }

    /// The live item for a browser tab / window id.
    pub fn live_item_by_external(&self, app_id: &str, external_id: &str) -> Result<Option<Item>> {
        let sql = format!(
            "SELECT {COLUMNS} FROM items WHERE app_id = ?1 AND external_id = ?2
             AND state IN ('active', 'stale', 'hidden', 'archiving')"
        );
        Ok(query_items(&self.conn, &sql, [app_id, external_id])?.pop())
    }

    /// Every live item (open now).
    pub fn live_items(&self) -> Result<Vec<Item>> {
        let sql = format!(
            "SELECT {COLUMNS} FROM items WHERE state IN ('active', 'stale', 'hidden', 'archiving')
             ORDER BY seq"
        );
        query_items(&self.conn, &sql, [])
    }

    /// Items a sweep may act on (DB Design §4): live, out of grace, at least `min_age_s`
    /// old, oldest first.
    pub fn sweep_candidates(&self, now: i64, min_age_s: u64, limit: usize) -> Result<Vec<Item>> {
        let sql = format!(
            "SELECT {COLUMNS} FROM items
             WHERE state IN ('active', 'stale', 'hidden')
               AND (grace_until IS NULL OR grace_until < ?1)
               AND active_age_s >= ?2
             ORDER BY active_age_s DESC LIMIT ?3"
        );
        query_items(&self.conn, &sql, params![now, min_age_s, limit])
    }

    /// The archived items of a sweep from one source window, in tab order (batch restore).
    pub fn archived_batch(&self, sweep_id: &str, source_id: Option<&str>) -> Result<Vec<Item>> {
        let sql = format!(
            "SELECT {COLUMNS} FROM items
             WHERE last_sweep_id = ?1 AND source_id IS ?2 AND state = 'archived'
             ORDER BY tab_index"
        );
        query_items(&self.conn, &sql, params![sweep_id, source_id])
    }

    /// The items a sweep archived that are still archived (Undo), in tab order per window.
    pub fn archived_in_sweep(&self, sweep_id: &str) -> Result<Vec<Item>> {
        let sql = format!(
            "SELECT {COLUMNS} FROM items WHERE last_sweep_id = ?1 AND state = 'archived'
             ORDER BY source_id, tab_index"
        );
        query_items(&self.conn, &sql, [sweep_id])
    }

    /// Items still archived that were archived in `[from, to)`, newest first, at most
    /// `limit` (the daily summary's list).
    pub fn archived_between(&self, from: i64, to: i64, limit: usize) -> Result<Vec<Item>> {
        let sql = format!(
            "SELECT {COLUMNS} FROM items WHERE state = 'archived' AND archived_at >= ?1
             AND archived_at < ?2 ORDER BY archived_at DESC, id LIMIT ?3"
        );
        query_items(&self.conn, &sql, rusqlite::params![from, to, limit])
    }

    /// Deletes an item (and its events). Used for items the user closed themselves: Decay
    /// keeps no history of those (DB Design §5).
    pub fn delete_item(&self, id: &str) -> Result<bool> {
        Ok(self.conn.execute("DELETE FROM items WHERE id = ?1", [id])? > 0)
    }

    /// Retention purge: deletes archived and restored records archived before `cutoff`.
    /// Returns how many were deleted.
    pub fn purge_archived_before(&self, cutoff: i64) -> Result<usize> {
        Ok(self.conn.execute(
            "DELETE FROM items WHERE state IN ('archived', 'restored', 'purged')
               AND coalesce(archived_at, state_changed_at) < ?1",
            [cutoff],
        )?)
    }

    /// Deletes archived / restored records archived at or after `from` and before `to`
    /// (the free retention for what's archived since Pro ended, P4.2).
    pub fn purge_archived_between(&self, from: i64, to: i64) -> Result<usize> {
        Ok(self.conn.execute(
            "DELETE FROM items WHERE state IN ('archived', 'restored', 'purged')
               AND coalesce(archived_at, state_changed_at) >= ?1
               AND coalesce(archived_at, state_changed_at) < ?2",
            [from, to],
        )?)
    }

    /// Saves the aging clock's new ages in one transaction-friendly batch. Unlike
    /// [`Store::update_item`], this writes only `active_age_s`, so a tick never touches the
    /// search index. Returns how many items were found.
    pub fn set_active_ages(&self, ages: &[(&str, u64)]) -> Result<usize> {
        let mut stmt =
            self.conn.prepare_cached("UPDATE items SET active_age_s = ?2 WHERE id = ?1")?;
        let mut found = 0;
        for (id, age) in ages {
            found += stmt.execute(params![id, age])?;
        }
        Ok(found)
    }

    /// Moves an item to `state` at `now` (and records `archived_at` when archiving).
    pub fn set_item_state(&self, id: &str, state: ItemState, now: i64) -> Result<()> {
        let n = self.conn.execute(
            "UPDATE items SET state = ?2, state_changed_at = ?3,
               archived_at = CASE WHEN ?2 = 'archived' THEN ?3 ELSE archived_at END
             WHERE id = ?1",
            params![id, state.as_sql(), now],
        )?;
        if n == 0 { Err(StoreError::NotFound) } else { Ok(()) }
    }

    /// Archived items, and when the oldest was archived (`get_storage_info`).
    pub fn archive_stats(&self) -> Result<(u64, Option<i64>)> {
        Ok(self.conn.query_row(
            "SELECT count(*), min(archived_at) FROM items WHERE state = 'archived'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?)
    }

    /// Adds or replaces an app in the catalog.
    pub fn upsert_app(&self, app: &App) -> Result<()> {
        let restore = RESTORE_KINDS.iter().find(|(k, _)| *k == app.restore_kind).map(|(_, s)| *s);
        let icon = app.icon_png.as_deref().filter(|i| i.len() <= 16 * 1024);
        self.conn.execute(
            "INSERT INTO apps (id, name, platform, kind, close_allowed, restore_kind, icon_png)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT (id) DO UPDATE SET name = excluded.name, platform = excluded.platform,
               kind = excluded.kind, close_allowed = excluded.close_allowed,
               restore_kind = excluded.restore_kind,
               icon_png = coalesce(excluded.icon_png, apps.icon_png)",
            params![
                app.id,
                app.name,
                match app.platform {
                    Platform::MacOs => "macos",
                    Platform::Windows => "windows",
                },
                if app.is_browser { "browser" } else { "app" },
                app.close_allowed,
                restore,
                icon,
            ],
        )?;
        Ok(())
    }

    pub fn app(&self, id: &str) -> Result<Option<App>> {
        self.conn
            .query_row(
                "SELECT id, name, platform, kind, close_allowed, restore_kind, icon_png
                 FROM apps WHERE id = ?1",
                [id],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, bool>(4)?,
                        r.get::<_, String>(5)?,
                        r.get::<_, Option<Vec<u8>>>(6)?,
                    ))
                },
            )
            .optional()?
            .map(|(id, name, platform, kind, close_allowed, restore, icon_png)| {
                Ok(App {
                    id,
                    name,
                    platform: match platform.as_str() {
                        "macos" => Platform::MacOs,
                        "windows" => Platform::Windows,
                        other => {
                            return Err(StoreError::InvalidData(format!("platform {other:?}")));
                        }
                    },
                    is_browser: kind == "browser",
                    close_allowed,
                    restore_kind: RESTORE_KINDS
                        .iter()
                        .find(|(_, s)| *s == restore)
                        .map(|(k, _)| *k)
                        .ok_or_else(|| {
                            StoreError::InvalidData(format!("restore_kind {restore:?}"))
                        })?,
                    icon_png,
                })
            })
            .transpose()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::new_id;
    use crate::test_util::*;

    pub fn tab(store: &Store, external: &str) -> Item {
        let mut item = Item::new_live(new_id(NOW), ItemKind::Tab, "com.google.Chrome", NOW);
        item.external_id = Some(external.into());
        item.title = format!("Tab {external}");
        item.url = Some(format!("https://example.com/{external}"));
        item.domain = Some("example.com".into());
        store.insert_item(&item).unwrap();
        item
    }

    #[test]
    fn items_round_trip_every_field() {
        let (store, _dir) = store();
        let mut item = tab(&store, "1");
        item.state = ItemState::Archived;
        item.archived_at = Some(NOW + 5);
        item.archive_reason = Some(Reason::Unused { active_hours: 61, threshold: 60.0 });
        item.favicon = Some(vec![1, 2, 3]);
        item.group_title = Some("Research".into());
        item.pinned = true;
        item.active_age_s = 220_000;
        item.restore_count = 2;
        store.update_item(&item).unwrap();
        assert_eq!(store.item(&item.id).unwrap(), Some(item));
    }

    #[test]
    fn oversized_favicons_are_dropped() {
        let (store, _dir) = store();
        let mut item = tab(&store, "1");
        item.favicon = Some(vec![0; MAX_FAVICON_BYTES + 1]);
        store.update_item(&item).unwrap();
        assert_eq!(store.item(&item.id).unwrap().unwrap().favicon, None);
    }

    #[test]
    fn external_ids_are_unique_among_live_items_only() {
        let (store, _dir) = store();
        let mut first = tab(&store, "7");
        let dup = Item::new_live(new_id(NOW + 1), ItemKind::Tab, "com.google.Chrome", NOW);
        let dup = Item { external_id: Some("7".into()), ..dup };
        assert!(store.insert_item(&dup).is_err());
        first.state = ItemState::Archived;
        store.update_item(&first).unwrap();
        store.insert_item(&dup).unwrap(); // the tab id was reused by the browser
        assert_eq!(store.live_item_by_external("com.google.Chrome", "7").unwrap(), Some(dup));
    }

    #[test]
    fn finds_sweep_candidates_and_purges_old_archives() {
        let (store, _dir) = store();
        let mut old = tab(&store, "1");
        old.active_age_s = 300_000;
        store.update_item(&old).unwrap();
        let mut graced = tab(&store, "2");
        graced.active_age_s = 400_000;
        graced.grace_until = Some(NOW + 1);
        store.update_item(&graced).unwrap();
        tab(&store, "3"); // too young
        let ids: Vec<String> =
            store.sweep_candidates(NOW, 200_000, 50).unwrap().into_iter().map(|i| i.id).collect();
        assert_eq!(ids, vec![old.id.clone()]);

        old.state = ItemState::Archived;
        old.archived_at = Some(NOW - 1);
        store.update_item(&old).unwrap();
        assert_eq!(store.purge_archived_before(NOW).unwrap(), 1);
        assert_eq!(store.item(&old.id).unwrap(), None);
        assert_eq!(store.live_items().unwrap().len(), 2);
    }

    #[test]
    fn routine_updates_leave_the_search_index_alone() {
        let (store, _dir) = store();
        let item = tab(&store, "1");
        let fts_changes = |store: &Store| -> i64 {
            store.conn.query_row("SELECT total_changes()", [], |r| r.get(0)).unwrap()
        };
        // update_item with unchanged text: only the items row changes (1), no FTS delete
        // and insert.
        let before = fts_changes(&store);
        store.update_item(&Item { active_age_s: 99, ..item.clone() }).unwrap();
        assert_eq!(fts_changes(&store) - before, 1);
        assert_eq!(store.set_active_ages(&[(&item.id, 120), ("missing", 1)]).unwrap(), 1);
        store.set_item_state(&item.id, ItemState::Archived, NOW + 9).unwrap();
        let stored = store.item(&item.id).unwrap().unwrap();
        assert_eq!((stored.active_age_s, stored.archived_at), (120, Some(NOW + 9)));
        assert_eq!(store.archive_stats().unwrap(), (1, Some(NOW + 9)));
    }

    #[test]
    fn renaming_an_app_reindexes_its_items() {
        let (store, _dir) = store();
        let mut item = tab(&store, "1");
        item.state = ItemState::Archived;
        item.archived_at = Some(NOW);
        store.update_item(&item).unwrap();
        let mut chrome = store.app("com.google.Chrome").unwrap().unwrap();
        chrome.name = "Chromium Nightly".into();
        store.upsert_app(&chrome).unwrap();
        let q = |text: &str| crate::SearchQuery {
            text: text.into(),
            filters: crate::SearchFilters::default(),
            limit: 0,
            offset: 0,
            now: NOW,
            utc_offset_minutes: 0,
        };
        assert_eq!(store.search(&q("nightly")).unwrap().hits.len(), 1);
        assert_eq!(store.search(&q("google")).unwrap().hits.len(), 0);
    }

    #[test]
    fn updates_and_reads_apps() {
        let (store, _dir) = store();
        let finder = store.app("com.apple.finder").unwrap().unwrap();
        assert!(finder.close_allowed);
        assert_eq!(finder.restore_kind, RestoreKind::Folder);
        let app = App {
            id: "com.figma.Desktop".into(),
            name: "Figma".into(),
            platform: Platform::MacOs,
            is_browser: false,
            close_allowed: false,
            restore_kind: RestoreKind::AppOnly,
            icon_png: Some(vec![9]),
        };
        store.upsert_app(&app).unwrap();
        store.upsert_app(&App { icon_png: None, ..app.clone() }).unwrap(); // keeps the icon
        assert_eq!(store.app("com.figma.Desktop").unwrap(), Some(app));
    }
}
