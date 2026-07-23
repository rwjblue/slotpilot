# Security policy

SlotPilot will control transmit-capable radio equipment. Security reports that could lead to unintended transmission, persistent transmit authority, unauthorized local control, unsafe frequency or mode changes, disclosure of station credentials, or corruption of the operating log should be treated as safety-sensitive.

## Reporting

Use GitHub's private vulnerability reporting or security-advisory mechanism for this repository when available. Do not publish exploit details in a public issue before a maintainer has had an opportunity to assess them.

## Current status

The repository does not yet contain an executable implementation. Security policy and threat modeling are nevertheless part of the design because local IPC, external process integrations, rig control, and transmit scheduling will become privileged boundaries.

## Expected security properties

- Local control endpoints are not network-exposed by default.
- Every mutating client command is attributable and can be idempotently retried.
- Recovery does not restore operator transmit authority.
- External bundles or profiles cannot silently grant process-execution or transmit authority.
- Credentials and service tokens are never stored in portable station/profile exports unless explicitly designed and encrypted.
- Emergency stop bypasses ordinary admission and queueing paths.
