# Product design

## Operating model

SlotPilot is an attended operating assistant. The operator selects a station context, confirms radio/audio readiness, and explicitly arms a bounded operating run. SlotPilot may then perform only the actions permitted by that run policy.

The main FT8 preset is **Drain then CQ**:

```text
Call CQ
  -> collect eligible callers
  -> work the longest-waiting eligible caller
  -> log the completed QSO
  -> work the next eligible caller
  -> when the queue is empty, call CQ again
  -> pause after the configured unanswered-CQ limit
```

Other selectable policies are expected:

- **One and stop**: complete one QSO, then disarm;
- **Drain queue and stop**: finish the eligible queue, then stop;
- **Drain then CQ**: finish the queue and resume CQ;
- **Continuous attended**: continue until the arm expires, a configured limit is reached, or the operator stops.

## FT8 QSO behavior

Automatic transitions are driven by typed, resolved FT8 messages and a deterministic state machine. Standard exchanges are in scope, including representable special/compound calls such as `W1AW/1`.

Free text, unsupported contest messages, unresolved callsign hashes, or ambiguous messages may be displayed but must not silently advance automatic operation.

Separate retry controls are required for:

- a QSO stage, such as repeating a report or final acknowledgement;
- unanswered CQ cycles after the caller queue becomes empty.

A caller heard while another QSO is active may remain queued when the message is clearly directed to this station.

## Caller queue

The initial selector is **oldest eligible caller**, using first-seen UTC as the primary ordering key. Later selectors may include first decoded, strongest, weakest, repeated-call count, new grid, new DXCC, weighted scoring, and manual ordering.

A caller entry retains:

- full and normalized base callsign;
- first and last seen times;
- call count;
- last and best SNR;
- grid when available;
- audio frequency and parity;
- eligibility and rule trace;
- selection-score explanation.

The operator can pin, select, skip, or manually prioritize a caller at any time.

## Duplicate and ignore policy

The default worked-before key is:

```text
remote base call + UTC date + band + mode + station callsign
```

Rules may use full/base call, day, week, session, band, mode, station call, operator call, grid, DXCC, or activation context. Actions include allow, ignore, deprioritize, prioritize, highlight, and require-manual-confirmation.

Every decision exposes the matching rule and relevant completed QSO or context.

## Odd/even parity and audio lane

Parity and audio lane are independent decisions.

- When answering a station, parity follows the decoded exchange.
- When starting CQ, SlotPilot scores recent odd and even occupancy and selects the quieter option when automatic parity is enabled.
- A selected CQ parity remains stable through the run unless the operator changes it or a new run begins.

Audio-lane planning uses recent spectral energy, decoded-signal locations, strong-signal penalties, persistent carriers, passband edges, collision history, and hysteresis. It returns a recommendation with an explanation.

Automatic lane movement is initially limited to boundaries between QSOs or before a CQ run. It never moves during an active exchange.

## WSPR

WSPR is a separate coordinator, not an FT8 QSO state.

Receive mode:

- captures and decodes the WSPR interval;
- stores every local result;
- emits client events;
- queues optional WSPRnet upload;
- preserves data across upload failure.

Transmit mode initially supports:

- one transmission;
- a bounded number of transmissions;
- a bounded duration;
- a configured transmit percentage;
- explicitly selected two-minute slots.

The schedule model includes band and frequency from the beginning so future band hopping does not require replacing the scheduler.

## Logging

SQLite is authoritative. A completed FT8 QSO, transcript, duplicate update, and pending log-sink work are committed together. ADIF is the first sink and import/export format.

The log model supports distinct:

- station callsign;
- operator callsign;
- owner/host callsign;
- station and remote grids;
- activation references;
- reports sent and received;
- dial and audio-offset frequency information;
- station/radio/antenna descriptions;
- start and completion UTC.

Historical ADIF import is required before duplicate policy is considered complete.

## Profiles

Operating state is composed from versioned operator, station, activation, rig, audio, and operating profiles. Starting a session snapshots the resolved revisions so later profile edits do not rewrite history.

## User interfaces

Desktop delivery is incremental. The FT8 MVP includes a minimum operator
console for station health, decode activity, caller queue, current QSO
transcript, arm/pause/stop controls, and transmit inhibition reasons. The
packaged product expands that client with the full waterfall, profile
management, logging, WSPR status, diagnostics, backup, and recovery workflows.

The CLI provides equivalent commands and a JSON/JSONL mode suitable for scripts and AntennaBench. The desktop application must not bypass the service API.
