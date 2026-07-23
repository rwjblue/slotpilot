//! Typed schema-v2 receive evidence and bounded query API.

use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use slotpilot_audio::{
    CapturePosition, CaptureTimeEvidence, Ft8ReceiveSlot, InputConfiguration, InputDeviceIdentity,
    InputHealth, InputPlatform, InputSampleFormat, ProcessGeneration, ReceiveTimelineHealth,
    StreamGeneration,
};
use slotpilot_domain::{EventId, ReceiveWindowId, ServiceInstanceId};
use slotpilot_protocol::{
    AmbiguousFt8Message, ClassifiedFt8Message, FreeTextFt8Message, Ft8Decode, Ft8DecodeMetadata,
    Ft8MessageClass, ResolvedFt8Message, UnresolvedHashFt8Message, UnsupportedFt8Message,
};

use crate::{StorageError, Store, sequence_from_i64};

/// Maximum decodes retained for one receive window.
pub const MAX_STORED_DECODES_PER_WINDOW: usize = 128;
/// Maximum records returned by one receive-history page.
pub const MAX_RECEIVE_PAGE_SIZE: usize = 100;
const MAX_CLOCK_MAPPING_AGE_MILLIS: u64 = 86_400_000;

/// Storage-owned clock fault representation independent of operations internals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiveClockFault {
    /// A sample came from another process generation.
    ProcessGenerationChanged,
    /// UTC or monotonic evidence moved backwards.
    TimelineRegressed,
    /// UTC progress diverged from monotonic progress.
    UtcJump {
        /// Signed UTC-minus-monotonic divergence.
        divergence_millis: i64,
    },
    /// The latest mapping exceeded its freshness bound.
    StaleMapping {
        /// Mapping age at failure.
        age_millis: u64,
    },
    /// A sample reached the monitor too late.
    SamplingDelayed {
        /// Observed sampling delay.
        delay_millis: u64,
    },
    /// A suspend/resume-like sample gap exceeded policy.
    SampleGap {
        /// Observed monotonic gap.
        gap_millis: u64,
    },
    /// Capture and independent clock mapping disagreed.
    WindowMisaligned {
        /// Signed capture-minus-clock divergence.
        divergence_millis: i64,
    },
    /// Checked clock arithmetic overflowed.
    ArithmeticOverflow,
}

/// Bounded durable receive-clock summary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiveClockHealth {
    /// Mapping was healthy when evidence was recorded.
    Healthy {
        /// Age of the accepted mapping.
        mapping_age_millis: u64,
    },
    /// Mapping was latched unhealthy with visible recovery progress.
    Unhealthy {
        /// Exact owned failure.
        fault: ReceiveClockFault,
        /// Consecutive consistent recovery observations.
        recovery_progress: u8,
        /// Required observations for recovery.
        recovery_required: u8,
        /// Age of the last accepted mapping.
        mapping_age_millis: u64,
    },
}

impl ReceiveClockHealth {
    fn validate(self) -> Result<(), StorageError> {
        match self {
            Self::Healthy { mapping_age_millis }
                if mapping_age_millis <= MAX_CLOCK_MAPPING_AGE_MILLIS =>
            {
                Ok(())
            }
            Self::Unhealthy {
                recovery_progress,
                recovery_required,
                mapping_age_millis,
                ..
            } if (2..=10).contains(&recovery_required)
                && recovery_progress < recovery_required
                && mapping_age_millis <= MAX_CLOCK_MAPPING_AGE_MILLIS =>
            {
                Ok(())
            }
            _ => Err(StorageError::InvalidReceiveRecord(
                "clock health or recovery bounds are invalid",
            )),
        }
    }

    const fn is_healthy(self) -> bool {
        matches!(self, Self::Healthy { .. })
    }
}

/// Audio, timeline, and clock summaries stored atomically with one window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReceiveDiagnosticSummary {
    /// Bounded capture health.
    pub audio: InputHealth,
    /// Bounded canonical timeline health.
    pub timeline: ReceiveTimelineHealth,
    /// Bounded receive-clock state.
    pub clock: ReceiveClockHealth,
}

/// Stable durable context for one exact receive window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceiveWindowContext {
    /// Idempotent receive-window identity.
    pub receive_window_id: ReceiveWindowId,
    /// Running daemon generation.
    pub service_instance_id: ServiceInstanceId,
    /// Capture process generation.
    pub process_generation: ProcessGeneration,
    /// Exact input stream/device generation.
    pub stream_generation: StreamGeneration,
    /// Exact typed FT8 slot.
    pub slot: Ft8ReceiveSlot,
    /// Stable configured platform device identity.
    pub device_identity: InputDeviceIdentity,
    /// Exact selected device configuration.
    pub configuration: InputConfiguration,
    /// Source-position and UTC/monotonic capture mapping.
    pub capture_mapping: CaptureTimeEvidence,
}

/// One atomic receive window, diagnostic summary, and ordered decode set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceiveRecord {
    context: ReceiveWindowContext,
    diagnostics: ReceiveDiagnosticSummary,
    decodes: Vec<Ft8Decode>,
    recorded_utc_millis: i64,
}

impl ReceiveRecord {
    /// Constructs and deterministically orders one bounded durable record.
    pub fn new(
        context: ReceiveWindowContext,
        diagnostics: ReceiveDiagnosticSummary,
        mut decodes: Vec<Ft8Decode>,
        recorded_utc_millis: i64,
    ) -> Result<Self, StorageError> {
        if context.capture_mapping.utc_unix_millis != context.slot.start_utc_unix_millis() {
            return Err(StorageError::InvalidReceiveRecord(
                "capture mapping must begin at the declared FT8 slot",
            ));
        }
        if recorded_utc_millis < 0 {
            return Err(StorageError::InvalidReceiveRecord(
                "recorded UTC time must be nonnegative",
            ));
        }
        if decodes.len() > MAX_STORED_DECODES_PER_WINDOW {
            return Err(StorageError::InvalidReceiveRecord(
                "decode count exceeds the bounded window limit",
            ));
        }
        if !(-100_000..=100_000).contains(&diagnostics.timeline.drift_parts_per_million) {
            return Err(StorageError::InvalidReceiveRecord(
                "timeline drift is outside durable bounds",
            ));
        }
        diagnostics.clock.validate()?;
        for decode in &decodes {
            validate_decode_metadata(decode.metadata)?;
        }
        if !diagnostics.clock.is_healthy() && !decodes.is_empty() {
            return Err(StorageError::InvalidReceiveRecord(
                "unhealthy clock evidence cannot carry decoder-ready results",
            ));
        }
        Ft8Decode::sort_deterministically(&mut decodes);
        Ok(Self {
            context,
            diagnostics,
            decodes,
            recorded_utc_millis,
        })
    }

    /// Returns stable window/capture identity.
    #[must_use]
    pub const fn context(&self) -> &ReceiveWindowContext {
        &self.context
    }

    /// Returns stored diagnostic evidence.
    #[must_use]
    pub const fn diagnostics(&self) -> ReceiveDiagnosticSummary {
        self.diagnostics
    }

    /// Returns deterministic ordered decodes.
    #[must_use]
    pub fn decodes(&self) -> &[Ft8Decode] {
        &self.decodes
    }

    /// Returns persistence time in UTC milliseconds.
    #[must_use]
    pub const fn recorded_utc_millis(&self) -> i64 {
        self.recorded_utc_millis
    }
}

/// Result of idempotently recording one receive identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiveInsertOutcome {
    /// The supplied record was inserted.
    Inserted {
        /// Assigned global receive sequence.
        sequence: u64,
    },
    /// The exact same record already existed.
    Existing {
        /// Existing global receive sequence.
        sequence: u64,
    },
}

