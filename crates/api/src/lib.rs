//! Versioned command, result, error, capability, and snapshot contracts.
//!
//! Versions 1 and 2 use JSON objects. Unknown additive object fields are ignored
//! during deserialization, while unknown command/result variants and
//! incompatible API versions fail explicitly. Every daemon process receives a
//! new [`ServiceInstanceId`]; the identity grants no authority and is not
//! restored after restart.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use slotpilot_domain::{EventId, RequestId, ServiceInstanceId};
use thiserror::Error;

mod receive;

pub use receive::*;

/// Current API version. Version 2 adds receive-only station behavior.
pub const API_VERSION: u32 = 2;
/// Legacy Phase 0 API version retained for its original commands and fixtures.
pub const LEGACY_API_VERSION: u32 = 1;
/// Ordered service-supported versions.
pub const SUPPORTED_API_VERSIONS: [u32; 2] = [API_VERSION, LEGACY_API_VERSION];

/// Maximum number of version entries accepted in a negotiation request.
pub const MAX_NEGOTIATION_VERSIONS: usize = 16;

/// Maximum events returned by one replay exchange.
pub const MAX_REPLAY_EVENTS: u16 = 256;

/// Position in one daemon event generation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventCursor {
    /// Daemon generation that owns the sequence.
    pub service_instance_id: ServiceInstanceId,
    /// Last event already observed; zero means before the first event.
    pub sequence: u64,
}

/// One ordered, versioned daemon event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventEnvelope {
    /// Version used to encode this event.
    pub api_version: u32,
    /// Generation that emitted the event.
    pub service_instance_id: ServiceInstanceId,
    /// Monotonic database sequence within the retained event log.
    pub sequence: u64,
    /// Stable event identity.
    pub event_id: EventId,
    /// Event occurrence time in UTC milliseconds since the Unix epoch.
    pub occurred_utc_millis: i64,
    /// Typed known event or safely opaque unknown event.
    pub event: EventPayload,
}

/// Phase 0 event payloads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventPayload {
    /// Side-effect-free marker used only to exercise observation plumbing.
    Phase0Notice {
        /// Bounded human-readable marker with no control semantics.
        message: String,
    },
    /// Receive lifecycle changed.
    ReceiveLifecycle(ReceiveLifecycleSnapshot),
    /// One durable receive record became observable.
    ReceiveDecode(ReceiveRecordSummary),
    /// Bounded current receive health.
    ReceiveHealth(ReceiveHealthSnapshot),
    /// Capture continuity was explicitly invalidated.
    ReceiveDiscontinuity(ReceiveDiscontinuity),
    /// One bounded, rate-limited waterfall frame.
    WaterfallFrame(WaterfallFrame),
    /// Event kind unknown to this client version.
    ///
    /// Clients may display or record this value, but must not derive state
    /// transitions or authority from its string kind or fields.
    Unknown {
        /// Unrecognized symbolic kind.
        kind: String,
        /// Complete payload object excluding `kind`.
        fields: serde_json::Map<String, serde_json::Value>,
    },
}

impl Serialize for EventPayload {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let object = match self {
            Self::Phase0Notice { message } => {
                let mut object = serde_json::Map::new();
                object.insert("kind".into(), "phase0_notice".into());
                object.insert("message".into(), message.clone().into());
                object
            }
            Self::ReceiveLifecycle(value) => {
                event_object("receive_lifecycle", value).map_err(serde::ser::Error::custom)?
            }
            Self::ReceiveDecode(value) => {
                event_object("receive_decode", value).map_err(serde::ser::Error::custom)?
            }
            Self::ReceiveHealth(value) => {
                event_object("receive_health", value).map_err(serde::ser::Error::custom)?
            }
            Self::ReceiveDiscontinuity(value) => {
                event_object("receive_discontinuity", value).map_err(serde::ser::Error::custom)?
            }
            Self::WaterfallFrame(value) => {
                event_object("waterfall_frame", value).map_err(serde::ser::Error::custom)?
            }
            Self::Unknown { kind, fields } => {
                let mut object = fields.clone();
                object.insert("kind".into(), kind.clone().into());
                object
            }
        };
        object.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for EventPayload {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let mut object = serde_json::Map::<String, serde_json::Value>::deserialize(deserializer)?;
        let kind = object
            .remove("kind")
            .and_then(|value| value.as_str().map(str::to_owned))
            .ok_or_else(|| serde::de::Error::custom("event payload requires string kind"))?;
        if kind == "phase0_notice" {
            let message = object
                .remove("message")
                .and_then(|value| value.as_str().map(str::to_owned))
                .ok_or_else(|| serde::de::Error::custom("phase0_notice requires string message"))?;
            return Ok(Self::Phase0Notice { message });
        }
        let known = serde_json::Value::Object(object.clone());
        match kind.as_str() {
            "receive_lifecycle" => serde_json::from_value(known)
                .map(Self::ReceiveLifecycle)
                .map_err(serde::de::Error::custom),
            "receive_decode" => serde_json::from_value(known)
                .map(Self::ReceiveDecode)
                .map_err(serde::de::Error::custom),
            "receive_health" => serde_json::from_value(known)
                .map(Self::ReceiveHealth)
                .map_err(serde::de::Error::custom),
            "receive_discontinuity" => serde_json::from_value(known)
                .map(Self::ReceiveDiscontinuity)
                .map_err(serde::de::Error::custom),
            "waterfall_frame" => serde_json::from_value(known)
                .map(Self::WaterfallFrame)
                .map_err(serde::de::Error::custom),
            _ => Ok(Self::Unknown {
                kind,
                fields: object,
            }),
        }
    }
}

