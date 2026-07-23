# `rustfft` 6.4.1 review

- Review date: 2026-07-23
- Selected crate: `rustfft =0.6.4.1`
- Crates.io archive SHA-256:
  `21db5f9893e91f41798c88680037dba611ca6674703c1a18601b01a72c8adb89`
- Upstream tag commit:
  `4758ab0dd6f256c50ac8987c75c9cb96152dc2ca`
- Enabled optional features: none
- Default features: disabled

This record authorizes one private receive-only spectrum adapter in
`slotpilot-audio`. It does not authorize dependency types in SlotPilot public
contracts, unbounded FFT work, callback use, decode coupling, rendering,
persistence, audio output, rig control, PTT, scheduling, or RF.

## Selection and closure

Version 6.4.1 is already present in the exact Phase 1 lockfile as the reviewed
`mfsk-core` FFT backend. Phase 2 makes the same artifact a direct, exact
workspace dependency so the audio crate can reuse it without acquiring a
second implementation or version. The published archive checksum matches the
lockfile, and the upstream `6.4.1` tag resolves to the commit above.

RustFFT declares dual MIT/Apache-2.0 licensing, compatible with SlotPilot's
GPL-3.0-or-later distribution. Its normal closure is `num-complex`,
`num-integer`, `num-traits`, `primal-check`, `strength_reduce`, and
`transpose`; every selected version was already in the reviewed Phase 1
closure.

The 6.4.1 release fixes large Rader-algorithm twiddle calculations. The
adapter permits power-of-two lengths only, so it uses planned radix/SIMD paths
and does not depend on prime-length behavior. The package supports the
repository's exact Rust 1.97.1 toolchain.

An OSV crates.io query for `rustfft` 6.4.1 on 2026-07-23 returned no advisory.
The locked closure remains subject to the complete repository gate.

## Reviewed adapter behavior

`SpectrumModel` creates a forward `FftPlanner<f32>` plan once at worker-side
construction. It allocates the exact in-place scratch length once and rejects
any request beyond four times the maximum supported FFT length. Every push
reuses the plan, complex input buffer, and scratch buffer through
`process_with_scratch`; it never calls the allocating convenience
`Fft::process`.

The adapter fixes FFT lengths to powers of two from 256 through 4,096 and
validates every slice length before the dependency call. It applies a periodic
Hann window and converts only positive-frequency output into SlotPilot-owned
integer millihertz and millidecibel-full-scale values. Dependency complex,
planner, FFT, and numeric types never leave the private module.

RustFFT contains architecture-specific SIMD unsafe code. SlotPilot keeps
`unsafe_code = "forbid"` in workspace crates and calls only the safe planning
and in-place processing surfaces. Platform-specific acceleration may change
small floating-point rounding, so owned results quantize to integer
millidecibels and deterministic tone tests use documented numerical
tolerances.

## Boundary and upgrade procedure

All direct RustFFT imports live in `crates/audio/src/spectrum.rs`.
Dependency/public-rustdoc guards enforce that boundary. The adapter runs only
after canonical resampling, outside CPAL callbacks.

For an upgrade:

1. open a focused issue and repeat this review against the exact artifact;
2. inspect license, release notes, MSRV, normal closure, advisories, unsafe/SIMD
   changes, scratch contracts, and planner behavior;
3. update the exact root pin, tag commit, and archive checksum without enabling
   optional/default features;
4. rerun silence, known-tone, multi-tone, overlap, discontinuity, capacity,
   coalescing, memory, and CPU-bound tests;
5. run `mise run ci` on macOS, Windows, and Linux before landing.
