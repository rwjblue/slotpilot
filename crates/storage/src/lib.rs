//! Versioned SQLite operational storage.
//!
//! Schema version 1 represents accepted command identities/results, immutable
//! profile/session context, ordered events, generic outbox work, and external
//! receipts. It deliberately contains no final QSO, WSPR, ADIF, upload, live
//! arm token, transmit authority, or resumable PTT state.

use std::{path::Path, time::Duration};

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use slotpilot_domain::{CommandId, RequestId};
use thiserror::Error;

const SCHEMA_VERSION: u32 = 1;
const MIGRATION_1: &str = include_str!("../migrations/0001_phase0.sql");

/// Typed storage-boundary failure.
#[derive(Debug, Error)]
pub enum StorageError {
    /// SQLite returned a database or constraint error.
    #[error("SQLite storage failure: {0}")]
    Database(#[from] rusqlite::Error),
    /// The database was created by a newer unsupported SlotPilot version.
    #[error("database schema version {found} is newer than supported version {supported}")]
    SchemaTooNew {
        /// Version found in `PRAGMA user_version`.
        found: u32,
        /// Highest version this crate understands.
        supported: u32,
    },
    /// A persisted typed identity failed validation.
    #[error("invalid persisted identity: {0}")]
    InvalidIdentity(#[from] slotpilot_domain::IdError),
    /// A non-request unique identity collided without a matching request row.
    #[error("durable identity collision")]
    IdentityCollision,
}

/// Original accepted command identity and result recovered for safe retry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptedCommand {
    /// Client-supplied stable request identity.
    pub request_id: RequestId,
    /// Service-assigned accepted command identity.
    pub command_id: CommandId,
    /// Canonical command bytes used for exact identity comparison.
    pub canonical_command: Vec<u8>,
    /// Original bounded result bytes returned on replay.
    pub original_result: Vec<u8>,
    /// Acceptance time in UTC milliseconds since the Unix epoch.
    pub accepted_utc_millis: i64,
}

/// Atomic outcome of attempting to accept a request identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AcceptOutcome {
    /// This call inserted the supplied accepted command.
    Inserted(AcceptedCommand),
    /// Another call had already accepted the request identity.
    Existing(AcceptedCommand),
}

/// SQLite store with migrations applied on open.
pub struct Store {
    connection: Connection,
}

impl Store {
    /// Opens or creates a file database and applies forward migrations.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StorageError> {
        Self::from_connection(Connection::open(path)?)
    }

    /// Opens an isolated in-memory database and applies forward migrations.
    pub fn open_in_memory() -> Result<Self, StorageError> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    fn from_connection(connection: Connection) -> Result<Self, StorageError> {
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        let mut store = Self { connection };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&mut self) -> Result<(), StorageError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let found: u32 = transaction.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if found > SCHEMA_VERSION {
            return Err(StorageError::SchemaTooNew {
                found,
                supported: SCHEMA_VERSION,
            });
        }
        if found < 1 {
            transaction.execute_batch(MIGRATION_1)?;
            transaction.pragma_update(None, "user_version", 1)?;
        }
        transaction.commit()?;
        Ok(())
    }

    /// Atomically records an accepted request identity and original result.
    pub fn record_accepted_command(
        &mut self,
        command: &AcceptedCommand,
    ) -> Result<(), StorageError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            "INSERT INTO accepted_commands (
                request_id,
                command_id,
                canonical_command,
                original_result,
                accepted_utc_millis
             ) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                command.request_id.as_str(),
                command.command_id.as_str(),
                command.canonical_command,
                command.original_result,
                command.accepted_utc_millis
            ],
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// Recovers an accepted command and its original result by request ID.
    pub fn accepted_command(
        &self,
        request_id: &RequestId,
    ) -> Result<Option<AcceptedCommand>, StorageError> {
        read_accepted_command(&self.connection, request_id)
    }

    /// Atomically inserts a command or returns the winner for its request ID.
    ///
    /// `INSERT OR IGNORE` and the subsequent read share one immediate
    /// transaction, making concurrent same-ID acceptance deterministic.
    pub fn accept_or_existing(
        &mut self,
        command: &AcceptedCommand,
    ) -> Result<AcceptOutcome, StorageError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let inserted = transaction.execute(
            "INSERT OR IGNORE INTO accepted_commands (
                request_id,
                command_id,
                canonical_command,
                original_result,
                accepted_utc_millis
             ) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                command.request_id.as_str(),
                command.command_id.as_str(),
                command.canonical_command,
                command.original_result,
                command.accepted_utc_millis
            ],
        )?;
        let outcome = if inserted == 1 {
            AcceptOutcome::Inserted(command.clone())
        } else {
            let existing = read_accepted_command(&transaction, &command.request_id)?
                .ok_or(StorageError::IdentityCollision)?;
            AcceptOutcome::Existing(existing)
        };
        transaction.commit()?;
        Ok(outcome)
    }

    /// Returns the migrated schema version.
    pub fn schema_version(&self) -> Result<u32, StorageError> {
        Ok(self
            .connection
            .pragma_query_value(None, "user_version", |row| row.get(0))?)
    }
}

