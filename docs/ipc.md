# Local IPC contract

Phase 0 uses one request and one response per local connection, framed as a
four-byte big-endian payload length followed by one JSON value. Payloads are
limited to 64 KiB. Oversized lengths are rejected before allocation; malformed
JSON, incomplete frames, disconnect, and cooperative cancellation have typed
outcomes and do not terminate the listener.

## Platform adapters

- macOS and Linux use a filesystem Unix-domain socket in a caller-supplied
  per-user runtime directory.
- Windows uses a local named pipe. A protected DACL grants access to the pipe
  owner and Local System, never a remote host pipe.
- There is no TCP adapter and no network listener.

The Unix runtime directory must be owned by the daemon's effective UID and have
no group/other permissions. The socket and ownership lock are mode `0600`; the
directory is mode `0700`. Each accepted Unix stream also verifies peer
effective UID and rejects a mismatch.

Windows exposes a peer process ID through the portable adapter but not a stable
user token. Endpoint ACLs reject ordinary cross-user opens. This evidence is
explicitly insufficient for any future privileged command; such a command must
fail closed until a focused Windows client-token design is accepted.

## Ownership and recovery

An OS file lock provides exclusive daemon ownership. A second daemon fails
without touching the active endpoint. After a crash, the lock is released by
the OS; only the new lock owner may remove a stale Unix socket before binding.
This avoids replacing a live daemon's socket.

Dropping a connection is graceful. A reconnect opens a new connection and
requests a fresh snapshot, whose service-instance ID distinguishes daemon
restart. Cooperative cancellation is checked before connection acceptance and
at every frame boundary; it does not reinterpret a partial message.

Transport admission provides no transmit authority and carries only the
versioned Phase 0 API contracts.
