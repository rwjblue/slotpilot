//! Daemon-side composition of API commands and the durable request journal.
//!
//! The Phase 0 processor evaluates read-only commands directly and journals
//! only the explicitly side-effect-free no-op mutation. It contains no
//! station, hardware, logging sink, network, transmit, or external side effect.

use std::path::Path;

use slotpilot_api::{
    API_VERSION, ApiError, CanonicalizationError, CommandClass, CommandEnvelope, ErrorCode,
    ErrorDetails, NoopService, ResponseEnvelope, ResponseOutcome, ResultBody,
};
use slotpilot_domain::{CommandId, RequestId, ServiceInstanceId};
use slotpilot_ipc::{CancellationToken, IpcError, LocalServer};
use slotpilot_storage::{AcceptOutcome, AcceptedCommand, StorageError, Store};
use thiserror::Error;

/// Failure before a bounded API response could be durably produced.
#[derive(Debug, Error)]
pub enum ProcessorError {
    /// Command canonicalization rejected an invalid bounded value.
    #[error(transparent)]
    Canonicalization(#[from] CanonicalizationError),
    /// SQLite could not atomically accept or recover the request.
    #[error(transparent)]
    Storage(#[from] StorageError),
    /// A persisted original result was not valid versioned API JSON.
    #[error("invalid persisted API result: {0}")]
    InvalidPersistedResult(#[from] serde_json::Error),
}

/// No-op service processor with durable mutating-request semantics.
pub struct CommandProcessor {
    service: NoopService,
    store: Store,
}

/// Serves one read-only no-op API exchange through the local endpoint.
///
/// This handshake path cannot grant authority or produce a station side
/// effect.
pub fn serve_noop_once(
    server: &LocalServer,
    service_instance_id: ServiceInstanceId,
    cancellation: &CancellationToken,
) -> Result<(), IpcError> {
    server.serve_once(&NoopService::new(service_instance_id), cancellation)
}

impl CommandProcessor {
    /// Opens a migrated request journal for one daemon process generation.
    pub fn open(
        path: impl AsRef<Path>,
        service_instance_id: ServiceInstanceId,
    ) -> Result<Self, ProcessorError> {
        Ok(Self {
            service: NoopService::new(service_instance_id),
            store: Store::open(path)?,
        })
    }

    /// Builds an isolated in-memory processor.
    pub fn in_memory(service_instance_id: ServiceInstanceId) -> Result<Self, ProcessorError> {
        Ok(Self {
            service: NoopService::new(service_instance_id),
            store: Store::open_in_memory()?,
        })
    }

    /// Executes one command with explicit acceptance time for deterministic tests.
    pub fn execute(
        &mut self,
        envelope: CommandEnvelope,
        accepted_utc_millis: i64,
    ) -> Result<ResponseEnvelope, ProcessorError> {
        if envelope.api_version != API_VERSION || envelope.command.class() == CommandClass::ReadOnly
        {
            return Ok(self.service.execute(envelope));
        }

        let canonical_command = envelope.command.canonical_bytes()?;
        let marker = match &envelope.command {
            slotpilot_api::Command::NoopMutation { marker } => marker.clone(),
            _ => return Ok(self.service.execute(envelope)),
        };
        let accepted_response = ResponseEnvelope {
            api_version: API_VERSION,
            request_id: envelope.request_id.clone(),
            outcome: ResponseOutcome::Success(ResultBody::NoopMutationAccepted { marker }),
        };
        let accepted = AcceptedCommand {
            request_id: envelope.request_id.clone(),
            command_id: CommandId::for_request(&envelope.request_id),
            canonical_command: canonical_command.clone(),
            original_result: serde_json::to_vec(&accepted_response)?,
            accepted_utc_millis,
        };

        match self.store.accept_or_existing(&accepted)? {
            AcceptOutcome::Inserted(_) => Ok(accepted_response),
            AcceptOutcome::Existing(existing)
                if existing.canonical_command == canonical_command =>
            {
                Ok(serde_json::from_slice(&existing.original_result)?)
            }
            AcceptOutcome::Existing(_) => Ok(conflict_response(envelope.request_id)),
        }
    }

    /// Reports whether a mutating request identity has been journaled.
    pub fn is_journaled(&self, request_id: &RequestId) -> Result<bool, ProcessorError> {
        Ok(self.store.accepted_command(request_id)?.is_some())
    }
}

fn conflict_response(request_id: RequestId) -> ResponseEnvelope {
    ResponseEnvelope {
        api_version: API_VERSION,
        request_id,
        outcome: ResponseOutcome::Error(ApiError {
            code: ErrorCode::RequestIdConflict,
            message: "request ID was already accepted for a different command".into(),
            retryable: false,
            details: ErrorDetails::RequestConflict,
        }),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        sync::{
            Arc, Barrier,
            atomic::{AtomicU64, Ordering},
        },
        thread,
    };

    use slotpilot_api::{Command, ResponseOutcome};

    use super::*;

    static DATABASE_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temp_database() -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "slotpilot-processor-{}-{}.sqlite3",
            std::process::id(),
            DATABASE_COUNTER.fetch_add(1, Ordering::Relaxed)
        ))
    }

