# 0008 — Cross-platform design with macOS as primary platform

- Status: Accepted
- Date: 2026-07-23

## Context

SlotPilot must support macOS, Windows, and Linux. The project owner's personal platform is macOS, which will receive the earliest daily use, packaging, and hardware validation.

## Decision

- Treat all three operating systems as first-class architecture targets.
- Use macOS as the primary development and first packaging target.
- Isolate platform-specific audio identity, IPC, permissions, helper lifecycle, and packaging behind adapters.
- Do not persist only device display names.

## Consequences

- CI and API design must expose platform differences rather than hiding them in domain code.
- macOS microphone permission, signing, helper, and notarization work is planned early.
- Windows named-pipe and Linux audio/backend behavior remain roadmap requirements, not later ports.

## Revisit when

- a supported platform is formally dropped or a new platform is added.
