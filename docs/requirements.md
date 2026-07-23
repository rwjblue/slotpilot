# Requirements

Requirement identifiers are stable references for issues, tests, and design reviews. This document describes intended scope; an issue still defines the exact implementation authorized in a pull request.

## Functional requirements

### Service and clients

- **F-001** SlotPilot shall run a single local station service that owns live audio, rig, PTT, protocol, scheduler, and operating-session state.
- **F-002** The desktop client and CLI shall use the same versioned command/result/event API.
- **F-003** Every desktop mutation shall have an equivalent API and CLI route.
- **F-004** The CLI shall support bounded JSON output and streaming JSON Lines events without interactive prompts.
- **F-005** Mutating commands shall accept stable request IDs and define idempotent retry behavior.

### FT8

- **F-100** SlotPilot shall decode and encode ordinary FT8 messages through a replaceable protocol boundary.
- **F-101** SlotPilot shall support standard FT8 QSO sequencing with typed states and messages.
- **F-102** The operator shall be able to select one-and-stop, drain-and-stop, drain-then-CQ, and continuous-attended run behavior.
- **F-103** QSO-stage retry limit and unanswered-CQ-cycle limit shall be independently configurable.
- **F-104** The initial automatic caller selector shall be longest waiting among eligible callers.
- **F-105** The operator shall be able to pin, select, skip, or reprioritize callers.
- **F-106** SlotPilot shall support configurable duplicate/ignore rules across callsign, time, band, mode, profile, and location dimensions.
- **F-107** Every automatic caller eligibility and selection decision shall be explainable.
- **F-108** SlotPilot shall support automatic or manual odd/even CQ parity.
- **F-109** SlotPilot shall support automatic recommendation or selection of an audio-frequency lane between QSOs.
- **F-110** SlotPilot shall validate the actual message sequence for special/compound station calls before arming automatic operation.
- **F-111** Unresolved hashed calls or unsupported messages shall not drive automatic transitions.

### WSPR

- **F-200** SlotPilot shall decode and locally store WSPR spots.
- **F-201** SlotPilot shall generate and schedule WSPR transmissions.
- **F-202** SlotPilot shall upload WSPR receive spots through a replaceable WSPRnet integration.
- **F-203** Upload failure shall not discard local decode data.
- **F-204** The scheduler model shall support a future sequence of per-band transmissions even though automatic band hopping is initially deferred.

### Profiles and hardware

- **F-300** SlotPilot shall compose operator, station, activation, rig, audio, and operating profiles.
- **F-301** Profiles shall be versioned and operating sessions shall snapshot resolved revisions.
- **F-302** Station callsign, operator callsign, and owner/host callsign shall remain distinct.
- **F-303** Separate input and output audio devices and channels shall be selectable.
- **F-304** Configured audio devices shall use stable platform identifiers when available and shall not silently fall back to system defaults.
- **F-305** Rig control shall begin with a persistent `rigctld`/Hamlib adapter and capability probing.
- **F-306** Initial validation targets shall be Elecraft K4, Yaesu FT-891 with DigiRig, and Yaesu FTDX10.
- **F-307** The FT8 MVP shall fully validate at least one designated initial
  radio/audio configuration; completing validation of the other initial
  targets shall not gate the first FT8 MVP.

### Logging and integrations

- **F-400** Completed QSOs shall be stored transactionally in SQLite.
- **F-401** ADIF import and export shall preserve station/operator distinctions and ordinary FT8 exchange fields.
- **F-402** Log and spot integrations shall use replaceable traits and durable outboxes.
- **F-403** WSPR spots shall not be mixed into the FT8 QSO log as ordinary contacts.
- **F-404** SlotPilot shall provide a stable process-separated integration path for AntennaBench.

## Safety requirements

- **S-001** Transmission shall require explicit, unexpired operator authority.
- **S-002** Process restart or recovery shall never restore transmit authority automatically.
- **S-003** One transmit supervisor shall be the sole logical owner of PTT.
- **S-004** An independent watchdog shall be able to deassert PTT after a mode-specific maximum duration.
- **S-005** Loss of the configured output stream during transmission shall trigger immediate dekey and inhibit further transmission.
- **S-006** Rig disconnect or inability to verify requested state shall inhibit transmission.
- **S-007** Unexpected frequency, mode, or VFO changes shall pause automation instead of fighting the operator.
- **S-008** Clock jumps or unhealthy synchronization shall inhibit synchronized transmission.
- **S-009** Emergency stop shall bypass ordinary queueing and admission paths.
- **S-010** Startup and shutdown shall perform best-effort unkey without treating that as the sole protection.
- **S-011** FT8 and WSPR schedules shall not overlap.
- **S-012** Test-tone and PTT-test functions shall have hard time limits.
- **S-013** Ordinary CI shall not require or cause physical RF transmission.
- **S-014** Automatic FT8 transitions shall require resolved sender, recipient, parity, and state consistency.

## Non-functional requirements

- **N-001** The architecture shall support macOS, Windows, and Linux; macOS is the primary development and packaging target.
- **N-002** Audio callbacks shall avoid allocation, blocking I/O, database access, rig access, and GUI locks.
- **N-003** Slot scheduling shall use monotonic deadlines mapped to UTC rather than relying only on wall-clock sleeps.
- **N-004** Time-dependent behavior shall be deterministic under an injected virtual clock.
- **N-005** Library crates shall expose typed `thiserror` errors; executables may add `anyhow` context.
- **N-006** Protocol dependency types shall not leak through SlotPilot public APIs.
- **N-007** Local IPC shall be machine-local by default and shall not listen on all network interfaces.
- **N-008** Wire schemas and persisted schemas shall be explicitly versioned and migration-tested.
- **N-009** The application shall retain structured diagnostics for audio overruns/underruns, clock health, rig state, state transitions, and integration attempts.
- **N-010** External side effects shall be recoverable without duplicating committed QSOs or spot records.

## Delivery acceptance themes

### Contact-capable alpha

The Phase 5 contact-capable alpha is not complete until it demonstrates:

- reproducible reference FT8 decode/encode results;
- safe failure under audio loss, rig loss, clock jump, panic, and restart;
- correct special-call message planning;
- correct UTC-date behavior at midnight;
- distinct station and operator ADIF output;
- crash-safe QSO and outbox handling;
- immediate operator pause, disarm, and emergency stop through the API and CLI;
- one bounded human-observed QSO with the designated primary configuration
  after dummy-load and failure-path validation passes.

This is an engineering milestone, not a packaged or generally supported
release.

### FT8 MVP

The Phase 6 FT8 MVP is not complete until the contact-capable alpha criteria
pass and it demonstrates:

- explainable duplicate and caller-selection behavior;
- deterministic queue advancement after durable QSO completion;
- bounded one-and-stop, drain-and-stop, drain-then-CQ, and
  continuous-attended operation;
- immediate operator override through the API, CLI, and minimum desktop
  operator console;
- one human-observed bounded CQ run with the designated primary configuration.

The FT8 MVP remains a development release until Phase 9.

### Packaged product release

The first packaged product release is not ready until:

- all FT8 MVP criteria remain satisfied;
- WSPR receive, transmit, storage, and upload failure behavior pass their
  Phase 8 gates;
- WSPR records cannot cross into the FT8 QSO log;
- all three initial hardware targets have explicit validation evidence;
- supported-platform packaging, permission, recovery, backup, migration, and
  safety smoke tests pass;
- immediate operator override remains available from the packaged desktop and
  CLI clients.
