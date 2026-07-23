# CLI and local API contract

## Binaries

```text
slotpilotd    local station daemon
slotpilot     command-line client
SlotPilot     desktop application
```

The CLI is a client, not a second implementation of station logic.

## Output modes

Every ordinary command supports:

```text
--format table|json
--non-interactive
--request-id <stable-id>   # mutating commands
```

Streaming commands use:

```text
slotpilot events follow --format jsonl
```

Machine mode requirements:

- stdout contains only the selected response format;
- diagnostics go to stderr;
- no prompts;
- no ANSI styling unless explicitly requested;
- exact UTC timestamps in RFC 3339 form;
- stable symbolic error codes;
- bounded ordinary responses;
- one JSON value per line for streams.

## Command families

Planned command shape:

```text
slotpilot api capabilities
slotpilot status
slotpilot events follow

slotpilot devices audio list
slotpilot rigs list
slotpilot rigs probe --profile <name>

slotpilot profiles list
slotpilot profiles show <name>
slotpilot profiles validate <name>
slotpilot profiles export <name>

slotpilot ft8 run start ...
slotpilot ft8 run pause
slotpilot ft8 run pause-after-qso
slotpilot ft8 run resume
slotpilot ft8 run stop
slotpilot ft8 callers list
slotpilot ft8 callers select <call>
slotpilot ft8 callers skip <call>
slotpilot ft8 callers pin <call>
slotpilot ft8 lane auto|pin|set
slotpilot ft8 parity auto|odd|even

slotpilot wspr rx start
slotpilot wspr tx once
slotpilot wspr tx enqueue
slotpilot wspr schedule list
slotpilot wspr stop

slotpilot rules list
slotpilot rules explain <call>
slotpilot rules add|update|delete

slotpilot log import-adif <path>
slotpilot log export-adif <path>
slotpilot log show <qso-id>

slotpilot emergency-stop
```

Names may be refined during implementation, but command concepts and parity with the desktop client are requirements.

## Command envelope

Every Phase 0 request uses a bounded versioned envelope:

```json
{
  "api_version": 1,
  "request_id": "req_01jabcde9",
  "command": {
    "kind": "get_snapshot"
  }
}
```

The no-op service supports `get_capabilities` and `get_snapshot`.
`get_capabilities` carries at most 16 client-supported versions and selects
version 1. An unsupported envelope version or lack of a common version returns
`incompatible_api_version`; an oversized list returns
`negotiation_too_large`.

JSON objects may gain additive fields, which version-1 readers ignore. Unknown
command, result, or error-detail variants are incompatible rather than guessed
from strings.

Read-only commands are evaluated afresh and are not recorded in the request
journal. Phase 0 includes one bounded `noop_mutation` solely to exercise
durable retry behavior; it persists no station state and performs no external
side effect.

Typed command serialization produces canonical JSON bytes. Every semantic
field participates, and the no-op marker is limited to 128 bytes.

Request-ID behavior:

- a new mutating request ID is accepted transactionally with its exact result;
- the same ID and canonical command returns the stored original result, even
  after restart;
- the same ID and different command returns `request_id_conflict` with
  `retryable: false`;
- a transaction failure leaves no partial acceptance;
- after an uncertain timeout, retry the identical command with the identical
  ID rather than generating a new operation.

## Results and errors

Success responses contain the request identity and a stable result kind. Errors contain:

```json
{
  "code": "rig.verification_failed",
  "message": "Rig state did not match the requested state.",
  "retryable": false,
  "details": {
    "expected_mode": "DATA-U",
    "actual_mode": "USB"
  }
}
```

The human message may improve over time. Automation should branch on `code`, not prose.

Provisional exit categories:

```text
0   success
2   command-line usage error
10  configuration/profile error
20  service/device unavailable
30  safety inhibition or missing authority
40  requested operation failed
50  internal/unclassified failure
```

These categories require fixtures before being treated as stable.

## Snapshot and events

A client first requests a bounded snapshot containing current service, rig, audio, clock, session, operating-mode, and queue state. It then subscribes to ordered events beginning from a cursor when supported.

The Phase 0 no-op snapshot is narrower: it reports a `service_instance_id`,
`not_configured`, `not_running`, and unavailable transmit authority. Each
daemon process generation has a new `svc_` identity. That identity is for
reconnect/restart detection only; it is not persisted authority and a changed
identity never implies restoration of station state.

Reviewed version-1 command, result, error, capability, and snapshot JSON
fixtures are maintained under `crates/api/tests/fixtures/`. Human table output
and JSON output are renderings of the same typed response model.

Events include stable IDs, UTC time, monotonic/sequence ordering information, and a schema version. Clients must tolerate unknown additive event fields and should surface unsupported event kinds rather than silently reinterpreting them.

## Local transport

Default endpoints are user-local Unix sockets on macOS/Linux and named pipes on Windows. Loopback TCP is development-only and must bind explicitly to loopback.

Authentication/authorization should initially rely on OS-local peer and endpoint permissions, with explicit review before adding multi-user or remote access.

## Example FT8 run

```text
slotpilot ft8 run start   --profile home-k4   --behavior drain-then-cq   --caller-selector oldest   --idle-cq-limit 6   --qso-retry-limit 3   --request-id 0190e4c0-...   --format json   --non-interactive
```

## Example AntennaBench WSPR request

```text
slotpilot wspr tx enqueue   --request-id "$INTENT_ID"   --external-session-id "$SESSION_ID"   --start-utc "2026-07-23T20:14:00Z"   --band 20m   --frequency-hz 14095600   --station-profile antennabench-home   --power-dbm 30   --format json   --non-interactive
```

The enqueue command returns after durable acceptance or rejection. Actual transmit timing and result are emitted as later events and retained as transmission receipts.
