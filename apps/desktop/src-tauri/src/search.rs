//! The search panel's commands (P1.9, PRD F5/F6, API Design §2.1): search the archive,
//! restore one item, its batch, or into a new window, "always keep" its site, and open
//! its details.
//!
//! Search runs in the store (FTS5 prefix match, bm25 × recency, inline `app:` `site:`
//! `before:` `after:` operators); the panel groups the ranked results by local day and
//! source window itself (it knows the user's calendar and locale).

#![allow(clippy::needless_pass_by_value, reason = "Tauri commands take their arguments by value")]

use std::collections::HashMap;
use std::time::Instant;

use decay_core::{ItemKind, Reason};
use decay_engine::{Engine, ProfileInfo, RestoreRefused};
use decay_store::{Item, SearchFilters, SearchQuery, Store};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tauri::AppHandle;

use crate::commands::{Res, blocking};
use crate::core::AppError;
use crate::deeplink::Window;
use crate::i18n::{t, tf};
use crate::shell;

/// The most a page may hold (API Design: ≤ 200).
const MAX_LIMIT: usize = 200;

/// `search` input.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchInput {
    #[serde(default)]
    pub query: String,
    /// `tab` or `window`; none = all.
    pub kind: Option<ItemKind>,
    pub limit: Option<usize>,
    /// Only items archived at or after this (epoch ms; the Library's Today / This week).
    pub after: Option<i64>,
    /// The `nextCursor` of the previous page.
    pub cursor: Option<String>,
    /// Only this app (its id) and browser profile (its id): the Library's browser filter.
    pub app: Option<String>,
    pub profile: Option<String>,
}

/// One result row.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PanelItem {
    pub id: String,
    pub kind: ItemKind,
    pub title: String,
    pub url: Option<String>,
    pub domain: Option<String>,
    /// The app it came from ("Google Chrome").
    pub app_name: String,
    /// The window it came from (its title, when known).
    pub source_id: Option<String>,
    pub source_title: Option<String>,
    /// The browser profile it came from, when there's something to show (P2.8): named,
    /// or one of several profiles of its browser.
    pub profile: Option<ProfileTag>,
    pub archived_at: Option<i64>,
    /// Why it was archived, in words ("Unused 61 active h").
    pub reason: Option<String>,
    /// Archived with others from the same window by the same sweep (⌘↵ restores them).
    pub batch: bool,
}

/// A row's browser profile: its name, else "Profile {ordinal}".
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileTag {
    pub id: String,
    pub name: Option<String>,
    pub ordinal: u32,
}

impl ProfileTag {
    /// `None` for a browser's only, unnamed profile (just the browser is shown).
    pub fn of(p: &decay_store::Profile) -> Option<Self> {
        match p.label() {
            decay_store::ProfileLabel::Only => None,
            decay_store::ProfileLabel::Named(name) => {
                Some(Self { id: p.profile.clone(), name: Some(name), ordinal: p.ordinal })
            }
            decay_store::ProfileLabel::Numbered(ordinal) => {
                Some(Self { id: p.profile.clone(), name: None, ordinal })
            }
        }
    }
}

/// A source's profile and title.
type SourceInfo = (Option<String>, Option<String>);

/// Lookups shared by the rows of one list (app names, sources, profiles).
#[derive(Default)]
pub struct RowCache {
    apps: HashMap<String, String>,
    /// Source id → (profile, title).
    sources: HashMap<String, Option<SourceInfo>>,
    profiles: Option<HashMap<(String, String), decay_store::Profile>>,
}

impl RowCache {
    fn app_name(&mut self, store: &Store, app_id: &str) -> String {
        self.apps
            .entry(app_id.to_owned())
            .or_insert_with(|| {
                store.app(app_id).ok().flatten().map_or_else(|| app_id.to_owned(), |a| a.name)
            })
            .clone()
    }

    fn source(&mut self, store: &Store, id: &str) -> Option<SourceInfo> {
        self.sources
            .entry(id.to_owned())
            .or_insert_with(|| store.source(id).ok().flatten().map(|s| (s.profile, s.title)))
            .clone()
    }

