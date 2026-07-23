# `mfsk-core` 0.7.4 review

- Review date: 2026-07-23
- Selected crate: `mfsk-core =0.7.4`
- Crates.io archive SHA-256:
  `4db9209181f9b9ac5fc401611da3af9269bf9c6621c3dcc51484c46594bd1d2a`
- Upstream tag commit:
  `c0bfaaa780c630f3843ea11fecfa3f0cf2dcf56e`
- Enabled features: `ft8`, `fft-rustfft`
- Default features: disabled

This is the maintained adoption record required by ADR 0001. It authorizes
only the private offline FT8 adapter in `slotpilot-protocol`; it does not make
upstream application examples, QSO state, station control, device access, or
other protocols part of SlotPilot.

## Selection

Version 0.7.4 was the current published release when Phase 1.3 began. The
workspace uses an exact Cargo version requirement and the lockfile records the
published archive checksum. The release tag resolves to the commit above.
Pinning the published release gives a stable, checksummed source artifact while
avoiding unreviewed commits added to upstream `main` after the tag.

The `ft8` feature selects the protocol implementation. `fft-rustfft` supplies
the standard host FFT backend needed by later Phase 1 offline synthesis and
decode adapters; it transitively enables `std` and `alloc`. Defaults are
disabled, so `ft4`, `parallel`/Rayon, WSPR, FST4, JT9, JT65, Q65, MSK144,
packet-byte, embedded fixed-point, and broad `full` feature surfaces are not
selected.

## License and lineage

The crate declares `GPL-3.0-or-later`, matching SlotPilot. Upstream describes it
as a faithful port of the GPL-3.0-or-later WSJT-X reference implementation and
identifies `lib/77bit/packjt77.f90` as the message-codec lineage. The selected
transitive crates declare MIT, Apache-2.0, `MIT OR Apache-2.0`, or compatible
combinations:

- `crc` 3.4.0 and `crc-catalog` 2.5.0;
- `num-complex` 0.4.6, `num-integer` 0.1.46, and `num-traits` 0.2.19;
- `rustfft` 6.4.1, `primal-check` 0.3.4, `strength_reduce` 0.2.4, and
  `transpose` 0.2.3;
- `libm` 0.2.16 and build-only `autocfg` 1.5.1.

This review does not authorize linking SlotPilot into incompatibly licensed
software. Distribution remains subject to the repository GPL license and the
notices of the dependency closure.

## Changelog and correctness review

The 0.7.3 release added FT8 CCIR-fading recall work and follow-up classification
for the reviewed noisy sample. Version 0.7.4 changes MSK144 behavior and
documentation, not the FT8 message surface. Upstream issue 150 explicitly
distinguishes authoritative WSJT-X recall from uncorroborated JTDX-only extra
decodes; it is closed and supports SlotPilot's decision not to claim aggressive
decoder parity. No open upstream issue reviewed on 2026-07-23 defeated the
bounded Phase 1 message contract.

The owned fixture matrix exposed three adapter-level differences that are
handled and tested locally:

- numeric roger reports unpack as `R-8` upstream and are normalized to the
  reviewed WSJT-X spelling `R-08`;
- the first 12 bits of a Type 4 `CQ W1AW/1` are ignored during unpacking, but
  SlotPilot fills them with the reviewed WSJT-X callsign hash for deterministic
  bit parity;
- `RR73` is also a valid four-character Maidenhead locator. WSJT-X 3.0.0's
  reviewed command-line vector encodes it as a grid while the Rust packer
  chooses the reserved response code. SlotPilot classifies the packed field,
  not the text token, so the grid and ending remain distinct owned classes.

The public pack API cannot encode the reviewed 22-bit-hashed compound call plus
numeric report. SlotPilot decodes that vector with and without supplied hash
knowledge, but returns a typed `NotRepresentable` error if asked to encode it.
It never silently substitutes a base call or an unresolved identity.

## Toolchain, dependencies, advisories, and unsafe exposure

The exact crate and selected features compile and test with the repository pin,
Rust/Cargo 1.97.1 on `aarch64-apple-darwin`. `cargo tree -e features` records
only `ft8` and `fft-rustfft` as direct `mfsk-core` feature selections and does
not include Rayon.

On 2026-07-23, a GitHub Advisory Database query for the selected Rust package
closure found no advisory affecting the locked versions. The two records
returned for `transpose` affect versions before 0.2.3; one is withdrawn as a
duplicate and 0.2.3 is the first patched version. The complete repository gate
also performs the locked build and test.

The dependency closure is not claimed to be `unsafe`-free. The selected
`mfsk-core` host path contains a reviewed boxed-slice-to-array conversion in
its mixed FFT implementation. Its embedded extern and packet-byte unsafe paths
are feature-gated off. `rustfft` contains architecture-specific SIMD unsafe
code, and supporting numeric crates contain their own reviewed unsafe
implementations. SlotPilot forbids unsafe code in its own workspace crates,
keeps the dependency behind one private adapter, and validates behavior at the
owned boundary.

## Boundary and upgrade procedure

All dependency imports live in `crates/protocol/src/offline_adapter.rs`.
Public methods use only SlotPilot-owned messages, packed bits, hash context
inputs, outcomes, and typed errors. Fixtures contain neutral facts rather than
dependency serialization. No API, storage, operations, testkit, CLI, daemon,
or documentation example exposes dependency types.

For an upgrade:

1. open a focused issue and repeat this review against the new exact artifact;
2. inspect license/lineage, release changes, open correctness issues, Rust
   compatibility, dependencies, advisories, selected features, and unsafe code;
3. update the exact root pin and archive checksum without enabling defaults;
4. run the owned message, waveform, recording, and conformance matrix;
5. review any fixture change against the authoritative reference rather than
   accepting dependency output as golden;
6. run `mise run ci` and update this maintained record.
