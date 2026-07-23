//! Version-2 receive API mapping and durable command processing.

use std::path::Path;

use slotpilot_api::{
    API_VERSION, ApiError, Availability, Command, CommandClass, CommandEnvelope, ErrorCode,
    ErrorDetails, Ft8Classification, Ft8Decode, Ft8MessageClass, InputConfiguration,
    InputConfigurationRange, InputDevice, InputDeviceIdentity, InputDevicePage, InputPlatform,
    InputSampleFormat, LEGACY_API_VERSION, ReceiveAudioHealth, ReceiveClockHealth,
    ReceiveHistoryPage, ReceiveInhibitionKind, ReceiveLifecycleSnapshot, ReceiveRecordSummary,
    ReceiveSelection as ApiReceiveSelection, ReceiveStatus, ResponseEnvelope, ResponseOutcome,
    ResultBody, StationSnapshot,
};
use slotpilot_audio::{
    InputConfiguration as AudioInputConfiguration,
    InputConfigurationRange as AudioInputConfigurationRange, InputDeviceDescriptor,
    InputDeviceIdentity as AudioInputDeviceIdentity, InputPlatform as AudioInputPlatform,
    InputSampleFormat as AudioInputSampleFormat, WaterfallSnapshot,
};
use slotpilot_domain::{CommandId, EventId, RequestId, ServiceInstanceId};
use slotpilot_protocol::{ClassifiedFt8Message, Ft8MessageClass as ProtocolFt8MessageClass};
use slotpilot_storage::{
    AcceptOutcome, AcceptedCommand, ReceiveClockHealth as StorageReceiveClockHealth, ReceivePage,
    ReceiveRecord, SequencedReceiveRecord, StorageError, Store,
};
use thiserror::Error;

use crate::{
    DaemonReceiveStore, ProcessorError, ReceiveInhibition, ReceiveLifecycleState, ReceiveSelection,
};

/// Production SQLite adapter that atomically couples receive evidence to its
/// public ordered decode event.
pub struct PublicReceiveStore {
    store: Store,
}

impl PublicReceiveStore {
    /// Opens a migrated production store.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StorageError> {
        Ok(Self {
            store: Store::open(path)?,
        })
    }

    /// Builds an isolated coupled store for tests.
    pub fn in_memory() -> Result<Self, StorageError> {
        Ok(Self {
            store: Store::open_in_memory()?,
        })
    }

    /// Returns the contained store.
    #[must_use]
    pub fn into_inner(self) -> Store {
        self.store
    }
}

impl DaemonReceiveStore for PublicReceiveStore {
    fn record_receive(
        &mut self,
        record: &ReceiveRecord,
    ) -> Result<slotpilot_storage::ReceiveInsertOutcome, StorageError> {
        let suffix = record
            .context()
            .receive_window_id
            .as_str()
            .strip_prefix("rxw_")
            .ok_or(StorageError::InvalidReceiveEventPayload)?;
        let event_id: EventId = format!("evt_{suffix}").parse()?;
        let occurred = record.recorded_utc_millis();
        let commit = self.store.record_receive_with_event_builder(
            record,
            &event_id,
            occurred,
            |sequence| {
                let summary = map_receive_record(&SequencedReceiveRecord {
                    sequence,
                    record: record.clone(),
                });
                serde_json::to_string(&slotpilot_api::EventPayload::ReceiveDecode(summary))
                    .map_err(|_| StorageError::InvalidReceiveEventPayload)
            },
        )?;
        Ok(commit.receive)
    }
}

/// Receive service failure mapped to stable API semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ReceiveApiFault {
    /// Device discovery or exact selection is unavailable.
    #[error("receive device is unavailable")]
    DeviceUnavailable,
    /// Receive is inhibited by explicit health or continuity evidence.
    #[error("receive is inhibited: {0:?}")]
    Inhibited(ReceiveInhibitionKind),
    /// Receive request is invalid for current lifecycle/configuration.
    #[error("receive request is invalid")]
    InvalidRequest,
    /// Durable history could not be read.
    #[error("receive storage is unavailable")]
    StorageUnavailable,
}

