# Attended-operation and transmit safety

This document defines invariants. Implementations may add stricter behavior but may not weaken these rules without an accepted architecture decision and explicit review.

## Current capability boundary

Phase 1 produces and inspects FT8 bits, in-memory PCM, and static RIFF/WAVE
files. Phase 2 adds one daemon-owned, explicitly selected receive-only input,
bounded processing, durable decode evidence, ordered local events, and public
API/CLI observation and control. Receive starts inactive and device, time,
timeline, decode, or storage failure inhibits it without fallback or automatic
restart. Ordinary tests open no physical input, and the separate macOS
validation must use only RF-free known audio or loopback.

These artifacts and receive buffers are not transmit plans, operating
authority, or evidence of safe on-air behavior. No audio output, radio,
rig-control adapter, PTT owner, transmit scheduler, transmit authority, or
automatic QSO transition exists in the current implementation.

Phase 3.1 adds read-only profile, capability, observation, validation, and fake
contracts only. Its consumer-owned rig port has no setter, raw-command, PTT, or
unkey method. Optional PTT readback is evidence, never control. The reserved
rig crate contains no process, network, serial, Hamlib, or physical-radio
implementation. Managed profiles require a distinct loopback service endpoint;
the later lifecycle adapter must force PTT type `NONE`.

## Authority

Transmission requires an explicit operator arm tied to:

- a selected station-context snapshot;
- permitted mode or run type;
- maximum duration and/or operation count;
- current daemon process lifetime;
- a unique authority token.

The arm expires automatically. Background decodes, client reconnects, daemon restart, profile reload, or recovered database state cannot renew it.

## Single PTT owner

Only `TxSupervisor` may request PTT assertion. Rig adapters expose a controlled operation to it; FT8, WSPR, GUI, CLI, protocol, and tests do not key the rig directly.

The supervisor accepts an immutable prepared plan containing:

- operating mode;
- UTC slot and monotonic deadline;
- dial frequency and audio frequency;
- complete prepared waveform;
- station-context snapshot;
- PTT lead/tail;
- arm token;
- maximum expected duration.

## Admission checks

Before accepting a transmit plan, verify:

- authority is present and unexpired;
- requested operation is permitted by the arm;
- clock mapping and health are acceptable;
- rig is connected and state is current;
- frequency, mode, VFO, and split state match the verified plan;
- output device is the configured device and is healthy;
- waveform is complete before its deadline;
- no other transmission is active or scheduled to overlap;
- configured power and regulatory/profile constraints are satisfied;
- no emergency-stop or inhibit latch is active.

A failed check produces a typed inhibition with operator-readable reasons.

## Active-transmission safeguards

- An independent watchdog knows the maximum permitted keyed duration and can call emergency unkey.
- Loss of output audio or an unrecoverable underrun dekeys immediately.
- Rig disconnect or inability to confirm PTT state triggers best-effort unkey and a persistent inhibit.
- Emergency stop bypasses normal command queues.
- Shutdown and panic boundaries attempt unkey, but correctness does not depend on destructors alone.
- Test-tone and PTT-test modes have hard, non-configurable upper limits in addition to user settings.

## Unexpected operator or hardware changes

SlotPilot does not fight the operator. Unexpected frequency, mode, VFO, split, or output-device changes pause automation. The UI and CLI report the expected and observed states and require explicit resolution.

## Clock behavior

Synchronized modes schedule against monotonic deadlines derived from sampled UTC/monotonic pairs. A material wall-clock jump, mapping inconsistency, or unhealthy time source inhibits new synchronized transmissions.

A decode `DT` trend is diagnostic evidence, not a substitute for trusted local time.

## Recovery

On start or recovery:

1. establish rig connection only according to explicit startup configuration;
2. request PTT off before ordinary state restoration;
3. load durable sessions, attempts, logs, and outboxes;
4. mark incomplete on-air operations inactive;
5. restore no transmit authority;
6. require the operator to review current rig, audio, time, and profile state before re-arming.

## Automatic FT8 limits

A decoded message may advance a QSO only when:

- sender and recipient match the active attempt;
- full calls or hashes are sufficiently resolved;
- message type is supported;
- parity and timing are consistent;
- the current state permits the transition;
- the run remains armed;
- the operator has not paused, skipped, or changed the selected caller.

Free text and unknown structured messages do not drive automation.

## Development and test safety

- Fake rig/audio/protocol implementations are the default.
- Phase 2 software conformance uses fake, replay, generated, or local-loopback
  seams; ordinary CI never opens a physical input.
- A physical-hardware feature or test target requires explicit opt-in.
- No test assumes an antenna is connected.
- PTT tests use a dummy load or equivalent safe setup and short hard limits.
- Captured rig transcripts must not contain private credentials or unrelated station data.
- On-air experiments are manual release steps, not CI.

## Security boundary

The local service endpoint is user-scoped and local-only. Remote network access is out of scope. Imported profiles, logs, or bundles cannot grant process execution or transmit authority.