/// Atomic durable receive plus public-event publication outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReceiveEventCommit {
    /// Idempotent receive insert result.
    pub receive: ReceiveInsertOutcome,
    /// Ordered operational-event sequence.
    pub event_sequence: u64,
}

/// One receive record with its global pagination sequence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SequencedReceiveRecord {
    /// Global monotonically increasing receive sequence.
    pub sequence: u64,
    /// Exact reconstructed owned record.
    pub record: ReceiveRecord,
}

/// Bounded deterministic receive-history page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceivePage {
    /// Ordered records after the requested cursor.
    pub records: Vec<SequencedReceiveRecord>,
    /// Earliest retained receive sequence.
    pub earliest_sequence: Option<u64>,
    /// Latest retained receive sequence.
    pub latest_sequence: Option<u64>,
    /// Whether another record exists beyond this page.
    pub has_more: bool,
}

impl Store {
    /// Atomically inserts a receive record or returns the exact existing retry.
    pub fn record_receive(
        &mut self,
        record: &ReceiveRecord,
    ) -> Result<ReceiveInsertOutcome, StorageError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(sequence) = find_existing_sequence(&transaction, &record.context)? {
            let existing = read_receive_by_sequence(&transaction, sequence)?.ok_or(
                StorageError::InvalidPersistedReceiveValue("missing collided receive record"),
            )?;
            if existing.record != *record {
                return Err(StorageError::IdentityCollision);
            }
            transaction.commit()?;
            return Ok(ReceiveInsertOutcome::Existing {
                sequence: existing.sequence,
            });
        }

        insert_window(&transaction, record)?;
        let sequence = sequence_from_i64(transaction.last_insert_rowid())?;
        insert_diagnostics(&transaction, record)?;
        for (index, decode) in record.decodes.iter().enumerate() {
            insert_decode(
                &transaction,
                &record.context.receive_window_id,
                index,
                decode,
            )?;
        }
        transaction.commit()?;
        Ok(ReceiveInsertOutcome::Inserted { sequence })
    }

    /// Atomically records receive evidence and its public event payload.
    ///
    /// A failed event insert rolls back a newly inserted receive record. A
    /// retry repairs an older committed receive record that predates event
    /// coupling, while exact event retries return the original sequence.
    pub fn record_receive_with_event(
        &mut self,
        record: &ReceiveRecord,
        event_id: &EventId,
        event_json: &str,
        occurred_utc_millis: i64,
    ) -> Result<ReceiveEventCommit, StorageError> {
        self.record_receive_with_event_builder(record, event_id, occurred_utc_millis, |_| {
            Ok(event_json.to_owned())
        })
    }

    /// Atomically records receive evidence and builds its event after the
    /// receive sequence is known inside the same transaction.
    pub fn record_receive_with_event_builder(
        &mut self,
        record: &ReceiveRecord,
        event_id: &EventId,
        occurred_utc_millis: i64,
        event_json: impl FnOnce(u64) -> Result<String, StorageError>,
    ) -> Result<ReceiveEventCommit, StorageError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let receive = if let Some(sequence) = find_existing_sequence(&transaction, &record.context)?
        {
            let existing = read_receive_by_sequence(&transaction, sequence)?.ok_or(
                StorageError::InvalidPersistedReceiveValue("missing collided receive record"),
            )?;
            if existing.record != *record {
                return Err(StorageError::IdentityCollision);
            }
            ReceiveInsertOutcome::Existing {
                sequence: existing.sequence,
            }
        } else {
            insert_window(&transaction, record)?;
            let sequence = sequence_from_i64(transaction.last_insert_rowid())?;
            insert_diagnostics(&transaction, record)?;
            for (index, decode) in record.decodes.iter().enumerate() {
                insert_decode(
                    &transaction,
                    &record.context.receive_window_id,
                    index,
                    decode,
                )?;
            }
            ReceiveInsertOutcome::Inserted { sequence }
        };
        let receive_sequence = match receive {
            ReceiveInsertOutcome::Inserted { sequence }
            | ReceiveInsertOutcome::Existing { sequence } => sequence,
        };
        let event_json = event_json(receive_sequence)?;

        let existing_event = transaction
            .query_row(
                "SELECT sequence, service_instance_id, event_json, occurred_utc_millis
                 FROM operational_events
                 WHERE event_id = ?1",
                [event_id.as_str()],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                    ))
                },
            )
            .optional()?;
        let event_sequence =
            if let Some((sequence, service_instance_id, stored_json, stored_time)) = existing_event
            {
                if service_instance_id != record.context.service_instance_id.as_str()
                    || stored_json != event_json
                    || stored_time != occurred_utc_millis
                {
                    return Err(StorageError::IdentityCollision);
                }
                sequence_from_i64(sequence)?
            } else {
                transaction.execute(
                    "INSERT INTO operational_events (
                        event_id, service_instance_id, event_json, occurred_utc_millis
                     ) VALUES (?1, ?2, ?3, ?4)",
                    params![
                        event_id.as_str(),
                        record.context.service_instance_id.as_str(),
                        event_json,
                        occurred_utc_millis
                    ],
                )?;
                sequence_from_i64(transaction.last_insert_rowid())?
            };
        transaction.commit()?;
        Ok(ReceiveEventCommit {
            receive,
            event_sequence,
        })
    }

    /// Loads one exact receive identity with all diagnostic/decode evidence.
    pub fn receive_record(
        &self,
        receive_window_id: &ReceiveWindowId,
    ) -> Result<Option<SequencedReceiveRecord>, StorageError> {
        let sequence = self
            .connection
            .query_row(
                "SELECT sequence FROM receive_windows WHERE receive_window_id = ?1",
                [receive_window_id.as_str()],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;
        let Some(sequence) = sequence else {
            return Ok(None);
        };
        read_receive_by_sequence(&self.connection, sequence)?.map_or_else(
            || {
                Err(StorageError::InvalidPersistedReceiveValue(
                    "receive diagnostics are missing",
                ))
            },
            |record| Ok(Some(record)),
        )
    }

    /// Reads a bounded receive page in deterministic global insertion order.
    pub fn receive_page(
        &self,
        after_sequence: u64,
        limit: usize,
    ) -> Result<ReceivePage, StorageError> {
        if !(1..=MAX_RECEIVE_PAGE_SIZE).contains(&limit) {
            return Err(StorageError::InvalidPageLimit);
        }
        let (earliest, latest): (Option<i64>, Option<i64>) = self.connection.query_row(
            "SELECT MIN(sequence), MAX(sequence) FROM receive_windows",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let earliest_sequence = earliest.map(sequence_from_i64).transpose()?;
        let latest_sequence = latest.map(sequence_from_i64).transpose()?;
        let after = i64::try_from(after_sequence).map_err(|_| StorageError::InvalidSequence)?;
        let fetch_limit =
            i64::try_from(limit.saturating_add(1)).map_err(|_| StorageError::InvalidSequence)?;
        let mut statement = self.connection.prepare(
            "SELECT sequence
             FROM receive_windows
             WHERE sequence > ?1
             ORDER BY sequence
             LIMIT ?2",
        )?;
        let rows = statement.query_map(params![after, fetch_limit], |row| row.get::<_, i64>(0))?;
        let mut records = Vec::new();
        for sequence in rows {
            records.push(
                read_receive_by_sequence(&self.connection, sequence?)?.ok_or(
                    StorageError::InvalidPersistedReceiveValue(
                        "receive sequence disappeared during query",
                    ),
                )?,
            );
        }
        let has_more = records.len() > limit;
        records.truncate(limit);
        Ok(ReceivePage {
            records,
            earliest_sequence,
            latest_sequence,
            has_more,
        })
    }

    /// Deletes receive evidence older than a retained global sequence.
    pub fn prune_receive_before(
        &mut self,
        first_retained_sequence: u64,
    ) -> Result<usize, StorageError> {
        let sequence =
            i64::try_from(first_retained_sequence).map_err(|_| StorageError::InvalidSequence)?;
        Ok(self.connection.execute(
            "DELETE FROM receive_windows WHERE sequence < ?1",
            [sequence],
        )?)
    }
}

fn validate_decode_metadata(metadata: Ft8DecodeMetadata) -> Result<(), StorageError> {
    if !(-60_000..=60_000).contains(&metadata.start_offset_millis)
        || metadata.audio_frequency_hz > 6_000
        || !(-100..=100).contains(&metadata.signal_to_noise_db)
    {
        return Err(StorageError::InvalidReceiveRecord(
            "decode metadata is outside durable bounds",
        ));
    }
    Ok(())
}

fn find_existing_sequence(
    transaction: &Transaction<'_>,
    context: &ReceiveWindowContext,
) -> Result<Option<i64>, StorageError> {
    Ok(transaction
        .query_row(
            "SELECT sequence
             FROM receive_windows
             WHERE receive_window_id = ?1
                OR (
                    service_instance_id = ?2
                    AND process_generation = ?3
                    AND stream_generation = ?4
                    AND slot_start_utc_millis = ?5
                    AND device_platform = ?6
                    AND device_opaque_id = ?7
                    AND sample_rate_hz = ?8
                    AND channels = ?9
                    AND sample_format = ?10
                    AND selected_channel = ?11
                )",
            params![
                context.receive_window_id.as_str(),
                context.service_instance_id.as_str(),
                to_i64(context.process_generation.get())?,
                to_i64(context.stream_generation.get())?,
                context.slot.start_utc_unix_millis(),
                platform_to_str(context.device_identity.platform()),
                context.device_identity.opaque_id(),
                i64::from(context.configuration.sample_rate_hz()),
                i64::from(context.configuration.channels()),
                sample_format_to_str(context.configuration.sample_format()),
                i64::from(context.configuration.selected_channel()),
            ],
            |row| row.get(0),
        )
        .optional()?)
}

fn insert_window(
    transaction: &Transaction<'_>,
    record: &ReceiveRecord,
) -> Result<(), StorageError> {
    let context = &record.context;
    transaction.execute(
        "INSERT INTO receive_windows (
            receive_window_id,
            service_instance_id,
            process_generation,
            stream_generation,
            slot_start_utc_millis,
            device_platform,
            device_opaque_id,
            sample_rate_hz,
            channels,
            sample_format,
            selected_channel,
            capture_position_frames,
            capture_utc_millis,
            capture_monotonic_millis,
            recorded_utc_millis
         ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15
         )",
        params![
            context.receive_window_id.as_str(),
            context.service_instance_id.as_str(),
            to_i64(context.process_generation.get())?,
            to_i64(context.stream_generation.get())?,
            context.slot.start_utc_unix_millis(),
            platform_to_str(context.device_identity.platform()),
            context.device_identity.opaque_id(),
            i64::from(context.configuration.sample_rate_hz()),
            i64::from(context.configuration.channels()),
            sample_format_to_str(context.configuration.sample_format()),
            i64::from(context.configuration.selected_channel()),
            to_i64(context.capture_mapping.position.frames())?,
            context.capture_mapping.utc_unix_millis,
            to_i64(context.capture_mapping.monotonic_millis)?,
            record.recorded_utc_millis,
        ],
    )?;
    Ok(())
}

