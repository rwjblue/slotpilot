# Profile model

Profiles are reusable, versioned configuration. An operating session composes exact profile revisions and snapshots the resolved result.

## Operator profile

Describes the person at the controls:

- operator callsign;
- name or label;
- optional default location;
- preferences that belong to the person rather than the station.

## Station profile

Describes the station presented on the air:

- station callsign;
- owner or host callsign when different;
- home grid and optional precise location;
- station description;
- antenna description;
- default power limits;
- regulatory region and band-plan source;
- ADIF defaults.

## Activation profile

Describes temporary operating context:

- portable grid/location;
- POTA reference;
- SOTA reference;
- WWFF reference;
- club or special-event context;
- temporary antenna details;
- notes and validity window.

An activation profile overrides appropriate station-location values without replacing the station identity.

## Rig profile

Describes control and verification:

- immutable profile revision identity;
- nonzero Hamlib model and bounded exact version expectations;
- managed or external `rigctld` mode;
- an exact downstream radio CAT network `host:port`;
- a structurally distinct exact `rigctld` service `host:port`;
- baud and transport settings;
- PTT method: CAT, RTS, DTR, or a later verified method;
- required digital mode and passband;
- split/Fake-It capabilities;
- PTT lead and tail;
- power limits;
- expected meter and state-readback capabilities;
- radio-specific quirks.

Phase 3 designates the Elecraft K4 Ethernet CAT path through Hamlib model 2047.
The K4 host/IP and TCP port are both explicit operator input. SlotPilot does
not discover, infer, default, hardcode, or fall back to either value. In
managed mode the separate service endpoint must be a loopback IP literal and
the later lifecycle adapter must force `--ptt-type=NONE`; no PTT method is
configurable in the read-only contract. External mode also requires an exact
service endpoint and remains read-only at the SlotPilot boundary.

The K4 built-in USB sound-card input remains an independently selected stable
identity in the audio profile. It is not derived from either rig endpoint, and
a display name cannot select it. Ethernet audio streaming is outside the
current architecture and Phase 3 scope.

## Audio profile

Describes independent input and output paths:

- stable platform input-device ID;
- stable platform output-device ID;
- human-readable names retained for diagnostics;
- selected channels;
- native sample formats;
- receive gain and expected levels;
- transmit level;
- input/output latency calibration;
- clipping and health thresholds.

Input and output may be different devices. The absence of a configured device is an error, not permission to use the system default.

## Operating profile

Describes policy:

- FT8 run preset;
- QSO retry and unanswered-CQ limits;
- caller selector;
- duplicate and manual rules;
- parity and lane behavior;
- attended-arm duration and bounds;
- WSPR receive/upload/transmit policy;
- log sinks;
- display/notification preferences that affect clients consistently.

## Session composition

Conceptually:

```rust
pub struct StationContextSelection {
    pub operator_profile: ProfileRevisionId,
    pub station_profile: ProfileRevisionId,
    pub activation_profile: Option<ProfileRevisionId>,
    pub rig_profile: ProfileRevisionId,
    pub audio_profile: ProfileRevisionId,
    pub operating_profile: ProfileRevisionId,
}
```

Starting a session resolves these revisions into an immutable context snapshot. Editing a reusable profile later creates a new revision; it does not alter the active session or prior logs.

## Callsign semantics

Keep these independent:

- `station_callsign`: the call transmitted over the air;
- `operator`: the licensed person operating;
- `owner_callsign`: the station owner or host where applicable;
- `base_callsign`: normalized value used only where a policy explicitly requests it.

Never overwrite the full transmitted call with a normalized base call.

Special calls such as `W1AW/1` require protocol message planning before an automatic run can be armed. The profile validator should show the exact sequence SlotPilot expects to transmit and reject an unrepresentable or ambiguous combination.

## Import and export

Portable profile export must not silently include:

- service credentials;
- executable commands;
- active transmit authority;
- OS-private device handles;
- machine-local secrets.

Imports create reviewed local revisions. They do not automatically attach themselves to a live session or arm transmission.