    fn profile(&mut self, store: &Store, app_id: &str, profile: &str) -> Option<ProfileTag> {
        let all = self.profiles.get_or_insert_with(|| store.profile_map().unwrap_or_default());
        all.get(&(app_id.to_owned(), profile.to_owned())).and_then(ProfileTag::of)
    }
}

/// `search` result.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchOutput {
    pub items: Vec<PanelItem>,
    pub next_cursor: Option<String>,
    /// Everything in the archive ("5 of 318 archived").
    pub total_archived: u64,
    pub took_ms: u64,
}

/// Why an item was archived, as its reason chip says it.
pub fn reason_label(item: &Item) -> Option<String> {
    reason_text(item.archive_reason.as_ref()?)
}

/// A reason as its chip says it ("Unused 61 active h"); none for a protection.
pub fn reason_text(reason: &Reason) -> Option<String> {
    match reason {
        Reason::Unused { active_hours, .. } => {
            Some(tf("reason.unused", &[("n", (*active_hours).into())]))
        }
        Reason::CustomRule { active_hours, .. } => {
            Some(tf("reason.rule", &[("n", (*active_hours).into())]))
        }
        Reason::Protected { .. } => None,
    }
}

/// The user's offset from UTC in minutes now (`before:` / `after:` dates are local).
fn local_offset_minutes() -> i32 {
    jiff::Zoned::now().offset().seconds() / 60
}

/// An item as a list row (app, source window and profile looked up, cached in `cache`).
pub fn row(store: &Store, item: Item, cache: &mut RowCache) -> PanelItem {
    let app_name = cache.app_name(store, &item.app_id);
    let (profile, source_title) =
        item.source_id.as_deref().and_then(|id| cache.source(store, id)).unwrap_or_default();
    let source_title = source_title.filter(|t| !t.trim().is_empty());
    let profile = profile.and_then(|p| cache.profile(store, &item.app_id, &p));
    PanelItem {
        reason: reason_label(&item),
        batch: item.last_sweep_id.is_some(),
        id: item.id,
        kind: item.kind,
        title: item.title,
        url: item.url,
        domain: item.domain,
        app_name,
        source_id: item.source_id,
        source_title,
        profile,
        archived_at: item.archived_at,
    }
}

/// Runs a search against the store (pure apart from the store: tested directly).
pub fn run_search(
    store: &Store,
    input: &SearchInput,
    now: i64,
    utc_offset: i32,
) -> Res<SearchOutput> {
    let started = Instant::now();
    let offset = match input.cursor.as_deref() {
        None => 0,
        Some(c) => c.parse().map_err(|_| AppError::new("INVALID_INPUT", "bad cursor"))?,
    };
    let query = SearchQuery {
        text: input.query.chars().take(500).collect(),
        filters: SearchFilters {
            kind: input.kind,
            after: input.after,
            app: input.app.clone().filter(|a| !a.is_empty() && a.len() <= 256),
            profile: input.profile.clone().filter(|p| !p.is_empty() && p.len() <= 256),
            ..SearchFilters::default()
        },
        limit: input.limit.unwrap_or(50).clamp(1, MAX_LIMIT),
        offset,
        now,
        utc_offset_minutes: utc_offset,
    };
    let page = store.search(&query).map_err(|e| AppError::db(&e))?;
    let mut names = RowCache::default();
    let items = page.hits.into_iter().map(|h| row(store, h.item, &mut names)).collect();
    Ok(SearchOutput {
        items,
        next_cursor: page.next_offset.map(|o| o.to_string()),
        total_archived: store.archived_count().map_err(|e| AppError::db(&e))?,
        took_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
    })
}

/// Searches the archive (empty query: the most recent first).
#[tauri::command]
pub async fn search(app: AppHandle, input: SearchInput) -> Res<SearchOutput> {
    let utc_offset = local_offset_minutes();
    blocking(app, move |core| {
        core.ask(move |e, now| (run_search(e.store(), &input, now, utc_offset), Vec::new()))?
    })
    .await
}

