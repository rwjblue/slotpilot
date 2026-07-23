//! User-scoped local IPC for bounded SlotPilot API messages.
//!
//! The synchronous adapter uses filesystem Unix-domain sockets on Unix and
//! local Windows named pipes on Windows. An OS file lock owns the endpoint, so
//! stale Unix cleanup occurs only after exclusive daemon ownership is proven.
//! Framing is a four-byte big-endian length followed by one JSON value.

use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use fs2::FileExt;
#[cfg(unix)]
use interprocess::local_socket::GenericFilePath;
#[cfg(windows)]
use interprocess::local_socket::GenericNamespaced;
use interprocess::local_socket::{Listener, ListenerOptions, Stream, prelude::*};
use serde::{Serialize, de::DeserializeOwned};
use slotpilot_api::{CommandEnvelope, NoopService, ResponseEnvelope};
use thiserror::Error;

/// Maximum JSON payload accepted by the Phase 0 transport.
pub const MAX_FRAME_BYTES: usize = 64 * 1024;

/// Platform transport represented independently of its adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EndpointKind {
    /// Filesystem Unix-domain socket used on macOS and Linux.
    UnixSocket(PathBuf),
    /// Local Windows named pipe, never a remote host pipe.
    WindowsNamedPipe(String),
}

/// User-scoped endpoint plus its exclusive ownership lock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndpointAddress {
    kind: EndpointKind,
    lock_path: PathBuf,
}

impl EndpointAddress {
    /// Builds the current platform endpoint beneath a private runtime directory.
    ///
    /// `user_scope` must be a stable sanitized per-user token. On Windows it
    /// becomes part of the local pipe name; on Unix directory ownership is
    /// additionally verified against the effective UID.
    pub fn for_user(
        runtime_directory: impl AsRef<Path>,
        user_scope: &str,
    ) -> Result<Self, IpcError> {
        if !(1..=64).contains(&user_scope.len())
            || !user_scope
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(IpcError::InvalidUserScope);
        }
        let runtime_directory = runtime_directory.as_ref().to_path_buf();
        prepare_runtime_directory(&runtime_directory)?;
        let lock_path = runtime_directory.join("slotpilot.lock");
        #[cfg(unix)]
        let kind = EndpointKind::UnixSocket(runtime_directory.join("slotpilot.sock"));
        #[cfg(windows)]
        let kind = EndpointKind::WindowsNamedPipe(format!("slotpilot-{user_scope}"));
        Ok(Self { kind, lock_path })
    }

    /// Returns the represented platform transport.
    #[must_use]
    pub fn kind(&self) -> &EndpointKind {
        &self.kind
    }

    /// Returns the ownership lock path.
    #[must_use]
    pub fn lock_path(&self) -> &Path {
        &self.lock_path
    }
}

/// Cooperative cancellation checked at connection and frame boundaries.
#[derive(Debug, Clone, Default)]
pub struct CancellationToken(Arc<AtomicBool>);

impl CancellationToken {
    /// Creates an active token.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Requests cancellation.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    /// Returns whether cancellation was requested.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }

    fn check(&self) -> Result<(), IpcError> {
        if self.is_cancelled() {
            Err(IpcError::Cancelled)
        } else {
            Ok(())
        }
    }
}

/// Strength of peer authentication available from the platform adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeerAuthorization {
    /// Unix peer credentials match the daemon's effective user ID.
    SameUserVerified,
    /// Windows named-pipe DACL restricts endpoint opening to owner/system, but
    /// the portable adapter exposes only the peer process ID.
    PrivateEndpointOnly,
}

impl PeerAuthorization {
    /// Whether this evidence is sufficient for a future privileged command.
    ///
    /// Phase 0 has no privileged command. Windows deliberately returns false
    /// until a focused design verifies the client token rather than trusting a
    /// reusable process ID.
    #[must_use]
    pub const fn permits_future_privileged_command(self) -> bool {
        matches!(self, Self::SameUserVerified)
    }
}

/// Bound local server with exclusive endpoint ownership.
pub struct LocalServer {
    listener: Listener,
    _ownership: File,
}

impl LocalServer {
    /// Claims and binds one local endpoint.
    pub fn bind(address: &EndpointAddress) -> Result<Self, IpcError> {
        let ownership = open_lock(&address.lock_path)?;
        ownership
            .try_lock_exclusive()
            .map_err(|error| match error.kind() {
                io::ErrorKind::WouldBlock => IpcError::EndpointActive,
                _ => IpcError::Io(error),
            })?;

        #[cfg(unix)]
        if let EndpointKind::UnixSocket(path) = &address.kind
            && path.exists()
        {
            fs::remove_file(path)?;
        }

        let listener = create_listener(&address.kind)?;

        #[cfg(unix)]
        if let EndpointKind::UnixSocket(path) = &address.kind {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
        }

        Ok(Self {
            listener,
            _ownership: ownership,
        })
    }