/// Daemon-internal service seam consumed by the versioned command processor.
///
/// Mutations receive the stable request identity and must be idempotent within
/// the running process. The durable processor adds cross-restart exact-result
/// replay after the operation returns.
pub trait ReceiveApiPort {
    /// Enumerates bounded exact input choices.
    fn input_devices(&mut self) -> Result<InputDevicePage, ReceiveApiFault>;
    /// Returns current inactive/active/faulted receive status.
    fn status(&mut self) -> Result<ReceiveStatus, ReceiveApiFault>;
    /// Explicitly starts the exact selection.
    fn start(
        &mut self,
        request_id: &RequestId,
        selection: &ApiReceiveSelection,
    ) -> Result<ReceiveStatus, ReceiveApiFault>;
    /// Explicitly stops receive.
    fn stop(&mut self, request_id: &RequestId) -> Result<ReceiveStatus, ReceiveApiFault>;
    /// Reads one bounded committed history page.
    fn history(
        &mut self,
        after_sequence: u64,
        limit: u16,
    ) -> Result<ReceiveHistoryPage, ReceiveApiFault>;
}

/// Full versioned receive command processor with durable mutation replay.
pub struct ReceiveApiProcessor<P> {
    service_instance_id: ServiceInstanceId,
    journal: Store,
    port: P,
}

impl<P: ReceiveApiPort> ReceiveApiProcessor<P> {
    /// Opens a processor over one durable request journal.
    pub fn open(
        path: impl AsRef<Path>,
        service_instance_id: ServiceInstanceId,
        port: P,
    ) -> Result<Self, ProcessorError> {
        Ok(Self {
            service_instance_id,
            journal: Store::open(path)?,
            port,
        })
    }

    /// Builds an isolated processor for fake/replay tests.
    pub fn in_memory(
        service_instance_id: ServiceInstanceId,
        port: P,
    ) -> Result<Self, ProcessorError> {
        Ok(Self {
            service_instance_id,
            journal: Store::open_in_memory()?,
            port,
        })
    }

    /// Returns shared access to the composed port for diagnostics/tests.
    #[must_use]
    pub const fn port(&self) -> &P {
        &self.port
    }

    /// Executes one versioned command with deterministic acceptance time.
    pub fn execute(
        &mut self,
        envelope: CommandEnvelope,
        accepted_utc_millis: i64,
    ) -> Result<ResponseEnvelope, ProcessorError> {
        if envelope.api_version != API_VERSION && envelope.api_version != LEGACY_API_VERSION {
            return Ok(
                slotpilot_api::NoopService::new(self.service_instance_id.clone()).execute(envelope),
            );
        }
        if envelope.api_version == LEGACY_API_VERSION && is_receive_command(&envelope.command) {
            return Ok(api_error(
                envelope.api_version,
                envelope.request_id,
                ErrorCode::CommandUnavailableInVersion,
                ErrorDetails::CommandVersion {
                    minimum_version: API_VERSION,
                },
            ));
        }

        match envelope.command.class() {
            CommandClass::ReadOnly => self.execute_read_only(envelope),
            CommandClass::Mutating => self.execute_mutating(envelope, accepted_utc_millis),
        }
    }

    fn execute_read_only(
        &mut self,
        envelope: CommandEnvelope,
    ) -> Result<ResponseEnvelope, ProcessorError> {
        let api_version = envelope.api_version;
        let request_id = envelope.request_id;
        let outcome = match envelope.command {
            Command::GetCapabilities { supported_versions } => {
                let base = slotpilot_api::NoopService::new(self.service_instance_id.clone())
                    .execute(CommandEnvelope {
                        api_version,
                        request_id: request_id.clone(),
                        command: Command::GetCapabilities { supported_versions },
                    });
                match base.outcome {
                    ResponseOutcome::Success(ResultBody::Capabilities(mut capabilities)) => {
                        capabilities.receive_input = Some(Availability::Available);
                        ResponseOutcome::Success(ResultBody::Capabilities(capabilities))
                    }
                    other => other,
                }
            }
            Command::GetSnapshot => {
                let receive = self.port.status().map_err(api_fault_outcome);
                match receive {
                    Ok(receive) => {
                        ResponseOutcome::Success(ResultBody::Snapshot(StationSnapshot {
                            service_instance_id: self.service_instance_id.clone(),
                            event_cursor: slotpilot_api::EventCursor {
                                service_instance_id: self.service_instance_id.clone(),
                                sequence: 0,
                            },
                            configuration: slotpilot_api::ConfigurationState::NotConfigured,
                            operation: slotpilot_api::OperationState::NotRunning,
                            receive: Some(receive),
                            transmit_authority: Availability::Unavailable,
                        }))
                    }
                    Err(outcome) => outcome,
                }
            }
            Command::ListInputDevices => self
                .port
                .input_devices()
                .and_then(|devices| {
                    devices
                        .validate()
                        .map_err(|_| ReceiveApiFault::InvalidRequest)?;
                    Ok(devices)
                })
                .map(|devices| ResponseOutcome::Success(ResultBody::InputDevices(devices)))
                .unwrap_or_else(api_fault_outcome),
            Command::GetReceiveStatus => self
                .port
                .status()
                .map(|status| ResponseOutcome::Success(ResultBody::ReceiveStatus(status)))
                .unwrap_or_else(api_fault_outcome),
            Command::QueryReceiveHistory {
                after_sequence,
                limit,
            } => {
                if envelope.command.canonical_bytes().is_err() {
                    invalid_request_outcome()
                } else {
                    self.port
                        .history(after_sequence, limit)
                        .and_then(|page| {
                            page.validate()
                                .map_err(|_| ReceiveApiFault::InvalidRequest)?;
                            Ok(page)
                        })
                        .map(|page| ResponseOutcome::Success(ResultBody::ReceiveHistory(page)))
                        .unwrap_or_else(api_fault_outcome)
                }
            }
            Command::NoopMutation { .. } | Command::ReceiveStart { .. } | Command::ReceiveStop => {
                unreachable!("command class is mutating")
            }
        };
        Ok(ResponseEnvelope {
            api_version,
            request_id,
            outcome,
        })
    }