fn restore_refused(why: &RestoreRefused) -> AppError {
    let (code, key) = match why {
        RestoreRefused::NothingToRestore => ("NOT_FOUND", "refused.restore.nothing"),
        RestoreRefused::NoBrowser => ("ADAPTER_UNAVAILABLE", "refused.restore.noBrowser"),
        RestoreRefused::ProRequired => ("PRO_REQUIRED", "refused.restore.proRequired"),
    };
    AppError::new(code, t(key))
}

/// "Google Chrome · Work", or just the app for its only, unnamed profile.
pub fn profile_label(p: &ProfileInfo) -> String {
    let name = match &p.name {
        Some(n) => Some(n.clone()),
        None if p.has_siblings => Some(tf("profile.numbered", &[("n", p.ordinal.into())])),
        None => None,
    };
    match name {
        Some(profile) => tf(
            "profile.appWith",
            &[("app", p.app_name.clone().into()), ("profile", profile.into())],
        ),
        None => p.app_name.clone(),
    }
}

/// A restore refused (P2.8): when it's because the profile the tabs came from isn't
/// connected, the message names that profile and the details offer the connected ones
/// (`{ itemIds, home, alternatives: [{ appId, profileId, label }] }`) for "Open in … instead".
fn refused_with_choices(e: &Engine, why: &RestoreRefused, ids: &[String], now: i64) -> AppError {
    let err = restore_refused(why);
    if *why != RestoreRefused::NoBrowser {
        return err;
    }
    let Ok(profiles) = e.profiles(now) else { return err };
    let home = e.unreachable_home(ids, now).ok().flatten();
    let home =
        home.and_then(|(app, id)| profiles.iter().find(|p| p.app_id == app && p.profile_id == id));
    let entry = |p: &ProfileInfo| json!({ "appId": p.app_id, "profileId": p.profile_id, "label": profile_label(p) });
    let alternatives: Vec<Value> = profiles.iter().filter(|p| p.connected).map(entry).collect();
    let message = match home {
        Some(h) => tf("refused.restore.openProfile", &[("browser", profile_label(h).into())]),
        // No known profile (imported) and several could take it: the user picks one.
        None if !alternatives.is_empty() => t("refused.restore.chooseProfile"),
        None => err.message.clone(),
    };
    AppError::new(err.code, message).with_details(json!({
        "itemIds": ids,
        "home": home.map(entry),
        "alternatives": alternatives,
    }))
}

/// Where `restore_items` puts the tabs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RestoreTarget {
    /// Back into their original window (or a new one per original window).
    #[default]
    Original,
    /// All into one new window.
    NewWindow,
}

/// Restores items (↵, or ⌥↵ into a new window). Starts the restore; its result arrives as
/// `items:restored`.
#[tauri::command]
pub async fn restore_items(
    app: AppHandle,
    ids: Vec<String>,
    target: Option<RestoreTarget>,
    search_ms: Option<u64>,
) -> Res<Value> {
    if ids.is_empty() || ids.len() > 500 {
        return Err(AppError::new("INVALID_INPUT", t("error.restore")));
    }
    let new_window = target.unwrap_or_default() == RestoreTarget::NewWindow;
    blocking(app, move |core| {
        core.ask(move |e, now| match e.restore_searched(&ids, new_window, search_ms, now) {
            Ok(Ok(out)) => (Ok(json!({ "started": true })), out),
            Ok(Err(why)) => (Err(refused_with_choices(e, &why, &ids, now)), Vec::new()),
            Err(err) => (Err(AppError::db(&err)), Vec::new()),
        })?
    })
    .await
}

/// Restores the batch an item was archived with (⌘↵), in order and with its groups.
#[tauri::command]
pub async fn restore_batch(app: AppHandle, item_id: String) -> Res<Value> {
    blocking(app, move |core| {
        core.ask(move |e, now| match e.restore_batch(&item_id, now) {
            Ok(Ok(out)) => (Ok(json!({ "started": true })), out),
            Ok(Err(why)) => {
                let ids = e.restore_ids(&item_id).unwrap_or_default();
                (Err(refused_with_choices(e, &why, &ids, now)), Vec::new())
            }
            Err(err) => (Err(AppError::db(&err)), Vec::new()),
        })?
    })
    .await
}

