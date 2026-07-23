# FT8 fixture corpus v1

This directory is the complete, bounded Phase 1 FT8 reference corpus. The
checked-in `manifest.json` is the review surface for message vectors, recording
provenance, exact tool settings, units, tolerances, recall floors, and permitted
extra decodes.

## Reference and licensing

WSJT-X 3.0.0 is the authoritative reference for this corpus. The reviewed
release is the official `wsjtx-3.0.0-ARM-Darwin.dmg` published by the
[WSJT project](https://sourceforge.net/projects/wsjt/files/wsjtx-3.0.0/),
with SHA-256
`f9d95aad28e4da29b6f1d1fdf75647ecffe68162f3edc9e13d1984645bb21a37`.
The SourceForge project identifies WSJT-X as GPL software. SlotPilot and these
generated fixture outputs are distributed under GPL-3.0-or-later.

Both recordings were generated offline with the reference package. They are
not live recordings and contain no private station data. The overlapping case
uses FFmpeg 8.1 only to mix three generated mono PCM files. Version 1 records no
JTDX or aggressive-decoder observation; `supplemental_observations` is
deliberately empty.

## Review and refresh procedure

Refreshing expectations is an explicit review, never a snapshot update:

1. Select an official reference release and record its download URL, version,
   license, and SHA-256 before running it.
2. Run `ft8code`, `ft8sim`, and `jt9` with the exact settings recorded in the
   manifest, in empty writable data and temporary directories.
3. Inspect every canonical-text or packed-bit change against upstream release
   notes and the FT8 protocol specification. Do not accept changes merely
   because a newer tool emitted them.
4. For recordings, preserve the original checked bytes unless the purpose of a
   fixture changes. `ft8sim` includes generated noise, so a refreshed file need
   not reproduce an earlier checksum even with identical arguments.
5. Review redistribution rights, remove private calls or station data, update
   provenance and checksums, and run `mise run ci`.
6. Increment the fixture schema or directory version when a semantic field,
   authority, or comparison rule changes.

Ordinary CI reads only repository contents. It performs no download, runs no
WSJT-X/JTDX executable, and opens no audio or radio device.

`RR73` deserves special care: it is both the conventional ending token and a
syntactically valid four-character locator. The reviewed WSJT-X 3.0.0 command
line vector encodes the locator value. Consumers must inspect the packed field
rather than infer the owned message class from the rendered text alone.

## Compatibility limit

Passing this corpus proves only the enumerated message and recording cases
under the documented tolerances. It is not a general decoder-sensitivity,
performance, WSJT-X, or JTDX parity claim.