fn insert_diagnostics(
    transaction: &Transaction<'_>,
    record: &ReceiveRecord,
) -> Result<(), StorageError> {
    let diagnostic = record.diagnostics;
    let clock = durable_clock_columns(diagnostic.clock)?;
    transaction.execute(
        "INSERT INTO receive_diagnostics (
            receive_window_id,
            audio_latency_millis,
            audio_drift_parts_per_million,
            audio_overflow_count,
            audio_clipped_sample_count,
            audio_max_callback_delay_millis,
            timeline_max_jitter_millis,
            timeline_drift_parts_per_million,
            timeline_incomplete_slot_count,
            timeline_late_batch_count,
            clock_state,
            clock_fault_kind,
            clock_fault_value,
            clock_recovery_progress,
            clock_recovery_required,
            clock_mapping_age_millis
         ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16
         )",
        params![
            record.context.receive_window_id.as_str(),
            i64::from(diagnostic.audio.latency_millis),
            i64::from(diagnostic.audio.drift_parts_per_million),
            to_i64(diagnostic.audio.overflow_count)?,
            to_i64(diagnostic.audio.clipped_sample_count)?,
            i64::from(diagnostic.audio.max_callback_delay_millis),
            i64::from(diagnostic.timeline.max_jitter_millis),
            i64::from(diagnostic.timeline.drift_parts_per_million),
            to_i64(diagnostic.timeline.incomplete_slot_count)?,
            to_i64(diagnostic.timeline.late_batch_count)?,
            clock.state,
            clock.fault_kind,
            clock.fault_value,
            i64::from(clock.recovery_progress),
            i64::from(clock.recovery_required),
            to_i64(clock.mapping_age_millis)?,
        ],
    )?;
    Ok(())
}

fn insert_decode(
    transaction: &Transaction<'_>,
    receive_window_id: &ReceiveWindowId,
    index: usize,
    decode: &Ft8Decode,
) -> Result<(), StorageError> {
    let message = durable_message_columns(&decode.message);
    transaction.execute(
        "INSERT INTO receive_decodes (
            receive_window_id,
            decode_index,
            start_offset_millis,
            audio_frequency_hz,
            signal_to_noise_db,
            outcome_kind,
            canonical_text,
            classification_detail,
            message_class,
            sender_callsign,
            recipient_callsign
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            receive_window_id.as_str(),
            i64::try_from(index).map_err(|_| StorageError::InvalidSequence)?,
            i64::from(decode.metadata.start_offset_millis),
            i64::from(decode.metadata.audio_frequency_hz),
            i64::from(decode.metadata.signal_to_noise_db),
            message.outcome_kind,
            message.canonical_text,
            message.detail,
            message.message_class,
            message.sender,
            message.recipient,
        ],
    )?;
    Ok(())
}