/// Restores items into a new window of another connected browser profile: "Open in …
/// instead" after a restore was refused because theirs isn't connected (P2.8).
#[tauri::command]
pub async fn restore_items_in(
    app: AppHandle,
    ids: Vec<String>,
    app_id: String,
    profile_id: String,
) -> Res<Value> {
    if ids.is_empty() || ids.len() > 500 {
        return Err(AppError::new("INVALID_INPUT", t("error.restore")));
    }
    blocking(app, move |core| {
        core.ask(move |e, now| match e.restore_into(&ids, &app_id, &profile_id, now) {
            Ok(Ok(out)) => (Ok(json!({ "started": true })), out),
            Ok(Err(why)) => (Err(restore_refused(&why)), Vec::new()),
            Err(err) => (Err(AppError::db(&err)), Vec::new()),
        })?
    })
    .await
}

/// "Always keep" an item's site (⌘P): a protect rule for its domain.
#[tauri::command]
pub async fn protect_item(app: AppHandle, id: String) -> Res<decay_core::Rule> {
    let handle = app.clone();
    let out = blocking(app, move |core| {
        core.ask(move |e, now| match e.protect_site(&id, now) {
            Ok(Some(rule)) => (Ok(rule), Vec::new()),
            Ok(None) => (Err(AppError::new("NOT_FOUND", t("error.noSite"))), Vec::new()),
            Err(err) => (Err(AppError::db(&err)), Vec::new()),
        })?
    })
    .await;
    if out.is_ok() {
        crate::rules::announce(&handle);
    }
    out
}

/// Opens an item's details in the Library (⌘I) and hides the panel.
#[tauri::command]
pub fn ui_show_item(app: AppHandle, id: String) -> Res<Value> {
    if !(1..=64).contains(&id.len()) || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return Err(AppError::new("INVALID_INPUT", t("error.notItem")));
    }
    shell::hide_panel(&app);
    shell::show_window(&app, Window::Library, &format!("/item/{id}"));
    Ok(json!({}))
}

#[cfg(test)]
mod tests {
    use decay_core::{ItemState, ProtectedBy};
    use decay_store::{DbKey, StorePaths};

    use super::*;

