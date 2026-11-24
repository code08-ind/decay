//! Forward-only schema migrations (DB Design §6): SQL files embedded in the binary, each
//! applied in its own transaction and recorded in `schema_migrations` with a checksum.
//! The caller takes a backup first (a down-migration is a restore from it).

use rusqlite::{Connection, OptionalExtension, params};

use crate::StoreError;

pub(crate) struct Migration {
    pub version: u32,
    pub name: &'static str,
    pub sql: &'static str,
}

pub(crate) const MIGRATIONS: &[Migration] = &[
    Migration { version: 1, name: "initial", sql: include_str!("../migrations/V1__initial.sql") },
    Migration {
        version: 2,
        name: "onboarded",
        sql: include_str!("../migrations/V2__onboarded.sql"),
    },
    Migration { version: 3, name: "metrics", sql: include_str!("../migrations/V3__metrics.sql") },
    Migration { version: 4, name: "profiles", sql: include_str!("../migrations/V4__profiles.sql") },
    Migration { version: 5, name: "license", sql: include_str!("../migrations/V5__license.sql") },
];

/// The newest schema version this build knows.
pub const SCHEMA_VERSION: u32 = 5;

/// FNV-1a over the SQL with line endings normalised (a Windows checkout has CRLF).
fn checksum(sql: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for b in sql.bytes().filter(|&b| b != b'\r') {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// The migrations not applied yet. Fails if an applied one changed or the database is
/// newer than this build.
pub(crate) fn pending(conn: &Connection) -> Result<Vec<&'static Migration>, StoreError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
           version    INTEGER PRIMARY KEY,
           name       TEXT NOT NULL,
           applied_on INTEGER NOT NULL,
           checksum   TEXT NOT NULL
         );",
    )?;
    let newest: Option<u32> =
        conn.query_row("SELECT max(version) FROM schema_migrations", [], |r| r.get(0))?;
    if let Some(found) = newest.filter(|&v| v > SCHEMA_VERSION) {
        return Err(StoreError::TooNew { found, known: SCHEMA_VERSION });
    }
    let mut out = Vec::new();
    for m in MIGRATIONS {
        let stored: Option<String> = conn
            .query_row(
                "SELECT checksum FROM schema_migrations WHERE version = ?1",
                [m.version],
                |r| r.get(0),
            )
            .optional()?;
        match stored {
            Some(sum) if sum != checksum(m.sql) => {
                return Err(StoreError::MigrationChanged { version: m.version });
            }
            Some(_) => {}
            None => out.push(m),
        }
    }
    Ok(out)
}

/// The newest applied version (0 if none).
pub(crate) fn current_version(conn: &Connection) -> Result<u32, StoreError> {
    let has_table: bool = conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE name = 'schema_migrations')",
        [],
        |r| r.get(0),
    )?;
    if !has_table {
        return Ok(0);
    }
    Ok(conn
        .query_row("SELECT coalesce(max(version), 0) FROM schema_migrations", [], |r| r.get(0))?)
}

/// SQLite's message without the SQL text rusqlite appends to batch errors.
fn short_reason(err: &rusqlite::Error) -> String {
    let text = match err {
        rusqlite::Error::SqliteFailure(_, Some(msg)) => msg.clone(),
        other => other.to_string(),
    };
    let line = text.split(" in ").next().unwrap_or(&text).lines().next().unwrap_or("");
    line.chars().take(200).collect()
}