fn read_receive_by_sequence(
    connection: &Connection,
    sequence: i64,
) -> Result<Option<SequencedReceiveRecord>, StorageError> {
    let raw = connection
        .query_row(
            "SELECT
                w.sequence,
                w.receive_window_id,
                w.service_instance_id,
                w.process_generation,
                w.stream_generation,
                w.slot_start_utc_millis,
                w.device_platform,
                w.device_opaque_id,
                w.sample_rate_hz,
                w.channels,
                w.sample_format,
                w.selected_channel,
                w.capture_position_frames,
                w.capture_utc_millis,
                w.capture_monotonic_millis,
                w.recorded_utc_millis,
                d.audio_latency_millis,
                d.audio_drift_parts_per_million,
                d.audio_overflow_count,
                d.audio_clipped_sample_count,
                d.audio_max_callback_delay_millis,
                d.timeline_max_jitter_millis,
                d.timeline_drift_parts_per_million,
                d.timeline_incomplete_slot_count,
                d.timeline_late_batch_count,
                d.clock_state,
                d.clock_fault_kind,
                d.clock_fault_value,
                d.clock_recovery_progress,
                d.clock_recovery_required,
                d.clock_mapping_age_millis
             FROM receive_windows w
             JOIN receive_diagnostics d USING (receive_window_id)
             WHERE w.sequence = ?1",
            [sequence],
            |row| {
                Ok(RawReceive {
                    sequence: row.get(0)?,
                    receive_window_id: row.get(1)?,
                    service_instance_id: row.get(2)?,
                    process_generation: row.get(3)?,
                    stream_generation: row.get(4)?,
                    slot_start_utc_millis: row.get(5)?,
                    device_platform: row.get(6)?,
                    device_opaque_id: row.get(7)?,
                    sample_rate_hz: row.get(8)?,
                    channels: row.get(9)?,
                    sample_format: row.get(10)?,
                    selected_channel: row.get(11)?,
                    capture_position_frames: row.get(12)?,
                    capture_utc_millis: row.get(13)?,
                    capture_monotonic_millis: row.get(14)?,
                    recorded_utc_millis: row.get(15)?,
                    audio_latency_millis: row.get(16)?,
                    audio_drift_parts_per_million: row.get(17)?,
                    audio_overflow_count: row.get(18)?,
                    audio_clipped_sample_count: row.get(19)?,
                    audio_max_callback_delay_millis: row.get(20)?,
                    timeline_max_jitter_millis: row.get(21)?,
                    timeline_drift_parts_per_million: row.get(22)?,
                    timeline_incomplete_slot_count: row.get(23)?,
                    timeline_late_batch_count: row.get(24)?,
                    clock_state: row.get(25)?,
                    clock_fault_kind: row.get(26)?,
                    clock_fault_value: row.get(27)?,
                    clock_recovery_progress: row.get(28)?,
                    clock_recovery_required: row.get(29)?,
                    clock_mapping_age_millis: row.get(30)?,
                })
            },
        )
        .optional()?;
    raw.map(|raw| reconstruct_receive(connection, raw))
        .transpose()
}

fn reconstruct_receive(
    connection: &Connection,
    raw: RawReceive,
) -> Result<SequencedReceiveRecord, StorageError> {
    let sequence = sequence_from_i64(raw.sequence)?;
    let clock = reconstruct_clock(&raw)?;
    let receive_window_id = raw.receive_window_id.parse()?;
    let device_identity =
        InputDeviceIdentity::new(str_to_platform(&raw.device_platform)?, raw.device_opaque_id)
            .map_err(|_| StorageError::InvalidPersistedReceiveValue("device identity"))?;
    let configuration = InputConfiguration::new(
        to_u32(raw.sample_rate_hz, "sample rate")?,
        to_u16(raw.channels, "channel count")?,
        str_to_sample_format(&raw.sample_format)?,
        to_u16(raw.selected_channel, "selected channel")?,
    )
    .map_err(|_| StorageError::InvalidPersistedReceiveValue("input configuration"))?;
    let context = ReceiveWindowContext {
        receive_window_id,
        service_instance_id: raw.service_instance_id.parse()?,
        process_generation: ProcessGeneration::new(to_u64(raw.process_generation, "process")?)
            .map_err(|_| StorageError::InvalidPersistedReceiveValue("process generation"))?,
        stream_generation: StreamGeneration::new(to_u64(raw.stream_generation, "stream")?)
            .map_err(|_| StorageError::InvalidPersistedReceiveValue("stream generation"))?,
        slot: Ft8ReceiveSlot::new(raw.slot_start_utc_millis)
            .map_err(|_| StorageError::InvalidPersistedReceiveValue("FT8 slot"))?,
        device_identity,
        configuration,
        capture_mapping: CaptureTimeEvidence::new(
            CapturePosition::from_frames(to_u64(raw.capture_position_frames, "capture position")?),
            raw.capture_utc_millis,
            to_u64(raw.capture_monotonic_millis, "capture monotonic time")?,
        )
        .map_err(|_| StorageError::InvalidPersistedReceiveValue("capture mapping"))?,
    };
    let diagnostics = ReceiveDiagnosticSummary {
        audio: InputHealth::new(
            to_u32(raw.audio_latency_millis, "audio latency")?,
            to_i32(raw.audio_drift_parts_per_million, "audio drift")?,
            to_u64(raw.audio_overflow_count, "audio overflow count")?,
            to_u64(raw.audio_clipped_sample_count, "audio clipped count")?,
            to_u32(raw.audio_max_callback_delay_millis, "callback delay")?,
        )
        .map_err(|_| StorageError::InvalidPersistedReceiveValue("audio health"))?,
        timeline: ReceiveTimelineHealth {
            max_jitter_millis: to_u32(raw.timeline_max_jitter_millis, "timeline jitter")?,
            drift_parts_per_million: to_i32(
                raw.timeline_drift_parts_per_million,
                "timeline drift",
            )?,
            incomplete_slot_count: to_u64(
                raw.timeline_incomplete_slot_count,
                "incomplete slot count",
            )?,
            late_batch_count: to_u64(raw.timeline_late_batch_count, "late batch count")?,
        },
        clock,
    };
    let decodes = read_decodes(connection, context.receive_window_id.as_str())?;
    let record = ReceiveRecord::new(context, diagnostics, decodes, raw.recorded_utc_millis)
        .map_err(|_| StorageError::InvalidPersistedReceiveValue("receive record invariants"))?;
    Ok(SequencedReceiveRecord { sequence, record })
}

fn read_decodes(
    connection: &Connection,
    receive_window_id: &str,
) -> Result<Vec<Ft8Decode>, StorageError> {
    let mut statement = connection.prepare(
        "SELECT
            decode_index,
            start_offset_millis,
            audio_frequency_hz,
            signal_to_noise_db,
            outcome_kind,
            canonical_text,
            classification_detail,
            message_class,
            sender_callsign,
            recipient_callsign
         FROM receive_decodes
         WHERE receive_window_id = ?1
         ORDER BY decode_index",
    )?;
    let rows = statement.query_map([receive_window_id], |row| {
        Ok(RawDecode {
            index: row.get(0)?,
            start_offset_millis: row.get(1)?,
            audio_frequency_hz: row.get(2)?,
            signal_to_noise_db: row.get(3)?,
            outcome_kind: row.get(4)?,
            canonical_text: row.get(5)?,
            detail: row.get(6)?,
            message_class: row.get(7)?,
            sender: row.get(8)?,
            recipient: row.get(9)?,
        })
    })?;
    let mut decodes = Vec::new();
    for (expected_index, row) in rows.enumerate() {
        let raw = row?;
        if raw.index != i64::try_from(expected_index).map_err(|_| StorageError::InvalidSequence)? {
            return Err(StorageError::InvalidPersistedReceiveValue(
                "decode indices are not contiguous",
            ));
        }
        decodes.push(Ft8Decode {
            metadata: Ft8DecodeMetadata {
                start_offset_millis: to_i32(raw.start_offset_millis, "decode offset")?,
                audio_frequency_hz: to_u32(raw.audio_frequency_hz, "decode frequency")?,
                signal_to_noise_db: to_i16(raw.signal_to_noise_db, "decode SNR")?,
            },
            message: reconstruct_message(raw)?,
        });
    }
    let mut sorted = decodes.clone();
    Ft8Decode::sort_deterministically(&mut sorted);
    if sorted != decodes {
        return Err(StorageError::InvalidPersistedReceiveValue(
            "decode order is nondeterministic",
        ));
    }
    Ok(decodes)
}

