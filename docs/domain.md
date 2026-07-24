# Domain vocabulary

The `slotpilot-domain` crate owns infrastructure-independent values shared by
commands, events, persistence boundaries, operations, and tests.

## Identities

Request, command, event, session, service-instance, receive-window,
profile-revision, QSO, QSO-attempt, and transmission identities are opaque
strings with a type-specific prefix and a bounded lowercase ASCII payload.
Their display and JSON forms are the same string. Callers must not infer
ordering or meaning from the payload.

## Callsigns and roles

`FullCallsign` preserves the exact accepted spelling used at the boundary.
`BaseCallsign` is a separate uppercase policy key derived from the longest
slash-separated component containing both a letter and a digit. Normalization
never replaces the full value.

Station, operator, and owner callsigns use distinct wrapper types. Code must
select the correct role explicitly rather than passing an unlabelled callsign.

## Radio values

Dial and audio frequencies use integer hertz. Power uses integer milliwatts.
The constructors enforce documented bounds before values can reach future
hardware or scheduling adapters. Bands and modes use closed symbolic enums with
stable wire names.

`UtcSlot` stores a non-negative UTC millisecond timestamp aligned to the
selected mode: 15 seconds for FT8 and 120 seconds for WSPR. It describes a
boundary only; it grants no scheduling or transmit authority.

Radio-side modulation is a separate closed type from synchronized
`OperatingMode`; FT8 therefore cannot be passed where a rig modulation is
required. Passband width uses checked integer hertz. Exact VFO and split
readback preserve an absent transmit VFO instead of inventing one.

The Phase 3 read-only rig profile retains an immutable profile revision,
nonzero Hamlib model, bounded exact Hamlib version expectation, required radio
modulation/passband, and two structurally distinct endpoint types:

- the exact operator-configured downstream radio CAT `host:port`;
- the separate exact `rigctld` service `host:port`.

Neither endpoint type performs discovery, name resolution, implicit-port
selection, or fallback. Managed `rigctld` profiles require a loopback IP
literal for the service endpoint and expose no configurable PTT method; the
later lifecycle adapter must force PTT type `NONE`. The K4 model identifier
2047 is a profile value, not a public radio-specific type or default.

The reviewed display and JSON examples live beside the types as unit-test
fixtures. Domain parsing returns typed `thiserror` errors and has no GUI,
database, protocol-library, audio, or rig dependency.
