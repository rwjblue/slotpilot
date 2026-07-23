//! Versioned command, result, error, capability, and snapshot contracts.
//!
//! Version 1 uses JSON objects. Unknown additive object fields are ignored
//! during deserialization, while unknown command/result variants and
//! incompatible API versions fail explicitly. Every daemon process receives a
//! new [`ServiceInstanceId`]; the identity grants no authority and is not
//! restored after restart.

use serde::{Deserialize, Serialize};
use slotpilot_domain::{RequestId, ServiceInstanceId};
use thiserror::Error;

/// The only API version supported by this Phase 0 contract.
pub const API_VERSION: u32 = 1;

/// Maximum number of version entries accepted in a negotiation request.
pub const MAX_NEGOTIATION_VERSIONS: usize = 16;

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
    /// Stable JSON serialization unexpectedly failed.
    #[error("failed to serialize canonical command: {0}")]
    Serialization(#[from] serde_json::Error),
}

impl Command {
    /// Returns the retry classification for this command.
    #[must_use]
    pub const fn class(&self) -> CommandClass {
        match self {
            Self::GetCapabilities { .. } | Self::GetSnapshot => CommandClass::ReadOnly,
            Self::NoopMutation { .. } => CommandClass::Mutating,
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
    /// Explicitly unavailable transmit authority.
    pub transmit_authority: Availability,
}

/// Whether a capability exists in the current implementation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Availability {
    /// The capability is not implemented or configured.
    Unavailable,
}

/// Deterministic bounded snapshot returned by the no-op service.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StationSnapshot {
    /// Current daemon process generation.
    pub service_instance_id: ServiceInstanceId,
    /// Station configuration state.
    pub configuration: ConfigurationState,
    /// Operating-session state.
    pub operation: OperationState,
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
        if envelope.api_version != API_VERSION {
            return error_response(
                request_id,
                ErrorCode::IncompatibleApiVersion,
                "client and service API versions are incompatible",
                ErrorDetails::VersionMismatch {
                    requested_version: envelope.api_version,
                    supported_versions: vec![API_VERSION],
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
                } else if supported_versions.contains(&API_VERSION) {
                    ResponseOutcome::Success(ResultBody::Capabilities(Capabilities {
                        negotiated_version: API_VERSION,
                        supported_versions: vec![API_VERSION],
                        service_instance_id: self.instance_id.clone(),
                        station_control: Availability::Unavailable,
                        transmit_authority: Availability::Unavailable,
                    }))
                } else {
                    ResponseOutcome::Error(ApiError {
                        code: ErrorCode::IncompatibleApiVersion,
                        message: "client and service API versions are incompatible".into(),
                        retryable: false,
                        details: ErrorDetails::VersionMismatch {
                            requested_version: envelope.api_version,
                            supported_versions: vec![API_VERSION],
                        },
                    })
                }
            }
            Command::GetSnapshot => {
                ResponseOutcome::Success(ResultBody::Snapshot(StationSnapshot {
                    service_instance_id: self.instance_id.clone(),
                    configuration: ConfigurationState::NotConfigured,
                    operation: OperationState::NotRunning,
                    transmit_authority: Availability::Unavailable,
                }))
            }
            Command::NoopMutation { .. } => ResponseOutcome::Error(ApiError {
                code: ErrorCode::RequestJournalRequired,
                message: "mutating commands require the durable request journal".into(),
                retryable: false,
                details: ErrorDetails::JournalRequired,
            }),
        };

        ResponseEnvelope {
            api_version: API_VERSION,
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
