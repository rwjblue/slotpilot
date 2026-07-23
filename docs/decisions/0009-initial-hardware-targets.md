# 0009 — Initial hardware targets

- Status: Accepted
- Date: 2026-07-23

## Context

Broad nominal Hamlib support would spread validation effort across many radios before the safety and profile model is proven. The project owner has identified three initial station configurations.

## Decision

Initial validated targets are:

- Elecraft K4;
- Yaesu FT-891 using DigiRig for audio/control integration;
- Yaesu FTDX10.

Use a persistent Hamlib/`rigctld` adapter first, with model-specific quirks only behind the common capability and safety interfaces.

## Consequences

- Release claims distinguish “Hamlib may recognize” from “SlotPilot validated.”
- Profiles explicitly separate rig, PTT, input audio, and output audio.
- Additional radios require scoped capability and failure-path work.

## Revisit when

- the initial targets are reliable and a new model has an owner and test plan.