    /// Accepts and serves one no-op API request.
    pub fn serve_once(
        &self,
        service: &NoopService,
        cancellation: &CancellationToken,
    ) -> Result<(), IpcError> {
        cancellation.check()?;
        let mut stream = self.listener.accept()?;
        let _authorization = authorize_peer(&stream)?;
        cancellation.check()?;
        let request: CommandEnvelope = read_frame(&mut stream, cancellation)?;
        let response = service.execute(request);
        write_frame(&mut stream, &response, cancellation)?;
        Ok(())
    }
}

/// Local client for one-request-per-connection exchanges.
pub struct LocalClient;

impl LocalClient {
    /// Connects, sends one command, and receives one bounded response.
    pub fn request(
        address: &EndpointAddress,
        request: &CommandEnvelope,
        cancellation: &CancellationToken,
    ) -> Result<ResponseEnvelope, IpcError> {
        cancellation.check()?;
        let mut stream = connect(&address.kind)?;
        write_frame(&mut stream, request, cancellation)?;
        read_frame(&mut stream, cancellation)
    }
}

/// Writes one bounded JSON frame.
pub fn write_frame<T: Serialize>(
    writer: &mut impl Write,
    value: &T,
    cancellation: &CancellationToken,
) -> Result<(), IpcError> {
    cancellation.check()?;
    let payload = serde_json::to_vec(value)?;
    if payload.len() > MAX_FRAME_BYTES {
        return Err(IpcError::FrameTooLarge {
            length: payload.len(),
            maximum: MAX_FRAME_BYTES,
        });
    }
    let length = u32::try_from(payload.len()).map_err(|_| IpcError::FrameTooLarge {
        length: payload.len(),
        maximum: MAX_FRAME_BYTES,
    })?;
    writer.write_all(&length.to_be_bytes())?;
    writer.write_all(&payload)?;
    writer.flush()?;
    Ok(())
}

/// Reads one bounded JSON frame without allocating oversized payloads.
pub fn read_frame<T: DeserializeOwned>(
    reader: &mut impl Read,
    cancellation: &CancellationToken,
) -> Result<T, IpcError> {
    cancellation.check()?;
    let mut prefix = [0_u8; 4];
    let read = reader.read(&mut prefix[..1])?;
    if read == 0 {
        return Err(IpcError::Disconnected);
    }
    reader
        .read_exact(&mut prefix[1..])
        .map_err(|error| match error.kind() {
            io::ErrorKind::UnexpectedEof => IpcError::Disconnected,
            _ => IpcError::Io(error),
        })?;
    let length = u32::from_be_bytes(prefix) as usize;
    if length > MAX_FRAME_BYTES {
        return Err(IpcError::FrameTooLarge {
            length,
            maximum: MAX_FRAME_BYTES,
        });
    }
    cancellation.check()?;
    let mut payload = vec![0; length];
    reader
        .read_exact(&mut payload)
        .map_err(|error| match error.kind() {
            io::ErrorKind::UnexpectedEof => IpcError::Disconnected,
            _ => IpcError::Io(error),
        })?;
    Ok(serde_json::from_slice(&payload)?)
}

/// Stable local-transport failure.
#[derive(Debug, Error)]
pub enum IpcError {
    /// Runtime directory, socket, pipe, or stream operation failed.
    #[error("local IPC I/O failed: {0}")]
    Io(#[from] io::Error),
    /// Another daemon owns the endpoint lock.
    #[error("another daemon already owns the local endpoint")]
    EndpointActive,
    /// The runtime directory is not private and owned by the current user.
    #[error("runtime directory is not private to the current user")]
    InsecureRuntimeDirectory,
    /// The per-user scope token is not portable and bounded.
    #[error("user scope must be 1-64 ASCII letters, digits, hyphens, or underscores")]
    InvalidUserScope,
    /// A frame declared or serialized more than the fixed maximum.
    #[error("frame length {length} exceeds maximum {maximum}")]
    FrameTooLarge {
        /// Observed frame length.
        length: usize,
        /// Configured maximum.
        maximum: usize,
    },
    /// JSON framing contained a malformed or incompatible value.
    #[error("malformed JSON frame: {0}")]
    MalformedFrame(#[from] serde_json::Error),
    /// The peer disconnected before a complete frame.
    #[error("peer disconnected before completing a frame")]
    Disconnected,
    /// Cooperative cancellation was requested.
    #[error("operation cancelled")]
    Cancelled,
    /// The address kind is not valid on this build platform.
    #[error("endpoint kind is not supported on this platform")]
    UnsupportedEndpoint,
    /// Peer credentials identify another local user.
    #[error("cross-user local IPC access rejected")]
    CrossUserRejected,
    /// The platform did not provide identity evidence required by its policy.
    #[error("peer identity is unavailable; access fails closed")]
    PeerIdentityUnavailable,
}

fn open_lock(path: &Path) -> Result<File, IpcError> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    Ok(options.open(path)?)
}

#[cfg(unix)]
fn prepare_runtime_directory(path: &Path) -> Result<(), IpcError> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};

    let mut builder = fs::DirBuilder::new();
    builder.recursive(true).mode(0o700);
    builder.create(path)?;
    let metadata = fs::metadata(path)?;
    if !metadata.is_dir()
        || metadata.uid() != nix::unistd::Uid::effective().as_raw()
        || metadata.permissions().mode() & 0o077 != 0
    {
        return Err(IpcError::InsecureRuntimeDirectory);
    }
    Ok(())
}