    fn execute_mutating(
        &mut self,
        envelope: CommandEnvelope,
        accepted_utc_millis: i64,
    ) -> Result<ResponseEnvelope, ProcessorError> {
        let canonical = match envelope.command.canonical_bytes() {
            Ok(canonical) => canonical,
            Err(_) => {
                return Ok(ResponseEnvelope {
                    api_version: envelope.api_version,
                    request_id: envelope.request_id,
                    outcome: invalid_request_outcome(),
                });
            }
        };
        if let Some(existing) = self.journal.accepted_command(&envelope.request_id)? {
            if existing.canonical_command == canonical {
                return Ok(serde_json::from_slice(&existing.original_result)?);
            }
            return Ok(api_error(
                envelope.api_version,
                envelope.request_id,
                ErrorCode::RequestIdConflict,
                ErrorDetails::RequestConflict,
            ));
        }

        let outcome = match &envelope.command {
            Command::NoopMutation { marker } => {
                ResponseOutcome::Success(ResultBody::NoopMutationAccepted {
                    marker: marker.clone(),
                })
            }
            Command::ReceiveStart { selection } => self
                .port
                .start(&envelope.request_id, selection)
                .map(|status| {
                    ResponseOutcome::Success(ResultBody::ReceiveStarted(status.lifecycle))
                })
                .unwrap_or_else(api_fault_outcome),
            Command::ReceiveStop => self
                .port
                .stop(&envelope.request_id)
                .map(|status| {
                    ResponseOutcome::Success(ResultBody::ReceiveStopped(status.lifecycle))
                })
                .unwrap_or_else(api_fault_outcome),
            _ => unreachable!("command class is read-only"),
        };
        let response = ResponseEnvelope {
            api_version: envelope.api_version,
            request_id: envelope.request_id.clone(),
            outcome,
        };
        let accepted = AcceptedCommand {
            request_id: envelope.request_id.clone(),
            command_id: CommandId::for_request(&envelope.request_id),
            canonical_command: canonical,
            original_result: serde_json::to_vec(&response)?,
            accepted_utc_millis,
        };
        match self.journal.accept_or_existing(&accepted)? {
            AcceptOutcome::Inserted(_) => Ok(response),
            AcceptOutcome::Existing(existing)
                if existing.canonical_command == accepted.canonical_command =>
            {
                Ok(serde_json::from_slice(&existing.original_result)?)
            }
            AcceptOutcome::Existing(_) => Ok(api_error(
                envelope.api_version,
                envelope.request_id,
                ErrorCode::RequestIdConflict,
                ErrorDetails::RequestConflict,
            )),
        }
    }
}

fn is_receive_command(command: &Command) -> bool {
    matches!(
        command,
        Command::ListInputDevices
            | Command::ReceiveStart { .. }
            | Command::ReceiveStop
            | Command::GetReceiveStatus
            | Command::QueryReceiveHistory { .. }
    )
}

fn invalid_request_outcome() -> ResponseOutcome {
    ResponseOutcome::Error(ApiError {
        code: ErrorCode::InvalidReceiveRequest,
        message: "receive request violates a documented bound".into(),
        retryable: false,
        details: ErrorDetails::ReceiveRequest,
    })
}

