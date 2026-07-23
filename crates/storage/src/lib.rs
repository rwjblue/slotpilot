//! Versioned SQLite operational storage.
//!
//! Schema version 2 adds bounded receive-window, decode-classification, and
//! diagnostic evidence to the Phase 0 command/event/outbox foundation. It
//! deliberately contains no raw continuous PCM, bulk waterfall rows, final
//! QSO, WSPR, ADIF, live arm token, transmit authority, or resumable PTT state.

use std::{path::Path, time::Duration};

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use slotpilot_domain::{CommandId, EventId, RequestId, ServiceInstanceId};
use thiserror::Error;

mod receive;

pub use receive::{
    MAX_RECEIVE_PAGE_SIZE, MAX_STORED_DECODES_PER_WINDOW, ReceiveClockFault, ReceiveClockHealth,
    ReceiveDiagnosticSummary, ReceiveEventCommit, ReceiveInsertOutcome, ReceivePage, ReceiveRecord,
    ReceiveWindowContext, SequencedReceiveRecord,
};

const SCHEMA_VERSION: u32 = 2;
const MIGRATION_1: &str = include_str!("../migrations/0001_phase0.sql");
const MIGRATION_2: &str = include_str!("../migrations/0002_phase2_receive.sql");

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
    /// A SQLite integer could not represent the owned unsigned value.
    #[error("persisted integer is outside the supported range")]
    InvalidSequence,
    /// A receive record violated a bounded cross-field invariant before insert.
    #[error("invalid receive record: {0}")]
    InvalidReceiveRecord(&'static str),
    /// A persisted receive value could not be reconstructed as an owned type.
    #[error("invalid persisted receive value: {0}")]
    InvalidPersistedReceiveValue(&'static str),
    /// A receive query requested an unbounded or empty page.
    #[error("receive page limit must be between 1 and {MAX_RECEIVE_PAGE_SIZE}")]
    InvalidPageLimit,
    /// A daemon-supplied public receive event could not be encoded.
    #[error("receive event payload is invalid")]
    InvalidReceiveEventPayload,
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

/// Storage-owned representation of one operational event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredEvent {
    /// Global monotonically increasing SQLite sequence.
    pub sequence: u64,
    /// Stable event identity.
    pub event_id: EventId,
    /// Daemon generation that emitted the event.
    pub service_instance_id: ServiceInstanceId,
    /// SlotPilot API payload JSON.
    pub event_json: String,
    /// Occurrence time in UTC milliseconds since the Unix epoch.
    pub occurred_utc_millis: i64,
}

/// Bounded retained history for one service generation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplayWindow {
    /// Ordered page after the requested sequence.
    pub events: Vec<StoredEvent>,
    /// Earliest retained sequence for this generation.
    pub earliest_sequence: Option<u64>,
    /// Latest retained sequence for this generation.
    pub latest_sequence: Option<u64>,
    /// Whether another event exists beyond this page.
    pub has_more: bool,
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
        if found < 2 {
            transaction.execute_batch(MIGRATION_2)?;
            transaction.pragma_update(None, "user_version", 2)?;
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

    /// Appends one validated JSON event and returns its assigned sequence.
    pub fn append_event(
        &mut self,
        event_id: &EventId,
        service_instance_id: &ServiceInstanceId,
        event_json: &str,
        occurred_utc_millis: i64,
    ) -> Result<u64, StorageError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            "INSERT INTO operational_events (
                event_id, service_instance_id, event_json, occurred_utc_millis
             ) VALUES (?1, ?2, ?3, ?4)",
            params![
                event_id.as_str(),
                service_instance_id.as_str(),
                event_json,
                occurred_utc_millis
            ],
        )?;
        let sequence = u64::try_from(transaction.last_insert_rowid())
            .map_err(|_| StorageError::InvalidSequence)?;
        transaction.commit()?;
        Ok(sequence)
    }

    /// Reads at most `limit` ordered events after a cursor for one generation.
    pub fn replay_events(
        &self,
        service_instance_id: &ServiceInstanceId,
        after_sequence: u64,
        limit: usize,
    ) -> Result<ReplayWindow, StorageError> {
        let (earliest, latest): (Option<i64>, Option<i64>) = self.connection.query_row(
            "SELECT MIN(sequence), MAX(sequence)
             FROM operational_events
             WHERE service_instance_id = ?1",
            [service_instance_id.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let earliest_sequence = earliest.map(sequence_from_i64).transpose()?;
        let latest_sequence = latest.map(sequence_from_i64).transpose()?;
        let after = i64::try_from(after_sequence).map_err(|_| StorageError::InvalidSequence)?;
        let fetch_limit =
            i64::try_from(limit.saturating_add(1)).map_err(|_| StorageError::InvalidSequence)?;
        let mut statement = self.connection.prepare(
            "SELECT sequence, event_id, service_instance_id, event_json, occurred_utc_millis
             FROM operational_events
             WHERE service_instance_id = ?1 AND sequence > ?2
             ORDER BY sequence
             LIMIT ?3",
        )?;
        let rows = statement.query_map(
            params![service_instance_id.as_str(), after, fetch_limit],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            },
        )?;
        let mut events = Vec::new();
        for row in rows {
            let (sequence, event_id, instance_id, event_json, occurred_utc_millis) = row?;
            events.push(StoredEvent {
                sequence: sequence_from_i64(sequence)?,
                event_id: event_id.parse()?,
                service_instance_id: instance_id.parse()?,
                event_json,
                occurred_utc_millis,
            });
        }
        let has_more = events.len() > limit;
        events.truncate(limit);
        Ok(ReplayWindow {
            events,
            earliest_sequence,
            latest_sequence,
            has_more,
        })
    }

    /// Deletes events older than `first_retained_sequence`.
    ///
    /// This is the explicit retention primitive; callers remain responsible
    /// for choosing a bounded policy.
    pub fn prune_events_before(
        &mut self,
        first_retained_sequence: u64,
    ) -> Result<usize, StorageError> {
        let sequence =
            i64::try_from(first_retained_sequence).map_err(|_| StorageError::InvalidSequence)?;
        Ok(self.connection.execute(
            "DELETE FROM operational_events WHERE sequence < ?1",
            [sequence],
        )?)
    }
}

