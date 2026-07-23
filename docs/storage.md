# Versioned operational storage

SQLite schema version 2 is the authoritative operational foundation.
`slotpilot-storage` creates version 2 from an empty database, migrates version
1 forward inside an immediate transaction, and rejects databases created by
newer unsupported schema versions.

The Phase 0 tables continue to represent:

- accepted request and command identities, canonical command bytes, and the
  exact original bounded result;
- immutable profile revisions and resolved session-context snapshots;
- monotonically ordered operational events with stable event and service
  instance identities;
- generic pending/completed outbox work with unique idempotency keys;
- external receipts linked to outbox work without performing delivery.

Accepted requests use an immediate transaction with insert-or-load semantics,
so concurrent same-ID calls observe one winner. Same-command retries recover
the winner's exact serialized response; conflicting canonical bytes are mapped
by the daemon composition boundary to `request_id_conflict`.

Session-context snapshots cannot be updated or deleted. Foreign keys, JSON
validity, identity uniqueness, non-negative UTC times, revision bounds, and
closed state/profile-kind values are database constraints rather than
application assumptions.

Operational events use SQLite's monotonically assigned sequence as their
durable order. Reads are scoped to one `service_instance_id`, ordered by
sequence, and fetch only a caller-supplied bounded page plus one row to
determine `has_more`. Retention deletes older rows explicitly; a cursor before
the first retained sequence becomes a structured API gap rather than an
implicit jump. Event IDs remain unique, so a duplicate publication fails
without appending another sequence. The storage crate retains SlotPilot domain
IDs and JSON only; API envelope reconstruction belongs to the daemon boundary.

## Receive-only schema version 2

Schema version 2 adds three normalized tables:

- `receive_windows` assigns a global insertion sequence to a stable
  `ReceiveWindowId` and stores the exact service/process/stream generations,
  FT8 slot, stable configured device identity, selected input configuration,
  source-frame position, UTC/monotonic capture mapping, and record time;
- `receive_diagnostics` stores one bounded audio, timeline, and receive-clock
  summary for every window;
- `receive_decodes` stores at most 128 deterministic ordered FT8 results with
  integer offset/frequency/SNR units and exact resolved, unresolved-hash,
  unsupported-structured, ambiguous, or free-text owned classification.

The receive-window identity is the retry key. A repeated exact record returns
the existing sequence. Reuse with different content, or a different identity
for the same service/generation/slot/device/configuration context, is a typed
collision. The window, diagnostic row, and all decode rows are inserted in one
immediate transaction; any constraint or injected failure rolls back all of
them.

The daemon's public receive-store adapter extends that transaction through the
ordered `receive_decode` event insert. Event serialization or constraint
failure therefore rolls back a new window and its evidence. Exact retry returns
the original receive and event sequences; an older receive row created before
event coupling may be repaired by adding its missing deterministic event. No
public event can claim an uncommitted decode.

Receive pages are globally ordered by SQLite sequence, accept only 1 through
100 records, fetch one look-ahead row for `has_more`, and report earliest and
latest retained cursors. Explicit pruning deletes older windows and cascades
only to their diagnostics/decodes. Restart reconstructs every public value
through SlotPilot-owned constructors and fails typed on malformed identity,
classification, units, clock shape, missing diagnostics, or non-deterministic
decode order.

Clock health may be stored without decode results for diagnostic evidence.
An unhealthy record carrying decoder-ready results is rejected. Resolved
messages remain protocol classifications, not QSO eligibility or logging
records.

Version 2 intentionally has no raw continuous PCM, bulk waterfall rows, final
FT8 QSO or WSPR field set, ADIF behavior, network delivery, integration
credentials, live arm token, transmit authority, resumable PTT state, or
hardware access. Later migrations must preserve these fail-closed recovery
rules unless a focused issue and accepted safety decision explicitly change
them.
