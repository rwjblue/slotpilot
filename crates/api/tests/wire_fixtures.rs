//! Reviewed version-1 JSON fixture compatibility checks.

use slotpilot_api::{
    CommandEnvelope, EventEnvelope, EventPayload, ResponseEnvelope, SubscriptionRequest,
    SubscriptionResponse,
};

fn assert_fixture_round_trip<T>(fixture: &str)
where
    T: serde::de::DeserializeOwned + serde::Serialize,
{
    let expected: serde_json::Value = serde_json::from_str(fixture).unwrap();
    let parsed: T = serde_json::from_value(expected.clone()).unwrap();
    let actual = serde_json::to_value(parsed).unwrap();
    assert_eq!(actual, expected);
}

#[test]
fn command_fixtures_are_stable() {
    assert_fixture_round_trip::<CommandEnvelope>(include_str!(
        "fixtures/get-capabilities-command.json"
    ));
    assert_fixture_round_trip::<CommandEnvelope>(include_str!(
        "fixtures/get-snapshot-command.json"
    ));
    assert_fixture_round_trip::<CommandEnvelope>(include_str!(
        "fixtures/noop-mutation-command.json"
    ));
    assert_fixture_round_trip::<CommandEnvelope>(include_str!(
        "fixtures/receive-start-command.json"
    ));
}

#[test]
fn response_fixtures_are_stable() {
    assert_fixture_round_trip::<ResponseEnvelope>(include_str!(
        "fixtures/capabilities-response.json"
    ));
    assert_fixture_round_trip::<ResponseEnvelope>(include_str!("fixtures/snapshot-response.json"));
    assert_fixture_round_trip::<ResponseEnvelope>(include_str!(
        "fixtures/incompatible-version-error.json"
    ));
    assert_fixture_round_trip::<ResponseEnvelope>(include_str!(
        "fixtures/noop-mutation-response.json"
    ));
    assert_fixture_round_trip::<ResponseEnvelope>(include_str!(
        "fixtures/request-id-conflict-error.json"
    ));
    assert_fixture_round_trip::<ResponseEnvelope>(include_str!(
        "fixtures/receive-status-response.json"
    ));
    assert_fixture_round_trip::<ResponseEnvelope>(include_str!(
        "fixtures/receive-history-response.json"
    ));
}

#[test]
fn event_and_subscription_fixtures_are_stable() {
    assert_fixture_round_trip::<EventEnvelope>(include_str!("fixtures/event-envelope.json"));
    assert_fixture_round_trip::<EventEnvelope>(include_str!("fixtures/receive-decode-event.json"));
    assert_fixture_round_trip::<EventEnvelope>(include_str!("fixtures/waterfall-event.json"));
    assert_fixture_round_trip::<SubscriptionRequest>(include_str!(
        "fixtures/subscription-request.json"
    ));
    assert_fixture_round_trip::<SubscriptionResponse>(include_str!(
        "fixtures/subscription-gap-response.json"
    ));
}

#[test]
fn unknown_events_round_trip_without_becoming_typed_transitions() {
    let json = r#"{"kind":"future_authority_changed","enabled":true}"#;
    let payload: EventPayload = serde_json::from_str(json).unwrap();
    assert!(matches!(
        payload,
        EventPayload::Unknown { ref kind, .. } if kind == "future_authority_changed"
    ));
    assert_eq!(
        serde_json::to_value(payload).unwrap(),
        serde_json::from_str::<serde_json::Value>(json).unwrap()
    );
}
