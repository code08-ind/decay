-- V1: the full schema of DB Design §3, default rules and the default app catalog.
-- Timestamps are Unix epoch milliseconds (UTC), durations seconds, booleans 0/1.

CREATE TABLE apps (
  id            TEXT PRIMARY KEY,
  name          TEXT NOT NULL,
  platform      TEXT NOT NULL CHECK (platform IN ('macos', 'windows')),
  kind          TEXT NOT NULL CHECK (kind IN ('browser', 'app')),
  close_allowed INTEGER NOT NULL DEFAULT 0 CHECK (close_allowed IN (0, 1)),
  restore_kind  TEXT NOT NULL DEFAULT 'none'
                CHECK (restore_kind IN ('url', 'folder', 'document', 'app_only', 'none')),
  icon_png      BLOB CHECK (icon_png IS NULL OR length(icon_png) <= 16384)
);

CREATE TABLE sources (
  id            TEXT PRIMARY KEY,
  app_id        TEXT NOT NULL REFERENCES apps(id),
  profile       TEXT,
  external_id   TEXT,
  title         TEXT,
  first_seen_at INTEGER NOT NULL,
  last_seen_at  INTEGER NOT NULL,
  closed_at     INTEGER
);

CREATE TABLE sweeps (
  id           TEXT PRIMARY KEY,
  started_at   INTEGER NOT NULL,
  finished_at  INTEGER,
  trigger      TEXT NOT NULL CHECK (trigger IN ('scheduled', 'wake', 'manual')),
  is_shadow    INTEGER NOT NULL DEFAULT 0 CHECK (is_shadow IN (0, 1)),
  status       TEXT NOT NULL CHECK (status IN ('pending', 'running', 'done', 'failed', 'undone')),
  item_count   INTEGER NOT NULL DEFAULT 0,
  failed_count INTEGER NOT NULL DEFAULT 0,
  reason       TEXT,
  undone_at    INTEGER
);
CREATE INDEX idx_sweeps_started ON sweeps(started_at DESC);

