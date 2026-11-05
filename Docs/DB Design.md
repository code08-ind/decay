# Decay — Database Design

| | |
| --- | --- |
| **Version** | 1.0 |
| **Date** | 4 Nov 2026 |
| **Engine** | SQLite 3 + FTS5, encrypted with SQLCipher 4 (AES-256) |
| **Related** | [Software Design](./Software%20Design.md) · [API Design](./API%20Design.md) |

---

## 1. Storage overview

| Store | Location | Purpose |
| --- | --- | --- |
| Main DB | macOS: `~/Library/Application Support/Decay/decay.db` · Windows: `%APPDATA%\Decay\decay.db` | Items, events, sweeps, rules, settings, metrics |
| Backups | `…/Decay/backups/decay-YYYYMMDD.db` | Daily online backup (`sqlite3_backup`), 7 kept, encrypted with the same key |
| Logs | `…/Decay/logs/` | Rolling, 7 days, redacted |
| DB key | Keychain service `com.decay.app` / Credential Manager `Decay/db-key` | 256-bit random key |
| Extension (Phase 0) | IndexedDB `decay` (Dexie) | Standalone archive; migrated to the main DB when the app connects |

**PRAGMAs at open:**
```sql
PRAGMA key = "x'<hex>'";          -- SQLCipher
PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;      -- FULL inside executor commits
PRAGMA foreign_keys = ON;
PRAGMA busy_timeout = 3000;
PRAGMA temp_store = MEMORY;
```

**Conventions:** IDs are **UUIDv7** stored as `TEXT` (sortable by time). Timestamps are **INTEGER Unix epoch milliseconds (UTC)**. Durations are **INTEGER seconds**. Booleans are `INTEGER 0/1`. Enums are `TEXT` with `CHECK` constraints.

## 2. Entity-relationship diagram

```
 sources 1───* items *───1 apps
              │  │
              │  └──* events
              │
              *──* sweeps (via sweep_items)
 items 1──1 items_fts (external content)
 rules (standalone)   settings (KV)   metrics_daily   schema_migrations   license
```

## 3. Tables

### 3.1 `apps`
Known applications and browsers.

| Column | Type | Constraints | Notes |
| --- | --- | --- | --- |
| id | TEXT | PK | e.g. `com.google.Chrome`, `explorer.exe` |
| name | TEXT | NOT NULL | Display name |
| platform | TEXT | CHECK IN ('macos','windows') | |
| kind | TEXT | CHECK IN ('browser','app') | |
| close_allowed | INTEGER | NOT NULL DEFAULT 0 | On the close allowlist (F11) |
| restore_kind | TEXT | CHECK IN ('url','folder','document','app_only','none') | From the spec's restore table |
| icon_png | BLOB | NULL | ≤ 16 KB cached icon |

### 3.2 `sources`
A source window (a browser window or an app window) that items belong to. Used for grouping and batch restore.

| Column | Type | Constraints | Notes |
| --- | --- | --- | --- |
| id | TEXT | PK | UUIDv7 |
| app_id | TEXT | FK → apps.id | |
| profile | TEXT | NULL | Browser profile (e.g. `Default`, `Profile 1`) |
| external_id | TEXT | NULL | Browser `windowId` / CGWindowID / HWND — **not stable across restarts** |
| title | TEXT | NULL | Last known window title |
| first_seen_at | INTEGER | NOT NULL | |
| last_seen_at | INTEGER | NOT NULL | |
| closed_at | INTEGER | NULL | |

### 3.3 `items` (core)

