# Agent instructions

This repository is designed so that a local coding agent can orient itself without reconstructing the product from chat history.

## Read order

Before changing files, read:

1. `README.md`
2. `docs/vision.md`
3. `docs/product.md`
4. `docs/requirements.md`
5. `docs/architecture.md`
6. `docs/safety.md`
7. `docs/roadmap.md`
8. the relevant records under `docs/decisions/`
9. the assigned issue or the matching task in `docs/backlog/phase-0.md`

When documentation conflicts, use this priority:

1. the current explicit issue or maintainer instruction;
2. accepted architecture decision records;
3. requirements and safety invariants;
4. architecture and product documentation;
5. roadmap and backlog descriptions.

Code and tests become authoritative only after implementation exists and the maintained documentation has been updated to match.

## Current repository state

The repository is a design scaffold. There is deliberately no Rust application or library implementation yet. Do not create broad speculative implementations in an orientation task.

A first implementation pull request should be narrow, reviewable, and tied to one Phase 0 task. It should not open an audio device, connect to a physical rig, key PTT, transmit RF, or introduce a desktop framework unless the assigned issue explicitly calls for it.

## Non-negotiable boundaries

### Operating safety

- SlotPilot is for attended operation. It must not become a general unattended QSO robot.
- `slotpilotd` is the only component permitted to own live rig, audio, PTT, slot scheduler, and operating-session state.
- One `TxSupervisor` is the sole logical PTT owner.
- A process restart must never restore transmit authority automatically.
- Audio failure, rig disconnect, clock-health failure, unexpected mode/frequency movement, or expired authority must inhibit or stop transmission.
- Tests must use fake or loopback hardware by default. Physical radio tests require an explicit opt-in and a documented dummy-load/test procedure.
- Never make a test depend on transmitting through an antenna.

### Architecture

- The desktop and CLI are clients of the same versioned service API.
- No feature may have a GUI-only mutation path.
- Library crates return typed errors implemented with `thiserror`.
- Executable/application boundaries may use `anyhow` for top-level context and reporting.
- Protocol dependencies such as `mfsk-core` must remain behind SlotPilot-owned traits and data types.
- CPAL callbacks must not allocate, block, access SQLite, call Hamlib, or lock GUI state.
- External integrations use durable outboxes and idempotent identities where side effects can be repeated.
- Profiles are versioned reusable objects; an operating session snapshots the exact resolved revisions it used.
- SQLite is the authoritative operational store. ADIF is an import/export format, not the live database.

### Cross-platform behavior

- macOS is the primary developer platform, not a license to embed macOS assumptions in domain code.
- Platform-specific audio identity, IPC, packaging, and permissions belong behind explicit adapters.
- Unix-domain sockets are expected on macOS/Linux and named pipes on Windows. Loopback TCP is development-only unless a later decision changes this.
- Persist stable audio identifiers when the platform provides them; never identify a device only by its display name.

## Expected workspace shape

The planned workspace is documented in `docs/architecture.md`. Do not create every crate at once merely because it appears there. Create the smallest subset needed by the assigned task, keeping dependency direction toward domain types and away from applications.

Expected binaries:

- `slotpilotd`: local station daemon;
- `slotpilot`: CLI client;
- desktop application name to remain `SlotPilot` even if its package identifier differs.

## Development rules

- Use Rust 2024 edition unless an accepted decision changes it.
- Prefer explicit domain types over strings and primitive integers for callsigns, frequencies, bands, slot identities, request IDs, and profile revisions.
- Avoid `unwrap`, `expect`, and panics in production paths. Tests may use them when the failure message is clear.
- Make time an injected dependency. Slot-bound behavior must be testable with a virtual clock.
- Commands that may be retried require stable request IDs and deterministic duplicate handling.
- Wire schemas are versioned and should have serialization fixtures before clients depend on them.
- A state-machine transition should be driven by typed protocol messages, not string matching in UI code.
- Add an architecture decision record when changing a durable boundary, not for routine implementation detail.
- Keep generated files out of the repository unless they are deterministic and reviewed artifacts required by packaging or tests.

## Pull-request expectations

A pull request should state:

- the issue/task it implements;
- the boundary it changes;
- user-visible or API-visible behavior;
- tests and commands run;
- safety implications, including an explicit statement when there are none;
- documentation or ADR updates.

For any change related to transmit scheduling, PTT, rig mutation, audio output, automatic caller selection, duplicate policy, profile resolution, or durable side effects, include focused failure-path tests.

## Suggested first assignment

The safest first task is **Phase 0.1: establish the workspace skeleton and boundary crates** from `docs/backlog/phase-0.md`.

That task should create manifests and compile-only shells for the smallest agreed crate set, establish dependency direction, and add CI for formatting, linting, and tests. It must not implement radio, audio, protocol, persistence, or UI behavior.

## Definition of done

A task is complete when:

- its acceptance criteria pass;
- tests cover the behavior and important failure paths;
- public types and wire changes are documented;
- the CLI path exists or is explicitly deferred by the issue because the API does not yet exist;
- no physical hardware is required for normal CI;
- relevant design documents remain accurate;
- the pull request is narrow enough to review as one coherent change.