fn api_fault_outcome(fault: ReceiveApiFault) -> ResponseOutcome {
    let (code, reason, retryable) = match fault {
        ReceiveApiFault::DeviceUnavailable => (
            ErrorCode::ReceiveUnavailable,
            ReceiveInhibitionKind::DeviceUnavailable,
            true,
        ),
        ReceiveApiFault::Inhibited(reason) => (ErrorCode::ReceiveInhibited, reason, false),
        ReceiveApiFault::InvalidRequest => (
            ErrorCode::InvalidReceiveRequest,
            ReceiveInhibitionKind::ServiceUnavailable,
            false,
        ),
        ReceiveApiFault::StorageUnavailable => (
            ErrorCode::ReceiveUnavailable,
            ReceiveInhibitionKind::StorageFailed,
            true,
        ),
    };
    ResponseOutcome::Error(ApiError {
        code,
        message: fault.to_string(),
        retryable,
        details: ErrorDetails::Receive { reason },
    })
}

fn api_error(
    api_version: u32,
    request_id: RequestId,
    code: ErrorCode,
    details: ErrorDetails,
) -> ResponseEnvelope {
    ResponseEnvelope {
        api_version,
        request_id,
        outcome: ResponseOutcome::Error(ApiError {
            code,
            message: code.as_str().replace('_', " "),
            retryable: false,
            details,
        }),
    }
}

/// Maps an audio descriptor without using its display name as identity.
#[must_use]
pub fn map_input_device(device: &InputDeviceDescriptor) -> InputDevice {
    InputDevice {
        identity: map_device_identity(device.identity()),
        display_name: device.display().name().to_owned(),
        manufacturer: device.display().manufacturer().map(str::to_owned),
        configuration_ranges: device
            .configuration_ranges()
            .iter()
            .copied()
            .map(map_configuration_range)
            .collect(),
    }
}

/// Converts a checked wire selection to the owned audio selection.
pub fn map_receive_selection(
    selection: &ApiReceiveSelection,
) -> Result<ReceiveSelection, ReceiveApiFault> {
    let identity = AudioInputDeviceIdentity::new(
        map_platform_from_api(selection.device_identity.platform),
        selection.device_identity.opaque_id.clone(),
    )
    .map_err(|_| ReceiveApiFault::InvalidRequest)?;
    let configuration = AudioInputConfiguration::new(
        selection.configuration.sample_rate_hz,
        selection.configuration.channels,
        map_sample_format_from_api(selection.configuration.sample_format),
        selection.configuration.selected_channel,
    )
    .map_err(|_| ReceiveApiFault::InvalidRequest)?;
    Ok(ReceiveSelection {
        device_identity: identity,
        configuration,
    })
}

/// Maps daemon-internal lifecycle into stable API state.
#[must_use]
pub fn map_lifecycle(state: ReceiveLifecycleState) -> ReceiveLifecycleSnapshot {
    match state {
        ReceiveLifecycleState::Stopped {
            last_stream_generation,
            ..
        } => ReceiveLifecycleSnapshot::Stopped {
            last_stream_generation,
        },
        ReceiveLifecycleState::Starting { stream_generation } => {
            ReceiveLifecycleSnapshot::Starting {
                stream_generation: stream_generation.get(),
            }
        }
        ReceiveLifecycleState::Receiving { stream_generation } => {
            ReceiveLifecycleSnapshot::Receiving {
                stream_generation: stream_generation.get(),
            }
        }
        ReceiveLifecycleState::Inhibited {
            stream_generation,
            reason,
        } => ReceiveLifecycleSnapshot::Inhibited {
            stream_generation: stream_generation.get(),
            reason: map_inhibition(reason),
        },
        ReceiveLifecycleState::Stopping {
            stream_generation, ..
        } => ReceiveLifecycleSnapshot::Stopping {
            stream_generation: stream_generation.get(),
        },
    }
}

/// Maps one committed storage record into bounded public evidence.
#[must_use]
pub fn map_receive_record(record: &SequencedReceiveRecord) -> ReceiveRecordSummary {
    let context = record.record.context();
    let diagnostics = record.record.diagnostics();
    ReceiveRecordSummary {
        sequence: record.sequence,
        receive_window_id: context.receive_window_id.clone(),
        slot_start_utc_millis: context.slot.start_utc_unix_millis(),
        selection: ApiReceiveSelection {
            device_identity: map_device_identity(&context.device_identity),
            configuration: map_configuration(context.configuration),
        },
        audio: ReceiveAudioHealth {
            latency_millis: diagnostics.audio.latency_millis,
            drift_parts_per_million: diagnostics.audio.drift_parts_per_million,
            overflow_count: diagnostics.audio.overflow_count,
            clipped_sample_count: diagnostics.audio.clipped_sample_count,
            max_callback_delay_millis: diagnostics.audio.max_callback_delay_millis,
        },
        clock: map_storage_clock(diagnostics.clock),
        decodes: record.record.decodes().iter().map(map_decode).collect(),
    }
}