impl EventPayload {
    /// Validates event-specific collection and text bounds before publication.
    pub fn validate(&self) -> Result<(), WireBoundError> {
        match self {
            Self::Phase0Notice { message } if message.len() <= 512 => Ok(()),
            Self::ReceiveLifecycle(_) | Self::ReceiveHealth(_) | Self::ReceiveDiscontinuity(_) => {
                Ok(())
            }
            Self::ReceiveDecode(record) => record.validate(),
            Self::WaterfallFrame(frame) => frame.validate(),
            Self::Unknown { kind, fields }
                if !kind.is_empty() && kind.len() <= 64 && fields.len() <= 64 =>
            {
                Ok(())
            }
            _ => Err(WireBoundError::Exceeded),
        }
    }
}

fn event_object<T: Serialize>(
    kind: &str,
    value: &T,
) -> Result<serde_json::Map<String, serde_json::Value>, serde_json::Error> {
    let mut object = match serde_json::to_value(value)? {
        serde_json::Value::Object(object) => object,
        _ => serde_json::Map::new(),
    };
    object.insert("kind".into(), kind.into());
    Ok(object)
}

/// One bounded replay request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubscriptionRequest {
    /// API version requested by the client.
    pub api_version: u32,
    /// Last event already observed, or none to begin at retained history.
    pub after: Option<EventCursor>,
    /// Requested page size, from 1 through [`MAX_REPLAY_EVENTS`].
    pub limit: u16,
}

/// One bounded replay response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubscriptionResponse {
    /// Version used to encode this response.
    pub api_version: u32,
    /// Typed replay result.
    #[serde(flatten)]
    pub outcome: SubscriptionOutcome,
}

/// Stable replay outcomes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", content = "payload", rename_all = "snake_case")]
pub enum SubscriptionOutcome {
    /// A bounded page, possibly empty.
    Events {
        /// Ordered retained events after the requested cursor.
        events: Vec<EventEnvelope>,
        /// Cursor through the final returned event, or the request cursor when empty.
        next_cursor: EventCursor,
        /// More retained events are immediately available.
        has_more: bool,
    },
    /// Requested cursor predates retained history.
    CursorGap {
        /// Cursor supplied by the client.
        requested: EventCursor,
        /// Earliest cursor from which replay is possible.
        earliest_available: EventCursor,
    },
    /// Requested cursor is ahead of committed history.
    CursorUnavailable {
        /// Cursor supplied by the client.
        requested: EventCursor,
        /// Latest committed cursor.
        latest_available: EventCursor,
    },
    /// Cursor belongs to another daemon generation.
    IncompatibleGeneration {
        /// Generation supplied by the client.
        requested_service_instance_id: ServiceInstanceId,
        /// Current generation.
        current_service_instance_id: ServiceInstanceId,
    },
    /// Request version or page bound is invalid.
    InvalidRequest {
        /// Stable symbolic reason.
        reason: SubscriptionInvalidReason,
    },
}

/// Stable replay request validation failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubscriptionInvalidReason {
    /// Requested API version is not supported.
    IncompatibleApiVersion,
    /// Limit was zero or exceeded [`MAX_REPLAY_EVENTS`].
    InvalidLimit,
}