-- `seq` is the FTS rowid: an explicit INTEGER PRIMARY KEY never changes (an implicit
-- rowid may be renumbered by VACUUM, which would detach items from their search entries).
CREATE TABLE items (
  seq              INTEGER PRIMARY KEY,
  id               TEXT NOT NULL UNIQUE,
  kind             TEXT NOT NULL CHECK (kind IN ('tab', 'window')),
  app_id           TEXT NOT NULL REFERENCES apps(id),
  source_id        TEXT REFERENCES sources(id) ON DELETE SET NULL,
  external_id      TEXT,
  title            TEXT NOT NULL DEFAULT '',
  url              TEXT,
  domain           TEXT,
  path             TEXT,
  favicon          BLOB CHECK (favicon IS NULL OR length(favicon) <= 4096),
  -- Where the favicon can be fetched from (imported archives arrive without the image).
  favicon_url      TEXT,
  tab_index        INTEGER,
  group_title      TEXT,
  group_color      TEXT,
  pinned           INTEGER NOT NULL DEFAULT 0 CHECK (pinned IN (0, 1)),
  state            TEXT NOT NULL CHECK (state IN
                     ('active', 'stale', 'hidden', 'archiving', 'archived', 'restored', 'purged')),
  save_state       TEXT NOT NULL DEFAULT 'unknown'
                     CHECK (save_state IN ('saved', 'unsaved', 'unknown', 'n/a')),
  active_age_s     INTEGER NOT NULL DEFAULT 0 CHECK (active_age_s >= 0),
  first_seen_at    INTEGER NOT NULL,
  last_used_at     INTEGER NOT NULL,
  state_changed_at INTEGER NOT NULL,
  archived_at      INTEGER,
  archive_reason   TEXT,
  last_sweep_id    TEXT REFERENCES sweeps(id) ON DELETE SET NULL,
  restored_from_id TEXT REFERENCES items(id) ON DELETE SET NULL,
  grace_until      INTEGER,
  restore_count    INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX idx_items_state_age   ON items(state, active_age_s DESC);
CREATE INDEX idx_items_archived_at ON items(archived_at DESC) WHERE state = 'archived';
CREATE INDEX idx_items_source      ON items(source_id, tab_index);
CREATE INDEX idx_items_domain      ON items(domain);
CREATE INDEX idx_items_app         ON items(app_id);
CREATE INDEX idx_items_last_sweep  ON items(last_sweep_id);
-- Foreign-key children: without these, deleting an item or a sweep scans every row.
CREATE INDEX idx_items_restored_from ON items(restored_from_id) WHERE restored_from_id IS NOT NULL;
CREATE UNIQUE INDEX idx_items_live_ext ON items(app_id, external_id)
  WHERE state IN ('active', 'stale', 'hidden', 'archiving');

-- Contentless: the text lives in `items` only. URLs are indexed split into words
-- (`decay_url_words`, registered on every connection by decay-store).
CREATE VIRTUAL TABLE items_fts USING fts5(
  title, url, domain, path, group_title, app_name,
  content = '',
  contentless_delete = 1,
  tokenize = 'unicode61 remove_diacritics 2 tokenchars ''-_''',
  prefix = '2 3'
);

CREATE TRIGGER items_fts_insert AFTER INSERT ON items BEGIN
  INSERT INTO items_fts (rowid, title, url, domain, path, group_title, app_name)
  VALUES (new.seq, new.title, decay_url_words(new.url), new.domain, new.path, new.group_title,
          (SELECT name FROM apps WHERE id = new.app_id));
END;

CREATE TRIGGER items_fts_delete AFTER DELETE ON items BEGIN
  DELETE FROM items_fts WHERE rowid = old.seq;
END;

-- Only when indexed text changes: routine updates (age, state) don't touch the index.
CREATE TRIGGER items_fts_update AFTER UPDATE OF title, url, domain, path, group_title, app_id
ON items
WHEN old.title IS NOT new.title OR old.url IS NOT new.url OR old.domain IS NOT new.domain
  OR old.path IS NOT new.path OR old.group_title IS NOT new.group_title
  OR old.app_id IS NOT new.app_id
BEGIN
  DELETE FROM items_fts WHERE rowid = old.seq;
  INSERT INTO items_fts (rowid, title, url, domain, path, group_title, app_name)
  VALUES (new.seq, new.title, decay_url_words(new.url), new.domain, new.path, new.group_title,
          (SELECT name FROM apps WHERE id = new.app_id));
END;

-- A renamed app: re-index its items' app name.
CREATE TRIGGER items_fts_app_rename AFTER UPDATE OF name ON apps
WHEN old.name IS NOT new.name
BEGIN
  DELETE FROM items_fts WHERE rowid IN (SELECT seq FROM items WHERE app_id = new.id);
  INSERT INTO items_fts (rowid, title, url, domain, path, group_title, app_name)
  SELECT seq, title, decay_url_words(url), domain, path, group_title, new.name
  FROM items WHERE app_id = new.id;
END;

CREATE TABLE events (
  id       INTEGER PRIMARY KEY AUTOINCREMENT,
  item_id  TEXT NOT NULL REFERENCES items(id) ON DELETE CASCADE,
  type     TEXT NOT NULL CHECK (type IN ('seen', 'used', 'stale', 'hidden', 'archived',
             'close_failed', 'restored', 'undo', 'protected', 'purged', 'shadow_would_archive')),
  sweep_id TEXT REFERENCES sweeps(id) ON DELETE SET NULL,
  reason   TEXT,
  at       INTEGER NOT NULL
);
CREATE INDEX idx_events_item_at ON events(item_id, at DESC);
CREATE INDEX idx_events_type_at ON events(type, at);
CREATE INDEX idx_events_sweep   ON events(sweep_id) WHERE sweep_id IS NOT NULL;

CREATE TABLE sweep_items (
  sweep_id   TEXT NOT NULL REFERENCES sweeps(id) ON DELETE CASCADE,
  item_id    TEXT NOT NULL REFERENCES items(id) ON DELETE CASCADE,
  action     TEXT NOT NULL CHECK (action IN ('archive', 'hide', 'close')),
  prev_state TEXT NOT NULL CHECK (prev_state IN
               ('active', 'stale', 'hidden', 'archiving', 'archived', 'restored', 'purged')),
  result     TEXT NOT NULL DEFAULT 'pending'
               CHECK (result IN ('pending', 'ok', 'failed', 'skipped')),
  PRIMARY KEY (sweep_id, item_id)
);
CREATE INDEX idx_sweep_items_item ON sweep_items(item_id);

CREATE TABLE rules (
  id              TEXT PRIMARY KEY,
  scope           TEXT NOT NULL CHECK (scope IN ('app', 'domain', 'url_glob', 'title_regex', 'global')),
  match           TEXT NOT NULL,
  action          TEXT NOT NULL
                    CHECK (action IN ('protect', 'exclude', 'custom_threshold', 'close_allowed')),
  threshold_hours TEXT,
  enabled         INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
  source          TEXT NOT NULL DEFAULT 'user' CHECK (source IN ('user', 'default', 'quick_action')),
  created_at      INTEGER NOT NULL,
  hit_count       INTEGER NOT NULL DEFAULT 0,
  UNIQUE (scope, match, action)
);

CREATE TABLE settings (
  key        TEXT PRIMARY KEY,
  value      TEXT NOT NULL CHECK (json_valid(value)),
  updated_at INTEGER NOT NULL
);

CREATE TABLE metrics_daily (
  day                      TEXT PRIMARY KEY,
  active_seconds           INTEGER NOT NULL DEFAULT 0,
  open_tabs_p50            INTEGER,
  open_windows_p50         INTEGER,
  archived_count           INTEGER NOT NULL DEFAULT 0,
  restored_24h_count       INTEGER NOT NULL DEFAULT 0,
  undo_count               INTEGER NOT NULL DEFAULT 0,
  search_count             INTEGER NOT NULL DEFAULT 0,
  search_to_restore_ms_p50 INTEGER,
  auto_mode_on             INTEGER NOT NULL DEFAULT 0 CHECK (auto_mode_on IN (0, 1))
);

CREATE TABLE license (
  id               INTEGER PRIMARY KEY CHECK (id = 1),
  key_blob         TEXT,
  tier             TEXT NOT NULL DEFAULT 'free' CHECK (tier IN ('free', 'trial', 'pro')),
  trial_started_at INTEGER,
  activated_at     INTEGER
);

-- Default app catalog (browsers, and folder windows that may be closed: F11).
INSERT INTO apps (id, name, platform, kind, close_allowed, restore_kind) VALUES
  ('com.google.Chrome',           'Google Chrome',   'macos',   'browser', 0, 'url'),
  ('com.microsoft.edgemac',       'Microsoft Edge',  'macos',   'browser', 0, 'url'),
  ('com.brave.Browser',           'Brave',           'macos',   'browser', 0, 'url'),
  ('company.thebrowser.Browser',  'Arc',             'macos',   'browser', 0, 'url'),
  ('com.apple.finder',            'Finder',          'macos',   'app',     1, 'folder'),
  ('chrome.exe',                  'Google Chrome',   'windows', 'browser', 0, 'url'),
  ('msedge.exe',                  'Microsoft Edge',  'windows', 'browser', 0, 'url'),
  ('brave.exe',                   'Brave',           'windows', 'browser', 0, 'url'),
  ('explorer.exe',                'File Explorer',   'windows', 'app',     1, 'folder');

-- Default rules: calls and sign-in pages are never archived (domain rules match the host
-- and its subdomains). The policy reads close permission from `close_allowed` rules;
-- `apps.close_allowed` mirrors them for display.
INSERT INTO rules (id, scope, match, action, source, created_at) VALUES
  ('default-accounts-google', 'domain', 'accounts.google.com',  'protect', 'default', 0),
  ('default-zoom',            'domain', 'zoom.us',              'protect', 'default', 0),
  ('default-meet',            'domain', 'meet.google.com',      'protect', 'default', 0),
  ('default-teams',           'domain', 'teams.microsoft.com',  'protect', 'default', 0),
  ('default-close-finder',    'app',    'com.apple.finder',     'close_allowed', 'default', 0),
  ('default-close-explorer',  'app',    'explorer.exe',         'close_allowed', 'default', 0);

INSERT INTO license (id, tier) VALUES (1, 'free');
