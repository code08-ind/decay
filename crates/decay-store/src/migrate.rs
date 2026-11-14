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
];

/// The newest schema version this build knows.
pub const SCHEMA_VERSION: u32 = 2;

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
            assert_eq!(rest.iter().map(|m| m.version).collect::<Vec<_>>(), [2]);
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

    #[test]
    fn versions_are_sequential() {
        for (i, m) in MIGRATIONS.iter().enumerate() {
            assert_eq!(m.version as usize, i + 1);
        }
        assert_eq!(MIGRATIONS.last().map(|m| m.version), Some(SCHEMA_VERSION));
    }
}