/// A bounded command submitted by a client.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandEnvelope {
    /// Version requested for this command and response.
    pub api_version: u32,
    /// Stable client identity for correlation and future retry semantics.
    pub request_id: RequestId,
    /// Typed service command.
    pub command: Command,
}

/// Read-only commands supported by the no-op Phase 0 service.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Command {
    /// Negotiate a mutually supported API version and inspect capabilities.
    GetCapabilities {
        /// Versions the client can consume, ordered by client preference.
        supported_versions: Vec<u32>,
    },
    /// Obtain the deterministic no-op station snapshot.
    GetSnapshot,
    /// Persist a bounded marker solely to exercise durable retry semantics.
    ///
    /// This command has no station, hardware, logging, or external side effect.
    NoopMutation {
        /// Opaque marker retained only in the original result.
        marker: String,
    },
    /// Enumerate bounded input devices and exact configurations.
    ListInputDevices,
    /// Start receive on one exact stable identity/configuration.
    ReceiveStart {
        /// Explicit selection; no default or display-name fallback exists.
        selection: ReceiveSelection,
    },
    /// Stop receive and release its input resources.
    ReceiveStop,
    /// Read current receive lifecycle and health.
    GetReceiveStatus,
    /// Query one bounded page of durable receive evidence.
    QueryReceiveHistory {
        /// Last global receive sequence already observed.
        after_sequence: u64,
        /// Requested page size.
        limit: u16,
    },
}

/// Whether a command is evaluated afresh or journaled for idempotent replay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandClass {
    /// Observational commands are not retained in the request journal.
    ReadOnly,
    /// Mutating commands require durable same-ID replay/conflict handling.
    Mutating,
}

/// Failure producing a bounded canonical command identity.
#[derive(Debug, Error)]
pub enum CanonicalizationError {
    /// A no-op marker exceeded the bounded wire contract.
    #[error("no-op marker must not exceed 128 bytes")]
    MarkerTooLong,
    /// A receive selection or page request violated a documented bound.
    #[error("receive command contains an invalid or unbounded value")]
    InvalidReceiveCommand,
    /// Stable JSON serialization unexpectedly failed.
    #[error("failed to serialize canonical command: {0}")]
    Serialization(#[from] serde_json::Error),
}

impl Command {
    /// Returns the retry classification for this command.
    #[must_use]
    pub const fn class(&self) -> CommandClass {
        match self {
            Self::GetCapabilities { .. }
            | Self::GetSnapshot
            | Self::ListInputDevices
            | Self::GetReceiveStatus
            | Self::QueryReceiveHistory { .. } => CommandClass::ReadOnly,
            Self::NoopMutation { .. } | Self::ReceiveStart { .. } | Self::ReceiveStop => {
                CommandClass::Mutating
            }
        }
    }

    /// Returns a deterministic byte identity for semantic command comparison.
    ///
    /// Typed serialization preserves every field and variant. Object key order
    /// cannot affect identity because callers never provide an untyped object.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, CanonicalizationError> {
        if let Self::NoopMutation { marker } = self
            && marker.len() > 128
        {
            return Err(CanonicalizationError::MarkerTooLong);
        }
        match self {
            Self::ReceiveStart { selection } => selection.validate()?,
            Self::QueryReceiveHistory { limit, .. }
                if *limit == 0 || *limit > MAX_RECEIVE_HISTORY_PAGE =>
            {
                return Err(CanonicalizationError::InvalidReceiveCommand);
            }
            _ => {}
        }
        Ok(serde_json::to_vec(self)?)
    }
}

/// A response correlated to the original request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResponseEnvelope {
    /// Version used to encode the response.
    pub api_version: u32,
    /// Original request identity.
    pub request_id: RequestId,
    /// Success result or structured stable error.
    #[serde(flatten)]
    pub outcome: ResponseOutcome,
}

/// Success or failure payload for a service response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", content = "payload", rename_all = "snake_case")]
pub enum ResponseOutcome {
    /// A typed successful result.
    Success(ResultBody),
    /// A stable symbolic service failure.
    Error(ApiError),
}

/// Successful no-op service results.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum ResultBody {
    /// Negotiated service capabilities.
    Capabilities(Capabilities),
    /// Current bounded station snapshot.
    Snapshot(StationSnapshot),
    /// Durable acceptance of the side-effect-free Phase 0 mutation.
    NoopMutationAccepted {
        /// Original marker, proving exact result replay.
        marker: String,
    },
    /// Bounded input discovery results.
    InputDevices(InputDevicePage),
    /// Receive start completed.
    ReceiveStarted(ReceiveLifecycleSnapshot),
    /// Receive stop completed.
    ReceiveStopped(ReceiveLifecycleSnapshot),
    /// Current receive-only state and health.
    ReceiveStatus(ReceiveStatus),
    /// Bounded durable receive-history page.
    ReceiveHistory(ReceiveHistoryPage),
}

