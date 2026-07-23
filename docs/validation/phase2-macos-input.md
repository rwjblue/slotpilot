# Phase 2 macOS RF-free input validation

This is the operator protocol for human-required issue #36. It validates only
receive input. It must not use a radio, rig-control connection, PTT, audio
output intended for transmission, transmitter, antenna, or RF source.

## Entry gate and tested revision

Do not start until issues #26 through #35 are closed, their commits are
reachable from remote `main`, and the complete GitHub Actions matrix for the
top commit is green. From a clean checkout, record the immutable tested
revision:

```sh
jj git fetch
git rev-parse refs/remotes/origin/main
jj log -r 'main@origin' --no-graph -T 'commit_id ++ "\n"'
gh run list --branch main --commit "$(git rev-parse refs/remotes/origin/main)" \
  --workflow ci.yml --limit 1
```

The two revision commands must print the same full commit ID. Copy that ID,
the successful run URL, build commands, and the UTC start time into the #36
evidence before opening any input device. If the revision is not exact or CI
is not green, stop.

Build and repeat the repository gate at that revision:

```sh
mise install
mise run ci
cargo build --locked --package slotpilotd --package slotpilot
```

Use only the binaries produced from this checkout. Do not modify source,
fixtures, configuration, or the executable during validation.

## Environment record

Record only sanitized information:

- macOS version and architecture;
- input or loopback interface make/model and driver version;
- microphone/input permission state before and after the test;
- a salted hash of the stable device identity plus enough display metadata to
  distinguish duplicate display names;
- supported and selected sample rate, channel, and format;
- exact SlotPilot revision, build commands, GitHub Actions run, and test times;
- the RF-free audio source and its checksum or public fixture identity.

Do not record usernames, home-directory paths, hardware serial numbers, raw
stable identifiers, private microphone audio, or non-public callsigns/grids.

## Fixed software bounds

Use the production constants from the tested revision. Record them before the
run:

- native callback queue: the explicitly configured fixed batch capacity;
- daemon worker backlog: 4 batches with one worker;
- receive history page: at most 100 rows;
- receive decodes per public record: at most 128;
- waterfall row: at most 2,048 bins;
- event replay request: at most 256 events;
- receive-clock sample cadence: 1,000 ms;
- receive-clock freshness: 2,500 ms;
- receive-clock gap: 5,000 ms;
- mapping tolerance: 100 ms;
- sampling-delay tolerance: 250 ms;
- healthy recovery: 3 consecutive samples.

Run healthy capture for 20 consecutive FT8 slots, at least five minutes. Stop
early if any stop-test criterion occurs.

## Protocol

1. Start `slotpilotd` with a fresh temporary SQLite database and a user-scoped
   local endpoint. Save sanitized daemon diagnostics only.
2. Enumerate inputs through `slotpilot`. Record the intended stable identity,
   supported configurations, and display metadata. If two devices share a
   display name, prove the selection remains tied to the intended stable
   identity; otherwise record duplicate-name coverage as not available.
3. Start receive explicitly through the public CLI with the exact stable
   identity and configuration. Confirm status reports the same selection and a
   fresh stream generation.
4. Feed a checked-in Phase 1 FT8 WAV fixture or a separately checksummed,
   documented FT8 fixture through an RF-free loopback or isolated input. Never
   route SlotPilot output to the input and never connect a radio.
5. For 20 consecutive slots, record aggregate capture health, clock health,
   window/slot identities, decode classifications, diagnostic summaries,
   history rows, ordered event sequences, JSONL envelopes, and bounded
   waterfall metadata. Retain no raw ambient audio or waterfall rows.
6. Disconnect and reconnect the CLI while the daemon continues. Confirm daemon
   ownership and stream generation remain stable, event replay is ordered and
   bounded, and no decode/event is duplicated.
7. Disable or remove only the selected input, or revoke its input permission
   if the environment supports doing so safely. Confirm a typed visible
   inhibition/stop, no selection of another device, no stale decode after
   loss, no uncommitted receive event, and no automatic restart.
8. Restore the input or permission. Confirm receive remains inactive. Enumerate
   again and verify the intended stable identity reappears without display-name
   substitution.
9. Start receive explicitly and repeat at least two known-audio FT8 slots.
   Confirm a fresh stream generation and bounded recovery.
10. Stop receive explicitly, terminate the daemon, and confirm the input
    resource and local endpoint are released. Preserve the temporary database
    only if its sanitized aggregate evidence is needed; otherwise delete it.

Use the landed CLI grammar below, substituting only values copied from device
discovery. Keep each mutating request ID stable across retries:

```sh
RUNTIME=/private/path/to/owner-only-runtime
slotpilot devices audio list "$RUNTIME" --json --non-interactive
slotpilot receive start "$RUNTIME" macos_core_audio "$OPAQUE_ID" \
  "$SAMPLE_RATE" "$CHANNELS" "$SAMPLE_FORMAT" "$SELECTED_CHANNEL" \
  --request-id req_phase2start --json --non-interactive
slotpilot receive status "$RUNTIME" --json --non-interactive
slotpilot receive history "$RUNTIME" 0 100 --json --non-interactive
slotpilot events follow "$RUNTIME" 0 256 --jsonl --non-interactive
slotpilot receive stop "$RUNTIME" \
  --request-id req_phase2stop --json --non-interactive
```

The issue evidence must also record the exact daemon launch command supplied
by the landed Phase 2 build. Do not infer or substitute an undocumented
development mode.

The issue evidence must distinguish observed pass/fail results from commands
and expectations. A missing observation is not a pass.

## Stop-test criteria

Stop immediately for:

- any output-device, rig, PTT, transmit, antenna, or RF access;
- capture of private ambient audio;
- silent fallback to another input;
- stale decode or event publication after loss;
- automatic receive restart;
- unbounded queue, history, event, waterfall, memory, CPU, or disk growth;
- crash, database corruption, mismatched revision, or unexpected permission
  behavior.

Open a focused issue for every material finding. Do not patch or continue from
a modified checkout and do not close #36 until blocking findings are fixed and
the exact landed replacement revision is retested.

## Evidence checklist

- [ ] Exact remote `main` commit and green cross-platform Actions run recorded.
- [ ] Sanitized macOS, interface, driver, permission, and format recorded.
- [ ] Stable exact selection and duplicate-name behavior observed or bounded as
      unavailable.
- [ ] Explicit start and exact status observed.
- [ ] Twenty consecutive FT8 slots completed within fixed health/resource
      bounds.
- [ ] Known RF-free FT8 audio decoded, persisted, replayed, rendered as JSONL,
      and represented by bounded waterfall metadata.
- [ ] Client reconnect preserved daemon ownership and ordered replay.
- [ ] Device/permission loss inhibited visibly with no fallback, stale work,
      uncommitted event, or automatic restart.
- [ ] Explicit restart after restoration used a fresh stream generation.
- [ ] Explicit stop and daemon shutdown released resources.
- [ ] Evidence was sanitized and material findings were filed.
- [ ] Tracker #25 records the result, limitations, and remaining blockers.
