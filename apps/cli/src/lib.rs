//! Rendering boundary for the SlotPilot command-line client.
//!
//! Table and JSON output consume the same typed API response. Snapshot requests
//! use the shared user-scoped local transport.

use slotpilot_api::{
    ResponseEnvelope, ResponseOutcome, ResultBody, SubscriptionRequest, SubscriptionResponse,
};
use slotpilot_domain::RequestId;
use slotpilot_ipc::{CancellationToken, EndpointAddress, IpcError, LocalClient};

/// Requests the bounded no-op snapshot through the shared local transport.
pub fn request_snapshot(
    address: &EndpointAddress,
    request_id: RequestId,
    cancellation: &CancellationToken,
) -> Result<ResponseEnvelope, IpcError> {
    LocalClient::request(
        address,
        &slotpilot_api::CommandEnvelope {
            api_version: slotpilot_api::API_VERSION,
            request_id,
            command: slotpilot_api::Command::GetSnapshot,
        },
        cancellation,
    )
}

/// Requests one bounded event replay page through the shared local transport.
pub fn request_events(
    address: &EndpointAddress,
    request: &SubscriptionRequest,
    cancellation: &CancellationToken,
) -> Result<SubscriptionResponse, IpcError> {
    LocalClient::exchange(address, request, cancellation)
}

/// Renders one bounded API response as JSON.
pub fn render_json(response: &ResponseEnvelope) -> serde_json::Result<String> {
    serde_json::to_string(response)
}

/// Renders one bounded API response as a stable human-oriented table.
#[must_use]
pub fn render_table(response: &ResponseEnvelope) -> String {
    match &response.outcome {
        ResponseOutcome::Success(ResultBody::Capabilities(capabilities)) => format!(
            "FIELD                VALUE\n\
             api_version          {}\n\
             service_instance     {}\n\
             station_control      unavailable\n\
             transmit_authority   unavailable",
            capabilities.negotiated_version, capabilities.service_instance_id
        ),
        ResponseOutcome::Success(ResultBody::Snapshot(snapshot)) => format!(
            "FIELD                VALUE\n\
             service_instance     {}\n\
             configuration        not_configured\n\
             operation            not_running\n\
             transmit_authority   unavailable",
            snapshot.service_instance_id
        ),
        ResponseOutcome::Success(ResultBody::NoopMutationAccepted { marker }) => format!(
            "FIELD                VALUE\n\
             result               noop_mutation_accepted\n\
             marker               {marker}"
        ),
        ResponseOutcome::Error(error) => format!(
            "FIELD                VALUE\n\
             code                 {}\n\
             retryable            {}",
            error.code.as_str(),
            error.retryable
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        sync::atomic::{AtomicU64, Ordering},
        thread,
    };

    use slotpilot_api::{
        API_VERSION, Command, CommandEnvelope, EventCursor, NoopService, OperationState,
        ResponseOutcome, ResultBody, SubscriptionOutcome, SubscriptionRequest,
        SubscriptionResponse,
    };
    use slotpilot_ipc::LocalServer;

    use super::*;

    #[test]
    fn table_and_json_consume_the_same_snapshot_result() {
        let response =
            NoopService::new("svc_01jabcde9".parse().unwrap()).execute(CommandEnvelope {
                api_version: API_VERSION,
                request_id: "req_01jabcde9".parse().unwrap(),
                command: Command::GetSnapshot,
            });
        assert!(matches!(
            response.outcome,
            ResponseOutcome::Success(ResultBody::Snapshot(_))
        ));
        assert!(render_table(&response).contains("not_configured"));
        assert!(
            render_json(&response)
                .unwrap()
                .contains(r#""configuration":"not_configured""#)
        );
    }

    #[test]
    fn cli_snapshot_path_uses_local_transport() {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let directory = std::env::temp_dir().join(format!(
            "slotpilot-cli-ipc-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let address = EndpointAddress::for_user(&directory, "cli_test").unwrap();
        let server = LocalServer::bind(&address).unwrap();
        let handle = thread::spawn(move || {
            server
                .serve_once(
                    &NoopService::new("svc_01jabcde9".parse().unwrap()),
                    &CancellationToken::new(),
                )
                .unwrap();
        });
        let response = request_snapshot(
            &address,
            "req_01jabcde9".parse().unwrap(),
            &CancellationToken::new(),
        )
        .unwrap();
        handle.join().unwrap();
        assert!(matches!(
            response.outcome,
            ResponseOutcome::Success(ResultBody::Snapshot(snapshot))
                if snapshot.operation == OperationState::NotRunning
        ));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn snapshot_then_subscription_share_the_typed_local_transport() {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let directory = std::env::temp_dir().join(format!(
            "slotpilot-cli-events-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let address = EndpointAddress::for_user(&directory, "events_test").unwrap();
        let server = LocalServer::bind(&address).unwrap();
        let service_id = "svc_01jabcde9".parse().unwrap();
        let handle = thread::spawn(move || {
            let cancellation = CancellationToken::new();
            server
                .serve_once(&NoopService::new(service_id), &cancellation)
                .unwrap();
            server
                .serve_exchange(&cancellation, |request: SubscriptionRequest| {
                    SubscriptionResponse {
                        api_version: API_VERSION,
                        outcome: SubscriptionOutcome::Events {
                            events: Vec::new(),
                            next_cursor: request.after.unwrap(),
                            has_more: false,
                        },
                    }
                })
                .unwrap();
        });
        let cancellation = CancellationToken::new();
        let snapshot =
            request_snapshot(&address, "req_01jabcde9".parse().unwrap(), &cancellation).unwrap();
        let ResponseOutcome::Success(ResultBody::Snapshot(snapshot)) = snapshot.outcome else {
            panic!("expected snapshot");
        };
        let response = request_events(
            &address,
            &SubscriptionRequest {
                api_version: API_VERSION,
                after: Some(EventCursor {
                    service_instance_id: snapshot.service_instance_id,
                    sequence: snapshot.event_cursor.sequence,
                }),
                limit: 16,
            },
            &cancellation,
        )
        .unwrap();
        handle.join().unwrap();
        assert!(matches!(
            response.outcome,
            SubscriptionOutcome::Events {
                events,
                has_more: false,
                ..
            } if events.is_empty()
        ));
        fs::remove_dir_all(directory).unwrap();
    }
}