/// Negotiated capabilities of the Phase 0 service.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capabilities {
    /// Version selected for subsequent commands.
    pub negotiated_version: u32,
    /// Complete bounded set of versions the service supports.
    pub supported_versions: Vec<u32>,
    /// Current daemon process generation.
    pub service_instance_id: ServiceInstanceId,
    /// Explicitly unavailable live capabilities.
    pub station_control: Availability,
    /// Receive-only input control, present in API version 2.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receive_input: Option<Availability>,
    /// Explicitly unavailable transmit authority.
    pub transmit_authority: Availability,
}

/// Whether a capability exists in the current implementation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Availability {
    /// The capability is implemented in this API/service composition.
    Available,
    /// The capability is not implemented or configured.
    Unavailable,
}

/// Deterministic bounded snapshot returned by the no-op service.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StationSnapshot {
    /// Current daemon process generation.
    pub service_instance_id: ServiceInstanceId,
    /// Event cursor compatible with this snapshot.
    pub event_cursor: EventCursor,
    /// Station configuration state.
    pub configuration: ConfigurationState,
    /// Operating-session state.
    pub operation: OperationState,
    /// Receive-only state, present in API version 2.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receive: Option<ReceiveStatus>,
    /// Transmit authority state; Phase 0 is always unavailable.
    pub transmit_authority: Availability,
}

/// Station configuration state represented by the no-op snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigurationState {
    /// No station context has been configured.
    NotConfigured,
}

/// Operating state represented by the no-op snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationState {
    /// No operating session is running.
    NotRunning,
}

/// Stable symbolic service error.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiError {
    /// Code on which automation may branch.
    pub code: ErrorCode,
    /// Human-oriented message that is not a stable automation contract.
    pub message: String,
    /// Whether retrying the same request without changes may succeed.
    pub retryable: bool,
    /// Typed structured detail appropriate to the code.
    pub details: ErrorDetails,
}

/// Stable symbolic API error codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// Client and service have no compatible API version.
    IncompatibleApiVersion,
    /// A negotiation request exceeded its documented bound.
    NegotiationTooLarge,
    /// A request identity was reused for a different canonical mutation.
    RequestIdConflict,
    /// A mutating command bypassed the required durable processor.
    RequestJournalRequired,
    /// A version-1 envelope attempted a version-2 receive command.
    CommandUnavailableInVersion,
    /// Receive input or service is unavailable.
    ReceiveUnavailable,
    /// Receive is inhibited by typed health or continuity evidence.
    ReceiveInhibited,
    /// A receive command violated a documented bound.
    InvalidReceiveRequest,
}

impl ErrorCode {
    /// Returns the stable symbolic wire code.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::IncompatibleApiVersion => "incompatible_api_version",
            Self::NegotiationTooLarge => "negotiation_too_large",
            Self::RequestIdConflict => "request_id_conflict",
            Self::RequestJournalRequired => "request_journal_required",
            Self::CommandUnavailableInVersion => "command_unavailable_in_version",
            Self::ReceiveUnavailable => "receive_unavailable",
            Self::ReceiveInhibited => "receive_inhibited",
            Self::InvalidReceiveRequest => "invalid_receive_request",
        }
    }
}

/// Structured details associated with a service error.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ErrorDetails {
    /// Version mismatch details.
    VersionMismatch {
        /// Version placed in the command envelope.
        requested_version: u32,
        /// Bounded service-supported versions.
        supported_versions: Vec<u32>,
    },
    /// Negotiation request bound details.
    NegotiationLimit {
        /// Maximum supported entries.
        maximum_versions: usize,
        /// Entries supplied by the client.
        received_versions: usize,
    },
    /// Request reuse did not match the originally accepted command.
    RequestConflict,
    /// The in-process read-only seam received a mutating command.
    JournalRequired,
    /// Command requires a newer negotiated version.
    CommandVersion {
        /// Minimum version containing the command.
        minimum_version: u32,
    },
    /// Receive failure/inhibition details.
    Receive {
        /// Stable typed receive reason.
        reason: ReceiveInhibitionKind,
    },
    /// A bounded receive value was invalid.
    ReceiveRequest,
}

