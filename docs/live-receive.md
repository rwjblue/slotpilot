# Live FT8 receive coordinator

Phase 2 composes live input only inside `slotpilotd`. Construction and daemon
restart leave receive stopped. A caller must explicitly start the exact stable
device identity and checked configuration already held by the coordinator.
There is no default-device lookup, display-name recovery, device fallback, or
automatic restart.

## Ownership and worker boundary

The daemon owns at most one native input stream and one live receive
coordinator. The native callback only normalizes samples into the audio crate's
preallocated bounded queue and emits counters or faults. A daemon
worker/event-loop poll copies out at most four batches into a second bounded
queue and processes at most one batch. Rational resampling, canonical-window
assembly, clock gating, offline FT8 decode, SQLite transactions, and internal
event creation all occur after the callback boundary. Worker concurrency is
therefore one and worker backlog is fixed at four batches.

One poll either:

- has no work;
- reports an incomplete slot that was deliberately withheld;
- persists one complete clock-healthy canonical window and its deterministically
  ordered typed decode outcomes; or
- enters an inhibited state and stops input.

## Lifecycle and generations

Lifecycle transitions are serialized and typed:

`stopped -> starting -> receiving -> stopping -> stopped`

Any input, timeline, clock, decoder, or storage failure instead reaches
`inhibited`. An inhibited coordinator performs no more work. It must be
explicitly stopped before another explicit start. Every start attempt reserves
a fresh stream generation, including a failed attempt, and the process-scoped
clock generation must match the capture process generation. Cancellation is an
explicit stopping reason. Restart constructs a new stopped coordinator and
does not recover an active stream from SQLite.

Device loss, callback overflow or discontinuity, stale or unhealthy time,
timeline invalidation, decoder failure, and storage failure are all terminal
for that receive generation. None silently switches inputs or treats uncertain
evidence as decoder-ready.

## Durable identity and evidence

Only a complete window admitted by the independent receive-clock gate reaches
the Phase 1 `Ft8OfflineDecoder` interface. Its receive-window ID is the
SHA-256 digest of owned stable context: service, process and stream generation,
slot, platform device identity, and exact selected configuration. Retrying the
same record therefore returns SQLite's existing outcome, while a later
generation cannot collide with stale work.

The schema-v2 transaction stores the exact device and configuration, capture
mapping, bounded audio/timeline/clock diagnostics, and every resolved,
unresolved-hash, unsupported, ambiguous, or free-text decode with its metadata.
The coordinator derives no QSO state, caller policy, duplicate decision,
logging side effect, transmit authority, or scheduling decision.

This is an internal service seam only. It adds no API, CLI, or wire behavior,
waterfall publication, output device, rig connection, PTT path, transmit
authority, WSPR behavior, or RF action.
