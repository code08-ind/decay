-- F15 (Pro, off by default): a short name for each archived batch (the items one sweep
-- archived from one window), made on this computer (Apple's on-device model, or a
-- keyword labeller). `source_key` is the window's source id, '' for none.
CREATE TABLE batch_labels (
  sweep_id   TEXT NOT NULL REFERENCES sweeps(id) ON DELETE CASCADE,
  source_key TEXT NOT NULL,
  label      TEXT NOT NULL CHECK (length(label) BETWEEN 1 AND 80),
  model      TEXT NOT NULL CHECK (model IN ('apple', 'keywords')),
  created_at INTEGER NOT NULL,
  PRIMARY KEY (sweep_id, source_key)
) WITHOUT ROWID;