/// Maps one bounded storage page.
#[must_use]
pub fn map_receive_page(page: ReceivePage, after_sequence: u64) -> ReceiveHistoryPage {
    let records: Vec<_> = page.records.iter().map(map_receive_record).collect();
    let next_sequence = records
        .last()
        .map_or(after_sequence, |record| record.sequence);
    ReceiveHistoryPage {
        records,
        next_sequence,
        has_more: page.has_more,
    }
}

/// Maps the latest already-rate-limited spectrum snapshot to one public frame.
#[must_use]
pub fn map_waterfall_snapshot(
    snapshot: &WaterfallSnapshot,
    frame_sequence: u64,
) -> Option<slotpilot_api::WaterfallFrame> {
    let row = snapshot.rows.last()?;
    Some(slotpilot_api::WaterfallFrame {
        stream_generation: row.stream_generation.get(),
        utc_unix_millis: row.start_utc_unix_micros / 1_000,
        frame_sequence,
        coalesced: snapshot.coalesced_publications > 0,
        bins: row
            .bins
            .iter()
            .map(|bin| slotpilot_api::WaterfallBin {
                frequency_millihz: u64::from(bin.frequency.millihertz()),
                magnitude_millidbfs: bin.magnitude.millidecibels_full_scale(),
            })
            .collect(),
    })
}

fn map_decode(decode: &slotpilot_protocol::Ft8Decode) -> Ft8Decode {
    Ft8Decode {
        start_offset_millis: decode.metadata.start_offset_millis,
        audio_frequency_hz: decode.metadata.audio_frequency_hz,
        signal_to_noise_db: decode.metadata.signal_to_noise_db,
        classification: match &decode.message {
            ClassifiedFt8Message::Resolved(value) => Ft8Classification::Resolved {
                canonical_text: value.canonical_text().to_owned(),
                sender: value.sender().to_string(),
                recipient: value.recipient().map(ToString::to_string),
                message_class: map_message_class(value.class()),
            },
            ClassifiedFt8Message::UnresolvedHash(value) => Ft8Classification::UnresolvedHash {
                canonical_text: value.canonical_text().to_owned(),
                detail: value.detail().to_owned(),
            },
            ClassifiedFt8Message::Unsupported(value) => Ft8Classification::Unsupported {
                canonical_text: value.canonical_text().to_owned(),
                detail: value.detail().to_owned(),
            },
            ClassifiedFt8Message::Ambiguous(value) => Ft8Classification::Ambiguous {
                canonical_text: value.canonical_text().to_owned(),
                detail: value.detail().to_owned(),
            },
            ClassifiedFt8Message::FreeText(value) => Ft8Classification::FreeText {
                text: value.text().to_owned(),
            },
        },
    }
}

const fn map_message_class(value: ProtocolFt8MessageClass) -> Ft8MessageClass {
    match value {
        ProtocolFt8MessageClass::GeneralCall => Ft8MessageClass::GeneralCall,
        ProtocolFt8MessageClass::DirectedGrid => Ft8MessageClass::DirectedGrid,
        ProtocolFt8MessageClass::SignalReport => Ft8MessageClass::SignalReport,
        ProtocolFt8MessageClass::RogerSignalReport => Ft8MessageClass::RogerSignalReport,
        ProtocolFt8MessageClass::Roger => Ft8MessageClass::Roger,
        ProtocolFt8MessageClass::Ending73 => Ft8MessageClass::Ending73,
        ProtocolFt8MessageClass::EndingRr73 => Ft8MessageClass::EndingRr73,
    }
}

const fn map_storage_clock(value: StorageReceiveClockHealth) -> ReceiveClockHealth {
    match value {
        StorageReceiveClockHealth::Healthy { mapping_age_millis } => {
            ReceiveClockHealth::Healthy { mapping_age_millis }
        }
        StorageReceiveClockHealth::Unhealthy {
            recovery_progress,
            recovery_required,
            mapping_age_millis,
            ..
        } => ReceiveClockHealth::Unhealthy {
            reason: ReceiveInhibitionKind::ClockUnhealthy,
            recovery_progress,
            recovery_required,
            mapping_age_millis,
        },
    }
}

