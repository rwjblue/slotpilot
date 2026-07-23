# Phase 2 receive-only software conformance

This record closes the software-test boundary for Phase 2. It does not close
the phase: issue #36 still requires a human to validate the exact landed
revision against a real macOS input or RF-free loopback device.

## Boundary

The implemented station behavior is receive-only. `slotpilotd` alone owns an
explicitly selected input identity and configuration. It starts inactive,
admits bounded callback and worker work, gates complete FT8 windows on healthy
UTC/monotonic time, decodes behind SlotPilot-owned types, atomically commits
decode evidence with ordered public events, and exposes bounded API v2 and CLI
routes. Device loss or any input/timeline/clock/decode/storage fault inhibits
receive without selecting another device or restarting automatically.

There is no audio-output, radio, rig-control, PTT, transmit-authority,
transmit-scheduling, remote-network, WSPR, logging-sink, or desktop path.

## Conformance matrix

| Contract | Deterministic evidence |
| --- | --- |
| Stable discovery and exact selection | Discovery adapter tests retain duplicate display names as distinct stable identities; daemon API tests keep identity separate from display metadata. |
| Explicit start, status, stop, reconnect, and restart | API processor tests cover start/stop replay and conflict; local IPC tests cover reconnect and cancellation; restart snapshots are inactive even when an old start response replays. |
| Healthy live pipeline | Daemon replay tests pass complete canonical windows through clock gating, all five FT8 classifications, idempotent persistence, and ordered public events. |
| History, events, and waterfall | API fixtures and bounds cover receive status, history, decode events, and waterfall events; CLI tests render the same owned values in human, JSON, and JSONL modes. |
| Callback and worker backpressure | Platform-free capture tests exercise sustained overflow without callback allocation or blocking; daemon tests enforce four queued batches and one worker; waterfall publication coalesces to a fixed latest-row bound. |
| Time and timeline faults | Virtual tests cover discontinuity, drift, stale mappings, delayed sampling, UTC jumps in both directions, monotonic regression, suspend-like gaps, misalignment, and explicit recovery. |
| Input and processing faults | Daemon tests inject permission denial, device loss, overflow, discontinuity, clipping, drift, callback delay, backend failure, decoder failure, storage failure, cancellation, and shutdown. |
| Atomic durability | Storage tests prove receive evidence and its ordered event commit together, roll back together, and reuse the exact sequences on retry. |
| No fallback, stale work, duplication, or automatic restart | Generation and cancellation tests discard old batches; request IDs replay exactly or conflict deterministically; inhibited and restarted processes remain inactive until a new explicit start. |
| Cross-platform and public-boundary safety | macOS, Linux, and Windows run `mise run ci`; dependency audits and rustdoc checks reject private audio, FFT, decoder, and SQLite types from public API/CLI surfaces. |

## Automated gates

`mise run check` is the development loop. `mise run ci` is the complete
software landing gate and includes `mise run check-phase2-receive`. The Phase 2
audit checks fixed queue/page/event limits, reviewed API fixtures, public
dependency direction, absence of remote/audio-output/rig/PTT/transmit paths,
named fault and retry evidence, public rustdoc, and the existence of the
human-validation protocol.

All automated receive tests use fakes, replay, generated samples, or local IPC.
They do not enumerate or open a physical device and do not access a radio or
emit RF.

## Remaining human boundary

Follow [`validation/phase2-macos-input.md`](validation/phase2-macos-input.md)
only after every software issue is reachable from remote `main` and its
cross-platform GitHub Actions run is green. Record the exact landed revision
before starting. Generated or simulated evidence cannot satisfy issue #36.

The Phase 3 rig boundary remains unimplemented and separately authorized.
Nothing in this closeout creates a Phase 3 tracker, selects hardware, or grants
read-only rig access.
