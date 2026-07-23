# SlotPilot

**Attended weak-signal operation for FT8 and WSPR.**

SlotPilot is planned as a local-first, cross-platform station service that handles the repetitive parts of FT8 and WSPR operation while leaving the licensed operator present, informed, and in control.

The intended system has one hardware-owning daemon and two first-party clients:

- `slotpilotd`: owns rig control, audio streams, protocol timing, operating state, and transmit safety;
- `slotpilot`: a machine-friendly and human-friendly command-line client;
- SlotPilot Desktop: a native desktop client for macOS, Windows, and Linux.

All clients use the same versioned command/event API. No capability should exist only in the GUI.

> [!IMPORTANT]
> This repository currently contains the design and project scaffold only. There is no working radio, audio, FT8, WSPR, logging, daemon, CLI, or desktop implementation. Do not treat the repository as safe for on-air use.

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

The repository is intentionally implementation-free. The first assigned issue should create only the narrow slice it owns and should preserve the documented boundaries.

Phase 0 implementation is coordinated through
[tracking issue #1](https://github.com/rwjblue/slotpilot/issues/1). GitHub issues
are the durable execution contracts; roadmap and backlog documents describe
sequencing and stable acceptance themes.

## Documentation

See [`docs/README.md`](docs/README.md) for the full index, including the CLI contract, profile model, hardware targets, testing strategy, AntennaBench integration, and architecture decisions.

## License

SlotPilot is licensed under the GNU General Public License, version 3 or later. See [`LICENSE`](LICENSE).
