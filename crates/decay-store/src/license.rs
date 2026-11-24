//! The license row (DB Design §3.11, P4.2): the activated key, when the trial started and
//! when Pro last ended. The tier column is what the app last worked out (for reading the
//! database by hand); the app always recomputes it from the key and the trial.

use rusqlite::params;

use crate::{Result, Store};

/// What's stored about the license.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LicenseRow {
    pub key: Option<String>,
    /// `free`, `trial` or `pro`.
    pub tier: String,
    pub trial_started_at: Option<i64>,
    pub activated_at: Option<i64>,
    pub free_since: Option<i64>,
}

impl Store {
    pub fn license(&self) -> Result<LicenseRow> {
        Ok(self.conn.query_row(
            "SELECT key_blob, tier, trial_started_at, activated_at, free_since
             FROM license WHERE id = 1",
            [],
            |r| {
                Ok(LicenseRow {
                    key: r.get(0)?,
                    tier: r.get(1)?,
                    trial_started_at: r.get(2)?,
                    activated_at: r.get(3)?,
                    free_since: r.get(4)?,
                })
            },
        )?)
    }

    /// Replaces the row. A tier other than `free` / `trial` / `pro` is refused by the
    /// table.
    pub fn save_license(&self, row: &LicenseRow) -> Result<()> {
        self.conn.execute(
            "INSERT INTO license (id, key_blob, tier, trial_started_at, activated_at, free_since)
             VALUES (1, ?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (id) DO UPDATE SET key_blob = excluded.key_blob, tier = excluded.tier,
               trial_started_at = excluded.trial_started_at,
               activated_at = excluded.activated_at, free_since = excluded.free_since",
            params![row.key, row.tier, row.trial_started_at, row.activated_at, row.free_since],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::LicenseRow;
    use crate::test_util::store;

    #[test]
    fn a_new_store_is_free_and_the_row_round_trips() {
        let (store, _dir) = store();
        let fresh = store.license().unwrap();
        assert_eq!(fresh, LicenseRow { tier: "free".into(), ..LicenseRow::default() });
        let row = LicenseRow {
            key: Some("DECAY-X".into()),
            tier: "pro".into(),
            trial_started_at: Some(1),
            activated_at: Some(2),
            free_since: Some(3),
        };
        store.save_license(&row).unwrap();
        assert_eq!(store.license().unwrap(), row);
        assert!(store.save_license(&LicenseRow { tier: "gold".into(), ..row }).is_err());
    }
}