    fn instance() -> ServiceInstanceId {
        "svc_01jabcde9".parse().unwrap()
    }

    fn request(id: &str, command: Command) -> CommandEnvelope {
        CommandEnvelope {
            api_version: API_VERSION,
            request_id: id.parse().unwrap(),
            command,
        }
    }

    fn mutation(id: &str, marker: &str) -> CommandEnvelope {
        request(
            id,
            Command::NoopMutation {
                marker: marker.into(),
            },
        )
    }

    #[test]
    fn same_id_same_command_replays_exact_original_result() {
        let mut processor = CommandProcessor::in_memory(instance()).unwrap();
        let first = processor
            .execute(mutation("req_01jabcde9", "alpha"), 10)
            .unwrap();
        let replay = processor
            .execute(mutation("req_01jabcde9", "alpha"), 99)
            .unwrap();
        assert_eq!(replay, first);
    }

    #[test]
    fn same_id_different_command_returns_stable_conflict() {
        let mut processor = CommandProcessor::in_memory(instance()).unwrap();
        processor
            .execute(mutation("req_01jabcde9", "alpha"), 10)
            .unwrap();
        let conflict = processor
            .execute(mutation("req_01jabcde9", "beta"), 11)
            .unwrap();
        assert!(matches!(
            conflict.outcome,
            ResponseOutcome::Error(ApiError {
                code: ErrorCode::RequestIdConflict,
                retryable: false,
                ..
            })
        ));
    }

    #[test]
    fn restart_replays_persisted_original_result() {
        let path = temp_database();
        let first = {
            let mut processor = CommandProcessor::open(&path, instance()).unwrap();
            processor
                .execute(mutation("req_01jabcde9", "alpha"), 10)
                .unwrap()
        };
        let replay = {
            let mut processor =
                CommandProcessor::open(&path, "svc_01jabcdf0".parse().unwrap()).unwrap();
            processor
                .execute(mutation("req_01jabcde9", "alpha"), 99)
                .unwrap()
        };
        assert_eq!(replay, first);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn read_only_requests_are_not_journaled_and_may_reuse_id() {
        let mut processor = CommandProcessor::in_memory(instance()).unwrap();
        let id: RequestId = "req_01jabcde9".parse().unwrap();
        processor
            .execute(request(id.as_str(), Command::GetSnapshot), 1)
            .unwrap();
        processor
            .execute(
                request(
                    id.as_str(),
                    Command::GetCapabilities {
                        supported_versions: vec![API_VERSION],
                    },
                ),
                2,
            )
            .unwrap();
        assert!(!processor.is_journaled(&id).unwrap());
    }

    #[test]
    fn concurrent_same_id_calls_observe_one_original_result() {
        let path = temp_database();
        let barrier = Arc::new(Barrier::new(2));
        let mut handles = Vec::new();
        for accepted_time in [10, 20] {
            let path = path.clone();
            let barrier = Arc::clone(&barrier);
            handles.push(thread::spawn(move || {
                let mut processor = CommandProcessor::open(path, instance()).unwrap();
                barrier.wait();
                processor
                    .execute(mutation("req_01jabcde9", "alpha"), accepted_time)
                    .unwrap()
            }));
        }
        let first = handles.remove(0).join().unwrap();
        let second = handles.remove(0).join().unwrap();
        assert_eq!(first, second);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn transaction_failure_does_not_leave_partial_acceptance() {
        let mut processor = CommandProcessor::in_memory(instance()).unwrap();
        let id: RequestId = "req_01jabcde9".parse().unwrap();
        assert!(matches!(
            processor.execute(mutation(id.as_str(), "alpha"), -1),
            Err(ProcessorError::Storage(_))
        ));
        assert!(!processor.is_journaled(&id).unwrap());
    }
}
