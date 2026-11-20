-- P2.8 (F35): the browser profiles Decay has seen, so the UI can tell them apart.
-- `profile` is the extension's random profile id; `name` is what the user called it
-- (NULL: unnamed, shown as "Profile N" by first sighting when its browser has several).
CREATE TABLE profiles (
  app_id        TEXT NOT NULL REFERENCES apps(id),
  profile       TEXT NOT NULL,
  name          TEXT CHECK (name IS NULL OR length(name) BETWEEN 1 AND 40),
  first_seen_at INTEGER NOT NULL,
  last_seen_at  INTEGER NOT NULL,
  PRIMARY KEY (app_id, profile)
) WITHOUT ROWID;

-- Profiles already known from their windows keep their order of first sighting.
INSERT OR IGNORE INTO profiles (app_id, profile, first_seen_at, last_seen_at)
SELECT app_id, profile, min(first_seen_at), max(last_seen_at)
FROM sources
WHERE profile IS NOT NULL AND profile <> ''
GROUP BY app_id, profile;
