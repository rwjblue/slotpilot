# AntennaBench integration

AntennaBench and SlotPilot should remain separate processes with a documented local contract.

Reasons:

- SlotPilot is GPL-3.0-or-later because of its selected protocol implementation path;
- AntennaBench is Apache-2.0;
- independent processes preserve lifecycle and fault isolation;
- AntennaBench needs a stable machine contract, not direct access to SlotPilot internals;
- SlotPilot must remain the sole owner of radio, audio, PTT, and synchronized scheduling.

This is an engineering boundary, not legal advice.

## Integration goals

AntennaBench should be able to:

- discover SlotPilot API capabilities;
- validate a station profile without keying a radio;
- durably enqueue one WSPR transmission for a specific UTC slot;
- attach stable AntennaBench session and intention identities;
- learn whether the request was accepted, rejected, inhibited, missed, cancelled, or completed;
- receive actual PTT and audio timing evidence;
- obtain local WSPR decodes through a stable stream or compatible adapter;
- retry uncertain requests without duplicating a transmission.

## CLI enqueue example

```text
slotpilot wspr tx enqueue   --request-id "$INTENT_ID"   --external-session-id "$SESSION_ID"   --start-utc "2026-07-23T20:14:00Z"   --band 20m   --frequency-hz 14095600   --station-profile antennabench-home   --power-dbm 30   --format json   --non-interactive
```

Acceptance means the daemon durably stored the requested intention and passed current admission checks. It does not mean transmission completed.

Example accepted result:

```json
{
  "api_version": 1,
  "request_id": "intent-123",
  "status": "accepted",
  "transmission_id": "0190e4c0-...",
  "scheduled_start_utc": "2026-07-23T20:14:00Z"
}
```

## Transmission receipt

A later event records actual behavior:

```json
{
  "event": "wspr.transmission_completed",
  "transmission_id": "0190e4c0-...",
  "external_session_id": "session-456",
  "external_intent_id": "intent-123",
  "scheduled_start_utc": "2026-07-23T20:14:00Z",
  "actual_audio_start_utc": "2026-07-23T20:14:00.998Z",
  "actual_audio_end_utc": "2026-07-23T20:15:51.590Z",
  "ptt_on_utc": "2026-07-23T20:14:00.850Z",
  "ptt_off_utc": "2026-07-23T20:15:51.720Z",
  "dial_frequency_hz": 14095600,
  "audio_frequency_hz": 1500,
  "power_dbm": 30,
  "result": "completed"
}
```

The exact schema will be versioned and fixture-tested before AntennaBench depends on it.

## Receive observations

Potential compatibility outputs include:

- SlotPilot native JSONL events;
- WSJT-X-compatible UDP messages where appropriate;
- `ALL_WSPR.TXT`-compatible records for existing ingestion paths.

Native events are authoritative for SlotPilot-specific identities and timing. Compatibility formats are adapters and may not represent every field.

## Authority and recovery

An external enqueue request does not create permanent transmit authority. SlotPilot must have an operator-approved station context and applicable WSPR authority. Daemon restart never restores authority silently, even when a durable future intention remains recorded.

AntennaBench should treat an inhibited or missed receipt as experiment evidence rather than assuming a scheduled cycle transmitted.