fn read_accepted_command(
    connection: &Connection,
    request_id: &RequestId,
) -> Result<Option<AcceptedCommand>, StorageError> {
    let row = connection
        .query_row(
            "SELECT
                    request_id,
                    command_id,
                    canonical_command,
                    original_result,
                    accepted_utc_millis
                 FROM accepted_commands
                 WHERE request_id = ?1",
            [request_id.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Vec<u8>>(2)?,
                    row.get::<_, Vec<u8>>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            },
        )
        .optional()?;
    row.map(
        |(request_id, command_id, canonical_command, original_result, accepted_utc_millis)| {
            Ok(AcceptedCommand {
                request_id: request_id.parse()?,
                command_id: command_id.parse()?,
                canonical_command,
                original_result,
                accepted_utc_millis,
            })
        },
    )
    .transpose()
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        sync::atomic::{AtomicU64, Ordering},
    };

    use super::*;

    static DATABASE_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temp_database() -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "slotpilot-storage-{}-{}.sqlite3",
            std::process::id(),
            DATABASE_COUNTER.fetch_add(1, Ordering::Relaxed)
        ))
    }

    fn accepted() -> AcceptedCommand {
        AcceptedCommand {
            request_id: "req_01jabcde9".parse().unwrap(),
            command_id: "cmd_01jabcde9".parse().unwrap(),
            canonical_command: br#"{"kind":"future_mutation"}"#.to_vec(),
            original_result: br#"{"outcome":"accepted"}"#.to_vec(),
            accepted_utc_millis: 1_721_798_400_000,
        }
    }

    #[test]
    fn clean_creation_and_forward_migration_reach_version_one() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(store.schema_version().unwrap(), 1);
    }

    #[test]
    fn accepted_identity_and_result_survive_reopen() {
        let path = temp_database();
        {
            let mut store = Store::open(&path).unwrap();
            store.record_accepted_command(&accepted()).unwrap();
        }
        {
            let store = Store::open(&path).unwrap();
            assert_eq!(
                store
                    .accepted_command(&accepted().request_id)
                    .unwrap()
                    .unwrap(),
                accepted()
            );
        }
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn request_and_command_identity_constraints_reject_duplicates() {
        let mut store = Store::open_in_memory().unwrap();
        store.record_accepted_command(&accepted()).unwrap();
        assert!(matches!(
            store.record_accepted_command(&accepted()),
            Err(StorageError::Database(rusqlite::Error::SqliteFailure(_, _)))
        ));
        let mut same_command = accepted();
        same_command.request_id = "req_01jabcdf0".parse().unwrap();
        assert!(store.record_accepted_command(&same_command).is_err());
    }

    #[test]
    fn session_snapshots_are_immutable() {
        let store = Store::open_in_memory().unwrap();
        store
            .connection
            .execute(
                "INSERT INTO session_context_snapshots
                 (session_id, resolved_context_json, created_utc_millis)
                 VALUES ('ses_01jabcde9', '{}', 1)",
                [],
            )
            .unwrap();
        assert!(
            store
                .connection
                .execute(
                    "UPDATE session_context_snapshots
                 SET resolved_context_json = '{\"changed\":true}'
                 WHERE session_id = 'ses_01jabcde9'",
                    [],
                )
                .is_err()
        );
    }

    #[test]
    fn uncommitted_data_does_not_survive_reopen() {
        let path = temp_database();
        {
            let mut store = Store::open(&path).unwrap();
            let transaction = store.connection.transaction().unwrap();
            transaction
                .execute(
                    "INSERT INTO profile_revisions (
                        profile_revision_id, profile_id, profile_kind, revision,
                        document_json, created_utc_millis
                     ) VALUES ('prv_01jabcde9', 'profile-one', 'station', 1, '{}', 1)",
                    [],
                )
                .unwrap();
        }
        {
            let store = Store::open(&path).unwrap();
            let count: u32 = store
                .connection
                .query_row("SELECT COUNT(*) FROM profile_revisions", [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(count, 0);
        }
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn schema_represents_events_outbox_receipts_and_no_transmit_authority() {
        let store = Store::open_in_memory().unwrap();
        let schema: String = store
            .connection
            .query_row(
                "SELECT group_concat(sql, '\n') FROM sqlite_schema WHERE sql IS NOT NULL",
                [],
                |row| row.get(0),
            )
            .unwrap();
        for required in [
            "operational_events",
            "profile_revisions",
            "session_context_snapshots",
            "outbox_work",
            "external_receipts",
        ] {
            assert!(schema.contains(required));
        }
        for forbidden in ["arm_token", "transmit_authority", "resumable_ptt"] {
            assert!(!schema.to_ascii_lowercase().contains(forbidden));
        }
    }

    #[test]
    fn future_schema_fails_closed() {
        let connection = Connection::open_in_memory().unwrap();
        connection.pragma_update(None, "user_version", 99).unwrap();
        assert!(matches!(
            Store::from_connection(connection),
            Err(StorageError::SchemaTooNew {
                found: 99,
                supported: 1
            })
        ));
    }
}
