# SlotPilot

**Attended weak-signal operation for FT8 and WSPR.**

SlotPilot is planned as a local-first, cross-platform station service that handles the repetitive parts of FT8 and WSPR operation while leaving the licensed operator present, informed, and in control.

The intended system has one hardware-owning daemon and two first-party clients:

- `slotpilotd`: owns rig control, audio streams, protocol timing, operating state, and transmit safety;
- `slotpilot`: a machine-friendly and human-friendly command-line client;
- SlotPilot Desktop: a native desktop client for macOS, Windows, and Linux.

All clients use the same versioned command/event API. No capability should exist only in the GUI.

> [!IMPORTANT]
> This repository contains the Phase 0 local service foundation, the Phase 1
> offline FT8 protocol harness, and initial owned Phase 2 receive-audio
> contracts, discovery, and fakes. Static/in-memory FT8 messages, waveforms,
> reviewed WAV fixtures, and bounded synthetic capture batches can be
> processed in tests. Receive-only device discovery can enumerate stable input
> identities, but no stream or microphone capture, radio, PTT, live decode,
> logging, transmit, WSPR, or desktop behavior is available; live
> station capability remains unavailable. Do not treat the repository or its
> unsigned local artifacts as safe for on-air use.

## Product direction

The first supported operating workflow is:

1. the operator explicitly arms an attended FT8 run;
2. SlotPilot calls CQ;
3. callers are queued and worked, initially in longest-waiting order;
4. each completed QSO is logged locally and affects duplicate policy;
5. SlotPilot resumes CQ when the queue is empty;
6. the run pauses after a configurable number of unanswered CQ cycles or when its attended-operation arm expires.

WSPR is a separate operating mode with receive, transmit scheduling, local spot storage, and WSPRnet upload. Future band hopping should be possible without being part of the first WSPR release.

Initial radio targets are:

- Elecraft K4;
- Yaesu FT-891 with DigiRig audio/CAT/PTT integration;
- Yaesu FTDX10.

## Core principles

- **Attended, not robotic.** Automation is bounded by an explicit, expiring operator arm.
- **One engine, multiple clients.** The daemon is the sole owner of hardware and live operating state.
- **Safety before convenience.** PTT has one owner, an independent watchdog, and fail-closed behavior.
- **Explainable policy.** Caller selection, duplicate suppression, lane selection, and transmit inhibition expose their reasons.
- **Local first.** The authoritative database and profiles remain on the operator's machine.
- **Cross-platform by design.** macOS is the primary development platform; Windows and Linux are first-class targets.
- **Protocol boundaries are replaceable.** FT8 and WSPR implementations stay behind SlotPilot-owned traits.
- **CLI parity.** Every operation available to the desktop client must be available through the command API and CLI.

## Start here

A contributor or coding agent should read these files in order:

1. [`AGENTS.md`](AGENTS.md)
2. [`docs/vision.md`](docs/vision.md)
3. [`docs/product.md`](docs/product.md)
4. [`docs/requirements.md`](docs/requirements.md)
5. [`docs/architecture.md`](docs/architecture.md)
6. [`docs/safety.md`](docs/safety.md)
7. [`docs/roadmap.md`](docs/roadmap.md)
8. [`docs/backlog/phase-0.md`](docs/backlog/phase-0.md)

The repository is intentionally free of live station behavior beyond the local
no-op API, persistence/test seams, and offline FT8 library harness. An assigned
issue should create only the narrow slice it owns and preserve the documented
boundaries.

Phase 0 is complete; its durable implementation record is
[tracking issue #1](https://github.com/rwjblue/slotpilot/issues/1). Phase 1's
bounded offline FT8 work is tracked by
[issue #16](https://github.com/rwjblue/slotpilot/issues/16). It establishes
owned message classifications, the exact reviewed protocol dependency,
redistributable fixtures, deterministic in-memory synthesis, bounded
RIFF/WAVE parsing, and reproducible offline decode. This is protocol evidence,
not a station feature. Phase 2 live receive remains unimplemented and requires
its own focused tracker and explicit handoff. WSPR remains planned after the
FT8 MVP.

## Validation

Use `mise run check` for the fast local formatting, Clippy, and test loop. Run
`mise run ci` before proposing or landing a change; it is the complete
repository gate used by GitHub Actions.

Run `mise run handshake` for the one-request no-op daemon/CLI exchange and
`mise run build-dev` for an unsigned, unpublished local artifact. See
[`docs/development-release.md`](docs/development-release.md). These workflows
cannot operate a radio.

## Documentation

See [`docs/README.md`](docs/README.md) for the full index, including the CLI contract, profile model, hardware targets, testing strategy, AntennaBench integration, and architecture decisions.

## License

SlotPilot is licensed under the GNU General Public License, version 3 or later. See [`LICENSE`](LICENSE).
