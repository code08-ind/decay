//! Sources (DB Design §3.2): the browser window (or app) an item lives in, so a batch can
//! be restored into its original window.

use rusqlite::{OptionalExtension, params};

use crate::{Result, Store};

/// A browser window (per profile and browser session) or an app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    pub id: String,
    pub app_id: String,
    /// Browser profile id (the extension's random `profileId`).
    pub profile: Option<String>,
    /// Browser windows: `<session>:<windowId>`, the extension's `tabs.restore` window key.
    pub external_id: Option<String>,
    pub title: Option<String>,
    pub first_seen_at: i64,
    pub last_seen_at: i64,
    pub closed_at: Option<i64>,
}

impl Store {
    /// Adds a source, or marks a known one as seen at `source.last_seen_at` (its other
    /// fields are kept, and a closed one opens again).
    pub fn upsert_source(&self, source: &Source) -> Result<()> {
        self.conn
            .prepare_cached(
                "INSERT INTO sources (id, app_id, profile, external_id, title, first_seen_at,
                   last_seen_at, closed_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 ON CONFLICT (id) DO UPDATE SET
                   last_seen_at = max(sources.last_seen_at, excluded.last_seen_at),
                   title = coalesce(excluded.title, sources.title),
                   closed_at = NULL",
            )?
            .execute(params![
                source.id,
                source.app_id,
                source.profile,
                source.external_id,
                source.title,
                source.first_seen_at,
                source.last_seen_at,
                source.closed_at,
            ])?;
        Ok(())
    }

    pub fn source(&self, id: &str) -> Result<Option<Source>> {
        Ok(self
            .conn
            .prepare_cached(
                "SELECT id, app_id, profile, external_id, title, first_seen_at, last_seen_at,
                   closed_at FROM sources WHERE id = ?1",
            )?
            .query_row([id], |r| {
                Ok(Source {
                    id: r.get(0)?,
                    app_id: r.get(1)?,
                    profile: r.get(2)?,
                    external_id: r.get(3)?,
                    title: r.get(4)?,
                    first_seen_at: r.get(5)?,
                    last_seen_at: r.get(6)?,
                    closed_at: r.get(7)?,
                })
            })
            .optional()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::*;

    #[test]
    fn upsert_keeps_the_first_sighting() {
        let (store, _dir) = store();
        let mut s = Source {
            id: "s1".into(),
            app_id: "com.google.Chrome".into(),
            profile: Some("p".into()),
            external_id: Some("sess:7".into()),
            title: None,
            first_seen_at: NOW,
            last_seen_at: NOW,
            closed_at: None,
        };
        store.upsert_source(&s).unwrap();
        s.first_seen_at = NOW + 50;
        s.last_seen_at = NOW + 50;
        store.upsert_source(&s).unwrap();
        let got = store.source("s1").unwrap().unwrap();
        assert_eq!((got.first_seen_at, got.last_seen_at), (NOW, NOW + 50));
        assert_eq!(got.external_id.as_deref(), Some("sess:7"));
        assert_eq!(store.source("nope").unwrap(), None);
    }
}
