//! Development-only Phase 0 daemon handshake.
//!
//! This binary serves one no-op snapshot request and exits. It has no station,
//! hardware, audio, protocol, scheduling, persistence, or transmit behavior.

fn main() -> anyhow::Result<()> {
    use anyhow::{Context, bail};
    use slotpilot_domain::ServiceInstanceId;
    use slotpilot_ipc::{CancellationToken, EndpointAddress, LocalServer};

    let mut arguments = std::env::args_os().skip(1);
    let Some(mode) = arguments.next() else {
        bail!("RF operation is unavailable; use --phase0-once <runtime-directory>");
    };
    if mode != "--phase0-once" {
        bail!("unsupported mode; RF operation is unavailable");
    }
    let runtime_directory = arguments
        .next()
        .context("--phase0-once requires a runtime directory")?;
    if arguments.next().is_some() {
        bail!("unexpected argument");
    }
    let address = EndpointAddress::for_user(runtime_directory, "phase0-dev")?;
    let server = LocalServer::bind(&address)?;
    let service_instance_id: ServiceInstanceId = "svc_phase0dev"
        .parse()
        .context("invalid built-in development service identity")?;
    slotpilotd::serve_noop_once(&server, service_instance_id, &CancellationToken::new())?;
    Ok(())
}