const fn map_inhibition(value: ReceiveInhibition) -> ReceiveInhibitionKind {
    match value {
        ReceiveInhibition::Input(slotpilot_audio::InputFaultKind::DeviceLost)
        | ReceiveInhibition::Input(slotpilot_audio::InputFaultKind::PermissionDenied)
        | ReceiveInhibition::Input(slotpilot_audio::InputFaultKind::BackendFailure) => {
            ReceiveInhibitionKind::DeviceUnavailable
        }
        ReceiveInhibition::Input(slotpilot_audio::InputFaultKind::Overflow { .. })
        | ReceiveInhibition::WorkerBackpressure => ReceiveInhibitionKind::Overflow,
        ReceiveInhibition::Input(slotpilot_audio::InputFaultKind::Discontinuity(_)) => {
            ReceiveInhibitionKind::Discontinuity
        }
        ReceiveInhibition::Input(
            slotpilot_audio::InputFaultKind::Clipping { .. }
            | slotpilot_audio::InputFaultKind::Drift { .. }
            | slotpilot_audio::InputFaultKind::CallbackDelay { .. },
        )
        | ReceiveInhibition::Timeline(_) => ReceiveInhibitionKind::TimelineInvalid,
        ReceiveInhibition::Clock(_) => ReceiveInhibitionKind::ClockUnhealthy,
        ReceiveInhibition::DecoderFailure => ReceiveInhibitionKind::DecoderFailed,
        ReceiveInhibition::StorageFailure => ReceiveInhibitionKind::StorageFailed,
    }
}

fn map_device_identity(value: &AudioInputDeviceIdentity) -> InputDeviceIdentity {
    InputDeviceIdentity {
        platform: map_platform(value.platform()),
        opaque_id: value.opaque_id().to_owned(),
    }
}

const fn map_configuration(value: AudioInputConfiguration) -> InputConfiguration {
    InputConfiguration {
        sample_rate_hz: value.sample_rate_hz(),
        channels: value.channels(),
        sample_format: map_sample_format(value.sample_format()),
        selected_channel: value.selected_channel(),
    }
}

const fn map_configuration_range(value: AudioInputConfigurationRange) -> InputConfigurationRange {
    InputConfigurationRange {
        min_sample_rate_hz: value.min_sample_rate_hz(),
        max_sample_rate_hz: value.max_sample_rate_hz(),
        channels: value.channels(),
        sample_format: map_sample_format(value.sample_format()),
    }
}

const fn map_platform(value: AudioInputPlatform) -> InputPlatform {
    match value {
        AudioInputPlatform::MacOsCoreAudio => InputPlatform::MacOsCoreAudio,
        AudioInputPlatform::WindowsWasapi => InputPlatform::WindowsWasapi,
        AudioInputPlatform::LinuxAlsa => InputPlatform::LinuxAlsa,
        AudioInputPlatform::LinuxJack => InputPlatform::LinuxJack,
    }
}

const fn map_platform_from_api(value: InputPlatform) -> AudioInputPlatform {
    match value {
        InputPlatform::MacOsCoreAudio => AudioInputPlatform::MacOsCoreAudio,
        InputPlatform::WindowsWasapi => AudioInputPlatform::WindowsWasapi,
        InputPlatform::LinuxAlsa => AudioInputPlatform::LinuxAlsa,
        InputPlatform::LinuxJack => AudioInputPlatform::LinuxJack,
    }
}

const fn map_sample_format(value: AudioInputSampleFormat) -> InputSampleFormat {
    match value {
        AudioInputSampleFormat::Signed8 => InputSampleFormat::Signed8,
        AudioInputSampleFormat::Signed16 => InputSampleFormat::Signed16,
        AudioInputSampleFormat::Signed24 => InputSampleFormat::Signed24,
        AudioInputSampleFormat::Signed32 => InputSampleFormat::Signed32,
        AudioInputSampleFormat::Signed64 => InputSampleFormat::Signed64,
        AudioInputSampleFormat::Unsigned8 => InputSampleFormat::Unsigned8,
        AudioInputSampleFormat::Unsigned16 => InputSampleFormat::Unsigned16,
        AudioInputSampleFormat::Unsigned24 => InputSampleFormat::Unsigned24,
        AudioInputSampleFormat::Unsigned32 => InputSampleFormat::Unsigned32,
        AudioInputSampleFormat::Unsigned64 => InputSampleFormat::Unsigned64,
        AudioInputSampleFormat::Float32 => InputSampleFormat::Float32,
        AudioInputSampleFormat::Float64 => InputSampleFormat::Float64,
    }
}