| Column | Type | Constraints | Notes |
| --- | --- | --- | --- |
| id | TEXT | PK | UUIDv7 |
| kind | TEXT | NOT NULL CHECK IN ('tab','window') | |
| app_id | TEXT | NOT NULL FK → apps.id | |
| source_id | TEXT | FK → sources.id ON DELETE SET NULL | Source window |
| external_id | TEXT | NULL | `tabId` / window id while live |
| title | TEXT | NOT NULL DEFAULT '' | |
| url | TEXT | NULL | Tabs |
| domain | TEXT | NULL | eTLD+1, lowercase (for rules/filters) |
| path | TEXT | NULL | Folder/document path for windows |
| favicon | BLOB | NULL | ≤ 4 KB |
| tab_index | INTEGER | NULL | Position in the window |
| group_title | TEXT | NULL | Tab group name |
| group_color | TEXT | NULL | Chrome group colour enum |
| pinned | INTEGER | NOT NULL DEFAULT 0 | |
| state | TEXT | NOT NULL CHECK IN ('active','stale','hidden','archiving','archived','restored','purged') | |
| save_state | TEXT | NOT NULL DEFAULT 'unknown' CHECK IN ('saved','unsaved','unknown','n/a') | Windows; tabs = 'n/a' |
| active_age_s | INTEGER | NOT NULL DEFAULT 0 | Active seconds since last use |
| first_seen_at | INTEGER | NOT NULL | |
| last_used_at | INTEGER | NOT NULL | Wall clock of last use |
| state_changed_at | INTEGER | NOT NULL | |
| archived_at | INTEGER | NULL | |
| archive_reason | TEXT | NULL | JSON-encoded `Reason` |
| last_sweep_id | TEXT | FK → sweeps.id ON DELETE SET NULL | |
| restored_from_id | TEXT | FK → items.id ON DELETE SET NULL | Links a restored live item to its archive record |
| grace_until | INTEGER | NULL | Protected until this time after undo/restore |
| restore_count | INTEGER | NOT NULL DEFAULT 0 | |

**Indexes:**
```sql
CREATE INDEX idx_items_state_age   ON items(state, active_age_s DESC);
CREATE INDEX idx_items_archived_at ON items(archived_at DESC) WHERE state = 'archived';
CREATE INDEX idx_items_source      ON items(source_id, tab_index);
CREATE INDEX idx_items_domain      ON items(domain);
CREATE INDEX idx_items_app         ON items(app_id);
CREATE UNIQUE INDEX idx_items_live_ext ON items(app_id, external_id)
  WHERE state IN ('active','stale','hidden','archiving');
```

### 3.4 `items_fts` (full-text search)
```sql
CREATE VIRTUAL TABLE items_fts USING fts5(
  title, url, domain, path, group_title, app_name,
  content='',                       -- contentless; populated by triggers
  tokenize = 'unicode61 remove_diacritics 2 tokenchars ''-_''',
  prefix = '2 3'
);
-- Triggers on items INSERT/UPDATE/DELETE keep items_fts in sync (rowid = items.rowid).
```
Ranking: `bm25(items_fts, 10.0, 3.0, 4.0, 2.0, 2.0, 1.0)` combined in Rust with a recency boost `1 / (1 + days_since_archived / 7)`.

The URL is tokenized after splitting on `/ . ? = &` (pre-processed in Rust) so that "pricing" matches `…/pricing?plan=pro`.

### 3.5 `events` (append-only audit log)

| Column | Type | Constraints | Notes |
| --- | --- | --- | --- |
| id | INTEGER | PK AUTOINCREMENT | |
| item_id | TEXT | NOT NULL FK → items.id ON DELETE CASCADE | |
| type | TEXT | NOT NULL CHECK IN ('seen','used','stale','hidden','archived','close_failed','restored','undo','protected','purged','shadow_would_archive') | |
| sweep_id | TEXT | NULL FK → sweeps.id | |
| reason | TEXT | NULL | JSON |
| at | INTEGER | NOT NULL | |

Index: `CREATE INDEX idx_events_item_at ON events(item_id, at DESC);` · `CREATE INDEX idx_events_type_at ON events(type, at);`
`used` events are **sampled** (at most one per item per 10 min) to limit growth.

### 3.6 `sweeps`

| Column | Type | Constraints | Notes |
| --- | --- | --- | --- |
| id | TEXT | PK | UUIDv7 |
| started_at | INTEGER | NOT NULL | |
| finished_at | INTEGER | NULL | |
| trigger | TEXT | CHECK IN ('scheduled','wake','manual') | |
| is_shadow | INTEGER | NOT NULL DEFAULT 0 | Shadow-mode simulation |
| status | TEXT | CHECK IN ('pending','running','done','failed','undone') | |
| item_count | INTEGER | NOT NULL DEFAULT 0 | |
| failed_count | INTEGER | NOT NULL DEFAULT 0 | |
| reason | TEXT | NULL | Summary (e.g. `{"preset":"balanced"}`) |
| undone_at | INTEGER | NULL | |

