-- P2.7: on-device metrics (F34, DB Design §3.10).
--
-- `metrics_daily` keeps the samples behind each daily median, so a range's median can be
-- taken over them.
ALTER TABLE metrics_daily ADD COLUMN sampled_hours        TEXT NOT NULL DEFAULT '[]';
ALTER TABLE metrics_daily ADD COLUMN open_tabs_samples    TEXT NOT NULL DEFAULT '[]';
ALTER TABLE metrics_daily ADD COLUMN open_windows_samples TEXT NOT NULL DEFAULT '[]';
ALTER TABLE metrics_daily ADD COLUMN search_samples       TEXT NOT NULL DEFAULT '[]';

-- What happened, copied as it happens: not tied to items, so deleting an item (closing a
-- tab, a delete, an exclude rule, retention) never changes a day's counts. Kept 400 days.
-- `day` is the local day (`YYYY-MM-DD`), set by the engine within a tick of the event in
-- the zone of the time (NULL until then), so travelling later doesn't move it.
--   archived        an item archived
--   restored_24h    an item restored (or undone) within 24 h of being archived
--   undo            a tab sweep undone (once per sweep: `ref` = sweep id)
--   shadow_flagged  shadow mode would have archived an item (`ref` = the event id)
--   shadow_regret   a flagged item used deliberately within 24 h (`at` = the flag's time,
--                   `ref` = the flag's event id: once per flag)
CREATE TABLE metric_events (
  id   INTEGER PRIMARY KEY,
  kind TEXT NOT NULL CHECK (kind IN ('archived', 'restored_24h', 'undo', 'shadow_flagged',
         'shadow_regret')),
  at   INTEGER NOT NULL,
  ref  TEXT,
  day  TEXT,
  UNIQUE (kind, ref)
);
CREATE INDEX idx_metric_events_at ON metric_events(at);
CREATE INDEX idx_metric_events_undated ON metric_events(id) WHERE day IS NULL;

CREATE TRIGGER metric_archived AFTER INSERT ON events WHEN new.type = 'archived'
BEGIN
  INSERT INTO metric_events (kind, at) VALUES ('archived', new.at);
END;

CREATE TRIGGER metric_shadow_flagged AFTER INSERT ON events
WHEN new.type = 'shadow_would_archive'
BEGIN
  INSERT OR IGNORE INTO metric_events (kind, at, ref)
  VALUES ('shadow_flagged', new.at, CAST(new.id AS TEXT));
END;

CREATE TRIGGER metric_restored_24h AFTER INSERT ON events
WHEN new.type IN ('restored', 'undo')
  AND EXISTS (SELECT 1 FROM events a WHERE a.item_id = new.item_id AND a.type = 'archived'
              AND a.at <= new.at AND a.at > new.at - 86400000)
BEGIN
  INSERT INTO metric_events (kind, at) VALUES ('restored_24h', new.at);
END;

-- Window-hide sweeps (P2.2) aren't archives: their Undo isn't counted.
CREATE TRIGGER metric_undo AFTER INSERT ON events
WHEN new.type = 'undo' AND new.sweep_id IS NOT NULL
  AND NOT EXISTS (SELECT 1 FROM sweeps s WHERE s.id = new.sweep_id
                  AND s.reason LIKE '%"kind":"hide"%')
BEGIN
  INSERT OR IGNORE INTO metric_events (kind, at, ref) VALUES ('undo', new.at, new.sweep_id);
END;

-- What's already recorded (shadow regret can't be told apart from history: not copied).
INSERT INTO metric_events (kind, at) SELECT 'archived', at FROM events WHERE type = 'archived';
INSERT OR IGNORE INTO metric_events (kind, at, ref)
  SELECT 'shadow_flagged', at, CAST(id AS TEXT) FROM events WHERE type = 'shadow_would_archive';
INSERT INTO metric_events (kind, at)
  SELECT 'restored_24h', e.at FROM events e
  WHERE e.type IN ('restored', 'undo')
    AND EXISTS (SELECT 1 FROM events a WHERE a.item_id = e.item_id AND a.type = 'archived'
                AND a.at <= e.at AND a.at > e.at - 86400000);
INSERT OR IGNORE INTO metric_events (kind, at, ref)
  SELECT 'undo', min(e.at), e.sweep_id FROM events e
  WHERE e.type = 'undo' AND e.sweep_id IS NOT NULL
    AND NOT EXISTS (SELECT 1 FROM sweeps s WHERE s.id = e.sweep_id
                    AND s.reason LIKE '%"kind":"hide"%')
  GROUP BY e.sweep_id;
