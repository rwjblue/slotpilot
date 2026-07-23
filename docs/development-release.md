# Development build and release boundary

SlotPilot is at `0.1.0-dev.0`. Development versions use `0.y.z-dev.N`; increment
`N` for another repository-development checkpoint and update `CHANGELOG.md` in
the same change. A stable version requires a separately authorized release
issue and is outside Phase 0.

Install the pinned Rust toolchain with:

```sh
mise install
```

The canonical workflows are:

```sh
mise run handshake
mise run check
mise run ci
mise run build-dev
```

`handshake` starts the daemon in one-request Phase 0 mode, asks the CLI for its
no-op snapshot, and exits. The expected status is `not_configured`,
`not_running`, and unavailable transmit authority.

`check` is the fast formatter, Clippy, and workspace-test loop. `ci` is the
complete landing gate and explicitly rechecks reviewed API wire fixtures and
SQLite schema compatibility. `build-dev` creates `target/slotpilot-dev/`
containing the two debug binaries and `DEVELOPMENT-ONLY.txt`.

The local artifact is unsigned, unnotarized, unpublished, and unsuitable for
distribution or on-air use. It has no radio, audio, FT8, WSPR, logging,
station-control, transmit, or desktop capability. No credential, physical
radio, audio interface, antenna, network service, signing identity, or operator
transmission is used by these workflows.
