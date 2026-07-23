# `cpal` 0.18.1 review

- Review date: 2026-07-23
- Selected crate: `cpal =0.18.1`
- Crates.io archive SHA-256:
  `5f77b11176c37874be37e8d691c946e31b2b8c357abce9526f6a99eb469e1028`
- Upstream tag commit:
  `94ecb6ec64546308885a59b38e29f938796e8100`
- Enabled optional features: none
- Default features: disabled

This is the maintained Phase 2 adoption record for receive-only device
discovery and input capture. It authorizes only private adapters in
`slotpilot-audio`. It does not authorize an output stream, default-device
selection, automatic fallback, persistence, a wire contract, rig control,
PTT, transmit scheduling, or RF.

## Selection

Version 0.18.1 was the current published release when Phase 2.2 began. The
workspace uses an exact Cargo requirement and the lockfile records the
published checksum. The release tag resolves to the commit above. The crate
requires Rust 1.85; SlotPilot's exact Rust 1.97.1 toolchain satisfies it.

CPAL 0.18 introduced a host-qualified `DeviceId` whose `Display`/`FromStr`
representation is documented as stable across application restarts and
supports exact host lookup. That is the first reviewed CPAL surface that
directly satisfies SlotPilot's stable-identity rule without private platform
FFI or display-name fallback. Version 0.18 also supplies structured device
metadata and typed error kinds for host unavailability, permission denial,
device disappearance, and unsupported configurations.

No optional feature is enabled. In particular, the review does not select
ASIO, JACK, PipeWire, PulseAudio, custom backends, browser audio, or real-time
thread promotion. Native target dependencies still select the standard host:
Core Audio on macOS, WASAPI on Windows, and ALSA on Linux/BSD.

## License, platform closure, and build requirements

The crate declares Apache-2.0. SlotPilot remains distributed under
GPL-3.0-or-later, which is compatible with linking Apache-2.0 code under the
project's accepted license.

The target-specific closure includes:

- Core Audio bindings and Objective-C support crates on Apple targets;
- `windows`/`windows-core` bindings on Windows;
- `alsa`, `libc`, and their system ALSA library requirement on Linux/BSD;
- `dasp_sample` on all targets.

The Linux CI job installs `libasound2-dev` before compiling the workspace.
Linux development hosts need the equivalent ALSA development package.

Linux builds require the ALSA development package documented upstream. The
repository's GitHub Actions matrix must compile and test the exact lockfile on
macOS, Windows, and Ubuntu before any issue using this pin lands. Optional
backends require a separately reviewed issue and cannot silently change a
persisted identity's host.

## Reviewed discovery and capture behavior

The private adapter uses only safe enumeration and metadata methods:

- enumerate every device from the system's standard host;
- inspect input configuration ranges to reject output-only devices;
- obtain the stable host-qualified ID from `DeviceTrait::id`;
- map structured name/manufacturer fields to non-identifying display metadata;
- map PCM rate/channel/sample-format ranges into checked SlotPilot values;
- look up only an exact selected identity with `device_by_id`.

Discovery never calls `default_input_device`, `default_output_device`, or a
stream builder. Duplicate display names remain distinct because sorting,
equality, lookup, and selection use only stable identities. DSD-only or
out-of-bounds configurations fail as unsupported rather than being coerced
into PCM.

The private capture adapter resolves the same exact identity, verifies the
exact configuration against the device's input ranges, and calls only the
typed input-stream builder. It uses the dependency's input callback timestamps
to retain capture/callback delay evidence. It never opens an output stream,
consults a default device/configuration, or performs fallback.

CPAL documents that a stable ID is provided across supported backends "where
possible." If a backend cannot return one, SlotPilot reports the owned
`IdentityUnavailable` result and does not substitute a name. Host absence,
permission denial, device disappearance, unsupported configuration, no input
device, and unclassified backend failure remain separate owned errors.

## Advisories and unsafe exposure

An OSV/RustSec ecosystem query for `cpal` 0.18.1 on 2026-07-23 returned no
advisory. The complete locked dependency closure is still rebuilt and tested
by the repository gate.

CPAL and platform binding crates contain internal unsafe code needed to call
native audio APIs. SlotPilot's workspace keeps `unsafe_code = "forbid"` and
uses only CPAL's safe discovery and input-stream surfaces. All dependency types
remain in private adapters; public signatures expose only SlotPilot-owned
identity, metadata, configuration, batch, health, and error values.

## Boundary and upgrade procedure

All CPAL imports live in `crates/audio/src/discovery.rs` and
`crates/audio/src/capture.rs`. Discovery may enumerate platform devices and
therefore can encounter permission behavior. Capture may open only the exact
input identity/configuration selected through discovery. No other crate
imports CPAL, and dependency/public-rustdoc guards enforce the boundary.

For an upgrade:

1. open a focused issue and repeat this review against the new exact artifact;
2. inspect stable-ID serialization/lookup guarantees and platform behavior;
3. inspect license, release changes, Rust compatibility, features,
   dependencies, advisories, and unsafe exposure;
4. update the exact root pin, tag commit, and archive checksum without enabling
   optional features;
5. run owned mapping, duplicate-name, error, exact-lookup, conversion, queue,
   overflow, lifecycle, and callback-boundary tests;
6. run `mise run ci` on the complete cross-platform matrix and update this
   record.