const fn map_sample_format_from_api(value: InputSampleFormat) -> AudioInputSampleFormat {
    match value {
        InputSampleFormat::Signed8 => AudioInputSampleFormat::Signed8,
        InputSampleFormat::Signed16 => AudioInputSampleFormat::Signed16,
        InputSampleFormat::Signed24 => AudioInputSampleFormat::Signed24,
        InputSampleFormat::Signed32 => AudioInputSampleFormat::Signed32,
        InputSampleFormat::Signed64 => AudioInputSampleFormat::Signed64,
        InputSampleFormat::Unsigned8 => AudioInputSampleFormat::Unsigned8,
        InputSampleFormat::Unsigned16 => AudioInputSampleFormat::Unsigned16,
        InputSampleFormat::Unsigned24 => AudioInputSampleFormat::Unsigned24,
        InputSampleFormat::Unsigned32 => AudioInputSampleFormat::Unsigned32,
        InputSampleFormat::Unsigned64 => AudioInputSampleFormat::Unsigned64,
        InputSampleFormat::Float32 => AudioInputSampleFormat::Float32,
        InputSampleFormat::Float64 => AudioInputSampleFormat::Float64,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use slotpilot_api::{
        CommandEnvelope, ErrorCode, InputConfiguration, InputDeviceIdentity, InputPlatform,
        InputSampleFormat, ReceiveLifecycleSnapshot, ResponseOutcome,
    };

    use super::*;

    struct FakePort {
        status: ReceiveStatus,
        mutations: usize,
        results: BTreeMap<RequestId, ReceiveStatus>,
    }

    impl Default for FakePort {
        fn default() -> Self {
            Self {
                status: ReceiveStatus::stopped(),
                mutations: 0,
                results: BTreeMap::new(),
            }
        }
    }

    impl ReceiveApiPort for FakePort {
        fn input_devices(&mut self) -> Result<InputDevicePage, ReceiveApiFault> {
            Ok(InputDevicePage {
                devices: vec![InputDevice {
                    identity: selection().device_identity,
                    display_name: "Duplicate Display Name".into(),
                    manufacturer: None,
                    configuration_ranges: vec![InputConfigurationRange {
                        min_sample_rate_hz: 12_000,
                        max_sample_rate_hz: 48_000,
                        channels: 2,
                        sample_format: InputSampleFormat::Signed16,
                    }],
                }],
            })
        }

        fn status(&mut self) -> Result<ReceiveStatus, ReceiveApiFault> {
            Ok(self.status.clone())
        }

        fn start(
            &mut self,
            request_id: &RequestId,
            selection: &ApiReceiveSelection,
        ) -> Result<ReceiveStatus, ReceiveApiFault> {
            if let Some(existing) = self.results.get(request_id) {
                return Ok(existing.clone());
            }
            self.mutations += 1;
            self.status = ReceiveStatus {
                lifecycle: ReceiveLifecycleSnapshot::Receiving {
                    stream_generation: 1,
                },
                selection: Some(selection.clone()),
                audio: None,
                clock: None,
            };
            self.results.insert(request_id.clone(), self.status.clone());
            Ok(self.status.clone())
        }

        fn stop(&mut self, request_id: &RequestId) -> Result<ReceiveStatus, ReceiveApiFault> {
            if let Some(existing) = self.results.get(request_id) {
                return Ok(existing.clone());
            }
            self.mutations += 1;
            self.status = ReceiveStatus::stopped();
            self.results.insert(request_id.clone(), self.status.clone());
            Ok(self.status.clone())
        }

        fn history(
            &mut self,
            after_sequence: u64,
            limit: u16,
        ) -> Result<ReceiveHistoryPage, ReceiveApiFault> {
            if limit == 0 || limit > slotpilot_api::MAX_RECEIVE_HISTORY_PAGE {
                return Err(ReceiveApiFault::InvalidRequest);
            }
            Ok(ReceiveHistoryPage {
                records: Vec::new(),
                next_sequence: after_sequence,
                has_more: false,
            })
        }
    }

    fn instance() -> ServiceInstanceId {
        "svc_phase2api".parse().unwrap()
    }

    fn selection() -> ApiReceiveSelection {
        ApiReceiveSelection {
            device_identity: InputDeviceIdentity {
                platform: InputPlatform::MacOsCoreAudio,
                opaque_id: "stable-device-1".into(),
            },
            configuration: InputConfiguration {
                sample_rate_hz: 48_000,
                channels: 2,
                sample_format: InputSampleFormat::Signed16,
                selected_channel: 1,
            },
        }
    }

    fn command(id: &str, api_version: u32, command: Command) -> CommandEnvelope {
        CommandEnvelope {
            api_version,
            request_id: id.parse().unwrap(),
            command,
        }
    }

    #[test]
    fn version_one_remains_compatible_and_receive_requires_version_two() {
        let mut processor =
            ReceiveApiProcessor::in_memory(instance(), FakePort::default()).unwrap();
        let legacy = processor
            .execute(
                command("req_legacy001", LEGACY_API_VERSION, Command::GetSnapshot),
                1,
            )
            .unwrap();
        assert_eq!(legacy.api_version, LEGACY_API_VERSION);
        let rejected = processor
            .execute(
                command(
                    "req_legacy002",
                    LEGACY_API_VERSION,
                    Command::GetReceiveStatus,
                ),
                2,
            )
            .unwrap();
        assert!(matches!(
            rejected.outcome,
            ResponseOutcome::Error(ApiError {
                code: ErrorCode::CommandUnavailableInVersion,
                ..
            })
        ));
    }

    #[test]
    fn start_stop_replay_and_conflict_are_durable_and_bounded() {
        let mut processor =
            ReceiveApiProcessor::in_memory(instance(), FakePort::default()).unwrap();
        let start = command(
            "req_start0001",
            API_VERSION,
            Command::ReceiveStart {
                selection: selection(),
            },
        );
        let first = processor.execute(start.clone(), 10).unwrap();
        let replay = processor.execute(start, 99).unwrap();
        assert_eq!(first, replay);
        assert_eq!(processor.port().mutations, 1);
        let conflict = processor
            .execute(
                command("req_start0001", API_VERSION, Command::ReceiveStop),
                11,
            )
            .unwrap();
        assert!(matches!(
            conflict.outcome,
            ResponseOutcome::Error(ApiError {
                code: ErrorCode::RequestIdConflict,
                ..
            })
        ));
        assert_eq!(processor.port().mutations, 1);

        let invalid = processor
            .execute(
                command(
                    "req_history01",
                    API_VERSION,
                    Command::QueryReceiveHistory {
                        after_sequence: 0,
                        limit: slotpilot_api::MAX_RECEIVE_HISTORY_PAGE + 1,
                    },
                ),
                12,
            )
            .unwrap();
        assert!(matches!(
            invalid.outcome,
            ResponseOutcome::Error(ApiError {
                code: ErrorCode::InvalidReceiveRequest,
                ..
            })
        ));
    }

    #[test]
    fn journal_failure_retries_through_port_identity_without_duplicate_mutation() {
        let mut processor =
            ReceiveApiProcessor::in_memory(instance(), FakePort::default()).unwrap();
        let start = command(
            "req_start0002",
            API_VERSION,
            Command::ReceiveStart {
                selection: selection(),
            },
        );
        assert!(processor.execute(start.clone(), -1).is_err());
        assert_eq!(processor.port().mutations, 1);
        let response = processor.execute(start, 10).unwrap();
        assert!(matches!(
            response.outcome,
            ResponseOutcome::Success(ResultBody::ReceiveStarted(_))
        ));
        assert_eq!(processor.port().mutations, 1);
    }

    #[test]
    fn restart_snapshot_is_inactive_even_when_old_start_result_replays() {
        let path = std::env::temp_dir().join(format!(
            "slotpilot-receive-api-{}-{}.sqlite3",
            std::process::id(),
            34
        ));
        let start = command(
            "req_start0003",
            API_VERSION,
            Command::ReceiveStart {
                selection: selection(),
            },
        );
        let original = {
            let mut processor =
                ReceiveApiProcessor::open(&path, instance(), FakePort::default()).unwrap();
            processor.execute(start.clone(), 10).unwrap()
        };
        let mut restarted =
            ReceiveApiProcessor::open(&path, "svc_phase2new".parse().unwrap(), FakePort::default())
                .unwrap();
        assert_eq!(restarted.execute(start, 20).unwrap(), original);
        let snapshot = restarted
            .execute(
                command("req_snapshot2", API_VERSION, Command::GetSnapshot),
                21,
            )
            .unwrap();
        assert!(matches!(
            snapshot.outcome,
            ResponseOutcome::Success(ResultBody::Snapshot(StationSnapshot {
                receive: Some(ReceiveStatus {
                    lifecycle: ReceiveLifecycleSnapshot::Stopped { .. },
                    ..
                }),
                ..
            }))
        ));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn discovery_identity_is_separate_from_duplicate_display_metadata() {
        let mut processor =
            ReceiveApiProcessor::in_memory(instance(), FakePort::default()).unwrap();
        let response = processor
            .execute(
                command("req_devices01", API_VERSION, Command::ListInputDevices),
                1,
            )
            .unwrap();
        let ResponseOutcome::Success(ResultBody::InputDevices(page)) = response.outcome else {
            panic!("expected devices");
        };
        assert_eq!(page.devices[0].display_name, "Duplicate Display Name");
        assert_eq!(
            page.devices[0].identity.opaque_id,
            "stable-device-1".to_owned()
        );
    }
}