fn sequence_from_i64(sequence: i64) -> Result<u64, StorageError> {
    u64::try_from(sequence).map_err(|_| StorageError::InvalidSequence)
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

    fn event(id: &str) -> EventId {
        id.parse().unwrap()
    }

    fn instance() -> ServiceInstanceId {
        "svc_01jabcde9".parse().unwrap()
    }

    #[test]
    fn clean_creation_reaches_schema_version_two() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(store.schema_version().unwrap(), 2);
    }

    #[test]
    fn schema_version_one_migrates_forward_with_receive_constraints_and_indexes() {
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch(MIGRATION_1).unwrap();
        connection.pragma_update(None, "user_version", 1).unwrap();
        let store = Store::from_connection(connection).unwrap();
        assert_eq!(store.schema_version().unwrap(), 2);
        let schema: String = store
            .connection
            .query_row(
                "SELECT group_concat(sql, '\n') FROM sqlite_schema WHERE sql IS NOT NULL",
                [],
                |row| row.get(0),
            )
            .unwrap();
        for required in [
            "receive_windows",
            "receive_diagnostics",
            "receive_decodes",
            "receive_windows_slot_order",
            "receive_windows_service_order",
            "receive_decodes_deterministic_order",
            "ON DELETE CASCADE",
            "outcome_kind IN",
        ] {
            assert!(schema.contains(required), "{required}");
        }
        assert!(
            store
                .connection
                .execute(
                    "INSERT INTO receive_windows (
                        receive_window_id, service_instance_id,
                        process_generation, stream_generation,
                        slot_start_utc_millis, device_platform, device_opaque_id,
                        sample_rate_hz, channels, sample_format, selected_channel,
                        capture_position_frames, capture_utc_millis,
                        capture_monotonic_millis, recorded_utc_millis
                     ) VALUES (
                        'rxw_01jabcde!', 'svc_01jabcde9',
                        1, 1, 30000, 'macos_core_audio', 'device-1',
                        48000, 1, 'signed_16', 0,
                        0, 30000, 1000, 40000
                     )",
                    [],
                )
                .is_err()
        );
    }

    #[test]
    fn event_replay_is_ordered_bounded_and_reports_retained_window() {
        let mut store = Store::open_in_memory().unwrap();
        for (id, marker) in [
            ("evt_01jabcde9", "one"),
            ("evt_01jabcdf0", "two"),
            ("evt_01jabcdf1", "three"),
        ] {
            store
                .append_event(
                    &event(id),
                    &instance(),
                    &format!(r#"{{"kind":"phase0_notice","message":"{marker}"}}"#),
                    10,
                )
                .unwrap();
        }
        let page = store.replay_events(&instance(), 0, 2).unwrap();
        assert_eq!(page.events.len(), 2);
        assert!(page.events[0].sequence < page.events[1].sequence);
        assert_eq!(page.earliest_sequence, Some(1));
        assert_eq!(page.latest_sequence, Some(3));
        assert!(page.has_more);
    }

    #[test]
    fn event_retention_and_persistence_failures_are_explicit() {
        let mut store = Store::open_in_memory().unwrap();
        let event_id = event("evt_01jabcde9");
        let sequence = store
            .append_event(
                &event_id,
                &instance(),
                r#"{"kind":"phase0_notice","message":"one"}"#,
                10,
            )
            .unwrap();
        assert!(
            store
                .append_event(&event_id, &instance(), "{}", 11)
                .is_err()
        );
        assert_eq!(store.prune_events_before(sequence + 1).unwrap(), 1);
        assert!(
            store
                .replay_events(&instance(), 0, 10)
                .unwrap()
                .events
                .is_empty()
        );
        assert!(
            store
                .append_event(&event("evt_01jabcdf0"), &instance(), "not json", 12)
                .is_err()
        );
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
            "receive_windows",
            "receive_diagnostics",
            "receive_decodes",
        ] {
            assert!(schema.contains(required));
        }
        for forbidden in [
            "arm_token",
            "transmit_authority",
            "resumable_ptt",
            "raw_pcm",
            "waterfall_rows",
        ] {
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
                supported: 2
            })
        ));
    }
}
