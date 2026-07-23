# Work tracking

GitHub Issues are SlotPilot's durable record for unfinished work and open
implementation decisions. Roadmap phases describe sequencing; they do not
authorize implementation of an entire phase. The Phase 0 backlog seeded the
initial issue set, but focused issues govern execution once created.

Accepted decision [0010](decisions/0010-github-issues-and-agent-handoffs.md)
records this operating model.

## Tracking layers

Each layer answers a different question:

- The [roadmap](roadmap.md) describes product outcomes and risk-reduction order.
- A GitHub milestone groups issues that deliver one recognizable outcome.
- A tracking issue maps focused child issues, dependencies, and exit criteria.
- A focused implementation, decision, bug, or validation issue owns one bounded
  outcome and its acceptance evidence.
- An ADR records a durable settled decision. It does not track unfinished work.
- A local agent plan may elaborate an approved issue while work is active, but
  it is temporary and non-authoritative.

An issue belongs to at most one milestone. Cross-cutting relationships remain
linked from issue bodies and tracking checklists.

## Labels

Lifecycle and authority:

```text
agent-ready
in-progress
needs-decision
human-required
tracking
decision
blocked
safety
```

Roadmap phase:

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
```

Area:

```text
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
```

Kind and priority:

```text
kind: design
kind: implementation
kind: bug
kind: test
kind: maintenance

priority: critical
priority: high
priority: normal
```

The normal implementation lifecycle is:

```text
planned -> agent-ready -> in-progress -> landed and closed
               |              |
               |              +-> needs-decision
               |
               +-> explicit handoff authorizes implementation
```

`agent-ready` means the contract is bounded, objectively verifiable, and every
blocking dependency has landed. Readiness is not authorization; a user must
explicitly hand the issue to an agent.

Tracking issues are not executable and must not receive `agent-ready`.
`human-required` marks work whose complete evidence requires owner action,
credentials, regulatory judgment, physical-hardware validation, or real
operator observations. An agent may prepare explicitly handed-off artifacts but
may not claim the required human evidence.

## Issue types

Use the repository templates:

- **Planned implementation** for a bounded future slice with open dependencies.
- **Agent-ready implementation** for a bounded slice whose dependencies landed.
- **Agent-ready technical decision** for delegated technical research and a
  recommendation.
- **Product, safety, or owner decision** for choices requiring human judgment or
  authority.
- **Human or hardware validation** for physical equipment, on-air, operator, or
  credentialed evidence.
- **Tracking issue** for a milestone or multi-issue dependency map.
- **Bug report** for incorrect or unsafe implemented behavior.

An implementation issue states one outcome, context, implementation contract,
in-scope deliverables, explicit non-goals, dependencies, objective acceptance
criteria, safety and physical-hardware implications, and documentation or ADR
impact.

Avoid issues such as "implement daemon" or "add FT8." Split work at a boundary
that can be reviewed and tested independently.

## Dependency maintenance

- Design decisions precede implementations that would make them expensive to
  reverse.
- Interfaces and fakes precede physical adapters.
- Receive paths precede transmit paths.
- Manual bounded transmit precedes automatic sequencing.
- Persistence transactions precede queue advancement or external upload retries.
- CLI/API behavior is designed before a desktop-only workflow.
- Before beginning an issue, confirm every `Depends on` issue has landed on the
  remote default branch.
- After landing an issue, update its tracker and reassess every open issue it
  unblocks.
- Apply `agent-ready` only after all remaining blockers land. Remove stale
  readiness whenever an unmet dependency is discovered.
- A local-only commit or bookmark does not satisfy a GitHub dependency.

## Completion evidence

Implementation issues close only after their work lands. Their completion
record includes:

- delivered behavior;
- Jujutsu change ID and landed commit or pull request;
- verification commands and results;
- safety and physical-hardware implications;
- maintained documentation and ADR updates;
- follow-up issues and tracker changes; and
- explicitly deferred or blocked behavior.

Decision issues record the selected option, alternatives, rationale,
consequences, ADR when warranted, and focused follow-up issues. Human-required
issues record aggregate evidence without exposing credentials, private station
details, or unsafe operating information.

## Useful queries

```text
is:issue is:open label:agent-ready -label:in-progress
is:issue is:open label:in-progress
is:issue is:open label:needs-decision
is:issue is:open label:human-required
is:issue is:open label:tracking
is:issue is:open milestone:"Phase 0 — Workspace and contracts"
```

The first query is the executable queue. If it contains work with an open
blocking dependency, correct the labels before handing out more work.

## Pull request practice

Use a Jujutsu bookmark scoped to the issue. A pull request should close one
focused issue when possible. Large stacks may use dependent draft pull requests,
but every revision must remain buildable, testable, and independently
reviewable. A tracking issue coordinates a stack; it does not replace focused
pull-request scope.

## First milestone

The first milestone is
[**Phase 0 — Workspace and contracts**](https://github.com/rwjblue/slotpilot/milestone/1),
coordinated by
[tracking issue #1](https://github.com/rwjblue/slotpilot/issues/1). The initial
focused contracts were seeded from [the Phase 0 backlog](backlog/phase-0.md).
