# Phase 0 storage contract

SQLite schema version 1 is the authoritative Phase 0 operational foundation.
`slotpilot-storage` applies forward migrations inside an immediate transaction
and rejects databases created by newer unsupported schema versions.

The schema represents:

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

Version 1 intentionally has no final FT8 QSO or WSPR field set, ADIF behavior,
network delivery, integration credentials, live arm token, transmit authority,
resumable PTT state, or hardware access. Later migrations must preserve these
fail-closed recovery rules unless a focused issue and accepted safety decision
explicitly change them.
