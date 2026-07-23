//! Development-only Phase 0 CLI handshake.
//!
//! This binary requests and renders one no-op snapshot. RF operation is
//! unavailable.

fn main() -> anyhow::Result<()> {
    use anyhow::{Context, bail};
    use slotpilot_domain::RequestId;
    use slotpilot_ipc::{CancellationToken, EndpointAddress};

    let mut arguments = std::env::args_os().skip(1);
    let Some(mode) = arguments.next() else {
        bail!("RF operation is unavailable; use phase0-status <runtime-directory>");
    };
    if mode != "phase0-status" {
        bail!("unsupported command; RF operation is unavailable");
    }
    let runtime_directory = arguments
        .next()
        .context("phase0-status requires a runtime directory")?;
    if arguments.next().is_some() {
        bail!("unexpected argument");
    }
    let address = EndpointAddress::for_user(runtime_directory, "phase0-dev")?;
    let request_id: RequestId = "req_phase0dev"
        .parse()
        .context("invalid built-in development request identity")?;
    let response = slotpilot::request_snapshot(&address, request_id, &CancellationToken::new())?;
    println!("{}", slotpilot::render_table(&response));
    Ok(())
}
