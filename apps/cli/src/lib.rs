//! Rendering boundary for the SlotPilot command-line client.
//!
//! Table and JSON output consume the same typed API response. Transport and
//! command-line parsing remain deferred to the local-IPC issue.

use slotpilot_api::{ResponseEnvelope, ResponseOutcome, ResultBody};

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
    use slotpilot_api::{
        API_VERSION, Command, CommandEnvelope, NoopService, ResponseOutcome, ResultBody,
    };

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
}