    const NOW: i64 = 1_793_750_400_000;

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let (s, _) =
            Store::open(StorePaths::in_dir(dir.path()), DbKey::from_bytes([7; 32]), NOW).unwrap();
        (dir, s)
    }

    fn archived(id: &str, title: &str, url: &str, at: i64) -> Item {
        let mut item = Item::new_live(id.into(), ItemKind::Tab, "com.google.Chrome", at - 1000);
        item.title = title.into();
        item.url = Some(url.into());
        item.domain = url.split('/').nth(2).map(str::to_owned);
        item.state = ItemState::Archived;
        item.archived_at = Some(at);
        item.state_changed_at = at;
        item.archive_reason = Some(Reason::Unused { active_hours: 61, threshold: 60.0 });
        item
    }

    #[test]
    fn reasons_in_words() {
        let mut item = archived("a", "A", "https://a.example/", NOW);
        assert_eq!(reason_label(&item).as_deref(), Some("Unused 61 active h"));
        item.archive_reason = Some(Reason::CustomRule { rule_id: "r".into(), active_hours: 5 });
        assert_eq!(reason_label(&item).as_deref(), Some("Rule: 5 active h"));
        item.archive_reason = Some(Reason::Protected { by: ProtectedBy::Pinned, rule_id: None });
        assert_eq!(reason_label(&item), None);
        item.archive_reason = None;
        assert_eq!(reason_label(&item), None);
    }

    /// P2.8: a row names its profile when the browser has several (or it's named), and
    /// says nothing for a browser's only, unnamed profile.
    #[test]
    fn rows_carry_their_profile() {
        let (_dir, s) = store();
        let chrome = "com.google.Chrome";
        s.upsert_app(&decay_store::App {
            id: chrome.into(),
            name: "Google Chrome".into(),
            platform: decay_store::Platform::MacOs,
            is_browser: true,
            close_allowed: false,
            restore_kind: decay_store::RestoreKind::Url,
            icon_png: None,
        })
        .unwrap();
        for (src, profile) in [("w1", "pa"), ("w2", "pb")] {
            s.upsert_source(&decay_store::Source {
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
        let mut a = archived("a", "A", "https://a.example/", NOW);
        a.source_id = Some("w1".into());
        let mut b = archived("b", "B", "https://b.example/", NOW);
        b.source_id = Some("w2".into());
        s.touch_profile(chrome, "pa", NOW).unwrap();
        let only = row(&s, a.clone(), &mut RowCache::default());
        assert_eq!(only.profile, None, "a browser's only profile isn't named");
        s.touch_profile(chrome, "pb", NOW + 1).unwrap();
        s.rename_profile(chrome, "pb", Some("Work")).unwrap();
        let mut cache = RowCache::default();
        let ra = row(&s, a, &mut cache);
        let rb = row(&s, b, &mut cache);
        assert_eq!(ra.profile, Some(ProfileTag { id: "pa".into(), name: None, ordinal: 1 }));
        assert_eq!(
            rb.profile,
            Some(ProfileTag { id: "pb".into(), name: Some("Work".into()), ordinal: 2 })
        );
        assert_eq!(rb.app_name, "Google Chrome");
    }

    #[test]
    fn search_finds_ranks_pages_and_counts() {
        let (_dir, s) = store();
        s.insert_item(&archived(
            "p1",
            "Stripe pricing — Compare plans",
            "https://stripe.com/pricing",
            NOW - 3_600_000,
        ))
        .unwrap();
        s.insert_item(&archived(
            "p2",
            "Vercel pricing",
            "https://vercel.com/pricing",
            NOW - 7_200_000,
        ))
        .unwrap();
        s.insert_item(&archived("x", "Unrelated notes", "https://notes.example/", NOW)).unwrap();
        let out = run_search(
            &s,
            &SearchInput { query: "pric".into(), limit: Some(1), ..Default::default() },
            NOW,
            0,
        )
        .unwrap();
        assert_eq!(out.total_archived, 3);
        assert_eq!(out.items.len(), 1);
        let first = out.items[0].id.clone();
        assert!(first == "p1" || first == "p2", "a match: {first}");
        assert_eq!(out.items[0].reason.as_deref(), Some("Unused 61 active h"));
        assert_eq!(out.items[0].app_name, "Google Chrome");
        let next = SearchInput {
            query: "pric".into(),
            limit: Some(1),
            cursor: out.next_cursor,
            ..Default::default()
        };
        let page2 = run_search(&s, &next, NOW, 0).unwrap();
        let second: Vec<&str> = page2.items.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(second, [if first == "p1" { "p2" } else { "p1" }], "the other match, once");
        assert_eq!(page2.next_cursor, None);
        // An empty query lists the archive, most recent first; `site:` filters.
        let all = run_search(&s, &SearchInput::default(), NOW, 0).unwrap();
        assert_eq!(all.items[0].id, "x");
        let site = run_search(
            &s,
            &SearchInput { query: "site:vercel.com".into(), ..Default::default() },
            NOW,
            0,
        )
        .unwrap();
        assert_eq!(site.items.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), ["p2"]);
        // Windows only: none here.
        let windows = run_search(
            &s,
            &SearchInput { kind: Some(ItemKind::Window), ..Default::default() },
            NOW,
            0,
        )
        .unwrap();
        assert_eq!(windows.items, []);
    }

    #[test]
    fn a_bad_cursor_is_invalid_input() {
        let (_dir, s) = store();
        let input = SearchInput { cursor: Some("x".into()), ..Default::default() };
        assert_eq!(run_search(&s, &input, NOW, 0).unwrap_err().code, "INVALID_INPUT");
    }

    #[test]
    fn refusals_map_to_app_errors() {
        assert_eq!(restore_refused(&RestoreRefused::NoBrowser).code, "ADAPTER_UNAVAILABLE");
        assert_eq!(restore_refused(&RestoreRefused::NothingToRestore).code, "NOT_FOUND");
    }
}
