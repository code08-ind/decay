-- P4.2 (F27): when Pro last ended (a trial running out, a license deactivated) or the
-- installation was first seen without it. Items archived from then on keep the free
-- retention (90 days); older ones keep the one they were archived under.
ALTER TABLE license ADD COLUMN free_since INTEGER;
