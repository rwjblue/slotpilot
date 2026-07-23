//! Reviewed version-1 JSON fixture compatibility checks.

use slotpilot_api::{CommandEnvelope, ResponseEnvelope};

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
}