#[cfg(windows)]
fn prepare_runtime_directory(path: &Path) -> Result<(), IpcError> {
    fs::create_dir_all(path)?;
    Ok(())
}

#[cfg(unix)]
fn create_listener(kind: &EndpointKind) -> Result<Listener, IpcError> {
    let EndpointKind::UnixSocket(path) = kind else {
        return Err(IpcError::UnsupportedEndpoint);
    };
    let name = path.as_os_str().to_fs_name::<GenericFilePath>()?;
    Ok(ListenerOptions::new()
        .name(name)
        .try_overwrite(false)
        .create_sync()?)
}

#[cfg(windows)]
fn create_listener(kind: &EndpointKind) -> Result<Listener, IpcError> {
    use interprocess::os::windows::{
        local_socket::ListenerOptionsExt, security_descriptor::SecurityDescriptor,
    };
    use widestring::U16CString;

    let EndpointKind::WindowsNamedPipe(pipe) = kind else {
        return Err(IpcError::UnsupportedEndpoint);
    };
    let name = pipe.as_str().to_ns_name::<GenericNamespaced>()?;
    let sddl = U16CString::from_str("D:P(A;;GA;;;OW)(A;;GA;;;SY)")
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    let descriptor = SecurityDescriptor::deserialize(&sddl)?;
    Ok(ListenerOptions::new()
        .name(name)
        .security_descriptor(descriptor)
        .create_sync()?)
}

#[cfg(unix)]
fn connect(kind: &EndpointKind) -> Result<Stream, IpcError> {
    let EndpointKind::UnixSocket(path) = kind else {
        return Err(IpcError::UnsupportedEndpoint);
    };
    let name = path.as_os_str().to_fs_name::<GenericFilePath>()?;
    Ok(Stream::connect(name)?)
}

#[cfg(unix)]
fn authorize_peer(stream: &Stream) -> Result<PeerAuthorization, IpcError> {
    let effective_user = stream
        .peer_creds()?
        .euid()
        .ok_or(IpcError::PeerIdentityUnavailable)?;
    if effective_user != nix::unistd::Uid::effective().as_raw() {
        return Err(IpcError::CrossUserRejected);
    }
    Ok(PeerAuthorization::SameUserVerified)
}

#[cfg(windows)]
fn authorize_peer(stream: &Stream) -> Result<PeerAuthorization, IpcError> {
    let _process_id = stream
        .peer_creds()?
        .pid()
        .ok_or(IpcError::PeerIdentityUnavailable)?;
    Ok(PeerAuthorization::PrivateEndpointOnly)
}

#[cfg(windows)]
fn connect(kind: &EndpointKind) -> Result<Stream, IpcError> {
    let EndpointKind::WindowsNamedPipe(pipe) = kind else {
        return Err(IpcError::UnsupportedEndpoint);
    };
    let name = pipe.as_str().to_ns_name::<GenericNamespaced>()?;
    Ok(Stream::connect(name)?)
}

#[cfg(test)]
mod tests {
    use std::{
        io::Cursor,
        sync::atomic::{AtomicU64, Ordering as AtomicOrdering},
        thread,
    };

    use slotpilot_api::{
        API_VERSION, Availability, Command, CommandEnvelope, OperationState, ResponseOutcome,
        ResultBody,
    };

    use super::*;

    static ENDPOINT_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn runtime_directory() -> PathBuf {
        std::env::temp_dir().join(format!(
            "slotpilot-ipc-{}-{}",
            std::process::id(),
            ENDPOINT_COUNTER.fetch_add(1, AtomicOrdering::Relaxed)
        ))
    }

