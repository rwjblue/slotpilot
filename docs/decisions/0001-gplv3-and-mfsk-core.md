# 0001 — GPLv3-or-later and `mfsk-core` boundary

- Status: Accepted
- Date: 2026-07-23

## Context

SlotPilot needs FT8 and WSPR encode/decode capability. A clean-room implementation would materially expand the initial DSP and protocol scope. The project owner accepts GPLv3 licensing. `mfsk-core` provides a Rust implementation path but is a young `0.x` dependency whose API should not define SlotPilot's public surface.

## Decision

- License SlotPilot under GPL-3.0-or-later.
- Use `mfsk-core` as the initial FT8/WSPR implementation, pinned to an explicitly reviewed version or commit when introduced.
- Place it behind SlotPilot-owned protocol traits, messages, decodes, errors, and waveform types.
- Validate behavior with reviewed fixtures and reference comparisons.

The initial implementation review selected exact crate version 0.7.4 with
defaults disabled and only `ft8` plus the host `fft-rustfft` backend enabled.
The maintained review and upgrade procedure is
[`../dependencies/mfsk-core-0.7.4.md`](../dependencies/mfsk-core-0.7.4.md).

## Consequences

- Distributed SlotPilot binaries and derivative combined works must comply with GPL terms.
- AntennaBench integration remains process-separated rather than linking SlotPilot crates into its Apache-2.0 codebase.
- Replacing or upgrading the protocol implementation should not require client, storage, or operations API changes.

## Revisit when

- the dependency no longer meets correctness, performance, maintenance, or platform requirements;
- a better compatible implementation becomes available;
- project licensing goals materially change.