/// Failure constructing the in-process no-op service seam.
#[derive(Debug, Error)]
pub enum ServiceError {
    /// The supplied service-instance identity was invalid.
    #[error("invalid service instance identity: {0}")]
    InvalidInstance(#[from] slotpilot_domain::IdError),
}

/// Minimal in-process service seam used to validate the Phase 0 contract.
///
/// This type performs no I/O, persistence, station operation, or hardware
/// access. Constructing a new value with a new identity models daemon restart.
#[derive(Debug, Clone)]
pub struct NoopService {
    instance_id: ServiceInstanceId,
}

impl NoopService {
    /// Creates one process generation from an externally generated identity.
    #[must_use]
    pub fn new(instance_id: ServiceInstanceId) -> Self {
        Self { instance_id }
    }

    /// Executes one bounded, read-only no-op command.
    #[must_use]
    pub fn execute(&self, envelope: CommandEnvelope) -> ResponseEnvelope {
        let request_id = envelope.request_id;
        if !SUPPORTED_API_VERSIONS.contains(&envelope.api_version) {
            return error_response(
                request_id,
                ErrorCode::IncompatibleApiVersion,
                "client and service API versions are incompatible",
                ErrorDetails::VersionMismatch {
                    requested_version: envelope.api_version,
                    supported_versions: SUPPORTED_API_VERSIONS.to_vec(),
                },
            );
        }

        let outcome = match envelope.command {
            Command::GetCapabilities { supported_versions } => {
                if supported_versions.len() > MAX_NEGOTIATION_VERSIONS {
                    ResponseOutcome::Error(ApiError {
                        code: ErrorCode::NegotiationTooLarge,
                        message: "version negotiation request exceeds the supported bound".into(),
                        retryable: false,
                        details: ErrorDetails::NegotiationLimit {
                            maximum_versions: MAX_NEGOTIATION_VERSIONS,
                            received_versions: supported_versions.len(),
                        },
                    })
                } else if let Some(negotiated_version) = supported_versions
                    .iter()
                    .copied()
                    .find(|version| SUPPORTED_API_VERSIONS.contains(version))
                {
                    ResponseOutcome::Success(ResultBody::Capabilities(Capabilities {
                        negotiated_version,
                        supported_versions: SUPPORTED_API_VERSIONS.to_vec(),
                        service_instance_id: self.instance_id.clone(),
                        station_control: Availability::Unavailable,
                        receive_input: (negotiated_version >= API_VERSION)
                            .then_some(Availability::Unavailable),
                        transmit_authority: Availability::Unavailable,
                    }))
                } else {
                    ResponseOutcome::Error(ApiError {
                        code: ErrorCode::IncompatibleApiVersion,
                        message: "client and service API versions are incompatible".into(),
                        retryable: false,
                        details: ErrorDetails::VersionMismatch {
                            requested_version: envelope.api_version,
                            supported_versions: SUPPORTED_API_VERSIONS.to_vec(),
                        },
                    })
                }
            }
            Command::GetSnapshot => {
                ResponseOutcome::Success(ResultBody::Snapshot(StationSnapshot {
                    service_instance_id: self.instance_id.clone(),
                    event_cursor: EventCursor {
                        service_instance_id: self.instance_id.clone(),
                        sequence: 0,
                    },
                    configuration: ConfigurationState::NotConfigured,
                    operation: OperationState::NotRunning,
                    receive: (envelope.api_version >= API_VERSION)
                        .then_some(ReceiveStatus::stopped()),
                    transmit_authority: Availability::Unavailable,
                }))
            }
            Command::NoopMutation { .. } => ResponseOutcome::Error(ApiError {
                code: ErrorCode::RequestJournalRequired,
                message: "mutating commands require the durable request journal".into(),
                retryable: false,
                details: ErrorDetails::JournalRequired,
            }),
            Command::ListInputDevices
            | Command::ReceiveStart { .. }
            | Command::ReceiveStop
            | Command::GetReceiveStatus
            | Command::QueryReceiveHistory { .. } => ResponseOutcome::Error(ApiError {
                code: if envelope.api_version < API_VERSION {
                    ErrorCode::CommandUnavailableInVersion
                } else {
                    ErrorCode::ReceiveUnavailable
                },
                message: "receive commands require the daemon receive service".into(),
                retryable: false,
                details: if envelope.api_version < API_VERSION {
                    ErrorDetails::CommandVersion {
                        minimum_version: API_VERSION,
                    }
                } else {
                    ErrorDetails::Receive {
                        reason: ReceiveInhibitionKind::ServiceUnavailable,
                    }
                },
            }),
        };

        ResponseEnvelope {
            api_version: envelope.api_version,
            request_id,
            outcome,
        }
    }
}

fn error_response(
    request_id: RequestId,
    code: ErrorCode,
    message: &str,
    details: ErrorDetails,
) -> ResponseEnvelope {
    ResponseEnvelope {
        api_version: API_VERSION,
        request_id,
        outcome: ResponseOutcome::Error(ApiError {
            code,
            message: message.into(),
            retryable: false,
            details,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn service(id: &str) -> NoopService {
        NoopService::new(id.parse().unwrap())
    }

    fn request(command: Command) -> CommandEnvelope {
        CommandEnvelope {
            api_version: API_VERSION,
            request_id: "req_01jabcde9".parse().unwrap(),
            command,
        }
    }

    #[test]
    fn negotiates_capabilities_and_returns_noop_snapshot() {
        let service = service("svc_01jabcde9");
        let capabilities = service.execute(request(Command::GetCapabilities {
            supported_versions: vec![API_VERSION],
        }));
        assert!(matches!(
            capabilities.outcome,
            ResponseOutcome::Success(ResultBody::Capabilities(_))
        ));

        let snapshot = service.execute(request(Command::GetSnapshot));
        assert!(matches!(
            snapshot.outcome,
            ResponseOutcome::Success(ResultBody::Snapshot(StationSnapshot {
                configuration: ConfigurationState::NotConfigured,
                operation: OperationState::NotRunning,
                transmit_authority: Availability::Unavailable,
                ..
            }))
        ));
    }

    #[test]
    fn incompatible_and_oversized_negotiation_are_structured() {
        let service = service("svc_01jabcde9");
        let incompatible = service.execute(CommandEnvelope {
            api_version: 99,
            request_id: "req_01jabcde9".parse().unwrap(),
            command: Command::GetSnapshot,
        });
        assert!(matches!(
            incompatible.outcome,
            ResponseOutcome::Error(ApiError {
                code: ErrorCode::IncompatibleApiVersion,
                ..
            })
        ));

        let oversized = service.execute(request(Command::GetCapabilities {
            supported_versions: vec![API_VERSION; MAX_NEGOTIATION_VERSIONS + 1],
        }));
        assert!(matches!(
            oversized.outcome,
            ResponseOutcome::Error(ApiError {
                code: ErrorCode::NegotiationTooLarge,
                ..
            })
        ));
    }

    #[test]
    fn restart_generation_changes_without_restoring_state() {
        let first = service("svc_01jabcde9").execute(request(Command::GetSnapshot));
        let restarted = service("svc_01jabcdf0").execute(request(Command::GetSnapshot));
        assert_ne!(first, restarted);
        for response in [first, restarted] {
            assert!(matches!(
                response.outcome,
                ResponseOutcome::Success(ResultBody::Snapshot(StationSnapshot {
                    operation: OperationState::NotRunning,
                    transmit_authority: Availability::Unavailable,
                    ..
                }))
            ));
        }
    }

    #[test]
    fn additive_unknown_fields_are_ignored() {
        let fixture = r#"{
          "api_version": 1,
          "request_id": "req_01jabcde9",
          "future_envelope_field": true,
          "command": {
            "kind": "get_snapshot",
            "future_command_field": {"nested": true}
          }
        }"#;
        let parsed: CommandEnvelope = serde_json::from_str(fixture).unwrap();
        assert_eq!(parsed.command, Command::GetSnapshot);
    }

    #[test]
    fn command_classification_and_canonicalization_are_explicit() {
        assert_eq!(Command::GetSnapshot.class(), CommandClass::ReadOnly);
        let alpha = Command::NoopMutation {
            marker: "alpha".into(),
        };
        let beta = Command::NoopMutation {
            marker: "beta".into(),
        };
        assert_eq!(alpha.class(), CommandClass::Mutating);
        assert_ne!(
            alpha.canonical_bytes().unwrap(),
            beta.canonical_bytes().unwrap()
        );
        assert_eq!(
            alpha.canonical_bytes().unwrap(),
            alpha.canonical_bytes().unwrap()
        );
        assert!(matches!(
            Command::NoopMutation {
                marker: "x".repeat(129)
            }
            .canonical_bytes(),
            Err(CanonicalizationError::MarkerTooLong)
        ));
    }
}
