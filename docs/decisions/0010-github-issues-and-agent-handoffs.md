# 0010 — GitHub issues and explicit agent handoffs

- Status: Accepted
- Date: 2026-07-23

## Context

SlotPilot is expected to be developed through narrow agent-assisted changes.
Roadmaps and issue-ready backlog prose are useful for sequencing and seeding
work, but they become stale if they also act as a second execution tracker.
Generated implementation plans are useful while work is active but are not a
durable project record.

Transmit safety, durable schemas, external side effects, and cross-platform
boundaries also make ambiguous implementation authority unusually risky.

## Decision

- GitHub Issues are the durable source of truth for unfinished work and open
  implementation decisions.
- Roadmaps describe outcome order, milestones group recognizable outcomes, and
  tracking issues map focused dependencies.
- Once a focused issue exists, its current contract governs implementation
  rather than the backlog text that seeded it.
- Local or generated plans are temporary execution aids and do not authorize
  implementation.
- `agent-ready` records that an issue is bounded, objectively verifiable, and
  unblocked. It does not authorize work without explicit user handoff.
- An explicit handoff authorizes only the focused issue's scope. Material
  expansion of public behavior, durable schema, safety authority,
  physical-hardware scope, or architecture requires user direction.
- A local commit does not satisfy a dependency. Implementation completion
  requires landed work and durable evidence on the issue.
- Work requiring credentials, owner authority, regulatory judgment, real
  operator observations, or physical-hardware validation is marked
  `human-required`.

## Consequences

- The Phase 0 backlog seeds issues but does not carry live execution status.
- Agents update lifecycle labels, completion evidence, parent trackers, and
  newly unblocked issues as part of an explicitly handed-off workflow.
- Newly discovered adjacent work becomes a focused issue instead of silently
  expanding active scope.
- Maintained documentation and ADRs continue to describe stable behavior and
  decisions rather than generated plan steps.
- Safety or architecture ambiguity remains visible as `needs-decision` instead
  of being resolved implicitly in code.

## Revisit when

- GitHub Issues no longer provide an adequate durable work-tracking surface;
- the project adopts another explicit tracker with equivalent handoff,
  dependency, and completion-evidence semantics;
- agent execution is no longer a material project workflow.
