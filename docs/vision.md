# Vision

## Problem

FT8 and WSPR are highly structured, time-synchronized modes. Existing applications provide extensive capability, but an attended operator may still spend substantial effort on routine sequencing: selecting callers, repeating exchange stages, deciding when to resume CQ, updating worked-before state, logging, choosing a quiet transmit lane, managing profiles, and coordinating radio/audio state.

The operator should remain responsible for the station without having to manually click after every successful QSO.

## Vision statement

SlotPilot will be a trustworthy, local-first operating assistant for synchronized weak-signal modes. It will conduct bounded, explainable workflows under an explicit operator arm, expose the same capabilities through desktop and command-line clients, and fail safely when time, audio, rig, policy, or state becomes uncertain.

## Primary users

- an attended home-station operator running ordinary FT8 exchanges;
- a portable operator using different callsigns, grids, activations, rigs, and audio interfaces;
- a club or special-event operator whose station callsign differs from the person at the controls;
- a WSPR experimenter who needs reproducible transmit scheduling and durable receive/upload records;
- an external local application, such as AntennaBench, that needs a stable machine interface rather than GUI automation.

## Product principles

### Operator authority is explicit

Automation begins only after an affirmative action and ends when its configured scope, time, cycle count, or safety conditions end. Recovery never silently re-arms the transmitter.

### Routine work can be automated without hiding decisions

The application may select the oldest eligible caller, recommend a parity, choose an apparently quiet lane, or ignore a duplicate. It must show why.

### The operating engine is independent of presentation

Desktop, CLI, and external clients interact with the same service boundary. This makes behavior testable, scriptable, and consistent.

### Profiles describe real operating context

The application must correctly distinguish station callsign, operator callsign, owner/host callsign, portable location, grid, activation references, radio, audio interface, and operating policy.

### Safety behavior is part of the product

PTT ownership, transmit timing, audio health, clock health, unexpected rig changes, crash recovery, and emergency stop are core requirements rather than later hardening work.

### Local data remains useful without a hosted account

The operator can run, log, inspect, export, and recover locally. External reporting services are optional integrations.

## Measures of success

SlotPilot is successful when an attended operator can:

- start a bounded CQ run and work a queue without clicking after every completed contact;
- understand every skipped or selected caller;
- stop transmission immediately and trust that faults dekey the radio;
- use separate station and operator identities without corrupting ADIF;
- operate the same engine through a desktop interface, CLI, or local integration;
- receive and transmit WSPR while preserving local evidence even when WSPRnet is unavailable;
- move between supported radios and locations by selecting composed profiles rather than re-entering settings.

## Non-goals for the initial product

- full WSJT-X feature parity;
- unattended robotic QSO operation;
- Fox/Hound or DXpedition multi-stream operation;
- contest exchange support;
- arbitrary free-text automation;
- a hosted station-control service;
- remote Internet exposure of the local control API;
- native binary plugin loading;
- automatic WSPR band hopping in the first WSPR release;
- support for every Hamlib radio before the initial three targets are reliable.
