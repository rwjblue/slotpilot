# 0002 — One daemon with multiple API clients

- Status: Accepted
- Date: 2026-07-23

## Context

The desktop application, CLI, and external local integrations must produce consistent behavior. Rig serial ports, audio streams, PTT, slot scheduling, and live QSO state cannot be safely owned by multiple processes independently.

## Decision

Create one local daemon, `slotpilotd`, that is the sole owner of live station hardware and operating state. The desktop application, `slotpilot` CLI, and external applications interact through the same versioned local API.

## Consequences

- No desktop-only station logic or private mutation path.
- Client failure does not imply hardware ownership loss or transfer.
- Daemon lifecycle, packaging, permissions, and local IPC become first-class product concerns.
- Tests can exercise the service through the same contract used by users and integrations.

## Revisit when

- a platform makes the helper-process model infeasible;
- a future embedded deployment requires a single-process build while retaining the same logical ownership boundary.