#[derive(Debug)]
struct RawReceive {
    sequence: i64,
    receive_window_id: String,
    service_instance_id: String,
    process_generation: i64,
    stream_generation: i64,
    slot_start_utc_millis: i64,
    device_platform: String,
    device_opaque_id: String,
    sample_rate_hz: i64,
    channels: i64,
    sample_format: String,
    selected_channel: i64,
    capture_position_frames: i64,
    capture_utc_millis: i64,
    capture_monotonic_millis: i64,
    recorded_utc_millis: i64,
    audio_latency_millis: i64,
    audio_drift_parts_per_million: i64,
    audio_overflow_count: i64,
    audio_clipped_sample_count: i64,
    audio_max_callback_delay_millis: i64,
    timeline_max_jitter_millis: i64,
    timeline_drift_parts_per_million: i64,
    timeline_incomplete_slot_count: i64,
    timeline_late_batch_count: i64,
    clock_state: String,
    clock_fault_kind: Option<String>,
    clock_fault_value: Option<i64>,
    clock_recovery_progress: i64,
    clock_recovery_required: i64,
    clock_mapping_age_millis: i64,
}

struct DurableClockColumns {
    state: &'static str,
    fault_kind: Option<&'static str>,
    fault_value: Option<i64>,
    recovery_progress: u8,
    recovery_required: u8,
    mapping_age_millis: u64,
}

fn durable_clock_columns(clock: ReceiveClockHealth) -> Result<DurableClockColumns, StorageError> {
    clock.validate()?;
    match clock {
        ReceiveClockHealth::Healthy { mapping_age_millis } => Ok(DurableClockColumns {
            state: "healthy",
            fault_kind: None,
            fault_value: None,
            recovery_progress: 0,
            recovery_required: 0,
            mapping_age_millis,
        }),
        ReceiveClockHealth::Unhealthy {
            fault,
            recovery_progress,
            recovery_required,
            mapping_age_millis,
        } => {
            let (fault_kind, fault_value) = match fault {
                ReceiveClockFault::ProcessGenerationChanged => ("process_generation_changed", None),
                ReceiveClockFault::TimelineRegressed => ("timeline_regressed", None),
                ReceiveClockFault::UtcJump { divergence_millis } => {
                    ("utc_jump", Some(divergence_millis))
                }
                ReceiveClockFault::StaleMapping { age_millis } => {
                    ("stale_mapping", Some(to_i64(age_millis)?))
                }
                ReceiveClockFault::SamplingDelayed { delay_millis } => {
                    ("sampling_delayed", Some(to_i64(delay_millis)?))
                }
                ReceiveClockFault::SampleGap { gap_millis } => {
                    ("sample_gap", Some(to_i64(gap_millis)?))
                }
                ReceiveClockFault::WindowMisaligned { divergence_millis } => {
                    ("window_misaligned", Some(divergence_millis))
                }
                ReceiveClockFault::ArithmeticOverflow => ("arithmetic_overflow", None),
            };
            Ok(DurableClockColumns {
                state: "unhealthy",
                fault_kind: Some(fault_kind),
                fault_value,
                recovery_progress,
                recovery_required,
                mapping_age_millis,
            })
        }
    }
}

fn reconstruct_clock(raw: &RawReceive) -> Result<ReceiveClockHealth, StorageError> {
    let mapping_age_millis = to_u64(raw.clock_mapping_age_millis, "clock mapping age")?;
    let clock = match raw.clock_state.as_str() {
        "healthy"
            if raw.clock_fault_kind.is_none()
                && raw.clock_fault_value.is_none()
                && raw.clock_recovery_progress == 0
                && raw.clock_recovery_required == 0 =>
        {
            ReceiveClockHealth::Healthy { mapping_age_millis }
        }
        "unhealthy" => ReceiveClockHealth::Unhealthy {
            fault: reconstruct_clock_fault(raw.clock_fault_kind.as_deref(), raw.clock_fault_value)?,
            recovery_progress: to_u8(raw.clock_recovery_progress, "clock recovery progress")?,
            recovery_required: to_u8(raw.clock_recovery_required, "clock recovery required")?,
            mapping_age_millis,
        },
        _ => {
            return Err(StorageError::InvalidPersistedReceiveValue(
                "clock state shape",
            ));
        }
    };
    clock
        .validate()
        .map_err(|_| StorageError::InvalidPersistedReceiveValue("clock health"))?;
    Ok(clock)
}

fn reconstruct_clock_fault(
    kind: Option<&str>,
    value: Option<i64>,
) -> Result<ReceiveClockFault, StorageError> {
    match (kind, value) {
        (Some("process_generation_changed"), None) => {
            Ok(ReceiveClockFault::ProcessGenerationChanged)
        }
        (Some("timeline_regressed"), None) => Ok(ReceiveClockFault::TimelineRegressed),
        (Some("utc_jump"), Some(divergence_millis)) => {
            Ok(ReceiveClockFault::UtcJump { divergence_millis })
        }
        (Some("stale_mapping"), Some(value)) => Ok(ReceiveClockFault::StaleMapping {
            age_millis: to_u64(value, "stale mapping age")?,
        }),
        (Some("sampling_delayed"), Some(value)) => Ok(ReceiveClockFault::SamplingDelayed {
            delay_millis: to_u64(value, "sampling delay")?,
        }),
        (Some("sample_gap"), Some(value)) => Ok(ReceiveClockFault::SampleGap {
            gap_millis: to_u64(value, "sample gap")?,
        }),
        (Some("window_misaligned"), Some(divergence_millis)) => {
            Ok(ReceiveClockFault::WindowMisaligned { divergence_millis })
        }
        (Some("arithmetic_overflow"), None) => Ok(ReceiveClockFault::ArithmeticOverflow),
        _ => Err(StorageError::InvalidPersistedReceiveValue(
            "clock fault shape",
        )),
    }
}

struct DurableMessageColumns {
    outcome_kind: &'static str,
    canonical_text: String,
    detail: Option<String>,
    message_class: Option<&'static str>,
    sender: Option<String>,
    recipient: Option<String>,
}

fn durable_message_columns(message: &ClassifiedFt8Message) -> DurableMessageColumns {
    match message {
        ClassifiedFt8Message::Resolved(message) => DurableMessageColumns {
            outcome_kind: "resolved",
            canonical_text: message.canonical_text().to_owned(),
            detail: None,
            message_class: Some(message_class_to_str(message.class())),
            sender: Some(message.sender().original().to_owned()),
            recipient: message
                .recipient()
                .map(|callsign| callsign.original().to_owned()),
        },
        ClassifiedFt8Message::UnresolvedHash(message) => DurableMessageColumns {
            outcome_kind: "unresolved_hash",
            canonical_text: message.canonical_text().to_owned(),
            detail: Some(message.detail().to_owned()),
            message_class: None,
            sender: None,
            recipient: None,
        },
        ClassifiedFt8Message::Unsupported(message) => DurableMessageColumns {
            outcome_kind: "unsupported",
            canonical_text: message.canonical_text().to_owned(),
            detail: Some(message.detail().to_owned()),
            message_class: None,
            sender: None,
            recipient: None,
        },
        ClassifiedFt8Message::Ambiguous(message) => DurableMessageColumns {
            outcome_kind: "ambiguous",
            canonical_text: message.canonical_text().to_owned(),
            detail: Some(message.detail().to_owned()),
            message_class: None,
            sender: None,
            recipient: None,
        },
        ClassifiedFt8Message::FreeText(message) => DurableMessageColumns {
            outcome_kind: "free_text",
            canonical_text: message.text().to_owned(),
            detail: None,
            message_class: None,
            sender: None,
            recipient: None,
        },
    }
}

#[derive(Debug)]
struct RawDecode {
    index: i64,
    start_offset_millis: i64,
    audio_frequency_hz: i64,
    signal_to_noise_db: i64,
    outcome_kind: String,
    canonical_text: String,
    detail: Option<String>,
    message_class: Option<String>,
    sender: Option<String>,
    recipient: Option<String>,
}

