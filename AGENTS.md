jj-commit-default: auto

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
9. the explicitly assigned GitHub issue

When documentation conflicts, use this priority:

1. the current explicit issue or maintainer instruction;
2. accepted architecture decision records;
3. requirements and safety invariants;
4. architecture and product documentation;
5. roadmap and backlog descriptions.

Code and tests become authoritative only after implementation exists and the maintained documentation has been updated to match.

## Planning and issue authority

- GitHub Issues are the durable source of truth for unfinished work and open
  implementation decisions.
- Roadmaps and backlogs describe sequencing and seed issue creation. Once a
  focused issue exists, its current contract and GitHub state govern execution.
- Local or generated plans are temporary execution aids. They do not authorize
  implementation and must not become the only record of unfinished work.
- Do not implement from a plan until the user approves it. An explicitly
  handed-off, agent-ready GitHub issue is approved implementation scope.
- The `agent-ready` label records readiness only. It does not authorize work to
  begin without explicit handoff.
- Stop and request direction before materially expanding an approved issue's
  public behavior, durable schema, safety authority, physical-hardware scope, or
  architectural boundary.
- When implementation changes stable behavior, update the relevant maintained
  documentation in the same change when practical.

## Issue workflow

When explicitly handed a GitHub issue:

- Confirm every blocking dependency has landed on the remote default branch.
- Replace `agent-ready` with `in-progress` and inspect the current checkout
  before planning or implementing.
- Stay within the issue contract; create a focused follow-up issue rather than
  silently absorbing newly discovered work.
- Run verification proportional to the change and the complete repository gate
  before landing once that gate exists.
- Land the work before declaring completion. A local-only commit or bookmark
  does not satisfy a dependency.
- Post completion evidence, close the focused issue, and update its tracking
  issue.
- Review open issues that the landed work unblocks. Apply `agent-ready` only
  when all remaining dependencies have landed and the issue remains bounded
  and objectively verifiable. Remove stale readiness when a blocker is found.
- If blocked by a product, safety, or architecture choice, apply
  `needs-decision`, explain the blocker and viable choices, and leave the issue
  open.
- Treat `human-required` as a completion boundary. Agents may prepare explicitly
  handed-off artifacts, but may not substitute generated evidence for owner
  action, credentials, regulatory judgment, physical-hardware validation, or
  real operator observations.

## Current repository state

Phases 0 and 1 are complete. The repository has a development-only no-op
daemon/CLI handshake, versioned API contracts, local IPC, deterministic time
and hardware test seams, initial SQLite durability, and the complete repository
gate. `slotpilot-protocol` also has a bounded offline FT8-only harness:
SlotPilot-owned message outcomes, an exact private dependency adapter, reviewed
fixtures, deterministic in-memory PCM synthesis, bounded RIFF/WAVE parsing,
and reproducible static-recording decode. There is deliberately no live audio,
station FT8 command/event path, WSPR protocol, audio-device, rig-control,
logging, transmit, or desktop behavior. Do not create broad speculative
implementations in an orientation task.

An implementation pull request should be narrow, reviewable, and tied to one
focused issue. It should not open an audio device, connect to a physical rig,
key PTT, transmit RF, or introduce a desktop framework unless the assigned
issue explicitly calls for that boundary and its prerequisite safety work has
landed.

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

## Version control

- Use Jujutsu (`jj`) for maintained local version-control workflows.
- Do not create Git worktrees unless explicitly requested.
- Inspect `jj status` before starting and preserve unrelated user changes.
- Use bookmarks for issue branches and the shared `jj-pr` workflow for ordinary
  or stacked pull requests.
- Use `jj commit`, not `jj describe`, for a completed working-copy change so the
  working copy advances to a new empty revision.
- Pushable changes must remain signed. Never disable or bypass signing to make a
  push succeed.
- A change is landed only when it is reachable from the remote default branch,
  normally through a merged pull request or an explicitly authorized signed
  push.

## Development rules

- Use Rust 2024 edition unless an accepted decision changes it.
- Define shared third-party dependencies in the root `Cargo.toml` under
  `[workspace.dependencies]`.
- Reference shared dependencies from member crates with `workspace = true`.
- Prefer explicit domain types over strings and primitive integers for callsigns, frequencies, bands, slot identities, request IDs, and profile revisions.
- Avoid `unwrap`, `expect`, and panics in production paths. Tests may use them when the failure message is clear.
- Make time an injected dependency. Slot-bound behavior must be testable with a virtual clock.
- Commands that may be retried require stable request IDs and deterministic duplicate handling.
- Wire schemas are versioned and should have serialization fixtures before clients depend on them.
- A state-machine transition should be driven by typed protocol messages, not string matching in UI code.
- Add an architecture decision record when changing a durable boundary, not for routine implementation detail.
- Keep generated files out of the repository unless they are deterministic and reviewed artifacts required by packaging or tests.

## Validation

- Use `mise run check` for the fast local formatting, Clippy, and workspace-test
  loop.
- Use `mise run ci` for the complete repository landing gate, including
  toolchain, dependency-direction, documentation, and CI-configuration checks.
- Run the complete gate before committing, opening a pull request, and landing
  a change.

## Pull-request expectations

A pull request should state:

- the focused issue it implements;
- the boundary it changes;
- user-visible or API-visible behavior;
- tests and commands run;
- safety implications, including an explicit statement when there are none;
- documentation or ADR updates.

For any change related to transmit scheduling, PTT, rig mutation, audio output, automatic caller selection, duplicate policy, profile resolution, or durable side effects, include focused failure-path tests.

## Assignment source

Phase 0's completed implementation record is
[tracker #1](https://github.com/rwjblue/slotpilot/issues/1), and Phase 1's is
[tracker #16](https://github.com/rwjblue/slotpilot/issues/16). New
implementation must use focused issues linked from the active phase tracker.
Phase 2 receive-only work is not authorized by Phase 1 completion; it requires
its own tracker, bounded focused issues, landed prerequisites, revalidation,
and explicit user handoff. A downstream issue may begin only after its
dependencies have landed, its contract has been revalidated, and the user
explicitly hands it off.

## Definition of done

A task is complete when:

- its acceptance criteria pass;
- tests cover the behavior and important failure paths;
- public types and wire changes are documented;
- the CLI path exists or is explicitly deferred by the issue because the API does not yet exist;
- no physical hardware is required for normal CI;
- relevant design documents remain accurate;
- the pull request is narrow enough to review as one coherent change.
