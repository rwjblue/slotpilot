# Logging and spot integrations

## SQLite source of truth

ADIF files and network services are sinks. SlotPilot first commits domain records locally.

For a completed FT8 QSO, one transaction records:

- completed QSO identity and fields;
- transcript and QSO state events;
- worked-before/duplicate projection update;
- pending log-sink outbox items.

For a WSPR decode, one transaction records:

- local spot and decode diagnostics;
- receiving station/profile context;
- pending WSPRnet outbox item when enabled.

## Log sink trait

Conceptually:

```rust
#[async_trait::async_trait]
pub trait LogSink: Send + Sync {
    fn sink_id(&self) -> &'static str;

    async fn submit(
        &self,
        qso: &CompletedQso,
    ) -> Result<LogReceipt, LogSinkError>;
}
```

The initial sink is local ADIF. Future sinks may include QRZ, Club Log, eQSL, or TQSL without changing QSO completion logic.

Do not load arbitrary native dynamic plugins initially. In-tree traits and later process-separated JSON integrations are safer and more portable.

## ADIF

Expected ordinary FT8 fields include:

```text
CALL
QSO_DATE
TIME_ON
QSO_DATE_OFF
TIME_OFF
BAND
FREQ
MODE=FT8
RST_SENT
RST_RCVD
GRIDSQUARE
STATION_CALLSIGN
OPERATOR
OWNER_CALLSIGN when applicable
MY_GRIDSQUARE
TX_PWR
activation references when applicable
```

Internally retain both dial frequency and audio-offset-derived on-air information so export policy is explicit rather than lossy by accident.

Historical ADIF import feeds duplicate policy and must retain provenance. Import does not imply every record was created by SlotPilot.

## WSPRnet

WSPRnet is an external, informally versioned service. Its adapter must be replaceable and isolated from decode storage.

Upload behavior:

- queue after local commit;
- retry transient failures with bounded backoff;
- retain last response and attempt metadata;
- avoid duplicate submission when a stable receipt or idempotency strategy is available;
- never delete local data merely because upload failed;
- expose queue status to CLI and desktop clients.

## Separation of record types

A received WSPR spot is not an FT8 QSO. The schemas, UI, exports, and integrations remain separate even when they share station/profile and frequency types.