fn reconstruct_message(raw: RawDecode) -> Result<ClassifiedFt8Message, StorageError> {
    let message = match raw.outcome_kind.as_str() {
        "resolved" => ClassifiedFt8Message::Resolved(
            ResolvedFt8Message::new(
                raw.canonical_text,
                raw.sender
                    .ok_or(StorageError::InvalidPersistedReceiveValue(
                        "resolved sender",
                    ))?
                    .parse()
                    .map_err(|_| {
                        StorageError::InvalidPersistedReceiveValue("resolved sender callsign")
                    })?,
                raw.recipient
                    .map(|value| {
                        value.parse().map_err(|_| {
                            StorageError::InvalidPersistedReceiveValue(
                                "resolved recipient callsign",
                            )
                        })
                    })
                    .transpose()?,
                str_to_message_class(raw.message_class.as_deref().ok_or(
                    StorageError::InvalidPersistedReceiveValue("resolved message class"),
                )?)?,
            )
            .map_err(|_| StorageError::InvalidPersistedReceiveValue("resolved message"))?,
        ),
        "unresolved_hash" => ClassifiedFt8Message::UnresolvedHash(
            UnresolvedHashFt8Message::new(
                raw.canonical_text,
                raw.detail
                    .ok_or(StorageError::InvalidPersistedReceiveValue(
                        "unresolved detail",
                    ))?,
            )
            .map_err(|_| StorageError::InvalidPersistedReceiveValue("unresolved message"))?,
        ),
        "unsupported" => ClassifiedFt8Message::Unsupported(
            UnsupportedFt8Message::new(
                raw.canonical_text,
                raw.detail
                    .ok_or(StorageError::InvalidPersistedReceiveValue(
                        "unsupported detail",
                    ))?,
            )
            .map_err(|_| StorageError::InvalidPersistedReceiveValue("unsupported message"))?,
        ),
        "ambiguous" => ClassifiedFt8Message::Ambiguous(
            AmbiguousFt8Message::new(
                raw.canonical_text,
                raw.detail
                    .ok_or(StorageError::InvalidPersistedReceiveValue(
                        "ambiguous detail",
                    ))?,
            )
            .map_err(|_| StorageError::InvalidPersistedReceiveValue("ambiguous message"))?,
        ),
        "free_text" => ClassifiedFt8Message::FreeText(
            FreeTextFt8Message::new(raw.canonical_text)
                .map_err(|_| StorageError::InvalidPersistedReceiveValue("free-text message"))?,
        ),
        _ => {
            return Err(StorageError::InvalidPersistedReceiveValue(
                "decode outcome classification",
            ));
        }
    };
    Ok(message)
}

const fn platform_to_str(platform: InputPlatform) -> &'static str {
    match platform {
        InputPlatform::MacOsCoreAudio => "macos_core_audio",
        InputPlatform::WindowsWasapi => "windows_wasapi",
        InputPlatform::LinuxAlsa => "linux_alsa",
        InputPlatform::LinuxJack => "linux_jack",
    }
}

fn str_to_platform(value: &str) -> Result<InputPlatform, StorageError> {
    match value {
        "macos_core_audio" => Ok(InputPlatform::MacOsCoreAudio),
        "windows_wasapi" => Ok(InputPlatform::WindowsWasapi),
        "linux_alsa" => Ok(InputPlatform::LinuxAlsa),
        "linux_jack" => Ok(InputPlatform::LinuxJack),
        _ => Err(StorageError::InvalidPersistedReceiveValue("input platform")),
    }
}

const fn sample_format_to_str(format: InputSampleFormat) -> &'static str {
    match format {
        InputSampleFormat::Signed8 => "signed_8",
        InputSampleFormat::Signed16 => "signed_16",
        InputSampleFormat::Signed24 => "signed_24",
        InputSampleFormat::Signed32 => "signed_32",
        InputSampleFormat::Signed64 => "signed_64",
        InputSampleFormat::Unsigned8 => "unsigned_8",
        InputSampleFormat::Unsigned16 => "unsigned_16",
        InputSampleFormat::Unsigned24 => "unsigned_24",
        InputSampleFormat::Unsigned32 => "unsigned_32",
        InputSampleFormat::Unsigned64 => "unsigned_64",
        InputSampleFormat::Float32 => "float_32",
        InputSampleFormat::Float64 => "float_64",
    }
}

fn str_to_sample_format(value: &str) -> Result<InputSampleFormat, StorageError> {
    match value {
        "signed_8" => Ok(InputSampleFormat::Signed8),
        "signed_16" => Ok(InputSampleFormat::Signed16),
        "signed_24" => Ok(InputSampleFormat::Signed24),
        "signed_32" => Ok(InputSampleFormat::Signed32),
        "signed_64" => Ok(InputSampleFormat::Signed64),
        "unsigned_8" => Ok(InputSampleFormat::Unsigned8),
        "unsigned_16" => Ok(InputSampleFormat::Unsigned16),
        "unsigned_24" => Ok(InputSampleFormat::Unsigned24),
        "unsigned_32" => Ok(InputSampleFormat::Unsigned32),
        "unsigned_64" => Ok(InputSampleFormat::Unsigned64),
        "float_32" => Ok(InputSampleFormat::Float32),
        "float_64" => Ok(InputSampleFormat::Float64),
        _ => Err(StorageError::InvalidPersistedReceiveValue(
            "input sample format",
        )),
    }
}

const fn message_class_to_str(class: Ft8MessageClass) -> &'static str {
    match class {
        Ft8MessageClass::GeneralCall => "general_call",
        Ft8MessageClass::DirectedGrid => "directed_grid",
        Ft8MessageClass::SignalReport => "signal_report",
        Ft8MessageClass::RogerSignalReport => "roger_signal_report",
        Ft8MessageClass::Roger => "roger",
        Ft8MessageClass::Ending73 => "ending_73",
        Ft8MessageClass::EndingRr73 => "ending_rr73",
    }
}

fn str_to_message_class(value: &str) -> Result<Ft8MessageClass, StorageError> {
    match value {
        "general_call" => Ok(Ft8MessageClass::GeneralCall),
        "directed_grid" => Ok(Ft8MessageClass::DirectedGrid),
        "signal_report" => Ok(Ft8MessageClass::SignalReport),
        "roger_signal_report" => Ok(Ft8MessageClass::RogerSignalReport),
        "roger" => Ok(Ft8MessageClass::Roger),
        "ending_73" => Ok(Ft8MessageClass::Ending73),
        "ending_rr73" => Ok(Ft8MessageClass::EndingRr73),
        _ => Err(StorageError::InvalidPersistedReceiveValue(
            "FT8 message class",
        )),
    }
}

fn to_i64(value: u64) -> Result<i64, StorageError> {
    i64::try_from(value).map_err(|_| StorageError::InvalidSequence)
}

fn to_u64(value: i64, field: &'static str) -> Result<u64, StorageError> {
    u64::try_from(value).map_err(|_| StorageError::InvalidPersistedReceiveValue(field))
}

fn to_u32(value: i64, field: &'static str) -> Result<u32, StorageError> {
    u32::try_from(value).map_err(|_| StorageError::InvalidPersistedReceiveValue(field))
}

fn to_u16(value: i64, field: &'static str) -> Result<u16, StorageError> {
    u16::try_from(value).map_err(|_| StorageError::InvalidPersistedReceiveValue(field))
}

fn to_u8(value: i64, field: &'static str) -> Result<u8, StorageError> {
    u8::try_from(value).map_err(|_| StorageError::InvalidPersistedReceiveValue(field))
}

fn to_i32(value: i64, field: &'static str) -> Result<i32, StorageError> {
    i32::try_from(value).map_err(|_| StorageError::InvalidPersistedReceiveValue(field))
}