    fn address() -> EndpointAddress {
        EndpointAddress::for_user(runtime_directory(), "test_user").unwrap()
    }

    fn snapshot_request() -> CommandEnvelope {
        CommandEnvelope {
            api_version: API_VERSION,
            request_id: "req_01jabcde9".parse().unwrap(),
            command: Command::GetSnapshot,
        }
    }

    #[test]
    fn endpoint_contract_represents_both_platform_adapters() {
        assert!(matches!(
            EndpointKind::UnixSocket(PathBuf::from("/tmp/slotpilot.sock")),
            EndpointKind::UnixSocket(_)
        ));
        assert!(matches!(
            EndpointKind::WindowsNamedPipe("slotpilot-user".into()),
            EndpointKind::WindowsNamedPipe(_)
        ));
    }

    #[test]
    fn oversized_malformed_disconnect_and_cancellation_are_deterministic() {
        let token = CancellationToken::new();
        let mut oversized = Cursor::new(((MAX_FRAME_BYTES + 1) as u32).to_be_bytes());
        assert!(matches!(
            read_frame::<CommandEnvelope>(&mut oversized, &token),
            Err(IpcError::FrameTooLarge { .. })
        ));

        let payload = b"not-json";
        let mut malformed = Cursor::new(
            (payload.len() as u32)
                .to_be_bytes()
                .into_iter()
                .chain(payload.iter().copied())
                .collect::<Vec<_>>(),
        );
        assert!(matches!(
            read_frame::<CommandEnvelope>(&mut malformed, &token),
            Err(IpcError::MalformedFrame(_))
        ));
        assert!(matches!(
            read_frame::<CommandEnvelope>(&mut Cursor::new(Vec::<u8>::new()), &token),
            Err(IpcError::Disconnected)
        ));
        token.cancel();
        assert!(matches!(
            write_frame(&mut Vec::new(), &snapshot_request(), &token),
            Err(IpcError::Cancelled)
        ));
    }

    #[test]
    fn second_daemon_cannot_take_over_and_reconnect_gets_snapshot() {
        let address = address();
        let server = LocalServer::bind(&address).unwrap();
        assert!(matches!(
            LocalServer::bind(&address),
            Err(IpcError::EndpointActive)
        ));
        let thread_address = address.clone();
        let handle = thread::spawn(move || {
            server
                .serve_once(
                    &NoopService::new("svc_01jabcde9".parse().unwrap()),
                    &CancellationToken::new(),
                )
                .unwrap();
        });
        let response = LocalClient::request(
            &thread_address,
            &snapshot_request(),
            &CancellationToken::new(),
        )
        .unwrap();
        handle.join().unwrap();
        assert!(matches!(
            response.outcome,
            ResponseOutcome::Success(ResultBody::Snapshot(slotpilot_api::StationSnapshot {
                operation: OperationState::NotRunning,
                transmit_authority: Availability::Unavailable,
                ..
            }))
        ));

        let restarted = LocalServer::bind(&address).unwrap();
        let thread_address = address.clone();
        let handle = thread::spawn(move || {
            restarted
                .serve_once(
                    &NoopService::new("svc_01jabcdf0".parse().unwrap()),
                    &CancellationToken::new(),
                )
                .unwrap();
        });
        let response = LocalClient::request(
            &thread_address,
            &snapshot_request(),
            &CancellationToken::new(),
        )
        .unwrap();
        handle.join().unwrap();
        assert!(matches!(
            response.outcome,
            ResponseOutcome::Success(ResultBody::Snapshot(snapshot))
                if snapshot.service_instance_id.as_str() == "svc_01jabcdf0"
        ));
        fs::remove_dir_all(address.lock_path().parent().unwrap()).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn unix_endpoint_permissions_are_private() {
        use std::os::unix::fs::PermissionsExt;

        let address = address();
        let _server = LocalServer::bind(&address).unwrap();
        let EndpointKind::UnixSocket(path) = address.kind() else {
            panic!("Unix build must use a Unix socket");
        };
        let directory_mode = fs::metadata(path.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        let socket_mode = fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(directory_mode, 0o700);
        assert_eq!(socket_mode, 0o600);
    }

    #[cfg(unix)]
    #[test]
    fn stale_socket_is_removed_only_after_ownership_is_acquired() {
        use std::os::unix::fs::FileTypeExt;

        let address = address();
        let EndpointKind::UnixSocket(path) = address.kind() else {
            panic!("Unix build must use a Unix socket");
        };
        fs::write(path, b"stale endpoint marker").unwrap();
        let _server = LocalServer::bind(&address).unwrap();
        assert!(fs::metadata(path).unwrap().file_type().is_socket());
    }
}
