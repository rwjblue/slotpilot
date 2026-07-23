//! Rendering boundary for the SlotPilot command-line client.
//!
//! Table and JSON output consume the same typed API response. Snapshot requests
//! use the shared user-scoped local transport.

use slotpilot_api::{
    Command, CommandEnvelope, EventPayload, ReceiveHistoryPage, ReceiveLifecycleSnapshot,
    ResponseEnvelope, ResponseOutcome, ResultBody, SubscriptionOutcome, SubscriptionRequest,
    SubscriptionResponse,
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

/// Sends any bounded typed command through the shared local transport.
pub fn request_command(
    address: &EndpointAddress,
    request_id: RequestId,
    command: Command,
    cancellation: &CancellationToken,
) -> Result<ResponseEnvelope, IpcError> {
    LocalClient::request(
        address,
        &CommandEnvelope {
            api_version: slotpilot_api::API_VERSION,
            request_id,
            command,
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

/// Renders a bounded subscription response as one JSON value per line.
///
/// Event pages produce one envelope per line. Cursor/error outcomes produce
/// one response value so machine consumers never need a second framing mode.
pub fn render_jsonl(response: &SubscriptionResponse) -> serde_json::Result<String> {
    match &response.outcome {
        SubscriptionOutcome::Events { events, .. } => events
            .iter()
            .map(serde_json::to_string)
            .collect::<Result<Vec<_>, _>>()
            .map(|lines| lines.join("\n")),
        _ => serde_json::to_string(response),
    }
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
        ResponseOutcome::Success(ResultBody::InputDevices(page)) => {
            let mut output = String::from("STABLE_ID                       DISPLAY\n");
            for device in &page.devices {
                output.push_str(&format!(
                    "{:<31} {}\n",
                    device.identity.opaque_id, device.display_name
                ));
            }
            output.trim_end().to_owned()
        }
        ResponseOutcome::Success(ResultBody::ReceiveStarted(lifecycle)) => {
            render_lifecycle("receive_started", *lifecycle)
        }
        ResponseOutcome::Success(ResultBody::ReceiveStopped(lifecycle)) => {
            render_lifecycle("receive_stopped", *lifecycle)
        }
        ResponseOutcome::Success(ResultBody::ReceiveStatus(status)) => {
            render_lifecycle("receive_status", status.lifecycle)
        }
        ResponseOutcome::Success(ResultBody::ReceiveHistory(page)) => render_history(page),
        ResponseOutcome::Error(error) => format!(
            "FIELD                VALUE\n\
             code                 {}\n\
             retryable            {}",
            error.code.as_str(),
            error.retryable
        ),
    }
}

fn render_lifecycle(result: &str, lifecycle: ReceiveLifecycleSnapshot) -> String {
    let state = match lifecycle {
        ReceiveLifecycleSnapshot::Stopped { .. } => "stopped",
        ReceiveLifecycleSnapshot::Starting { .. } => "starting",
        ReceiveLifecycleSnapshot::Receiving { .. } => "receiving",
        ReceiveLifecycleSnapshot::Inhibited { .. } => "inhibited",
        ReceiveLifecycleSnapshot::Stopping { .. } => "stopping",
    };
    format!(
        "FIELD                VALUE\n\
         result               {result}\n\
         receive_state        {state}"
    )
}

fn render_history(page: &ReceiveHistoryPage) -> String {
    let mut output = String::from("SEQUENCE  SLOT_UTC_MS  WINDOW_ID  DECODES\n");
    for record in &page.records {
        output.push_str(&format!(
            "{}  {}  {}  {}\n",
            record.sequence,
            record.slot_start_utc_millis,
            record.receive_window_id,
            record.decodes.len()
        ));
    }
    output.trim_end().to_owned()
}

/// Human rendering for one ordered event.
#[must_use]
pub fn render_event(event: &slotpilot_api::EventEnvelope) -> String {
    match &event.event {
        EventPayload::Phase0Notice { message } => {
            format!("{} phase0_notice {message}", event.sequence)
        }
        EventPayload::ReceiveLifecycle(state) => {
            format!(
                "{} receive_lifecycle {}",
                event.sequence,
                render_lifecycle("event", *state)
                    .lines()
                    .last()
                    .unwrap_or_default()
            )
        }
        EventPayload::ReceiveDecode(record) => format!(
            "{} receive_decode {} {}",
            event.sequence,
            record.receive_window_id,
            record.decodes.len()
        ),
        EventPayload::ReceiveHealth(_) => format!("{} receive_health", event.sequence),
        EventPayload::ReceiveDiscontinuity(value) => format!(
            "{} receive_discontinuity {:?}",
            event.sequence, value.reason
        ),
        EventPayload::WaterfallFrame(frame) => format!(
            "{} waterfall_frame {} {}",
            event.sequence,
            frame.frame_sequence,
            frame.bins.len()
        ),
        EventPayload::Unknown { kind, .. } => format!("{} unknown {kind}", event.sequence),
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
        API_VERSION, Command, CommandEnvelope, EventCursor, EventEnvelope, EventPayload,
        NoopService, OperationState, ReceiveLifecycleSnapshot, ReceiveStatus, ResponseOutcome,
        ResultBody, SubscriptionOutcome, SubscriptionRequest, SubscriptionResponse,
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

    #[test]
    fn receive_human_json_and_jsonl_share_typed_values() {
        let response = ResponseEnvelope {
            api_version: API_VERSION,
            request_id: "req_phase2status".parse().unwrap(),
            outcome: ResponseOutcome::Success(ResultBody::ReceiveStatus(ReceiveStatus {
                lifecycle: ReceiveLifecycleSnapshot::Receiving {
                    stream_generation: 2,
                },
                selection: None,
                audio: None,
                clock: None,
            })),
        };
        assert!(render_table(&response).contains("receiving"));
        assert!(
            render_json(&response)
                .unwrap()
                .contains("\"receive_status\"")
        );

        let service: slotpilot_domain::ServiceInstanceId = "svc_01jabcde9".parse().unwrap();
        let events = vec![
            EventEnvelope {
                api_version: API_VERSION,
                service_instance_id: service.clone(),
                sequence: 1,
                event_id: "evt_01jabcde9".parse().unwrap(),
                occurred_utc_millis: 1,
                event: EventPayload::ReceiveLifecycle(ReceiveLifecycleSnapshot::Receiving {
                    stream_generation: 2,
                }),
            },
            EventEnvelope {
                api_version: API_VERSION,
                service_instance_id: service.clone(),
                sequence: 2,
                event_id: "evt_01jabcdf0".parse().unwrap(),
                occurred_utc_millis: 2,
                event: EventPayload::Unknown {
                    kind: "future_receive_value".into(),
                    fields: serde_json::Map::new(),
                },
            },
        ];
        let response = SubscriptionResponse {
            api_version: API_VERSION,
            outcome: SubscriptionOutcome::Events {
                events,
                next_cursor: EventCursor {
                    service_instance_id: service,
                    sequence: 2,
                },
                has_more: false,
            },
        };
        let jsonl = render_jsonl(&response).unwrap();
        assert_eq!(jsonl.lines().count(), 2);
        for line in jsonl.lines() {
            serde_json::from_str::<EventEnvelope>(line).unwrap();
        }
        assert!(jsonl.contains("future_receive_value"));
    }

    #[test]
    fn generic_cli_command_route_carries_receive_status_without_prompting() {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let directory = std::env::temp_dir().join(format!(
            "slotpilot-cli-receive-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let address = EndpointAddress::for_user(&directory, "receive_test").unwrap();
        let server = LocalServer::bind(&address).unwrap();
        let handle = thread::spawn(move || {
            server
                .serve_exchange(&CancellationToken::new(), |request: CommandEnvelope| {
                    assert!(matches!(request.command, Command::GetReceiveStatus));
                    ResponseEnvelope {
                        api_version: API_VERSION,
                        request_id: request.request_id,
                        outcome: ResponseOutcome::Success(ResultBody::ReceiveStatus(
                            ReceiveStatus::stopped(),
                        )),
                    }
                })
                .unwrap();
        });
        let response = request_command(
            &address,
            "req_phase2read".parse().unwrap(),
            Command::GetReceiveStatus,
            &CancellationToken::new(),
        )
        .unwrap();
        handle.join().unwrap();
        assert!(matches!(
            response.outcome,
            ResponseOutcome::Success(ResultBody::ReceiveStatus(ReceiveStatus {
                lifecycle: ReceiveLifecycleSnapshot::Stopped { .. },
                ..
            }))
        ));
        fs::remove_dir_all(directory).unwrap();
    }
}
