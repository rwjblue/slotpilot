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

Every request uses a bounded versioned envelope:

```json
{
  "api_version": 2,
  "request_id": "req_01jabcde9",
  "command": {
    "kind": "get_snapshot"
  }
}
```

Version 2 adds receive-only commands while version 1 remains supported for its
landed Phase 0 commands and fixtures. `get_capabilities` carries at most 16
client-supported versions and selects the first client preference present in
the service set `[2, 1]`. An unsupported envelope version or lack of a common version returns
`incompatible_api_version`; an oversized list returns
`negotiation_too_large`.

Version-2 receive commands are:

```text
list_input_devices
receive_start { selection }
receive_stop
get_receive_status
query_receive_history { after_sequence, limit }
```

`receive_start` requires the stable platform identity and exact rate, channel
count, sample format, and selected channel returned by discovery. Display names
cannot appear in the selection and there is no default-device fallback.
History pages contain 1–100 records. Discovery contains at most 64 devices and
64 configuration ranges per device. Receive records retain at most 128 decode
outcomes. Waterfall events retain at most 2,048 bins and are emitted only from
the spectrum model's rate-limited, single-pending-token snapshot path.

A version-1 envelope using any receive command returns
`command_unavailable_in_version`; it is never guessed or interpreted as a
Phase 0 command.

JSON objects may gain additive fields, which version-1 readers ignore. Unknown
command, result, or error-detail variants are incompatible rather than guessed
from strings.

Read-only commands are evaluated afresh and are not recorded in the request
journal. Phase 0 includes one bounded `noop_mutation` solely to exercise
durable retry behavior; it persists no station state and performs no external
side effect. Version-2 receive start/stop are mutating commands. The running
receive owner applies a stable request ID idempotently, then the durable journal
retains the exact response for cross-restart replay. If journaling fails after
the live operation, retrying the same ID reaches the port's same-process
identity cache rather than duplicating the mutation. A daemon restart remains
receive-inactive even when an old request's historical result is replayed.

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

The Phase 0 no-op snapshot is narrower: it reports a `service_instance_id`, an
event cursor at sequence zero, `not_configured`, `not_running`, and unavailable
transmit authority. Each daemon process generation has a new `svc_` identity.
That identity is for reconnect/restart detection only; it is not persisted
authority and a changed identity never implies restoration of station state.

Reviewed version-1 compatibility fixtures and version-2 receive command,
result, snapshot, decode-event, and waterfall-event fixtures are maintained under
`crates/api/tests/fixtures/`. Human table output and JSON output are renderings
of the same typed response model.

Version-2 snapshots add receive lifecycle, explicit selection, audio health,
and clock health. Receive events cover lifecycle, health, discontinuity,
durable decode evidence, and bounded waterfall frames. Every resolved,
unresolved-hash, unsupported, ambiguous, and free-text classification remains
typed evidence; none is a QSO or automation transition.

A receive decode event is inserted in the same SQLite transaction as its
receive window, diagnostics, and classifications. Event-insert failure rolls
back a new receive record. Exact retries return the existing receive and event
sequences, and an older committed receive record may be repaired by adding its
missing event without ever publishing an uncommitted decode.

Events include API version, stable `evt_` identity, UTC milliseconds, daemon
generation, monotonically increasing SQLite sequence, and a SlotPilot-owned
payload. A cursor is the pair `(service_instance_id, sequence)` and represents
the last event already observed. Sequence zero means before the first event.

Replay is one request per local IPC connection and is capped at 256 events.
The response carries a `next_cursor` and `has_more`; a slow client repeatedly
requests bounded pages and is never given an unbounded server queue. A clean
disconnect loses no cursor state. After reconnect the client obtains a fresh
snapshot, compares generations, and subscribes from a compatible cursor.

Replay outcomes are explicit:

- `events` is an ordered, possibly empty page;
- `cursor_gap` means retention removed required history and reports the first
  retained event position;
- `cursor_unavailable` means the cursor is ahead of committed history;
- `incompatible_generation` means the cursor belongs to another daemon
  process and must never be continued silently;
- `invalid_request` covers an incompatible API version or a limit outside
  1–256.

Reopening storage with the same service-instance identity preserves compatible
replay. A daemon restart uses a new identity, so persisted old cursors receive
`incompatible_generation`; clients must refresh their snapshot. Cancellation
is checked at IPC connection and frame boundaries.

Versioned clients deserialize recognized payloads into typed variants.
Unrecognized event kinds retain their opaque JSON fields for display or
diagnostics, but cannot trigger a typed state transition and never grant
authority. Unknown additive envelope fields remain tolerated.

## Local transport

Default endpoints are user-local Unix sockets on macOS/Linux and named pipes on Windows. Loopback TCP is development-only and must bind explicitly to loopback.

Authentication/authorization should initially rely on OS-local peer and endpoint permissions, with explicit review before adding multi-user or remote access.

## Receive CLI

The current receive-only routes are:

```text
slotpilot status <runtime> [--json]
slotpilot devices audio list <runtime> [--json]
slotpilot receive status <runtime> [--json]
slotpilot receive start <runtime> <platform> <opaque-id> <rate> <channels> <format> <channel> --request-id <req_...> [--json]
slotpilot receive stop <runtime> --request-id <req_...> [--json]
slotpilot receive history <runtime> <after-sequence> <limit> [--json]
slotpilot events follow <runtime> <after-sequence> <limit> --jsonl
```

Machine modes never prompt. JSON emits one bounded response. JSONL emits one
typed event envelope per line; non-event cursor outcomes emit one response
value. Event follow obtains a coherent snapshot first and uses its daemon
generation, so it cannot silently continue a stale cursor after restart.

## Example FT8 run

```text
slotpilot ft8 run start   --profile home-k4   --behavior drain-then-cq   --caller-selector oldest   --idle-cq-limit 6   --qso-retry-limit 3   --request-id 0190e4c0-...   --format json   --non-interactive
```

## Example AntennaBench WSPR request

```text
slotpilot wspr tx enqueue   --request-id "$INTENT_ID"   --external-session-id "$SESSION_ID"   --start-utc "2026-07-23T20:14:00Z"   --band 20m   --frequency-hz 14095600   --station-profile antennabench-home   --power-dbm 30   --format json   --non-interactive
```

The enqueue command returns after durable acceptance or rejection. Actual transmit timing and result are emitted as later events and retained as transmission receipts.
