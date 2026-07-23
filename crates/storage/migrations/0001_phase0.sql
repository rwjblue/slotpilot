CREATE TABLE accepted_commands (
    request_id TEXT PRIMARY KEY,
    command_id TEXT NOT NULL UNIQUE,
    canonical_command BLOB NOT NULL,
    original_result BLOB NOT NULL,
    accepted_utc_millis INTEGER NOT NULL CHECK (accepted_utc_millis >= 0)
) STRICT;

CREATE TABLE profile_revisions (
    profile_revision_id TEXT PRIMARY KEY,
    profile_id TEXT NOT NULL,
    profile_kind TEXT NOT NULL CHECK (
        profile_kind IN ('operator', 'station', 'activation', 'rig', 'audio', 'operating')
    ),
    revision INTEGER NOT NULL CHECK (revision > 0),
    document_json TEXT NOT NULL CHECK (json_valid(document_json)),
    created_utc_millis INTEGER NOT NULL CHECK (created_utc_millis >= 0),
    UNIQUE (profile_id, revision)
) STRICT;

CREATE TABLE session_context_snapshots (
    session_id TEXT PRIMARY KEY,
    resolved_context_json TEXT NOT NULL CHECK (json_valid(resolved_context_json)),
    created_utc_millis INTEGER NOT NULL CHECK (created_utc_millis >= 0)
) STRICT;

CREATE TRIGGER session_context_snapshots_no_update
BEFORE UPDATE ON session_context_snapshots
BEGIN
    SELECT RAISE(ABORT, 'session context snapshots are immutable');
END;

CREATE TRIGGER session_context_snapshots_no_delete
BEFORE DELETE ON session_context_snapshots
BEGIN
    SELECT RAISE(ABORT, 'session context snapshots are immutable');
END;

CREATE TABLE operational_events (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    event_id TEXT NOT NULL UNIQUE,
    service_instance_id TEXT NOT NULL,
    event_json TEXT NOT NULL CHECK (json_valid(event_json)),
    occurred_utc_millis INTEGER NOT NULL CHECK (occurred_utc_millis >= 0)
) STRICT;

CREATE TABLE outbox_work (
    outbox_id TEXT PRIMARY KEY,
    work_kind TEXT NOT NULL,
    payload_json TEXT NOT NULL CHECK (json_valid(payload_json)),
    idempotency_key TEXT NOT NULL UNIQUE,
    state TEXT NOT NULL DEFAULT 'pending' CHECK (state IN ('pending', 'completed')),
    created_utc_millis INTEGER NOT NULL CHECK (created_utc_millis >= 0)
) STRICT;

CREATE TABLE external_receipts (
    receipt_id TEXT PRIMARY KEY,
    outbox_id TEXT NOT NULL REFERENCES outbox_work(outbox_id),
    external_identity TEXT NOT NULL,
    receipt_json TEXT NOT NULL CHECK (json_valid(receipt_json)),
    received_utc_millis INTEGER NOT NULL CHECK (received_utc_millis >= 0),
    UNIQUE (outbox_id, external_identity)
) STRICT;