/// Applies migrations in order, each atomically with its `schema_migrations` row.
pub(crate) fn apply(
    conn: &mut Connection,
    migrations: &[&Migration],
    now: i64,
) -> Result<Vec<u32>, StoreError> {
    let mut applied = Vec::new();
    for m in migrations {
        let tx = conn.transaction()?;
        tx.execute_batch(m.sql)
            .map_err(|e| StoreError::Migration { version: m.version, reason: short_reason(&e) })?;
        tx.execute(
            "INSERT INTO schema_migrations (version, name, applied_on, checksum)
             VALUES (?1, ?2, ?3, ?4)",
            params![m.version, m.name, now, checksum(m.sql)],
        )?;
        tx.commit()?;
        applied.push(m.version);
    }
    Ok(applied)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksums_ignore_line_endings() {
        assert_eq!(checksum("a\r\nb"), checksum("a\nb"));
        assert_ne!(checksum("a\nb"), checksum("a\nc"));
    }

    /// V2 marks databases already in use as onboarded, and only those.
    #[test]
    fn databases_in_use_before_onboarding_count_as_onboarded() {
        let onboarded = |seed: &str| {
            let mut conn = Connection::open_in_memory().unwrap();
            let v1 =
                pending(&conn).unwrap().into_iter().filter(|m| m.version == 1).collect::<Vec<_>>();
            apply(&mut conn, &v1, 5).unwrap();
            conn.execute_batch(seed).unwrap();
            let rest = pending(&conn).unwrap();
            assert_eq!(rest.iter().map(|m| m.version).collect::<Vec<_>>(), [2, 3, 4, 5]);
            apply(&mut conn, &rest, 5).unwrap();
            let at: Option<String> = conn
                .query_row("SELECT value FROM settings WHERE key = 'onboarded_at'", [], |r| {
                    r.get(0)
                })
                .optional()
                .unwrap();
            at.map(|v| v.parse::<i64>().unwrap())
        };
        assert_eq!(onboarded(""), None, "a new database isn't onboarded");
        let at = onboarded(
            "INSERT INTO sweeps (id, started_at, trigger, is_shadow, status) VALUES ('s', 1, 'manual', 1, 'done');",
        );
        assert!(at.is_some_and(|t| t > 1_700_000_000_000), "{at:?}");
    }

    /// V4 lists the profiles already known from their windows, first sighting kept.
    #[test]
    fn profiles_come_from_known_windows() {
        let mut conn = Connection::open_in_memory().unwrap();
        crate::words::register(&conn).unwrap();
        let v123 =
            pending(&conn).unwrap().into_iter().filter(|m| m.version < 4).collect::<Vec<_>>();
        apply(&mut conn, &v123, 5).unwrap();
        conn.execute_batch(
            "INSERT INTO apps (id, name, platform, kind) VALUES ('c', 'Chrome', 'macos', 'browser');
             INSERT INTO sources (id, app_id, profile, first_seen_at, last_seen_at) VALUES
               ('s1', 'c', 'p1', 30, 40), ('s2', 'c', 'p1', 10, 90),
               ('s3', 'c', 'p2', 20, 20), ('s4', 'c', NULL, 1, 1), ('s5', 'c', '', 1, 1);",
        )
        .unwrap();
        let rest = pending(&conn).unwrap();
        apply(&mut conn, &rest, 5).unwrap();
        let rows: Vec<(String, i64, i64)> = conn
            .prepare("SELECT profile, first_seen_at, last_seen_at FROM profiles ORDER BY profile")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(rows, [("p1".into(), 10, 90), ("p2".into(), 20, 20)]);
    }

    /// V3 copies what's already recorded into `metric_events`, and its triggers count
    /// what comes next: restores within 24 h, an Undo once per tab sweep, never a
    /// window-hide sweep's Undo.
    #[test]
    fn metrics_copy_the_history_and_count_what_follows() {
        const H: i64 = 3_600_000;
        let mut conn = Connection::open_in_memory().unwrap();
        crate::words::register(&conn).unwrap();
        let v12 = pending(&conn).unwrap().into_iter().filter(|m| m.version < 3).collect::<Vec<_>>();
        apply(&mut conn, &v12, 5).unwrap();
        let mut seed = String::from(
            "INSERT INTO sweeps (id, started_at, trigger, status, reason) VALUES
               ('tabs', 1, 'manual', 'undone', NULL),
               ('hide', 1, 'manual', 'undone', '{\"kind\":\"hide\"}'),
               ('tabs2', 1, 'manual', 'done', NULL);",
        );
        for i in 1..=8 {
            use std::fmt::Write as _;
            let _ = write!(
                seed,
                "INSERT INTO items (id, kind, app_id, state, first_seen_at, last_used_at,
                   state_changed_at) VALUES ('i{i}', 'tab', 'com.google.Chrome', 'archived', 0, 0, 0);"
            );
        }
        let ev = |item: u8, ty: &str, sweep: &str, at: i64| {
            let sweep = if sweep.is_empty() { "NULL".into() } else { format!("'{sweep}'") };
            format!(
                "INSERT INTO events (item_id, type, sweep_id, at) VALUES ('i{item}', '{ty}', {sweep}, {at});"
            )
        };
        for e in [
            ev(1, "archived", "", 10 * H),
            ev(1, "restored", "", 11 * H), // within a day
            ev(2, "archived", "", 0),
            ev(2, "restored", "", 30 * H), // too late
            ev(3, "archived", "tabs", 12 * H),
            ev(4, "archived", "tabs", 12 * H),
            ev(3, "undo", "tabs", 13 * H), // one Undo of two tabs
            ev(4, "undo", "tabs", 13 * H),
            ev(5, "hidden", "hide", 12 * H),
            ev(5, "undo", "hide", 13 * H), // a window shown again: not counted
            ev(6, "shadow_would_archive", "", 14 * H),
        ] {
            seed.push_str(&e);
        }
        conn.execute_batch(&seed).unwrap();
        let rest = pending(&conn).unwrap();
        apply(&mut conn, &rest, 5).unwrap();
        let counted = |conn: &Connection| -> Vec<(String, i64)> {
            let mut stmt = conn
                .prepare("SELECT kind, count(*) FROM metric_events GROUP BY kind ORDER BY kind")
                .unwrap();
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                .unwrap()
                .collect::<rusqlite::Result<_>>()
                .unwrap()
        };
        let c = |k: &str, n: i64| (k.to_owned(), n);
        assert_eq!(
            counted(&conn),
            [c("archived", 4), c("restored_24h", 3), c("shadow_flagged", 1), c("undo", 1)]
        );
        let undo_at: i64 = conn
            .query_row(
                "SELECT at FROM metric_events WHERE kind = 'undo' AND ref = 'tabs'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(undo_at, 13 * H);
        // From now on, the triggers.
        let mut live = String::new();
        for e in [
            ev(7, "hidden", "hide", 20 * H),
            ev(7, "undo", "hide", 21 * H), // a hide sweep: nothing
            ev(8, "archived", "tabs2", 20 * H),
            ev(8, "undo", "tabs2", 21 * H), // archived, restored within a day, one Undo
        ] {
            live.push_str(&e);
        }
        conn.execute_batch(&live).unwrap();
        assert_eq!(
            counted(&conn),
            [c("archived", 5), c("restored_24h", 4), c("shadow_flagged", 1), c("undo", 2)]
        );
    }

    #[test]
    fn versions_are_sequential() {
        for (i, m) in MIGRATIONS.iter().enumerate() {
            assert_eq!(m.version as usize, i + 1);
        }
        assert_eq!(MIGRATIONS.last().map(|m| m.version), Some(SCHEMA_VERSION));
    }
}