fn to_i16(value: i64, field: &'static str) -> Result<i16, StorageError> {
    i16::try_from(value).map_err(|_| StorageError::InvalidPersistedReceiveValue(field))
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        sync::atomic::{AtomicU64, Ordering},
    };

    use slotpilot_protocol::Ft8OutcomeKind;

    use super::*;

    static DATABASE_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temp_database() -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "slotpilot-receive-storage-{}-{}.sqlite3",
            std::process::id(),
            DATABASE_COUNTER.fetch_add(1, Ordering::Relaxed)
        ))
    }

    fn classified_decodes() -> Vec<Ft8Decode> {
        vec![
            Ft8Decode {
                metadata: Ft8DecodeMetadata {
                    start_offset_millis: 400,
                    audio_frequency_hz: 1_500,
                    signal_to_noise_db: -8,
                },
                message: ClassifiedFt8Message::FreeText(FreeTextFt8Message::new("HELLO").unwrap()),
            },
            Ft8Decode {
                metadata: Ft8DecodeMetadata {
                    start_offset_millis: 100,
                    audio_frequency_hz: 1_000,
                    signal_to_noise_db: -12,
                },
                message: ClassifiedFt8Message::Resolved(
                    ResolvedFt8Message::new(
                        "CQ K1ABC FN42",
                        "K1ABC".parse().unwrap(),
                        None,
                        Ft8MessageClass::GeneralCall,
                    )
                    .unwrap(),
                ),
            },
            Ft8Decode {
                metadata: Ft8DecodeMetadata {
                    start_offset_millis: 200,
                    audio_frequency_hz: 1_100,
                    signal_to_noise_db: -10,
                },
                message: ClassifiedFt8Message::UnresolvedHash(
                    UnresolvedHashFt8Message::new("<123> K1ABC -10", "sender hash unresolved")
                        .unwrap(),
                ),
            },
            Ft8Decode {
                metadata: Ft8DecodeMetadata {
                    start_offset_millis: 250,
                    audio_frequency_hz: 1_200,
                    signal_to_noise_db: -9,
                },
                message: ClassifiedFt8Message::Unsupported(
                    UnsupportedFt8Message::new("CQ TEST K1ABC", "unsupported structured subtype")
                        .unwrap(),
                ),
            },
            Ft8Decode {
                metadata: Ft8DecodeMetadata {
                    start_offset_millis: 300,
                    audio_frequency_hz: 1_300,
                    signal_to_noise_db: -7,
                },
                message: ClassifiedFt8Message::Ambiguous(
                    AmbiguousFt8Message::new("RR73", "grid or reserved response").unwrap(),
                ),
            },
        ]
    }

    fn healthy_diagnostics() -> ReceiveDiagnosticSummary {
        ReceiveDiagnosticSummary {
            audio: InputHealth::new(25, 10, 2, 3, 4).unwrap(),
            timeline: ReceiveTimelineHealth {
                max_jitter_millis: 5,
                drift_parts_per_million: 10,
                incomplete_slot_count: 6,
                late_batch_count: 7,
            },
            clock: ReceiveClockHealth::Healthy {
                mapping_age_millis: 100,
            },
        }
    }

    fn record(
        identity: &str,
        slot_start: i64,
        stream: u64,
        device: &str,
        decodes: Vec<Ft8Decode>,
    ) -> ReceiveRecord {
        ReceiveRecord::new(
            ReceiveWindowContext {
                receive_window_id: identity.parse().unwrap(),
                service_instance_id: "svc_01jabcde9".parse().unwrap(),
                process_generation: ProcessGeneration::new(1).unwrap(),
                stream_generation: StreamGeneration::new(stream).unwrap(),
                slot: Ft8ReceiveSlot::new(slot_start).unwrap(),
                device_identity: InputDeviceIdentity::new(InputPlatform::MacOsCoreAudio, device)
                    .unwrap(),
                configuration: InputConfiguration::new(48_000, 2, InputSampleFormat::Signed16, 1)
                    .unwrap(),
                capture_mapping: CaptureTimeEvidence::new(
                    CapturePosition::from_frames(42),
                    slot_start,
                    u64::try_from(slot_start).unwrap() + 1_000,
                )
                .unwrap(),
            },
            healthy_diagnostics(),
            decodes,
            slot_start + 20_000,
        )
        .unwrap()
    }

    #[test]
    fn every_owned_classification_round_trips_in_deterministic_order() {
        let mut store = Store::open_in_memory().unwrap();
        let record = record(
            "rxw_01jabcde9",
            30_000,
            1,
            "coreaudio:input-1",
            classified_decodes(),
        );
        assert!(matches!(
            store.record_receive(&record).unwrap(),
            ReceiveInsertOutcome::Inserted { sequence: 1 }
        ));
        assert_eq!(
            store.record_receive(&record).unwrap(),
            ReceiveInsertOutcome::Existing { sequence: 1 }
        );
        let loaded = store
            .receive_record(&"rxw_01jabcde9".parse().unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(loaded.record, record);
        assert_eq!(
            loaded
                .record
                .decodes()
                .iter()
                .map(|decode| decode.message.kind())
                .collect::<Vec<_>>(),
            vec![
                Ft8OutcomeKind::Resolved,
                Ft8OutcomeKind::UnresolvedHash,
                Ft8OutcomeKind::Unsupported,
                Ft8OutcomeKind::Ambiguous,
                Ft8OutcomeKind::FreeText,
            ]
        );
    }

    #[test]
    fn receive_and_public_event_commit_atomically_and_retry_exactly() {
        let mut store = Store::open_in_memory().unwrap();
        let record = record(
            "rxw_01jabcdf9",
            45_000,
            1,
            "coreaudio:input-1",
            classified_decodes(),
        );
        let event_id: EventId = "evt_01jabcdf9".parse().unwrap();
        assert!(
            store
                .record_receive_with_event(&record, &event_id, "{invalid", 65_000)
                .is_err()
        );
        assert!(
            store
                .receive_record(&record.context().receive_window_id)
                .unwrap()
                .is_none()
        );
        let first = store
            .record_receive_with_event(
                &record,
                &event_id,
                r#"{"kind":"receive_decode","receive_window_id":"rxw_01jabcdf9"}"#,
                65_000,
            )
            .unwrap();
        assert!(matches!(
            first.receive,
            ReceiveInsertOutcome::Inserted { sequence: 1 }
        ));
        let replay = store
            .record_receive_with_event(
                &record,
                &event_id,
                r#"{"kind":"receive_decode","receive_window_id":"rxw_01jabcdf9"}"#,
                65_000,
            )
            .unwrap();
        assert_eq!(
            replay,
            ReceiveEventCommit {
                receive: ReceiveInsertOutcome::Existing { sequence: 1 },
                event_sequence: first.event_sequence,
            }
        );
        let events = store
            .replay_events(&record.context().service_instance_id, 0, 10)
            .unwrap();
        assert_eq!(events.events.len(), 1);
    }

    #[test]
    fn retry_identity_does_not_collapse_context_or_conflicts() {
        let mut store = Store::open_in_memory().unwrap();
        let first = record("rxw_01jabcde9", 30_000, 1, "coreaudio:input-1", Vec::new());
        store.record_receive(&first).unwrap();

        let same_context_new_id =
            record("rxw_01jabcdf0", 30_000, 1, "coreaudio:input-1", Vec::new());
        assert!(matches!(
            store.record_receive(&same_context_new_id),
            Err(StorageError::IdentityCollision)
        ));

        for distinct in [
            record("rxw_01jabcdf1", 45_000, 1, "coreaudio:input-1", Vec::new()),
            record("rxw_01jabcdf2", 30_000, 2, "coreaudio:input-1", Vec::new()),
            record("rxw_01jabcdf3", 30_000, 1, "coreaudio:input-2", Vec::new()),
        ] {
            assert!(matches!(
                store.record_receive(&distinct).unwrap(),
                ReceiveInsertOutcome::Inserted { .. }
            ));
        }
    }

    #[test]
    fn injected_decode_failure_rolls_back_window_and_diagnostics() {
        let mut store = Store::open_in_memory().unwrap();
        store
            .connection
            .execute_batch(
                "CREATE TEMP TRIGGER fail_receive_decode
                 BEFORE INSERT ON receive_decodes
                 BEGIN
                    SELECT RAISE(ABORT, 'injected receive decode failure');
                 END;",
            )
            .unwrap();
        let record = record(
            "rxw_01jabcde9",
            30_000,
            1,
            "coreaudio:input-1",
            classified_decodes(),
        );
        assert!(matches!(
            store.record_receive(&record),
            Err(StorageError::Database(_))
        ));
        for table in ["receive_windows", "receive_diagnostics", "receive_decodes"] {
            let count: u32 = store
                .connection
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(count, 0, "{table}");
        }
    }

    #[test]
    fn restart_pagination_retention_and_page_bounds_are_deterministic() {
        let path = temp_database();
        {
            let mut store = Store::open(&path).unwrap();
            for (identity, slot, stream) in [
                ("rxw_01jabcde9", 30_000, 1),
                ("rxw_01jabcdf0", 45_000, 2),
                ("rxw_01jabcdf1", 60_000, 3),
            ] {
                store
                    .record_receive(&record(
                        identity,
                        slot,
                        stream,
                        "coreaudio:input-1",
                        Vec::new(),
                    ))
                    .unwrap();
            }
        }
        {
            let mut store = Store::open(&path).unwrap();
            let page = store.receive_page(0, 2).unwrap();
            assert_eq!(
                page.records
                    .iter()
                    .map(|entry| entry.sequence)
                    .collect::<Vec<_>>(),
                vec![1, 2]
            );
            assert_eq!(page.earliest_sequence, Some(1));
            assert_eq!(page.latest_sequence, Some(3));
            assert!(page.has_more);
            assert!(matches!(
                store.receive_page(0, 0),
                Err(StorageError::InvalidPageLimit)
            ));
            assert!(matches!(
                store.receive_page(0, MAX_RECEIVE_PAGE_SIZE + 1),
                Err(StorageError::InvalidPageLimit)
            ));
            assert_eq!(store.prune_receive_before(3).unwrap(), 2);
            let retained = store.receive_page(0, 10).unwrap();
            assert_eq!(retained.earliest_sequence, Some(3));
            assert_eq!(retained.records.len(), 1);
        }
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn clock_fault_matrix_round_trips_and_unhealthy_decodes_fail_closed() {
        let faults = [
            ReceiveClockFault::ProcessGenerationChanged,
            ReceiveClockFault::TimelineRegressed,
            ReceiveClockFault::UtcJump {
                divergence_millis: -101,
            },
            ReceiveClockFault::StaleMapping { age_millis: 2_501 },
            ReceiveClockFault::SamplingDelayed { delay_millis: 251 },
            ReceiveClockFault::SampleGap { gap_millis: 6_000 },
            ReceiveClockFault::WindowMisaligned {
                divergence_millis: 101,
            },
            ReceiveClockFault::ArithmeticOverflow,
        ];
        let mut store = Store::open_in_memory().unwrap();
        for (index, fault) in faults.into_iter().enumerate() {
            let slot_start = 30_000 + i64::try_from(index).unwrap() * 15_000;
            let mut record = record(
                &format!("rxw_01jabcdf{index}"),
                slot_start,
                u64::try_from(index).unwrap() + 1,
                "coreaudio:input-1",
                Vec::new(),
            );
            record.diagnostics.clock = ReceiveClockHealth::Unhealthy {
                fault,
                recovery_progress: 1,
                recovery_required: 3,
                mapping_age_millis: 100,
            };
            store.record_receive(&record).unwrap();
            assert_eq!(
                store
                    .receive_record(&record.context.receive_window_id)
                    .unwrap()
                    .unwrap()
                    .record
                    .diagnostics
                    .clock,
                record.diagnostics.clock
            );
        }

        let context = record(
            "rxw_01jabcxyz",
            180_000,
            20,
            "coreaudio:input-1",
            Vec::new(),
        )
        .context;
        let mut diagnostics = healthy_diagnostics();
        diagnostics.clock = ReceiveClockHealth::Unhealthy {
            fault: ReceiveClockFault::StaleMapping { age_millis: 3_000 },
            recovery_progress: 0,
            recovery_required: 3,
            mapping_age_millis: 3_000,
        };
        assert!(matches!(
            ReceiveRecord::new(context, diagnostics, classified_decodes(), 200_000),
            Err(StorageError::InvalidReceiveRecord(_))
        ));
    }

    #[test]
    fn malformed_classification_identity_and_foreign_key_fail_typed() {
        let mut store = Store::open_in_memory().unwrap();
        let record = record(
            "rxw_01jabcde9",
            30_000,
            1,
            "coreaudio:input-1",
            classified_decodes(),
        );
        store.record_receive(&record).unwrap();
        store
            .connection
            .pragma_update(None, "ignore_check_constraints", "ON")
            .unwrap();
        store
            .connection
            .execute(
                "UPDATE receive_decodes
                 SET outcome_kind = 'dependency_private'
                 WHERE receive_window_id = 'rxw_01jabcde9' AND decode_index = 0",
                [],
            )
            .unwrap();
        assert!(matches!(
            store.receive_record(&"rxw_01jabcde9".parse().unwrap()),
            Err(StorageError::InvalidPersistedReceiveValue(
                "decode outcome classification"
            ))
        ));

        store
            .connection
            .execute(
                "UPDATE receive_decodes
                 SET outcome_kind = 'resolved'
                 WHERE receive_window_id = 'rxw_01jabcde9' AND decode_index = 0",
                [],
            )
            .unwrap();
        store
            .connection
            .execute(
                "UPDATE receive_windows
                 SET service_instance_id = 'invalid'
                 WHERE receive_window_id = 'rxw_01jabcde9'",
                [],
            )
            .unwrap();
        assert!(matches!(
            store.receive_record(&"rxw_01jabcde9".parse().unwrap()),
            Err(StorageError::InvalidIdentity(_))
        ));
        store
            .connection
            .execute(
                "UPDATE receive_windows
                 SET service_instance_id = 'svc_01jabcde9'
                 WHERE receive_window_id = 'rxw_01jabcde9'",
                [],
            )
            .unwrap();
        store
            .connection
            .execute(
                "DELETE FROM receive_diagnostics
                 WHERE receive_window_id = 'rxw_01jabcde9'",
                [],
            )
            .unwrap();
        assert!(matches!(
            store.receive_record(&"rxw_01jabcde9".parse().unwrap()),
            Err(StorageError::InvalidPersistedReceiveValue(
                "receive diagnostics are missing"
            ))
        ));

        store
            .connection
            .pragma_update(None, "ignore_check_constraints", "OFF")
            .unwrap();
        assert!(matches!(
            store.connection.execute(
                "INSERT INTO receive_decodes (
                    receive_window_id, decode_index, start_offset_millis,
                    audio_frequency_hz, signal_to_noise_db, outcome_kind,
                    canonical_text
                 ) VALUES (
                    'rxw_01jmissing', 0, 0, 1000, -10, 'free_text', 'HELLO'
                 )",
                []
            ),
            Err(rusqlite::Error::SqliteFailure(_, _))
        ));
    }
}
