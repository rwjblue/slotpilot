# Receive clock health

Phase 2 assigns UTC slot meaning to captured audio only while an independent
UTC/monotonic mapping is fresh and internally consistent. This boundary is
receive-only and creates no scheduler side effect or transmit authority.

## Production sampling

`SystemClock` is constructed with an explicit non-zero process generation and
a new process-local monotonic origin. Each sample reads monotonic elapsed time,
then UTC, then monotonic elapsed time again. The midpoint of the bracketing
monotonic observations is paired with UTC. Both use integer milliseconds.

A restart constructs a new generation and origin. A monitor never accepts a
sample from another generation, so neither monotonic state nor recovered
health can cross a process restart.

The monitor publishes the next desired sample instant on the monotonic
timeline. It does not sleep or run a wall-clock scheduling loop; the daemon
composition layer will arrange bounded polling later.

## Default policy

- desired sample cadence: 1,000 ms;
- maximum mapping age: 2,500 ms;
- maximum sample-to-sample monotonic gap: 5,000 ms;
- maximum UTC-minus-monotonic divergence: 100 ms;
- maximum sample-to-observer delay: 250 ms;
- recovery: three consecutive consistent same-generation samples.

All values are checked and bounded when a monitor is created.

## Latch and recovery

The monitor latches unhealthy for:

- forward or backward UTC jumps relative to monotonic progress;
- UTC or monotonic regression;
- stale mapping age;
- delayed observation of a sample;
- a suspend/resume-like monotonic gap;
- another process generation;
- arithmetic overflow;
- disagreement between a complete window's capture mapping and the independent
  clock mapping.

While latched, a complete timeline window becomes `WindowRejected` with its
typed slot and fault. It cannot become decoder-ready. Incomplete timeline
evidence remains incomplete.

Except for a process-generation mismatch, a recovery candidate begins with the
next timely sample. Three consecutive candidates must agree in UTC/monotonic
progress and stay within the same bounds. Progress is visible in snapshots,
and completion emits a distinct `Recovered` transition. Any new fault resets
the sequence. A generation mismatch requires a new monitor.

## Alignment evidence

For a complete typed FT8 slot, the monitor maps the UTC boundary to a
monotonic instant using the latest accepted paired sample. It compares that
instant with the capture mapping carried by the canonical window. Difference
beyond the jump tolerance latches `WindowMisaligned`; otherwise the ready
window carries the slot-start monotonic instant and mapping age.

Snapshots and transitions are internal owned values reserved for later API
mapping. This slice adds no wire or durable schema, decoder invocation, audio
output, rig, PTT, transmission, or RF behavior.