### 3.7 `sweep_items`

| Column | Type | Notes |
| --- | --- | --- |
| sweep_id | TEXT FK → sweeps.id ON DELETE CASCADE | |
| item_id | TEXT FK → items.id ON DELETE CASCADE | |
| action | TEXT CHECK IN ('archive','hide','close') | |
| prev_state | TEXT | For undo |
| result | TEXT CHECK IN ('pending','ok','failed','skipped') | |
| PRIMARY KEY (sweep_id, item_id) | | |

### 3.8 `rules`

| Column | Type | Constraints | Notes |
| --- | --- | --- | --- |
| id | TEXT | PK | |
| scope | TEXT | NOT NULL CHECK IN ('app','domain','url_glob','title_regex','global') | |
| match | TEXT | NOT NULL | e.g. `github.com`, `com.figma.Desktop`, `https://*.notion.so/*` |
| action | TEXT | NOT NULL CHECK IN ('protect','exclude','custom_threshold','close_allowed') | `exclude` = never stored (F9) |
| threshold_hours | TEXT | NULL | JSON `{stale,hidden,archive}` for `custom_threshold` |
| enabled | INTEGER | NOT NULL DEFAULT 1 | |
| source | TEXT | CHECK IN ('user','default','quick_action') | quick_action = created from Undo |
| created_at | INTEGER | NOT NULL | |
| hit_count | INTEGER | NOT NULL DEFAULT 0 | Shown in UI |

Unique: `(scope, match, action)`.

### 3.9 `settings` (key-value)

| Column | Type |
| --- | --- |
| key | TEXT PK |
| value | TEXT NOT NULL (JSON) |
| updated_at | INTEGER NOT NULL |

**Keys and defaults:**

| Key | Default |
| --- | --- |
| `mode` | `"shadow"` (`shadow` \| `automatic` \| `paused`) |
| `shadow_started_at` | first-run time |
| `preset` | `"balanced"` |
| `thresholds` | `{"stale":12,"hidden":36,"archive":60}` |
| `idle_cutoff_s` | `300` |
| `max_items_per_sweep` | `50` |
| `retention_days` | `90` |
| `hold_until` | `null` |
| `hotkey` | `"Alt+Super+D"` (macOS) / `"Ctrl+Alt+D"` (Windows) |
| `theme` | `"system"` |
| `launch_at_login` | `true` |
| `update_channel` | `"stable"` |
| `daily_summary_time` | `"09:00"` |
| `notifications` | `{"summary":true,"sweep":false,"health":true}` |
| `window_management` | `"hide"` (`off` \| `hide` \| `hide_and_close`) |
| `crash_reports` | `false` |
| `locale` | `"system"` |

### 3.10 `metrics_daily` (on-device success metrics)

| Column | Type | Notes |
| --- | --- | --- |
| day | TEXT PK | `YYYY-MM-DD` local |
| active_seconds | INTEGER | |
| open_tabs_p50 | INTEGER | Median open tabs sampled hourly |
| open_windows_p50 | INTEGER | |
| archived_count | INTEGER | |
| restored_24h_count | INTEGER | For regret rate |
| undo_count | INTEGER | |
| search_count | INTEGER | |
| search_to_restore_ms_p50 | INTEGER | |
| auto_mode_on | INTEGER | |

### 3.11 `license`

| Column | Type | Notes |
| --- | --- | --- |
| id | INTEGER PK CHECK (id = 1) | Singleton |
| key_blob | TEXT | Signed license (also mirrored in Keychain) |
| tier | TEXT CHECK IN ('free','trial','pro') | |
| trial_started_at | INTEGER | |
| activated_at | INTEGER | |

### 3.12 `schema_migrations`
Managed by `refinery` (`version`, `name`, `applied_on`, `checksum`).

## 4. Key queries

