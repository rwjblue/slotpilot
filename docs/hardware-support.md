# Initial hardware support

Initial hardware support is deliberately narrow so safety and verification behavior can be tested deeply.

## Validation sequencing

The K4, FT-891/DigiRig, and FTDX10 remain the initial validation targets. The
Phase 3 tracker designates one primary radio/audio configuration according to
physical availability, repeatability, and safe test equipment. That
configuration alone gates the Phase 5 contact-capable alpha and Phase 6 FT8
MVP.

The remaining initial targets advance through the same test ladder in Phase 7
and must be complete before the packaged product release. A primary designation
is a sequencing choice, not a claim that the other targets are unsupported or
architecturally deferred.

## Common model

Every rig profile must distinguish:

- CAT/control endpoint;
- PTT method;
- input audio device;
- output audio device;
- radio mode/passband mapping;
- supported state readback;
- split/Fake-It behavior;
- maximum configured digital power;
- PTT lead/tail and output latency;
- verified quirks.

Hamlib capability claims are necessary but not sufficient. Profile setup records which operations were actually verified against the connected radio.

## Elecraft K4

Designated Phase 3/5/6 primary path:

- Hamlib model 2047 K4 backend through persistent `rigctld`;
- K4 CAT over Ethernet at an explicit operator-configured downstream
  `host:port`, with no discovery, implicit port, default, or fallback;
- a separate explicit `rigctld` service `host:port`;
- managed service binding only to a loopback IP literal with PTT type forced
  to `NONE`;
- external service mode remaining read-only at the SlotPilot boundary;
- K4 built-in USB sound-card receive input selected independently by stable
  Phase 2 audio identity;
- capability probing for frequency, radio modulation/passband, VFO, split,
  optional power, and optional PTT evidence.

Ethernet audio streaming, rig mutation, raw CAT, PTT control, and audio output
are not part of Phase 3. A missing K4 endpoint or CAT port is a profile error,
not permission to guess from WSJT-X, Elecraft remote-stream conventions, DNS,
another endpoint, or a hardcoded value.

A direct K4 command adapter is permitted only for a documented Hamlib gap and must remain behind the same rig and safety interfaces.

## Yaesu FT-891 with DigiRig

The profile treats control and audio as separate resources:

- CAT endpoint through the configured serial interface;
- DigiRig input and output audio devices;
- selectable CAT, RTS, or DTR PTT according to tested wiring/profile;
- verified data-mode and passband settings;
- conservative digital-power limits.

No implementation should assume that USB CAT implies a radio-provided USB sound card.

## Yaesu FTDX10

The profile expects:

- CAT through the configured enhanced control port or Hamlib endpoint;
- PTT through CAT by default, while preserving explicit alternate-port configuration;
- built-in USB audio selected independently;
- DATA-U mode and verified passband;
- capability-tested split/Fake-It behavior.

No implementation should identify the pair of serial functions only by changing operating-system port numbers.

## Test ladder

A radio adapter advances through these levels:

1. Hamlib dummy and deterministic transcript tests.
2. Parser/command tests against captured non-sensitive exchanges.
3. Read-only physical connection.
4. State mutation without PTT, with operator confirmation.
5. PTT and audio loopback or dummy-load validation under hard timeout.
6. Bounded on-air validation after all failure tests pass.

Ordinary CI stops before physical connection.

## Additional radios

Additional models require:

- an issue defining minimum supported operations;
- capability and quirk documentation;
- read-only and failure-path tests;
- profile validation behavior;
- no regression to existing safety invariants.
