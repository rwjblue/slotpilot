# Work tracking

GitHub issues define authorized implementation scope. Roadmap phases describe sequencing, not permission to implement an entire phase in one pull request.

## Recommended labels

```text
phase: 0-bootstrap
phase: 1-protocol
phase: 2-receive
phase: 3-rig
phase: 4-transmit
phase: 5-qso-log
phase: 6-automation
phase: 7-lane-parity
phase: 8-wspr-integrations
phase: 9-packaging

area: domain
area: api
area: ipc
area: protocol
area: audio
area: rig
area: operations
area: policy
area: storage
area: logging
area: integration
area: desktop
area: cli
area: docs

kind: design
kind: implementation
kind: bug
kind: test
kind: maintenance

priority: critical
priority: high
priority: normal
safety
blocked
```

Labels should be created only when issues begin; this document is the canonical initial vocabulary.

## Issue structure

An implementation issue should state:

- one goal;
- in-scope deliverables;
- explicit non-goals;
- dependencies;
- acceptance criteria;
- safety and physical-hardware implications;
- documentation/ADR impact.

Avoid issues such as “implement daemon” or “add FT8.” Split work at a boundary that can be reviewed and tested independently.

## Dependency practice

- Design decisions precede implementations that would make the decision expensive to reverse.
- Interfaces and fakes precede physical adapters.
- Receive paths precede transmit paths.
- Manual bounded transmit precedes automatic sequencing.
- Persistence transactions precede queue advancement or external upload retries.
- CLI/API behavior is designed before a desktop-only workflow.

## Pull request practice

Use a branch scoped to the issue. A pull request should close one issue when possible. Large issue stacks may use dependent draft pull requests, but each branch must preserve a buildable, testable state.

## First milestone

The first milestone is **Phase 0 — Workspace and contracts**. Its proposed issues are specified in `backlog/phase-0.md`.
