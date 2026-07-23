# Receive audio timeline

Phase 2 converts owned mono capture batches into the exact Phase 1 FT8 decoder
input without performing device I/O or decode in the timeline processor.

## Canonical output

One valid output is identified by a typed UTC slot starting on a 15,000 ms
boundary and contains exactly 180,000 mono signed-16-bit samples at 12,000 Hz.
The first output sample represents the slot boundary. The processor may need
the first source sample after a slot to interpolate its final output sample;
this bounded delay does not change the slot identity.

Capture beginning inside a slot creates a partial accumulator. That accumulator
is emitted only as an `Incomplete` diagnostic when the next slot begins and is
never padded or passed to the decoder. Capture beginning before a boundary may
discard the partial lead-in and still publish the following complete slot.

## Deterministic resampling

The implementation is SlotPilot-owned integer rational linear interpolation:

- UTC milliseconds map exactly to 12 canonical output ticks;
- source positions remain integers at the configured source rate;
- interpolation uses checked signed integer arithmetic and deterministic
  rounding;
- the state retains one previous source sample and at most one canonical
  output vector;
- source batches remain bounded by the capture contract.

This avoids platform floating-point and worker-order differences. It is a
receive-only first implementation suited to the radio-audio bandwidth. A
future filter-quality change requires a focused issue, replay evidence, and an
updated compatibility matrix; dependency types may not enter the public
contract.

## Timing evidence and tolerances

Scheduling and continuity use source positions plus monotonic evidence. UTC is
retained as the slot identity mapping and is checked against monotonic time;
the processor never sleeps on wall time.

The current bounded tolerances are:

- batch arrival lateness: at most 2,000 ms;
- source-position mapping jitter: at most 50 ms;
- UTC/monotonic offset change: at most 20 ms;
- estimated absolute source-rate error: at most 5,000 ppm after at least one
  second of evidence.

Health retains maximum observed jitter, latest rate-error estimate, incomplete
slot count, and late-batch count. These values are receive diagnostics only
and grant no transmit authority.

## Fail-closed reset behavior

The active window and resampler state are discarded when any of these occurs:

- a source-position gap, overlap, or out-of-order batch;
- an overflow, backend gap, clock-remap, or unexpected restart discontinuity;
- monotonic regression or excessive jitter/drift;
- UTC/monotonic remapping;
- late data;
- checked arithmetic failure.

A source gap advances the expected position past the rejected batch so a later
contiguous batch can establish a fresh anchor. Overlap and out-of-order input
cannot advance the timeline. Process generation, stream generation, and exact
configuration are immutable for one timeline; a change requires a newly
constructed timeline and cannot carry prior samples across the boundary.

No failure path invents samples. A discarded partial slot is counted, and the
typed error identifies why processing stopped or resynchronized.