```sql
-- Candidates for a sweep
SELECT * FROM items
WHERE state IN ('active','stale','hidden')
  AND (grace_until IS NULL OR grace_until < :now)
  AND active_age_s >= :min_threshold_s
ORDER BY active_age_s DESC
LIMIT :max_items;

-- Search
SELECT i.*, bm25(items_fts, 10,3,4,2,2,1) AS rank
FROM items_fts JOIN items i ON i.rowid = items_fts.rowid
WHERE items_fts MATCH :q
  AND i.state = 'archived'
  AND (:app IS NULL OR i.app_id = :app)
  AND (:after IS NULL OR i.archived_at >= :after)
ORDER BY rank LIMIT 50;

-- Batch for an item (same sweep + same source)
SELECT * FROM items WHERE last_sweep_id = :sweep AND source_id = :source AND state='archived'
ORDER BY tab_index;

-- Regret rate (last 14 days)
SELECT SUM(restored_24h_count) * 1.0 / NULLIF(SUM(archived_count),0) FROM metrics_daily
WHERE day >= date('now','-14 days');
```

## 5. Data lifecycle and retention

| Job | Schedule | Action |
| --- | --- | --- |
| Retention purge | Daily, idle time | `archived` items older than `retention_days` → hard delete (cascade events), FTS cleaned via trigger. Pro "unlimited" skips |
| Event compaction | Weekly | Delete `used`/`seen` events older than 30 days |
| Closed live items | On adapter snapshot | Items that disappear without being archived (user closed them) → **hard delete** (we don't keep a history of what users close themselves) |
| Backup | Daily | `sqlite3_backup` → `backups/`, keep 7 |
| VACUUM | Monthly, idle | `PRAGMA incremental_vacuum` (auto_vacuum=INCREMENTAL) |
| Erase all data | User action | Close DB → securely delete DB, WAL, backups → delete the Keychain key → re-init |

## 6. Migrations

- Embedded SQL files `crates/decay-store/migrations/V{n}__{name}.sql`, applied at startup inside a transaction, after an automatic pre-migration backup.
- Forward-only; a down-migration is a restore from the pre-migration backup.
- V1 creates everything above, plus default rules: protect `accounts.google.com`, `zoom.us` (domain rules match the host and its subdomains), `meet.google.com`, `teams.microsoft.com`, and the default app catalog with `close_allowed` for Finder / Explorer folder windows.

## 7. Extension IndexedDB schema (Phase 0)

```ts
db.version(1).stores({
  items:   'id, state, domain, archivedAt, lastSweepId, [state+activeAgeS], [appId+externalId]',
  events:  '++id, itemId, type, at',
  sweeps:  'id, startedAt, isShadow',
  rules:   'id, scope, match, &[scope+match+action]',
  settings:'key',
  metrics: 'day'
});
// Search in Phase 0: MiniSearch index over title/url/domain, rebuilt on load (≤ 20k items).
```
Field names mirror the SQL schema (camelCase). Phase 0 differences:
- `sweeps` records embed their `items` (no separate `sweep_items` store); `isShadow`/`autoModeOn` are `0|1` because IndexedDB can't index booleans.
- `[appId+externalId]` is not unique or state-aware like `idx_items_live_ext`, so `externalId` is cleared when an item leaves a live state and lookups use `findLiveByExternalId`.
- Domain rule matches are stored lowercase.
- Phase 0-only item fields: `windowExternalId` (live browser window id, stand-in for `sources`) and `lastUsedEventAt` (sampling of `used` events). Not imported.
- Item domains use eTLD+1 **including private suffixes** (`user.github.io`, `*.vercel.app` are separate domains). Same rule for the Rust core.
- The `ImportArchive` migration maps camelCase enums and settings keys to the snake_case SQL values, fetches `faviconUrl` into the `favicon` BLOB, and fills `saveState='n/a'`.

## 8. Export format (JSON, v1)

```json
{
  "format": "decay-archive",
  "version": 1,
  "exportedAt": 1793750400000,
  "items": [ { "id": "…", "kind": "tab", "title": "…", "url": "…", "domain": "…",
               "app": "com.google.Chrome", "profile": "Default", "sourceTitle": "…",
               "tabIndex": 3, "group": {"title":"Research","color":"blue"},
               "archivedAt": 0, "reason": {"type":"unused","activeHours":61} } ],
  "rules": [ … ],
  "settings": { … }
}
```
