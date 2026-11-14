-- P1.10: onboarding (F21) shows until `onboarded_at` is set. Databases from before it
-- that were already in use (they have items or sweeps) count as onboarded, so an update
-- doesn't send their users back through onboarding (which resets mode and pace).
-- A new database runs this on empty tables: nothing is inserted.
INSERT OR IGNORE INTO settings (key, value, updated_at)
SELECT 'onboarded_at', CAST(t AS TEXT), t
FROM (SELECT CAST(strftime('%s', 'now') AS INTEGER) * 1000 AS t)
WHERE EXISTS (SELECT 1 FROM items) OR EXISTS (SELECT 1 FROM sweeps);
